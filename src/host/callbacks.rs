use crate::runtime::entry::RuntimeEntryDescriptor;
use crate::skill::manager::{SkillLifecycleAction, SkillManagementAuthority, SkillOperationPlane};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::{Arc, Mutex, OnceLock};

/// Callback type used by hosts to receive runtime skill-lifecycle events.
/// 宿主用于接收运行时技能生命周期事件的回调类型。
pub type RuntimeSkillLifecycleCallback = Arc<dyn Fn(&RuntimeSkillLifecycleEvent) + Send + Sync>;

/// Callback type used by hosts to receive runtime entry-registry change events.
/// 宿主用于接收运行时入口注册表变化事件的回调类型。
pub type RuntimeEntryRegistryCallback = Arc<dyn Fn(&RuntimeEntryRegistryDelta) + Send + Sync>;

/// Callback type used by hosts to handle one Lua-triggered runtime skill-management request.
/// 宿主用于处理单个由 Lua 触发的运行时技能管理请求的回调类型。
pub type RuntimeSkillManagementCallback =
    Arc<dyn Fn(&RuntimeSkillManagementRequest) -> Result<Value, String> + Send + Sync>;

/// Callback type used by hosts to handle one Lua-triggered host-tool bridge request.
/// 宿主用于处理单个由 Lua 触发的宿主工具桥接请求的回调类型。
pub type RuntimeHostToolCallback =
    Arc<dyn Fn(&RuntimeHostToolRequest) -> Result<Value, String> + Send + Sync>;

/// Callback type used by hosts to handle one standard model embedding request.
/// 宿主用于处理单个标准模型 embedding 请求的回调类型。
pub type RuntimeModelEmbedCallback = Arc<
    dyn Fn(&RuntimeModelEmbedRequest) -> Result<RuntimeModelEmbedResponse, RuntimeModelError>
        + Send
        + Sync,
>;

/// Callback type used by hosts to handle one standard non-streaming LLM request.
/// 宿主用于处理单个标准非流式 LLM 请求的回调类型。
pub type RuntimeModelLlmCallback = Arc<
    dyn Fn(&RuntimeModelLlmRequest) -> Result<RuntimeModelLlmResponse, RuntimeModelError>
        + Send
        + Sync,
>;

/// Structured host-tool bridge actions that Lua may request through `vulcan.host.*`.
/// Lua 可以通过 `vulcan.host.*` 请求的结构化宿主工具桥接动作集合。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeHostToolAction {
    /// Request the current host-visible tool list.
    /// 请求当前对宿主可见的工具列表。
    List,
    /// Request whether one host tool exists.
    /// 请求判断某个宿主工具是否存在。
    Has,
    /// Request one host-tool invocation.
    /// 请求执行一次宿主工具调用。
    Call,
}

/// Structured Lua-triggered host-tool bridge request forwarded to the host.
/// 转发给宿主的结构化 Lua 触发宿主工具桥接请求。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuntimeHostToolRequest {
    /// Requested host-tool bridge action kind.
    /// 请求的宿主工具桥接动作类型。
    pub action: RuntimeHostToolAction,
    /// Optional host tool name, required for `has` and `call`.
    /// 可选宿主工具名称，`has` 与 `call` 动作必填。
    pub tool_name: Option<String>,
    /// JSON payload converted from the Lua table argument.
    /// 从 Lua table 参数转换得到的 JSON 载荷。
    pub args: Value,
}

/// Caller context automatically attached to one standard model capability request.
/// 自动附加到单个标准模型能力请求上的调用方上下文。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeModelCaller {
    /// Skill identifier that owns the currently executing Lua entry when available.
    /// 当前执行 Lua 入口所属的 skill 标识符（如果存在）。
    pub skill_id: Option<String>,
    /// Local entry name declared by the owning skill when available.
    /// 所属 skill 声明的局部入口名称（如果存在）。
    pub entry_name: Option<String>,
    /// Canonical runtime tool name currently executing when available.
    /// 当前正在执行的 canonical 运行时工具名称（如果存在）。
    pub canonical_tool_name: Option<String>,
    /// Runtime root name that owns the current skill when available.
    /// 拥有当前 skill 的运行时根名称（如果存在）。
    pub root_name: Option<String>,
    /// Host-visible absolute skill directory when available.
    /// 对宿主可见的绝对 skill 目录（如果存在）。
    pub skill_dir: Option<String>,
    /// Host-provided client name from the current request context when available.
    /// 当前请求上下文中的宿主提供客户端名称（如果存在）。
    pub client_name: Option<String>,
    /// Host-provided request identifier from the current request context when available.
    /// 当前请求上下文中的宿主提供请求标识符（如果存在）。
    pub request_id: Option<String>,
}

/// Standard embedding request passed from LuaSkills into the host callback.
/// 从 LuaSkills 传递给宿主回调的标准 embedding 请求。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuntimeModelEmbedRequest {
    /// Single input text that Lua requested to embed.
    /// Lua 请求进行 embedding 的单条输入文本。
    pub text: String,
    /// Caller context captured from the active Lua runtime scope.
    /// 从当前 Lua 运行时作用域捕获的调用方上下文。
    pub caller: RuntimeModelCaller,
}

/// Standard embedding response returned by the host callback.
/// 宿主回调返回的标准 embedding 响应。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuntimeModelEmbedResponse {
    /// Embedding vector returned by the host-managed model provider.
    /// 宿主管理的模型 provider 返回的 embedding 向量。
    pub vector: Vec<f32>,
    /// Number of vector dimensions reported by the host.
    /// 宿主报告的向量维度数量。
    pub dimensions: usize,
    /// Optional token usage metadata reported by the host.
    /// 宿主报告的可选 token 用量元数据。
    pub usage: Option<RuntimeModelUsage>,
}

/// Standard non-streaming LLM request passed from LuaSkills into the host callback.
/// 从 LuaSkills 传递给宿主回调的标准非流式 LLM 请求。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuntimeModelLlmRequest {
    /// System instruction text supplied by Lua.
    /// Lua 提供的 system 指令文本。
    pub system: String,
    /// User message text supplied by Lua.
    /// Lua 提供的 user 消息文本。
    pub user: String,
    /// Caller context captured from the active Lua runtime scope.
    /// 从当前 Lua 运行时作用域捕获的调用方上下文。
    pub caller: RuntimeModelCaller,
}

/// Standard non-streaming LLM response returned by the host callback.
/// 宿主回调返回的标准非流式 LLM 响应。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeModelLlmResponse {
    /// Assistant text returned by the host-managed model provider.
    /// 宿主管理的模型 provider 返回的 assistant 文本。
    pub assistant: String,
    /// Optional token usage metadata reported by the host.
    /// 宿主报告的可选 token 用量元数据。
    pub usage: Option<RuntimeModelUsage>,
}

/// Optional token usage metadata returned by a host-managed model provider.
/// 宿主管理的模型 provider 返回的可选 token 用量元数据。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeModelUsage {
    /// Optional input token count reported by the host.
    /// 宿主报告的可选输入 token 数量。
    pub input_tokens: Option<u64>,
    /// Optional output token count reported by the host.
    /// 宿主报告的可选输出 token 数量。
    pub output_tokens: Option<u64>,
}

/// Structured model error returned by LuaSkills or the host model callback.
/// LuaSkills 或宿主模型回调返回的结构化模型错误。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeModelError {
    /// Stable LuaSkills-level model error code.
    /// 稳定的 LuaSkills 级模型错误码。
    pub code: RuntimeModelErrorCode,
    /// Human-readable error summary.
    /// 人类可读的错误摘要。
    pub message: String,
    /// Optional raw provider error text after host-side redaction.
    /// 宿主侧脱敏后的可选 provider 原始错误文本。
    pub provider_message: Option<String>,
    /// Optional raw provider error code.
    /// 可选 provider 原始错误码。
    pub provider_code: Option<String>,
    /// Optional provider status such as an HTTP status code.
    /// 可选 provider 状态，例如 HTTP 状态码。
    pub provider_status: Option<u16>,
}

/// Stable model error codes exposed through the Lua result envelope.
/// 通过 Lua 返回包络暴露的稳定模型错误码。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeModelErrorCode {
    /// The requested model capability has no registered host callback.
    /// 请求的模型能力没有注册宿主回调。
    ModelUnavailable,
    /// Lua supplied an invalid argument count, type, or empty string.
    /// Lua 提供了非法参数数量、类型或空字符串。
    InvalidArgument,
    /// The host model provider returned an error.
    /// 宿主模型 provider 返回了错误。
    ProviderError,
    /// The host model invocation timed out.
    /// 宿主模型调用超时。
    Timeout,
    /// The host rejected the invocation because a budget or limit was exceeded.
    /// 宿主因预算或限制超额拒绝了本次调用。
    BudgetExceeded,
    /// LuaSkills or the host bridge hit an internal failure.
    /// LuaSkills 或宿主桥接遇到内部故障。
    InternalError,
}

impl RuntimeModelErrorCode {
    /// Return the stable snake_case code string used by Lua result envelopes.
    /// 返回 Lua 返回包络使用的稳定 snake_case 错误码字符串。
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ModelUnavailable => "model_unavailable",
            Self::InvalidArgument => "invalid_argument",
            Self::ProviderError => "provider_error",
            Self::Timeout => "timeout",
            Self::BudgetExceeded => "budget_exceeded",
            Self::InternalError => "internal_error",
        }
    }

    /// Convert a stable snake_case model error code into the internal enum.
    /// 将稳定的 snake_case 模型错误码转换为内部枚举。
    pub fn from_code_str(value: &str) -> Self {
        match value {
            "model_unavailable" => Self::ModelUnavailable,
            "invalid_argument" => Self::InvalidArgument,
            "provider_error" => Self::ProviderError,
            "timeout" => Self::Timeout,
            "budget_exceeded" => Self::BudgetExceeded,
            _ => Self::InternalError,
        }
    }
}

/// Structured management actions that one Lua-exposed runtime bridge may request from the host.
/// Lua 暴露的运行时桥接可能向宿主请求的结构化管理动作集合。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeSkillManagementAction {
    /// Request one managed install operation.
    /// 请求执行一次受管安装操作。
    Install,
    /// Request one managed update operation.
    /// 请求执行一次受管更新操作。
    Update,
    /// Request one uninstall operation.
    /// 请求执行一次卸载操作。
    Uninstall,
    /// Request one enable operation.
    /// 请求执行一次启用操作。
    Enable,
    /// Request one disable operation.
    /// 请求执行一次停用操作。
    Disable,
}

/// Structured Lua-triggered skill-management request forwarded to the host.
/// 转发给宿主的结构化 Lua 触发技能管理请求。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuntimeSkillManagementRequest {
    /// Requested management action kind.
    /// 请求的管理动作类型。
    pub action: RuntimeSkillManagementAction,
    /// Host-injected authority level for this ordinary runtime bridge request.
    /// 当前普通运行时桥接请求的宿主注入权限等级。
    pub authority: SkillManagementAuthority,
    /// Arbitrary JSON payload supplied by the Lua caller.
    /// 由 Lua 调用方提供的任意 JSON 载荷。
    pub input: Value,
}

/// Structured lifecycle event emitted after one skill-management operation is evaluated.
/// 在评估一次技能管理操作后发出的结构化生命周期事件。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RuntimeSkillLifecycleEvent {
    /// Operation plane that triggered the lifecycle event.
    /// 触发生命周期事件的操作平面。
    pub plane: SkillOperationPlane,
    /// Lifecycle action represented by the current event.
    /// 当前事件所表示的生命周期动作。
    pub action: SkillLifecycleAction,
    /// Skill identifier targeted by the current lifecycle operation.
    /// 当前生命周期操作对应的技能标识符。
    pub skill_id: String,
    /// Optional named skill root that owns the effective target skill instance.
    /// 拥有当前生效目标技能实例的可选命名技能根。
    pub root_name: Option<String>,
    /// Optional physical skill directory of the effective target skill instance.
    /// 当前生效目标技能实例的可选物理技能目录。
    pub skill_dir: Option<String>,
    /// High-level event status such as completed, failed, or blocked.
    /// 当前事件的高层状态，例如 completed、failed 或 blocked。
    pub status: String,
    /// Optional human-readable explanation of the current lifecycle outcome.
    /// 当前生命周期结果的可选人类可读说明。
    pub message: Option<String>,
}

/// Structured entry-registry delta emitted when one reload changes exposed runtime entries.
/// 当一次重载改变已暴露运行时入口时发出的结构化入口注册表差异。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RuntimeEntryRegistryDelta {
    /// Newly added runtime entries after the latest reload.
    /// 最近一次重载后新增的运行时入口。
    pub added_entries: Vec<RuntimeEntryDescriptor>,
    /// Canonical names removed after the latest reload.
    /// 最近一次重载后移除的 canonical 入口名称。
    pub removed_entry_names: Vec<String>,
    /// Existing canonical entries whose structure changed after the latest reload.
    /// 最近一次重载后结构发生变化的既有 canonical 入口。
    pub updated_entries: Vec<RuntimeEntryDescriptor>,
}

/// Install or clear the process-wide skill-lifecycle callback used by the host.
/// 安装或清理供宿主使用的进程级技能生命周期回调。
pub fn set_skill_lifecycle_callback(callback: Option<RuntimeSkillLifecycleCallback>) {
    let registry = skill_lifecycle_callback_registry();
    let mut guard = registry.lock().unwrap();
    *guard = callback;
}

/// Install or clear the process-wide entry-registry callback used by the host.
/// 安装或清理供宿主使用的进程级入口注册表回调。
pub fn set_entry_registry_callback(callback: Option<RuntimeEntryRegistryCallback>) {
    let registry = entry_registry_callback_registry();
    let mut guard = registry.lock().unwrap();
    *guard = callback;
}

/// Install or clear the process-wide Lua-triggered skill-management callback used by the host.
/// 安装或清理由宿主使用的进程级 Lua 触发技能管理回调。
pub fn set_skill_management_callback(callback: Option<RuntimeSkillManagementCallback>) {
    let registry = skill_management_callback_registry();
    let mut guard = registry.lock().unwrap();
    *guard = callback;
}

/// Install or clear the process-wide Lua-triggered host-tool callback used by the host.
/// 安装或清理由宿主使用的进程级 Lua 触发宿主工具回调。
pub fn set_host_tool_callback(callback: Option<RuntimeHostToolCallback>) {
    let registry = host_tool_callback_registry();
    let mut guard = registry.lock().unwrap();
    *guard = callback;
}

/// Install or clear the process-wide standard model embedding callback used by the host.
/// 安装或清理由宿主使用的进程级标准模型 embedding 回调。
pub fn set_model_embed_callback(callback: Option<RuntimeModelEmbedCallback>) {
    let registry = model_embed_callback_registry();
    let mut guard = registry.lock().unwrap();
    *guard = callback;
}

/// Install or clear the process-wide standard non-streaming LLM callback used by the host.
/// 安装或清理由宿主使用的进程级标准非流式 LLM 回调。
pub fn set_model_llm_callback(callback: Option<RuntimeModelLlmCallback>) {
    let registry = model_llm_callback_registry();
    let mut guard = registry.lock().unwrap();
    *guard = callback;
}

/// Guard one process-wide model callback test and clear global callback state on drop.
/// 保护单个进程级模型回调测试，并在释放时清理全局回调状态。
#[cfg(test)]
pub(crate) struct RuntimeModelCallbackTestGuard {
    /// Hold the process-wide mutex guard until the current test finishes.
    /// 持有进程级互斥锁直到当前测试结束。
    _guard: std::sync::MutexGuard<'static, ()>,
}

#[cfg(test)]
impl Drop for RuntimeModelCallbackTestGuard {
    /// Clear global model callbacks when one guarded test finishes.
    /// 当受保护测试结束时清理全局模型回调。
    fn drop(&mut self) {
        set_model_embed_callback(None);
        set_model_llm_callback(None);
    }
}

/// Acquire the process-wide model callback test guard.
/// 获取进程级模型回调测试保护锁。
#[cfg(test)]
pub(crate) fn runtime_model_callback_test_guard() -> RuntimeModelCallbackTestGuard {
    static GUARD: OnceLock<Mutex<()>> = OnceLock::new();
    let guard = GUARD
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("lock model callback test guard");
    set_model_embed_callback(None);
    set_model_llm_callback(None);
    RuntimeModelCallbackTestGuard { _guard: guard }
}

/// Emit one skill-lifecycle event to the currently registered host callback when it exists.
/// 当宿主已注册回调时向其发送一条技能生命周期事件。
pub(crate) fn emit_skill_lifecycle_event(event: &RuntimeSkillLifecycleEvent) {
    let registry = skill_lifecycle_callback_registry();
    let callback = {
        let guard = registry.lock().unwrap();
        guard.clone()
    };
    if let Some(callback) = callback {
        callback(event);
    }
}

/// Emit one entry-registry delta to the currently registered host callback when it exists.
/// 当宿主已注册回调时向其发送一条入口注册表差异事件。
pub(crate) fn emit_entry_registry_delta(delta: &RuntimeEntryRegistryDelta) {
    let registry = entry_registry_callback_registry();
    let callback = {
        let guard = registry.lock().unwrap();
        guard.clone()
    };
    if let Some(callback) = callback {
        callback(delta);
    }
}

/// Dispatch one Lua-triggered skill-management request into the currently registered host callback.
/// 将单个 Lua 触发的技能管理请求分发给当前已注册的宿主回调。
pub(crate) fn dispatch_skill_management_request(
    request: &RuntimeSkillManagementRequest,
) -> Result<Value, String> {
    let registry = skill_management_callback_registry();
    let callback = {
        let guard = registry
            .lock()
            .map_err(|_| "Skill management callback registry lock poisoned".to_string())?;
        guard.clone()
    };
    let callback = callback.ok_or_else(|| {
        "Runtime skill management bridge is enabled but no host callback is registered".to_string()
    })?;
    callback(request)
}

/// Dispatch one Lua-triggered host-tool request into the currently registered host callback.
/// 将单个 Lua 触发的宿主工具请求分发给当前已注册的宿主回调。
pub(crate) fn dispatch_host_tool_request(
    request: &RuntimeHostToolRequest,
) -> Result<Value, String> {
    let registry = host_tool_callback_registry();
    let callback = {
        let guard = registry
            .lock()
            .map_err(|_| "Host tool callback registry lock poisoned".to_string())?;
        guard.clone()
    };
    let callback = callback.ok_or_else(|| {
        "Host tool bridge is enabled but no host callback is registered".to_string()
    })?;
    callback(request)
}

/// Build an internal model bridge error for registry or dispatch failures.
/// 为注册表或分发故障构造内部模型桥接错误。
fn model_internal_error(message: impl Into<String>) -> RuntimeModelError {
    RuntimeModelError {
        code: RuntimeModelErrorCode::InternalError,
        message: message.into(),
        provider_message: None,
        provider_code: None,
        provider_status: None,
    }
}

/// Build a model unavailable error for a missing capability callback.
/// 为缺失能力回调构造模型不可用错误。
fn model_unavailable_error(message: impl Into<String>) -> RuntimeModelError {
    RuntimeModelError {
        code: RuntimeModelErrorCode::ModelUnavailable,
        message: message.into(),
        provider_message: None,
        provider_code: None,
        provider_status: None,
    }
}

/// Dispatch one standard embedding request into the currently registered host callback.
/// 将单个标准 embedding 请求分发给当前已注册的宿主回调。
pub(crate) fn dispatch_model_embed_request(
    request: &RuntimeModelEmbedRequest,
) -> Result<RuntimeModelEmbedResponse, RuntimeModelError> {
    let registry = model_embed_callback_registry();
    let callback = {
        let guard = registry
            .lock()
            .map_err(|_| model_internal_error("Model embed callback registry lock poisoned"))?;
        guard.clone()
    };
    let callback =
        callback.ok_or_else(|| model_unavailable_error("embedding callback is not registered"))?;
    callback(request)
}

/// Dispatch one standard non-streaming LLM request into the currently registered host callback.
/// 将单个标准非流式 LLM 请求分发给当前已注册的宿主回调。
pub(crate) fn dispatch_model_llm_request(
    request: &RuntimeModelLlmRequest,
) -> Result<RuntimeModelLlmResponse, RuntimeModelError> {
    let registry = model_llm_callback_registry();
    let callback = {
        let guard = registry
            .lock()
            .map_err(|_| model_internal_error("Model llm callback registry lock poisoned"))?;
        guard.clone()
    };
    let callback =
        callback.ok_or_else(|| model_unavailable_error("llm callback is not registered"))?;
    callback(request)
}

/// Return whether one host callback is currently registered for runtime skill-management dispatch.
/// 返回当前是否已为运行时技能管理分发注册宿主回调。
pub(crate) fn try_has_skill_management_callback() -> Result<bool, String> {
    let registry = skill_management_callback_registry();
    let guard = registry
        .lock()
        .map_err(|_| "Skill management callback registry lock poisoned".to_string())?;
    Ok(guard.is_some())
}

/// Return whether one host callback is currently registered for host-tool dispatch.
/// 返回当前是否已为宿主工具分发注册宿主回调。
pub(crate) fn try_has_host_tool_callback() -> Result<bool, String> {
    let registry = host_tool_callback_registry();
    let guard = registry
        .lock()
        .map_err(|_| "Host tool callback registry lock poisoned".to_string())?;
    Ok(guard.is_some())
}

/// Return whether one host callback is currently registered for standard embedding dispatch.
/// 返回当前是否已为标准 embedding 分发注册宿主回调。
pub(crate) fn try_has_model_embed_callback() -> Result<bool, String> {
    let registry = model_embed_callback_registry();
    let guard = registry
        .lock()
        .map_err(|_| "Model embed callback registry lock poisoned".to_string())?;
    Ok(guard.is_some())
}

/// Return whether one host callback is currently registered for standard LLM dispatch.
/// 返回当前是否已为标准 LLM 分发注册宿主回调。
pub(crate) fn try_has_model_llm_callback() -> Result<bool, String> {
    let registry = model_llm_callback_registry();
    let guard = registry
        .lock()
        .map_err(|_| "Model llm callback registry lock poisoned".to_string())?;
    Ok(guard.is_some())
}

/// Return the process-wide lifecycle callback storage.
/// 返回进程级生命周期回调存储。
fn skill_lifecycle_callback_registry() -> &'static Mutex<Option<RuntimeSkillLifecycleCallback>> {
    static REGISTRY: OnceLock<Mutex<Option<RuntimeSkillLifecycleCallback>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(None))
}

/// Return the process-wide entry-registry callback storage.
/// 返回进程级入口注册表回调存储。
fn entry_registry_callback_registry() -> &'static Mutex<Option<RuntimeEntryRegistryCallback>> {
    static REGISTRY: OnceLock<Mutex<Option<RuntimeEntryRegistryCallback>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(None))
}

/// Return the process-wide skill-management callback storage.
/// 返回进程级技能管理回调存储。
fn skill_management_callback_registry() -> &'static Mutex<Option<RuntimeSkillManagementCallback>> {
    static REGISTRY: OnceLock<Mutex<Option<RuntimeSkillManagementCallback>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(None))
}

/// Return the process-wide host-tool callback storage.
/// 返回进程级宿主工具回调存储。
fn host_tool_callback_registry() -> &'static Mutex<Option<RuntimeHostToolCallback>> {
    static REGISTRY: OnceLock<Mutex<Option<RuntimeHostToolCallback>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(None))
}

/// Return the process-wide standard embedding callback storage.
/// 返回进程级标准 embedding 回调存储。
fn model_embed_callback_registry() -> &'static Mutex<Option<RuntimeModelEmbedCallback>> {
    static REGISTRY: OnceLock<Mutex<Option<RuntimeModelEmbedCallback>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(None))
}

/// Return the process-wide standard LLM callback storage.
/// 返回进程级标准 LLM 回调存储。
fn model_llm_callback_registry() -> &'static Mutex<Option<RuntimeModelLlmCallback>> {
    static REGISTRY: OnceLock<Mutex<Option<RuntimeModelLlmCallback>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(None))
}
