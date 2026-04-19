use crate::runtime::entry::RuntimeEntryDescriptor;
use crate::skill::manager::{SkillLifecycleAction, SkillOperationPlane};
use serde::Serialize;
use std::sync::{Arc, Mutex, OnceLock};

/// English: Callback type used by hosts to receive runtime skill-lifecycle events.
/// 宿主用于接收运行时技能生命周期事件的回调类型。
pub type RuntimeSkillLifecycleCallback = Arc<dyn Fn(&RuntimeSkillLifecycleEvent) + Send + Sync>;

/// English: Callback type used by hosts to receive runtime entry-registry change events.
/// 宿主用于接收运行时入口注册表变化事件的回调类型。
pub type RuntimeEntryRegistryCallback = Arc<dyn Fn(&RuntimeEntryRegistryDelta) + Send + Sync>;

/// English: Structured lifecycle event emitted after one skill-management operation is evaluated.
/// 在评估一次技能管理操作后发出的结构化生命周期事件。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RuntimeSkillLifecycleEvent {
    /// English: Operation plane that triggered the lifecycle event.
    /// 触发生命周期事件的操作平面。
    pub plane: SkillOperationPlane,
    /// English: Lifecycle action represented by the current event.
    /// 当前事件所表示的生命周期动作。
    pub action: SkillLifecycleAction,
    /// English: Skill identifier targeted by the current lifecycle operation.
    /// 当前生命周期操作对应的技能标识符。
    pub skill_id: String,
    /// English: High-level event status such as completed, failed, or blocked.
    /// 当前事件的高层状态，例如 completed、failed 或 blocked。
    pub status: String,
    /// English: Optional human-readable explanation of the current lifecycle outcome.
    /// 当前生命周期结果的可选人类可读说明。
    pub message: Option<String>,
}

/// English: Structured entry-registry delta emitted when one reload changes exposed runtime entries.
/// 当一次重载改变已暴露运行时入口时发出的结构化入口注册表差异。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RuntimeEntryRegistryDelta {
    /// English: Newly added runtime entries after the latest reload.
    /// 最近一次重载后新增的运行时入口。
    pub added_entries: Vec<RuntimeEntryDescriptor>,
    /// English: Canonical names removed after the latest reload.
    /// 最近一次重载后移除的 canonical 入口名称。
    pub removed_entry_names: Vec<String>,
    /// English: Existing canonical entries whose structure changed after the latest reload.
    /// 最近一次重载后结构发生变化的既有 canonical 入口。
    pub updated_entries: Vec<RuntimeEntryDescriptor>,
}

/// English: Install or clear the process-wide skill-lifecycle callback used by the host.
/// 安装或清理供宿主使用的进程级技能生命周期回调。
pub fn set_skill_lifecycle_callback(callback: Option<RuntimeSkillLifecycleCallback>) {
    let registry = skill_lifecycle_callback_registry();
    let mut guard = registry.lock().unwrap();
    *guard = callback;
}

/// English: Install or clear the process-wide entry-registry callback used by the host.
/// 安装或清理供宿主使用的进程级入口注册表回调。
pub fn set_entry_registry_callback(callback: Option<RuntimeEntryRegistryCallback>) {
    let registry = entry_registry_callback_registry();
    let mut guard = registry.lock().unwrap();
    *guard = callback;
}

/// English: Emit one skill-lifecycle event to the currently registered host callback when it exists.
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

/// English: Emit one entry-registry delta to the currently registered host callback when it exists.
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

/// English: Return the process-wide lifecycle callback storage.
/// 返回进程级生命周期回调存储。
fn skill_lifecycle_callback_registry() -> &'static Mutex<Option<RuntimeSkillLifecycleCallback>> {
    static REGISTRY: OnceLock<Mutex<Option<RuntimeSkillLifecycleCallback>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(None))
}

/// English: Return the process-wide entry-registry callback storage.
/// 返回进程级入口注册表回调存储。
fn entry_registry_callback_registry() -> &'static Mutex<Option<RuntimeEntryRegistryCallback>> {
    static REGISTRY: OnceLock<Mutex<Option<RuntimeEntryRegistryCallback>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(None))
}
