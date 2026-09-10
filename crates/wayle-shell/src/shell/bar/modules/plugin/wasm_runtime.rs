use std::{
    collections::{HashMap, HashSet},
    future::Future,
    path::PathBuf,
    pin::Pin,
    process::Command,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};

use console::style;
use serde::Deserialize;
use tokio::time::timeout;
use tracing::{debug, info, warn};
use wasmtime::{Caller, Engine, Linker, Memory, Module, Store, TypedFunc};
use wayle_config::{
    ClickAction,
    schemas::modules::{PluginDefinition, PluginKind},
};

use super::registry::{
    PluginAction, PluginActionResult, PluginDropdown, PluginDropdownItem, PluginRowAction,
    PluginRuntime, PluginStyle, PluginUpdate,
};

const PLUGIN_ABI_VERSION: i32 = 1;
const NO_PAYLOAD: i64 = 0;

fn wasm_engine() -> &'static Engine {
    static ENGINE: OnceLock<Engine> = OnceLock::new();

    ENGINE.get_or_init(|| {
        let config = wasmtime::Config::new();
        match Engine::new(&config) {
            Ok(engine) => engine,
            Err(error) => {
                warn!(error = %error, "failed to build custom wasmtime engine, using default engine");
                Engine::default()
            }
        }
    })
}

fn wasm_module(engine: &Engine, wasm_path: &PathBuf) -> Result<Module, String> {
    static MODULE_CACHE: OnceLock<Mutex<HashMap<PathBuf, Module>>> = OnceLock::new();

    let cache = MODULE_CACHE.get_or_init(|| Mutex::new(HashMap::new()));

    if let Ok(cache_guard) = cache.lock()
        && let Some(module) = cache_guard.get(wasm_path)
    {
        return Ok(module.clone());
    }

    let module = Module::from_file(engine, wasm_path).map_err(|error| error.to_string())?;

    if let Ok(mut cache_guard) = cache.lock() {
        cache_guard.insert(wasm_path.clone(), module.clone());
    }

    Ok(module)
}

fn should_log_lazy_init(plugin_id: &str) -> bool {
    static LOGGED_LAZY_INIT: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

    let logged = LOGGED_LAZY_INIT.get_or_init(|| Mutex::new(HashSet::new()));

    let Ok(mut guard) = logged.lock() else {
        return true;
    };

    if guard.contains(plugin_id) {
        return false;
    }

    guard.insert(plugin_id.to_owned());
    true
}

pub(crate) fn build(definition: &PluginDefinition) -> Option<Arc<dyn PluginRuntime>> {
    if !matches!(definition.kind, PluginKind::Wasm) {
        return None;
    }

    Some(Arc::new(WasmRuntime {
        definition: definition.clone(),
        instance: Arc::new(Mutex::new(None)),
    }))
}

#[derive(Clone)]
struct WasmRuntime {
    definition: PluginDefinition,
    instance: Arc<Mutex<Option<WasmPluginInstance>>>,
}

#[derive(Debug, Deserialize)]
struct WasmRefreshOutput {
    label: String,
    #[serde(default)]
    tooltip: Option<String>,
    #[serde(default = "default_visible")]
    visible: bool,
    #[serde(default)]
    dropdown: Option<WasmDropdownOutput>,
    #[serde(default)]
    style: Option<WasmStyleOutput>,
}

#[derive(Debug, Deserialize)]
struct WasmStyleOutput {
    #[serde(default)]
    button_class: Option<String>,
    #[serde(default)]
    dropdown_class: Option<String>,
    #[serde(default)]
    css: Option<String>,
    #[serde(default)]
    once: bool,
    #[serde(default)]
    dropdown_height_percent: Option<u8>,
}

#[derive(Debug, Deserialize)]
struct WasmDropdownOutput {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    title_xalign: Option<f32>,
    #[serde(default)]
    empty_label: Option<String>,
    #[serde(default)]
    items: Vec<WasmDropdownItemOutput>,
}

#[derive(Debug, Deserialize)]
struct WasmDropdownItemOutput {
    label: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    action: Option<WasmRowActionOutput>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum WasmRowActionOutput {
    Legacy(String),
    Typed(WasmRowActionTypedOutput),
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
enum WasmRowActionTypedOutput {
    OpenUrl { url: String },
    RunCommand { command: String },
    CopyText { text: String },
    Custom { payload: String },
    RefreshNow,
}

fn default_visible() -> bool {
    true
}

struct WasmPluginInstance {
    store: Store<()>,
    memory: Memory,
    manifest_name: Option<String>,
    manifest_author: Option<String>,
    manifest_version: Option<String>,
    alloc: TypedFunc<i32, i32>,
    init: TypedFunc<(i32, i32), i64>,
    manifest: Option<TypedFunc<(), i64>>,
    refresh: TypedFunc<(), i64>,
    on_action: Option<TypedFunc<i32, i64>>,
    on_row_action: Option<TypedFunc<(i32, i32), i64>>,
}

#[derive(Debug, Deserialize)]
struct WasmManifestOutput {
    id: String,
    version: String,
    abi: i32,
    name: String,
    author: String,
    license: String,
    capabilities: Vec<String>,
    #[serde(default)]
    command_exec_allowed_prefixes: Vec<String>,
}

impl PluginRuntime for WasmRuntime {
    fn css_class(&self) -> &str {
        "plugin-wasm"
    }

    fn action_for(&self, action: PluginAction) -> ClickAction {
        match action {
            PluginAction::LeftClick => self.definition.left_click.clone(),
            PluginAction::RightClick => self.definition.right_click.clone(),
            PluginAction::MiddleClick => self.definition.middle_click.clone(),
            PluginAction::ScrollUp => self.definition.scroll_up.clone(),
            PluginAction::ScrollDown => self.definition.scroll_down.clone(),
        }
    }

    fn refresh(&self) -> Pin<Box<dyn Future<Output = Result<PluginUpdate, String>> + Send + '_>> {
        Box::pin(async move {
            let timeout_ms = self.definition.wasm_timeout_ms.max(1);
            let runtime = self.clone();
            let join = tokio::task::spawn_blocking(move || {
                runtime.with_instance(|instance| instance.refresh())
            });

            timeout(Duration::from_millis(timeout_ms), join)
                .await
                .map_err(|_| String::from("WASM refresh timed out"))?
                .map_err(|error| error.to_string())?
        })
    }

    fn handle_action(
        &self,
        action: PluginAction,
    ) -> Pin<Box<dyn Future<Output = Result<PluginActionResult, String>> + Send + '_>> {
        Box::pin(async move {
            let timeout_ms = self.definition.wasm_timeout_ms.max(1);
            let runtime = self.clone();
            let join = tokio::task::spawn_blocking(move || {
                runtime.with_instance(|instance| instance.handle_action(action))
            });

            timeout(Duration::from_millis(timeout_ms), join)
                .await
                .map_err(|_| String::from("WASM action timed out"))?
                .map_err(|error| error.to_string())?
        })
    }

    fn handle_row_action(
        &self,
        payload: String,
    ) -> Pin<Box<dyn Future<Output = Result<PluginActionResult, String>> + Send + '_>> {
        Box::pin(async move {
            let timeout_ms = self.definition.wasm_timeout_ms.max(1);
            let runtime = self.clone();
            let join = tokio::task::spawn_blocking(move || {
                runtime.with_instance(|instance| instance.handle_row_action(payload))
            });

            timeout(Duration::from_millis(timeout_ms), join)
                .await
                .map_err(|_| String::from("WASM row action timed out"))?
                .map_err(|error| error.to_string())?
        })
    }
}

impl WasmRuntime {
    fn with_instance<T>(
        &self,
        f: impl FnOnce(&mut WasmPluginInstance) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut guard = self
            .instance
            .lock()
            .map_err(|_| String::from("WASM plugin instance mutex poisoned"))?;

        if guard.is_none() {
            let start = std::time::Instant::now();
            let instance = self.create_instance()?;

            if should_log_lazy_init(&self.definition.id) {
                let elapsed = start.elapsed().as_millis();
                let name = instance
                    .manifest_name
                    .clone()
                    .unwrap_or_else(|| self.definition.id.clone());
                let author = instance
                    .manifest_author
                    .clone()
                    .unwrap_or_else(|| String::from("unknown"));
                let version = instance
                    .manifest_version
                    .clone()
                    .unwrap_or_else(|| String::from("unknown"));

                eprintln!(
                    "\n{} Plugin {} {} by {} ({}ms - lazy-loaded)",
                    style("✓").green().bold(),
                    name,
                    version,
                    author,
                    elapsed
                );
            }

            *guard = Some(instance);
        }

        let instance = guard
            .as_mut()
            .ok_or_else(|| String::from("WASM plugin instance unavailable"))?;
        f(instance)
    }

    #[allow(clippy::too_many_lines)]
    fn create_instance(&self) -> Result<WasmPluginInstance, String> {
        let wasm_path = self
            .definition
            .wasm_path
            .as_deref()
            .ok_or("wasm-path is required for kind = 'wasm'")?;
        let wasm_path = expand_path(wasm_path);

        let engine = wasm_engine();
        let module = wasm_module(engine, &wasm_path)?;

        let mut linker = Linker::new(engine);
        let effective_definition = Arc::new(Mutex::new(self.definition.clone()));
        linker
            .func_wrap("wayle", "unix_time_seconds", || -> i64 {
                let now = std::time::SystemTime::now();
                match now.duration_since(std::time::UNIX_EPOCH) {
                    Ok(duration) => duration.as_secs() as i64,
                    Err(_) => 0,
                }
            })
            .map_err(|error| error.to_string())?;
        linker
            .func_wrap(
                "wayle",
                "log_utf8",
                |caller: Caller<'_, ()>, ptr: i32, len: i32| {
                    if let Err(error) = log_guest_utf8(caller, ptr, len) {
                        warn!(error = %error, "wasm plugin log_utf8 failed");
                    }
                },
            )
            .map_err(|error| error.to_string())?;
        let effective_definition_for_import = effective_definition.clone();
        linker
            .func_wrap(
                "wayle",
                "run_command_utf8",
                move |mut caller: Caller<'_, ()>, ptr: i32, len: i32| -> i64 {
                    run_command_utf8_import(&mut caller, ptr, len, &effective_definition_for_import)
                        .unwrap_or(NO_PAYLOAD)
                },
            )
            .map_err(|error| error.to_string())?;

        let mut store = Store::new(engine, ());
        let instance = linker
            .instantiate(&mut store, &module)
            .map_err(|error| error.to_string())?;

        let memory = instance
            .get_memory(&mut store, "memory")
            .ok_or_else(|| String::from("WASM module must export memory"))?;
        let abi_version = instance
            .get_typed_func::<(), i32>(&mut store, "plugin_abi_version")
            .map_err(|_| String::from("WASM module must export plugin_abi_version() -> i32"))?
            .call(&mut store, ())
            .map_err(|error| error.to_string())?;
        if abi_version != PLUGIN_ABI_VERSION {
            return Err(format!(
                "unsupported plugin ABI version {abi_version}, expected {PLUGIN_ABI_VERSION}"
            ));
        }

        let alloc = instance
            .get_typed_func::<i32, i32>(&mut store, "plugin_alloc")
            .map_err(|_| String::from("WASM module must export plugin_alloc(i32) -> i32"))?;
        let init = instance
            .get_typed_func::<(i32, i32), i64>(&mut store, "plugin_init")
            .map_err(|_| String::from("WASM module must export plugin_init(i32, i32) -> i64"))?;
        let manifest = instance
            .get_typed_func::<(), i64>(&mut store, "plugin_manifest")
            .map_err(|_| String::from("WASM module must export plugin_manifest() -> i64"))?;
        let refresh = instance
            .get_typed_func::<(), i64>(&mut store, &self.definition.wasm_refresh_export)
            .map_err(|error| error.to_string())?;
        let on_action = instance
            .get_typed_func::<i32, i64>(&mut store, "plugin_on_action")
            .ok();
        let on_row_action = instance
            .get_typed_func::<(i32, i32), i64>(&mut store, "plugin_on_row_action")
            .ok();

        let mut plugin = WasmPluginInstance {
            store,
            memory,
            manifest_name: None,
            manifest_author: None,
            manifest_version: None,
            alloc,
            init,
            manifest: Some(manifest),
            refresh,
            on_action,
            on_row_action,
        };
        let mut policy = self.definition.clone();
        let manifest = plugin.load_manifest(&self.definition.id)?;
        if policy.capabilities.is_empty() && !manifest.capabilities.is_empty() {
            policy.capabilities = manifest.capabilities.clone();
        }
        if policy.command_exec_allowed_prefixes.is_empty()
            && !manifest.command_exec_allowed_prefixes.is_empty()
        {
            policy.command_exec_allowed_prefixes = manifest.command_exec_allowed_prefixes;
        }
        if let Ok(mut guard) = effective_definition.lock() {
            *guard = policy.clone();
        }

        plugin.init(&policy)?;
        info!(
            plugin_id = %self.definition.id,
            wasm_path = ?self.definition.wasm_path,
            "WASM plugin runtime ready"
        );
        Ok(plugin)
    }
}

impl WasmPluginInstance {
    fn init(&mut self, definition: &PluginDefinition) -> Result<(), String> {
        let config_json = serde_json::to_string(definition).map_err(|error| error.to_string())?;
        let init = self.init.clone();
        let result = self.call_with_string_arg(init, &config_json)?;
        if result != NO_PAYLOAD {
            let update = self.read_payload(result)?;
            debug!(label = %update.label, "wasm plugin emitted init payload");
        }
        Ok(())
    }

    fn load_manifest(&mut self, expected_id: &str) -> Result<WasmManifestOutput, String> {
        let manifest_fn = self
            .manifest
            .clone()
            .ok_or_else(|| String::from("WASM module must export plugin_manifest() -> i64"))?;

        let packed = manifest_fn
            .call(&mut self.store, ())
            .map_err(|error| error.to_string())?;
        if packed == NO_PAYLOAD {
            return Err(String::from("WASM plugin manifest payload missing"));
        }

        let manifest = self.read_manifest_payload(packed)?;
        self.manifest_name = Some(manifest.name.clone());
        self.manifest_author = Some(manifest.author.clone());
        self.manifest_version = Some(manifest.version.clone());
        if manifest.id != expected_id {
            warn!(
                expected_plugin_id = %expected_id,
                manifest_id = %manifest.id,
                "plugin manifest id does not match configured plugin id"
            );
        }

        info!(
            plugin_id = %expected_id,
            manifest_version = %manifest.version,
            manifest_abi = manifest.abi,
            manifest_name = %manifest.name,
            manifest_author = %manifest.author,
            manifest_license = %manifest.license,
            manifest_capabilities = ?manifest.capabilities,
            "plugin manifest loaded"
        );

        Ok(manifest)
    }

    fn refresh(&mut self) -> Result<PluginUpdate, String> {
        let packed = self
            .refresh
            .call(&mut self.store, ())
            .map_err(|error| error.to_string())?;
        self.read_payload(packed)
    }

    fn handle_action(&mut self, action: PluginAction) -> Result<PluginActionResult, String> {
        let Some(on_action) = &self.on_action else {
            return Ok(PluginActionResult::Unhandled);
        };

        let packed = on_action
            .call(&mut self.store, encode_action(action))
            .map_err(|error| error.to_string())?;
        if packed == NO_PAYLOAD {
            return Ok(PluginActionResult::Unhandled);
        }
        Ok(PluginActionResult::Consumed(Some(
            self.read_payload(packed)?,
        )))
    }

    fn handle_row_action(&mut self, payload: String) -> Result<PluginActionResult, String> {
        let Some(on_row_action) = self.on_row_action.clone() else {
            return Ok(PluginActionResult::Unhandled);
        };

        let packed = self.call_with_string_arg(on_row_action, &payload)?;
        if packed == NO_PAYLOAD {
            return Ok(PluginActionResult::Unhandled);
        }
        Ok(PluginActionResult::Consumed(Some(
            self.read_payload(packed)?,
        )))
    }

    fn call_with_string_arg(
        &mut self,
        func: TypedFunc<(i32, i32), i64>,
        input: &str,
    ) -> Result<i64, String> {
        let len = i32::try_from(input.len()).map_err(|_| String::from("input too large"))?;
        let ptr = self
            .alloc
            .call(&mut self.store, len)
            .map_err(|error| error.to_string())?;

        self.write_memory(ptr as usize, input.as_bytes())?;
        func.call(&mut self.store, (ptr, len))
            .map_err(|error| error.to_string())
    }

    fn write_memory(&mut self, ptr: usize, bytes: &[u8]) -> Result<(), String> {
        let memory = self.memory.data_mut(&mut self.store);
        let end = ptr
            .checked_add(bytes.len())
            .ok_or_else(|| String::from("WASM write range overflow"))?;
        if end > memory.len() {
            return Err(String::from("WASM write range out of memory bounds"));
        }
        memory[ptr..end].copy_from_slice(bytes);
        Ok(())
    }

    fn read_payload(&mut self, packed: i64) -> Result<PluginUpdate, String> {
        if packed == NO_PAYLOAD {
            return Err(String::from("WASM payload missing"));
        }

        let ptr = ((packed >> 32) as u32) as usize;
        let len = (packed as u32) as usize;
        if len == 0 {
            return Err(String::from("WASM payload length is zero"));
        }

        let bytes = self.memory.data(&self.store);
        let end = ptr
            .checked_add(len)
            .ok_or_else(|| String::from("WASM payload range overflow"))?;
        if end > bytes.len() {
            return Err(String::from("WASM payload range out of memory bounds"));
        }

        let payload = std::str::from_utf8(&bytes[ptr..end]).map_err(|error| error.to_string())?;
        decode_payload(payload)
    }

    fn read_manifest_payload(&mut self, packed: i64) -> Result<WasmManifestOutput, String> {
        let ptr = ((packed >> 32) as u32) as usize;
        let len = (packed as u32) as usize;
        if len == 0 {
            return Err(String::from("WASM manifest payload length is zero"));
        }

        let bytes = self.memory.data(&self.store);
        let end = ptr
            .checked_add(len)
            .ok_or_else(|| String::from("WASM manifest payload range overflow"))?;
        if end > bytes.len() {
            return Err(String::from(
                "WASM manifest payload range out of memory bounds",
            ));
        }

        let payload = std::str::from_utf8(&bytes[ptr..end]).map_err(|error| error.to_string())?;
        serde_json::from_str(payload).map_err(|error| error.to_string())
    }
}

fn decode_payload(payload: &str) -> Result<PluginUpdate, String> {
    if let Ok(decoded) = serde_json::from_str::<WasmRefreshOutput>(payload) {
        return Ok(PluginUpdate {
            label: decoded.label,
            tooltip: decoded.tooltip,
            visible: decoded.visible,
            dropdown: decoded.dropdown.map(|dropdown| PluginDropdown {
                title: dropdown.title,
                title_xalign: dropdown.title_xalign,
                empty_label: dropdown.empty_label,
                items: dropdown
                    .items
                    .into_iter()
                    .map(|item| PluginDropdownItem {
                        label: item.label,
                        description: item.description,
                        action: item.action.map(map_row_action),
                    })
                    .collect(),
            }),
            style: decoded.style.map(|style| PluginStyle {
                button_class: style.button_class,
                dropdown_class: style.dropdown_class,
                css: style.css,
                once: style.once,
                dropdown_height_percent: style.dropdown_height_percent,
            }),
        });
    }

    Ok(PluginUpdate {
        label: payload.trim().to_owned(),
        tooltip: None,
        visible: true,
        dropdown: None,
        style: None,
    })
}

fn map_row_action(action: WasmRowActionOutput) -> PluginRowAction {
    match action {
        WasmRowActionOutput::Legacy(command) => PluginRowAction::LegacyCommand(command),
        WasmRowActionOutput::Typed(WasmRowActionTypedOutput::OpenUrl { url }) => {
            PluginRowAction::OpenUrl { url }
        }
        WasmRowActionOutput::Typed(WasmRowActionTypedOutput::RunCommand { command }) => {
            PluginRowAction::RunCommand { command }
        }
        WasmRowActionOutput::Typed(WasmRowActionTypedOutput::CopyText { text }) => {
            PluginRowAction::CopyText { text }
        }
        WasmRowActionOutput::Typed(WasmRowActionTypedOutput::Custom { payload }) => {
            PluginRowAction::Custom { payload }
        }
        WasmRowActionOutput::Typed(WasmRowActionTypedOutput::RefreshNow) => {
            PluginRowAction::RefreshNow
        }
    }
}

fn encode_action(action: PluginAction) -> i32 {
    match action {
        PluginAction::LeftClick => 1,
        PluginAction::RightClick => 2,
        PluginAction::MiddleClick => 3,
        PluginAction::ScrollUp => 4,
        PluginAction::ScrollDown => 5,
    }
}

fn log_guest_utf8(mut caller: Caller<'_, ()>, ptr: i32, len: i32) -> Result<(), String> {
    let Some(export) = caller.get_export("memory") else {
        return Err(String::from("guest memory export missing"));
    };
    let memory = export
        .into_memory()
        .ok_or_else(|| String::from("guest memory export is not a memory"))?;
    let data = memory.data(&caller);

    let start = usize::try_from(ptr).map_err(|_| String::from("negative log pointer"))?;
    let len = usize::try_from(len).map_err(|_| String::from("negative log length"))?;
    let end = start
        .checked_add(len)
        .ok_or_else(|| String::from("guest log range overflow"))?;
    if end > data.len() {
        return Err(String::from("guest log range out of bounds"));
    }

    let text = std::str::from_utf8(&data[start..end]).map_err(|error| error.to_string())?;
    debug!(target: "wayle_plugin", message = %text, "wasm plugin log");
    Ok(())
}

fn run_command_utf8_import(
    caller: &mut Caller<'_, ()>,
    ptr: i32,
    len: i32,
    effective_definition: &Arc<Mutex<PluginDefinition>>,
) -> Result<i64, String> {
    let definition = effective_definition
        .lock()
        .map_err(|_| String::from("effective policy mutex poisoned"))?;

    if !definition.has_capability("command.exec") {
        return write_guest_utf8(
            caller,
            "__WAYLE_ERROR__:WAYLE_ERR_PERMISSION_DENIED:missing capability command.exec",
        );
    }

    let command = read_guest_utf8(caller, ptr, len)?;
    if !definition.is_command_allowed(&command) {
        return write_guest_utf8(
            caller,
            "__WAYLE_ERROR__:WAYLE_ERR_POLICY_VIOLATION:command not allowed by command-exec-allowed-prefixes",
        );
    }

    match run_shell_capture(command.as_str()) {
        Ok(output) => write_guest_utf8(caller, output.as_str()),
        Err(error) => write_guest_utf8(
            caller,
            &format!("__WAYLE_ERROR__:WAYLE_ERR_UNAVAILABLE:{error}"),
        ),
    }
}

fn run_shell_capture(command: &str) -> Result<String, String> {
    let output = Command::new("sh")
        .arg("-c")
        .arg(command)
        .output()
        .map_err(|error| error.to_string())?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        if stderr.is_empty() {
            return Err(format!(
                "command exited with status {:?}",
                output.status.code()
            ));
        }
        return Err(stderr.to_owned());
    }

    String::from_utf8(output.stdout).map_err(|error| error.to_string())
}

fn read_guest_utf8(caller: &mut Caller<'_, ()>, ptr: i32, len: i32) -> Result<String, String> {
    let memory = lookup_memory(caller)?;
    let bytes = memory.data(caller);

    let start = usize::try_from(ptr).map_err(|_| String::from("negative pointer"))?;
    let len = usize::try_from(len).map_err(|_| String::from("negative length"))?;
    let end = start
        .checked_add(len)
        .ok_or_else(|| String::from("range overflow"))?;
    if end > bytes.len() {
        return Err(String::from("range out of bounds"));
    }

    std::str::from_utf8(&bytes[start..end])
        .map(|value| value.to_owned())
        .map_err(|error| error.to_string())
}

fn write_guest_utf8(caller: &mut Caller<'_, ()>, payload: &str) -> Result<i64, String> {
    let Some(export) = caller.get_export("plugin_alloc") else {
        return Err(String::from("guest export plugin_alloc missing"));
    };
    let alloc = export
        .into_func()
        .ok_or_else(|| String::from("guest export plugin_alloc is not a function"))?
        .typed::<i32, i32>(&mut *caller)
        .map_err(|error| error.to_string())?;

    let payload_len =
        i32::try_from(payload.len()).map_err(|_| String::from("payload too large"))?;
    let alloc_len = if payload_len == 0 { 1 } else { payload_len };
    let ptr = alloc
        .call(&mut *caller, alloc_len)
        .map_err(|error| error.to_string())?;

    let memory = lookup_memory(caller)?;
    let bytes = memory.data_mut(caller);
    let start = usize::try_from(ptr).map_err(|_| String::from("negative pointer"))?;
    let len = usize::try_from(alloc_len).map_err(|_| String::from("payload too large"))?;
    let end = start
        .checked_add(len)
        .ok_or_else(|| String::from("range overflow"))?;
    if end > bytes.len() {
        return Err(String::from("range out of bounds"));
    }

    if payload_len == 0 {
        bytes[start] = 0;
    } else {
        bytes[start..end].copy_from_slice(payload.as_bytes());
    }
    Ok(pack_payload(ptr, alloc_len))
}

fn lookup_memory(caller: &mut Caller<'_, ()>) -> Result<Memory, String> {
    let Some(export) = caller.get_export("memory") else {
        return Err(String::from("guest memory export missing"));
    };
    export
        .into_memory()
        .ok_or_else(|| String::from("guest memory export is not a memory"))
}

fn pack_payload(ptr: i32, len: i32) -> i64 {
    ((ptr as i64) << 32) | i64::from(len as u32)
}

/// Expands `~` to the home directory and `$VAR` / `${VAR}` to environment
/// variable values. Unknown variables are left as-is.
fn expand_path(raw: &str) -> PathBuf {
    let expanded = if let Some(rest) = raw.strip_prefix("~/") {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{home}/{rest}")
    } else if raw == "~" {
        std::env::var("HOME").unwrap_or_else(|_| raw.to_owned())
    } else {
        raw.to_owned()
    };

    // Expand $VAR and ${VAR}
    let mut result = String::with_capacity(expanded.len());
    let mut chars = expanded.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '$' {
            result.push(ch);
            continue;
        }
        let braced = chars.peek() == Some(&'{');
        if braced {
            chars.next();
        }
        let var_name: String = chars
            .by_ref()
            .take_while(|c| {
                if braced {
                    *c != '}'
                } else {
                    c.is_alphanumeric() || *c == '_'
                }
            })
            .collect();
        match std::env::var(&var_name) {
            Ok(value) => result.push_str(&value),
            Err(_) => {
                if braced {
                    result.push_str(&format!("${{{var_name}}}"));
                } else {
                    result.push_str(&format!("${var_name}"));
                }
            }
        }
    }

    PathBuf::from(result)
}
