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

/// Return whether one host callback is currently registered for runtime skill-management dispatch.
/// 返回当前是否已为运行时技能管理分发注册宿主回调。
pub(crate) fn try_has_skill_management_callback() -> Result<bool, String> {
    let registry = skill_management_callback_registry();
    let guard = registry
        .lock()
        .map_err(|_| "Skill management callback registry lock poisoned".to_string())?;
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
