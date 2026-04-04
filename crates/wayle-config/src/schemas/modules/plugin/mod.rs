use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    ClickAction,
    schemas::styling::{ColorValue, CssToken},
};

/// Supported native plugin kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum PluginKind {
    /// External WebAssembly plugin loaded at runtime.
    Wasm,
}

/// Native plugin definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub struct PluginDefinition {
    /// Unique plugin identifier.
    pub id: String,

    /// Plugin implementation kind.
    pub kind: PluginKind,

    /// Whether this plugin is enabled.
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// Poll interval in milliseconds.
    #[serde(rename = "interval-ms", default = "default_interval_ms")]
    pub interval_ms: u64,

    /// Path to a WebAssembly module for `kind = "wasm"`.
    #[serde(rename = "wasm-path", default)]
    pub wasm_path: Option<String>,

    /// Export name called for each refresh in `kind = "wasm"` plugins.
    #[serde(
        rename = "wasm-refresh-export",
        default = "default_wasm_refresh_export"
    )]
    pub wasm_refresh_export: String,

    /// Timeout in milliseconds for one WASM refresh call.
    #[serde(rename = "wasm-timeout-ms", default = "default_wasm_timeout_ms")]
    pub wasm_timeout_ms: u64,

    /// Declared plugin capabilities.
    #[serde(default = "default_capabilities")]
    pub capabilities: Vec<String>,

    /// Allowed command prefixes for `command.exec` capability.
    #[serde(rename = "command-exec-allowed-prefixes", default)]
    pub command_exec_allowed_prefixes: Vec<String>,

    /// Allowed filesystem paths for `fs.read.allowed_paths` capability.
    #[serde(rename = "fs-read-allowed-paths", default)]
    pub fs_read_allowed_paths: Vec<String>,

    /// Plugin-specific initialization configuration passed to plugin_init.
    #[serde(rename = "plugin-config", default = "default_plugin_config")]
    pub plugin_config: Value,

    /// Allow this plugin to execute shell commands through host APIs.
    /// Deprecated in favor of `capabilities`, retained for backward compatibility.
    #[serde(
        rename = "allow-shell-commands",
        default = "default_allow_shell_commands"
    )]
    pub allow_shell_commands: bool,

    /// Static symbolic icon name.
    #[serde(rename = "icon-name", default = "default_icon_name")]
    pub icon_name: String,

    /// Display border around button.
    #[serde(rename = "border-show", default)]
    pub border_show: bool,

    /// Border color token.
    #[serde(rename = "border-color", default = "default_border_color")]
    pub border_color: ColorValue,

    /// Display module icon.
    #[serde(rename = "icon-show", default = "default_icon_show")]
    pub icon_show: bool,

    /// Icon foreground color.
    #[serde(rename = "icon-color", default = "default_icon_color")]
    pub icon_color: ColorValue,

    /// Icon container background color token.
    #[serde(rename = "icon-bg-color", default = "default_icon_bg_color")]
    pub icon_bg_color: ColorValue,

    /// Display text label.
    #[serde(rename = "label-show", default = "default_label_show")]
    pub label_show: bool,

    /// Label text color token.
    #[serde(rename = "label-color", default = "default_label_color")]
    pub label_color: ColorValue,

    /// Max label characters before truncation with ellipsis.
    #[serde(rename = "label-max-length", default)]
    pub label_max_length: u32,

    /// Button background color token.
    #[serde(rename = "button-bg-color", default = "default_button_bg_color")]
    pub button_bg_color: ColorValue,

    /// Action on left click.
    #[serde(rename = "left-click", default = "default_left_click")]
    pub left_click: ClickAction,

    /// Action on right click.
    #[serde(rename = "right-click", default)]
    pub right_click: ClickAction,

    /// Action on middle click.
    #[serde(rename = "middle-click", default)]
    pub middle_click: ClickAction,

    /// Action on scroll up.
    #[serde(rename = "scroll-up", default)]
    pub scroll_up: ClickAction,

    /// Action on scroll down.
    #[serde(rename = "scroll-down", default)]
    pub scroll_down: ClickAction,

    /// Hide module when the runtime reports it as not visible.
    #[serde(rename = "hide-if-empty", default)]
    pub hide_if_empty: bool,
}

fn default_enabled() -> bool {
    true
}

fn default_interval_ms() -> u64 {
    900_000
}

fn default_icon_name() -> String {
    String::from("tb-bolt-symbolic")
}

fn default_wasm_refresh_export() -> String {
    String::from("refresh")
}

fn default_wasm_timeout_ms() -> u64 {
    2_000
}

fn default_allow_shell_commands() -> bool {
    false
}

fn default_capabilities() -> Vec<String> {
    Vec::new()
}

fn default_plugin_config() -> Value {
    Value::Object(Default::default())
}

fn default_border_color() -> ColorValue {
    ColorValue::Token(CssToken::BorderAccent)
}

fn default_icon_show() -> bool {
    true
}

fn default_icon_color() -> ColorValue {
    ColorValue::Auto
}

fn default_icon_bg_color() -> ColorValue {
    ColorValue::Token(CssToken::Accent)
}

fn default_label_show() -> bool {
    true
}

fn default_label_color() -> ColorValue {
    ColorValue::Token(CssToken::Accent)
}

fn default_button_bg_color() -> ColorValue {
    ColorValue::Token(CssToken::BgSurfaceElevated)
}

fn default_left_click() -> ClickAction {
    ClickAction::None
}

impl PluginDefinition {
    /// Returns true when the plugin declares the requested capability.
    pub fn has_capability(&self, capability: &str) -> bool {
        self.capabilities.iter().any(|entry| entry == capability)
    }

    /// Returns true when command execution is allowed by capability and prefix policy.
    pub fn is_command_allowed(&self, command: &str) -> bool {
        if !self.has_capability("command.exec") {
            return false;
        }

        if self.command_exec_allowed_prefixes.is_empty() {
            return false;
        }

        let command = command.trim();
        self.command_exec_allowed_prefixes
            .iter()
            .map(String::as_str)
            .map(str::trim)
            .filter(|prefix| !prefix.is_empty())
            .any(|prefix| command.starts_with(prefix))
    }
}

#[cfg(test)]
mod tests {
    use super::{PluginDefinition, PluginKind};

    fn sample_definition() -> PluginDefinition {
        PluginDefinition {
            id: String::from("demo"),
            kind: PluginKind::Wasm,
            enabled: true,
            interval_ms: 1_000,
            wasm_path: Some(String::from("/tmp/demo.wasm")),
            wasm_refresh_export: String::from("refresh"),
            wasm_timeout_ms: 2_000,
            capabilities: vec![String::from("command.exec")],
            command_exec_allowed_prefixes: Vec::new(),
            fs_read_allowed_paths: Vec::new(),
            plugin_config: serde_json::Value::Object(Default::default()),
            allow_shell_commands: false,
            icon_name: String::from("tb-bolt-symbolic"),
            border_show: false,
            border_color: crate::schemas::styling::ColorValue::Token(
                crate::schemas::styling::CssToken::BorderAccent,
            ),
            icon_show: true,
            icon_color: crate::schemas::styling::ColorValue::Auto,
            icon_bg_color: crate::schemas::styling::ColorValue::Token(
                crate::schemas::styling::CssToken::Accent,
            ),
            label_show: true,
            label_color: crate::schemas::styling::ColorValue::Token(
                crate::schemas::styling::CssToken::Accent,
            ),
            label_max_length: 0,
            button_bg_color: crate::schemas::styling::ColorValue::Token(
                crate::schemas::styling::CssToken::BgSurfaceElevated,
            ),
            left_click: crate::ClickAction::None,
            right_click: crate::ClickAction::None,
            middle_click: crate::ClickAction::None,
            scroll_up: crate::ClickAction::None,
            scroll_down: crate::ClickAction::None,
            hide_if_empty: false,
        }
    }

    #[test]
    fn command_is_denied_when_prefix_list_is_empty() {
        let definition = sample_definition();
        assert!(!definition.is_command_allowed("checkupdates 2>/dev/null || true"));
    }

    #[test]
    fn command_is_allowed_when_matching_prefix_exists() {
        let mut definition = sample_definition();
        definition.command_exec_allowed_prefixes = vec![String::from("checkupdates")];
        assert!(definition.is_command_allowed("checkupdates 2>/dev/null || true"));
        assert!(!definition.is_command_allowed("yay -Qua 2>/dev/null || true"));
    }

    #[test]
    fn missing_capability_blocks_command_exec_even_with_legacy_flag() {
        let mut definition = sample_definition();
        definition.capabilities.clear();
        definition.allow_shell_commands = true;
        definition.command_exec_allowed_prefixes = vec![String::from("checkupdates")];
        assert!(!definition.is_command_allowed("checkupdates 2>/dev/null || true"));
    }
}
