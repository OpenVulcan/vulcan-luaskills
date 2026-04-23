use std::ffi::{CStr, CString, c_char, c_void};
use std::path::PathBuf;
use std::ptr;
use std::sync::atomic::Ordering;

use serde_json::Value;

use crate::ffi::{FFI_ENGINE_COUNTER, ffi_engine_registry, with_engine, with_engine_mut};
use crate::host::database::{
    LuaRuntimeDatabaseCallbackMode, LuaRuntimeDatabaseProviderMode, RuntimeDatabaseBindingContext,
    RuntimeDatabaseKind, RuntimeLanceDbProviderAction, RuntimeLanceDbProviderCallback,
    RuntimeLanceDbProviderRequest, RuntimeLanceDbProviderResult, RuntimeSqliteProviderAction,
    RuntimeSqliteProviderCallback, RuntimeSqliteProviderRequest, set_lancedb_provider_callback,
    set_lancedb_provider_json_callback, set_sqlite_provider_callback,
    set_sqlite_provider_json_callback,
};
use crate::runtime_context::RuntimeRequestContext;
use crate::runtime_help::{
    RuntimeHelpDetail, RuntimeHelpNodeDescriptor, RuntimeSkillHelpDescriptor,
};
use crate::runtime_options::{
    LuaInvocationContext, LuaRuntimeCapabilityOptions, LuaRuntimeHostOptions,
    LuaRuntimeSpaceControllerOptions, LuaRuntimeSpaceControllerProcessMode, RuntimeSkillRoot,
};
use crate::skill::manager::{SkillInstallRequest, SkillUninstallOptions};
use crate::skill::source::SkillInstallSourceType;
use crate::tool_cache::ToolCacheConfig;
use crate::{
    LuaEngine, LuaEngineOptions, LuaVmPoolConfig, RuntimeEntryDescriptor,
    RuntimeEntryParameterDescriptor, RuntimeInvocationResult, SkillApplyResult,
    SkillUninstallResult,
};

const FFI_STATUS_OK: i32 = 0;
const FFI_STATUS_ERROR: i32 = 1;
const FFI_SOURCE_TYPE_ABSENT: i32 = -1;
const FFI_SOURCE_TYPE_GITHUB: i32 = 0;
const FFI_SOURCE_TYPE_URL: i32 = 1;
const FFI_PROVIDER_MODE_DYNAMIC_LIBRARY: i32 = 0;
const FFI_PROVIDER_MODE_HOST_CALLBACK: i32 = 1;
const FFI_PROVIDER_MODE_SPACE_CONTROLLER: i32 = 2;
const FFI_CALLBACK_MODE_STANDARD: i32 = 0;
const FFI_CALLBACK_MODE_JSON: i32 = 1;
const FFI_SPACE_CONTROLLER_PROCESS_MODE_SERVICE: i32 = 0;
const FFI_SPACE_CONTROLLER_PROCESS_MODE_MANAGED: i32 = 1;
const FFI_DATABASE_KIND_SQLITE: i32 = 0;
const FFI_DATABASE_KIND_LANCEDB: i32 = 1;
const FFI_SQLITE_PROVIDER_ACTION_EXECUTE_SCRIPT: i32 = 0;
const FFI_SQLITE_PROVIDER_ACTION_EXECUTE_BATCH: i32 = 1;
const FFI_SQLITE_PROVIDER_ACTION_QUERY_JSON: i32 = 2;
const FFI_SQLITE_PROVIDER_ACTION_QUERY_STREAM: i32 = 3;
const FFI_SQLITE_PROVIDER_ACTION_QUERY_STREAM_WAIT_METRICS: i32 = 4;
const FFI_SQLITE_PROVIDER_ACTION_QUERY_STREAM_CHUNK: i32 = 5;
const FFI_SQLITE_PROVIDER_ACTION_QUERY_STREAM_CLOSE: i32 = 6;
const FFI_SQLITE_PROVIDER_ACTION_TOKENIZE_TEXT: i32 = 7;
const FFI_SQLITE_PROVIDER_ACTION_UPSERT_CUSTOM_WORD: i32 = 8;
const FFI_SQLITE_PROVIDER_ACTION_REMOVE_CUSTOM_WORD: i32 = 9;
const FFI_SQLITE_PROVIDER_ACTION_LIST_CUSTOM_WORDS: i32 = 10;
const FFI_SQLITE_PROVIDER_ACTION_ENSURE_FTS_INDEX: i32 = 11;
const FFI_SQLITE_PROVIDER_ACTION_REBUILD_FTS_INDEX: i32 = 12;
const FFI_SQLITE_PROVIDER_ACTION_UPSERT_FTS_DOCUMENT: i32 = 13;
const FFI_SQLITE_PROVIDER_ACTION_DELETE_FTS_DOCUMENT: i32 = 14;
const FFI_SQLITE_PROVIDER_ACTION_SEARCH_FTS: i32 = 15;
const FFI_LANCEDB_PROVIDER_ACTION_CREATE_TABLE: i32 = 0;
const FFI_LANCEDB_PROVIDER_ACTION_VECTOR_UPSERT: i32 = 1;
const FFI_LANCEDB_PROVIDER_ACTION_VECTOR_SEARCH: i32 = 2;
const FFI_LANCEDB_PROVIDER_ACTION_DELETE: i32 = 3;
const FFI_LANCEDB_PROVIDER_ACTION_DROP_TABLE: i32 = 4;

/// Plain C ABI engine pool config used by standard non-JSON FFI calls.
/// 标准非 JSON FFI 调用使用的原生 C ABI 引擎池配置。
#[repr(C)]
pub struct FfiLuaVmPoolConfig {
    /// Minimum number of warm Lua VMs kept by the pool.
    /// 池内保持预热的最小 Lua VM 数量。
    pub min_size: usize,
    /// Maximum number of Lua VMs allowed in the pool.
    /// 池内允许存在的最大 Lua VM 数量。
    pub max_size: usize,
    /// Idle TTL in seconds applied to surplus Lua VMs.
    /// 应用于多余 Lua VM 的空闲过期秒数。
    pub idle_ttl_secs: u64,
}

/// Plain C ABI tool-cache config used by standard non-JSON FFI calls.
/// 标准非 JSON FFI 调用使用的原生 C ABI 工具缓存配置。
#[repr(C)]
pub struct FfiToolCacheConfig {
    /// Maximum number of shared tool-cache entries.
    /// 共享工具缓存允许存在的最大条目数。
    pub max_entries: usize,
    /// Default TTL in seconds applied when callers omit TTL.
    /// 当调用方省略 TTL 时应用的默认秒数。
    pub default_ttl_secs: u64,
    /// Maximum TTL in seconds accepted by the cache.
    /// 缓存允许接受的最大 TTL 秒数。
    pub max_ttl_secs: u64,
}

/// Borrowed byte-buffer view used by the refactored FFI callback surface.
/// 重构后 FFI 回调接口面使用的借用字节缓冲视图。
#[repr(C)]
pub struct FfiBorrowedBuffer {
    /// Borrowed pointer valid only for the duration of the current callback.
    /// 仅在当前回调执行期间有效的借用指针。
    pub ptr: *const u8,
    /// Number of readable bytes starting at `ptr`.
    /// 从 `ptr` 开始可读的字节数。
    pub len: usize,
}

/// Owned byte-buffer container transferred across the FFI boundary.
/// 跨 FFI 边界传递的拥有型字节缓冲容器。
#[repr(C)]
pub struct FfiOwnedBuffer {
    /// Owned heap pointer allocated by `luaskills` helper functions.
    /// 由 `luaskills` 辅助函数分配的拥有型堆指针。
    pub ptr: *mut u8,
    /// Number of owned bytes starting at `ptr`.
    /// 从 `ptr` 开始拥有的字节数。
    pub len: usize,
}

/// Plain C ABI host options used by standard non-JSON engine creation.
/// 标准非 JSON 引擎创建使用的原生 C ABI 宿主选项。
#[repr(C)]
pub struct FfiLuaRuntimeHostOptions {
    /// Optional temporary directory path.
    /// 可选临时目录路径。
    pub temp_dir: *const c_char,
    /// Optional resources directory path.
    /// 可选资源目录路径。
    pub resources_dir: *const c_char,
    /// Optional lua_packages directory path.
    /// 可选 lua_packages 目录路径。
    pub lua_packages_dir: *const c_char,
    /// Optional external luaexec program path.
    /// 可选外部 luaexec 程序路径。
    pub luaexec_program: *const c_char,
    /// Optional host-provided tools root path.
    /// 可选宿主提供工具根目录路径。
    pub host_provided_tool_root: *const c_char,
    /// Optional host-provided Lua packages root path.
    /// 可选宿主提供 Lua 包根目录路径。
    pub host_provided_lua_root: *const c_char,
    /// Optional host-provided FFI root path.
    /// 可选宿主提供 FFI 根目录路径。
    pub host_provided_ffi_root: *const c_char,
    /// Optional download-cache root path.
    /// 可选下载缓存根目录路径。
    pub download_cache_root: *const c_char,
    /// Required dependency sibling directory name.
    /// 必填的依赖兄弟目录名称。
    pub dependency_dir_name: *const c_char,
    /// Required state sibling directory name.
    /// 必填的状态兄弟目录名称。
    pub state_dir_name: *const c_char,
    /// Required database sibling directory name.
    /// 必填的数据库兄弟目录名称。
    pub database_dir_name: *const c_char,
    /// Protected skill identifiers reserved for the system plane.
    /// 为 system 平面保留的受保护技能标识符数组。
    pub protected_skill_ids: *const *const c_char,
    /// Number of protected skill identifiers.
    /// 受保护技能标识符数组长度。
    pub protected_skill_ids_len: usize,
    /// Whether the runtime may perform network downloads.
    /// 运行时是否允许执行网络下载。
    pub allow_network_download: u8,
    /// Optional GitHub site base URL.
    /// 可选 GitHub 站点基址。
    pub github_base_url: *const c_char,
    /// Optional GitHub API base URL.
    /// 可选 GitHub API 基址。
    pub github_api_base_url: *const c_char,
    /// Optional host SQLite dynamic library path.
    /// 可选宿主 SQLite 动态库路径。
    pub sqlite_library_path: *const c_char,
    /// SQLite provider mode integer where `0=dynamic_library`, `1=host_callback`, and `2=space_controller`.
    /// SQLite provider 模式整数，其中 `0=dynamic_library`、`1=host_callback`、`2=space_controller`。
    pub sqlite_provider_mode: i32,
    /// SQLite callback mode integer where `0=standard` and `1=json`.
    /// SQLite 回调模式整数，其中 `0=standard`、`1=json`。
    pub sqlite_callback_mode: i32,
    /// Optional host LanceDB dynamic library path.
    /// 可选宿主 LanceDB 动态库路径。
    pub lancedb_library_path: *const c_char,
    /// LanceDB provider mode integer where `0=dynamic_library` and `1=host_callback` and `2=space_controller`.
    /// LanceDB provider 模式整数，其中 `0=dynamic_library`、`1=host_callback`、`2=space_controller`。
    pub lancedb_provider_mode: i32,
    /// LanceDB callback mode integer where `0=standard` and `1=json`.
    /// LanceDB 回调模式整数，其中 `0=standard`、`1=json`。
    pub lancedb_callback_mode: i32,
    /// Optional shared space-controller endpoint.
    /// 可选共享空间控制器端点。
    pub space_controller_endpoint: *const c_char,
    /// Whether the runtime may auto-spawn the space-controller process.
    /// 运行时是否允许自动唤起空间控制器进程。
    pub space_controller_auto_spawn: u8,
    /// Optional copied local controller executable path.
    /// 可选的本地复制控制器可执行文件路径。
    pub space_controller_executable_path: *const c_char,
    /// Space-controller process mode integer where `0=service` and `1=managed`.
    /// 空间控制器进程模式整数，其中 `0=service`、`1=managed`。
    pub space_controller_process_mode: i32,
    /// Optional tool-cache config pointer.
    /// 可选工具缓存配置指针。
    pub cache_config: *const FfiToolCacheConfig,
    /// Reserved public entry names.
    /// 保留公开入口名称数组。
    pub reserved_entry_names: *const *const c_char,
    /// Number of reserved public entry names.
    /// 保留公开入口名称数组长度。
    pub reserved_entry_names_len: usize,
    /// Host-forced ignored skill identifiers.
    /// 宿主强制忽略的技能标识符数组。
    pub ignored_skill_ids: *const *const c_char,
    /// Number of host-forced ignored skill identifiers.
    /// 宿主强制忽略的技能标识符数组长度。
    pub ignored_skill_ids_len: usize,
    /// Whether Lua may use `vulcan.runtime.skills.*` management bridges.
    /// Lua 是否允许使用 `vulcan.runtime.skills.*` 管理桥接。
    pub enable_skill_management_bridge: u8,
}

/// C ABI JSON provider callback used by non-Rust hosts to bridge database requests.
/// 供非 Rust 宿主桥接数据库请求使用的 C ABI JSON provider 回调。
pub type FfiJsonProviderCallback = unsafe extern "C" fn(
    request_json: FfiBorrowedBuffer,
    user_data: *mut c_void,
    response_out: *mut FfiOwnedBuffer,
    error_out: *mut FfiOwnedBuffer,
) -> i32;

/// C ABI SQLite provider callback used by standard host integration.
/// 标准宿主集成使用的 C ABI SQLite provider 回调。
pub type FfiSqliteProviderCallback = unsafe extern "C" fn(
    request: *const FfiSqliteProviderRequest,
    user_data: *mut c_void,
    response_json_out: *mut FfiOwnedBuffer,
    error_out: *mut FfiOwnedBuffer,
) -> i32;

/// C ABI LanceDB provider callback used by standard host integration.
/// 标准宿主集成使用的 C ABI LanceDB provider 回调。
pub type FfiLanceDbProviderCallback = unsafe extern "C" fn(
    request: *const FfiLanceDbProviderRequest,
    user_data: *mut c_void,
    meta_json_out: *mut FfiOwnedBuffer,
    data_out: *mut FfiOwnedBuffer,
    error_out: *mut FfiOwnedBuffer,
) -> i32;

/// Plain C ABI engine options used by standard non-JSON engine creation.
/// 标准非 JSON 引擎创建使用的原生 C ABI 引擎选项。
#[repr(C)]
pub struct FfiLuaEngineOptions {
    /// Pool config applied to the runtime engine.
    /// 应用于运行时引擎的池配置。
    pub pool: FfiLuaVmPoolConfig,
    /// Host options applied to the runtime engine.
    /// 应用于运行时引擎的宿主选项。
    pub host: FfiLuaRuntimeHostOptions,
}

/// Plain C ABI skill root used by standard non-JSON lifecycle and load calls.
/// 标准非 JSON 生命周期与加载调用使用的原生 C ABI 技能根结构。
#[repr(C)]
pub struct FfiRuntimeSkillRoot {
    /// Stable root name such as ROOT or USER.
    /// 稳定根名称，例如 ROOT 或 USER。
    pub name: *const c_char,
    /// Physical skills directory path.
    /// 物理 skills 目录路径。
    pub skills_dir: *const c_char,
}

/// Plain C ABI invocation context used by standard non-JSON call paths.
/// 标准非 JSON 调用路径使用的原生 C ABI 调用上下文。
#[repr(C)]
pub struct FfiLuaInvocationContext {
    /// Optional request context encoded as one borrowed JSON byte buffer.
    /// 以借用 JSON 字节缓冲编码的可选请求上下文。
    pub request_context_json: FfiBorrowedBuffer,
    /// Optional client budget encoded as one borrowed JSON byte buffer.
    /// 以借用 JSON 字节缓冲编码的可选客户端预算。
    pub client_budget_json: FfiBorrowedBuffer,
    /// Optional tool config encoded as one borrowed JSON byte buffer.
    /// 以借用 JSON 字节缓冲编码的可选工具配置。
    pub tool_config_json: FfiBorrowedBuffer,
}

/// Plain C ABI managed install request used by standard lifecycle calls.
/// 标准生命周期调用使用的原生 C ABI 受管安装请求。
#[repr(C)]
pub struct FfiSkillInstallRequest {
    /// Optional target skill id.
    /// 可选目标技能标识符。
    pub skill_id: *const c_char,
    /// Optional source locator.
    /// 可选来源定位值。
    pub source: *const c_char,
    /// Managed source type encoded as one integer.
    /// 以整数编码的受管来源类型。
    pub source_type: i32,
}

/// Plain C ABI uninstall options used by standard lifecycle calls.
/// 标准生命周期调用使用的原生 C ABI 卸载选项。
#[repr(C)]
pub struct FfiSkillUninstallOptions {
    /// Remove SQLite data when non-zero.
    /// 非零时删除 SQLite 数据。
    pub remove_sqlite: u8,
    /// Remove LanceDB data when non-zero.
    /// 非零时删除 LanceDB 数据。
    pub remove_lancedb: u8,
}

/// Plain C ABI database binding context forwarded into one host-managed database provider.
/// 转发给宿主管理数据库 provider 的原生 C ABI 数据库绑定上下文。
#[repr(C)]
pub struct FfiRuntimeDatabaseBindingContext {
    /// Stable host-provided space label such as ROOT, USER, or PROJECT_A.
    /// 由宿主提供的稳定空间标签，例如 ROOT、USER 或 PROJECT_A。
    pub space_label: *const c_char,
    /// Stable skill identifier owning the current binding.
    /// 拥有当前绑定的稳定技能标识符。
    pub skill_id: *const c_char,
    /// Stable database binding tag composed from space label and skill id.
    /// 由空间标签与技能标识符组合得到的稳定数据库绑定标签。
    pub binding_tag: *const c_char,
    /// Effective physical skill root label that resolved the current skill instance.
    /// 解析出当前技能实例的生效物理技能根标签。
    pub root_name: *const c_char,
    /// Physical space root directory path.
    /// 物理空间根目录路径。
    pub space_root: *const c_char,
    /// Physical skill directory path.
    /// 物理技能目录路径。
    pub skill_dir: *const c_char,
    /// Basename of the physical skill directory.
    /// 物理技能目录名称。
    pub skill_dir_name: *const c_char,
    /// Database kind integer where `0=sqlite` and `1=lancedb`.
    /// 数据库类型整数，其中 `0=sqlite`、`1=lancedb`。
    pub database_kind: i32,
    /// Library-computed default embedded database path for diagnostics and fallback.
    /// 由库计算出的默认内嵌数据库路径，用于诊断和回退。
    pub default_database_path: *const c_char,
}

/// Plain C ABI SQLite provider request delivered to one host callback.
/// 传递给宿主回调的原生 C ABI SQLite provider 请求。
#[repr(C)]
pub struct FfiSqliteProviderRequest {
    /// Requested action integer defined by `FFI_SQLITE_PROVIDER_ACTION_*`.
    /// 由 `FFI_SQLITE_PROVIDER_ACTION_*` 定义的请求动作整数。
    pub action: i32,
    /// Stable binding context of the current skill-scoped database.
    /// 当前 skill 级数据库的稳定绑定上下文。
    pub binding: FfiRuntimeDatabaseBindingContext,
    /// Action-specific input payload encoded as one borrowed JSON byte buffer.
    /// 以借用 JSON 字节缓冲编码的动作专属输入载荷。
    pub input_json: FfiBorrowedBuffer,
}

/// Plain C ABI LanceDB provider request delivered to one host callback.
/// 传递给宿主回调的原生 C ABI LanceDB provider 请求。
#[repr(C)]
pub struct FfiLanceDbProviderRequest {
    /// Requested action integer defined by `FFI_LANCEDB_PROVIDER_ACTION_*`.
    /// 由 `FFI_LANCEDB_PROVIDER_ACTION_*` 定义的请求动作整数。
    pub action: i32,
    /// Stable binding context of the current skill-scoped database.
    /// 当前 skill 级数据库的稳定绑定上下文。
    pub binding: FfiRuntimeDatabaseBindingContext,
    /// Action-specific input payload encoded as one borrowed JSON byte buffer.
    /// 以借用 JSON 字节缓冲编码的动作专属输入载荷。
    pub input_json: FfiBorrowedBuffer,
}

/// Plain C ABI string-array result.
/// 原生 C ABI 字符串数组结果。
#[repr(C)]
pub struct FfiStringArray {
    /// Owned UTF-8 string buffers.
    /// 拥有所有权的 UTF-8 字符串缓冲数组。
    pub items: *mut FfiOwnedBuffer,
    /// Number of string items.
    /// 字符串条目数量。
    pub len: usize,
}

/// Plain C ABI entry parameter descriptor.
/// 原生 C ABI 入口参数描述结构。
#[repr(C)]
pub struct FfiRuntimeEntryParameterDescriptor {
    /// Stable parameter name.
    /// 稳定参数名。
    pub name: FfiOwnedBuffer,
    /// Runtime parameter type string.
    /// 运行时参数类型字符串。
    pub param_type: FfiOwnedBuffer,
    /// Human-readable parameter description.
    /// 人类可读参数说明。
    pub description: FfiOwnedBuffer,
    /// Whether the parameter is required.
    /// 当前参数是否必填。
    pub required: u8,
}

/// Plain C ABI entry descriptor.
/// 原生 C ABI 入口描述结构。
#[repr(C)]
pub struct FfiRuntimeEntryDescriptor {
    /// Canonical runtime entry name.
    /// canonical 运行时入口名称。
    pub canonical_name: FfiOwnedBuffer,
    /// Owning skill id.
    /// 所属技能标识符。
    pub skill_id: FfiOwnedBuffer,
    /// Local entry name declared by the skill.
    /// 技能声明的局部入口名。
    pub local_name: FfiOwnedBuffer,
    /// Effective root name.
    /// 生效根名称。
    pub root_name: FfiOwnedBuffer,
    /// Effective physical skill directory.
    /// 生效物理技能目录。
    pub skill_dir: FfiOwnedBuffer,
    /// Human-readable entry description.
    /// 人类可读入口描述。
    pub description: FfiOwnedBuffer,
    /// Parameter descriptor array.
    /// 参数描述数组。
    pub parameters: *mut FfiRuntimeEntryParameterDescriptor,
    /// Parameter descriptor count.
    /// 参数描述数量。
    pub parameters_len: usize,
}

/// Plain C ABI entry descriptor list.
/// 原生 C ABI 入口描述列表。
#[repr(C)]
pub struct FfiRuntimeEntryDescriptorList {
    /// Entry descriptor items.
    /// 入口描述条目。
    pub items: *mut FfiRuntimeEntryDescriptor,
    /// Entry descriptor count.
    /// 入口描述数量。
    pub len: usize,
}

/// Plain C ABI help node descriptor.
/// 原生 C ABI 帮助节点描述结构。
#[repr(C)]
pub struct FfiRuntimeHelpNodeDescriptor {
    /// Help flow name.
    /// 帮助流程名。
    pub flow_name: FfiOwnedBuffer,
    /// Human-readable node description.
    /// 人类可读节点描述。
    pub description: FfiOwnedBuffer,
    /// Related canonical runtime entry names.
    /// 关联的 canonical 运行时入口名称。
    pub related_entries: *mut FfiOwnedBuffer,
    /// Number of related canonical runtime entry names.
    /// 关联 canonical 运行时入口名称数量。
    pub related_entries_len: usize,
    /// Whether the node is the main help node.
    /// 当前节点是否为主帮助节点。
    pub is_main: u8,
}

/// Plain C ABI help tree descriptor.
/// 原生 C ABI 帮助树描述结构。
#[repr(C)]
pub struct FfiRuntimeSkillHelpDescriptor {
    /// Stable skill id.
    /// 稳定技能标识符。
    pub skill_id: FfiOwnedBuffer,
    /// Human-readable skill name.
    /// 人类可读技能名称。
    pub skill_name: FfiOwnedBuffer,
    /// Semantic skill version.
    /// 语义化技能版本。
    pub skill_version: FfiOwnedBuffer,
    /// Effective root name.
    /// 生效根名称。
    pub root_name: FfiOwnedBuffer,
    /// Effective physical skill directory.
    /// 生效物理技能目录。
    pub skill_dir: FfiOwnedBuffer,
    /// Main help node descriptor.
    /// 主帮助节点描述。
    pub main: FfiRuntimeHelpNodeDescriptor,
    /// Flow help node descriptor array.
    /// 流程帮助节点描述数组。
    pub flows: *mut FfiRuntimeHelpNodeDescriptor,
    /// Flow help node descriptor count.
    /// 流程帮助节点描述数量。
    pub flows_len: usize,
}

/// Plain C ABI help tree descriptor list.
/// 原生 C ABI 帮助树描述列表。
#[repr(C)]
pub struct FfiRuntimeSkillHelpDescriptorList {
    /// Help descriptor items.
    /// 帮助描述条目。
    pub items: *mut FfiRuntimeSkillHelpDescriptor,
    /// Help descriptor count.
    /// 帮助描述数量。
    pub len: usize,
}

/// Plain C ABI help detail descriptor.
/// 原生 C ABI 帮助详情描述结构。
#[repr(C)]
pub struct FfiRuntimeHelpDetail {
    /// Stable skill id.
    /// 稳定技能标识符。
    pub skill_id: FfiOwnedBuffer,
    /// Human-readable skill name.
    /// 人类可读技能名称。
    pub skill_name: FfiOwnedBuffer,
    /// Semantic skill version.
    /// 语义化技能版本。
    pub skill_version: FfiOwnedBuffer,
    /// Effective root name.
    /// 生效根名称。
    pub root_name: FfiOwnedBuffer,
    /// Effective physical skill directory.
    /// 生效物理技能目录。
    pub skill_dir: FfiOwnedBuffer,
    /// Flow name.
    /// 流程名称。
    pub flow_name: FfiOwnedBuffer,
    /// Human-readable description.
    /// 人类可读描述。
    pub description: FfiOwnedBuffer,
    /// Related canonical runtime entries.
    /// 关联的 canonical 运行时入口。
    pub related_entries: *mut FfiOwnedBuffer,
    /// Number of related canonical runtime entries.
    /// 关联 canonical 运行时入口数量。
    pub related_entries_len: usize,
    /// Whether the node is the main help node.
    /// 当前节点是否为主帮助节点。
    pub is_main: u8,
    /// Structured content type.
    /// 结构化内容类型。
    pub content_type: FfiOwnedBuffer,
    /// Final rendered help content.
    /// 最终渲染出的帮助内容。
    pub content: FfiOwnedBuffer,
}

/// Plain C ABI invocation result.
/// 原生 C ABI 调用结果结构。
#[repr(C)]
pub struct FfiRuntimeInvocationResult {
    /// Tool body content.
    /// 工具正文内容。
    pub content: FfiOwnedBuffer,
    /// Overflow mode encoded as 0 none, 1 truncate, 2 page.
    /// 以 0 无、1 截断、2 分页编码的超限模式。
    pub overflow_mode: i32,
    /// Optional template hint.
    /// 可选模板提示名。
    pub template_hint: FfiOwnedBuffer,
    /// Content byte count.
    /// 内容字节数。
    pub content_bytes: usize,
    /// Content line count.
    /// 内容行数。
    pub content_lines: usize,
}

/// Plain C ABI install or update result.
/// 原生 C ABI 安装或更新结果结构。
#[repr(C)]
pub struct FfiSkillApplyResult {
    /// Stable skill id.
    /// 稳定技能标识符。
    pub skill_id: FfiOwnedBuffer,
    /// High-level result status.
    /// 高层结果状态。
    pub status: FfiOwnedBuffer,
    /// Human-readable message.
    /// 人类可读消息。
    pub message: FfiOwnedBuffer,
    /// Optional semantic version.
    /// 可选语义化版本。
    pub version: FfiOwnedBuffer,
    /// Optional source type encoded as integer, where -1 means absent.
    /// 以整数编码的可选来源类型，-1 表示不存在。
    pub source_type: i32,
    /// Optional source locator.
    /// 可选来源定位值。
    pub source_locator: FfiOwnedBuffer,
}

/// Plain C ABI uninstall result.
/// 原生 C ABI 卸载结果结构。
#[repr(C)]
pub struct FfiSkillUninstallResult {
    /// Stable skill id.
    /// 稳定技能标识符。
    pub skill_id: FfiOwnedBuffer,
    /// Whether the skill directory was removed.
    /// 技能目录是否已被删除。
    pub skill_removed: u8,
    /// Whether the SQLite database was removed.
    /// SQLite 数据库是否已被删除。
    pub sqlite_removed: u8,
    /// Whether the LanceDB database was removed.
    /// LanceDB 数据库是否已被删除。
    pub lancedb_removed: u8,
    /// Whether the SQLite database was intentionally retained.
    /// SQLite 数据库是否被有意保留。
    pub sqlite_retained: u8,
    /// Whether the LanceDB database was intentionally retained.
    /// LanceDB 数据库是否被有意保留。
    pub lancedb_retained: u8,
    /// Human-readable message.
    /// 人类可读消息。
    pub message: FfiOwnedBuffer,
}

/// Write one owned UTF-8 error buffer into the caller-provided error output slot.
/// 将一段拥有型 UTF-8 错误缓冲写入调用方提供的错误输出槽位。
fn set_error_out(error_out: *mut FfiOwnedBuffer, message: impl Into<String>) {
    if error_out.is_null() {
        return;
    }
    let text = message.into();
    unsafe {
        *error_out = alloc_owned_buffer_from_bytes(text.as_bytes());
    }
}

/// Clear one caller-provided error output slot to an empty buffer.
/// 将调用方提供的错误输出槽位清空为空缓冲。
fn clear_error_out(error_out: *mut FfiOwnedBuffer) {
    clear_out_buffer(error_out);
}

/// Clear one caller-provided pointer output slot to null.
/// 将调用方提供的指针输出槽位清空为 null。
fn clear_out_ptr<T>(value_out: *mut *mut T) {
    if !value_out.is_null() {
        unsafe { *value_out = std::ptr::null_mut() };
    }
}

/// Clear one caller-provided owned-buffer output slot to an empty buffer.
/// 将调用方提供的拥有型缓冲输出槽位清空为空缓冲。
fn clear_out_buffer(value_out: *mut FfiOwnedBuffer) {
    if !value_out.is_null() {
        unsafe {
            *value_out = FfiOwnedBuffer {
                ptr: ptr::null_mut(),
                len: 0,
            }
        };
    }
}

/// Clear one caller-provided unsigned 64-bit output slot to zero.
/// 将调用方提供的无符号 64 位输出槽位清空为零。
fn clear_out_u64(value_out: *mut u64) {
    if !value_out.is_null() {
        unsafe { *value_out = 0 };
    }
}

/// Clear one caller-provided unsigned 8-bit output slot to zero.
/// 将调用方提供的无符号 8 位输出槽位清空为零。
fn clear_out_u8(value_out: *mut u8) {
    if !value_out.is_null() {
        unsafe { *value_out = 0 };
    }
}

/// Convert one Rust string into one owned raw C string pointer.
/// 将单个 Rust 字符串转换为一个拥有所有权的原生 C 字符串指针。
fn alloc_c_string(value: impl AsRef<str>) -> *mut c_char {
    CString::new(value.as_ref())
        .unwrap_or_else(|_| CString::new("FFI string contains NUL byte").expect("static text"))
        .into_raw()
}

/// Convert one byte slice into one owned FFI buffer.
/// 将单个字节切片转换为一个拥有所有权的 FFI 缓冲。
fn alloc_owned_buffer_from_bytes(value: &[u8]) -> FfiOwnedBuffer {
    if value.is_empty() {
        return FfiOwnedBuffer {
            ptr: ptr::null_mut(),
            len: 0,
        };
    }
    let mut bytes = value.to_vec();
    let pointer = bytes.as_mut_ptr();
    let len = bytes.len();
    std::mem::forget(bytes);
    FfiOwnedBuffer { ptr: pointer, len }
}

/// Convert one Rust string into one owned UTF-8 FFI buffer.
/// 将单个 Rust 字符串转换为一个拥有所有权的 UTF-8 FFI 缓冲。
fn alloc_owned_buffer_from_string(value: impl AsRef<str>) -> FfiOwnedBuffer {
    alloc_owned_buffer_from_bytes(value.as_ref().as_bytes())
}

/// Convert one Rust string into one owned C string while rejecting interior NUL bytes.
/// 将一个 Rust 字符串转换为拥有所有权的 C 字符串，并拒绝内部 NUL 字节。
fn to_cstring(value: impl AsRef<str>, field_name: &str) -> Result<CString, String> {
    CString::new(value.as_ref()).map_err(|_| format!("{} contains interior NUL bytes", field_name))
}

/// Convert one optional Rust string into one optional owned UTF-8 FFI buffer.
/// 将单个可选 Rust 字符串转换为一个可选拥有所有权的 UTF-8 FFI 缓冲。
fn alloc_optional_owned_buffer_from_string(value: Option<&str>) -> FfiOwnedBuffer {
    value.map_or(
        FfiOwnedBuffer {
            ptr: ptr::null_mut(),
            len: 0,
        },
        alloc_owned_buffer_from_string,
    )
}

/// Parse one required UTF-8 string pointer.
/// 解析单个必填 UTF-8 字符串指针。
fn parse_required_string(value: *const c_char, field_name: &str) -> Result<String, String> {
    if value.is_null() {
        return Err(format!("{} must not be null", field_name));
    }
    let text = unsafe { CStr::from_ptr(value) }
        .to_str()
        .map_err(|error| format!("{} contains invalid UTF-8: {}", field_name, error))?;
    if text.is_empty() {
        return Err(format!("{} must not be empty", field_name));
    }
    Ok(text.to_string())
}

/// Parse one optional UTF-8 string pointer.
/// 解析单个可选 UTF-8 字符串指针。
fn parse_optional_string(value: *const c_char, field_name: &str) -> Result<Option<String>, String> {
    if value.is_null() {
        return Ok(None);
    }
    let text = unsafe { CStr::from_ptr(value) }
        .to_str()
        .map_err(|error| format!("{} contains invalid UTF-8: {}", field_name, error))?;
    if text.is_empty() {
        return Ok(None);
    }
    Ok(Some(text.to_string()))
}

/// Parse one array of UTF-8 string pointers.
/// 解析一组 UTF-8 字符串指针数组。
fn parse_string_array(
    items: *const *const c_char,
    len: usize,
    field_name: &str,
) -> Result<Vec<String>, String> {
    if len == 0 {
        return Ok(Vec::new());
    }
    if items.is_null() {
        return Err(format!(
            "{} items pointer must not be null when len > 0",
            field_name
        ));
    }
    let slice = unsafe { std::slice::from_raw_parts(items, len) };
    slice
        .iter()
        .enumerate()
        .map(|(index, item)| parse_required_string(*item, &format!("{}[{}]", field_name, index)))
        .collect()
}

/// Parse one optional borrowed UTF-8 buffer into one owned Rust string.
/// 将单个可选借用 UTF-8 缓冲解析为一个 Rust 自有字符串。
fn parse_optional_borrowed_text(
    value: &FfiBorrowedBuffer,
    field_name: &str,
) -> Result<Option<String>, String> {
    if value.len == 0 {
        return Ok(None);
    }
    if value.ptr.is_null() {
        return Err(format!(
            "{} pointer must not be null when len > 0",
            field_name
        ));
    }
    let bytes = unsafe { std::slice::from_raw_parts(value.ptr, value.len) };
    let text = std::str::from_utf8(bytes)
        .map_err(|error| format!("{} contains invalid UTF-8: {}", field_name, error))?;
    if text.is_empty() {
        return Ok(None);
    }
    Ok(Some(text.to_string()))
}

/// Parse one optional borrowed JSON buffer into one serde_json value object.
/// 将单个可选借用 JSON 缓冲解析为一个 serde_json 值对象。
fn parse_json_value_or_empty_object_buffer(
    value: &FfiBorrowedBuffer,
    field_name: &str,
) -> Result<Value, String> {
    match parse_optional_borrowed_text(value, field_name)? {
        Some(text) => serde_json::from_str(&text)
            .map_err(|error| format!("{} contains invalid JSON: {}", field_name, error)),
        None => Ok(Value::Object(serde_json::Map::new())),
    }
}

/// Parse one optional borrowed request-context JSON buffer into one structured request context.
/// 将单个可选借用请求上下文 JSON 缓冲解析为一个结构化请求上下文。
fn parse_request_context_buffer(
    value: &FfiBorrowedBuffer,
    field_name: &str,
) -> Result<Option<RuntimeRequestContext>, String> {
    match parse_optional_borrowed_text(value, field_name)? {
        Some(text) => serde_json::from_str(&text)
            .map(Some)
            .map_err(|error| format!("{} contains invalid JSON: {}", field_name, error)),
        None => Ok(None),
    }
}

/// Convert one C ABI cache config pointer into one Rust cache config.
/// 将单个 C ABI 缓存配置指针转换为一个 Rust 缓存配置。
fn parse_cache_config(value: *const FfiToolCacheConfig) -> Option<ToolCacheConfig> {
    if value.is_null() {
        None
    } else {
        let config = unsafe { &*value };
        Some(ToolCacheConfig {
            max_entries: config.max_entries,
            default_ttl_secs: config.default_ttl_secs,
            max_ttl_secs: config.max_ttl_secs,
        })
    }
}

/// Convert one C ABI host options struct into one Rust host options value.
/// 将单个 C ABI 宿主选项结构转换为一个 Rust 宿主选项值。
fn parse_host_options(value: &FfiLuaRuntimeHostOptions) -> Result<LuaRuntimeHostOptions, String> {
    Ok(LuaRuntimeHostOptions {
        temp_dir: parse_optional_string(value.temp_dir, "temp_dir")?.map(PathBuf::from),
        resources_dir: parse_optional_string(value.resources_dir, "resources_dir")?
            .map(PathBuf::from),
        lua_packages_dir: parse_optional_string(value.lua_packages_dir, "lua_packages_dir")?
            .map(PathBuf::from),
        luaexec_program: parse_optional_string(value.luaexec_program, "luaexec_program")?
            .map(PathBuf::from),
        host_provided_tool_root: parse_optional_string(
            value.host_provided_tool_root,
            "host_provided_tool_root",
        )?
        .map(PathBuf::from),
        host_provided_lua_root: parse_optional_string(
            value.host_provided_lua_root,
            "host_provided_lua_root",
        )?
        .map(PathBuf::from),
        host_provided_ffi_root: parse_optional_string(
            value.host_provided_ffi_root,
            "host_provided_ffi_root",
        )?
        .map(PathBuf::from),
        download_cache_root: parse_optional_string(
            value.download_cache_root,
            "download_cache_root",
        )?
        .map(PathBuf::from),
        dependency_dir_name: parse_required_string(
            value.dependency_dir_name,
            "dependency_dir_name",
        )?,
        state_dir_name: parse_required_string(value.state_dir_name, "state_dir_name")?,
        database_dir_name: parse_required_string(value.database_dir_name, "database_dir_name")?,
        protection: crate::skill::manager::SkillProtectionConfig {
            protected_skill_ids: parse_string_array(
                value.protected_skill_ids,
                value.protected_skill_ids_len,
                "protected_skill_ids",
            )?,
        },
        allow_network_download: value.allow_network_download != 0,
        github_base_url: parse_optional_string(value.github_base_url, "github_base_url")?,
        github_api_base_url: parse_optional_string(
            value.github_api_base_url,
            "github_api_base_url",
        )?,
        sqlite_library_path: parse_optional_string(
            value.sqlite_library_path,
            "sqlite_library_path",
        )?
        .map(PathBuf::from),
        sqlite_provider_mode: parse_provider_mode(
            value.sqlite_provider_mode,
            "sqlite_provider_mode",
        )?,
        sqlite_callback_mode: parse_callback_mode(
            value.sqlite_callback_mode,
            "sqlite_callback_mode",
        )?,
        lancedb_library_path: parse_optional_string(
            value.lancedb_library_path,
            "lancedb_library_path",
        )?
        .map(PathBuf::from),
        lancedb_provider_mode: parse_provider_mode(
            value.lancedb_provider_mode,
            "lancedb_provider_mode",
        )?,
        lancedb_callback_mode: parse_callback_mode(
            value.lancedb_callback_mode,
            "lancedb_callback_mode",
        )?,
        space_controller: LuaRuntimeSpaceControllerOptions {
            endpoint: parse_optional_string(
                value.space_controller_endpoint,
                "space_controller_endpoint",
            )?,
            auto_spawn: value.space_controller_auto_spawn != 0,
            executable_path: parse_optional_string(
                value.space_controller_executable_path,
                "space_controller_executable_path",
            )?
            .map(PathBuf::from),
            process_mode: parse_space_controller_process_mode(
                value.space_controller_process_mode,
                "space_controller_process_mode",
            )?,
            ..LuaRuntimeSpaceControllerOptions::default()
        },
        cache_config: parse_cache_config(value.cache_config),
        reserved_entry_names: parse_string_array(
            value.reserved_entry_names,
            value.reserved_entry_names_len,
            "reserved_entry_names",
        )?,
        ignored_skill_ids: parse_string_array(
            value.ignored_skill_ids,
            value.ignored_skill_ids_len,
            "ignored_skill_ids",
        )?,
        capabilities: LuaRuntimeCapabilityOptions {
            enable_skill_management_bridge: value.enable_skill_management_bridge != 0,
        },
    })
}

/// Convert one stable integer provider-mode value into the Rust runtime enum.
/// 将一个稳定整数 provider 模式值转换为 Rust 运行时枚举。
fn parse_provider_mode(
    value: i32,
    field_name: &str,
) -> Result<LuaRuntimeDatabaseProviderMode, String> {
    match value {
        FFI_PROVIDER_MODE_DYNAMIC_LIBRARY => Ok(LuaRuntimeDatabaseProviderMode::DynamicLibrary),
        FFI_PROVIDER_MODE_HOST_CALLBACK => Ok(LuaRuntimeDatabaseProviderMode::HostCallback),
        FFI_PROVIDER_MODE_SPACE_CONTROLLER => Ok(LuaRuntimeDatabaseProviderMode::SpaceController),
        _ => Err(format!("Unsupported {} value '{}'", field_name, value)),
    }
}

/// Convert one stable integer callback-mode value into the Rust runtime enum.
/// 将一个稳定整数回调模式值转换为 Rust 运行时枚举。
fn parse_callback_mode(
    value: i32,
    field_name: &str,
) -> Result<LuaRuntimeDatabaseCallbackMode, String> {
    match value {
        FFI_CALLBACK_MODE_STANDARD => Ok(LuaRuntimeDatabaseCallbackMode::Standard),
        FFI_CALLBACK_MODE_JSON => Ok(LuaRuntimeDatabaseCallbackMode::Json),
        _ => Err(format!("Unsupported {} value '{}'", field_name, value)),
    }
}

/// Convert one stable integer space-controller process-mode value into the Rust runtime enum.
/// 将一个稳定整数空间控制器进程模式值转换为 Rust 运行时枚举。
fn parse_space_controller_process_mode(
    value: i32,
    field_name: &str,
) -> Result<LuaRuntimeSpaceControllerProcessMode, String> {
    match value {
        FFI_SPACE_CONTROLLER_PROCESS_MODE_SERVICE => {
            Ok(LuaRuntimeSpaceControllerProcessMode::Service)
        }
        FFI_SPACE_CONTROLLER_PROCESS_MODE_MANAGED => {
            Ok(LuaRuntimeSpaceControllerProcessMode::Managed)
        }
        _ => Err(format!("Unsupported {} value '{}'", field_name, value)),
    }
}

/// Convert one C ABI engine options struct into one Rust engine options value.
/// 将单个 C ABI 引擎选项结构转换为一个 Rust 引擎选项值。
fn parse_engine_options(value: &FfiLuaEngineOptions) -> Result<LuaEngineOptions, String> {
    Ok(LuaEngineOptions::new(
        LuaVmPoolConfig {
            min_size: value.pool.min_size,
            max_size: value.pool.max_size,
            idle_ttl_secs: value.pool.idle_ttl_secs,
        },
        parse_host_options(&value.host)?,
    ))
}

/// Convert one C ABI root slice into one Rust runtime root vector.
/// 将单个 C ABI 根切片转换为一个 Rust 运行时根向量。
fn parse_skill_roots(
    skill_roots: *const FfiRuntimeSkillRoot,
    skill_roots_len: usize,
) -> Result<Vec<RuntimeSkillRoot>, String> {
    if skill_roots_len == 0 {
        return Ok(Vec::new());
    }
    if skill_roots.is_null() {
        return Err("skill_roots pointer must not be null when len > 0".to_string());
    }
    let roots = unsafe { std::slice::from_raw_parts(skill_roots, skill_roots_len) };
    roots
        .iter()
        .enumerate()
        .map(|(index, root)| {
            Ok(RuntimeSkillRoot {
                name: parse_required_string(root.name, &format!("skill_roots[{}].name", index))?,
                skills_dir: PathBuf::from(parse_required_string(
                    root.skills_dir,
                    &format!("skill_roots[{}].skills_dir", index),
                )?),
            })
        })
        .collect()
}

/// Convert one optional C ABI invocation context pointer into one Rust invocation context.
/// 将单个可选 C ABI 调用上下文指针转换为一个 Rust 调用上下文。
fn parse_invocation_context(
    value: *const FfiLuaInvocationContext,
) -> Result<Option<LuaInvocationContext>, String> {
    if value.is_null() {
        return Ok(None);
    }
    let context = unsafe { &*value };
    Ok(Some(LuaInvocationContext::new(
        parse_request_context_buffer(&context.request_context_json, "request_context_json")?,
        parse_json_value_or_empty_object_buffer(&context.client_budget_json, "client_budget_json")?,
        parse_json_value_or_empty_object_buffer(&context.tool_config_json, "tool_config_json")?,
    )))
}

/// Convert one C ABI source type integer into one Rust source type value.
/// 将单个 C ABI 来源类型整数转换为一个 Rust 来源类型值。
fn parse_source_type(value: i32) -> Result<SkillInstallSourceType, String> {
    match value {
        FFI_SOURCE_TYPE_GITHUB => Ok(SkillInstallSourceType::Github),
        FFI_SOURCE_TYPE_URL => Ok(SkillInstallSourceType::Url),
        _ => Err(format!("Unsupported source_type '{}'", value)),
    }
}

/// Convert one C ABI install request into one Rust install request value.
/// 将单个 C ABI 安装请求转换为一个 Rust 安装请求值。
fn parse_install_request(value: &FfiSkillInstallRequest) -> Result<SkillInstallRequest, String> {
    Ok(SkillInstallRequest {
        skill_id: parse_optional_string(value.skill_id, "skill_id")?,
        source: parse_optional_string(value.source, "source")?,
        source_type: parse_source_type(value.source_type)?,
    })
}

/// Convert one C ABI uninstall options struct into one Rust uninstall options value.
/// 将单个 C ABI 卸载选项结构转换为一个 Rust 卸载选项值。
fn parse_uninstall_options(value: Option<&FfiSkillUninstallOptions>) -> SkillUninstallOptions {
    match value {
        Some(value) => SkillUninstallOptions {
            remove_sqlite: value.remove_sqlite != 0,
            remove_lancedb: value.remove_lancedb != 0,
        },
        None => SkillUninstallOptions::default(),
    }
}

/// Convert one string vector into one owned C string array.
/// 将一个字符串向量转换为一个拥有所有权的 C 字符串数组。
fn alloc_string_array(values: &[String]) -> FfiStringArray {
    let mut items: Vec<FfiOwnedBuffer> =
        values.iter().map(alloc_owned_buffer_from_string).collect();
    let result = FfiStringArray {
        items: items.as_mut_ptr(),
        len: items.len(),
    };
    std::mem::forget(items);
    result
}

/// Convert one runtime entry parameter descriptor into one C ABI descriptor.
/// 将单个运行时入口参数描述转换为一个 C ABI 描述结构。
fn alloc_entry_parameter_descriptor(
    value: &RuntimeEntryParameterDescriptor,
) -> FfiRuntimeEntryParameterDescriptor {
    FfiRuntimeEntryParameterDescriptor {
        name: alloc_owned_buffer_from_string(&value.name),
        param_type: alloc_owned_buffer_from_string(&value.param_type),
        description: alloc_owned_buffer_from_string(&value.description),
        required: u8::from(value.required),
    }
}

/// Convert one runtime entry descriptor into one C ABI descriptor.
/// 将单个运行时入口描述转换为一个 C ABI 描述结构。
fn alloc_entry_descriptor(value: &RuntimeEntryDescriptor) -> FfiRuntimeEntryDescriptor {
    let mut parameters: Vec<FfiRuntimeEntryParameterDescriptor> = value
        .parameters
        .iter()
        .map(alloc_entry_parameter_descriptor)
        .collect();
    let parameters_ptr = parameters.as_mut_ptr();
    let parameters_len = parameters.len();
    std::mem::forget(parameters);
    FfiRuntimeEntryDescriptor {
        canonical_name: alloc_owned_buffer_from_string(&value.canonical_name),
        skill_id: alloc_owned_buffer_from_string(&value.skill_id),
        local_name: alloc_owned_buffer_from_string(&value.local_name),
        root_name: alloc_owned_buffer_from_string(&value.root_name),
        skill_dir: alloc_owned_buffer_from_string(&value.skill_dir),
        description: alloc_owned_buffer_from_string(&value.description),
        parameters: parameters_ptr,
        parameters_len,
    }
}

/// Convert one help node descriptor into one C ABI descriptor.
/// 将单个帮助节点描述转换为一个 C ABI 描述结构。
fn alloc_help_node_descriptor(value: &RuntimeHelpNodeDescriptor) -> FfiRuntimeHelpNodeDescriptor {
    let related_entries = alloc_string_array(&value.related_entries);
    FfiRuntimeHelpNodeDescriptor {
        flow_name: alloc_owned_buffer_from_string(&value.flow_name),
        description: alloc_owned_buffer_from_string(&value.description),
        related_entries: related_entries.items,
        related_entries_len: related_entries.len,
        is_main: u8::from(value.is_main),
    }
}

/// Convert one runtime help tree descriptor into one C ABI descriptor.
/// 将单个运行时帮助树描述转换为一个 C ABI 描述结构。
fn alloc_help_descriptor(value: &RuntimeSkillHelpDescriptor) -> FfiRuntimeSkillHelpDescriptor {
    let mut flows: Vec<FfiRuntimeHelpNodeDescriptor> =
        value.flows.iter().map(alloc_help_node_descriptor).collect();
    let flows_ptr = flows.as_mut_ptr();
    let flows_len = flows.len();
    std::mem::forget(flows);
    FfiRuntimeSkillHelpDescriptor {
        skill_id: alloc_owned_buffer_from_string(&value.skill_id),
        skill_name: alloc_owned_buffer_from_string(&value.skill_name),
        skill_version: alloc_owned_buffer_from_string(&value.skill_version),
        root_name: alloc_owned_buffer_from_string(&value.root_name),
        skill_dir: alloc_owned_buffer_from_string(&value.skill_dir),
        main: alloc_help_node_descriptor(&value.main),
        flows: flows_ptr,
        flows_len,
    }
}

/// Convert one runtime help detail into one C ABI descriptor.
/// 将单个运行时帮助详情转换为一个 C ABI 描述结构。
fn alloc_help_detail(value: &RuntimeHelpDetail) -> FfiRuntimeHelpDetail {
    let related_entries = alloc_string_array(&value.related_entries);
    FfiRuntimeHelpDetail {
        skill_id: alloc_owned_buffer_from_string(&value.skill_id),
        skill_name: alloc_owned_buffer_from_string(&value.skill_name),
        skill_version: alloc_owned_buffer_from_string(&value.skill_version),
        root_name: alloc_owned_buffer_from_string(&value.root_name),
        skill_dir: alloc_owned_buffer_from_string(&value.skill_dir),
        flow_name: alloc_owned_buffer_from_string(&value.flow_name),
        description: alloc_owned_buffer_from_string(&value.description),
        related_entries: related_entries.items,
        related_entries_len: related_entries.len,
        is_main: u8::from(value.is_main),
        content_type: alloc_owned_buffer_from_string(&value.content_type),
        content: alloc_owned_buffer_from_string(&value.content),
    }
}

/// Convert one runtime invocation result into one C ABI result.
/// 将单个运行时调用结果转换为一个 C ABI 结果结构。
fn alloc_invocation_result(value: &RuntimeInvocationResult) -> FfiRuntimeInvocationResult {
    let overflow_mode = match value.overflow_mode {
        None => 0,
        Some(crate::ToolOverflowMode::Truncate) => 1,
        Some(crate::ToolOverflowMode::Page) => 2,
    };
    FfiRuntimeInvocationResult {
        content: alloc_owned_buffer_from_string(&value.content),
        overflow_mode,
        template_hint: alloc_optional_owned_buffer_from_string(value.template_hint.as_deref()),
        content_bytes: value.content_bytes,
        content_lines: value.content_lines,
    }
}

/// Convert one install or update result into one C ABI result.
/// 将单个安装或更新结果转换为一个 C ABI 结果结构。
fn alloc_skill_apply_result(value: &SkillApplyResult) -> FfiSkillApplyResult {
    let source_type = match value.source_type {
        None => FFI_SOURCE_TYPE_ABSENT,
        Some(SkillInstallSourceType::Github) => FFI_SOURCE_TYPE_GITHUB,
        Some(SkillInstallSourceType::Url) => FFI_SOURCE_TYPE_URL,
    };
    FfiSkillApplyResult {
        skill_id: alloc_owned_buffer_from_string(&value.skill_id),
        status: alloc_owned_buffer_from_string(&value.status),
        message: alloc_owned_buffer_from_string(&value.message),
        version: alloc_optional_owned_buffer_from_string(value.version.as_deref()),
        source_type,
        source_locator: alloc_optional_owned_buffer_from_string(value.source_locator.as_deref()),
    }
}

/// Convert one uninstall result into one C ABI result.
/// 将单个卸载结果转换为一个 C ABI 结果结构。
fn alloc_skill_uninstall_result(value: &SkillUninstallResult) -> FfiSkillUninstallResult {
    FfiSkillUninstallResult {
        skill_id: alloc_owned_buffer_from_string(&value.skill_id),
        skill_removed: u8::from(value.skill_removed),
        sqlite_removed: u8::from(value.sqlite_removed),
        lancedb_removed: u8::from(value.lancedb_removed),
        sqlite_retained: u8::from(value.sqlite_retained),
        lancedb_retained: u8::from(value.lancedb_retained),
        message: alloc_owned_buffer_from_string(&value.message),
    }
}

/// Owned C-string storage used to keep one provider binding context alive during one callback invocation.
/// 用于在单次回调调用期间保持 provider 绑定上下文存活的拥有型 C 字符串存储。
struct OwnedFfiRuntimeDatabaseBindingContext {
    /// Stable host-provided space label.
    /// 宿主提供的稳定空间标签。
    space_label: CString,
    /// Stable skill identifier.
    /// 稳定技能标识符。
    skill_id: CString,
    /// Stable database binding tag.
    /// 稳定数据库绑定标签。
    binding_tag: CString,
    /// Effective physical root label.
    /// 生效物理根标签。
    root_name: CString,
    /// Physical space root path.
    /// 物理空间根路径。
    space_root: CString,
    /// Physical skill directory path.
    /// 物理技能目录路径。
    skill_dir: CString,
    /// Physical skill directory basename.
    /// 物理技能目录名称。
    skill_dir_name: CString,
    /// Default embedded database path.
    /// 默认内嵌数据库路径。
    default_database_path: CString,
    /// Borrowed C ABI view built on top of the owned strings.
    /// 构建在拥有型字符串之上的借用式 C ABI 视图。
    ffi: FfiRuntimeDatabaseBindingContext,
}

impl OwnedFfiRuntimeDatabaseBindingContext {
    /// Build one owned C ABI binding context from one runtime binding context.
    /// 基于运行时绑定上下文构造一个拥有型 C ABI 绑定上下文。
    fn from_runtime(value: &RuntimeDatabaseBindingContext) -> Result<Self, String> {
        let space_label = to_cstring(&value.space_label, "space_label")?;
        let skill_id = to_cstring(&value.skill_id, "skill_id")?;
        let binding_tag = to_cstring(&value.binding_tag, "binding_tag")?;
        let root_name = to_cstring(&value.root_name, "root_name")?;
        let space_root = to_cstring(&value.space_root, "space_root")?;
        let skill_dir = to_cstring(&value.skill_dir, "skill_dir")?;
        let skill_dir_name = to_cstring(&value.skill_dir_name, "skill_dir_name")?;
        let default_database_path =
            to_cstring(&value.default_database_path, "default_database_path")?;
        let database_kind = ffi_database_kind_code(value.database_kind);
        let ffi = FfiRuntimeDatabaseBindingContext {
            space_label: space_label.as_ptr(),
            skill_id: skill_id.as_ptr(),
            binding_tag: binding_tag.as_ptr(),
            root_name: root_name.as_ptr(),
            space_root: space_root.as_ptr(),
            skill_dir: skill_dir.as_ptr(),
            skill_dir_name: skill_dir_name.as_ptr(),
            database_kind,
            default_database_path: default_database_path.as_ptr(),
        };
        Ok(Self {
            space_label,
            skill_id,
            binding_tag,
            root_name,
            space_root,
            skill_dir,
            skill_dir_name,
            default_database_path,
            ffi,
        })
    }

    /// Borrow the underlying C ABI binding context.
    /// 借用底层 C ABI 绑定上下文。
    fn as_ffi(&self) -> FfiRuntimeDatabaseBindingContext {
        FfiRuntimeDatabaseBindingContext {
            space_label: self.space_label.as_ptr(),
            skill_id: self.skill_id.as_ptr(),
            binding_tag: self.binding_tag.as_ptr(),
            root_name: self.root_name.as_ptr(),
            space_root: self.space_root.as_ptr(),
            skill_dir: self.skill_dir.as_ptr(),
            skill_dir_name: self.skill_dir_name.as_ptr(),
            database_kind: self.ffi.database_kind,
            default_database_path: self.default_database_path.as_ptr(),
        }
    }
}

/// Build one borrowed buffer view over one owned byte slice kept alive by the caller.
/// 基于调用方持有存活期的拥有型字节切片构造一个借用缓冲视图。
fn borrowed_buffer_from_bytes(bytes: &[u8]) -> FfiBorrowedBuffer {
    if bytes.is_empty() {
        return FfiBorrowedBuffer {
            ptr: ptr::null(),
            len: 0,
        };
    }
    FfiBorrowedBuffer {
        ptr: bytes.as_ptr(),
        len: bytes.len(),
    }
}

/// Owned SQLite provider request wrapper used during one standard callback invocation.
/// 在单次标准回调调用期间使用的拥有型 SQLite provider 请求包装器。
struct OwnedFfiSqliteProviderRequest {
    /// Owned binding context backing the request.
    /// 为请求提供支撑的拥有型绑定上下文。
    _binding: OwnedFfiRuntimeDatabaseBindingContext,
    /// JSON-encoded action input payload bytes.
    /// 以 JSON 编码的动作输入载荷字节。
    _input_json: Vec<u8>,
    /// Borrowed C ABI request view.
    /// 借用式 C ABI 请求视图。
    ffi: FfiSqliteProviderRequest,
}

impl OwnedFfiSqliteProviderRequest {
    /// Build one owned SQLite provider request wrapper from one runtime request.
    /// 基于运行时请求构造一个拥有型 SQLite provider 请求包装器。
    fn from_runtime(value: &RuntimeSqliteProviderRequest) -> Result<Self, String> {
        let binding = OwnedFfiRuntimeDatabaseBindingContext::from_runtime(&value.binding)?;
        let input_json = serde_json::to_vec(&value.input)
            .map_err(|error| format!("failed to encode sqlite input json: {}", error))?;
        let ffi = FfiSqliteProviderRequest {
            action: ffi_sqlite_provider_action_code(&value.action),
            binding: binding.as_ffi(),
            input_json: borrowed_buffer_from_bytes(&input_json),
        };
        Ok(Self {
            _binding: binding,
            _input_json: input_json,
            ffi,
        })
    }

    /// Borrow the underlying C ABI request pointer.
    /// 借用底层 C ABI 请求指针。
    fn as_ptr(&self) -> *const FfiSqliteProviderRequest {
        &self.ffi
    }
}

/// Owned LanceDB provider request wrapper used during one standard callback invocation.
/// 在单次标准回调调用期间使用的拥有型 LanceDB provider 请求包装器。
struct OwnedFfiLanceDbProviderRequest {
    /// Owned binding context backing the request.
    /// 为请求提供支撑的拥有型绑定上下文。
    _binding: OwnedFfiRuntimeDatabaseBindingContext,
    /// JSON-encoded action input payload bytes.
    /// 以 JSON 编码的动作输入载荷字节。
    _input_json: Vec<u8>,
    /// Borrowed C ABI request view.
    /// 借用式 C ABI 请求视图。
    ffi: FfiLanceDbProviderRequest,
}

impl OwnedFfiLanceDbProviderRequest {
    /// Build one owned LanceDB provider request wrapper from one runtime request.
    /// 基于运行时请求构造一个拥有型 LanceDB provider 请求包装器。
    fn from_runtime(value: &RuntimeLanceDbProviderRequest) -> Result<Self, String> {
        let binding = OwnedFfiRuntimeDatabaseBindingContext::from_runtime(&value.binding)?;
        let input_json = serde_json::to_vec(&value.input)
            .map_err(|error| format!("failed to encode lancedb input json: {}", error))?;
        let ffi = FfiLanceDbProviderRequest {
            action: ffi_lancedb_provider_action_code(&value.action),
            binding: binding.as_ffi(),
            input_json: borrowed_buffer_from_bytes(&input_json),
        };
        Ok(Self {
            _binding: binding,
            _input_json: input_json,
            ffi,
        })
    }

    /// Borrow the underlying C ABI request pointer.
    /// 借用底层 C ABI 请求指针。
    fn as_ptr(&self) -> *const FfiLanceDbProviderRequest {
        &self.ffi
    }
}

/// Convert one runtime database kind into one stable FFI integer code.
/// 将运行时数据库类型转换为稳定 FFI 整数编码。
fn ffi_database_kind_code(value: RuntimeDatabaseKind) -> i32 {
    match value {
        RuntimeDatabaseKind::Sqlite => FFI_DATABASE_KIND_SQLITE,
        RuntimeDatabaseKind::LanceDb => FFI_DATABASE_KIND_LANCEDB,
    }
}

/// Convert one runtime SQLite provider action into one stable FFI integer code.
/// 将运行时 SQLite provider 动作转换为稳定 FFI 整数编码。
fn ffi_sqlite_provider_action_code(value: &RuntimeSqliteProviderAction) -> i32 {
    match value {
        RuntimeSqliteProviderAction::ExecuteScript => FFI_SQLITE_PROVIDER_ACTION_EXECUTE_SCRIPT,
        RuntimeSqliteProviderAction::ExecuteBatch => FFI_SQLITE_PROVIDER_ACTION_EXECUTE_BATCH,
        RuntimeSqliteProviderAction::QueryJson => FFI_SQLITE_PROVIDER_ACTION_QUERY_JSON,
        RuntimeSqliteProviderAction::QueryStream => FFI_SQLITE_PROVIDER_ACTION_QUERY_STREAM,
        RuntimeSqliteProviderAction::QueryStreamWaitMetrics => {
            FFI_SQLITE_PROVIDER_ACTION_QUERY_STREAM_WAIT_METRICS
        }
        RuntimeSqliteProviderAction::QueryStreamChunk => {
            FFI_SQLITE_PROVIDER_ACTION_QUERY_STREAM_CHUNK
        }
        RuntimeSqliteProviderAction::QueryStreamClose => {
            FFI_SQLITE_PROVIDER_ACTION_QUERY_STREAM_CLOSE
        }
        RuntimeSqliteProviderAction::TokenizeText => FFI_SQLITE_PROVIDER_ACTION_TOKENIZE_TEXT,
        RuntimeSqliteProviderAction::UpsertCustomWord => {
            FFI_SQLITE_PROVIDER_ACTION_UPSERT_CUSTOM_WORD
        }
        RuntimeSqliteProviderAction::RemoveCustomWord => {
            FFI_SQLITE_PROVIDER_ACTION_REMOVE_CUSTOM_WORD
        }
        RuntimeSqliteProviderAction::ListCustomWords => {
            FFI_SQLITE_PROVIDER_ACTION_LIST_CUSTOM_WORDS
        }
        RuntimeSqliteProviderAction::EnsureFtsIndex => FFI_SQLITE_PROVIDER_ACTION_ENSURE_FTS_INDEX,
        RuntimeSqliteProviderAction::RebuildFtsIndex => {
            FFI_SQLITE_PROVIDER_ACTION_REBUILD_FTS_INDEX
        }
        RuntimeSqliteProviderAction::UpsertFtsDocument => {
            FFI_SQLITE_PROVIDER_ACTION_UPSERT_FTS_DOCUMENT
        }
        RuntimeSqliteProviderAction::DeleteFtsDocument => {
            FFI_SQLITE_PROVIDER_ACTION_DELETE_FTS_DOCUMENT
        }
        RuntimeSqliteProviderAction::SearchFts => FFI_SQLITE_PROVIDER_ACTION_SEARCH_FTS,
    }
}

/// Convert one runtime LanceDB provider action into one stable FFI integer code.
/// 将运行时 LanceDB provider 动作转换为稳定 FFI 整数编码。
fn ffi_lancedb_provider_action_code(value: &RuntimeLanceDbProviderAction) -> i32 {
    match value {
        RuntimeLanceDbProviderAction::CreateTable => FFI_LANCEDB_PROVIDER_ACTION_CREATE_TABLE,
        RuntimeLanceDbProviderAction::VectorUpsert => FFI_LANCEDB_PROVIDER_ACTION_VECTOR_UPSERT,
        RuntimeLanceDbProviderAction::VectorSearch => FFI_LANCEDB_PROVIDER_ACTION_VECTOR_SEARCH,
        RuntimeLanceDbProviderAction::Delete => FFI_LANCEDB_PROVIDER_ACTION_DELETE,
        RuntimeLanceDbProviderAction::DropTable => FFI_LANCEDB_PROVIDER_ACTION_DROP_TABLE,
    }
}

/// Invoke one host-supplied JSON provider callback and copy the returned string into Rust ownership.
/// 调用宿主提供的 JSON provider 回调，并把返回字符串复制到 Rust 所有权下。
fn invoke_json_provider_callback(
    callback: FfiJsonProviderCallback,
    user_data: usize,
    request_json: &str,
) -> Result<String, String> {
    let request_bytes = request_json.as_bytes();
    let mut response_out = FfiOwnedBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let mut error_out = FfiOwnedBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let status = unsafe {
        callback(
            FfiBorrowedBuffer {
                ptr: request_bytes.as_ptr(),
                len: request_bytes.len(),
            },
            user_data as *mut c_void,
            &mut response_out,
            &mut error_out,
        )
    };
    let callback_error =
        take_optional_owned_ffi_string_buffer(error_out, "json host provider callback error_out")?;
    if status != FFI_STATUS_OK {
        unsafe { free_ffi_bytes(response_out.ptr, response_out.len) };
        return Err(callback_error.unwrap_or_else(|| {
            "json host provider callback returned failure without error message".to_string()
        }));
    }
    let response = take_optional_owned_ffi_string_buffer(
        response_out,
        "json host provider callback response_out",
    )?
    .ok_or_else(|| "json host provider callback returned empty response_out".to_string())?;
    if let Some(message) = callback_error {
        if !message.is_empty() {
            return Err(format!(
                "json host provider callback returned unexpected error text on success: {}",
                message
            ));
        }
    }
    Ok(response)
}

/// Invoke one host-supplied standard SQLite provider callback and decode the returned JSON payload.
/// 调用宿主提供的标准 SQLite provider 回调，并解码返回的 JSON 载荷。
fn invoke_standard_sqlite_provider_callback(
    callback: FfiSqliteProviderCallback,
    user_data: usize,
    request: &RuntimeSqliteProviderRequest,
) -> Result<Value, String> {
    let request = OwnedFfiSqliteProviderRequest::from_runtime(request)?;
    let mut response_json_out = FfiOwnedBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let mut error_out = FfiOwnedBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let status = unsafe {
        callback(
            request.as_ptr(),
            user_data as *mut c_void,
            &mut response_json_out,
            &mut error_out,
        )
    };
    let callback_error = take_optional_owned_ffi_string_buffer(
        error_out,
        "sqlite host provider callback error_out",
    )?;
    if status != FFI_STATUS_OK {
        unsafe { free_ffi_bytes(response_json_out.ptr, response_json_out.len) };
        return Err(callback_error.unwrap_or_else(|| {
            "sqlite host provider callback returned failure without error message".to_string()
        }));
    }
    let response_json = take_optional_owned_ffi_string_buffer(
        response_json_out,
        "sqlite host provider callback response_json_out",
    )?
    .ok_or_else(|| "sqlite host provider callback returned empty response_json_out".to_string())?;
    if let Some(message) = callback_error {
        if !message.is_empty() {
            return Err(format!(
                "sqlite host provider callback returned unexpected error text on success: {}",
                message
            ));
        }
    }
    serde_json::from_str(&response_json).map_err(|error| {
        format!(
            "failed to parse sqlite provider callback response json: {}",
            error
        )
    })
}

/// Invoke one host-supplied standard LanceDB provider callback and decode the returned payload.
/// 调用宿主提供的标准 LanceDB provider 回调，并解码返回的载荷。
fn invoke_standard_lancedb_provider_callback(
    callback: FfiLanceDbProviderCallback,
    user_data: usize,
    request: &RuntimeLanceDbProviderRequest,
) -> Result<RuntimeLanceDbProviderResult, String> {
    let request = OwnedFfiLanceDbProviderRequest::from_runtime(request)?;
    let mut meta_json_out = FfiOwnedBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let mut data_out = FfiOwnedBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let mut error_out = FfiOwnedBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let status = unsafe {
        callback(
            request.as_ptr(),
            user_data as *mut c_void,
            &mut meta_json_out,
            &mut data_out,
            &mut error_out,
        )
    };
    let callback_error = take_optional_owned_ffi_string_buffer(
        error_out,
        "lancedb host provider callback error_out",
    )?;
    if status != FFI_STATUS_OK {
        unsafe {
            free_ffi_bytes(meta_json_out.ptr, meta_json_out.len);
            free_ffi_bytes(data_out.ptr, data_out.len);
        }
        return Err(callback_error.unwrap_or_else(|| {
            "lancedb host provider callback returned failure without error message".to_string()
        }));
    }
    let meta_json = take_optional_owned_ffi_string_buffer(
        meta_json_out,
        "lancedb host provider callback meta_json_out",
    )?
    .unwrap_or_else(|| "{}".to_string());
    let meta = serde_json::from_str::<Value>(&meta_json).map_err(|error| {
        format!(
            "failed to parse lancedb provider callback meta json: {}",
            error
        )
    })?;
    let bytes =
        take_optional_owned_ffi_buffer(data_out, "lancedb host provider callback data_out")?
            .unwrap_or_default();
    if let Some(message) = callback_error {
        if !message.is_empty() {
            return Err(format!(
                "lancedb host provider callback returned unexpected error text on success: {}",
                message
            ));
        }
    }
    Ok(RuntimeLanceDbProviderResult::binary(meta, bytes))
}

/// Copy one optional owned FFI buffer into Rust ownership and free the original allocation.
/// 将单个可选拥有型 FFI 缓冲复制到 Rust 所有权，并释放原始分配。
fn take_optional_owned_ffi_buffer(
    value: FfiOwnedBuffer,
    field_name: &str,
) -> Result<Option<Vec<u8>>, String> {
    if value.ptr.is_null() {
        if value.len == 0 {
            return Ok(None);
        }
        return Err(format!(
            "{} returned null ptr with non-zero len",
            field_name
        ));
    }
    let bytes = unsafe { std::slice::from_raw_parts(value.ptr, value.len) }.to_vec();
    unsafe { free_ffi_bytes(value.ptr, value.len) };
    Ok(Some(bytes))
}

/// Copy one optional owned UTF-8 buffer into Rust string ownership and free the original allocation.
/// 将单个可选拥有型 UTF-8 缓冲复制到 Rust 字符串所有权，并释放原始分配。
fn take_optional_owned_ffi_string_buffer(
    value: FfiOwnedBuffer,
    field_name: &str,
) -> Result<Option<String>, String> {
    let bytes = match take_optional_owned_ffi_buffer(value, field_name)? {
        Some(bytes) => bytes,
        None => return Ok(None),
    };
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|error| format!("{} returned non-utf8 bytes: {}", field_name, error))
}

/// Free one owned byte buffer allocated by one FFI callback helper.
/// 释放由某个 FFI 回调辅助函数分配的拥有型字节缓冲。
unsafe fn free_ffi_bytes(value: *mut u8, len: usize) {
    if value.is_null() || len == 0 {
        return;
    }
    let _ = unsafe { Vec::from_raw_parts(value, len, len) };
}

/// Free one owned string array and all nested string items.
/// 释放单个拥有所有权的字符串数组以及其嵌套字符串条目。
unsafe fn free_string_array_parts(items: *mut FfiOwnedBuffer, len: usize) {
    if items.is_null() || len == 0 {
        return;
    }
    let values = unsafe { Vec::from_raw_parts(items, len, len) };
    for value in values {
        unsafe { vulcan_luaskills_ffi_buffer_free(value) };
    }
}

/// Free one owned entry parameter descriptor.
/// 释放单个拥有所有权的入口参数描述结构。
unsafe fn free_entry_parameter_descriptor(value: FfiRuntimeEntryParameterDescriptor) {
    unsafe { vulcan_luaskills_ffi_buffer_free(value.name) };
    unsafe { vulcan_luaskills_ffi_buffer_free(value.param_type) };
    unsafe { vulcan_luaskills_ffi_buffer_free(value.description) };
}

/// Free one owned entry descriptor.
/// 释放单个拥有所有权的入口描述结构。
unsafe fn free_entry_descriptor(value: FfiRuntimeEntryDescriptor) {
    unsafe { vulcan_luaskills_ffi_buffer_free(value.canonical_name) };
    unsafe { vulcan_luaskills_ffi_buffer_free(value.skill_id) };
    unsafe { vulcan_luaskills_ffi_buffer_free(value.local_name) };
    unsafe { vulcan_luaskills_ffi_buffer_free(value.root_name) };
    unsafe { vulcan_luaskills_ffi_buffer_free(value.skill_dir) };
    unsafe { vulcan_luaskills_ffi_buffer_free(value.description) };
    if !value.parameters.is_null() && value.parameters_len > 0 {
        let parameters = unsafe {
            Vec::from_raw_parts(value.parameters, value.parameters_len, value.parameters_len)
        };
        for parameter in parameters {
            unsafe { free_entry_parameter_descriptor(parameter) };
        }
    }
}

/// Free one owned help node descriptor.
/// 释放单个拥有所有权的帮助节点描述结构。
unsafe fn free_help_node_descriptor(value: FfiRuntimeHelpNodeDescriptor) {
    unsafe { vulcan_luaskills_ffi_buffer_free(value.flow_name) };
    unsafe { vulcan_luaskills_ffi_buffer_free(value.description) };
    unsafe { free_string_array_parts(value.related_entries, value.related_entries_len) };
}

/// Write one successful status code.
/// 写入一个成功状态码。
fn ffi_ok_status(error_out: *mut FfiOwnedBuffer) -> i32 {
    clear_error_out(error_out);
    FFI_STATUS_OK
}

/// Write one failed status code and error text.
/// 写入一个失败状态码与错误文本。
fn ffi_error_status(error_out: *mut FfiOwnedBuffer, message: impl Into<String>) -> i32 {
    set_error_out(error_out, message);
    FFI_STATUS_ERROR
}

/// Clone one host string into one LuaSkills-owned heap string so callbacks can return safely across FFI.
/// 将宿主字符串克隆到 LuaSkills 管理的堆字符串，便于回调安全跨 FFI 返回。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_string_clone(value: *const c_char) -> *mut c_char {
    if value.is_null() {
        return alloc_c_string("");
    }
    let text = unsafe { CStr::from_ptr(value) }
        .to_string_lossy()
        .to_string();
    alloc_c_string(&text)
}

/// Clone one host buffer into one LuaSkills-owned buffer container for callback returns.
/// 将宿主缓冲克隆到 LuaSkills 管理的缓冲容器，便于 callback 返回。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_buffer_clone(
    value: *const u8,
    len: usize,
    buffer_out: *mut FfiOwnedBuffer,
    error_out: *mut FfiOwnedBuffer,
) -> i32 {
    clear_error_out(error_out);
    clear_out_buffer(buffer_out);
    if buffer_out.is_null() {
        return ffi_error_status(error_out, "buffer_out must not be null");
    }
    if value.is_null() && len != 0 {
        return ffi_error_status(error_out, "value must not be null when len > 0");
    }
    let slice = if len == 0 {
        &[][..]
    } else {
        unsafe { std::slice::from_raw_parts(value, len) }
    };
    unsafe {
        *buffer_out = alloc_owned_buffer_from_bytes(slice);
    }
    ffi_ok_status(error_out)
}

/// Clone one host byte buffer into one LuaSkills-owned heap buffer for standard callback returns.
/// 将宿主字节缓冲克隆到 LuaSkills 管理的堆缓冲，用于标准回调返回。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_bytes_clone(value: *const u8, len: usize) -> *mut u8 {
    if value.is_null() || len == 0 {
        return ptr::null_mut();
    }
    let slice = unsafe { std::slice::from_raw_parts(value, len) };
    let mut bytes = slice.to_vec();
    let pointer = bytes.as_mut_ptr();
    std::mem::forget(bytes);
    pointer
}

/// Free one LuaSkills-owned heap byte buffer created by `vulcan_luaskills_ffi_bytes_clone`.
/// 释放由 `vulcan_luaskills_ffi_bytes_clone` 创建的 LuaSkills 自主管理堆字节缓冲。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_bytes_free(value: *mut u8, len: usize) {
    unsafe { free_ffi_bytes(value, len) };
}

/// Free one LuaSkills-owned buffer container created by `vulcan_luaskills_ffi_buffer_clone`.
/// 释放由 `vulcan_luaskills_ffi_buffer_clone` 创建的 LuaSkills 自主管理缓冲容器。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_buffer_free(value: FfiOwnedBuffer) {
    unsafe { free_ffi_bytes(value.ptr, value.len) };
}

/// Register or clear one SQLite standard provider callback for host-managed database integration.
/// 为宿主管理数据库集成注册或清理一个 SQLite 标准 provider 回调。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_set_sqlite_provider_callback(
    callback: Option<FfiSqliteProviderCallback>,
    user_data: *mut c_void,
    error_out: *mut FfiOwnedBuffer,
) -> i32 {
    clear_error_out(error_out);
    let wrapped = callback.map(|callback_fn| {
        let user_data = user_data as usize;
        std::sync::Arc::new(move |request: &RuntimeSqliteProviderRequest| {
            invoke_standard_sqlite_provider_callback(callback_fn, user_data, request)
        }) as RuntimeSqliteProviderCallback
    });
    set_sqlite_provider_callback(wrapped);
    ffi_ok_status(error_out)
}

/// Register or clear one LanceDB standard provider callback for host-managed database integration.
/// 为宿主管理数据库集成注册或清理一个 LanceDB 标准 provider 回调。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_set_lancedb_provider_callback(
    callback: Option<FfiLanceDbProviderCallback>,
    user_data: *mut c_void,
    error_out: *mut FfiOwnedBuffer,
) -> i32 {
    clear_error_out(error_out);
    let wrapped = callback.map(|callback_fn| {
        let user_data = user_data as usize;
        std::sync::Arc::new(move |request: &RuntimeLanceDbProviderRequest| {
            invoke_standard_lancedb_provider_callback(callback_fn, user_data, request)
        }) as RuntimeLanceDbProviderCallback
    });
    set_lancedb_provider_callback(wrapped);
    ffi_ok_status(error_out)
}

/// Register or clear one SQLite JSON provider callback for cross-language host integration.
/// 为跨语言宿主集成注册或清理一个 SQLite JSON provider 回调。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_set_sqlite_provider_json_callback(
    callback: Option<FfiJsonProviderCallback>,
    user_data: *mut c_void,
    error_out: *mut FfiOwnedBuffer,
) -> i32 {
    clear_error_out(error_out);
    let wrapped = callback.map(|callback_fn| {
        let user_data = user_data as usize;
        std::sync::Arc::new(move |request_json: &str| {
            invoke_json_provider_callback(callback_fn, user_data, request_json)
        }) as crate::host::database::RuntimeSqliteProviderJsonCallback
    });
    set_sqlite_provider_json_callback(wrapped);
    ffi_ok_status(error_out)
}

/// Register or clear one LanceDB JSON provider callback for cross-language host integration.
/// 为跨语言宿主集成注册或清理一个 LanceDB JSON provider 回调。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_set_lancedb_provider_json_callback(
    callback: Option<FfiJsonProviderCallback>,
    user_data: *mut c_void,
    error_out: *mut FfiOwnedBuffer,
) -> i32 {
    clear_error_out(error_out);
    let wrapped = callback.map(|callback_fn| {
        let user_data = user_data as usize;
        std::sync::Arc::new(move |request_json: &str| {
            invoke_json_provider_callback(callback_fn, user_data, request_json)
        }) as crate::host::database::RuntimeLanceDbProviderJsonCallback
    });
    set_lancedb_provider_json_callback(wrapped);
    ffi_ok_status(error_out)
}

/// Free one string array result allocated by the standard FFI layer.
/// 释放由标准 FFI 层分配的单个字符串数组结果。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_string_array_free(value: *mut FfiStringArray) {
    if value.is_null() {
        return;
    }
    let value = unsafe { Box::from_raw(value) };
    unsafe { free_string_array_parts(value.items, value.len) };
}

/// Free one entry descriptor list allocated by the standard FFI layer.
/// 释放由标准 FFI 层分配的单个入口描述列表。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_entry_list_free(
    value: *mut FfiRuntimeEntryDescriptorList,
) {
    if value.is_null() {
        return;
    }
    let value = unsafe { Box::from_raw(value) };
    if !value.items.is_null() && value.len > 0 {
        let items = unsafe { Vec::from_raw_parts(value.items, value.len, value.len) };
        for item in items {
            unsafe { free_entry_descriptor(item) };
        }
    }
}

/// Free one help descriptor list allocated by the standard FFI layer.
/// 释放由标准 FFI 层分配的单个帮助描述列表。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_help_list_free(
    value: *mut FfiRuntimeSkillHelpDescriptorList,
) {
    if value.is_null() {
        return;
    }
    let value = unsafe { Box::from_raw(value) };
    if !value.items.is_null() && value.len > 0 {
        let items = unsafe { Vec::from_raw_parts(value.items, value.len, value.len) };
        for item in items {
            unsafe { vulcan_luaskills_ffi_buffer_free(item.skill_id) };
            unsafe { vulcan_luaskills_ffi_buffer_free(item.skill_name) };
            unsafe { vulcan_luaskills_ffi_buffer_free(item.skill_version) };
            unsafe { vulcan_luaskills_ffi_buffer_free(item.root_name) };
            unsafe { vulcan_luaskills_ffi_buffer_free(item.skill_dir) };
            unsafe { free_help_node_descriptor(item.main) };
            if !item.flows.is_null() && item.flows_len > 0 {
                let flows =
                    unsafe { Vec::from_raw_parts(item.flows, item.flows_len, item.flows_len) };
                for flow in flows {
                    unsafe { free_help_node_descriptor(flow) };
                }
            }
        }
    }
}

/// Free one help detail allocated by the standard FFI layer.
/// 释放由标准 FFI 层分配的单个帮助详情。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_help_detail_free(value: *mut FfiRuntimeHelpDetail) {
    if value.is_null() {
        return;
    }
    let value = unsafe { *Box::from_raw(value) };
    unsafe { vulcan_luaskills_ffi_buffer_free(value.skill_id) };
    unsafe { vulcan_luaskills_ffi_buffer_free(value.skill_name) };
    unsafe { vulcan_luaskills_ffi_buffer_free(value.skill_version) };
    unsafe { vulcan_luaskills_ffi_buffer_free(value.root_name) };
    unsafe { vulcan_luaskills_ffi_buffer_free(value.skill_dir) };
    unsafe { vulcan_luaskills_ffi_buffer_free(value.flow_name) };
    unsafe { vulcan_luaskills_ffi_buffer_free(value.description) };
    unsafe { free_string_array_parts(value.related_entries, value.related_entries_len) };
    unsafe { vulcan_luaskills_ffi_buffer_free(value.content_type) };
    unsafe { vulcan_luaskills_ffi_buffer_free(value.content) };
}

/// Free one invocation result allocated by the standard FFI layer.
/// 释放由标准 FFI 层分配的单个调用结果。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_invocation_result_free(
    value: *mut FfiRuntimeInvocationResult,
) {
    if value.is_null() {
        return;
    }
    let value = unsafe { *Box::from_raw(value) };
    unsafe { vulcan_luaskills_ffi_buffer_free(value.content) };
    unsafe { vulcan_luaskills_ffi_buffer_free(value.template_hint) };
}

/// Free one install or update result allocated by the standard FFI layer.
/// 释放由标准 FFI 层分配的单个安装或更新结果。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_skill_apply_result_free(
    value: *mut FfiSkillApplyResult,
) {
    if value.is_null() {
        return;
    }
    let value = unsafe { *Box::from_raw(value) };
    unsafe { vulcan_luaskills_ffi_buffer_free(value.skill_id) };
    unsafe { vulcan_luaskills_ffi_buffer_free(value.status) };
    unsafe { vulcan_luaskills_ffi_buffer_free(value.message) };
    unsafe { vulcan_luaskills_ffi_buffer_free(value.version) };
    unsafe { vulcan_luaskills_ffi_buffer_free(value.source_locator) };
}

/// Free one uninstall result allocated by the standard FFI layer.
/// 释放由标准 FFI 层分配的单个卸载结果。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_skill_uninstall_result_free(
    value: *mut FfiSkillUninstallResult,
) {
    if value.is_null() {
        return;
    }
    let value = unsafe { *Box::from_raw(value) };
    unsafe { vulcan_luaskills_ffi_buffer_free(value.skill_id) };
    unsafe { vulcan_luaskills_ffi_buffer_free(value.message) };
}

/// Return the stable FFI version string through the standard C ABI surface.
/// 通过标准 C ABI 接口返回稳定的 FFI 版本字符串。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_version(
    version_out: *mut FfiOwnedBuffer,
    error_out: *mut FfiOwnedBuffer,
) -> i32 {
    clear_error_out(error_out);
    clear_out_buffer(version_out);
    if version_out.is_null() {
        return ffi_error_status(error_out, "version_out must not be null");
    }
    unsafe { *version_out = alloc_owned_buffer_from_string(crate::ffi::FFI_VERSION) };
    ffi_ok_status(error_out)
}

/// Return the exported FFI entrypoint names through the standard C ABI surface.
/// 通过标准 C ABI 接口返回已导出 FFI 入口点名称。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_describe(
    functions_out: *mut *mut FfiStringArray,
    error_out: *mut FfiOwnedBuffer,
) -> i32 {
    clear_error_out(error_out);
    clear_out_ptr(functions_out);
    if functions_out.is_null() {
        return ffi_error_status(error_out, "functions_out must not be null");
    }
    let values = crate::ffi::exported_ffi_function_names();
    unsafe { *functions_out = Box::into_raw(Box::new(alloc_string_array(&values))) };
    ffi_ok_status(error_out)
}

/// Create one runtime engine through the standard C ABI surface.
/// 通过标准 C ABI 接口创建单个运行时引擎。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_engine_new(
    options: *const FfiLuaEngineOptions,
    engine_id_out: *mut u64,
    error_out: *mut FfiOwnedBuffer,
) -> i32 {
    clear_error_out(error_out);
    clear_out_u64(engine_id_out);
    if options.is_null() {
        return ffi_error_status(error_out, "options must not be null");
    }
    if engine_id_out.is_null() {
        return ffi_error_status(error_out, "engine_id_out must not be null");
    }
    let options = match parse_engine_options(unsafe { &*options }) {
        Ok(options) => options,
        Err(error) => return ffi_error_status(error_out, error),
    };
    match LuaEngine::new(options) {
        Ok(engine) => {
            let engine_id = FFI_ENGINE_COUNTER.fetch_add(1, Ordering::Relaxed);
            match ffi_engine_registry().lock() {
                Ok(mut registry) => {
                    registry.insert(engine_id, crate::ffi::FfiEngineSlot::new(engine));
                    unsafe { *engine_id_out = engine_id };
                    ffi_ok_status(error_out)
                }
                Err(_) => ffi_error_status(error_out, "FFI engine registry lock poisoned"),
            }
        }
        Err(error) => ffi_error_status(error_out, error.to_string()),
    }
}

/// Free one runtime engine through the standard C ABI surface.
/// 通过标准 C ABI 接口释放单个运行时引擎。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_engine_free(
    engine_id: u64,
    error_out: *mut FfiOwnedBuffer,
) -> i32 {
    clear_error_out(error_out);
    match ffi_engine_registry().lock() {
        Ok(mut registry) => {
            if registry.remove(&engine_id).is_none() {
                ffi_error_status(error_out, format!("FFI engine {} not found", engine_id))
            } else {
                ffi_ok_status(error_out)
            }
        }
        Err(_) => ffi_error_status(error_out, "FFI engine registry lock poisoned"),
    }
}

/// Load skills from one legacy directory pair through the standard C ABI surface.
/// 通过标准 C ABI 接口从一组旧目录风格根参数加载技能。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_load_from_dirs(
    engine_id: u64,
    base_dir: *const c_char,
    override_dir: *const c_char,
    error_out: *mut FfiOwnedBuffer,
) -> i32 {
    clear_error_out(error_out);
    let base_dir = match parse_required_string(base_dir, "base_dir") {
        Ok(value) => PathBuf::from(value),
        Err(error) => return ffi_error_status(error_out, error),
    };
    let override_dir = match parse_optional_string(override_dir, "override_dir") {
        Ok(value) => value.map(PathBuf::from),
        Err(error) => return ffi_error_status(error_out, error),
    };
    match with_engine_mut(engine_id, |engine| {
        engine
            .load_from_dirs(&base_dir, override_dir.as_deref())
            .map_err(|error| error.to_string())
    }) {
        Ok(()) => ffi_ok_status(error_out),
        Err(error) => ffi_error_status(error_out, error),
    }
}

/// Load skills from one ordered root chain through the standard C ABI surface.
/// 通过标准 C ABI 接口从一条有序根链加载技能。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_load_from_roots(
    engine_id: u64,
    skill_roots: *const FfiRuntimeSkillRoot,
    skill_roots_len: usize,
    error_out: *mut FfiOwnedBuffer,
) -> i32 {
    clear_error_out(error_out);
    let skill_roots = match parse_skill_roots(skill_roots, skill_roots_len) {
        Ok(skill_roots) => skill_roots,
        Err(error) => return ffi_error_status(error_out, error),
    };
    match with_engine_mut(engine_id, |engine| {
        engine
            .load_from_roots(&skill_roots)
            .map_err(|error| error.to_string())
    }) {
        Ok(()) => ffi_ok_status(error_out),
        Err(error) => ffi_error_status(error_out, error),
    }
}

/// Reload skills from one legacy directory pair through the standard C ABI surface.
/// 通过标准 C ABI 接口从一组旧目录风格根参数重载技能。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_reload_from_dirs(
    engine_id: u64,
    base_dir: *const c_char,
    override_dir: *const c_char,
    error_out: *mut FfiOwnedBuffer,
) -> i32 {
    clear_error_out(error_out);
    let base_dir = match parse_required_string(base_dir, "base_dir") {
        Ok(value) => PathBuf::from(value),
        Err(error) => return ffi_error_status(error_out, error),
    };
    let override_dir = match parse_optional_string(override_dir, "override_dir") {
        Ok(value) => value.map(PathBuf::from),
        Err(error) => return ffi_error_status(error_out, error),
    };
    match with_engine_mut(engine_id, |engine| {
        engine
            .reload_from_dirs(&base_dir, override_dir.as_deref())
            .map_err(|error| error.to_string())
    }) {
        Ok(()) => ffi_ok_status(error_out),
        Err(error) => ffi_error_status(error_out, error),
    }
}

/// Reload skills from one ordered root chain through the standard C ABI surface.
/// 通过标准 C ABI 接口从一条有序根链重载技能。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_reload_from_roots(
    engine_id: u64,
    skill_roots: *const FfiRuntimeSkillRoot,
    skill_roots_len: usize,
    error_out: *mut FfiOwnedBuffer,
) -> i32 {
    clear_error_out(error_out);
    let skill_roots = match parse_skill_roots(skill_roots, skill_roots_len) {
        Ok(skill_roots) => skill_roots,
        Err(error) => return ffi_error_status(error_out, error),
    };
    match with_engine_mut(engine_id, |engine| {
        engine
            .reload_from_roots(&skill_roots)
            .map_err(|error| error.to_string())
    }) {
        Ok(()) => ffi_ok_status(error_out),
        Err(error) => ffi_error_status(error_out, error),
    }
}

/// List runtime entries through the standard C ABI surface.
/// 通过标准 C ABI 接口列出运行时入口。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_list_entries(
    engine_id: u64,
    entries_out: *mut *mut FfiRuntimeEntryDescriptorList,
    error_out: *mut FfiOwnedBuffer,
) -> i32 {
    clear_error_out(error_out);
    clear_out_ptr(entries_out);
    if entries_out.is_null() {
        return ffi_error_status(error_out, "entries_out must not be null");
    }
    match with_engine(engine_id, |engine| Ok(engine.list_entries())) {
        Ok(entries) => {
            let mut items: Vec<FfiRuntimeEntryDescriptor> =
                entries.iter().map(alloc_entry_descriptor).collect();
            let list = FfiRuntimeEntryDescriptorList {
                items: items.as_mut_ptr(),
                len: items.len(),
            };
            std::mem::forget(items);
            unsafe { *entries_out = Box::into_raw(Box::new(list)) };
            ffi_ok_status(error_out)
        }
        Err(error) => ffi_error_status(error_out, error),
    }
}

/// List runtime help trees through the standard C ABI surface.
/// 通过标准 C ABI 接口列出运行时帮助树。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_list_skill_help(
    engine_id: u64,
    help_out: *mut *mut FfiRuntimeSkillHelpDescriptorList,
    error_out: *mut FfiOwnedBuffer,
) -> i32 {
    clear_error_out(error_out);
    clear_out_ptr(help_out);
    if help_out.is_null() {
        return ffi_error_status(error_out, "help_out must not be null");
    }
    match with_engine(engine_id, |engine| Ok(engine.list_skill_help())) {
        Ok(help_descriptors) => {
            let mut items: Vec<FfiRuntimeSkillHelpDescriptor> =
                help_descriptors.iter().map(alloc_help_descriptor).collect();
            let list = FfiRuntimeSkillHelpDescriptorList {
                items: items.as_mut_ptr(),
                len: items.len(),
            };
            std::mem::forget(items);
            unsafe { *help_out = Box::into_raw(Box::new(list)) };
            ffi_ok_status(error_out)
        }
        Err(error) => ffi_error_status(error_out, error),
    }
}

/// Render one help detail through the standard C ABI surface.
/// 通过标准 C ABI 接口渲染单个帮助详情。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_render_skill_help_detail(
    engine_id: u64,
    skill_id: *const c_char,
    flow_name: *const c_char,
    request_context_json: FfiBorrowedBuffer,
    detail_out: *mut *mut FfiRuntimeHelpDetail,
    error_out: *mut FfiOwnedBuffer,
) -> i32 {
    clear_error_out(error_out);
    clear_out_ptr(detail_out);
    if detail_out.is_null() {
        return ffi_error_status(error_out, "detail_out must not be null");
    }
    let skill_id = match parse_required_string(skill_id, "skill_id") {
        Ok(skill_id) => skill_id,
        Err(error) => return ffi_error_status(error_out, error),
    };
    let flow_name = match parse_required_string(flow_name, "flow_name") {
        Ok(flow_name) => flow_name,
        Err(error) => return ffi_error_status(error_out, error),
    };
    let request_context =
        match parse_request_context_buffer(&request_context_json, "request_context_json") {
            Ok(request_context) => request_context,
            Err(error) => return ffi_error_status(error_out, error),
        };
    match with_engine(engine_id, |engine| {
        engine.render_skill_help_detail(&skill_id, &flow_name, request_context.as_ref())
    }) {
        Ok(Some(detail)) => {
            unsafe { *detail_out = Box::into_raw(Box::new(alloc_help_detail(&detail))) };
            ffi_ok_status(error_out)
        }
        Ok(None) => ffi_error_status(error_out, "Requested help detail was not found"),
        Err(error) => ffi_error_status(error_out, error),
    }
}

/// Resolve prompt argument completions through the standard C ABI surface.
/// 通过标准 C ABI 接口解析提示词参数补全项。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_prompt_argument_completions(
    engine_id: u64,
    prompt_name: *const c_char,
    argument_name: *const c_char,
    values_out: *mut *mut FfiStringArray,
    error_out: *mut FfiOwnedBuffer,
) -> i32 {
    clear_error_out(error_out);
    clear_out_ptr(values_out);
    if values_out.is_null() {
        return ffi_error_status(error_out, "values_out must not be null");
    }
    let prompt_name = match parse_required_string(prompt_name, "prompt_name") {
        Ok(prompt_name) => prompt_name,
        Err(error) => return ffi_error_status(error_out, error),
    };
    let argument_name = match parse_required_string(argument_name, "argument_name") {
        Ok(argument_name) => argument_name,
        Err(error) => return ffi_error_status(error_out, error),
    };
    match with_engine(engine_id, |engine| {
        Ok(engine.prompt_argument_completions(&prompt_name, &argument_name))
    }) {
        Ok(Some(values)) => {
            unsafe { *values_out = Box::into_raw(Box::new(alloc_string_array(&values))) };
            ffi_ok_status(error_out)
        }
        Ok(None) => {
            unsafe { *values_out = Box::into_raw(Box::new(alloc_string_array(&[]))) };
            ffi_ok_status(error_out)
        }
        Err(error) => ffi_error_status(error_out, error),
    }
}

/// Check whether one tool belongs to a Lua skill through the standard C ABI surface.
/// 通过标准 C ABI 接口检查单个工具是否属于 Lua 技能。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_is_skill(
    engine_id: u64,
    tool_name: *const c_char,
    value_out: *mut u8,
    error_out: *mut FfiOwnedBuffer,
) -> i32 {
    clear_error_out(error_out);
    clear_out_u8(value_out);
    if value_out.is_null() {
        return ffi_error_status(error_out, "value_out must not be null");
    }
    let tool_name = match parse_required_string(tool_name, "tool_name") {
        Ok(tool_name) => tool_name,
        Err(error) => return ffi_error_status(error_out, error),
    };
    match with_engine(engine_id, |engine| Ok(engine.is_skill(&tool_name))) {
        Ok(value) => {
            unsafe { *value_out = u8::from(value) };
            ffi_ok_status(error_out)
        }
        Err(error) => ffi_error_status(error_out, error),
    }
}

/// Resolve the owning skill id of one tool through the standard C ABI surface.
/// 通过标准 C ABI 接口解析单个工具所属的技能标识符。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_skill_name_for_tool(
    engine_id: u64,
    tool_name: *const c_char,
    skill_id_out: *mut FfiOwnedBuffer,
    error_out: *mut FfiOwnedBuffer,
) -> i32 {
    clear_error_out(error_out);
    clear_out_buffer(skill_id_out);
    if skill_id_out.is_null() {
        return ffi_error_status(error_out, "skill_id_out must not be null");
    }
    let tool_name = match parse_required_string(tool_name, "tool_name") {
        Ok(tool_name) => tool_name,
        Err(error) => return ffi_error_status(error_out, error),
    };
    match with_engine(engine_id, |engine| {
        Ok(engine.skill_name_for_tool(&tool_name))
    }) {
        Ok(skill_id) => {
            unsafe { *skill_id_out = alloc_optional_owned_buffer_from_string(skill_id.as_deref()) };
            ffi_ok_status(error_out)
        }
        Err(error) => ffi_error_status(error_out, error),
    }
}

/// Call one loaded skill entry through the standard C ABI surface.
/// 通过标准 C ABI 接口调用单个已加载技能入口。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_call_skill(
    engine_id: u64,
    tool_name: *const c_char,
    args_json: FfiBorrowedBuffer,
    invocation_context: *const FfiLuaInvocationContext,
    result_out: *mut *mut FfiRuntimeInvocationResult,
    error_out: *mut FfiOwnedBuffer,
) -> i32 {
    clear_error_out(error_out);
    clear_out_ptr(result_out);
    if result_out.is_null() {
        return ffi_error_status(error_out, "result_out must not be null");
    }
    let tool_name = match parse_required_string(tool_name, "tool_name") {
        Ok(tool_name) => tool_name,
        Err(error) => return ffi_error_status(error_out, error),
    };
    let args = match parse_json_value_or_empty_object_buffer(&args_json, "args_json") {
        Ok(args) => args,
        Err(error) => return ffi_error_status(error_out, error),
    };
    let invocation_context = match parse_invocation_context(invocation_context) {
        Ok(invocation_context) => invocation_context,
        Err(error) => return ffi_error_status(error_out, error),
    };
    match with_engine(engine_id, |engine| {
        engine.call_skill(&tool_name, &args, invocation_context.as_ref())
    }) {
        Ok(result) => {
            unsafe { *result_out = Box::into_raw(Box::new(alloc_invocation_result(&result))) };
            ffi_ok_status(error_out)
        }
        Err(error) => ffi_error_status(error_out, error),
    }
}

/// Execute arbitrary Lua code through the standard C ABI surface.
/// 通过标准 C ABI 接口执行任意 Lua 代码。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_run_lua(
    engine_id: u64,
    code: *const c_char,
    args_json: FfiBorrowedBuffer,
    invocation_context: *const FfiLuaInvocationContext,
    result_json_out: *mut FfiOwnedBuffer,
    error_out: *mut FfiOwnedBuffer,
) -> i32 {
    clear_error_out(error_out);
    clear_out_buffer(result_json_out);
    if result_json_out.is_null() {
        return ffi_error_status(error_out, "result_json_out must not be null");
    }
    let code = match parse_required_string(code, "code") {
        Ok(code) => code,
        Err(error) => return ffi_error_status(error_out, error),
    };
    let args = match parse_json_value_or_empty_object_buffer(&args_json, "args_json") {
        Ok(args) => args,
        Err(error) => return ffi_error_status(error_out, error),
    };
    let invocation_context = match parse_invocation_context(invocation_context) {
        Ok(invocation_context) => invocation_context,
        Err(error) => return ffi_error_status(error_out, error),
    };
    match with_engine(engine_id, |engine| {
        engine.run_lua(&code, &args, invocation_context.as_ref())
    }) {
        Ok(result) => match serde_json::to_string(&result) {
            Ok(result_json) => {
                unsafe { *result_json_out = alloc_owned_buffer_from_string(result_json) };
                ffi_ok_status(error_out)
            }
            Err(error) => ffi_error_status(
                error_out,
                format!("Failed to serialize Lua result: {}", error),
            ),
        },
        Err(error) => ffi_error_status(error_out, error),
    }
}

/// Disable one skill through legacy directory-style roots via the standard C ABI surface.
/// 通过标准 C ABI 接口按旧目录风格根参数停用单个技能。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_disable_skill_in_dirs(
    engine_id: u64,
    base_dir: *const c_char,
    override_dir: *const c_char,
    skill_id: *const c_char,
    reason: *const c_char,
    error_out: *mut FfiOwnedBuffer,
) -> i32 {
    clear_error_out(error_out);
    let base_dir = match parse_required_string(base_dir, "base_dir") {
        Ok(value) => PathBuf::from(value),
        Err(error) => return ffi_error_status(error_out, error),
    };
    let override_dir = match parse_optional_string(override_dir, "override_dir") {
        Ok(value) => value.map(PathBuf::from),
        Err(error) => return ffi_error_status(error_out, error),
    };
    let skill_id = match parse_required_string(skill_id, "skill_id") {
        Ok(skill_id) => skill_id,
        Err(error) => return ffi_error_status(error_out, error),
    };
    let reason = match parse_optional_string(reason, "reason") {
        Ok(reason) => reason,
        Err(error) => return ffi_error_status(error_out, error),
    };
    match with_engine_mut(engine_id, |engine| {
        engine
            .disable_skill(
                &base_dir,
                override_dir.as_deref(),
                &skill_id,
                reason.as_deref(),
            )
            .map_err(|error| error.to_string())
    }) {
        Ok(()) => ffi_ok_status(error_out),
        Err(error) => ffi_error_status(error_out, error),
    }
}

/// Disable one skill through one ordered root chain via the standard C ABI surface.
/// 通过标准 C ABI 接口按一条有序根链停用单个技能。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_disable_skill(
    engine_id: u64,
    skill_roots: *const FfiRuntimeSkillRoot,
    skill_roots_len: usize,
    skill_id: *const c_char,
    reason: *const c_char,
    error_out: *mut FfiOwnedBuffer,
) -> i32 {
    clear_error_out(error_out);
    let skill_roots = match parse_skill_roots(skill_roots, skill_roots_len) {
        Ok(skill_roots) => skill_roots,
        Err(error) => return ffi_error_status(error_out, error),
    };
    let skill_id = match parse_required_string(skill_id, "skill_id") {
        Ok(skill_id) => skill_id,
        Err(error) => return ffi_error_status(error_out, error),
    };
    let reason = match parse_optional_string(reason, "reason") {
        Ok(reason) => reason,
        Err(error) => return ffi_error_status(error_out, error),
    };
    match with_engine_mut(engine_id, |engine| {
        engine
            .disable_skill_in_roots(&skill_roots, &skill_id, reason.as_deref())
            .map_err(|error| error.to_string())
    }) {
        Ok(()) => ffi_ok_status(error_out),
        Err(error) => ffi_error_status(error_out, error),
    }
}

/// Disable one skill on the system plane through legacy directory-style roots.
/// 通过标准 C ABI 接口按旧目录风格根参数在 system 平面停用单个技能。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_system_disable_skill_in_dirs(
    engine_id: u64,
    base_dir: *const c_char,
    override_dir: *const c_char,
    skill_id: *const c_char,
    reason: *const c_char,
    error_out: *mut FfiOwnedBuffer,
) -> i32 {
    clear_error_out(error_out);
    let base_dir = match parse_required_string(base_dir, "base_dir") {
        Ok(value) => PathBuf::from(value),
        Err(error) => return ffi_error_status(error_out, error),
    };
    let override_dir = match parse_optional_string(override_dir, "override_dir") {
        Ok(value) => value.map(PathBuf::from),
        Err(error) => return ffi_error_status(error_out, error),
    };
    let skill_id = match parse_required_string(skill_id, "skill_id") {
        Ok(skill_id) => skill_id,
        Err(error) => return ffi_error_status(error_out, error),
    };
    let reason = match parse_optional_string(reason, "reason") {
        Ok(reason) => reason,
        Err(error) => return ffi_error_status(error_out, error),
    };
    match with_engine_mut(engine_id, |engine| {
        engine
            .system_disable_skill(
                &base_dir,
                override_dir.as_deref(),
                &skill_id,
                reason.as_deref(),
            )
            .map_err(|error| error.to_string())
    }) {
        Ok(()) => ffi_ok_status(error_out),
        Err(error) => ffi_error_status(error_out, error),
    }
}

/// Disable one skill on the system plane through one ordered root chain.
/// 通过标准 C ABI 接口按一条有序根链在 system 平面停用单个技能。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_system_disable_skill(
    engine_id: u64,
    skill_roots: *const FfiRuntimeSkillRoot,
    skill_roots_len: usize,
    skill_id: *const c_char,
    reason: *const c_char,
    error_out: *mut FfiOwnedBuffer,
) -> i32 {
    clear_error_out(error_out);
    let skill_roots = match parse_skill_roots(skill_roots, skill_roots_len) {
        Ok(skill_roots) => skill_roots,
        Err(error) => return ffi_error_status(error_out, error),
    };
    let skill_id = match parse_required_string(skill_id, "skill_id") {
        Ok(skill_id) => skill_id,
        Err(error) => return ffi_error_status(error_out, error),
    };
    let reason = match parse_optional_string(reason, "reason") {
        Ok(reason) => reason,
        Err(error) => return ffi_error_status(error_out, error),
    };
    match with_engine_mut(engine_id, |engine| {
        engine
            .system_disable_skill_in_roots(&skill_roots, &skill_id, reason.as_deref())
            .map_err(|error| error.to_string())
    }) {
        Ok(()) => ffi_ok_status(error_out),
        Err(error) => ffi_error_status(error_out, error),
    }
}

/// Enable one skill through one ordered root chain via the standard C ABI surface.
/// 通过标准 C ABI 接口按一条有序根链启用单个技能。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_enable_skill(
    engine_id: u64,
    skill_roots: *const FfiRuntimeSkillRoot,
    skill_roots_len: usize,
    skill_id: *const c_char,
    error_out: *mut FfiOwnedBuffer,
) -> i32 {
    clear_error_out(error_out);
    let skill_roots = match parse_skill_roots(skill_roots, skill_roots_len) {
        Ok(skill_roots) => skill_roots,
        Err(error) => return ffi_error_status(error_out, error),
    };
    let skill_id = match parse_required_string(skill_id, "skill_id") {
        Ok(skill_id) => skill_id,
        Err(error) => return ffi_error_status(error_out, error),
    };
    match with_engine_mut(engine_id, |engine| {
        engine
            .enable_skill(&skill_roots, &skill_id)
            .map_err(|error| error.to_string())
    }) {
        Ok(()) => ffi_ok_status(error_out),
        Err(error) => ffi_error_status(error_out, error),
    }
}

/// Enable one skill on the system plane through one ordered root chain.
/// 通过标准 C ABI 接口按一条有序根链在 system 平面启用单个技能。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_system_enable_skill(
    engine_id: u64,
    skill_roots: *const FfiRuntimeSkillRoot,
    skill_roots_len: usize,
    skill_id: *const c_char,
    error_out: *mut FfiOwnedBuffer,
) -> i32 {
    clear_error_out(error_out);
    let skill_roots = match parse_skill_roots(skill_roots, skill_roots_len) {
        Ok(skill_roots) => skill_roots,
        Err(error) => return ffi_error_status(error_out, error),
    };
    let skill_id = match parse_required_string(skill_id, "skill_id") {
        Ok(skill_id) => skill_id,
        Err(error) => return ffi_error_status(error_out, error),
    };
    match with_engine_mut(engine_id, |engine| {
        engine
            .system_enable_skill(&skill_roots, &skill_id)
            .map_err(|error| error.to_string())
    }) {
        Ok(()) => ffi_ok_status(error_out),
        Err(error) => ffi_error_status(error_out, error),
    }
}

/// Uninstall one skill through one ordered root chain via the standard C ABI surface.
/// 通过标准 C ABI 接口按一条有序根链卸载单个技能。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_uninstall_skill(
    engine_id: u64,
    skill_roots: *const FfiRuntimeSkillRoot,
    skill_roots_len: usize,
    skill_id: *const c_char,
    options: *const FfiSkillUninstallOptions,
    result_out: *mut *mut FfiSkillUninstallResult,
    error_out: *mut FfiOwnedBuffer,
) -> i32 {
    clear_error_out(error_out);
    clear_out_ptr(result_out);
    if result_out.is_null() {
        return ffi_error_status(error_out, "result_out must not be null");
    }
    let skill_roots = match parse_skill_roots(skill_roots, skill_roots_len) {
        Ok(skill_roots) => skill_roots,
        Err(error) => return ffi_error_status(error_out, error),
    };
    let skill_id = match parse_required_string(skill_id, "skill_id") {
        Ok(skill_id) => skill_id,
        Err(error) => return ffi_error_status(error_out, error),
    };
    let options = parse_uninstall_options(unsafe { options.as_ref() });
    match with_engine_mut(engine_id, |engine| {
        engine
            .uninstall_skill(&skill_roots, &skill_id, &options)
            .map_err(|error| error.to_string())
    }) {
        Ok(result) => {
            unsafe { *result_out = Box::into_raw(Box::new(alloc_skill_uninstall_result(&result))) };
            ffi_ok_status(error_out)
        }
        Err(error) => ffi_error_status(error_out, error),
    }
}

/// Uninstall one skill on the system plane through one ordered root chain.
/// 通过标准 C ABI 接口按一条有序根链在 system 平面卸载单个技能。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_system_uninstall_skill(
    engine_id: u64,
    skill_roots: *const FfiRuntimeSkillRoot,
    skill_roots_len: usize,
    skill_id: *const c_char,
    options: *const FfiSkillUninstallOptions,
    result_out: *mut *mut FfiSkillUninstallResult,
    error_out: *mut FfiOwnedBuffer,
) -> i32 {
    clear_error_out(error_out);
    clear_out_ptr(result_out);
    if result_out.is_null() {
        return ffi_error_status(error_out, "result_out must not be null");
    }
    let skill_roots = match parse_skill_roots(skill_roots, skill_roots_len) {
        Ok(skill_roots) => skill_roots,
        Err(error) => return ffi_error_status(error_out, error),
    };
    let skill_id = match parse_required_string(skill_id, "skill_id") {
        Ok(skill_id) => skill_id,
        Err(error) => return ffi_error_status(error_out, error),
    };
    let options = parse_uninstall_options(unsafe { options.as_ref() });
    match with_engine_mut(engine_id, |engine| {
        engine
            .system_uninstall_skill(&skill_roots, &skill_id, &options)
            .map_err(|error| error.to_string())
    }) {
        Ok(result) => {
            unsafe { *result_out = Box::into_raw(Box::new(alloc_skill_uninstall_result(&result))) };
            ffi_ok_status(error_out)
        }
        Err(error) => ffi_error_status(error_out, error),
    }
}

/// Install one managed skill through one ordered root chain via the standard C ABI surface.
/// 通过标准 C ABI 接口按一条有序根链安装单个受管技能。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_install_skill(
    engine_id: u64,
    skill_roots: *const FfiRuntimeSkillRoot,
    skill_roots_len: usize,
    request: *const FfiSkillInstallRequest,
    result_out: *mut *mut FfiSkillApplyResult,
    error_out: *mut FfiOwnedBuffer,
) -> i32 {
    clear_error_out(error_out);
    clear_out_ptr(result_out);
    if result_out.is_null() {
        return ffi_error_status(error_out, "result_out must not be null");
    }
    if request.is_null() {
        return ffi_error_status(error_out, "request must not be null");
    }
    let skill_roots = match parse_skill_roots(skill_roots, skill_roots_len) {
        Ok(skill_roots) => skill_roots,
        Err(error) => return ffi_error_status(error_out, error),
    };
    let request = match parse_install_request(unsafe { &*request }) {
        Ok(request) => request,
        Err(error) => return ffi_error_status(error_out, error),
    };
    match with_engine_mut(engine_id, |engine| {
        engine
            .install_skill(&skill_roots, &request)
            .map_err(|error| error.to_string())
    }) {
        Ok(result) => {
            unsafe { *result_out = Box::into_raw(Box::new(alloc_skill_apply_result(&result))) };
            ffi_ok_status(error_out)
        }
        Err(error) => ffi_error_status(error_out, error),
    }
}

/// Install one managed skill on the system plane through one ordered root chain.
/// 通过标准 C ABI 接口按一条有序根链在 system 平面安装单个受管技能。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_system_install_skill(
    engine_id: u64,
    skill_roots: *const FfiRuntimeSkillRoot,
    skill_roots_len: usize,
    request: *const FfiSkillInstallRequest,
    result_out: *mut *mut FfiSkillApplyResult,
    error_out: *mut FfiOwnedBuffer,
) -> i32 {
    clear_error_out(error_out);
    clear_out_ptr(result_out);
    if result_out.is_null() {
        return ffi_error_status(error_out, "result_out must not be null");
    }
    if request.is_null() {
        return ffi_error_status(error_out, "request must not be null");
    }
    let skill_roots = match parse_skill_roots(skill_roots, skill_roots_len) {
        Ok(skill_roots) => skill_roots,
        Err(error) => return ffi_error_status(error_out, error),
    };
    let request = match parse_install_request(unsafe { &*request }) {
        Ok(request) => request,
        Err(error) => return ffi_error_status(error_out, error),
    };
    match with_engine_mut(engine_id, |engine| {
        engine
            .system_install_skill(&skill_roots, &request)
            .map_err(|error| error.to_string())
    }) {
        Ok(result) => {
            unsafe { *result_out = Box::into_raw(Box::new(alloc_skill_apply_result(&result))) };
            ffi_ok_status(error_out)
        }
        Err(error) => ffi_error_status(error_out, error),
    }
}

/// Update one managed skill through one ordered root chain via the standard C ABI surface.
/// 通过标准 C ABI 接口按一条有序根链更新单个受管技能。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_update_skill(
    engine_id: u64,
    skill_roots: *const FfiRuntimeSkillRoot,
    skill_roots_len: usize,
    request: *const FfiSkillInstallRequest,
    result_out: *mut *mut FfiSkillApplyResult,
    error_out: *mut FfiOwnedBuffer,
) -> i32 {
    clear_error_out(error_out);
    clear_out_ptr(result_out);
    if result_out.is_null() {
        return ffi_error_status(error_out, "result_out must not be null");
    }
    if request.is_null() {
        return ffi_error_status(error_out, "request must not be null");
    }
    let skill_roots = match parse_skill_roots(skill_roots, skill_roots_len) {
        Ok(skill_roots) => skill_roots,
        Err(error) => return ffi_error_status(error_out, error),
    };
    let request = match parse_install_request(unsafe { &*request }) {
        Ok(request) => request,
        Err(error) => return ffi_error_status(error_out, error),
    };
    match with_engine_mut(engine_id, |engine| {
        engine
            .update_skill(&skill_roots, &request)
            .map_err(|error| error.to_string())
    }) {
        Ok(result) => {
            unsafe { *result_out = Box::into_raw(Box::new(alloc_skill_apply_result(&result))) };
            ffi_ok_status(error_out)
        }
        Err(error) => ffi_error_status(error_out, error),
    }
}

/// Update one managed skill on the system plane through one ordered root chain.
/// 通过标准 C ABI 接口按一条有序根链在 system 平面更新单个受管技能。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vulcan_luaskills_ffi_system_update_skill(
    engine_id: u64,
    skill_roots: *const FfiRuntimeSkillRoot,
    skill_roots_len: usize,
    request: *const FfiSkillInstallRequest,
    result_out: *mut *mut FfiSkillApplyResult,
    error_out: *mut FfiOwnedBuffer,
) -> i32 {
    clear_error_out(error_out);
    clear_out_ptr(result_out);
    if result_out.is_null() {
        return ffi_error_status(error_out, "result_out must not be null");
    }
    if request.is_null() {
        return ffi_error_status(error_out, "request must not be null");
    }
    let skill_roots = match parse_skill_roots(skill_roots, skill_roots_len) {
        Ok(skill_roots) => skill_roots,
        Err(error) => return ffi_error_status(error_out, error),
    };
    let request = match parse_install_request(unsafe { &*request }) {
        Ok(request) => request,
        Err(error) => return ffi_error_status(error_out, error),
    };
    match with_engine_mut(engine_id, |engine| {
        engine
            .system_update_skill(&skill_roots, &request)
            .map_err(|error| error.to_string())
    }) {
        Ok(result) => {
            unsafe { *result_out = Box::into_raw(Box::new(alloc_skill_apply_result(&result))) };
            ffi_ok_status(error_out)
        }
        Err(error) => ffi_error_status(error_out, error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime_help::{
        RuntimeHelpDetail as RuntimeHelpDetailModel,
        RuntimeHelpNodeDescriptor as RuntimeHelpNodeDescriptorModel,
        RuntimeSkillHelpDescriptor as RuntimeSkillHelpDescriptorModel,
    };
    use crate::{
        RuntimeEntryDescriptor as RuntimeEntryDescriptorModel,
        RuntimeEntryParameterDescriptor as RuntimeEntryParameterDescriptorModel,
    };

    /// Read one owned UTF-8 buffer into one Rust string without freeing it.
    /// 将一个拥有型 UTF-8 缓冲读取为 Rust 字符串但不执行释放。
    fn read_owned_buffer_text(buffer: &FfiOwnedBuffer) -> String {
        if buffer.ptr.is_null() || buffer.len == 0 {
            return String::new();
        }
        let bytes = unsafe { std::slice::from_raw_parts(buffer.ptr, buffer.len) };
        String::from_utf8(bytes.to_vec()).expect("buffer text must be utf-8")
    }

    /// Build one borrowed buffer view over one UTF-8 text while keeping backing storage alive.
    /// 在保持底层存储存活的前提下，为一段 UTF-8 文本构造借用缓冲视图。
    fn make_borrowed_buffer(text: &str) -> (Vec<u8>, FfiBorrowedBuffer) {
        let bytes = text.as_bytes().to_vec();
        let buffer = if bytes.is_empty() {
            FfiBorrowedBuffer {
                ptr: ptr::null(),
                len: 0,
            }
        } else {
            FfiBorrowedBuffer {
                ptr: bytes.as_ptr(),
                len: bytes.len(),
            }
        };
        (bytes, buffer)
    }

    /// Verify buffer_clone copies one byte payload into luaskills-owned storage.
    /// 验证 buffer_clone 会把单个字节载荷复制到 luaskills 自主管理存储中。
    #[test]
    fn buffer_clone_copies_payload_into_owned_storage() {
        let input = b"ffi-buffer-demo";
        let mut buffer_out = FfiOwnedBuffer {
            ptr: ptr::null_mut(),
            len: 0,
        };
        let mut error_out = FfiOwnedBuffer {
            ptr: ptr::null_mut(),
            len: 0,
        };
        let status = unsafe {
            vulcan_luaskills_ffi_buffer_clone(
                input.as_ptr(),
                input.len(),
                &mut buffer_out,
                &mut error_out,
            )
        };
        assert_eq!(status, FFI_STATUS_OK);
        assert!(error_out.ptr.is_null());
        assert_eq!(error_out.len, 0);
        let copied = unsafe { std::slice::from_raw_parts(buffer_out.ptr, buffer_out.len) };
        assert_eq!(copied, input);
        unsafe { vulcan_luaskills_ffi_buffer_free(buffer_out) };
    }

    /// Verify JSON provider callback bridge accepts borrowed buffers and owned-buffer responses.
    /// 验证 JSON provider callback 桥接可接受借用缓冲输入并处理拥有型缓冲输出。
    #[test]
    fn json_provider_callback_bridge_round_trips_owned_buffers() {
        unsafe extern "C" fn callback(
            request_json: FfiBorrowedBuffer,
            _user_data: *mut c_void,
            response_out: *mut FfiOwnedBuffer,
            error_out: *mut FfiOwnedBuffer,
        ) -> i32 {
            let request_bytes =
                unsafe { std::slice::from_raw_parts(request_json.ptr, request_json.len) };
            let request_text = std::str::from_utf8(request_bytes).expect("request must be utf-8");
            let response_text = format!("{{\"echo\":{}}}", request_text);
            unsafe {
                *response_out = alloc_owned_buffer_from_bytes(response_text.as_bytes());
                *error_out = FfiOwnedBuffer {
                    ptr: ptr::null_mut(),
                    len: 0,
                };
            }
            FFI_STATUS_OK
        }

        let response = invoke_json_provider_callback(callback, 0, "{\"value\":1}")
            .expect("callback bridge should succeed");
        assert_eq!(response, "{\"echo\":{\"value\":1}}");
    }

    /// Verify one entry list allocates nested owned buffers for entry and parameter text fields.
    /// 验证入口列表会为入口及参数文本字段分配嵌套拥有型缓冲。
    #[test]
    fn entry_list_free_handles_nested_owned_buffers() {
        let runtime_entry = RuntimeEntryDescriptorModel {
            canonical_name: "demo-entry".to_string(),
            skill_id: "demo-skill".to_string(),
            local_name: "entry".to_string(),
            root_name: "ROOT".to_string(),
            skill_dir: "/tmp/demo-skill".to_string(),
            description: "Demo entry description".to_string(),
            parameters: vec![RuntimeEntryParameterDescriptorModel {
                name: "note".to_string(),
                param_type: "string".to_string(),
                description: "Optional note".to_string(),
                required: false,
            }],
        };

        let mut items = vec![alloc_entry_descriptor(&runtime_entry)];
        let list = FfiRuntimeEntryDescriptorList {
            items: items.as_mut_ptr(),
            len: items.len(),
        };
        std::mem::forget(items);
        let list_ptr = Box::into_raw(Box::new(list));

        let list_ref = unsafe { &*list_ptr };
        assert_eq!(list_ref.len, 1);
        let first_entry = unsafe { &*list_ref.items };
        assert_eq!(
            read_owned_buffer_text(&first_entry.canonical_name),
            "demo-entry"
        );
        assert_eq!(read_owned_buffer_text(&first_entry.skill_id), "demo-skill");
        assert_eq!(
            read_owned_buffer_text(&first_entry.description),
            "Demo entry description"
        );
        assert_eq!(first_entry.parameters_len, 1);

        let first_parameter = unsafe { &*first_entry.parameters };
        assert_eq!(read_owned_buffer_text(&first_parameter.name), "note");
        assert_eq!(
            read_owned_buffer_text(&first_parameter.param_type),
            "string"
        );
        assert_eq!(
            read_owned_buffer_text(&first_parameter.description),
            "Optional note"
        );
        assert_eq!(first_parameter.required, 0);

        unsafe { vulcan_luaskills_ffi_entry_list_free(list_ptr) };
    }

    /// Verify one help detail and one help list allocate nested owned buffers for text and related-entry arrays.
    /// 验证帮助详情与帮助列表会为文本字段和关联入口数组分配嵌套拥有型缓冲。
    #[test]
    fn help_results_free_handle_nested_owned_buffers() {
        let help_detail = RuntimeHelpDetailModel {
            skill_id: "demo-skill".to_string(),
            skill_name: "Demo Skill".to_string(),
            skill_version: "0.1.0".to_string(),
            root_name: "ROOT".to_string(),
            skill_dir: "/tmp/demo-skill".to_string(),
            flow_name: "main".to_string(),
            description: "Demo help detail".to_string(),
            related_entries: vec!["demo-entry".to_string(), "demo-entry-2".to_string()],
            is_main: true,
            content_type: "markdown".to_string(),
            content: "# Demo".to_string(),
        };
        let detail_ptr = Box::into_raw(Box::new(alloc_help_detail(&help_detail)));

        let detail_ref = unsafe { &*detail_ptr };
        assert_eq!(read_owned_buffer_text(&detail_ref.skill_id), "demo-skill");
        assert_eq!(read_owned_buffer_text(&detail_ref.flow_name), "main");
        assert_eq!(detail_ref.related_entries_len, 2);
        let related_entries = unsafe {
            std::slice::from_raw_parts(detail_ref.related_entries, detail_ref.related_entries_len)
        };
        assert_eq!(read_owned_buffer_text(&related_entries[0]), "demo-entry");
        assert_eq!(read_owned_buffer_text(&related_entries[1]), "demo-entry-2");

        unsafe { vulcan_luaskills_ffi_help_detail_free(detail_ptr) };

        let help_descriptor = RuntimeSkillHelpDescriptorModel {
            skill_id: "demo-skill".to_string(),
            skill_name: "Demo Skill".to_string(),
            skill_version: "0.1.0".to_string(),
            root_name: "ROOT".to_string(),
            skill_dir: "/tmp/demo-skill".to_string(),
            main: RuntimeHelpNodeDescriptorModel {
                flow_name: "main".to_string(),
                description: "Main help node".to_string(),
                related_entries: vec!["demo-entry".to_string()],
                is_main: true,
            },
            flows: vec![RuntimeHelpNodeDescriptorModel {
                flow_name: "secondary".to_string(),
                description: "Secondary node".to_string(),
                related_entries: vec!["demo-entry-2".to_string()],
                is_main: false,
            }],
        };

        let mut items = vec![alloc_help_descriptor(&help_descriptor)];
        let list = FfiRuntimeSkillHelpDescriptorList {
            items: items.as_mut_ptr(),
            len: items.len(),
        };
        std::mem::forget(items);
        let list_ptr = Box::into_raw(Box::new(list));

        let list_ref = unsafe { &*list_ptr };
        assert_eq!(list_ref.len, 1);
        let first_help = unsafe { &*list_ref.items };
        assert_eq!(read_owned_buffer_text(&first_help.skill_name), "Demo Skill");
        assert_eq!(read_owned_buffer_text(&first_help.main.flow_name), "main");
        assert_eq!(first_help.main.related_entries_len, 1);
        let main_related_entries = unsafe {
            std::slice::from_raw_parts(
                first_help.main.related_entries,
                first_help.main.related_entries_len,
            )
        };
        assert_eq!(
            read_owned_buffer_text(&main_related_entries[0]),
            "demo-entry"
        );
        assert_eq!(first_help.flows_len, 1);
        let first_flow = unsafe { &*first_help.flows };
        assert_eq!(read_owned_buffer_text(&first_flow.flow_name), "secondary");

        unsafe { vulcan_luaskills_ffi_help_list_free(list_ptr) };
    }

    /// Verify the standard FFI load/list pipeline returns one entry for one minimal temporary skill root.
    /// 验证标准 FFI 的加载与列举链路会为最小临时技能根返回一个入口。
    #[test]
    fn standard_ffi_load_and_list_entries_round_trip() {
        let temp_root = std::env::temp_dir().join(format!(
            "vulcan_luaskills_standard_ffi_entry_test_{}",
            std::process::id()
        ));
        if temp_root.exists() {
            let _ = std::fs::remove_dir_all(&temp_root);
        }

        let skills_root = temp_root.join("skills");
        let skill_dir = skills_root.join("demo-skill");
        std::fs::create_dir_all(skill_dir.join("runtime")).expect("create runtime directory");
        std::fs::create_dir_all(temp_root.join("temp")).expect("create temp directory");
        std::fs::create_dir_all(temp_root.join("resources")).expect("create resources directory");
        std::fs::create_dir_all(temp_root.join("lua_packages"))
            .expect("create lua_packages directory");
        std::fs::create_dir_all(temp_root.join("bin").join("tools"))
            .expect("create tools directory");
        std::fs::create_dir_all(temp_root.join("libs")).expect("create libs directory");
        std::fs::write(
            skill_dir.join("skill.yaml"),
            "name: demo-skill\nversion: 0.1.0\nenable: true\nentries:\n  - name: ping\n    description: Ping entry.\n    lua_entry: runtime/ping.lua\n    lua_module: demo_skill_ping\n    parameters:\n      - name: note\n        type: string\n        description: Optional note.\n        required: false\n",
        )
        .expect("write skill yaml");
        std::fs::write(
            skill_dir.join("runtime").join("ping.lua"),
            "return function(args)\n  return 'ok'\nend\n",
        )
        .expect("write runtime lua");

        let temp_dir_text =
            CString::new(temp_root.join("temp").display().to_string()).expect("temp_dir cstring");
        let resources_dir_text = CString::new(temp_root.join("resources").display().to_string())
            .expect("resources_dir cstring");
        let lua_packages_dir_text =
            CString::new(temp_root.join("lua_packages").display().to_string())
                .expect("lua_packages_dir cstring");
        let tool_root_dir_text =
            CString::new(temp_root.join("bin").join("tools").display().to_string())
                .expect("tool_root_dir cstring");
        let ffi_root_dir_text =
            CString::new(temp_root.join("libs").display().to_string()).expect("ffi_root cstring");
        let dependency_dir_name = CString::new("dependencies").expect("dependencies cstring");
        let state_dir_name = CString::new("state").expect("state cstring");
        let database_dir_name = CString::new("databases").expect("databases cstring");
        let root_name = CString::new("ROOT").expect("root name cstring");
        let skills_root_text =
            CString::new(skills_root.display().to_string()).expect("skills root cstring");

        let host_options = FfiLuaRuntimeHostOptions {
            temp_dir: temp_dir_text.as_ptr(),
            resources_dir: resources_dir_text.as_ptr(),
            lua_packages_dir: lua_packages_dir_text.as_ptr(),
            luaexec_program: ptr::null(),
            host_provided_tool_root: tool_root_dir_text.as_ptr(),
            host_provided_lua_root: lua_packages_dir_text.as_ptr(),
            host_provided_ffi_root: ffi_root_dir_text.as_ptr(),
            download_cache_root: ptr::null(),
            dependency_dir_name: dependency_dir_name.as_ptr(),
            state_dir_name: state_dir_name.as_ptr(),
            database_dir_name: database_dir_name.as_ptr(),
            protected_skill_ids: ptr::null(),
            protected_skill_ids_len: 0,
            allow_network_download: 0,
            github_base_url: ptr::null(),
            github_api_base_url: ptr::null(),
            sqlite_library_path: ptr::null(),
            sqlite_provider_mode: FFI_PROVIDER_MODE_DYNAMIC_LIBRARY,
            sqlite_callback_mode: FFI_CALLBACK_MODE_STANDARD,
            lancedb_library_path: ptr::null(),
            lancedb_provider_mode: FFI_PROVIDER_MODE_DYNAMIC_LIBRARY,
            lancedb_callback_mode: FFI_CALLBACK_MODE_STANDARD,
            space_controller_endpoint: ptr::null(),
            space_controller_auto_spawn: 0,
            space_controller_executable_path: ptr::null(),
            space_controller_process_mode: FFI_SPACE_CONTROLLER_PROCESS_MODE_SERVICE,
            cache_config: ptr::null(),
            reserved_entry_names: ptr::null(),
            reserved_entry_names_len: 0,
            ignored_skill_ids: ptr::null(),
            ignored_skill_ids_len: 0,
            enable_skill_management_bridge: 0,
        };
        let engine_options = FfiLuaEngineOptions {
            pool: FfiLuaVmPoolConfig {
                min_size: 1,
                max_size: 1,
                idle_ttl_secs: 30,
            },
            host: host_options,
        };

        let mut engine_id = 0_u64;
        let mut error_out = FfiOwnedBuffer {
            ptr: ptr::null_mut(),
            len: 0,
        };
        let engine_status = unsafe {
            vulcan_luaskills_ffi_engine_new(&engine_options, &mut engine_id, &mut error_out)
        };
        assert_eq!(engine_status, FFI_STATUS_OK);
        assert!(error_out.ptr.is_null());

        let ffi_skill_roots = [FfiRuntimeSkillRoot {
            name: root_name.as_ptr(),
            skills_dir: skills_root_text.as_ptr(),
        }];
        let mut load_error = FfiOwnedBuffer {
            ptr: ptr::null_mut(),
            len: 0,
        };
        let load_status = unsafe {
            vulcan_luaskills_ffi_load_from_roots(
                engine_id,
                ffi_skill_roots.as_ptr(),
                ffi_skill_roots.len(),
                &mut load_error,
            )
        };
        assert_eq!(load_status, FFI_STATUS_OK);
        assert!(load_error.ptr.is_null());

        let mut entries_out: *mut FfiRuntimeEntryDescriptorList = ptr::null_mut();
        let mut list_error = FfiOwnedBuffer {
            ptr: ptr::null_mut(),
            len: 0,
        };
        let list_status = unsafe {
            vulcan_luaskills_ffi_list_entries(engine_id, &mut entries_out, &mut list_error)
        };
        assert_eq!(list_status, FFI_STATUS_OK);
        assert!(list_error.ptr.is_null());
        assert!(!entries_out.is_null());

        let entries_ref = unsafe { &*entries_out };
        assert_eq!(entries_ref.len, 1);
        let entry_ref = unsafe { &*entries_ref.items };
        assert_eq!(
            read_owned_buffer_text(&entry_ref.canonical_name),
            "demo-skill-ping"
        );
        assert_eq!(read_owned_buffer_text(&entry_ref.skill_id), "demo-skill");
        assert_eq!(read_owned_buffer_text(&entry_ref.local_name), "ping");
        assert_eq!(read_owned_buffer_text(&entry_ref.root_name), "ROOT");
        assert_eq!(
            read_owned_buffer_text(&entry_ref.description),
            "Ping entry."
        );
        assert_eq!(entry_ref.parameters_len, 1);
        let parameter_ref = unsafe { &*entry_ref.parameters };
        assert_eq!(read_owned_buffer_text(&parameter_ref.name), "note");
        assert_eq!(read_owned_buffer_text(&parameter_ref.param_type), "string");
        assert_eq!(
            read_owned_buffer_text(&parameter_ref.description),
            "Optional note."
        );
        assert_eq!(parameter_ref.required, 0);

        unsafe { vulcan_luaskills_ffi_entry_list_free(entries_out) };

        let mut free_error = FfiOwnedBuffer {
            ptr: ptr::null_mut(),
            len: 0,
        };
        let free_status = unsafe { vulcan_luaskills_ffi_engine_free(engine_id, &mut free_error) };
        assert_eq!(free_status, FFI_STATUS_OK);
        assert!(free_error.ptr.is_null());

        let _ = std::fs::remove_dir_all(&temp_root);
    }

    /// Verify standard call_skill accepts borrowed JSON buffers for args and invocation context.
    /// 验证标准 call_skill 会接受作为 args 与调用上下文输入的借用 JSON 缓冲。
    #[test]
    fn standard_ffi_call_skill_accepts_borrowed_json_buffers() {
        let temp_root = std::env::temp_dir().join(format!(
            "vulcan_luaskills_standard_ffi_callskill_test_{}",
            std::process::id()
        ));
        if temp_root.exists() {
            let _ = std::fs::remove_dir_all(&temp_root);
        }

        let skills_root = temp_root.join("skills");
        let skill_dir = skills_root.join("demo-skill");
        std::fs::create_dir_all(skill_dir.join("runtime")).expect("create runtime directory");
        std::fs::create_dir_all(temp_root.join("temp")).expect("create temp directory");
        std::fs::create_dir_all(temp_root.join("resources")).expect("create resources directory");
        std::fs::create_dir_all(temp_root.join("lua_packages"))
            .expect("create lua_packages directory");
        std::fs::create_dir_all(temp_root.join("bin").join("tools"))
            .expect("create tools directory");
        std::fs::create_dir_all(temp_root.join("libs")).expect("create libs directory");
        std::fs::write(
            skill_dir.join("skill.yaml"),
            "name: demo-skill\nversion: 0.1.0\nenable: true\nentries:\n  - name: ping\n    description: Ping entry.\n    lua_entry: runtime/ping.lua\n    lua_module: demo_skill_ping\n    parameters:\n      - name: note\n        type: string\n        description: Optional note.\n        required: false\n",
        )
        .expect("write skill yaml");
        std::fs::write(
            skill_dir.join("runtime").join("ping.lua"),
            "return function(args)\n  local note = ''\n  if type(args) == 'table' and type(args.note) == 'string' then\n    note = args.note\n  end\n  if note ~= '' then\n    return 'standard-ffi-test:' .. note\n  end\n  return 'standard-ffi-test:ok'\nend\n",
        )
        .expect("write runtime lua");

        let temp_dir_text =
            CString::new(temp_root.join("temp").display().to_string()).expect("temp_dir cstring");
        let resources_dir_text = CString::new(temp_root.join("resources").display().to_string())
            .expect("resources_dir cstring");
        let lua_packages_dir_text =
            CString::new(temp_root.join("lua_packages").display().to_string())
                .expect("lua_packages_dir cstring");
        let tool_root_dir_text =
            CString::new(temp_root.join("bin").join("tools").display().to_string())
                .expect("tool_root_dir cstring");
        let ffi_root_dir_text =
            CString::new(temp_root.join("libs").display().to_string()).expect("ffi_root cstring");
        let dependency_dir_name = CString::new("dependencies").expect("dependencies cstring");
        let state_dir_name = CString::new("state").expect("state cstring");
        let database_dir_name = CString::new("databases").expect("databases cstring");
        let root_name = CString::new("ROOT").expect("root name cstring");
        let skills_root_text =
            CString::new(skills_root.display().to_string()).expect("skills root cstring");
        let tool_name = CString::new("demo-skill-ping").expect("tool name cstring");

        let host_options = FfiLuaRuntimeHostOptions {
            temp_dir: temp_dir_text.as_ptr(),
            resources_dir: resources_dir_text.as_ptr(),
            lua_packages_dir: lua_packages_dir_text.as_ptr(),
            luaexec_program: ptr::null(),
            host_provided_tool_root: tool_root_dir_text.as_ptr(),
            host_provided_lua_root: lua_packages_dir_text.as_ptr(),
            host_provided_ffi_root: ffi_root_dir_text.as_ptr(),
            download_cache_root: ptr::null(),
            dependency_dir_name: dependency_dir_name.as_ptr(),
            state_dir_name: state_dir_name.as_ptr(),
            database_dir_name: database_dir_name.as_ptr(),
            protected_skill_ids: ptr::null(),
            protected_skill_ids_len: 0,
            allow_network_download: 0,
            github_base_url: ptr::null(),
            github_api_base_url: ptr::null(),
            sqlite_library_path: ptr::null(),
            sqlite_provider_mode: FFI_PROVIDER_MODE_DYNAMIC_LIBRARY,
            sqlite_callback_mode: FFI_CALLBACK_MODE_STANDARD,
            lancedb_library_path: ptr::null(),
            lancedb_provider_mode: FFI_PROVIDER_MODE_DYNAMIC_LIBRARY,
            lancedb_callback_mode: FFI_CALLBACK_MODE_STANDARD,
            space_controller_endpoint: ptr::null(),
            space_controller_auto_spawn: 0,
            space_controller_executable_path: ptr::null(),
            space_controller_process_mode: FFI_SPACE_CONTROLLER_PROCESS_MODE_SERVICE,
            cache_config: ptr::null(),
            reserved_entry_names: ptr::null(),
            reserved_entry_names_len: 0,
            ignored_skill_ids: ptr::null(),
            ignored_skill_ids_len: 0,
            enable_skill_management_bridge: 0,
        };
        let engine_options = FfiLuaEngineOptions {
            pool: FfiLuaVmPoolConfig {
                min_size: 1,
                max_size: 1,
                idle_ttl_secs: 30,
            },
            host: host_options,
        };

        let mut engine_id = 0_u64;
        let mut error_out = FfiOwnedBuffer {
            ptr: ptr::null_mut(),
            len: 0,
        };
        let engine_status = unsafe {
            vulcan_luaskills_ffi_engine_new(&engine_options, &mut engine_id, &mut error_out)
        };
        assert_eq!(engine_status, FFI_STATUS_OK);
        assert!(error_out.ptr.is_null());

        let ffi_skill_roots = [FfiRuntimeSkillRoot {
            name: root_name.as_ptr(),
            skills_dir: skills_root_text.as_ptr(),
        }];
        let mut load_error = FfiOwnedBuffer {
            ptr: ptr::null_mut(),
            len: 0,
        };
        let load_status = unsafe {
            vulcan_luaskills_ffi_load_from_roots(
                engine_id,
                ffi_skill_roots.as_ptr(),
                ffi_skill_roots.len(),
                &mut load_error,
            )
        };
        assert_eq!(load_status, FFI_STATUS_OK);
        assert!(load_error.ptr.is_null());

        let (_args_storage, args_buffer) = make_borrowed_buffer(r#"{"note":"ffi"}"#);
        let (_request_storage, request_buffer) =
            make_borrowed_buffer(r#"{"transport_name":"ffi-test"}"#);
        let (_budget_storage, budget_buffer) = make_borrowed_buffer(r#"{"budget":7}"#);
        let (_tool_storage, tool_buffer) = make_borrowed_buffer(r#"{"mode":"demo-mode"}"#);
        let invocation_context = FfiLuaInvocationContext {
            request_context_json: request_buffer,
            client_budget_json: budget_buffer,
            tool_config_json: tool_buffer,
        };

        let mut result_out: *mut FfiRuntimeInvocationResult = ptr::null_mut();
        let mut call_error = FfiOwnedBuffer {
            ptr: ptr::null_mut(),
            len: 0,
        };
        let call_status = unsafe {
            vulcan_luaskills_ffi_call_skill(
                engine_id,
                tool_name.as_ptr(),
                args_buffer,
                &invocation_context,
                &mut result_out,
                &mut call_error,
            )
        };
        assert_eq!(call_status, FFI_STATUS_OK);
        assert!(call_error.ptr.is_null());
        assert!(!result_out.is_null());

        let result_ref = unsafe { &*result_out };
        assert_eq!(
            read_owned_buffer_text(&result_ref.content),
            "standard-ffi-test:ffi"
        );
        assert_eq!(result_ref.content_lines, 1);
        unsafe { vulcan_luaskills_ffi_invocation_result_free(result_out) };

        let mut free_error = FfiOwnedBuffer {
            ptr: ptr::null_mut(),
            len: 0,
        };
        let free_status = unsafe { vulcan_luaskills_ffi_engine_free(engine_id, &mut free_error) };
        assert_eq!(free_status, FFI_STATUS_OK);
        assert!(free_error.ptr.is_null());

        let _ = std::fs::remove_dir_all(&temp_root);
    }

    /// Verify standard run_lua accepts borrowed JSON buffers for args and invocation context.
    /// 验证标准 run_lua 会接受作为 args 与调用上下文输入的借用 JSON 缓冲。
    #[test]
    fn standard_ffi_run_lua_accepts_borrowed_json_buffers() {
        let temp_root = std::env::temp_dir().join(format!(
            "vulcan_luaskills_standard_ffi_runlua_test_{}",
            std::process::id()
        ));
        if temp_root.exists() {
            let _ = std::fs::remove_dir_all(&temp_root);
        }

        std::fs::create_dir_all(temp_root.join("temp")).expect("create temp directory");
        std::fs::create_dir_all(temp_root.join("resources")).expect("create resources directory");
        std::fs::create_dir_all(temp_root.join("lua_packages"))
            .expect("create lua_packages directory");
        std::fs::create_dir_all(temp_root.join("bin").join("tools"))
            .expect("create tools directory");
        std::fs::create_dir_all(temp_root.join("libs")).expect("create libs directory");

        let temp_dir_text =
            CString::new(temp_root.join("temp").display().to_string()).expect("temp_dir cstring");
        let resources_dir_text = CString::new(temp_root.join("resources").display().to_string())
            .expect("resources_dir cstring");
        let lua_packages_dir_text =
            CString::new(temp_root.join("lua_packages").display().to_string())
                .expect("lua_packages_dir cstring");
        let tool_root_dir_text =
            CString::new(temp_root.join("bin").join("tools").display().to_string())
                .expect("tool_root_dir cstring");
        let ffi_root_dir_text =
            CString::new(temp_root.join("libs").display().to_string()).expect("ffi_root cstring");
        let dependency_dir_name = CString::new("dependencies").expect("dependencies cstring");
        let state_dir_name = CString::new("state").expect("state cstring");
        let database_dir_name = CString::new("databases").expect("databases cstring");

        let host_options = FfiLuaRuntimeHostOptions {
            temp_dir: temp_dir_text.as_ptr(),
            resources_dir: resources_dir_text.as_ptr(),
            lua_packages_dir: lua_packages_dir_text.as_ptr(),
            luaexec_program: ptr::null(),
            host_provided_tool_root: tool_root_dir_text.as_ptr(),
            host_provided_lua_root: lua_packages_dir_text.as_ptr(),
            host_provided_ffi_root: ffi_root_dir_text.as_ptr(),
            download_cache_root: ptr::null(),
            dependency_dir_name: dependency_dir_name.as_ptr(),
            state_dir_name: state_dir_name.as_ptr(),
            database_dir_name: database_dir_name.as_ptr(),
            protected_skill_ids: ptr::null(),
            protected_skill_ids_len: 0,
            allow_network_download: 0,
            github_base_url: ptr::null(),
            github_api_base_url: ptr::null(),
            sqlite_library_path: ptr::null(),
            sqlite_provider_mode: FFI_PROVIDER_MODE_DYNAMIC_LIBRARY,
            sqlite_callback_mode: FFI_CALLBACK_MODE_STANDARD,
            lancedb_library_path: ptr::null(),
            lancedb_provider_mode: FFI_PROVIDER_MODE_DYNAMIC_LIBRARY,
            lancedb_callback_mode: FFI_CALLBACK_MODE_STANDARD,
            space_controller_endpoint: ptr::null(),
            space_controller_auto_spawn: 0,
            space_controller_executable_path: ptr::null(),
            space_controller_process_mode: FFI_SPACE_CONTROLLER_PROCESS_MODE_SERVICE,
            cache_config: ptr::null(),
            reserved_entry_names: ptr::null(),
            reserved_entry_names_len: 0,
            ignored_skill_ids: ptr::null(),
            ignored_skill_ids_len: 0,
            enable_skill_management_bridge: 0,
        };
        let engine_options = FfiLuaEngineOptions {
            pool: FfiLuaVmPoolConfig {
                min_size: 1,
                max_size: 1,
                idle_ttl_secs: 30,
            },
            host: host_options,
        };

        let mut engine_id = 0_u64;
        let mut error_out = FfiOwnedBuffer {
            ptr: ptr::null_mut(),
            len: 0,
        };
        let engine_status = unsafe {
            vulcan_luaskills_ffi_engine_new(&engine_options, &mut engine_id, &mut error_out)
        };
        assert_eq!(engine_status, FFI_STATUS_OK);
        assert!(error_out.ptr.is_null());

        let code =
            CString::new("return { note = args.note, transport = vulcan.context.request.transport_name, budget = vulcan.context.client_budget.budget, mode = vulcan.context.tool_config.mode }")
                .expect("code cstring");
        let (_args_storage, args_buffer) = make_borrowed_buffer(r#"{"note":"demo"}"#);
        let (_request_storage, request_buffer) =
            make_borrowed_buffer(r#"{"transport_name":"ffi-test"}"#);
        let (_budget_storage, budget_buffer) = make_borrowed_buffer(r#"{"budget":7}"#);
        let (_tool_storage, tool_buffer) = make_borrowed_buffer(r#"{"mode":"demo-mode"}"#);
        let invocation_context = FfiLuaInvocationContext {
            request_context_json: request_buffer,
            client_budget_json: budget_buffer,
            tool_config_json: tool_buffer,
        };

        let mut result_json_out = FfiOwnedBuffer {
            ptr: ptr::null_mut(),
            len: 0,
        };
        let mut run_error = FfiOwnedBuffer {
            ptr: ptr::null_mut(),
            len: 0,
        };
        let run_status = unsafe {
            vulcan_luaskills_ffi_run_lua(
                engine_id,
                code.as_ptr(),
                args_buffer,
                &invocation_context,
                &mut result_json_out,
                &mut run_error,
            )
        };
        assert_eq!(run_status, FFI_STATUS_OK);
        assert!(run_error.ptr.is_null());

        let result_json_text = read_owned_buffer_text(&result_json_out);
        let result_json: Value =
            serde_json::from_str(&result_json_text).expect("run_lua result must be valid json");
        assert_eq!(result_json["note"], "demo");
        assert_eq!(result_json["transport"], "ffi-test");
        assert_eq!(result_json["budget"], 7);
        assert_eq!(result_json["mode"], "demo-mode");
        unsafe { vulcan_luaskills_ffi_buffer_free(result_json_out) };

        let mut free_error = FfiOwnedBuffer {
            ptr: ptr::null_mut(),
            len: 0,
        };
        let free_status = unsafe { vulcan_luaskills_ffi_engine_free(engine_id, &mut free_error) };
        assert_eq!(free_status, FFI_STATUS_OK);
        assert!(free_error.ptr.is_null());

        let _ = std::fs::remove_dir_all(&temp_root);
    }

    /// Verify standard disable/enable lifecycle calls update the runtime view in place.
    /// 验证标准 disable/enable 生命周期调用会原地更新运行时视图。
    #[test]
    fn standard_ffi_disable_and_enable_skill_round_trip() {
        let temp_root = std::env::temp_dir().join(format!(
            "vulcan_luaskills_standard_ffi_lifecycle_test_{}",
            std::process::id()
        ));
        if temp_root.exists() {
            let _ = std::fs::remove_dir_all(&temp_root);
        }

        let skills_root = temp_root.join("skills");
        let skill_dir = skills_root.join("demo-skill");
        std::fs::create_dir_all(skill_dir.join("runtime")).expect("create runtime directory");
        std::fs::create_dir_all(temp_root.join("temp")).expect("create temp directory");
        std::fs::create_dir_all(temp_root.join("resources")).expect("create resources directory");
        std::fs::create_dir_all(temp_root.join("lua_packages"))
            .expect("create lua_packages directory");
        std::fs::create_dir_all(temp_root.join("bin").join("tools"))
            .expect("create tools directory");
        std::fs::create_dir_all(temp_root.join("libs")).expect("create libs directory");
        std::fs::write(
            skill_dir.join("skill.yaml"),
            "name: demo-skill\nversion: 0.1.0\nenable: true\nentries:\n  - name: ping\n    description: Ping entry.\n    lua_entry: runtime/ping.lua\n    lua_module: demo_skill_ping\n    parameters:\n      - name: note\n        type: string\n        description: Optional note.\n        required: false\n",
        )
        .expect("write skill yaml");
        std::fs::write(
            skill_dir.join("runtime").join("ping.lua"),
            "return function(args)\n  local note = ''\n  if type(args) == 'table' and type(args.note) == 'string' then\n    note = args.note\n  end\n  if note ~= '' then\n    return 'lifecycle:' .. note\n  end\n  return 'lifecycle:ok'\nend\n",
        )
        .expect("write runtime lua");

        let temp_dir_text =
            CString::new(temp_root.join("temp").display().to_string()).expect("temp_dir cstring");
        let resources_dir_text = CString::new(temp_root.join("resources").display().to_string())
            .expect("resources_dir cstring");
        let lua_packages_dir_text =
            CString::new(temp_root.join("lua_packages").display().to_string())
                .expect("lua_packages_dir cstring");
        let tool_root_dir_text =
            CString::new(temp_root.join("bin").join("tools").display().to_string())
                .expect("tool_root_dir cstring");
        let ffi_root_dir_text =
            CString::new(temp_root.join("libs").display().to_string()).expect("ffi_root cstring");
        let dependency_dir_name = CString::new("dependencies").expect("dependencies cstring");
        let state_dir_name = CString::new("state").expect("state cstring");
        let database_dir_name = CString::new("databases").expect("databases cstring");
        let root_name = CString::new("ROOT").expect("root name cstring");
        let skills_root_text =
            CString::new(skills_root.display().to_string()).expect("skills root cstring");
        let skill_id = CString::new("demo-skill").expect("skill_id cstring");
        let tool_name = CString::new("demo-skill-ping").expect("tool_name cstring");
        let disable_reason = CString::new("maintenance").expect("disable reason cstring");

        let host_options = FfiLuaRuntimeHostOptions {
            temp_dir: temp_dir_text.as_ptr(),
            resources_dir: resources_dir_text.as_ptr(),
            lua_packages_dir: lua_packages_dir_text.as_ptr(),
            luaexec_program: ptr::null(),
            host_provided_tool_root: tool_root_dir_text.as_ptr(),
            host_provided_lua_root: lua_packages_dir_text.as_ptr(),
            host_provided_ffi_root: ffi_root_dir_text.as_ptr(),
            download_cache_root: ptr::null(),
            dependency_dir_name: dependency_dir_name.as_ptr(),
            state_dir_name: state_dir_name.as_ptr(),
            database_dir_name: database_dir_name.as_ptr(),
            protected_skill_ids: ptr::null(),
            protected_skill_ids_len: 0,
            allow_network_download: 0,
            github_base_url: ptr::null(),
            github_api_base_url: ptr::null(),
            sqlite_library_path: ptr::null(),
            sqlite_provider_mode: FFI_PROVIDER_MODE_DYNAMIC_LIBRARY,
            sqlite_callback_mode: FFI_CALLBACK_MODE_STANDARD,
            lancedb_library_path: ptr::null(),
            lancedb_provider_mode: FFI_PROVIDER_MODE_DYNAMIC_LIBRARY,
            lancedb_callback_mode: FFI_CALLBACK_MODE_STANDARD,
            space_controller_endpoint: ptr::null(),
            space_controller_auto_spawn: 0,
            space_controller_executable_path: ptr::null(),
            space_controller_process_mode: FFI_SPACE_CONTROLLER_PROCESS_MODE_SERVICE,
            cache_config: ptr::null(),
            reserved_entry_names: ptr::null(),
            reserved_entry_names_len: 0,
            ignored_skill_ids: ptr::null(),
            ignored_skill_ids_len: 0,
            enable_skill_management_bridge: 0,
        };
        let engine_options = FfiLuaEngineOptions {
            pool: FfiLuaVmPoolConfig {
                min_size: 1,
                max_size: 1,
                idle_ttl_secs: 30,
            },
            host: host_options,
        };

        let mut engine_id = 0_u64;
        let mut error_out = FfiOwnedBuffer {
            ptr: ptr::null_mut(),
            len: 0,
        };
        let engine_status = unsafe {
            vulcan_luaskills_ffi_engine_new(&engine_options, &mut engine_id, &mut error_out)
        };
        assert_eq!(engine_status, FFI_STATUS_OK);
        assert!(error_out.ptr.is_null());

        let ffi_skill_roots = [FfiRuntimeSkillRoot {
            name: root_name.as_ptr(),
            skills_dir: skills_root_text.as_ptr(),
        }];

        let mut load_error = FfiOwnedBuffer {
            ptr: ptr::null_mut(),
            len: 0,
        };
        let load_status = unsafe {
            vulcan_luaskills_ffi_load_from_roots(
                engine_id,
                ffi_skill_roots.as_ptr(),
                ffi_skill_roots.len(),
                &mut load_error,
            )
        };
        assert_eq!(load_status, FFI_STATUS_OK);
        assert!(load_error.ptr.is_null());

        let mut entries_out: *mut FfiRuntimeEntryDescriptorList = ptr::null_mut();
        let mut list_error = FfiOwnedBuffer {
            ptr: ptr::null_mut(),
            len: 0,
        };
        let list_status = unsafe {
            vulcan_luaskills_ffi_list_entries(engine_id, &mut entries_out, &mut list_error)
        };
        assert_eq!(list_status, FFI_STATUS_OK);
        assert!(list_error.ptr.is_null());
        assert!(!entries_out.is_null());
        let entries_ref = unsafe { &*entries_out };
        assert_eq!(entries_ref.len, 1);
        unsafe { vulcan_luaskills_ffi_entry_list_free(entries_out) };

        let (_before_disable_args_storage, before_disable_args_buffer) =
            make_borrowed_buffer(r#"{"note":"before-disable"}"#);
        let mut result_out: *mut FfiRuntimeInvocationResult = ptr::null_mut();
        let mut call_error = FfiOwnedBuffer {
            ptr: ptr::null_mut(),
            len: 0,
        };
        let call_status = unsafe {
            vulcan_luaskills_ffi_call_skill(
                engine_id,
                tool_name.as_ptr(),
                before_disable_args_buffer,
                ptr::null(),
                &mut result_out,
                &mut call_error,
            )
        };
        assert_eq!(call_status, FFI_STATUS_OK);
        assert!(call_error.ptr.is_null());
        let result_ref = unsafe { &*result_out };
        assert_eq!(
            read_owned_buffer_text(&result_ref.content),
            "lifecycle:before-disable"
        );
        unsafe { vulcan_luaskills_ffi_invocation_result_free(result_out) };

        let mut disable_error = FfiOwnedBuffer {
            ptr: ptr::null_mut(),
            len: 0,
        };
        let disable_status = unsafe {
            vulcan_luaskills_ffi_disable_skill(
                engine_id,
                ffi_skill_roots.as_ptr(),
                ffi_skill_roots.len(),
                skill_id.as_ptr(),
                disable_reason.as_ptr(),
                &mut disable_error,
            )
        };
        assert_eq!(disable_status, FFI_STATUS_OK);
        assert!(disable_error.ptr.is_null());

        entries_out = ptr::null_mut();
        list_error = FfiOwnedBuffer {
            ptr: ptr::null_mut(),
            len: 0,
        };
        let disabled_list_status = unsafe {
            vulcan_luaskills_ffi_list_entries(engine_id, &mut entries_out, &mut list_error)
        };
        assert_eq!(disabled_list_status, FFI_STATUS_OK);
        assert!(list_error.ptr.is_null());
        assert!(!entries_out.is_null());
        let disabled_entries_ref = unsafe { &*entries_out };
        assert_eq!(disabled_entries_ref.len, 0);
        unsafe { vulcan_luaskills_ffi_entry_list_free(entries_out) };

        result_out = ptr::null_mut();
        call_error = FfiOwnedBuffer {
            ptr: ptr::null_mut(),
            len: 0,
        };
        let (_disabled_args_storage, disabled_args_buffer) =
            make_borrowed_buffer(r#"{"note":"before-disable"}"#);
        let disabled_call_status = unsafe {
            vulcan_luaskills_ffi_call_skill(
                engine_id,
                tool_name.as_ptr(),
                disabled_args_buffer,
                ptr::null(),
                &mut result_out,
                &mut call_error,
            )
        };
        assert_ne!(disabled_call_status, FFI_STATUS_OK);
        assert!(result_out.is_null());
        assert!(!call_error.ptr.is_null());
        unsafe { vulcan_luaskills_ffi_buffer_free(call_error) };

        let mut enable_error = FfiOwnedBuffer {
            ptr: ptr::null_mut(),
            len: 0,
        };
        let enable_status = unsafe {
            vulcan_luaskills_ffi_enable_skill(
                engine_id,
                ffi_skill_roots.as_ptr(),
                ffi_skill_roots.len(),
                skill_id.as_ptr(),
                &mut enable_error,
            )
        };
        assert_eq!(enable_status, FFI_STATUS_OK);
        assert!(enable_error.ptr.is_null());

        entries_out = ptr::null_mut();
        list_error = FfiOwnedBuffer {
            ptr: ptr::null_mut(),
            len: 0,
        };
        let enabled_list_status = unsafe {
            vulcan_luaskills_ffi_list_entries(engine_id, &mut entries_out, &mut list_error)
        };
        assert_eq!(enabled_list_status, FFI_STATUS_OK);
        assert!(list_error.ptr.is_null());
        assert!(!entries_out.is_null());
        let enabled_entries_ref = unsafe { &*entries_out };
        assert_eq!(enabled_entries_ref.len, 1);
        unsafe { vulcan_luaskills_ffi_entry_list_free(entries_out) };

        let (_enabled_args_storage, enabled_args_buffer) =
            make_borrowed_buffer(r#"{"note":"after-enable"}"#);
        result_out = ptr::null_mut();
        call_error = FfiOwnedBuffer {
            ptr: ptr::null_mut(),
            len: 0,
        };
        let enabled_call_status = unsafe {
            vulcan_luaskills_ffi_call_skill(
                engine_id,
                tool_name.as_ptr(),
                enabled_args_buffer,
                ptr::null(),
                &mut result_out,
                &mut call_error,
            )
        };
        assert_eq!(enabled_call_status, FFI_STATUS_OK);
        assert!(call_error.ptr.is_null());
        let enabled_result_ref = unsafe { &*result_out };
        assert_eq!(
            read_owned_buffer_text(&enabled_result_ref.content),
            "lifecycle:after-enable"
        );
        unsafe { vulcan_luaskills_ffi_invocation_result_free(result_out) };

        let mut free_error = FfiOwnedBuffer {
            ptr: ptr::null_mut(),
            len: 0,
        };
        let free_status = unsafe { vulcan_luaskills_ffi_engine_free(engine_id, &mut free_error) };
        assert_eq!(free_status, FFI_STATUS_OK);
        assert!(free_error.ptr.is_null());

        let _ = std::fs::remove_dir_all(&temp_root);
    }
}
