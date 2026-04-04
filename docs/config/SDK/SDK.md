# Wayle Plugin SDK

Wayle plugins are compiled as WebAssembly modules and loaded at runtime. The `wayle-plugin-sdk` crate provides ABI glue, host bindings, payload types, and the `export_plugin!` macro so plugins can be written safely in Rust.

## Quick Start

### Cargo.toml

```toml
[package]
name = "my-plugin"
version = "0.1.0"
edition = "2024"

[lib]
crate-type = ["cdylib"]

[dependencies]
wayle-plugin-sdk = { path = "../wayle/crates/wayle-plugin-sdk" }
serde_json = "1"
```

### Minimum Plugin

```rust
use wayle_plugin_sdk::{PluginUpdate, WaylePlugin, export_plugin};

#[derive(Default)]
struct MyPlugin;

impl WaylePlugin for MyPlugin {
    fn refresh(&mut self) -> Result<PluginUpdate, String> {
        Ok(PluginUpdate {
            label: "hello".into(),
            tooltip: None,
            visible: true,
            dropdown: None,
            style: None,
        })
    }
}

export_plugin!(MyPlugin);
```

Build:

```bash
rustup target add wasm32-unknown-unknown
cargo build --release --target wasm32-unknown-unknown
```

## Wayle Config

```toml
[[modules.plugins]]
id = "my-plugin"
kind = "wasm"
wasm-path = "/absolute/path/to/my_plugin.wasm"
capabilities = ["command.exec"]
command-exec-allowed-prefixes = ["kitty -e sh -lc '"]
icon-name = "tb-bolt-symbolic"
interval-ms = 60000
left-click = "dropdown:plugin-my-plugin"

[[bar.layout]]
monitor = "*"
right = ["plugin-my-plugin", "clock"]
```

Place plugin modules in bar layout using `plugin-<id>`.

## WaylePlugin Trait

```rust
pub trait WaylePlugin: Default + Send {
    fn init(&mut self, config: serde_json::Value) -> Result<Option<PluginUpdate>, String>;
    fn manifest(&self) -> PluginManifest;
    fn refresh(&mut self) -> Result<PluginUpdate, String>;
    fn on_action(&mut self, action: PluginAction) -> Result<Option<PluginUpdate>, String>;
    fn on_row_action(&mut self, payload: String) -> Result<Option<PluginUpdate>, String>;
}
```

`manifest` and `refresh` are required. Others have defaults.

## PluginManifest

```rust
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub abi: i32,
    pub description: Option<String>,
    pub author: String,
    pub homepage: Option<String>,
    pub license: String,
    pub capabilities: Vec<String>,
    pub command_exec_allowed_prefixes: Vec<String>,
    pub fs_read_allowed_paths: Vec<String>,
    pub features: Vec<String>,
}
```

Runtime notes:
- `plugin_manifest()` export is required.
- Missing export or zero payload is a runtime error.
- Manifest id mismatch logs warning and continues.

### Effective Security Policy (Runtime)

For command execution policy, Wayle computes an effective policy using config first, then manifest fallback:

- `capabilities`:
    - use `[[modules.plugins]].capabilities` when non-empty
    - otherwise fallback to `PluginManifest.capabilities`
- `command-exec-allowed-prefixes`:
    - use `[[modules.plugins]].command-exec-allowed-prefixes` when non-empty
    - otherwise fallback to `PluginManifest.command_exec_allowed_prefixes`

Security outcomes:

- command execution requires effective `command.exec`
- command execution requires a matching prefix in effective `command-exec-allowed-prefixes`
- if both config and manifest prefixes are empty, command execution is blocked
- `allow-shell-commands` does not grant implicit `command.exec`

This means users can always harden or override plugin-declared policy from config.

## PluginUpdate

```rust
pub struct PluginUpdate {
    pub label: String,
    pub tooltip: Option<String>,
    pub visible: bool,
    pub dropdown: Option<PluginDropdown>,
    pub style: Option<PluginStyle>,
}
```

## PluginDropdown

```rust
pub struct PluginDropdown {
    pub title: Option<String>,
    pub title_xalign: Option<f32>,
    pub empty_label: Option<String>,
    pub items: Vec<PluginDropdownItem>,
}
```

- `title_xalign` uses `0.0..1.0` (left..right).

### PluginDropdownItem

```rust
pub struct PluginDropdownItem {
    pub label: String,
    pub description: Option<String>,
    pub action: Option<PluginRowAction>,
}
```

- Empty `label` + no `description` renders a separator.
- Rows without description are rendered as section headers.

## Row Actions

```rust
pub enum PluginRowAction {
    Legacy(String),
    Typed(PluginTypedRowAction),
}

pub enum PluginTypedRowAction {
    OpenUrl { url: String },
    RunCommand { command: String },
    CopyText { text: String },
    Custom { payload: String },
    RefreshNow,
}
```

Capability mapping:
- `RunCommand` -> `command.exec`
- `OpenUrl` -> `net.http.get`
- `CopyText` -> `clipboard.write`
- `Custom` -> plugin-defined payload handled by `on_row_action`
- `RefreshNow` -> immediate refresh

## Builders and Validators

### ActionBuilder

All methods return `Result<PluginRowAction, ValidationError>` except `refresh_now`.

- `ActionBuilder::run_command(command: impl Into<String>) -> Result<PluginRowAction, ValidationError>`
- `ActionBuilder::open_url(url: impl Into<String>) -> Result<PluginRowAction, ValidationError>`
- `ActionBuilder::copy_text(text: impl Into<String>) -> Result<PluginRowAction, ValidationError>`
- `ActionBuilder::custom(payload: impl Into<String>) -> Result<PluginRowAction, ValidationError>`
- `ActionBuilder::refresh_now() -> PluginRowAction`

### DropdownBuilder

```rust
DropdownBuilder::new()
    .title(impl Into<String>)               // Set header title
    .title_xalign(f32)                      // Clamped to 0.0..1.0
    .empty_label(impl Into<String>)         // Shown when items is empty
    .item(PluginDropdownItem)               // Append a regular row
    .separator()                            // Append a visual separator row
    .build() -> PluginDropdown
```

`separator()` appends an item with empty label, no description, and no action.

### PluginUpdateBuilder

```rust
PluginUpdateBuilder::new(label: impl Into<String>)
    .tooltip(impl Into<String>)
    .visible(bool)
    .dropdown(PluginDropdown)
    .style(PluginStyle)
    .build() -> Result<PluginUpdate, ValidationError>
```

`build()` validates the label and all dropdown item labels before returning.

### ValidationError

```rust
pub enum ValidationError {
    EmptyLabel,
    EmptyCommand,
    EmptyText,
    EmptyPayload,
    InvalidUrlScheme,
}
```

### Validation helpers

- `validate_label(label: &str) -> Result<(), ValidationError>`
- `validate_command(command: &str) -> Result<(), ValidationError>`
- `validate_copy_text(text: &str) -> Result<(), ValidationError>`
- `validate_url(url: &str) -> Result<(), ValidationError>` — accepts `http://` and `https://` only
- `validate_custom_payload(payload: &str) -> Result<(), ValidationError>`
- `validate_typed_action(action: &PluginTypedRowAction) -> Result<(), ValidationError>`
- `validate_dropdown_item(item: &PluginDropdownItem) -> Result<(), ValidationError>`

## PluginStyle

```rust
pub struct PluginStyle {
    pub button_class: Option<String>,
    pub dropdown_class: Option<String>,
    pub css: Option<String>,
    pub once: bool,
    pub dropdown_height_percent: Option<u8>,
}
```

Helper:
- `with_dropdown_height_percent(percent: u8)` clamps to `1..100`.

## PluginAction (Bar Interactions)

- `LeftClick`
- `RightClick`
- `MiddleClick`
- `ScrollUp`
- `ScrollDown`

If middle click is unhandled and module action is `None`, shell triggers manual refresh.

## Host Functions (`wayle_plugin_sdk::host`)

- `host::log(text: &str)`
- `host::now_unix_seconds() -> i64`
- `host::run_command(command: &str) -> Result<String, String>`
- `host::run_command_typed(command: &str) -> HostResult<String>`

`command.exec` capability and prefix policy are enforced by host.

`HostResult<T>` is a type alias for `Result<T, HostError>`.

## HostError

```rust
pub enum HostError {
    PermissionDenied(String),
    PolicyViolation(String),
    InvalidAction(String),
    Unavailable(String),
    UnknownCode { code: String, message: String },
    Legacy(String),
}
```

Returned by `host::run_command_typed`. Use `run_command` for a plain `Result<String, String>` version.

- `PermissionDenied` — caller lacks a required capability.
- `PolicyViolation` — command rejected by prefix policy.
- `InvalidAction` — action was structurally invalid (e.g. command string too large).
- `Unavailable` — host could not execute the command.
- `UnknownCode` — a future host error code the SDK does not recognise.
- `Legacy` — raw error string from older host versions.

## Constants

- `PLUGIN_ABI_VERSION: i32 = 1` — current ABI version; embed in `PluginManifest.abi`.

## ABI Exports

Generated by `export_plugin!(MyPlugin)`:
- `plugin_abi_version() -> i32`
- `plugin_alloc(i32) -> i32`
- `plugin_init(i32, i32) -> i64`
- `refresh() -> i64`
- `plugin_on_action(i32) -> i64`
- `plugin_on_row_action(i32, i32) -> i64`
- `plugin_manifest() -> i64`

Payloads use packed pointer/len: `(ptr << 32 | len)`.

## Config Schema Reference

Fields for `[[modules.plugins]]`:

| Field | Type | Default | Description |
|---|---|---|---|
| `id` | string | required | Unique plugin identifier |
| `kind` | `"wasm"` | required | Plugin runtime kind |
| `enabled` | bool | `true` | Enable/disable plugin |
| `wasm-path` | string | none | Path to plugin `.wasm` |
| `interval-ms` | integer | `900000` | Refresh interval |
| `wasm-refresh-export` | string | `"refresh"` | Refresh export function name |
| `wasm-timeout-ms` | integer | `2000` | Timeout per WASM call |
| `capabilities` | `[string]` | `[]` | Declared capability list (must explicitly include `command.exec` to allow command execution) |
| `command-exec-allowed-prefixes` | `[string]` | `[]` | Allowed command prefixes; empty list blocks all command execution |
| `fs-read-allowed-paths` | `[string]` | `[]` | Reserved allowlist for fs-read capability |
| `plugin-config` | object | `{}` | Plugin-specific config forwarded verbatim to `plugin_init` |
| `allow-shell-commands` | bool | `false` | Legacy field (no longer grants `command.exec` when `capabilities` is empty) |
| `icon-name` | string | `"tb-bolt-symbolic"` | Button icon |
| `icon-show` | bool | `true` | Show icon |
| `icon-color` | color | `Auto` | Icon color token/value |
| `icon-bg-color` | color | `Accent` | Icon background color |
| `label-show` | bool | `true` | Show label |
| `label-color` | color | `Accent` | Label color |
| `label-max-length` | integer | `0` | Truncate label at max chars |
| `button-bg-color` | color | `BgSurfaceElevated` | Button background color |
| `border-show` | bool | `false` | Show border |
| `border-color` | color | `BorderAccent` | Border color |
| `hide-if-empty` | bool | `false` | Hide when plugin reports invisible |
| `left-click` | action | `None` | Left click action |
| `right-click` | action | `None` | Right click action |
| `middle-click` | action | `None` | Middle click action |
| `scroll-up` | action | `None` | Scroll up action |
| `scroll-down` | action | `None` | Scroll down action |

Plugin schema:
```json
{
  "$id": "plugin-manifest.json",
  "title": "Wayle Plugin Manifest",
  "type": "object",
  "required": ["id", "name", "version", "abi", "author", "license", "capabilities"],
  "properties": {
    "id": { "type": "string", "pattern": "^[a-z0-9-]+$" },
    "name": { "type": "string", "minLength": 1 },
    "description": { "type": "string" },
    "version": { "type": "string", "minLength": 1 },
    "abi": { "type": "integer", "minimum": 1 },
    "author": { "type": "string", "minLength": 1 },
    "homepage": { "type": "string" },
    "license": { "type": "string", "minLength": 1 },
    "capabilities": {
      "type": "array",
      "items": {
        "type": "string",
        "enum": [
          "command.exec",
          "net.http.get",
          "fs.read.allowed_paths",
          "clipboard.write"
        ]
      },
      "uniqueItems": true
    },
    "command_exec_allowed_prefixes": {
      "type": "array",
      "items": { "type": "string" }
    },
    "fs_read_allowed_paths": {
      "type": "array",
      "items": { "type": "string" }
    },
    "features": {
      "type": "array",
      "items": { "type": "string" },
      "uniqueItems": true
    }
  },
  "additionalProperties": false
}
```

### Security Override In User Config

User config is the strongest control surface for policy hardening. If you set explicit values, they override manifest fallback.

Example strict override:

```toml
[[modules.plugins]]
id = "my-plugin"
kind = "wasm"
wasm-path = "/absolute/path/to/my_plugin.wasm"

capabilities = ["command.exec"]
command-exec-allowed-prefixes = [
    "checkupdates",
]
```

In this example, even if the plugin manifest declares broader permissions, only commands beginning with `checkupdates` are allowed.
