mod registry;
mod wasm_runtime;

use std::{cell::Cell, rc::Rc, sync::Arc, time::Duration};

use gtk::prelude::*;
use relm4::prelude::*;
use tracing::{info, warn};
use wayle_config::{
    ClickAction, ConfigProperty,
    schemas::{modules::PluginDefinition, styling::CssToken},
};
use wayle_widgets::{
    WatcherToken,
    prelude::{
        BarButton, BarButtonBehavior, BarButtonColors, BarButtonInit, BarButtonInput,
        BarButtonOutput, BarSettings,
    },
    utils::force_window_resize,
};

use self::registry::{
    PluginAction, PluginActionResult, PluginDropdown, PluginRowAction, PluginRuntime, PluginStyle,
    PluginUpdate, build_runtime,
};
use crate::{
    process,
    shell::{
        bar::{
            dropdowns::{self, DropdownInstance, DropdownRegistry},
            modules::registry::{ModuleFactory, ModuleInstance, dynamic_controller},
        },
        helpers::monitors,
        services::ShellServices,
    },
};

pub(crate) struct Factory;

impl Factory {
    pub fn create_for_id(
        id: &str,
        settings: &BarSettings,
        services: &ShellServices,
        dropdowns: &Rc<DropdownRegistry>,
        class: Option<String>,
    ) -> Option<ModuleInstance> {
        let config = services.config.config();
        let definition = config
            .modules
            .plugins
            .get()
            .iter()
            .find(|def| def.id == id && def.enabled)?
            .clone();

        let runtime = build_runtime(&definition)?;

        info!(
            plugin_id = %definition.id,
            kind = ?definition.kind,
            "plugin module initialized"
        );
        let init = PluginInit {
            settings: settings.clone(),
            definition,
            dropdowns: dropdowns.clone(),
            runtime,
        };
        let controller = dynamic_controller(PluginModule::builder().launch(init).detach());
        Some(ModuleInstance { controller, class })
    }
}

impl ModuleFactory for Factory {
    fn create(
        _settings: &BarSettings,
        _services: &ShellServices,
        _dropdowns: &Rc<DropdownRegistry>,
        _class: Option<String>,
    ) -> Option<ModuleInstance> {
        None
    }
}

#[derive(Clone)]
pub(crate) struct PluginInit {
    pub settings: BarSettings,
    pub definition: PluginDefinition,
    pub dropdowns: Rc<DropdownRegistry>,
    pub runtime: Arc<dyn PluginRuntime>,
}

#[derive(Debug)]
pub(crate) enum PluginMsg {
    LeftClick,
    RightClick,
    MiddleClick,
    ScrollUp,
    ScrollDown,
    ManualRefresh,
    CustomRowAction(String),
}

#[derive(Debug)]
pub(crate) enum PluginCmd {
    Snapshot(PluginUpdate),
    ActionResult(PluginAction, PluginActionResult),
    RowActionResult(PluginActionResult),
    Error(String),
}

pub(crate) struct PluginModule {
    bar_button: Controller<BarButton>,
    definition: PluginDefinition,
    dropdowns: Rc<DropdownRegistry>,
    runtime: Arc<dyn PluginRuntime>,
    dropdown_popover: gtk::Popover,
    dropdown_scroller: gtk::ScrolledWindow,
    dropdown_content: gtk::Box,
    dropdown_container: gtk::Box,
    plugin_css_provider: gtk::CssProvider,
    last_plugin_css: Option<String>,
    custom_button_class: Option<String>,
    custom_dropdown_class: Option<String>,
    style_once_applied: bool,
    reopen_after_custom_row_action: bool,
    custom_row_action_in_flight: Rc<Cell<bool>>,
    dropdown_height_percent: Option<u8>,
    _poller_token: WatcherToken,
    last_label_len: usize,
}

#[relm4::component(pub(crate))]
impl Component for PluginModule {
    type Init = PluginInit;
    type Input = PluginMsg;
    type Output = ();
    type CommandOutput = PluginCmd;

    view! {
        gtk::Box {
            add_css_class: "plugin",
            add_css_class: model.runtime.css_class(),

            #[local_ref]
            bar_button -> gtk::MenuButton {},
        }
    }

    #[allow(clippy::too_many_lines)]
    fn init(
        init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let show_icon = ConfigProperty::new(init.definition.icon_show);
        let show_label = ConfigProperty::new(init.definition.label_show);
        let show_border = ConfigProperty::new(init.definition.border_show);
        let label_max_chars = ConfigProperty::new(init.definition.label_max_length);
        let icon_color = ConfigProperty::new(init.definition.icon_color.clone());
        let label_color = ConfigProperty::new(init.definition.label_color.clone());
        let icon_bg_color = ConfigProperty::new(init.definition.icon_bg_color.clone());
        let button_bg_color = ConfigProperty::new(init.definition.button_bg_color.clone());
        let border_color = ConfigProperty::new(init.definition.border_color.clone());

        let bar_button = BarButton::builder()
            .launch(BarButtonInit {
                icon: init.definition.icon_name.clone(),
                label: String::from("..."),
                tooltip: Some(String::from("Checking plugin state...")),
                colors: BarButtonColors {
                    icon_color: icon_color.clone(),
                    label_color: label_color.clone(),
                    icon_background: icon_bg_color.clone(),
                    button_background: button_bg_color.clone(),
                    border_color: border_color.clone(),
                    auto_icon_color: CssToken::Accent,
                },
                behavior: BarButtonBehavior {
                    label_max_chars: label_max_chars.clone(),
                    show_icon: show_icon.clone(),
                    show_label: show_label.clone(),
                    show_border: show_border.clone(),
                    visible: ConfigProperty::new(true),
                },
                settings: init.settings,
            })
            .forward(sender.input_sender(), |output| match output {
                BarButtonOutput::LeftClick => PluginMsg::LeftClick,
                BarButtonOutput::RightClick => PluginMsg::RightClick,
                BarButtonOutput::MiddleClick => PluginMsg::MiddleClick,
                BarButtonOutput::ScrollUp => PluginMsg::ScrollUp,
                BarButtonOutput::ScrollDown => PluginMsg::ScrollDown,
            });

        let mut poller_token = WatcherToken::new();
        spawn_polling(
            &sender,
            init.runtime.clone(),
            init.definition.interval_ms,
            poller_token.reset(),
        );

        let dropdown_content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(10)
            .build();
        dropdown_content.add_css_class("dropdown-content");

        let dropdown_container = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .build();
        dropdown_container.add_css_class("dropdown");

        let dropdown_scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .propagate_natural_height(true)
            .build();
        dropdown_scroller.add_css_class("plugin-dropdown-scroll");
        dropdown_scroller.set_child(Some(&dropdown_content));
        dropdown_container.append(&dropdown_scroller);

        let dropdown_root = gtk::Popover::builder().has_arrow(false).build();
        dropdown_root.set_css_classes(&["dropdown", "plugin-dropdown"]);
        dropdown_root.set_child(Some(&dropdown_container));
        let dropdown_popover = dropdown_root.clone();
        let custom_row_action_in_flight = Rc::new(Cell::new(false));
        let custom_row_action_in_flight_for_close = custom_row_action_in_flight.clone();
        dropdown_popover.connect_closed(move |popover| {
            if custom_row_action_in_flight_for_close.get() {
                popover.popup();
            }
        });

        let plugin_css_provider = gtk::CssProvider::new();
        if let Some(display) = gdk4::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &plugin_css_provider,
                gtk::STYLE_PROVIDER_PRIORITY_USER,
            );
        }

        let dropdown_name = plugin_dropdown_name(&init.definition.id);
        init.dropdowns.register_external_instance(
            dropdown_name,
            DropdownInstance::new(dropdown_root, Box::new(())),
        );

        let model = Self {
            bar_button,
            definition: init.definition,
            dropdowns: init.dropdowns,
            runtime: init.runtime,
            dropdown_popover,
            dropdown_scroller,
            dropdown_content,
            dropdown_container,
            plugin_css_provider,
            last_plugin_css: None,
            custom_button_class: None,
            custom_dropdown_class: None,
            style_once_applied: false,
            reopen_after_custom_row_action: false,
            custom_row_action_in_flight,
            dropdown_height_percent: None,
            _poller_token: poller_token,
            last_label_len: 3,
        };

        let bar_button = model.bar_button.widget();
        let widgets = view_output!();

        if model.definition.hide_if_empty {
            root.set_visible(false);
        }

        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        if matches!(msg, PluginMsg::ManualRefresh) {
            spawn_refresh_once(&sender, self.runtime.clone());
            return;
        }

        if let PluginMsg::CustomRowAction(payload) = msg {
            self.reopen_after_custom_row_action = self.dropdown_popover.is_visible();
            self.custom_row_action_in_flight.set(true);
            spawn_row_action(&sender, self.runtime.clone(), payload);
            return;
        }

        let Some(action_kind) = as_action(msg) else {
            return;
        };

        spawn_action(&sender, self.runtime.clone(), action_kind);
    }

    fn update_cmd(&mut self, msg: PluginCmd, sender: ComponentSender<Self>, root: &Self::Root) {
        let is_custom_row_action_completion =
            matches!(&msg, PluginCmd::RowActionResult(_) | PluginCmd::Error(_));

        match msg {
            PluginCmd::Snapshot(update) => {
                self.apply_update(update, root, &sender);
            }
            PluginCmd::ActionResult(action_kind, action_result) => match action_result {
                PluginActionResult::Unhandled => {
                    let action = self.runtime.action_for(action_kind);
                    if matches!(action, ClickAction::None)
                        && matches!(action_kind, PluginAction::MiddleClick)
                    {
                        sender.input(PluginMsg::ManualRefresh);
                        return;
                    }
                    dropdowns::dispatch_click(&action, &self.dropdowns, &self.bar_button);
                }
                PluginActionResult::Consumed(Some(update)) => {
                    self.apply_update(update, root, &sender);
                }
                PluginActionResult::Consumed(None) => {}
            },
            PluginCmd::RowActionResult(action_result) => match action_result {
                PluginActionResult::Unhandled | PluginActionResult::Consumed(None) => {}
                PluginActionResult::Consumed(Some(update)) => {
                    self.apply_update(update, root, &sender);
                }
            },
            PluginCmd::Error(error) => {
                warn!(error = %error, plugin_id = %self.definition.id, "plugin refresh failed");
                self.bar_button
                    .emit(BarButtonInput::SetLabel(String::from("?")));
                self.bar_button
                    .emit(BarButtonInput::SetTooltip(Some(String::from(
                        "Plugin refresh failed",
                    ))));
                root.set_visible(true);
            }
        }

        if self.reopen_after_custom_row_action && !self.dropdown_popover.is_visible() {
            self.dropdown_popover.popup();
        }
        self.reopen_after_custom_row_action = false;

        if is_custom_row_action_completion {
            let in_flight = self.custom_row_action_in_flight.clone();
            gtk::glib::idle_add_local_once(move || {
                in_flight.set(false);
            });
        }
    }
}

impl PluginModule {
    fn apply_update(
        &mut self,
        update: PluginUpdate,
        root: &gtk::Box,
        sender: &ComponentSender<PluginModule>,
    ) {
        let new_len = update.label.chars().count();
        self.bar_button.emit(BarButtonInput::SetLabel(update.label));
        self.bar_button
            .emit(BarButtonInput::SetTooltip(update.tooltip));
        root.set_visible(update.visible);

        if let Some(dropdown) = update.dropdown {
            render_dropdown_content(
                &self.dropdown_content,
                dropdown,
                &self.definition,
                sender.input_sender().clone(),
            );

            // Ensure geometry is recomputed both while open and before next reopen.
            self.dropdown_content.set_size_request(-1, -1);
            self.apply_dropdown_height_constraint();
            self.dropdown_container.set_size_request(-1, -1);
            self.dropdown_content.queue_resize();
            self.dropdown_scroller.queue_resize();
            self.dropdown_container.queue_resize();
            self.dropdown_popover.queue_resize();

            let popover = self.dropdown_popover.clone();
            gtk::glib::idle_add_local_once(move || {
                popover.queue_resize();
                if popover.is_visible() {
                    popover.popup();
                }
            });
        }

        if let Some(style) = update.style {
            self.apply_style_classes(root, style);
        }

        if new_len != self.last_label_len {
            self.last_label_len = new_len;
            force_window_resize(root);
        }
    }

    fn apply_style_classes(&mut self, root: &gtk::Box, style: PluginStyle) {
        let dropdown_height_percent = style.dropdown_height_percent;

        if style.once && self.style_once_applied {
            return;
        }

        update_css_class(
            root,
            &mut self.custom_button_class,
            style.button_class.as_deref(),
        );
        update_css_class(
            &self.dropdown_container,
            &mut self.custom_dropdown_class,
            style.dropdown_class.as_deref(),
        );
        if let Some(css) = style.css {
            self.apply_plugin_css(css);
        }

        self.dropdown_height_percent = dropdown_height_percent.map(|value| value.clamp(1, 100));
        self.apply_dropdown_height_constraint();

        if style.once {
            self.style_once_applied = true;
        }
    }

    fn apply_plugin_css(&mut self, css: String) {
        if self.last_plugin_css.as_deref() == Some(css.as_str()) {
            return;
        }
        self.plugin_css_provider.load_from_string(&css);
        self.last_plugin_css = Some(css);
    }

    fn apply_dropdown_height_constraint(&self) {
        let Some(percent) = self.dropdown_height_percent else {
            self.dropdown_scroller.set_propagate_natural_height(true);
            self.dropdown_scroller.set_min_content_height(-1);
            self.dropdown_scroller.set_max_content_height(-1);
            return;
        };

        let locked_height = detect_screen_height_percent(percent);
        self.dropdown_scroller.set_propagate_natural_height(false);
        self.dropdown_scroller.set_min_content_height(locked_height);
        self.dropdown_scroller.set_max_content_height(locked_height);
    }
}

fn update_css_class(widget: &impl IsA<gtk::Widget>, slot: &mut Option<String>, next: Option<&str>) {
    let Some(next) = next.and_then(normalize_css_class) else {
        return;
    };

    if slot.as_deref() == Some(next.as_str()) {
        return;
    }

    if let Some(previous) = slot.take() {
        widget.remove_css_class(&previous);
    }
    widget.add_css_class(&next);
    *slot = Some(next);
}

fn normalize_css_class(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }

    if trimmed
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return Some(trimmed.to_owned());
    }

    None
}

fn plugin_dropdown_name(id: &str) -> String {
    format!("plugin-{id}")
}

fn detect_screen_height_percent(percent: u8) -> i32 {
    const FALLBACK_HEIGHT: i32 = 540;
    const MIN_HEIGHT: i32 = 260;

    let tallest = monitors::current_monitors()
        .into_iter()
        .map(|(_, monitor)| monitor.geometry().height())
        .max()
        .unwrap_or(0);

    if tallest <= 0 {
        return FALLBACK_HEIGHT;
    }

    let ratio = (percent.clamp(1, 100) as f32) / 100.0;
    (((tallest as f32) * ratio).round() as i32).max(MIN_HEIGHT)
}

#[allow(clippy::too_many_lines)]
fn render_dropdown_content(
    container: &gtk::Box,
    dropdown: PluginDropdown,
    definition: &PluginDefinition,
    refresh_sender: relm4::Sender<PluginMsg>,
) {
    while let Some(child) = container.first_child() {
        child.unparent();
    }

    if let Some(title) = dropdown.title
        && !title.trim().is_empty()
    {
        let title_xalign = dropdown
            .title_xalign
            .filter(|value| value.is_finite())
            .map(|value| value.clamp(0.0, 1.0))
            .unwrap_or(0.0);

        let label = gtk::Label::builder()
            .xalign(title_xalign)
            .wrap(true)
            .label(&title)
            .build();
        label.add_css_class("dropdown-title");
        container.append(&label);
    }

    if dropdown.items.is_empty() {
        let empty_label = dropdown
            .empty_label
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("No items");
        let label = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .label(empty_label)
            .build();
        label.add_css_class("plugin-dropdown-empty");
        container.append(&label);
        return;
    }

    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .build();
    list.add_css_class("plugin-dropdown-list");

    for item in dropdown.items {
        if item.label.trim().is_empty() && item.description.is_none() {
            let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
            separator.add_css_class("plugin-dropdown-separator");
            container.append(&separator);
            continue;
        }

        let row_action = item.action.clone();
        let action_allowed = row_action
            .as_ref()
            .is_some_and(|action| validate_row_action(action, definition));
        let is_custom_row_action = row_action
            .as_ref()
            .is_some_and(|action| matches!(action, PluginRowAction::Custom { .. }));

        let row = gtk::ListBoxRow::new();
        row.set_selectable(false);
        row.set_activatable(row_action.is_some() && action_allowed && !is_custom_row_action);

        let row_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .build();
        row_box.add_css_class("plugin-dropdown-item");

        let label = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .label(&item.label)
            .build();
        label.add_css_class("plugin-dropdown-item-label");
        if item.description.is_none() {
            label.add_css_class("plugin-dropdown-section-label");
            row_box.add_css_class("plugin-dropdown-section");
        }
        row_box.append(&label);

        if let Some(description) = item.description
            && !description.trim().is_empty()
        {
            let description_label = gtk::Label::builder()
                .xalign(0.0)
                .wrap(true)
                .label(&description)
                .build();
            description_label.add_css_class("plugin-dropdown-item-description");
            row_box.append(&description_label);
        }

        if let Some(action) = row_action {
            if action_allowed {
                row.add_css_class("plugin-dropdown-actionable");
                let gesture = gtk::GestureClick::new();
                gesture.set_button(0);
                if matches!(action, PluginRowAction::Custom { .. }) {
                    gesture.set_propagation_phase(gtk::PropagationPhase::Capture);
                    let refresh_sender = refresh_sender.clone();
                    gesture.connect_pressed(move |gesture, _, _, _| {
                        gesture.set_state(gtk::EventSequenceState::Claimed);
                    });
                    let action_for_release = action.clone();
                    gesture.connect_released(move |gesture, _, _, _| {
                        gesture.set_state(gtk::EventSequenceState::Claimed);
                        let refresh_sender = refresh_sender.clone();
                        let action_for_idle = action_for_release.clone();
                        gtk::glib::idle_add_local_once(move || {
                            execute_row_action(action_for_idle, &refresh_sender);
                        });
                    });
                } else {
                    let refresh_sender = refresh_sender.clone();
                    gesture.connect_released(move |_, _, _, _| {
                        execute_row_action(action.clone(), &refresh_sender);
                    });
                }
                row.add_controller(gesture);
            } else {
                row.add_css_class("plugin-dropdown-action-disabled");
            }
        }

        row.set_child(Some(&row_box));
        list.append(&row);
    }

    container.append(&list);
}

fn validate_row_action(action: &PluginRowAction, definition: &PluginDefinition) -> bool {
    match action {
        PluginRowAction::RunCommand { command } | PluginRowAction::LegacyCommand(command) => {
            definition.is_command_allowed(command)
        }
        PluginRowAction::OpenUrl { url } => {
            definition.has_capability("net.http.get")
                && (url.starts_with("https://") || url.starts_with("http://"))
        }
        PluginRowAction::CopyText { .. } => definition.has_capability("clipboard.write"),
        PluginRowAction::Custom { .. } => true,
        PluginRowAction::RefreshNow => true,
    }
}

fn execute_row_action(action: PluginRowAction, refresh_sender: &relm4::Sender<PluginMsg>) {
    match action {
        PluginRowAction::RunCommand { command } | PluginRowAction::LegacyCommand(command) => {
            process::run_if_set(&command)
        }
        PluginRowAction::OpenUrl { url } => {
            process::run_if_set(&format!("xdg-open '{}'", shell_quote_single(&url)));
        }
        PluginRowAction::CopyText { text } => {
            process::run_if_set(&format!(
                "printf %s '{}' | wl-copy",
                shell_quote_single(&text)
            ));
        }
        PluginRowAction::Custom { payload } => {
            refresh_sender.emit(PluginMsg::CustomRowAction(payload));
        }
        PluginRowAction::RefreshNow => {
            refresh_sender.emit(PluginMsg::ManualRefresh);
        }
    }
}

fn shell_quote_single(input: &str) -> String {
    input.replace('\'', "'\"'\"'")
}

fn as_action(msg: PluginMsg) -> Option<PluginAction> {
    match msg {
        PluginMsg::LeftClick => Some(PluginAction::LeftClick),
        PluginMsg::RightClick => Some(PluginAction::RightClick),
        PluginMsg::MiddleClick => Some(PluginAction::MiddleClick),
        PluginMsg::ScrollUp => Some(PluginAction::ScrollUp),
        PluginMsg::ScrollDown => Some(PluginAction::ScrollDown),
        PluginMsg::ManualRefresh | PluginMsg::CustomRowAction(_) => None,
    }
}

fn spawn_polling(
    sender: &ComponentSender<PluginModule>,
    runtime: Arc<dyn PluginRuntime>,
    interval_ms: u64,
    token: tokio_util::sync::CancellationToken,
) {
    sender.command(move |out, shutdown| async move {
        if let Err(error) = refresh_runtime(&out, runtime.clone()).await {
            let _ = out.send(PluginCmd::Error(error));
        }

        if interval_ms == 0 {
            return;
        }

        let mut ticker = tokio::time::interval(Duration::from_millis(interval_ms));

        loop {
            tokio::select! {
                () = shutdown.clone().wait() => break,
                () = token.cancelled() => break,
                _ = ticker.tick() => {
                    if let Err(error) = refresh_runtime(&out, runtime.clone()).await {
                        let _ = out.send(PluginCmd::Error(error));
                    }
                }
            }
        }
    });
}

fn spawn_refresh_once(sender: &ComponentSender<PluginModule>, runtime: Arc<dyn PluginRuntime>) {
    sender.command(move |out, shutdown| async move {
        tokio::select! {
            () = shutdown.wait() => {}
            result = refresh_runtime(&out, runtime) => {
                if let Err(error) = result {
                    let _ = out.send(PluginCmd::Error(error));
                }
            }
        }
    });
}

async fn refresh_runtime(
    out: &relm4::Sender<PluginCmd>,
    runtime: Arc<dyn PluginRuntime>,
) -> Result<(), String> {
    let update = runtime.refresh().await?;
    let _ = out.send(PluginCmd::Snapshot(update));
    Ok(())
}

fn spawn_action(
    sender: &ComponentSender<PluginModule>,
    runtime: Arc<dyn PluginRuntime>,
    action: PluginAction,
) {
    sender.command(move |out, shutdown| async move {
        tokio::select! {
            () = shutdown.wait() => {}
            result = runtime.handle_action(action) => {
                match result {
                    Ok(action_result) => {
                        let _ = out.send(PluginCmd::ActionResult(action, action_result));
                    }
                    Err(error) => {
                        let _ = out.send(PluginCmd::Error(error));
                    }
                }
            }
        }
    });
}

fn spawn_row_action(
    sender: &ComponentSender<PluginModule>,
    runtime: Arc<dyn PluginRuntime>,
    payload: String,
) {
    sender.command(move |out, shutdown| async move {
        tokio::select! {
            () = shutdown.wait() => {}
            result = runtime.handle_row_action(payload) => {
                match result {
                    Ok(action_result) => {
                        let _ = out.send(PluginCmd::RowActionResult(action_result));
                    }
                    Err(error) => {
                        let _ = out.send(PluginCmd::Error(error));
                    }
                }
            }
        }
    });
}
