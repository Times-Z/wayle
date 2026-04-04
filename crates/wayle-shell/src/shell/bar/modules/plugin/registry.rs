use std::{future::Future, pin::Pin, sync::Arc};

use wayle_config::{ClickAction, schemas::modules::PluginDefinition};

use super::wasm_runtime;

type RuntimeBuilder = fn(&PluginDefinition) -> Option<Arc<dyn PluginRuntime>>;

const BUILDERS: &[RuntimeBuilder] = &[wasm_runtime::build];

#[derive(Debug, Clone, Copy)]
pub(crate) enum PluginAction {
    LeftClick,
    RightClick,
    MiddleClick,
    ScrollUp,
    ScrollDown,
}

#[derive(Debug, Clone)]
pub(crate) struct PluginUpdate {
    pub label: String,
    pub tooltip: Option<String>,
    pub visible: bool,
    pub dropdown: Option<PluginDropdown>,
    pub style: Option<PluginStyle>,
}

#[derive(Debug, Clone)]
pub(crate) struct PluginStyle {
    pub button_class: Option<String>,
    pub dropdown_class: Option<String>,
    pub css: Option<String>,
    pub once: bool,
    pub dropdown_height_percent: Option<u8>,
}

#[derive(Debug, Clone)]
pub(crate) struct PluginDropdown {
    pub title: Option<String>,
    pub title_xalign: Option<f32>,
    pub empty_label: Option<String>,
    pub items: Vec<PluginDropdownItem>,
}

#[derive(Debug, Clone)]
pub(crate) struct PluginDropdownItem {
    pub label: String,
    pub description: Option<String>,
    pub action: Option<PluginRowAction>,
}

#[derive(Debug, Clone)]
pub(crate) enum PluginRowAction {
    OpenUrl { url: String },
    RunCommand { command: String },
    CopyText { text: String },
    Custom { payload: String },
    RefreshNow,
    LegacyCommand(String),
}

#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum PluginActionResult {
    Unhandled,
    Consumed(Option<PluginUpdate>),
}

pub(crate) trait PluginRuntime: Send + Sync {
    fn css_class(&self) -> &str;

    fn action_for(&self, action: PluginAction) -> ClickAction;

    fn refresh(&self) -> Pin<Box<dyn Future<Output = Result<PluginUpdate, String>> + Send + '_>>;

    fn handle_action(
        &self,
        _action: PluginAction,
    ) -> Pin<Box<dyn Future<Output = Result<PluginActionResult, String>> + Send + '_>> {
        Box::pin(async { Ok(PluginActionResult::Unhandled) })
    }

    fn handle_row_action(
        &self,
        _payload: String,
    ) -> Pin<Box<dyn Future<Output = Result<PluginActionResult, String>> + Send + '_>> {
        Box::pin(async { Ok(PluginActionResult::Unhandled) })
    }
}

pub(crate) fn build_runtime(definition: &PluginDefinition) -> Option<Arc<dyn PluginRuntime>> {
    for builder in BUILDERS {
        if let Some(runtime) = builder(definition) {
            return Some(runtime);
        }
    }
    None
}
