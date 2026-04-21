use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use libloading::Library;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_uchar};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::host::controller::{LuaRuntimeSpaceControllerBridge, controller_space_id_for_binding};
use crate::host::database::{
    LuaRuntimeDatabaseCallbackMode, LuaRuntimeDatabaseProviderMode, RuntimeDatabaseBindingContext,
    RuntimeDatabaseKind, RuntimeLanceDbProviderAction, RuntimeLanceDbProviderRequest,
    dispatch_lancedb_provider_request, has_lancedb_provider_callback_for_mode,
};
use crate::lua_skill::{SkillLanceDbLogLevel, SkillLanceDbMeta};
use crate::runtime_logging::{info as log_info, warn as log_warn};
use crate::runtime_options::LuaRuntimeHostOptions;
use vldb_controller_client::ControllerLanceDbEnableRequest;

/// Forward declaration of the FFI runtime handle used only for raw cross-library pointers.
/// FFI 运行时句柄前置声明，仅用于跨动态库传递裸指针。
#[repr(C)]
struct VldbLancedbRuntimeHandle {
    _private: [u8; 0],
}

/// Forward declaration of the FFI engine handle used only for raw cross-library pointers.
/// FFI 引擎句柄前置声明，仅用于跨动态库传递裸指针。
#[repr(C)]
struct VldbLancedbEngineHandle {
    _private: [u8; 0],
}

/// Raw LanceDB FFI byte-buffer definition kept identical to the exported dynamic-library header.
/// LanceDB FFI 原始字节缓冲区定义，需与动态库头文件保持一致。
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct VldbLancedbByteBuffer {
    data: *mut c_uchar,
    len: usize,
    /// Original allocation capacity used only by the dynamic library when reconstructing the Vec during free.
    /// 原始分配容量，仅供动态库在释放时恢复 Vec 布局。
    cap: usize,
}

/// LanceDB FFI runtime options that must stay ABI-compatible with the exported header.
/// LanceDB FFI 运行时选项，需与导出的头文件严格对齐。
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct VldbLancedbRuntimeOptions {
    default_db_path: *const c_char,
    db_root: *const c_char,
    read_consistency_interval_ms: u64,
    has_read_consistency_interval: u8,
    max_upsert_payload: usize,
    max_search_limit: usize,
    max_concurrent_requests: usize,
}

type RuntimeOptionsDefaultFn = unsafe extern "C" fn() -> VldbLancedbRuntimeOptions;
type RuntimeCreateFn =
    unsafe extern "C" fn(VldbLancedbRuntimeOptions) -> *mut VldbLancedbRuntimeHandle;
type RuntimeDestroyFn = unsafe extern "C" fn(*mut VldbLancedbRuntimeHandle);
type RuntimeOpenDefaultEngineFn =
    unsafe extern "C" fn(*mut VldbLancedbRuntimeHandle) -> *mut VldbLancedbEngineHandle;
type RuntimeDatabasePathForNameFn =
    unsafe extern "C" fn(*mut VldbLancedbRuntimeHandle, *const c_char) -> *mut c_char;
type EngineCreateTableJsonFn =
    unsafe extern "C" fn(*mut VldbLancedbEngineHandle, *const c_char) -> *mut c_char;
type EngineVectorUpsertFn = unsafe extern "C" fn(
    *mut VldbLancedbEngineHandle,
    *const c_char,
    *const u8,
    usize,
) -> *mut c_char;
type EngineVectorSearchFn = unsafe extern "C" fn(
    *mut VldbLancedbEngineHandle,
    *const c_char,
    *mut VldbLancedbByteBuffer,
) -> *mut c_char;
type EngineDeleteJsonFn =
    unsafe extern "C" fn(*mut VldbLancedbEngineHandle, *const c_char) -> *mut c_char;
type EngineDropTableJsonFn =
    unsafe extern "C" fn(*mut VldbLancedbEngineHandle, *const c_char) -> *mut c_char;
type EngineDestroyFn = unsafe extern "C" fn(*mut VldbLancedbEngineHandle);
type BytesFreeFn = unsafe extern "C" fn(VldbLancedbByteBuffer);
type StringFreeFn = unsafe extern "C" fn(*mut c_char);
type LastErrorMessageFn = unsafe extern "C" fn() -> *const c_char;
type ClearLastErrorFn = unsafe extern "C" fn();

/// Loaded LanceDB FFI API table that owns the dynamic-library lifetime and exported function pointers.
/// 已加载的 LanceDB FFI API 表，负责持有动态库生命周期与导出函数指针。
struct LoadedLanceDbApi {
    _library: Library,
    library_path: PathBuf,
    runtime_options_default: RuntimeOptionsDefaultFn,
    runtime_create: RuntimeCreateFn,
    runtime_destroy: RuntimeDestroyFn,
    runtime_open_default_engine: RuntimeOpenDefaultEngineFn,
    runtime_database_path_for_name: RuntimeDatabasePathForNameFn,
    engine_create_table_json: EngineCreateTableJsonFn,
    engine_vector_upsert: EngineVectorUpsertFn,
    engine_vector_search: EngineVectorSearchFn,
    engine_delete_json: EngineDeleteJsonFn,
    engine_drop_table_json: EngineDropTableJsonFn,
    engine_destroy: EngineDestroyFn,
    bytes_free: BytesFreeFn,
    string_free: StringFreeFn,
    last_error_message: LastErrorMessageFn,
    clear_last_error: ClearLastErrorFn,
}

/// The loaded library handle and copied function pointers stay immutable after initialization, while callers serialize use via outer mutexes.
/// 动态库句柄与函数指针在初始化后只读，调用端通过外层互斥量串行化访问。
unsafe impl Send for LoadedLanceDbApi {}
unsafe impl Sync for LoadedLanceDbApi {}

impl LoadedLanceDbApi {
    /// Load the LanceDB dynamic library using host conventions, preferring an explicit environment variable and runtime libs directories.
    /// 按宿主约定加载 LanceDB 动态库，优先查找显式环境变量与运行时 libs 目录。
    fn load(library_path: &Path) -> Result<Self, String> {
        if !library_path.exists() {
            return Err(format!(
                "LanceDB dynamic library path does not exist: {}",
                library_path.display()
            ));
        }

        let library = unsafe { Library::new(library_path) }.map_err(|error| {
            format!(
                "failed to load {}: {}: {}",
                library_path.display(),
                error,
                error
            )
        })?;
        unsafe { Self::from_library(library_path.to_path_buf(), library) }
    }

    /// Copy all required exported function pointers from an opened dynamic library and keep the library handle alive.
    /// 从已打开的动态库中复制需要的函数指针，并保留库句柄防止提前卸载。
    unsafe fn from_library(library_path: PathBuf, library: Library) -> Result<Self, String> {
        macro_rules! load_symbol {
            ($name:literal, $ty:ty) => {{
                unsafe {
                    *library
                        .get::<$ty>(concat!($name, "\0").as_bytes())
                        .map_err(|error| {
                            format!(
                                "failed to load symbol {} from {}: {}",
                                $name,
                                library_path.display(),
                                error
                            )
                        })?
                }
            }};
        }

        Ok(Self {
            runtime_options_default: load_symbol!(
                "vldb_lancedb_runtime_options_default",
                RuntimeOptionsDefaultFn
            ),
            runtime_create: load_symbol!("vldb_lancedb_runtime_create", RuntimeCreateFn),
            runtime_destroy: load_symbol!("vldb_lancedb_runtime_destroy", RuntimeDestroyFn),
            runtime_open_default_engine: load_symbol!(
                "vldb_lancedb_runtime_open_default_engine",
                RuntimeOpenDefaultEngineFn
            ),
            runtime_database_path_for_name: load_symbol!(
                "vldb_lancedb_runtime_database_path_for_name",
                RuntimeDatabasePathForNameFn
            ),
            engine_create_table_json: load_symbol!(
                "vldb_lancedb_engine_create_table_json",
                EngineCreateTableJsonFn
            ),
            engine_vector_upsert: load_symbol!(
                "vldb_lancedb_engine_vector_upsert",
                EngineVectorUpsertFn
            ),
            engine_vector_search: load_symbol!(
                "vldb_lancedb_engine_vector_search",
                EngineVectorSearchFn
            ),
            engine_delete_json: load_symbol!("vldb_lancedb_engine_delete_json", EngineDeleteJsonFn),
            engine_drop_table_json: load_symbol!(
                "vldb_lancedb_engine_drop_table_json",
                EngineDropTableJsonFn
            ),
            engine_destroy: load_symbol!("vldb_lancedb_engine_destroy", EngineDestroyFn),
            bytes_free: load_symbol!("vldb_lancedb_bytes_free", BytesFreeFn),
            string_free: load_symbol!("vldb_lancedb_string_free", StringFreeFn),
            last_error_message: load_symbol!("vldb_lancedb_last_error_message", LastErrorMessageFn),
            clear_last_error: load_symbol!("vldb_lancedb_clear_last_error", ClearLastErrorFn),
            _library: library,
            library_path,
        })
    }

    /// Read the latest FFI error text and return it as a stable Rust string.
    /// 读取最近一次 FFI 调用错误文本，并返回稳定 Rust 字符串。
    fn take_last_error_message(&self) -> String {
        unsafe {
            let ptr = (self.last_error_message)();
            let text = if ptr.is_null() {
                "unknown LanceDB host error".to_string()
            } else {
                CStr::from_ptr(ptr).to_string_lossy().to_string()
            };
            (self.clear_last_error)();
            text
        }
    }

    /// Convert a dynamic-library allocated string into a Rust `String` and free the original allocation.
    /// 释放由动态库分配的字符串并转成 Rust `String`。
    fn take_owned_string(&self, ptr: *mut c_char) -> Result<String, String> {
        if ptr.is_null() {
            return Err(self.take_last_error_message());
        }

        unsafe {
            let text = CStr::from_ptr(ptr).to_string_lossy().to_string();
            (self.string_free)(ptr);
            Ok(text)
        }
    }

    /// Convert a dynamic-library allocated byte buffer into a Rust `Vec<u8>` and free the original allocation.
    /// 释放由动态库分配的字节缓冲区并转成 Rust `Vec<u8>`。
    fn take_owned_bytes(&self, buffer: VldbLancedbByteBuffer) -> Vec<u8> {
        if buffer.data.is_null() || buffer.len == 0 {
            return Vec::new();
        }

        unsafe {
            let bytes = std::slice::from_raw_parts(buffer.data, buffer.len).to_vec();
            (self.bytes_free)(buffer);
            bytes
        }
    }
}

/// One skill-scoped LanceDB handle set whose lifetime is managed entirely by the host.
/// 单个 skill 的 LanceDB 句柄集合，由宿主管理其生命周期。
struct SkillHandleState {
    runtime: *mut VldbLancedbRuntimeHandle,
    engine: *mut VldbLancedbEngineHandle,
}

/// Stable provider integration mode used by one LanceDB skill binding.
/// 单个 LanceDB skill 绑定所使用的稳定 provider 集成模式。
#[derive(Clone, Copy, PartialEq, Eq)]
enum LanceDbBindingMode {
    DynamicLibrary,
    HostCallback,
    SpaceController,
}

/// FFI handles are accessed only behind host-side mutexes, and cross-thread sharing is controlled centrally by the host.
/// FFI 句柄只通过宿主互斥量串行访问，跨线程共享由宿主统一控制。
unsafe impl Send for SkillHandleState {}

/// Database context bound to one LanceDB-enabled skill.
/// 某个启用 LanceDB 的 skill 所绑定的数据库上下文。
pub struct LanceDbSkillBinding {
    api: Option<Arc<LoadedLanceDbApi>>,
    skill_name: String,
    skill_dir_name: String,
    database_path: String,
    config: SkillLanceDbMeta,
    provider_mode: LanceDbBindingMode,
    callback_mode: LuaRuntimeDatabaseCallbackMode,
    handles: Option<Mutex<SkillHandleState>>,
    controller: Option<Arc<LuaRuntimeSpaceControllerBridge>>,
    provider_binding: RuntimeDatabaseBindingContext,
}

impl LanceDbSkillBinding {
    /// Return the stable LanceDB status payload for the current skill; the shape stays stable whether enabled or disabled.
    /// 返回当前 skill 的稳定 LanceDB 状态信息；无论启用与否，返回结构都应稳定。
    pub fn status_json(&self) -> Value {
        json!({
            "enabled": true,
            "initialized": true,
            "skill_name": self.skill_name,
            "skill_dir_name": self.skill_dir_name,
            "database_path": self.database_path,
            "integration_mode": self.integration_mode_name(),
            "library_path": self.api.as_ref().map(|api| api.library_path.to_string_lossy().to_string()).unwrap_or_default(),
            "space_label": self.provider_binding.space_label,
            "root_name": self.provider_binding.root_name,
            "binding_tag": self.provider_binding.binding_tag,
            "space_root": self.provider_binding.space_root,
            "default_database_path": self.provider_binding.default_database_path,
            "log_level": self.config.log_level.as_str(),
            "slow_log_enabled": self.config.slow_log_enabled,
            "slow_log_threshold_ms": self.config.slow_log_threshold_ms,
        })
    }

    /// Return base information about the LanceDB instance bound to the current skill for Lua and diagnostics.
    /// 返回当前 skill 所绑定 LanceDB 的基础信息，供 Lua 或诊断输出使用。
    pub fn info_json(&self) -> Value {
        self.status_json()
    }

    /// Execute create-table using the host-defined JSON input shape.
    /// 执行建表操作，输入必须符合宿主约定的 JSON 结构。
    pub fn create_table_json(&self, input: &Value) -> Result<Value, String> {
        if self.is_space_controller_mode() {
            self.log_info("create_table", None);
            let started_at = Instant::now();
            let bridge = self.controller_bridge()?;
            let space_id = self.controller_space_id();
            let binding_id = self.controller_binding_id();
            let result = bridge.block_on(bridge.client().create_lancedb_table(
                space_id,
                binding_id,
                serde_json::to_string(input).map_err(|error| error.to_string())?,
            ))?;
            self.log_if_slow("create_table", started_at, None);
            return Ok(json!({ "message": result.message }));
        }
        if self.is_host_provider_mode() {
            return self
                .dispatch_host_provider(RuntimeLanceDbProviderAction::CreateTable, input)
                .map(|result| result.meta);
        }
        self.call_json_string("create_table", input, |api, state, input_ptr| unsafe {
            (api.engine_create_table_json)(state.engine, input_ptr)
        })
    }

    /// Execute vector upsert; callers must provide an already encoded raw payload.
    /// 执行向量写入；调用方负责提供已经编码好的原始载荷。
    pub fn vector_upsert_json(&self, input: &Value, data: &[u8]) -> Result<Value, String> {
        if self.is_space_controller_mode() {
            self.log_info(
                "vector_upsert",
                Some(format!("payload_bytes={}", data.len())),
            );
            let started_at = Instant::now();
            let bridge = self.controller_bridge()?;
            let space_id = self.controller_space_id();
            let binding_id = self.controller_binding_id();
            let result = bridge.block_on(bridge.client().upsert_lancedb(
                space_id,
                binding_id,
                serde_json::to_string(input).map_err(|error| error.to_string())?,
                data.to_vec(),
            ))?;
            self.log_if_slow(
                "vector_upsert",
                started_at,
                Some(format!("payload_bytes={}", data.len())),
            );
            return Ok(json!({
                "message": result.message,
                "version": result.version,
                "input_rows": result.input_rows,
                "inserted_rows": result.inserted_rows,
                "updated_rows": result.updated_rows,
                "deleted_rows": result.deleted_rows,
            }));
        }
        if self.is_host_provider_mode() {
            let mut host_input = input.clone();
            if let Some(object) = host_input.as_object_mut() {
                object.insert(
                    "data_base64".to_string(),
                    Value::String(BASE64_STANDARD.encode(data)),
                );
            }
            return self
                .dispatch_host_provider(RuntimeLanceDbProviderAction::VectorUpsert, &host_input)
                .map(|result| result.meta);
        }
        let api = self.api_ref();
        let input_text = serde_json::to_string(input).map_err(|error| error.to_string())?;
        let input_cstr = CString::new(input_text)
            .map_err(|_| "input json contains interior NUL bytes".to_string())?;
        self.log_info(
            "vector_upsert",
            Some(format!("payload_bytes={}", data.len())),
        );
        let started_at = Instant::now();
        let guard = self.lock_handles()?;
        unsafe {
            let response = (api.engine_vector_upsert)(
                guard.engine,
                input_cstr.as_ptr(),
                data.as_ptr(),
                data.len(),
            );
            let text = match api.take_owned_string(response) {
                Ok(text) => text,
                Err(error) => {
                    drop(guard);
                    self.log_warning("vector_upsert", &error);
                    return Err(error);
                }
            };
            let value = serde_json::from_str(&text).map_err(|error| {
                format!("failed to parse LanceDB upsert response JSON: {}", error)
            })?;
            drop(guard);
            self.log_if_slow(
                "vector_upsert",
                started_at,
                Some(format!("payload_bytes={}", data.len())),
            );
            Ok(value)
        }
    }

    /// Execute vector search and return both metadata JSON and raw result bytes.
    /// 执行向量检索并返回元信息 JSON 与原始结果字节。
    pub fn vector_search_json(&self, input: &Value) -> Result<(Value, Vec<u8>), String> {
        if self.is_space_controller_mode() {
            self.log_info("vector_search", None);
            let started_at = Instant::now();
            let bridge = self.controller_bridge()?;
            let space_id = self.controller_space_id();
            let binding_id = self.controller_binding_id();
            let result = bridge.block_on(bridge.client().search_lancedb(
                space_id,
                binding_id,
                serde_json::to_string(input).map_err(|error| error.to_string())?,
            ))?;
            self.log_if_slow(
                "vector_search",
                started_at,
                Some(format!("result_bytes={}", result.data.len())),
            );
            return Ok((
                json!({
                    "message": result.message,
                    "format": result.format,
                    "rows": result.rows,
                }),
                result.data,
            ));
        }
        if self.is_host_provider_mode() {
            return self
                .dispatch_host_provider(RuntimeLanceDbProviderAction::VectorSearch, input)
                .map(|result| (result.meta, result.bytes));
        }
        let api = self.api_ref();
        let input_text = serde_json::to_string(input).map_err(|error| error.to_string())?;
        let input_cstr = CString::new(input_text)
            .map_err(|_| "input json contains interior NUL bytes".to_string())?;
        self.log_info("vector_search", None);
        let started_at = Instant::now();
        let guard = self.lock_handles()?;
        let mut buffer = VldbLancedbByteBuffer {
            data: ptr::null_mut(),
            len: 0,
            cap: 0,
        };
        unsafe {
            let response =
                (api.engine_vector_search)(guard.engine, input_cstr.as_ptr(), &mut buffer);
            let text = match api.take_owned_string(response) {
                Ok(text) => text,
                Err(error) => {
                    drop(guard);
                    self.log_warning("vector_search", &error);
                    return Err(error);
                }
            };
            let meta: Value = serde_json::from_str(&text).map_err(|error| {
                format!("failed to parse LanceDB search response JSON: {}", error)
            })?;
            let bytes = api.take_owned_bytes(buffer);
            drop(guard);
            self.log_if_slow(
                "vector_search",
                started_at,
                Some(format!("result_bytes={}", bytes.len())),
            );
            Ok((meta, bytes))
        }
    }

    /// Execute delete.
    /// 执行删除操作。
    pub fn delete_json(&self, input: &Value) -> Result<Value, String> {
        if self.is_space_controller_mode() {
            self.log_info("delete", None);
            let started_at = Instant::now();
            let bridge = self.controller_bridge()?;
            let space_id = self.controller_space_id();
            let binding_id = self.controller_binding_id();
            let result = bridge.block_on(bridge.client().delete_lancedb(
                space_id,
                binding_id,
                serde_json::to_string(input).map_err(|error| error.to_string())?,
            ))?;
            self.log_if_slow("delete", started_at, None);
            return Ok(json!({
                "message": result.message,
                "version": result.version,
                "deleted_rows": result.deleted_rows,
            }));
        }
        if self.is_host_provider_mode() {
            return self
                .dispatch_host_provider(RuntimeLanceDbProviderAction::Delete, input)
                .map(|result| result.meta);
        }
        self.call_json_string("delete", input, |api, state, input_ptr| unsafe {
            (api.engine_delete_json)(state.engine, input_ptr)
        })
    }

    /// Execute drop-table.
    /// 执行删表操作。
    pub fn drop_table_json(&self, input: &Value) -> Result<Value, String> {
        if self.is_space_controller_mode() {
            self.log_info("drop_table", None);
            let started_at = Instant::now();
            let bridge = self.controller_bridge()?;
            let space_id = self.controller_space_id();
            let binding_id = self.controller_binding_id();
            let result = bridge.block_on(bridge.client().drop_lancedb_table(
                space_id,
                binding_id,
                require_string_field(input, "table_name")?.to_string(),
            ))?;
            self.log_if_slow("drop_table", started_at, None);
            return Ok(json!({ "message": result.message }));
        }
        if self.is_host_provider_mode() {
            return self
                .dispatch_host_provider(RuntimeLanceDbProviderAction::DropTable, input)
                .map(|result| result.meta);
        }
        self.call_json_string("drop_table", input, |api, state, input_ptr| unsafe {
            (api.engine_drop_table_json)(state.engine, input_ptr)
        })
    }

    /// Execute an FFI call that maps a JSON input into a JSON-string response.
    /// 统一执行“输入 JSON -> 返回 JSON 字符串”的 FFI 调用。
    fn call_json_string<F>(
        &self,
        operation: &str,
        input: &Value,
        invoke: F,
    ) -> Result<Value, String>
    where
        F: Fn(&LoadedLanceDbApi, &SkillHandleState, *const c_char) -> *mut c_char,
    {
        let input_text = serde_json::to_string(input).map_err(|error| error.to_string())?;
        let input_cstr = CString::new(input_text)
            .map_err(|_| "input json contains interior NUL bytes".to_string())?;
        self.log_info(operation, None);
        let started_at = Instant::now();
        let api = self.api_ref();
        let guard = self.lock_handles()?;
        let response = invoke(api, &guard, input_cstr.as_ptr());
        let text = match api.take_owned_string(response) {
            Ok(text) => text,
            Err(error) => {
                drop(guard);
                self.log_warning(operation, &error);
                return Err(error);
            }
        };
        let value = serde_json::from_str(&text)
            .map_err(|error| format!("failed to parse LanceDB response JSON: {}", error))?;
        drop(guard);
        self.log_if_slow(operation, started_at, None);
        Ok(value)
    }

    /// Emit regular informational logs according to the skill-scoped log policy.
    /// 按 skill 配置输出普通信息级日志。
    fn log_info(&self, operation: &str, extra: Option<String>) {
        if self.config.log_level == SkillLanceDbLogLevel::Info {
            match extra {
                Some(extra) => log_info(format!(
                    "[LanceDb:info] skill={} db={} op={} {}",
                    self.skill_name, self.skill_dir_name, operation, extra
                )),
                None => log_info(format!(
                    "[LanceDb:info] skill={} db={} op={}",
                    self.skill_name, self.skill_dir_name, operation
                )),
            }
        }
    }

    /// Emit a slow-operation warning according to the slow-log policy; this is independent from regular log verbosity.
    /// 按慢日志配置输出耗时告警；该日志与普通日志开关独立。
    fn log_if_slow(&self, operation: &str, started_at: Instant, extra: Option<String>) {
        if !self.config.slow_log_enabled {
            return;
        }

        let elapsed_ms = started_at.elapsed().as_millis() as u64;
        if elapsed_ms < self.config.slow_log_threshold_ms {
            return;
        }

        match extra {
            Some(extra) => log_info(format!(
                "[LanceDb:slow] skill={} db={} op={} elapsed_ms={} {}",
                self.skill_name, self.skill_dir_name, operation, elapsed_ms, extra
            )),
            None => log_info(format!(
                "[LanceDb:slow] skill={} db={} op={} elapsed_ms={}",
                self.skill_name, self.skill_dir_name, operation, elapsed_ms
            )),
        }
    }

    /// Emit warning-level logs according to the skill policy, usually for FFI call failures or host-detected anomalies.
    /// 按 skill 配置输出告警级日志，通常用于 FFI 调用失败或宿主检测到的异常情况。
    fn log_warning(&self, operation: &str, message: &str) {
        if matches!(
            self.config.log_level,
            SkillLanceDbLogLevel::Info | SkillLanceDbLogLevel::Warning
        ) {
            log_warn(format!(
                "[LanceDb:warn] skill={} db={} op={} message={}",
                self.skill_name, self.skill_dir_name, operation, message
            ));
        }
    }

    /// Return whether the current binding dispatches requests into one host provider.
    /// 返回当前绑定是否把请求转发给宿主 provider。
    fn is_host_provider_mode(&self) -> bool {
        self.provider_mode == LanceDbBindingMode::HostCallback
    }

    /// Return whether the current binding dispatches requests into one external space controller.
    /// 返回当前绑定是否把请求转发给外部空间控制器。
    fn is_space_controller_mode(&self) -> bool {
        self.provider_mode == LanceDbBindingMode::SpaceController
    }

    /// Return the loaded dynamic-library API for dynamic mode bindings.
    /// 返回动态模式绑定所对应的已加载动态库 API。
    fn api_ref(&self) -> &LoadedLanceDbApi {
        self.api
            .as_ref()
            .expect("LanceDB dynamic-library API missing in host provider mode")
    }

    /// Return the stable integration mode name for diagnostics and Lua status payloads.
    /// 返回用于诊断和 Lua 状态输出的稳定集成模式名称。
    fn integration_mode_name(&self) -> &'static str {
        match self.provider_mode {
            LanceDbBindingMode::DynamicLibrary => "dynamic_library",
            LanceDbBindingMode::HostCallback => "host_callback",
            LanceDbBindingMode::SpaceController => "space_controller",
        }
    }

    /// Dispatch one LanceDB operation through the host-registered provider contract.
    /// 通过宿主已注册的 provider 协议分发一次 LanceDB 操作。
    fn dispatch_host_provider(
        &self,
        action: RuntimeLanceDbProviderAction,
        input: &Value,
    ) -> Result<crate::host::database::RuntimeLanceDbProviderResult, String> {
        let request = RuntimeLanceDbProviderRequest {
            action,
            binding: self.provider_binding.clone(),
            input: input.clone(),
        };
        dispatch_lancedb_provider_request(&request, self.callback_mode)
    }

    /// Acquire the handle lock so LanceDB FFI calls for the same skill execute serially.
    /// 获取句柄锁，确保同一个 skill 的 LanceDB FFI 调用按顺序串行执行。
    fn lock_handles(&self) -> Result<std::sync::MutexGuard<'_, SkillHandleState>, String> {
        self.handles
            .as_ref()
            .ok_or_else(|| {
                "LanceDB dynamic-library handles are unavailable in host provider mode".to_string()
            })?
            .lock()
            .map_err(|_| "failed to acquire LanceDB handle lock".to_string())
    }

    /// Return the shared controller bridge for one space-controller binding.
    /// 返回 space-controller 绑定所使用的共享控制器桥接。
    fn controller_bridge(&self) -> Result<&Arc<LuaRuntimeSpaceControllerBridge>, String> {
        self.controller
            .as_ref()
            .ok_or_else(|| "LanceDB space-controller bridge is unavailable".to_string())
    }

    /// Return the shared controller runtime-space identifier for the current skill binding.
    /// 返回当前 skill 绑定对应的共享控制器运行时空间标识。
    fn controller_space_id(&self) -> String {
        controller_space_id_for_binding(&self.provider_binding)
    }

    /// Return the stable controller database-binding identifier for the current skill binding.
    /// 返回当前 skill 绑定对应的稳定控制器数据库绑定标识。
    fn controller_binding_id(&self) -> String {
        self.provider_binding.binding_tag.clone()
    }
}

impl Drop for LanceDbSkillBinding {
    /// The host releases engine and runtime handles together when the skill binding is dropped.
    /// 由宿主在 skill 生命周期结束时统一释放引擎与运行时句柄。
    fn drop(&mut self) {
        let Some(handles) = self.handles.as_ref() else {
            return;
        };
        let Some(api) = self.api.as_ref() else {
            return;
        };
        if let Ok(mut guard) = handles.lock() {
            unsafe {
                if !guard.engine.is_null() {
                    (api.engine_destroy)(guard.engine);
                    guard.engine = ptr::null_mut();
                }
                if !guard.runtime.is_null() {
                    (api.runtime_destroy)(guard.runtime);
                    guard.runtime = ptr::null_mut();
                }
            }
        }
    }
}

/// Maintain skill-scoped LanceDB bindings, auto-creating and reusing them for enabled skills.
/// 按 skill 维度维护 LanceDB 绑定，负责技能启用后的自动创建与长期复用。
pub struct LanceDbSkillHost {
    api: Option<Arc<LoadedLanceDbApi>>,
    controller: Option<Arc<LuaRuntimeSpaceControllerBridge>>,
    skills: Mutex<HashMap<String, Arc<LanceDbSkillBinding>>>,
    host_options: LuaRuntimeHostOptions,
}

impl LanceDbSkillHost {
    /// Create the host-side LanceDB skill manager and load the dynamic library immediately.
    /// 创建宿主级 LanceDB 技能管理器，并立即加载动态库。
    pub fn new(host_options: LuaRuntimeHostOptions) -> Result<Self, String> {
        let api = match host_options.lancedb_provider_mode {
            LuaRuntimeDatabaseProviderMode::DynamicLibrary => {
                let library_path = host_options.lancedb_library_path.clone().ok_or_else(|| {
                    "LanceDB dynamic-library mode requires host_options.lancedb_library_path"
                        .to_string()
                })?;
                Some(Arc::new(LoadedLanceDbApi::load(&library_path)?))
            }
            LuaRuntimeDatabaseProviderMode::HostCallback => {
                if !has_lancedb_provider_callback_for_mode(host_options.lancedb_callback_mode)? {
                    return Err(format!(
                        "LanceDB host-callback mode is enabled but no {} callback is registered",
                        callback_mode_name(host_options.lancedb_callback_mode)
                    ));
                }
                None
            }
            LuaRuntimeDatabaseProviderMode::SpaceController => None,
        };
        let controller = match host_options.lancedb_provider_mode {
            LuaRuntimeDatabaseProviderMode::SpaceController => Some(
                LuaRuntimeSpaceControllerBridge::new(&host_options, "lancedb")?,
            ),
            _ => None,
        };
        Ok(Self {
            api,
            controller,
            skills: Mutex::new(HashMap::new()),
            host_options,
        })
    }

    /// Register the fixed database binding for a LanceDB-enabled skill; each skill is created only once.
    /// 为启用 LanceDB 的 skill 注册固定数据库绑定；同一个 skill 只会创建一次。
    pub fn register_skill(
        &self,
        root_name: &str,
        skill_name: &str,
        skill_dir: &Path,
        config: SkillLanceDbMeta,
    ) -> Result<Arc<LanceDbSkillBinding>, String> {
        let mut guard = self
            .skills
            .lock()
            .map_err(|_| "failed to acquire LanceDB skill registry lock".to_string())?;
        if let Some(existing) = guard.get(skill_name) {
            return Ok(existing.clone());
        }

        let skill_dir_name = skill_dir
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                format!(
                    "invalid skill directory name for {}: {}",
                    skill_name,
                    skill_dir.display()
                )
            })?
            .to_string();
        let skills_root = skill_dir.parent().ok_or_else(|| {
            format!(
                "invalid skill root for {}: {}",
                skill_name,
                skill_dir.display()
            )
        })?;
        let sidecar_root = skills_root
            .parent()
            .unwrap_or(skills_root)
            .join(self.host_options.database_dir_name.as_str());
        let db_path = sidecar_root.join("lancedb").join(skill_name);
        let database_path = db_path.to_string_lossy().to_string();
        let binding_context = RuntimeDatabaseBindingContext::new(
            root_name,
            skill_name,
            root_name,
            sidecar_root.to_string_lossy().to_string(),
            skill_dir.to_string_lossy().to_string(),
            skill_dir_name.clone(),
            RuntimeDatabaseKind::LanceDb,
            database_path.clone(),
        );
        let (resolved_path, handles, provider_mode, controller) = if let Some(api) =
            self.api.as_ref()
        {
            std::fs::create_dir_all(&db_path).map_err(|error| {
                format!(
                    "failed to create LanceDB directory {}: {}: {}",
                    db_path.display(),
                    error,
                    error
                )
            })?;
            let default_path = CString::new(database_path.clone())
                .map_err(|_| "database path contains interior NUL bytes".to_string())?;
            let mut options = unsafe { (api.runtime_options_default)() };
            options.default_db_path = default_path.as_ptr();
            options.db_root = ptr::null();
            let runtime = unsafe { (api.runtime_create)(options) };
            if runtime.is_null() {
                return Err(api.take_last_error_message());
            }

            let engine = unsafe { (api.runtime_open_default_engine)(runtime) };
            if engine.is_null() {
                unsafe {
                    (api.runtime_destroy)(runtime);
                }
                return Err(api.take_last_error_message());
            }

            let resolved_path = unsafe {
                api.take_owned_string((api.runtime_database_path_for_name)(runtime, ptr::null()))
            }
            .unwrap_or(database_path.clone());
            (
                resolved_path,
                Some(Mutex::new(SkillHandleState { runtime, engine })),
                LanceDbBindingMode::DynamicLibrary,
                None,
            )
        } else if matches!(
            self.host_options.lancedb_provider_mode,
            LuaRuntimeDatabaseProviderMode::SpaceController
        ) {
            let controller = self
                .controller
                .as_ref()
                .ok_or_else(|| "LanceDB space-controller bridge is unavailable".to_string())?
                .clone();
            let controller_space_id = controller_space_id_for_binding(&binding_context);
            let controller_binding_id = binding_context.binding_tag.clone();
            controller.attach_binding(&binding_context)?;
            controller.block_on(controller.client().enable_lancedb(
                ControllerLanceDbEnableRequest {
                    space_id: controller_space_id,
                    binding_id: controller_binding_id,
                    default_db_path: database_path.clone(),
                    ..ControllerLanceDbEnableRequest::default()
                },
            ))?;
            (
                database_path.clone(),
                None,
                LanceDbBindingMode::SpaceController,
                Some(controller),
            )
        } else {
            (
                database_path.clone(),
                None,
                LanceDbBindingMode::HostCallback,
                None,
            )
        };

        let binding = Arc::new(LanceDbSkillBinding {
            api: self.api.clone(),
            skill_name: skill_name.to_string(),
            skill_dir_name,
            database_path: resolved_path,
            config,
            provider_mode,
            callback_mode: self.host_options.lancedb_callback_mode,
            handles,
            controller,
            provider_binding: binding_context,
        });
        guard.insert(skill_name.to_string(), binding.clone());
        Ok(binding)
    }

    /// Fetch a registered binding by skill name so Lua injection and cross-skill calls can restore context.
    /// 按 skill 名称获取已注册绑定，供 Lua 注入与跨 skill 调用恢复上下文使用。
    pub fn binding_for_skill(
        &self,
        skill_name: &str,
    ) -> Result<Option<Arc<LanceDbSkillBinding>>, String> {
        let skills = self
            .skills
            .lock()
            .map_err(|_| "LanceDB skill binding registry lock poisoned".to_string())?;
        Ok(skills.get(skill_name).cloned())
    }
}

/// Build a stable status object for skills that do not enable LanceDB so Lua can check before calling.
/// 为未启用 LanceDB 的 skill 生成稳定状态对象，便于 Lua 侧先判断再调用。
pub fn disabled_skill_status_json(skill_name: Option<&str>) -> Value {
    json!({
        "enabled": false,
        "initialized": false,
        "skill_name": skill_name.unwrap_or(""),
        "integration_mode": "dynamic_library",
        "reason": "current skill has not enabled lancedb"
    })
}

/// Return the stable callback-mode display name used in host callback error messages.
/// 返回宿主回调错误消息中使用的稳定回调模式显示名称。
fn callback_mode_name(mode: LuaRuntimeDatabaseCallbackMode) -> &'static str {
    match mode {
        LuaRuntimeDatabaseCallbackMode::Standard => "standard",
        LuaRuntimeDatabaseCallbackMode::Json => "json",
    }
}

/// Ensure that a required string field exists in the JSON request payload.
/// 确保 JSON 请求载荷中存在指定的必填字符串字段。
fn require_string_field<'a>(input: &'a Value, field_name: &str) -> Result<&'a str, String> {
    input
        .get(field_name)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("missing or empty field `{}`", field_name))
}
