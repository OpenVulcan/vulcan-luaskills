use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{CString, c_char};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::ffi_standard::{FfiBorrowedBuffer, FfiOwnedBuffer};

use crate::runtime_context::RuntimeRequestContext;
use crate::runtime_help::{RuntimeHelpDetail, RuntimeSkillHelpDescriptor};
use crate::runtime_options::{LuaInvocationContext, RuntimeSkillRoot};
use crate::{
    LuaEngine, LuaEngineOptions, RuntimeEntryDescriptor, RuntimeInvocationResult, SkillApplyResult,
    SkillInstallRequest, SkillUninstallOptions, SkillUninstallResult,
};

/// Stable FFI protocol version derived from the crate package version.
/// 从 crate 包版本派生出的稳定 FFI 协议版本。
pub(crate) const FFI_VERSION: &str = env!("CARGO_PKG_VERSION");

/// One stable JSON response envelope returned by every LuaSkills FFI entrypoint.
/// 每个 LuaSkills FFI 入口统一返回的稳定 JSON 响应包络。
#[derive(Debug, Serialize)]
struct FfiJsonEnvelope<T: Serialize> {
    /// Whether the requested operation completed successfully.
    /// 当前请求操作是否执行成功。
    ok: bool,
    /// Structured result payload when the operation succeeds.
    /// 操作成功时返回的结构化结果载荷。
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<T>,
    /// Human-readable English error message when the operation fails.
    /// 操作失败时返回的人类可读英文错误消息。
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// One engine registry entry stored behind one stable numeric FFI handle id.
/// 通过稳定数值 FFI 句柄标识存放的单个引擎注册表条目。
pub(crate) struct FfiEngineSlot {
    /// Independently locked runtime engine instance owned by the current FFI handle.
    /// 由当前 FFI 句柄拥有并独立加锁的运行时引擎实例。
    pub(crate) engine: Arc<Mutex<LuaEngine>>,
}

impl FfiEngineSlot {
    /// Wrap one runtime engine into one independently locked shared FFI handle slot.
    /// 将单个运行时引擎封装为一个可独立加锁的共享 FFI 句柄槽位。
    pub(crate) fn new(engine: LuaEngine) -> Self {
        Self {
            engine: Arc::new(Mutex::new(engine)),
        }
    }
}

/// One JSON request used to create one runtime engine instance.
/// 用于创建单个运行时引擎实例的 JSON 请求。
#[derive(Debug, Serialize, Deserialize)]
struct EngineNewJsonRequest {
    /// Engine construction options forwarded to the Rust runtime.
    /// 直接转发给 Rust 运行时的引擎构造选项。
    options: LuaEngineOptions,
}

/// One JSON result containing one stable engine handle id.
/// 包含单个稳定引擎句柄标识的 JSON 结果。
#[derive(Debug, Serialize, Deserialize)]
struct EngineHandleJsonResult {
    /// Stable numeric FFI handle id of the created engine.
    /// 已创建引擎对应的稳定数值 FFI 句柄标识。
    engine_id: u64,
}

/// One JSON request that targets one existing engine handle.
/// 定位单个现有引擎句柄的 JSON 请求。
#[derive(Debug, Serialize, Deserialize)]
struct EngineIdJsonRequest {
    /// Stable numeric FFI handle id of the target engine.
    /// 目标引擎的稳定数值 FFI 句柄标识。
    engine_id: u64,
}

/// One JSON request that targets one engine together with an ordered root chain.
/// 同时携带单个引擎与一条有序根链的 JSON 请求。
#[derive(Debug, Serialize, Deserialize)]
struct EngineRootsJsonRequest {
    /// Stable numeric FFI handle id of the target engine.
    /// 目标引擎的稳定数值 FFI 句柄标识。
    engine_id: u64,
    /// Ordered skill roots used by the current operation.
    /// 当前操作使用的有序技能根链。
    skill_roots: Vec<RuntimeSkillRoot>,
}

/// One JSON request that targets one engine together with directory-style roots.
/// 同时携带单个引擎与目录风格根参数的 JSON 请求。
#[derive(Debug, Serialize, Deserialize)]
struct EngineDirsJsonRequest {
    /// Stable numeric FFI handle id of the target engine.
    /// 目标引擎的稳定数值 FFI 句柄标识。
    engine_id: u64,
    /// Base skill directory used by the legacy direct integration API.
    /// 旧直接集成 API 使用的基础技能目录。
    base_dir: PathBuf,
    /// Optional override skill directory used by the legacy direct integration API.
    /// 旧直接集成 API 使用的可选覆盖技能目录。
    #[serde(default)]
    override_dir: Option<PathBuf>,
}

/// One JSON request used to disable one skill through legacy directory-style roots.
/// 用于通过旧目录风格根参数停用单个技能的 JSON 请求。
#[derive(Debug, Serialize, Deserialize)]
struct DisableSkillDirsJsonRequest {
    /// Stable numeric FFI handle id of the target engine.
    /// 目标引擎的稳定数值 FFI 句柄标识。
    engine_id: u64,
    /// Base skill directory used by the legacy direct integration API.
    /// 旧直接集成 API 使用的基础技能目录。
    base_dir: PathBuf,
    /// Optional override skill directory used by the legacy direct integration API.
    /// 旧直接集成 API 使用的可选覆盖技能目录。
    #[serde(default)]
    override_dir: Option<PathBuf>,
    /// Stable target skill identifier.
    /// 稳定的目标技能标识符。
    skill_id: String,
    /// Optional disable reason persisted into the skill state.
    /// 持久化到技能状态中的可选停用原因。
    #[serde(default)]
    reason: Option<String>,
}

/// One JSON request used to render help detail for one skill flow.
/// 用于渲染单个技能帮助详情的 JSON 请求。
#[derive(Debug, Serialize, Deserialize)]
struct RenderHelpJsonRequest {
    /// Stable numeric FFI handle id of the target engine.
    /// 目标引擎的稳定数值 FFI 句柄标识。
    engine_id: u64,
    /// Stable skill identifier of the target help tree.
    /// 目标帮助树所属的稳定技能标识符。
    skill_id: String,
    /// Stable help flow name, where `main` means the main help node.
    /// 稳定帮助流程名，其中 `main` 表示主帮助节点。
    flow_name: String,
    /// Optional request context injected during help rendering.
    /// 在帮助渲染时一并注入的可选请求上下文。
    #[serde(default)]
    request_context: Option<RuntimeRequestContext>,
}

/// One JSON request used to query prompt argument completions.
/// 用于查询提示词参数补全项的 JSON 请求。
#[derive(Debug, Serialize, Deserialize)]
struct PromptCompletionJsonRequest {
    /// Stable numeric FFI handle id of the target engine.
    /// 目标引擎的稳定数值 FFI 句柄标识。
    engine_id: u64,
    /// Stable prompt name supplied by the host.
    /// 由宿主提供的稳定提示词名称。
    prompt_name: String,
    /// Stable prompt argument name supplied by the host.
    /// 由宿主提供的稳定提示词参数名称。
    argument_name: String,
}

/// One JSON request used to check whether one canonical tool name is a Lua skill entry.
/// 用于检查某个 canonical 工具名是否为 Lua 技能入口的 JSON 请求。
#[derive(Debug, Serialize, Deserialize)]
struct IsSkillJsonRequest {
    /// Stable numeric FFI handle id of the target engine.
    /// 目标引擎的稳定数值 FFI 句柄标识。
    engine_id: u64,
    /// Tool name to resolve against the runtime entry registry.
    /// 需要在运行时入口注册表中解析的工具名称。
    tool_name: String,
}

/// One JSON result that answers one boolean runtime query.
/// 用于返回单个布尔型运行时查询结果的 JSON 结果。
#[derive(Debug, Serialize, Deserialize)]
struct BoolJsonResult {
    /// Boolean value returned by the runtime query.
    /// 运行时查询返回的布尔值。
    value: bool,
}

/// One JSON request used to resolve the owning skill id of one canonical tool name.
/// 用于解析某个 canonical 工具名所属技能标识符的 JSON 请求。
#[derive(Debug, Serialize, Deserialize)]
struct SkillNameForToolJsonRequest {
    /// Stable numeric FFI handle id of the target engine.
    /// 目标引擎的稳定数值 FFI 句柄标识。
    engine_id: u64,
    /// Tool name to resolve against the runtime entry registry.
    /// 需要在运行时入口注册表中解析的工具名称。
    tool_name: String,
}

/// One JSON result containing the optional owning skill id of one tool.
/// 包含某个工具可选所属技能标识符的 JSON 结果。
#[derive(Debug, Serialize, Deserialize)]
struct OptionalSkillNameJsonResult {
    /// Optional owning skill id resolved from the current runtime registry.
    /// 当前运行时注册表解析出的可选所属技能标识符。
    #[serde(skip_serializing_if = "Option::is_none")]
    skill_id: Option<String>,
}

/// One JSON request used to call one loaded skill entry.
/// 用于调用单个已加载技能入口的 JSON 请求。
#[derive(Debug, Serialize, Deserialize)]
struct CallSkillJsonRequest {
    /// Stable numeric FFI handle id of the target engine.
    /// 目标引擎的稳定数值 FFI 句柄标识。
    engine_id: u64,
    /// Canonical tool name of the target Lua skill entry.
    /// 目标 Lua 技能入口的 canonical 工具名称。
    tool_name: String,
    /// JSON arguments forwarded to the target skill entry.
    /// 转发给目标技能入口的 JSON 参数。
    #[serde(default = "default_json_object")]
    args: Value,
    /// Optional invocation context injected into the runtime call.
    /// 注入到运行时调用中的可选调用上下文。
    #[serde(default)]
    invocation_context: Option<LuaInvocationContext>,
}

/// One JSON request used to execute arbitrary Lua code.
/// 用于执行任意 Lua 代码的 JSON 请求。
#[derive(Debug, Serialize, Deserialize)]
struct RunLuaJsonRequest {
    /// Stable numeric FFI handle id of the target engine.
    /// 目标引擎的稳定数值 FFI 句柄标识。
    engine_id: u64,
    /// Inline Lua source code executed by the runtime engine.
    /// 由运行时引擎执行的内联 Lua 源代码。
    code: String,
    /// JSON arguments exposed to the Lua snippet as `args`.
    /// 作为 `args` 暴露给 Lua 片段的 JSON 参数。
    #[serde(default = "default_json_object")]
    args: Value,
    /// Optional invocation context injected into the runtime call.
    /// 注入到运行时调用中的可选调用上下文。
    #[serde(default)]
    invocation_context: Option<LuaInvocationContext>,
}

/// One JSON request used to disable one skill in one ordered root chain.
/// 用于在一条有序根链中停用单个技能的 JSON 请求。
#[derive(Debug, Serialize, Deserialize)]
struct DisableSkillJsonRequest {
    /// Stable numeric FFI handle id of the target engine.
    /// 目标引擎的稳定数值 FFI 句柄标识。
    engine_id: u64,
    /// Ordered skill roots used by the current lifecycle operation.
    /// 当前生命周期操作使用的有序技能根链。
    skill_roots: Vec<RuntimeSkillRoot>,
    /// Stable target skill identifier.
    /// 稳定的目标技能标识符。
    skill_id: String,
    /// Optional disable reason persisted into the skill state.
    /// 持久化到技能状态中的可选停用原因。
    #[serde(default)]
    reason: Option<String>,
}

/// One JSON request used to enable one skill in one ordered root chain.
/// 用于在一条有序根链中启用单个技能的 JSON 请求。
#[derive(Debug, Serialize, Deserialize)]
struct EnableSkillJsonRequest {
    /// Stable numeric FFI handle id of the target engine.
    /// 目标引擎的稳定数值 FFI 句柄标识。
    engine_id: u64,
    /// Ordered skill roots used by the current lifecycle operation.
    /// 当前生命周期操作使用的有序技能根链。
    skill_roots: Vec<RuntimeSkillRoot>,
    /// Stable target skill identifier.
    /// 稳定的目标技能标识符。
    skill_id: String,
}

/// One JSON request used to uninstall one skill in one ordered root chain.
/// 用于在一条有序根链中卸载单个技能的 JSON 请求。
#[derive(Debug, Serialize, Deserialize)]
struct UninstallSkillJsonRequest {
    /// Stable numeric FFI handle id of the target engine.
    /// 目标引擎的稳定数值 FFI 句柄标识。
    engine_id: u64,
    /// Ordered skill roots used by the current lifecycle operation.
    /// 当前生命周期操作使用的有序技能根链。
    skill_roots: Vec<RuntimeSkillRoot>,
    /// Stable target skill identifier.
    /// 稳定的目标技能标识符。
    skill_id: String,
    /// Optional database cleanup switches applied after uninstall commit succeeds.
    /// 在卸载提交成功后应用的可选数据库清理开关。
    #[serde(default)]
    options: SkillUninstallOptions,
}

/// One JSON request used to install or update one managed skill in one ordered root chain.
/// 用于在一条有序根链中安装或更新单个受管技能的 JSON 请求。
#[derive(Debug, Serialize, Deserialize)]
struct ApplySkillJsonRequest {
    /// Stable numeric FFI handle id of the target engine.
    /// 目标引擎的稳定数值 FFI 句柄标识。
    engine_id: u64,
    /// Ordered skill roots used by the current lifecycle operation.
    /// 当前生命周期操作使用的有序技能根链。
    skill_roots: Vec<RuntimeSkillRoot>,
    /// Managed install or update request forwarded to the Rust runtime.
    /// 直接转发给 Rust 运行时的受管安装或更新请求。
    request: SkillInstallRequest,
}

/// One JSON result that lists the currently exported FFI entrypoints.
/// 列出当前已导出 FFI 入口点的 JSON 结果。
#[derive(Debug, Serialize, Deserialize)]
struct FfiDescribeJsonResult {
    /// Stable FFI version string for compatibility checks.
    /// 用于兼容性检查的稳定 FFI 版本字符串。
    ffi_version: String,
    /// Exported JSON entrypoint names currently provided by the library.
    /// 当前由库提供的已导出 JSON 入口点名称列表。
    exported_functions: Vec<String>,
}

pub(crate) static FFI_ENGINE_REGISTRY: OnceLock<Mutex<HashMap<u64, FfiEngineSlot>>> =
    OnceLock::new();
pub(crate) static FFI_ENGINE_COUNTER: AtomicU64 = AtomicU64::new(1);

thread_local! {
    /// Active engine ids currently executing on the calling thread.
    /// 当前调用线程上正在执行中的引擎标识列表。
    static ACTIVE_FFI_ENGINE_IDS: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
}

/// One thread-local engine activity guard used to reject same-thread reentrant access.
/// 用于拒绝同线程重入访问的线程局部引擎活动守卫。
struct ActiveFfiEngineGuard {
    engine_id: u64,
}

impl ActiveFfiEngineGuard {
    /// Enter one engine activity scope on the current thread.
    /// 在当前线程上进入单个引擎活动作用域。
    fn enter(engine_id: u64) -> Result<Self, String> {
        ACTIVE_FFI_ENGINE_IDS.with(|active_ids| {
            let mut active_ids = active_ids.borrow_mut();
            if active_ids.contains(&engine_id) {
                return Err(format!(
                    "FFI engine {} reentrant access is not allowed on the same thread",
                    engine_id
                ));
            }
            active_ids.push(engine_id);
            Ok(Self { engine_id })
        })
    }
}

impl Drop for ActiveFfiEngineGuard {
    fn drop(&mut self) {
        ACTIVE_FFI_ENGINE_IDS.with(|active_ids| {
            let mut active_ids = active_ids.borrow_mut();
            if let Some(position) = active_ids
                .iter()
                .rposition(|active| *active == self.engine_id)
            {
                active_ids.remove(position);
            }
        });
    }
}

/// Return the default empty JSON object payload.
/// 返回默认的空 JSON 对象载荷。
fn default_json_object() -> Value {
    Value::Object(serde_json::Map::new())
}

/// Return the global engine registry used by the FFI layer.
/// 返回 FFI 层使用的全局引擎注册表。
pub(crate) fn ffi_engine_registry() -> &'static Mutex<HashMap<u64, FfiEngineSlot>> {
    FFI_ENGINE_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Clone one shared engine handle out of the global registry without holding the registry lock during execution.
/// 从全局注册表中克隆一个共享引擎句柄，并确保执行期间不再持有注册表锁。
fn clone_engine_handle(engine_id: u64) -> Result<Arc<Mutex<LuaEngine>>, String> {
    let registry = ffi_engine_registry()
        .lock()
        .map_err(|_| "FFI engine registry lock poisoned".to_string())?;
    registry
        .get(&engine_id)
        .map(|slot| Arc::clone(&slot.engine))
        .ok_or_else(|| format!("FFI engine {} not found", engine_id))
}

/// Convert one owned byte slice into one LuaSkills-owned FFI buffer container.
/// 将一段拥有型字节切片转换为一个由 LuaSkills 管理的 FFI 缓冲容器。
fn owned_buffer_from_bytes(bytes: &[u8]) -> FfiOwnedBuffer {
    if bytes.is_empty() {
        return FfiOwnedBuffer {
            ptr: std::ptr::null_mut(),
            len: 0,
        };
    }
    let mut owned = bytes.to_vec();
    let pointer = owned.as_mut_ptr();
    let len = owned.len();
    std::mem::forget(owned);
    FfiOwnedBuffer { ptr: pointer, len }
}

/// Convert one Rust value into one LuaSkills-owned UTF-8 JSON response buffer.
/// 将单个 Rust 值转换为一个由 LuaSkills 管理的 UTF-8 JSON 响应缓冲。
fn encode_json_buffer<T: Serialize>(value: &T) -> FfiOwnedBuffer {
    let json_text = serde_json::to_string(value).unwrap_or_else(|error| {
        format!(
            "{{\"ok\":false,\"error\":\"Failed to serialize FFI response: {}\"}}",
            error
        )
    });
    owned_buffer_from_bytes(json_text.as_bytes())
}

/// Build one successful FFI JSON envelope.
/// 构造一个成功的 FFI JSON 响应包络。
fn ffi_ok<T: Serialize>(result: T) -> FfiOwnedBuffer {
    encode_json_buffer(&FfiJsonEnvelope {
        ok: true,
        result: Some(result),
        error: None::<String>,
    })
}

/// Build one failed FFI JSON envelope.
/// 构造一个失败的 FFI JSON 响应包络。
fn ffi_error(message: impl Into<String>) -> FfiOwnedBuffer {
    encode_json_buffer(&FfiJsonEnvelope::<Value> {
        ok: false,
        result: None,
        error: Some(message.into()),
    })
}

/// Parse one UTF-8 JSON request from one foreign string pointer.
/// 从外部字符串指针解析一段 UTF-8 JSON 请求。
fn decode_json_request<T: DeserializeOwned>(
    input_json: FfiBorrowedBuffer,
    function_name: &str,
) -> Result<T, String> {
    if input_json.ptr.is_null() {
        if input_json.len == 0 {
            return Err(format!(
                "{} requires one non-null JSON buffer",
                function_name
            ));
        }
        return Err(format!(
            "{} received null JSON buffer with non-zero len",
            function_name
        ));
    }
    let bytes = unsafe { std::slice::from_raw_parts(input_json.ptr, input_json.len) };
    let text = std::str::from_utf8(bytes)
        .map_err(|error| format!("{} received invalid UTF-8 input: {}", function_name, error))?;
    serde_json::from_str(text)
        .map_err(|error| format!("{} received invalid JSON input: {}", function_name, error))
}

/// Execute one read-only engine operation by engine id.
/// 按引擎标识执行一次只读引擎操作。
pub(crate) fn with_engine<T, F>(engine_id: u64, operation: F) -> Result<T, String>
where
    F: FnOnce(&LuaEngine) -> Result<T, String>,
{
    let engine_handle = clone_engine_handle(engine_id)?;
    let _active_guard = ActiveFfiEngineGuard::enter(engine_id)?;
    let engine = engine_handle
        .lock()
        .map_err(|_| format!("FFI engine {} lock poisoned", engine_id))?;
    operation(&engine)
}

/// Execute one mutable engine operation by engine id.
/// 按引擎标识执行一次可变引擎操作。
pub(crate) fn with_engine_mut<T, F>(engine_id: u64, operation: F) -> Result<T, String>
where
    F: FnOnce(&mut LuaEngine) -> Result<T, String>,
{
    let engine_handle = clone_engine_handle(engine_id)?;
    let _active_guard = ActiveFfiEngineGuard::enter(engine_id)?;
    let mut engine = engine_handle
        .lock()
        .map_err(|_| format!("FFI engine {} lock poisoned", engine_id))?;
    operation(&mut engine)
}

/// Return one stable list of all exported FFI entrypoints.
/// 返回一份稳定的全部已导出 FFI 入口点列表。
pub(crate) fn exported_ffi_function_names() -> Vec<String> {
    vec![
        "vulcan_luaskills_ffi_version",
        "vulcan_luaskills_ffi_describe",
        "vulcan_luaskills_ffi_engine_new",
        "vulcan_luaskills_ffi_engine_free",
        "vulcan_luaskills_ffi_load_from_dirs",
        "vulcan_luaskills_ffi_load_from_roots",
        "vulcan_luaskills_ffi_reload_from_dirs",
        "vulcan_luaskills_ffi_reload_from_roots",
        "vulcan_luaskills_ffi_list_entries",
        "vulcan_luaskills_ffi_list_skill_help",
        "vulcan_luaskills_ffi_render_skill_help_detail",
        "vulcan_luaskills_ffi_prompt_argument_completions",
        "vulcan_luaskills_ffi_is_skill",
        "vulcan_luaskills_ffi_skill_name_for_tool",
        "vulcan_luaskills_ffi_call_skill",
        "vulcan_luaskills_ffi_run_lua",
        "vulcan_luaskills_ffi_disable_skill_in_dirs",
        "vulcan_luaskills_ffi_disable_skill",
        "vulcan_luaskills_ffi_system_disable_skill_in_dirs",
        "vulcan_luaskills_ffi_system_disable_skill",
        "vulcan_luaskills_ffi_enable_skill",
        "vulcan_luaskills_ffi_system_enable_skill",
        "vulcan_luaskills_ffi_uninstall_skill",
        "vulcan_luaskills_ffi_system_uninstall_skill",
        "vulcan_luaskills_ffi_install_skill",
        "vulcan_luaskills_ffi_system_install_skill",
        "vulcan_luaskills_ffi_update_skill",
        "vulcan_luaskills_ffi_system_update_skill",
        "vulcan_luaskills_ffi_set_sqlite_provider_callback",
        "vulcan_luaskills_ffi_set_lancedb_provider_callback",
        "vulcan_luaskills_ffi_set_sqlite_provider_json_callback",
        "vulcan_luaskills_ffi_set_lancedb_provider_json_callback",
        "vulcan_luaskills_ffi_string_clone",
        "vulcan_luaskills_ffi_version_json",
        "vulcan_luaskills_ffi_describe_json",
        "vulcan_luaskills_ffi_engine_new_json",
        "vulcan_luaskills_ffi_engine_free_json",
        "vulcan_luaskills_ffi_load_from_dirs_json",
        "vulcan_luaskills_ffi_load_from_roots_json",
        "vulcan_luaskills_ffi_reload_from_dirs_json",
        "vulcan_luaskills_ffi_reload_from_roots_json",
        "vulcan_luaskills_ffi_list_entries_json",
        "vulcan_luaskills_ffi_list_skill_help_json",
        "vulcan_luaskills_ffi_render_skill_help_detail_json",
        "vulcan_luaskills_ffi_prompt_argument_completions_json",
        "vulcan_luaskills_ffi_is_skill_json",
        "vulcan_luaskills_ffi_skill_name_for_tool_json",
        "vulcan_luaskills_ffi_call_skill_json",
        "vulcan_luaskills_ffi_run_lua_json",
        "vulcan_luaskills_ffi_disable_skill_in_dirs_json",
        "vulcan_luaskills_ffi_disable_skill_json",
        "vulcan_luaskills_ffi_system_disable_skill_in_dirs_json",
        "vulcan_luaskills_ffi_system_disable_skill_json",
        "vulcan_luaskills_ffi_enable_skill_json",
        "vulcan_luaskills_ffi_system_enable_skill_json",
        "vulcan_luaskills_ffi_uninstall_skill_json",
        "vulcan_luaskills_ffi_system_uninstall_skill_json",
        "vulcan_luaskills_ffi_install_skill_json",
        "vulcan_luaskills_ffi_system_install_skill_json",
        "vulcan_luaskills_ffi_update_skill_json",
        "vulcan_luaskills_ffi_system_update_skill_json",
        "vulcan_luaskills_ffi_string_free",
        "vulcan_luaskills_ffi_bytes_clone",
        "vulcan_luaskills_ffi_bytes_free",
        "vulcan_luaskills_ffi_buffer_clone",
        "vulcan_luaskills_ffi_buffer_free",
        "vulcan_luaskills_ffi_string_array_free",
        "vulcan_luaskills_ffi_entry_list_free",
        "vulcan_luaskills_ffi_help_list_free",
        "vulcan_luaskills_ffi_help_detail_free",
        "vulcan_luaskills_ffi_invocation_result_free",
        "vulcan_luaskills_ffi_skill_apply_result_free",
        "vulcan_luaskills_ffi_skill_uninstall_result_free",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

/// Free one heap-allocated JSON string returned by the FFI layer.
/// 释放一段由 FFI 层返回并在堆上分配的 JSON 字符串。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_string_free(value: *mut c_char) {
    if !value.is_null() {
        let _ = unsafe { CString::from_raw(value) };
    }
}

/// Return the stable FFI version descriptor as one JSON envelope.
/// 以 JSON 响应包络形式返回稳定的 FFI 版本描述。
#[unsafe(no_mangle)]
pub extern "C" fn vulcan_luaskills_ffi_version_json() -> FfiOwnedBuffer {
    ffi_ok(json!({
        "ffi_version": FFI_VERSION,
        "protocol": "json-cabi"
    }))
}

/// Return the exported JSON FFI entrypoint list as one JSON envelope.
/// 以 JSON 响应包络形式返回已导出的 JSON FFI 入口列表。
#[unsafe(no_mangle)]
pub extern "C" fn vulcan_luaskills_ffi_describe_json() -> FfiOwnedBuffer {
    ffi_ok(FfiDescribeJsonResult {
        ffi_version: FFI_VERSION.to_string(),
        exported_functions: exported_ffi_function_names(),
    })
}

/// Create one new LuaSkills engine instance and return its stable FFI handle id.
/// 创建一个新的 LuaSkills 引擎实例，并返回其稳定的 FFI 句柄标识。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_engine_new_json(
    input_json: FfiBorrowedBuffer,
) -> FfiOwnedBuffer {
    let request = match decode_json_request::<EngineNewJsonRequest>(
        input_json,
        "vulcan_luaskills_ffi_engine_new_json",
    ) {
        Ok(request) => request,
        Err(error) => return ffi_error(error),
    };
    match LuaEngine::new(request.options) {
        Ok(engine) => {
            let engine_id = FFI_ENGINE_COUNTER.fetch_add(1, Ordering::Relaxed);
            let mut registry = match ffi_engine_registry().lock() {
                Ok(registry) => registry,
                Err(_) => return ffi_error("FFI engine registry lock poisoned"),
            };
            registry.insert(engine_id, FfiEngineSlot::new(engine));
            ffi_ok(EngineHandleJsonResult { engine_id })
        }
        Err(error) => ffi_error(error.to_string()),
    }
}

/// Free one existing LuaSkills engine handle.
/// 释放一个现有的 LuaSkills 引擎句柄。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_engine_free_json(
    input_json: FfiBorrowedBuffer,
) -> FfiOwnedBuffer {
    let request = match decode_json_request::<EngineIdJsonRequest>(
        input_json,
        "vulcan_luaskills_ffi_engine_free_json",
    ) {
        Ok(request) => request,
        Err(error) => return ffi_error(error),
    };
    let mut registry = match ffi_engine_registry().lock() {
        Ok(registry) => registry,
        Err(_) => return ffi_error("FFI engine registry lock poisoned"),
    };
    if registry.remove(&request.engine_id).is_none() {
        return ffi_error(format!("FFI engine {} not found", request.engine_id));
    }
    ffi_ok(json!({ "freed": true }))
}

/// Load skills from legacy directory-style roots through the JSON FFI surface.
/// 通过 JSON FFI 入口按旧目录风格根参数加载技能。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_load_from_dirs_json(
    input_json: FfiBorrowedBuffer,
) -> FfiOwnedBuffer {
    let request = match decode_json_request::<EngineDirsJsonRequest>(
        input_json,
        "vulcan_luaskills_ffi_load_from_dirs_json",
    ) {
        Ok(request) => request,
        Err(error) => return ffi_error(error),
    };
    match with_engine_mut(request.engine_id, |engine| {
        engine
            .load_from_dirs(&request.base_dir, request.override_dir.as_deref())
            .map_err(|error| error.to_string())
    }) {
        Ok(()) => ffi_ok(json!({ "loaded": true })),
        Err(error) => ffi_error(error),
    }
}

/// Load skills from one ordered root chain through the JSON FFI surface.
/// 通过 JSON FFI 入口按有序根链加载技能。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_load_from_roots_json(
    input_json: FfiBorrowedBuffer,
) -> FfiOwnedBuffer {
    let request = match decode_json_request::<EngineRootsJsonRequest>(
        input_json,
        "vulcan_luaskills_ffi_load_from_roots_json",
    ) {
        Ok(request) => request,
        Err(error) => return ffi_error(error),
    };
    match with_engine_mut(request.engine_id, |engine| {
        engine
            .load_from_roots(&request.skill_roots)
            .map_err(|error| error.to_string())
    }) {
        Ok(()) => ffi_ok(json!({ "loaded": true })),
        Err(error) => ffi_error(error),
    }
}

/// Reload skills from legacy directory-style roots through the JSON FFI surface.
/// 通过 JSON FFI 入口按旧目录风格根参数重载技能。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_reload_from_dirs_json(
    input_json: FfiBorrowedBuffer,
) -> FfiOwnedBuffer {
    let request = match decode_json_request::<EngineDirsJsonRequest>(
        input_json,
        "vulcan_luaskills_ffi_reload_from_dirs_json",
    ) {
        Ok(request) => request,
        Err(error) => return ffi_error(error),
    };
    match with_engine_mut(request.engine_id, |engine| {
        engine
            .reload_from_dirs(&request.base_dir, request.override_dir.as_deref())
            .map_err(|error| error.to_string())
    }) {
        Ok(()) => ffi_ok(json!({ "reloaded": true })),
        Err(error) => ffi_error(error),
    }
}

/// Reload skills from one ordered root chain through the JSON FFI surface.
/// 通过 JSON FFI 入口按有序根链重载技能。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_reload_from_roots_json(
    input_json: FfiBorrowedBuffer,
) -> FfiOwnedBuffer {
    let request = match decode_json_request::<EngineRootsJsonRequest>(
        input_json,
        "vulcan_luaskills_ffi_reload_from_roots_json",
    ) {
        Ok(request) => request,
        Err(error) => return ffi_error(error),
    };
    match with_engine_mut(request.engine_id, |engine| {
        engine
            .reload_from_roots(&request.skill_roots)
            .map_err(|error| error.to_string())
    }) {
        Ok(()) => ffi_ok(json!({ "reloaded": true })),
        Err(error) => ffi_error(error),
    }
}

/// List runtime entry descriptors through the JSON FFI surface.
/// 通过 JSON FFI 入口列出运行时入口描述。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_list_entries_json(
    input_json: FfiBorrowedBuffer,
) -> FfiOwnedBuffer {
    let request = match decode_json_request::<EngineIdJsonRequest>(
        input_json,
        "vulcan_luaskills_ffi_list_entries_json",
    ) {
        Ok(request) => request,
        Err(error) => return ffi_error(error),
    };
    match with_engine(request.engine_id, |engine| Ok(engine.list_entries())) {
        Ok(result) => ffi_ok::<Vec<RuntimeEntryDescriptor>>(result),
        Err(error) => ffi_error(error),
    }
}

/// List structured help trees through the JSON FFI surface.
/// 通过 JSON FFI 入口列出结构化帮助树。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_list_skill_help_json(
    input_json: FfiBorrowedBuffer,
) -> FfiOwnedBuffer {
    let request = match decode_json_request::<EngineIdJsonRequest>(
        input_json,
        "vulcan_luaskills_ffi_list_skill_help_json",
    ) {
        Ok(request) => request,
        Err(error) => return ffi_error(error),
    };
    match with_engine(request.engine_id, |engine| Ok(engine.list_skill_help())) {
        Ok(result) => ffi_ok::<Vec<RuntimeSkillHelpDescriptor>>(result),
        Err(error) => ffi_error(error),
    }
}

/// Render one structured help detail payload through the JSON FFI surface.
/// 通过 JSON FFI 入口渲染单个结构化帮助详情载荷。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_render_skill_help_detail_json(
    input_json: FfiBorrowedBuffer,
) -> FfiOwnedBuffer {
    let request = match decode_json_request::<RenderHelpJsonRequest>(
        input_json,
        "vulcan_luaskills_ffi_render_skill_help_detail_json",
    ) {
        Ok(request) => request,
        Err(error) => return ffi_error(error),
    };
    match with_engine(request.engine_id, |engine| {
        engine.render_skill_help_detail(
            &request.skill_id,
            &request.flow_name,
            request.request_context.as_ref(),
        )
    }) {
        Ok(result) => ffi_ok::<Option<RuntimeHelpDetail>>(result),
        Err(error) => ffi_error(error),
    }
}

/// Resolve prompt argument completion candidates through the JSON FFI surface.
/// 通过 JSON FFI 入口解析提示词参数补全候选项。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_prompt_argument_completions_json(
    input_json: FfiBorrowedBuffer,
) -> FfiOwnedBuffer {
    let request = match decode_json_request::<PromptCompletionJsonRequest>(
        input_json,
        "vulcan_luaskills_ffi_prompt_argument_completions_json",
    ) {
        Ok(request) => request,
        Err(error) => return ffi_error(error),
    };
    match with_engine(request.engine_id, |engine| {
        Ok(engine.prompt_argument_completions(&request.prompt_name, &request.argument_name))
    }) {
        Ok(result) => ffi_ok::<Option<Vec<String>>>(result),
        Err(error) => ffi_error(error),
    }
}

/// Check whether one canonical tool name belongs to one Lua skill entry.
/// 检查某个 canonical 工具名是否属于 Lua 技能入口。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_is_skill_json(
    input_json: FfiBorrowedBuffer,
) -> FfiOwnedBuffer {
    let request = match decode_json_request::<IsSkillJsonRequest>(
        input_json,
        "vulcan_luaskills_ffi_is_skill_json",
    ) {
        Ok(request) => request,
        Err(error) => return ffi_error(error),
    };
    match with_engine(request.engine_id, |engine| {
        Ok(engine.is_skill(&request.tool_name))
    }) {
        Ok(value) => ffi_ok(BoolJsonResult { value }),
        Err(error) => ffi_error(error),
    }
}

/// Resolve the owning skill id of one canonical tool name through the JSON FFI surface.
/// 通过 JSON FFI 入口解析某个 canonical 工具名所属的技能标识符。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_skill_name_for_tool_json(
    input_json: FfiBorrowedBuffer,
) -> FfiOwnedBuffer {
    let request = match decode_json_request::<SkillNameForToolJsonRequest>(
        input_json,
        "vulcan_luaskills_ffi_skill_name_for_tool_json",
    ) {
        Ok(request) => request,
        Err(error) => return ffi_error(error),
    };
    match with_engine(request.engine_id, |engine| {
        Ok(engine.skill_name_for_tool(&request.tool_name))
    }) {
        Ok(skill_id) => ffi_ok(OptionalSkillNameJsonResult { skill_id }),
        Err(error) => ffi_error(error),
    }
}

/// Call one loaded skill entry through the JSON FFI surface.
/// 通过 JSON FFI 入口调用单个已加载技能入口。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_call_skill_json(
    input_json: FfiBorrowedBuffer,
) -> FfiOwnedBuffer {
    let request = match decode_json_request::<CallSkillJsonRequest>(
        input_json,
        "vulcan_luaskills_ffi_call_skill_json",
    ) {
        Ok(request) => request,
        Err(error) => return ffi_error(error),
    };
    match with_engine(request.engine_id, |engine| {
        engine.call_skill(
            &request.tool_name,
            &request.args,
            request.invocation_context.as_ref(),
        )
    }) {
        Ok(result) => ffi_ok::<RuntimeInvocationResult>(result),
        Err(error) => ffi_error(error),
    }
}

/// Execute arbitrary Lua code through the JSON FFI surface.
/// 通过 JSON FFI 入口执行任意 Lua 代码。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_run_lua_json(
    input_json: FfiBorrowedBuffer,
) -> FfiOwnedBuffer {
    let request = match decode_json_request::<RunLuaJsonRequest>(
        input_json,
        "vulcan_luaskills_ffi_run_lua_json",
    ) {
        Ok(request) => request,
        Err(error) => return ffi_error(error),
    };
    match with_engine(request.engine_id, |engine| {
        engine.run_lua(
            &request.code,
            &request.args,
            request.invocation_context.as_ref(),
        )
    }) {
        Ok(result) => ffi_ok::<Value>(result),
        Err(error) => ffi_error(error),
    }
}

/// Disable one skill through the ordinary skills plane via the JSON FFI surface.
/// 通过 JSON FFI 入口在普通 skills 平面停用单个技能。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_disable_skill_in_dirs_json(
    input_json: FfiBorrowedBuffer,
) -> FfiOwnedBuffer {
    let request = match decode_json_request::<DisableSkillDirsJsonRequest>(
        input_json,
        "vulcan_luaskills_ffi_disable_skill_in_dirs_json",
    ) {
        Ok(request) => request,
        Err(error) => return ffi_error(error),
    };
    match with_engine_mut(request.engine_id, |engine| {
        engine
            .disable_skill(
                &request.base_dir,
                request.override_dir.as_deref(),
                &request.skill_id,
                request.reason.as_deref(),
            )
            .map_err(|error| error.to_string())
    }) {
        Ok(()) => ffi_ok(json!({ "disabled": true })),
        Err(error) => ffi_error(error),
    }
}

/// Disable one skill through the ordinary skills plane via the JSON FFI surface.
/// 通过 JSON FFI 入口在普通 skills 平面停用单个技能。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_disable_skill_json(
    input_json: FfiBorrowedBuffer,
) -> FfiOwnedBuffer {
    let request = match decode_json_request::<DisableSkillJsonRequest>(
        input_json,
        "vulcan_luaskills_ffi_disable_skill_json",
    ) {
        Ok(request) => request,
        Err(error) => return ffi_error(error),
    };
    match with_engine_mut(request.engine_id, |engine| {
        engine
            .disable_skill_in_roots(
                &request.skill_roots,
                &request.skill_id,
                request.reason.as_deref(),
            )
            .map_err(|error| error.to_string())
    }) {
        Ok(()) => ffi_ok(json!({ "disabled": true })),
        Err(error) => ffi_error(error),
    }
}

/// Disable one skill through the system plane via the JSON FFI surface.
/// 通过 JSON FFI 入口在 system 平面停用单个技能。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_system_disable_skill_in_dirs_json(
    input_json: FfiBorrowedBuffer,
) -> FfiOwnedBuffer {
    let request = match decode_json_request::<DisableSkillDirsJsonRequest>(
        input_json,
        "vulcan_luaskills_ffi_system_disable_skill_in_dirs_json",
    ) {
        Ok(request) => request,
        Err(error) => return ffi_error(error),
    };
    match with_engine_mut(request.engine_id, |engine| {
        engine
            .system_disable_skill(
                &request.base_dir,
                request.override_dir.as_deref(),
                &request.skill_id,
                request.reason.as_deref(),
            )
            .map_err(|error| error.to_string())
    }) {
        Ok(()) => ffi_ok(json!({ "disabled": true })),
        Err(error) => ffi_error(error),
    }
}

/// Disable one skill through the system plane via the JSON FFI surface.
/// 通过 JSON FFI 入口在 system 平面停用单个技能。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_system_disable_skill_json(
    input_json: FfiBorrowedBuffer,
) -> FfiOwnedBuffer {
    let request = match decode_json_request::<DisableSkillJsonRequest>(
        input_json,
        "vulcan_luaskills_ffi_system_disable_skill_json",
    ) {
        Ok(request) => request,
        Err(error) => return ffi_error(error),
    };
    match with_engine_mut(request.engine_id, |engine| {
        engine
            .system_disable_skill_in_roots(
                &request.skill_roots,
                &request.skill_id,
                request.reason.as_deref(),
            )
            .map_err(|error| error.to_string())
    }) {
        Ok(()) => ffi_ok(json!({ "disabled": true })),
        Err(error) => ffi_error(error),
    }
}

/// Enable one skill through the ordinary skills plane via the JSON FFI surface.
/// 通过 JSON FFI 入口在普通 skills 平面启用单个技能。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_enable_skill_json(
    input_json: FfiBorrowedBuffer,
) -> FfiOwnedBuffer {
    let request = match decode_json_request::<EnableSkillJsonRequest>(
        input_json,
        "vulcan_luaskills_ffi_enable_skill_json",
    ) {
        Ok(request) => request,
        Err(error) => return ffi_error(error),
    };
    match with_engine_mut(request.engine_id, |engine| {
        engine
            .enable_skill(&request.skill_roots, &request.skill_id)
            .map_err(|error| error.to_string())
    }) {
        Ok(()) => ffi_ok(json!({ "enabled": true })),
        Err(error) => ffi_error(error),
    }
}

/// Enable one skill through the system plane via the JSON FFI surface.
/// 通过 JSON FFI 入口在 system 平面启用单个技能。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_system_enable_skill_json(
    input_json: FfiBorrowedBuffer,
) -> FfiOwnedBuffer {
    let request = match decode_json_request::<EnableSkillJsonRequest>(
        input_json,
        "vulcan_luaskills_ffi_system_enable_skill_json",
    ) {
        Ok(request) => request,
        Err(error) => return ffi_error(error),
    };
    match with_engine_mut(request.engine_id, |engine| {
        engine
            .system_enable_skill(&request.skill_roots, &request.skill_id)
            .map_err(|error| error.to_string())
    }) {
        Ok(()) => ffi_ok(json!({ "enabled": true })),
        Err(error) => ffi_error(error),
    }
}

/// Uninstall one skill through the ordinary skills plane via the JSON FFI surface.
/// 通过 JSON FFI 入口在普通 skills 平面卸载单个技能。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_uninstall_skill_json(
    input_json: FfiBorrowedBuffer,
) -> FfiOwnedBuffer {
    let request = match decode_json_request::<UninstallSkillJsonRequest>(
        input_json,
        "vulcan_luaskills_ffi_uninstall_skill_json",
    ) {
        Ok(request) => request,
        Err(error) => return ffi_error(error),
    };
    match with_engine_mut(request.engine_id, |engine| {
        engine
            .uninstall_skill(&request.skill_roots, &request.skill_id, &request.options)
            .map_err(|error| error.to_string())
    }) {
        Ok(result) => ffi_ok::<SkillUninstallResult>(result),
        Err(error) => ffi_error(error),
    }
}

/// Uninstall one skill through the system plane via the JSON FFI surface.
/// 通过 JSON FFI 入口在 system 平面卸载单个技能。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_system_uninstall_skill_json(
    input_json: FfiBorrowedBuffer,
) -> FfiOwnedBuffer {
    let request = match decode_json_request::<UninstallSkillJsonRequest>(
        input_json,
        "vulcan_luaskills_ffi_system_uninstall_skill_json",
    ) {
        Ok(request) => request,
        Err(error) => return ffi_error(error),
    };
    match with_engine_mut(request.engine_id, |engine| {
        engine
            .system_uninstall_skill(&request.skill_roots, &request.skill_id, &request.options)
            .map_err(|error| error.to_string())
    }) {
        Ok(result) => ffi_ok::<SkillUninstallResult>(result),
        Err(error) => ffi_error(error),
    }
}

/// Install one managed skill through the ordinary skills plane via the JSON FFI surface.
/// 通过 JSON FFI 入口在普通 skills 平面安装单个受管技能。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_install_skill_json(
    input_json: FfiBorrowedBuffer,
) -> FfiOwnedBuffer {
    let request = match decode_json_request::<ApplySkillJsonRequest>(
        input_json,
        "vulcan_luaskills_ffi_install_skill_json",
    ) {
        Ok(request) => request,
        Err(error) => return ffi_error(error),
    };
    match with_engine_mut(request.engine_id, |engine| {
        engine
            .install_skill(&request.skill_roots, &request.request)
            .map_err(|error| error.to_string())
    }) {
        Ok(result) => ffi_ok::<SkillApplyResult>(result),
        Err(error) => ffi_error(error),
    }
}

/// Install one managed skill through the system plane via the JSON FFI surface.
/// 通过 JSON FFI 入口在 system 平面安装单个受管技能。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_system_install_skill_json(
    input_json: FfiBorrowedBuffer,
) -> FfiOwnedBuffer {
    let request = match decode_json_request::<ApplySkillJsonRequest>(
        input_json,
        "vulcan_luaskills_ffi_system_install_skill_json",
    ) {
        Ok(request) => request,
        Err(error) => return ffi_error(error),
    };
    match with_engine_mut(request.engine_id, |engine| {
        engine
            .system_install_skill(&request.skill_roots, &request.request)
            .map_err(|error| error.to_string())
    }) {
        Ok(result) => ffi_ok::<SkillApplyResult>(result),
        Err(error) => ffi_error(error),
    }
}

/// Update one managed skill through the ordinary skills plane via the JSON FFI surface.
/// 通过 JSON FFI 入口在普通 skills 平面更新单个受管技能。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_update_skill_json(
    input_json: FfiBorrowedBuffer,
) -> FfiOwnedBuffer {
    let request = match decode_json_request::<ApplySkillJsonRequest>(
        input_json,
        "vulcan_luaskills_ffi_update_skill_json",
    ) {
        Ok(request) => request,
        Err(error) => return ffi_error(error),
    };
    match with_engine_mut(request.engine_id, |engine| {
        engine
            .update_skill(&request.skill_roots, &request.request)
            .map_err(|error| error.to_string())
    }) {
        Ok(result) => ffi_ok::<SkillApplyResult>(result),
        Err(error) => ffi_error(error),
    }
}

/// Update one managed skill through the system plane via the JSON FFI surface.
/// 通过 JSON FFI 入口在 system 平面更新单个受管技能。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_system_update_skill_json(
    input_json: FfiBorrowedBuffer,
) -> FfiOwnedBuffer {
    let request = match decode_json_request::<ApplySkillJsonRequest>(
        input_json,
        "vulcan_luaskills_ffi_system_update_skill_json",
    ) {
        Ok(request) => request,
        Err(error) => return ffi_error(error),
    };
    match with_engine_mut(request.engine_id, |engine| {
        engine
            .system_update_skill(&request.skill_roots, &request.request)
            .map_err(|error| error.to_string())
    }) {
        Ok(result) => ffi_ok::<SkillApplyResult>(result),
        Err(error) => ffi_error(error),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        EngineHandleJsonResult, EngineIdJsonRequest, EngineNewJsonRequest, FFI_ENGINE_COUNTER,
        FfiEngineSlot, ffi_engine_registry, vulcan_luaskills_ffi_engine_free_json,
        vulcan_luaskills_ffi_engine_new_json, with_engine,
    };
    use crate::ffi_standard::{
        FfiBorrowedBuffer, FfiOwnedBuffer, vulcan_luaskills_ffi_buffer_free,
    };
    use crate::{LuaEngine, LuaEngineOptions, LuaVmPoolConfig};
    use std::ffi::CString;
    use std::sync::atomic::Ordering;

    /// Read one FFI JSON response string back into one serde_json value.
    /// 将单个 FFI JSON 响应字符串回读为一个 serde_json 值。
    unsafe fn decode_response_json(buffer: FfiOwnedBuffer) -> serde_json::Value {
        let bytes = if buffer.ptr.is_null() {
            assert_eq!(buffer.len, 0, "null response pointer must have zero len");
            &[][..]
        } else {
            unsafe { std::slice::from_raw_parts(buffer.ptr, buffer.len) }
        };
        let text = std::str::from_utf8(bytes).expect("ffi json must be utf-8");
        let value = serde_json::from_str(text).expect("ffi json must parse");
        unsafe { vulcan_luaskills_ffi_buffer_free(buffer) };
        value
    }

    /// Build one borrowed buffer view over one CString JSON payload for JSON FFI tests.
    /// 为 JSON FFI 测试中的单个 CString JSON 载荷构造一个借用缓冲视图。
    fn borrowed_json_buffer(value: &CString) -> FfiBorrowedBuffer {
        let bytes = value.as_bytes();
        FfiBorrowedBuffer {
            ptr: bytes.as_ptr(),
            len: bytes.len(),
        }
    }

    /// One test-only registered engine handle that cleans itself from the global registry on drop.
    /// 一个仅供测试使用的已注册引擎句柄，并在释放时自动从全局注册表清理。
    struct TestFfiEngineHandle {
        engine_id: u64,
    }

    impl Drop for TestFfiEngineHandle {
        fn drop(&mut self) {
            if let Ok(mut registry) = ffi_engine_registry().lock() {
                registry.remove(&self.engine_id);
            }
        }
    }

    /// Register one minimal engine into the global FFI registry for concurrency tests.
    /// 将一个最小引擎注册到全局 FFI 注册表中，用于并发相关测试。
    fn register_test_engine() -> TestFfiEngineHandle {
        let engine = LuaEngine::new(LuaEngineOptions::new(
            LuaVmPoolConfig {
                min_size: 1,
                max_size: 1,
                idle_ttl_secs: 30,
            },
            crate::LuaRuntimeHostOptions::default(),
        ))
        .expect("create ffi test engine");
        let engine_id = FFI_ENGINE_COUNTER.fetch_add(1, Ordering::Relaxed);
        ffi_engine_registry()
            .lock()
            .expect("lock ffi engine registry")
            .insert(engine_id, FfiEngineSlot::new(engine));
        TestFfiEngineHandle { engine_id }
    }

    /// Verify that one engine can be created and freed through the JSON FFI surface.
    /// 验证可以通过 JSON FFI 入口创建并释放单个引擎。
    #[test]
    fn ffi_engine_new_and_free_roundtrip() {
        let temp_root = std::env::temp_dir().join(format!(
            "vulcan_luaskills_ffi_engine_test_{}",
            std::process::id()
        ));
        let request = EngineNewJsonRequest {
            options: LuaEngineOptions::new(
                LuaVmPoolConfig {
                    min_size: 1,
                    max_size: 1,
                    idle_ttl_secs: 30,
                },
                crate::LuaRuntimeHostOptions {
                    temp_dir: Some(temp_root.join("temp")),
                    resources_dir: Some(temp_root.join("resources")),
                    lua_packages_dir: Some(temp_root.join("lua_packages")),
                    host_provided_tool_root: Some(temp_root.join("bin").join("tools")),
                    host_provided_lua_root: Some(temp_root.join("lua_packages")),
                    host_provided_ffi_root: Some(temp_root.join("libs")),
                    download_cache_root: Some(temp_root.join("temp").join("downloads")),
                    dependency_dir_name: "dependencies".to_string(),
                    state_dir_name: "state".to_string(),
                    database_dir_name: "databases".to_string(),
                    protection: Default::default(),
                    allow_network_download: false,
                    github_base_url: None,
                    github_api_base_url: None,
                    sqlite_library_path: None,
                    sqlite_provider_mode: crate::LuaRuntimeDatabaseProviderMode::DynamicLibrary,
                    sqlite_callback_mode: crate::LuaRuntimeDatabaseCallbackMode::Standard,
                    lancedb_library_path: None,
                    lancedb_provider_mode: crate::LuaRuntimeDatabaseProviderMode::DynamicLibrary,
                    lancedb_callback_mode: crate::LuaRuntimeDatabaseCallbackMode::Standard,
                    space_controller: crate::LuaRuntimeSpaceControllerOptions::default(),
                    cache_config: None,
                    runlua_pool_config: None,
                    reserved_entry_names: Vec::new(),
                    ignored_skill_ids: Vec::new(),
                    capabilities: Default::default(),
                },
            ),
        };
        let input = CString::new(serde_json::to_string(&request).expect("request json"))
            .expect("request cstring");
        let response = unsafe {
            decode_response_json(vulcan_luaskills_ffi_engine_new_json(borrowed_json_buffer(
                &input,
            )))
        };
        assert_eq!(response["ok"], true);
        let result: EngineHandleJsonResult =
            serde_json::from_value(response["result"].clone()).expect("engine result should parse");

        let free_request = CString::new(
            serde_json::to_string(&EngineIdJsonRequest {
                engine_id: result.engine_id,
            })
            .expect("free request json"),
        )
        .expect("free request cstring");
        let free_response = unsafe {
            decode_response_json(vulcan_luaskills_ffi_engine_free_json(borrowed_json_buffer(
                &free_request,
            )))
        };
        assert_eq!(free_response["ok"], true);
    }

    /// Verify that one engine operation no longer keeps the global registry mutex while running.
    /// 验证单次引擎操作执行期间不会继续持有全局注册表互斥锁。
    #[test]
    fn with_engine_releases_registry_lock_before_operation() {
        let handle = register_test_engine();
        let result = with_engine(handle.engine_id, |_engine| {
            let registry_lock = ffi_engine_registry().try_lock();
            assert!(
                registry_lock.is_ok(),
                "registry lock should be acquirable while engine operation is running"
            );
            Ok(())
        });
        assert!(result.is_ok());
    }

    /// Verify that same-thread reentrant access returns an explicit error instead of deadlocking.
    /// 验证同线程重入访问会返回明确错误，而不是直接死锁。
    #[test]
    fn with_engine_rejects_same_thread_reentry() {
        let handle = register_test_engine();
        let outer_result = with_engine(handle.engine_id, |_engine| {
            let nested_result = with_engine(handle.engine_id, |_nested| Ok(()));
            let nested_error = nested_result.expect_err("same-thread reentry should fail");
            assert!(nested_error.contains("reentrant access"));
            Ok(())
        });
        assert!(outer_result.is_ok());
    }
}
