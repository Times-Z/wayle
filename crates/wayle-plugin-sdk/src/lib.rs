use std::sync::{Mutex, OnceLock};

mod builders;
mod validators;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use builders::{ActionBuilder, DropdownBuilder, PluginUpdateBuilder};
pub use validators::{
    ValidationError, validate_command, validate_copy_text, validate_custom_payload,
    validate_dropdown_item, validate_label, validate_typed_action, validate_url,
};

pub const PLUGIN_ABI_VERSION: i32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostError {
    PermissionDenied(String),
    PolicyViolation(String),
    InvalidAction(String),
    Unavailable(String),
    UnknownCode { code: String, message: String },
    Legacy(String),
}

impl std::fmt::Display for HostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PermissionDenied(message)
            | Self::PolicyViolation(message)
            | Self::InvalidAction(message)
            | Self::Unavailable(message)
            | Self::Legacy(message) => f.write_str(message),
            Self::UnknownCode { code, message } => write!(f, "{code}:{message}"),
        }
    }
}

impl std::error::Error for HostError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginAction {
    LeftClick,
    RightClick,
    MiddleClick,
    ScrollUp,
    ScrollDown,
}

impl PluginAction {
    pub fn from_code(code: i32) -> Option<Self> {
        match code {
            1 => Some(Self::LeftClick),
            2 => Some(Self::RightClick),
            3 => Some(Self::MiddleClick),
            4 => Some(Self::ScrollUp),
            5 => Some(Self::ScrollDown),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginDropdownItem {
    pub label: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub action: Option<PluginRowAction>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PluginRowAction {
    Legacy(String),
    Typed(PluginTypedRowAction),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum PluginTypedRowAction {
    OpenUrl { url: String },
    RunCommand { command: String },
    CopyText { text: String },
    Custom { payload: String },
    RefreshNow,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginDropdown {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub title_xalign: Option<f32>,
    #[serde(default)]
    pub empty_label: Option<String>,
    #[serde(default)]
    pub items: Vec<PluginDropdownItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginUpdate {
    pub label: String,
    #[serde(default)]
    pub tooltip: Option<String>,
    #[serde(default = "default_visible")]
    pub visible: bool,
    #[serde(default)]
    pub dropdown: Option<PluginDropdown>,
    #[serde(default)]
    pub style: Option<PluginStyle>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginStyle {
    #[serde(default)]
    pub button_class: Option<String>,
    #[serde(default)]
    pub dropdown_class: Option<String>,
    #[serde(default)]
    pub css: Option<String>,
    #[serde(default)]
    pub once: bool,
    #[serde(default)]
    pub dropdown_height_percent: Option<u8>,
}

impl PluginStyle {
    pub fn with_dropdown_height_percent(mut self, percent: u8) -> Self {
        self.dropdown_height_percent = Some(percent.clamp(1, 100));
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub id: String,
    pub version: String,
    pub abi: i32,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub author: String,
    #[serde(default)]
    pub homepage: Option<String>,
    pub license: String,
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub command_exec_allowed_prefixes: Vec<String>,
    #[serde(default)]
    pub fs_read_allowed_paths: Vec<String>,
    #[serde(default)]
    pub features: Vec<String>,
}

fn default_visible() -> bool {
    true
}

pub trait WaylePlugin: Default + Send {
    fn init(&mut self, _config: Value) -> Result<Option<PluginUpdate>, String> {
        Ok(None)
    }

    fn manifest(&self) -> PluginManifest;

    fn refresh(&mut self) -> Result<PluginUpdate, String>;

    fn on_action(&mut self, _action: PluginAction) -> Result<Option<PluginUpdate>, String> {
        Ok(None)
    }

    fn on_row_action(&mut self, _payload: String) -> Result<Option<PluginUpdate>, String> {
        Ok(None)
    }
}

#[cfg(test)]
mod host_import_stubs {
    #[unsafe(no_mangle)]
    extern "C" fn unix_time_seconds() -> i64 {
        0
    }

    #[unsafe(no_mangle)]
    extern "C" fn log_utf8(_ptr: i32, _len: i32) {}

    #[unsafe(no_mangle)]
    extern "C" fn run_command_utf8(_ptr: i32, _len: i32) -> i64 {
        0
    }
}

pub mod host {
    use super::{HostError, pack_payload, read_packed_utf8};

    pub type HostResult<T> = Result<T, HostError>;

    #[link(wasm_import_module = "wayle")]
    unsafe extern "C" {
        fn unix_time_seconds() -> i64;
        fn log_utf8(ptr: i32, len: i32);
        fn run_command_utf8(ptr: i32, len: i32) -> i64;
    }

    pub fn now_unix_seconds() -> i64 {
        unsafe { unix_time_seconds() }
    }

    pub fn log(text: &str) {
        let Ok(len) = i32::try_from(text.len()) else {
            return;
        };
        unsafe {
            log_utf8(text.as_ptr() as i32, len);
        }
    }

    pub fn run_command_typed(command: &str) -> HostResult<String> {
        let len = i32::try_from(command.len())
            .map_err(|_| HostError::InvalidAction(String::from("command too large")))?;
        let packed = unsafe { run_command_utf8(command.as_ptr() as i32, len) };
        if packed == 0 {
            return Err(HostError::Unavailable(String::from(
                "host command returned no output",
            )));
        }
        let output =
            read_packed_utf8(packed).map_err(|error| HostError::Unavailable(error.to_string()))?;
        let output = output.trim_end_matches('\0').to_owned();
        if let Some(error) = output.strip_prefix("__WAYLE_ERROR__:") {
            return Err(parse_host_error(error));
        }
        Ok(output)
    }

    pub fn run_command(command: &str) -> Result<String, String> {
        run_command_typed(command).map_err(|error| error.to_string())
    }

    pub fn pack(ptr: i32, len: i32) -> i64 {
        pack_payload(ptr, len)
    }

    fn parse_host_error(raw: &str) -> HostError {
        let mut parts = raw.splitn(2, ':');
        let Some(code_or_message) = parts.next() else {
            return HostError::Legacy(String::new());
        };
        let Some(message) = parts.next() else {
            return HostError::Legacy(code_or_message.to_owned());
        };

        match code_or_message {
            "WAYLE_ERR_PERMISSION_DENIED" => HostError::PermissionDenied(message.to_owned()),
            "WAYLE_ERR_POLICY_VIOLATION" => HostError::PolicyViolation(message.to_owned()),
            "WAYLE_ERR_INVALID_ACTION" => HostError::InvalidAction(message.to_owned()),
            "WAYLE_ERR_UNAVAILABLE" => HostError::Unavailable(message.to_owned()),
            code if code.starts_with("WAYLE_ERR_") => HostError::UnknownCode {
                code: code.to_owned(),
                message: message.to_owned(),
            },
            _ => HostError::Legacy(raw.to_owned()),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::parse_host_error;
        use crate::HostError;

        #[test]
        fn parses_known_error_code() {
            let parsed = parse_host_error("WAYLE_ERR_PERMISSION_DENIED:missing capability");
            assert_eq!(
                parsed,
                HostError::PermissionDenied(String::from("missing capability"))
            );
        }

        #[test]
        fn parses_unknown_wayle_code() {
            let parsed = parse_host_error("WAYLE_ERR_FUTURE:details");
            assert_eq!(
                parsed,
                HostError::UnknownCode {
                    code: String::from("WAYLE_ERR_FUTURE"),
                    message: String::from("details"),
                }
            );
        }

        #[test]
        fn keeps_legacy_message() {
            let parsed = parse_host_error("plain host failure");
            assert_eq!(
                parsed,
                HostError::Legacy(String::from("plain host failure"))
            );
        }
    }
}

#[cfg(test)]
mod manifest_tests {
    use super::{PLUGIN_ABI_VERSION, PluginManifest};

    #[test]
    fn plugin_manifest_roundtrip_json() {
        let manifest = PluginManifest {
            id: String::from("system-updates"),
            version: String::from("0.1.0"),
            abi: PLUGIN_ABI_VERSION,
            name: String::from("System Updates"),
            description: Some(String::from("Shows package updates")),
            author: String::from("Wayle"),
            homepage: Some(String::from("https://example.org")),
            license: String::from("MIT"),
            capabilities: vec![String::from("command.exec")],
            command_exec_allowed_prefixes: vec![String::from("kitty -e sh -lc '")],
            fs_read_allowed_paths: vec![String::from("/var/lib/pacman")],
            features: vec![String::from("dropdown")],
        };

        let encoded = serde_json::to_string(&manifest).expect("manifest encode");
        let decoded: PluginManifest = serde_json::from_str(&encoded).expect("manifest decode");

        assert_eq!(decoded.id, manifest.id);
        assert_eq!(decoded.version, manifest.version);
        assert_eq!(decoded.abi, manifest.abi);
        assert_eq!(decoded.capabilities, manifest.capabilities);
        assert_eq!(
            decoded.command_exec_allowed_prefixes,
            manifest.command_exec_allowed_prefixes
        );
    }
}

#[cfg(test)]
mod core_tests {
    use super::{
        PLUGIN_ABI_VERSION, PluginAction, PluginManifest, PluginStyle, PluginUpdate, WaylePlugin,
        encode_manifest, encode_output, plugin_abi_version, plugin_alloc, read_input,
        read_packed_utf8,
    };

    #[test]
    fn plugin_action_from_code_maps_all_variants() {
        assert_eq!(PluginAction::from_code(1), Some(PluginAction::LeftClick));
        assert_eq!(PluginAction::from_code(2), Some(PluginAction::RightClick));
        assert_eq!(PluginAction::from_code(3), Some(PluginAction::MiddleClick));
        assert_eq!(PluginAction::from_code(4), Some(PluginAction::ScrollUp));
        assert_eq!(PluginAction::from_code(5), Some(PluginAction::ScrollDown));
        assert_eq!(PluginAction::from_code(99), None);
    }

    #[test]
    fn plugin_abi_and_alloc_basics() {
        assert_eq!(plugin_abi_version(), PLUGIN_ABI_VERSION);
        assert_eq!(plugin_alloc(0), 0);
        let _ = plugin_alloc(8);
    }

    #[test]
    fn read_input_validates_negative_arguments() {
        assert!(read_input(-1, 1).is_err());
        assert!(read_input(1, -1).is_err());
    }

    #[test]
    fn read_packed_utf8_handles_empty_and_invalid_utf8() {
        assert_eq!(read_packed_utf8(0).expect("empty payload"), "");

        let ptr = plugin_alloc(1);
        if ptr <= 0 {
            return;
        }
        let start = usize::try_from(ptr).expect("pointer conversion");
        unsafe {
            let memory = std::slice::from_raw_parts_mut(start as *mut u8, 1);
            memory[0] = 0xFF;
        }
        let packed = ((i64::from(ptr)) << 32) | 1;
        assert!(read_packed_utf8(packed).is_err());
    }

    #[test]
    fn encode_output_and_manifest_roundtrip() {
        assert_eq!(encode_output(None), 0);

        let update = PluginUpdate {
            label: String::from("ok"),
            tooltip: Some(String::from("tip")),
            visible: true,
            dropdown: None,
            style: Some(PluginStyle {
                button_class: Some(String::from("btn")),
                dropdown_class: None,
                css: None,
                once: true,
                dropdown_height_percent: None,
            }),
        };
        let packed_update = encode_output(Some(update));
        if packed_update == 0 {
            return;
        }
        let update_json = read_packed_utf8(packed_update).expect("decode packed update");
        let decoded_update: PluginUpdate =
            serde_json::from_str(&update_json).expect("decode update json");
        assert_eq!(decoded_update.label, "ok");

        let manifest = PluginManifest {
            id: String::from("demo"),
            version: String::from("0.1.0"),
            abi: PLUGIN_ABI_VERSION,
            name: String::from("Demo"),
            description: None,
            author: String::from("Wayle"),
            homepage: None,
            license: String::from("MIT"),
            capabilities: Vec::new(),
            command_exec_allowed_prefixes: Vec::new(),
            fs_read_allowed_paths: Vec::new(),
            features: Vec::new(),
        };
        let packed_manifest = encode_manifest(manifest);
        if packed_manifest == 0 {
            return;
        }
        let manifest_json = read_packed_utf8(packed_manifest).expect("decode packed manifest");
        let decoded_manifest: PluginManifest =
            serde_json::from_str(&manifest_json).expect("decode manifest json");
        assert_eq!(decoded_manifest.id, "demo");
    }

    #[derive(Default)]
    struct DefaultOnly;

    impl WaylePlugin for DefaultOnly {
        fn manifest(&self) -> PluginManifest {
            PluginManifest {
                id: String::from("default-only"),
                version: String::from("0.1.0"),
                abi: PLUGIN_ABI_VERSION,
                name: String::from("Default Only"),
                description: None,
                author: String::from("Wayle"),
                homepage: None,
                license: String::from("MIT"),
                capabilities: Vec::new(),
                command_exec_allowed_prefixes: Vec::new(),
                fs_read_allowed_paths: Vec::new(),
                features: Vec::new(),
            }
        }

        fn refresh(&mut self) -> Result<PluginUpdate, String> {
            Ok(PluginUpdate {
                label: String::from("r"),
                tooltip: None,
                visible: true,
                dropdown: None,
                style: None,
            })
        }
    }

    #[test]
    fn wayle_plugin_defaults() {
        let mut plugin = DefaultOnly;
        assert!(
            plugin
                .init(serde_json::Value::Null)
                .expect("default init")
                .is_none()
        );
        assert_eq!(plugin.manifest().id, "default-only");
        assert!(
            plugin
                .on_action(PluginAction::LeftClick)
                .expect("default on_action")
                .is_none()
        );
        assert!(
            plugin
                .on_row_action(String::from("payload"))
                .expect("default on_row_action")
                .is_none()
        );
    }

    mod macro_exports_tests {
        use super::{PLUGIN_ABI_VERSION, PluginAction, PluginManifest, PluginUpdate, WaylePlugin};

        #[derive(Default)]
        struct ExportedPlugin;

        impl WaylePlugin for ExportedPlugin {
            fn manifest(&self) -> PluginManifest {
                PluginManifest {
                    id: String::from("exported"),
                    version: String::from("0.1.0"),
                    abi: PLUGIN_ABI_VERSION,
                    name: String::from("Exported"),
                    description: None,
                    author: String::from("Wayle"),
                    homepage: None,
                    license: String::from("MIT"),
                    capabilities: vec![String::from("command.exec")],
                    command_exec_allowed_prefixes: vec![String::from("echo ")],
                    fs_read_allowed_paths: Vec::new(),
                    features: vec![String::from("dropdown")],
                }
            }

            fn refresh(&mut self) -> Result<PluginUpdate, String> {
                Ok(PluginUpdate {
                    label: String::from("refresh"),
                    tooltip: None,
                    visible: true,
                    dropdown: None,
                    style: None,
                })
            }

            fn on_action(&mut self, action: PluginAction) -> Result<Option<PluginUpdate>, String> {
                if matches!(action, PluginAction::LeftClick) {
                    return Ok(Some(PluginUpdate {
                        label: String::from("clicked"),
                        tooltip: None,
                        visible: true,
                        dropdown: None,
                        style: None,
                    }));
                }
                Ok(None)
            }

            fn on_row_action(&mut self, payload: String) -> Result<Option<PluginUpdate>, String> {
                if payload == "collapse:system" {
                    return Ok(Some(PluginUpdate {
                        label: String::from("row-action"),
                        tooltip: None,
                        visible: true,
                        dropdown: None,
                        style: None,
                    }));
                }
                Ok(None)
            }
        }

        crate::export_plugin!(ExportedPlugin);

        #[test]
        fn macro_exports_refresh_action_and_manifest_payloads() {
            let init_json = "{}";
            let init_len = i32::try_from(init_json.len()).expect("init json len");
            let init_ptr = crate::plugin_alloc(init_len);
            if init_ptr <= 0 {
                assert_eq!(refresh(), 0);
                assert_eq!(plugin_on_action(1), 0);
                return;
            }
            let start = usize::try_from(init_ptr).expect("init ptr conversion");
            unsafe {
                let memory = std::slice::from_raw_parts_mut(start as *mut u8, init_json.len());
                memory.copy_from_slice(init_json.as_bytes());
            }

            let init_packed = plugin_init(init_ptr, init_len);
            assert_eq!(init_packed, 0);

            let refresh_packed = refresh();
            assert_ne!(refresh_packed, 0);
            let refresh_json = crate::read_packed_utf8(refresh_packed).expect("decode refresh");
            let refresh_update: PluginUpdate =
                serde_json::from_str(&refresh_json).expect("refresh json");
            assert_eq!(refresh_update.label, "refresh");

            let action_packed = plugin_on_action(1);
            assert_ne!(action_packed, 0);
            let action_json = crate::read_packed_utf8(action_packed).expect("decode action");
            let action_update: PluginUpdate =
                serde_json::from_str(&action_json).expect("action json");
            assert_eq!(action_update.label, "clicked");

            let invalid_action = plugin_on_action(99);
            assert_eq!(invalid_action, 0);

            let row_payload = "collapse:system";
            let row_len = i32::try_from(row_payload.len()).expect("row payload len");
            let row_ptr = crate::plugin_alloc(row_len);
            if row_ptr > 0 {
                let row_start = usize::try_from(row_ptr).expect("row ptr conversion");
                unsafe {
                    let memory =
                        std::slice::from_raw_parts_mut(row_start as *mut u8, row_payload.len());
                    memory.copy_from_slice(row_payload.as_bytes());
                }

                let row_packed = plugin_on_row_action(row_ptr, row_len);
                assert_ne!(row_packed, 0);
                let row_json = crate::read_packed_utf8(row_packed).expect("decode row action");
                let row_update: PluginUpdate =
                    serde_json::from_str(&row_json).expect("row action json");
                assert_eq!(row_update.label, "row-action");
            }

            let manifest_packed = plugin_manifest();
            assert_ne!(manifest_packed, 0);
            let manifest_json =
                crate::read_packed_utf8(manifest_packed).expect("decode manifest payload");
            let manifest: PluginManifest =
                serde_json::from_str(&manifest_json).expect("manifest json");
            assert_eq!(manifest.id, "exported");
        }
    }
}

fn pack_payload(ptr: i32, len: i32) -> i64 {
    ((ptr as i64) << 32) | i64::from(len as u32)
}

#[doc(hidden)]
pub fn read_packed_utf8(packed: i64) -> Result<String, String> {
    let ptr = ((packed >> 32) as u32) as usize;
    let len = (packed as u32) as usize;
    if len == 0 {
        return Ok(String::new());
    }

    let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len) };

    std::str::from_utf8(bytes)
        .map(|value| value.to_owned())
        .map_err(|error| error.to_string())
}

#[doc(hidden)]
pub fn read_input(ptr: i32, len: i32) -> Result<String, String> {
    let ptr = usize::try_from(ptr).map_err(|_| String::from("negative pointer"))?;
    let len = usize::try_from(len).map_err(|_| String::from("negative length"))?;
    let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len) };
    std::str::from_utf8(bytes)
        .map(|value| value.to_owned())
        .map_err(|error| error.to_string())
}

#[doc(hidden)]
pub fn encode_output(update: Option<PluginUpdate>) -> i64 {
    let Some(update) = update else {
        return 0;
    };

    let Ok(payload) = serde_json::to_string(&update) else {
        return 0;
    };

    let Ok(len) = i32::try_from(payload.len()) else {
        return 0;
    };

    let ptr = plugin_alloc(len);
    if ptr <= 0 {
        return 0;
    }

    let start = usize::try_from(ptr).unwrap_or(0);
    unsafe {
        let memory = std::slice::from_raw_parts_mut(start as *mut u8, payload.len());
        if memory.len() != payload.len() {
            return 0;
        }
        memory.copy_from_slice(payload.as_bytes());
    }

    pack_payload(ptr, len)
}

#[doc(hidden)]
pub fn encode_manifest(manifest: PluginManifest) -> i64 {
    let Ok(payload) = serde_json::to_string(&manifest) else {
        return 0;
    };

    let Ok(len) = i32::try_from(payload.len()) else {
        return 0;
    };

    let ptr = plugin_alloc(len);
    if ptr <= 0 {
        return 0;
    }

    let start = usize::try_from(ptr).unwrap_or(0);
    unsafe {
        let memory = std::slice::from_raw_parts_mut(start as *mut u8, payload.len());
        if memory.len() != payload.len() {
            return 0;
        }
        memory.copy_from_slice(payload.as_bytes());
    }

    pack_payload(ptr, len)
}

#[unsafe(no_mangle)]
pub extern "C" fn plugin_abi_version() -> i32 {
    PLUGIN_ABI_VERSION
}

#[unsafe(no_mangle)]
pub extern "C" fn plugin_alloc(len: i32) -> i32 {
    if len <= 0 {
        return 0;
    }

    let Ok(len) = usize::try_from(len) else {
        return 0;
    };

    let mut bytes = vec![0u8; len].into_boxed_slice();
    let ptr = bytes.as_mut_ptr() as usize;
    if let Ok(mut store) = allocations().lock() {
        store.push(bytes);
    }
    i32::try_from(ptr).unwrap_or(0)
}

fn allocations() -> &'static Mutex<Vec<Box<[u8]>>> {
    static ALLOCATIONS: OnceLock<Mutex<Vec<Box<[u8]>>>> = OnceLock::new();
    ALLOCATIONS.get_or_init(|| Mutex::new(Vec::new()))
}

#[macro_export]
macro_rules! export_plugin {
    ($plugin_ty:ty) => {
        fn instance() -> &'static std::sync::Mutex<$plugin_ty> {
            static INSTANCE: std::sync::OnceLock<std::sync::Mutex<$plugin_ty>> =
                std::sync::OnceLock::new();
            INSTANCE.get_or_init(|| std::sync::Mutex::new(<$plugin_ty as Default>::default()))
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn plugin_init(ptr: i32, len: i32) -> i64 {
            let config = match $crate::read_input(ptr, len)
                .ok()
                .and_then(|value| serde_json::from_str::<serde_json::Value>(&value).ok())
            {
                Some(value) => value,
                None => serde_json::Value::Null,
            };

            let result = instance()
                .lock()
                .map_err(|_| String::from("plugin mutex poisoned"))
                .and_then(|mut plugin| plugin.init(config));

            match result {
                Ok(update) => $crate::encode_output(update),
                Err(error) => {
                    $crate::host::log(&format!("plugin_init failed: {error}"));
                    0
                }
            }
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn refresh() -> i64 {
            let result = instance()
                .lock()
                .map_err(|_| String::from("plugin mutex poisoned"))
                .and_then(|mut plugin| plugin.refresh().map(Some));

            match result {
                Ok(update) => $crate::encode_output(update),
                Err(error) => {
                    $crate::host::log(&format!("refresh failed: {error}"));
                    0
                }
            }
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn plugin_on_action(action: i32) -> i64 {
            let Some(action) = $crate::PluginAction::from_code(action) else {
                return 0;
            };

            let result = instance()
                .lock()
                .map_err(|_| String::from("plugin mutex poisoned"))
                .and_then(|mut plugin| plugin.on_action(action));

            match result {
                Ok(update) => $crate::encode_output(update),
                Err(error) => {
                    $crate::host::log(&format!("plugin_on_action failed: {error}"));
                    0
                }
            }
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn plugin_on_row_action(ptr: i32, len: i32) -> i64 {
            let Ok(payload) = $crate::read_input(ptr, len) else {
                return 0;
            };

            let result = instance()
                .lock()
                .map_err(|_| String::from("plugin mutex poisoned"))
                .and_then(|mut plugin| plugin.on_row_action(payload));

            match result {
                Ok(update) => $crate::encode_output(update),
                Err(error) => {
                    $crate::host::log(&format!("plugin_on_row_action failed: {error}"));
                    0
                }
            }
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn plugin_manifest() -> i64 {
            let result = instance()
                .lock()
                .map_err(|_| String::from("plugin mutex poisoned"))
                .map(|plugin| plugin.manifest());

            match result {
                Ok(manifest) => $crate::encode_manifest(manifest),
                Err(error) => {
                    $crate::host::log(&format!("plugin_manifest failed: {error}"));
                    0
                }
            }
        }
    };
}
