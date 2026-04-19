mod lancedb_host;
mod sqlite_host;
/// English: Main LuaSkills runtime engine that loads skills, injects host APIs, and dispatches entries.
/// 主 LuaSkills 运行时引擎，负责加载 skill、注入宿主 API 并分发入口调用。
pub mod lua_engine;
/// English: Strict skill manifest and metadata types loaded from `skill.yaml`.
/// 从 `skill.yaml` 加载的严格 skill 清单与元数据类型。
pub mod lua_skill;
/// English: Generic runtime request context types independent from MCP-specific protocol objects.
/// 独立于 MCP 协议对象的通用运行时请求上下文类型。
pub mod runtime_context;
/// English: Generic runtime entry descriptors independent from MCP tool/resource objects.
/// 独立于 MCP tool/resource 对象的通用运行时入口描述类型。
pub mod entry_descriptor;
/// English: Generic runtime result types returned by the LuaSkills core.
/// LuaSkills Core 返回的通用运行时结果类型。
pub mod runtime_result;
/// English: Structured help list/detail types returned by the runtime core.
/// 由运行时核心返回的结构化帮助列表/详情类型。
pub mod runtime_help;
/// English: Host-injected runtime options and invocation context types.
/// 宿主注入式运行时选项与调用上下文类型定义。
pub mod runtime_options;
/// English: Lightweight runtime logging helpers shared across the library.
/// 供库内部共享使用的轻量级运行时日志辅助模块。
pub mod runtime_logging;
/// English: Host-managed transient cache shared by runtime calls.
/// 运行时调用共享使用的宿主管理临时缓存模块。
pub mod tool_cache;

pub use entry_descriptor::{RuntimeEntryDescriptor, RuntimeEntryParameterDescriptor};
pub use lua_engine::{LuaEngine, LuaEngineOptions, LuaVmPoolConfig};
pub use lua_skill::{SkillHelpMeta, SkillHelpNodeMeta, SkillMeta, SkillToolMeta};
pub use runtime_context::{RuntimeClientInfo, RuntimeRequestContext};
pub use runtime_help::{RuntimeHelpDetail, RuntimeHelpNodeDescriptor, RuntimeSkillHelpDescriptor};
pub use runtime_logging::{RuntimeLogCallback, RuntimeLogEvent, RuntimeLogLevel, set_log_callback};
pub use runtime_result::{NON_STRING_TOOL_RESULT_ERROR, RuntimeInvocationResult, ToolOverflowMode};
pub use runtime_options::{
    LuaInvocationContext, LuaRuntimeHostOptions,
};
pub use tool_cache::{
    DEFAULT_TOOL_CACHE_DEFAULT_TTL_SECS, DEFAULT_TOOL_CACHE_MAX_ENTRIES,
    DEFAULT_TOOL_CACHE_MAX_TTL_SECS, ToolCacheConfig,
};
