//! Bar component methods: layer-shell positioning, layout diffing,
//! orientation, and section rebuilding.

use std::rc::Rc;

use gtk::prelude::*;
use gtk4_layer_shell::{Edge, LayerShell};
use relm4::{factory::FactoryVecDeque, gtk, gtk::gdk};
use wayle_config::schemas::bar::{BarItem, BarLayout, Location};
use wayle_widgets::prelude::BarSettings;

use super::{
    Bar,
    dropdowns::DropdownRegistry,
    factory::{BarItemFactory, BarItemFactoryInit},
};
use crate::shell::services::ShellServices;

impl Bar {
    pub(super) fn apply_anchors(window: &gtk::Window, location: Location) {
        let (anchor_edge, stretch_edges) = match location {
            Location::Top => (Edge::Top, [Edge::Left, Edge::Right]),
            Location::Bottom => (Edge::Bottom, [Edge::Left, Edge::Right]),
            Location::Left => (Edge::Left, [Edge::Top, Edge::Bottom]),
            Location::Right => (Edge::Right, [Edge::Top, Edge::Bottom]),
        };

        window.set_anchor(Edge::Top, false);
        window.set_anchor(Edge::Bottom, false);
        window.set_anchor(Edge::Left, false);
        window.set_anchor(Edge::Right, false);

        window.set_anchor(anchor_edge, true);

        for edge in stretch_edges {
            window.set_anchor(edge, true);
        }
    }

    pub(super) fn apply_css_classes(
        window: &gtk::Window,
        monitor: &gdk::Monitor,
        location: Location,
        is_floating: bool,
    ) {
        if let Some(connector) = monitor.connector() {
            window.add_css_class(&connector);
            window.set_namespace(Some(&format!("wayle-bar-{connector}")));
        }

        window.add_css_class(location.css_class());

        if is_floating {
            window.add_css_class("floating");
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn apply_orientations(
        center_box: &gtk::CenterBox,
        left_box: &gtk::Box,
        middle_box: &gtk::Box,
        right_box: &gtk::Box,
        left_factory: &gtk::Box,
        center_factory: &gtk::Box,
        right_factory: &gtk::Box,
        is_vertical: bool,
    ) {
        let orientation = if is_vertical {
            gtk::Orientation::Vertical
        } else {
            gtk::Orientation::Horizontal
        };

        center_box.set_orientation(orientation);
        left_box.set_orientation(orientation);
        middle_box.set_orientation(orientation);
        right_box.set_orientation(orientation);

        left_factory.set_orientation(orientation);
        center_factory.set_orientation(orientation);
        right_factory.set_orientation(orientation);

        left_box.set_vexpand(false);
        middle_box.set_vexpand(false);
        right_box.set_vexpand(false);
        left_box.set_hexpand(false);
        middle_box.set_hexpand(false);
        right_box.set_hexpand(false);
    }

    pub(super) fn suppress_alt_focus(window: &gtk::Window) {
        window.connect_focus_visible_notify(|window| {
            if window.gets_focus_visible() {
                window.set_focus_visible(false);
            }
        });

        window.connect_mnemonics_visible_notify(|window| {
            if window.is_mnemonics_visible() {
                window.set_mnemonics_visible(false);
            }
        });
    }

    pub(super) fn apply_layout(&mut self, new_layout: BarLayout, root: &gtk::Window) {
        if self.layout == new_layout {
            return;
        }

        if self.layout.show != new_layout.show {
            root.set_visible(new_layout.show);
        }

        let settings = &self.settings;
        let services = &self.services;
        let dropdowns = &self.dropdowns;

        if self.layout.left != new_layout.left {
            rebuild_section(
                &mut self.left,
                &self.layout.left,
                &new_layout.left,
                settings,
                services,
                dropdowns,
            );
        }

        if self.layout.center != new_layout.center {
            rebuild_section(
                &mut self.center,
                &self.layout.center,
                &new_layout.center,
                settings,
                services,
                dropdowns,
            );
        }

        if self.layout.right != new_layout.right {
            rebuild_section(
                &mut self.right,
                &self.layout.right,
                &new_layout.right,
                settings,
                services,
                dropdowns,
            );
        }

        self.layout = new_layout;
    }

    pub(super) fn reload_plugin_modules(&mut self) {
        let settings = &self.settings;
        let services = &self.services;
        let dropdowns = &self.dropdowns;

        if section_contains_plugin(&self.layout.left) {
            rebuild_section_force(
                &mut self.left,
                &self.layout.left,
                settings,
                services,
                dropdowns,
            );
        }
        if section_contains_plugin(&self.layout.center) {
            rebuild_section_force(
                &mut self.center,
                &self.layout.center,
                settings,
                services,
                dropdowns,
            );
        }
        if section_contains_plugin(&self.layout.right) {
            rebuild_section_force(
                &mut self.right,
                &self.layout.right,
                settings,
                services,
                dropdowns,
            );
        }
    }
}

/// Updates a bar section to match a new layout, only touching modules
/// that actually changed. Modules that stay in the config are left alone
/// (not destroyed and recreated), so they keep their widgets and state.
///
/// Two passes:
///
/// 1. **Remove** - walk the old list, drop anything not in the new list.
///    Uses a shrinking copy of the new list to handle duplicates correctly.
///
/// 2. **Place** - walk the new list by position. Skip if the right module
///    is already there, move it if it exists at a wrong position, or
///    create it if it's new.
fn rebuild_section(
    factory: &mut FactoryVecDeque<BarItemFactory>,
    old_layout: &[BarItem],
    new_layout: &[BarItem],
    settings: &BarSettings,
    services: &ShellServices,
    dropdowns: &Rc<DropdownRegistry>,
) {
    let mut guard = factory.guard();

    let mut remaining: Vec<&BarItem> = new_layout.iter().collect();
    let mut removal_cursor = 0;

    for old_item in old_layout {
        if let Some(matched) = remaining.iter().position(|item| *item == old_item) {
            remaining.remove(matched);
            removal_cursor += 1;
        } else {
            guard.remove(removal_cursor);
        }
    }

    for (target_position, target_item) in new_layout.iter().enumerate() {
        let already_correct = guard
            .get(target_position)
            .is_some_and(|module| module.matches(target_item));

        if already_correct {
            continue;
        }

        let current_position = (target_position..guard.len()).find(|&position| {
            guard
                .get(position)
                .is_some_and(|module| module.matches(target_item))
        });

        match current_position {
            Some(position) => guard.move_to(position, target_position),

            None => {
                guard.insert(
                    target_position,
                    BarItemFactoryInit {
                        item: target_item.clone(),
                        settings: settings.clone(),
                        services: services.clone(),
                        dropdowns: dropdowns.clone(),
                    },
                );
            }
        }
    }
}

fn rebuild_section_force(
    factory: &mut FactoryVecDeque<BarItemFactory>,
    layout: &[BarItem],
    settings: &BarSettings,
    services: &ShellServices,
    dropdowns: &Rc<DropdownRegistry>,
) {
    let mut guard = factory.guard();

    while !guard.is_empty() {
        guard.remove(guard.len() - 1);
    }

    for (idx, item) in layout.iter().enumerate() {
        guard.insert(
            idx,
            BarItemFactoryInit {
                item: item.clone(),
                settings: settings.clone(),
                services: services.clone(),
                dropdowns: dropdowns.clone(),
            },
        );
    }
}

fn section_contains_plugin(items: &[BarItem]) -> bool {
    items.iter().any(|item| match item {
        BarItem::Module(module_ref) => module_ref.module().plugin_id().is_some(),
        BarItem::Group(group) => group
            .modules
            .iter()
            .any(|module_ref| module_ref.module().plugin_id().is_some()),
    })
}

#[cfg(test)]
mod tests {
    use wayle_config::schemas::bar::{BarGroup, BarModule, ClassedModule, ModuleRef};

    use super::{BarItem, section_contains_plugin};

    #[test]
    fn section_contains_plugin_returns_true_for_plain_plugin_module() {
        let items = vec![BarItem::Module(ModuleRef::Plain(BarModule::Plugin(
            String::from("system-updates"),
        )))];

        assert!(section_contains_plugin(&items));
    }

    #[test]
    fn section_contains_plugin_returns_true_for_group_with_plugin_module() {
        let items = vec![BarItem::Group(BarGroup {
            name: String::from("group"),
            modules: vec![
                ModuleRef::Plain(BarModule::Clock),
                ModuleRef::Plain(BarModule::Plugin(String::from("system-updates"))),
            ],
        })];

        assert!(section_contains_plugin(&items));
    }

    #[test]
    fn section_contains_plugin_returns_true_for_classed_plugin_module() {
        let items = vec![BarItem::Module(ModuleRef::Classed(ClassedModule {
            module: BarModule::Plugin(String::from("system-updates")),
            class: String::from("updates-module"),
        }))];

        assert!(section_contains_plugin(&items));
    }

    #[test]
    fn section_contains_plugin_returns_false_for_builtin_and_custom_only() {
        let items = vec![
            BarItem::Module(ModuleRef::Plain(BarModule::Clock)),
            BarItem::Group(BarGroup {
                name: String::from("group"),
                modules: vec![
                    ModuleRef::Plain(BarModule::Volume),
                    ModuleRef::Plain(BarModule::Custom(String::from("updates"))),
                ],
            }),
        ];

        assert!(!section_contains_plugin(&items));
    }

    #[test]
    fn section_contains_plugin_returns_false_for_empty_section() {
        assert!(!section_contains_plugin(&[]));
    }
}
