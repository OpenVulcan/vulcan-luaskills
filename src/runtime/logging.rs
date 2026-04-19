use std::sync::{Arc, OnceLock, RwLock};

/// English: Stable runtime log level emitted by the LuaSkills library.
/// LuaSkills 库发出的稳定运行时日志级别。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeLogLevel {
    /// English: Informational runtime event.
    /// 信息级运行时事件。
    Info,
    /// English: Warning runtime event.
    /// 告警级运行时事件。
    Warn,
    /// English: Error runtime event.
    /// 错误级运行时事件。
    Error,
}

/// English: Structured runtime log event forwarded from the library to the host callback.
/// 从库转发到宿主回调的结构化运行时日志事件。
#[derive(Debug, Clone)]
pub struct RuntimeLogEvent {
    /// English: Stable runtime log level.
    /// 稳定的运行时日志级别。
    pub level: RuntimeLogLevel,
    /// English: Human-readable log message emitted by the library.
    /// 由库发出的可读日志消息。
    pub message: String,
}

/// English: Host callback type that receives runtime log events.
/// 接收运行时日志事件的宿主回调类型。
pub type RuntimeLogCallback = Arc<dyn Fn(&RuntimeLogEvent) + Send + Sync + 'static>;

/// English: Global host log callback shared by the runtime until per-host routing is introduced.
/// 在引入更细粒度宿主路由前，由运行时共享使用的全局宿主日志回调。
static RUNTIME_LOG_CALLBACK: OnceLock<RwLock<Option<RuntimeLogCallback>>> = OnceLock::new();

/// English: Return the shared runtime log callback container.
/// 返回共享运行时日志回调容器。
fn runtime_log_callback() -> &'static RwLock<Option<RuntimeLogCallback>> {
    RUNTIME_LOG_CALLBACK.get_or_init(|| RwLock::new(None))
}

/// English: Register or replace the host-side runtime log callback.
/// 注册或替换宿主侧运行时日志回调。
pub fn set_log_callback(callback: Option<RuntimeLogCallback>) {
    if let Ok(mut guard) = runtime_log_callback().write() {
        *guard = callback;
    }
}

/// English: Emit one structured runtime log event to the current host callback if it exists.
/// 若当前宿主回调存在，则向其发送一条结构化运行时日志事件。
pub fn emit(level: RuntimeLogLevel, message: impl Into<String>) {
    let event = RuntimeLogEvent {
        level,
        message: message.into(),
    };
    let callback = runtime_log_callback()
        .read()
        .ok()
        .and_then(|guard| guard.as_ref().cloned());
    if let Some(callback) = callback {
        callback(&event);
    }
}

/// English: Emit one informational runtime log event.
/// 发送一条信息级运行时日志事件。
pub fn info(message: impl Into<String>) {
    emit(RuntimeLogLevel::Info, message);
}

/// English: Emit one warning runtime log event.
/// 发送一条告警级运行时日志事件。
pub fn warn(message: impl Into<String>) {
    emit(RuntimeLogLevel::Warn, message);
}

/// English: Emit one error runtime log event.
/// 发送一条错误级运行时日志事件。
pub fn error(message: impl Into<String>) {
    emit(RuntimeLogLevel::Error, message);
}
