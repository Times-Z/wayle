# wayle-plugin-sdk

SDK for authoring Wayle WASM bar plugins.

## ABI

The SDK targets plugin ABI version `1` and exports:

- `plugin_abi_version() -> i32`
- `plugin_alloc(len: i32) -> i32`
- `plugin_init(config_ptr: i32, config_len: i32) -> i64`
- `plugin_manifest() -> i64`
- `refresh() -> i64`
- `plugin_on_action(action: i32) -> i64` (optional)
- `plugin_on_row_action(payload_ptr: i32, payload_len: i32) -> i64` (optional)

Use the `export_plugin!` macro to generate the required exports.

## Host imports

Wayle provides these imports under the `wayle` module:

- `unix_time_seconds() -> i64`
- `log_utf8(ptr: i32, len: i32)`
- `run_command_utf8(ptr: i32, len: i32) -> i64`

Use `wayle_plugin_sdk::host::*` helpers to call them.

Host helper variants:

- `host::run_command(...) -> Result<String, String>` (backward-compatible string errors)
- `host::run_command_typed(...) -> host::HostResult<String>` (typed `HostError`)

You can expose plugin metadata by implementing `WaylePlugin::manifest()` and returning
`PluginManifest { ... }`.

Builder helpers (additive, optional):

- `ActionBuilder::run_command/open_url/copy_text/custom/refresh_now`
- `PluginUpdateBuilder` for composing validated updates
- `DropdownBuilder` for dropdown rows + separator insertion

Validation helpers:

- `validate_label`, `validate_command`, `validate_copy_text`, `validate_url`
- `validate_custom_payload`, `validate_typed_action`, `validate_dropdown_item`

## Example

```rust
use wayle_plugin_sdk::{
    ActionBuilder,
    DropdownBuilder,
    PLUGIN_ABI_VERSION,
    PluginManifest,
    PluginStyle,
    PluginUpdateBuilder,
    WaylePlugin,
    export_plugin,
};

#[derive(Default)]
struct Demo;

impl WaylePlugin for Demo {
    fn manifest(&self) -> PluginManifest {
        PluginManifest {
            id: String::from("demo"),
            name: String::from("Demo"),
            description: None,
            version: String::from(env!("CARGO_PKG_VERSION")),
            abi: PLUGIN_ABI_VERSION,
            author: String::from("your-name"),
            homepage: None,
            license: String::from("MIT"),
            capabilities: Vec::new(),
            command_exec_allowed_prefixes: Vec::new(),
            fs_read_allowed_paths: Vec::new(),
            features: Vec::new(),
        }
    }

    fn refresh(&mut self) -> Result<wayle_plugin_sdk::PluginUpdate, String> {
        let dropdown = DropdownBuilder::new()
            .title("Demo")
            .empty_label("No entries")
            .item(wayle_plugin_sdk::PluginDropdownItem {
                label: "Open logs".into(),
                description: Some("Row-level action".into()),
                action: Some(
                    ActionBuilder::run_command("alacritty -e journalctl -f")
                        .map_err(|error| error.to_string())?,
                ),
            })
            .build();

        PluginUpdateBuilder::new("ok")
            .tooltip("plugin alive")
            .visible(true)
            .dropdown(dropdown)
            .style(
                PluginStyle {
                    button_class: Some("my-plugin-button".into()),
                    dropdown_class: Some("my-plugin-dropdown".into()),
                    css: Some(".my-plugin-dropdown { }".into()),
                    once: true,
                    dropdown_height_percent: None,
                }
                .with_dropdown_height_percent(50),
            )
            .build()
            .map_err(|error| error.to_string())
    }
}

export_plugin!(Demo);
```

Reference docs:

- `../../docs/config/SDK/SDK.md`
- `../../docs/config/SDK/plugin-schema.json`
