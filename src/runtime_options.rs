use crate::runtime_context::RuntimeRequestContext;
use crate::tool_cache::ToolCacheConfig;
use serde::Serialize;
use serde_json::{Map, Value};
use std::path::PathBuf;

/// English: Host-provided filesystem and runtime paths consumed by the LuaSkills library.
/// 宿主提供给 LuaSkills 库消费的文件系统与运行时路径集合。
#[derive(Debug, Clone, Default)]
pub struct LuaRuntimeHostOptions {
    /// English: Host-managed temporary directory used by luaexec spill files and similar transient artifacts.
    /// 宿主管理的临时目录，供 luaexec 请求文件等短生命周期产物使用。
    pub temp_dir: Option<PathBuf>,
    /// English: Optional host-managed resources directory exposed to Lua as `vulcan.runtime.resources_dir`.
    /// 以 `vulcan.runtime.resources_dir` 形式暴露给 Lua 的可选宿主管理资源目录。
    pub resources_dir: Option<PathBuf>,
    /// English: Optional lua_packages root used to build `package.path` and `package.cpath`.
    /// 用于拼接 `package.path` 与 `package.cpath` 的可选 lua_packages 根目录。
    pub lua_packages_dir: Option<PathBuf>,
    /// English: Optional external program path used by `vulcan.runtime.lua.exec` subprocess mode.
    /// `vulcan.runtime.lua.exec` 子进程模式使用的可选外部程序路径。
    pub luaexec_program: Option<PathBuf>,
    /// English: Explicit SQLite dynamic-library path owned by the host.
    /// 由宿主显式提供的 SQLite 动态库路径。
    pub sqlite_library_path: Option<PathBuf>,
    /// English: Explicit LanceDB dynamic-library path owned by the host.
    /// 由宿主显式提供的 LanceDB 动态库路径。
    pub lancedb_library_path: Option<PathBuf>,
    /// English: Host-managed root directory used to allocate one SQLite database per skill package.
    /// 宿主管理的 SQLite 根目录，用于为每个 skill 包分配单独数据库。
    pub sqlite_database_root: Option<PathBuf>,
    /// English: Host-managed root directory used to allocate one LanceDB directory per skill package.
    /// 宿主管理的 LanceDB 根目录，用于为每个 skill 包分配单独数据目录。
    pub lancedb_database_root: Option<PathBuf>,
    /// English: Host-provided transient cache policy consumed by `vulcan.cache`.
    /// 由宿主提供并供 `vulcan.cache` 消费的临时缓存策略。
    pub cache_config: Option<ToolCacheConfig>,
}

/// English: Host-injected invocation context delivered alongside one skill or runlua call.
/// 宿主在单次 skill 或 runlua 调用时一并注入的调用上下文。
#[derive(Debug, Clone, Default)]
pub struct LuaInvocationContext {
    /// English: Optional transport/request metadata preserved for Lua consumption.
    /// 供 Lua 消费的可选传输层/请求层元数据。
    pub request_context: Option<RuntimeRequestContext>,
    /// English: Host-resolved client budget object injected into `vulcan.context.client_budget`.
    /// 宿主解析后的客户端预算对象，将被注入到 `vulcan.context.client_budget`。
    pub client_budget: Value,
    /// English: Host-resolved tool configuration object injected into `vulcan.context.tool_config`.
    /// 宿主解析后的工具配置对象，将被注入到 `vulcan.context.tool_config`。
    pub tool_config: Value,
}

/// English: Host-resolved effective budget scope used by host-side render logic.
/// 供宿主侧渲染逻辑使用的宿主已解析生效预算场景结构。
#[derive(Debug, Clone, Serialize, Default, PartialEq, Eq)]
pub struct EffectiveBudgetScope {
    /// English: Effective byte limit for the current scope.
    /// 当前场景的生效字节上限。
    pub bytes: u64,
    /// English: Effective line limit for the current scope. `-1` means unlimited.
    /// 当前场景的生效行数上限，`-1` 表示不限。
    pub lines: i64,
}

/// English: Host-resolved client budget snapshot consumed by host-side overflow rendering.
/// 供宿主侧超限渲染消费的宿主已解析客户端预算快照。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ClientBudgetSnapshot {
    /// English: Optional client name of the active caller.
    /// 当前调用方的可选客户端名称。
    pub client_name: Option<String>,
    /// English: Optional resolved tool name.
    /// 可选的已解析工具名称。
    pub tool_name: Option<String>,
    /// English: Optional resolved skill name.
    /// 可选的已解析 skill 名称。
    pub skill_name: Option<String>,
    /// English: Optional matched client pattern decided by the host.
    /// 宿主判定出的可选客户端匹配模式。
    pub matched_client_pattern: Option<String>,
    /// English: Effective tool-result budget.
    /// 工具正文输出的生效预算。
    pub tool_result: EffectiveBudgetScope,
    /// English: Effective file-read budget.
    /// 文件读取场景的生效预算。
    pub file_read: EffectiveBudgetScope,
    /// English: Tool-scoped host configuration snapshot.
    /// 工具作用域下的宿主配置快照。
    pub tool_config: Value,
}

impl LuaInvocationContext {
    /// English: Construct one invocation context and normalize non-object JSON payloads into empty objects.
    /// 构造一次调用上下文，并把非对象类型的 JSON 载荷归一化为空对象。
    pub fn new(
        request_context: Option<RuntimeRequestContext>,
        client_budget: Value,
        tool_config: Value,
    ) -> Self {
        Self {
            request_context,
            client_budget: normalize_context_object(client_budget),
            tool_config: normalize_context_object(tool_config),
        }
    }

    /// English: Return an empty invocation context with stable empty-object payloads.
    /// 返回一个空调用上下文，并使用稳定的空对象载荷。
    pub fn empty() -> Self {
        Self::default()
    }
}

/// English: Normalize one host context payload so the runtime always sees an object.
/// 归一化单个宿主上下文载荷，确保运行时始终看到对象结构。
fn normalize_context_object(value: Value) -> Value {
    match value {
        Value::Object(_) => value,
        _ => Value::Object(Map::new()),
    }
}
