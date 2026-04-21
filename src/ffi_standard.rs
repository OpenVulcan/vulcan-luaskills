use std::ffi::{CStr, CString, c_char};
use std::path::PathBuf;
use std::sync::atomic::Ordering;

use serde_json::Value;

use crate::ffi::{FFI_ENGINE_COUNTER, ffi_engine_registry, with_engine, with_engine_mut};
use crate::runtime_context::RuntimeRequestContext;
use crate::runtime_help::{
    RuntimeHelpDetail, RuntimeHelpNodeDescriptor, RuntimeSkillHelpDescriptor,
};
use crate::runtime_options::{LuaInvocationContext, LuaRuntimeHostOptions, RuntimeSkillRoot};
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
    /// Optional host LanceDB dynamic library path.
    /// 可选宿主 LanceDB 动态库路径。
    pub lancedb_library_path: *const c_char,
    /// Optional tool-cache config pointer.
    /// 可选工具缓存配置指针。
    pub cache_config: *const FfiToolCacheConfig,
    /// Reserved public entry names.
    /// 保留公开入口名称数组。
    pub reserved_entry_names: *const *const c_char,
    /// Number of reserved public entry names.
    /// 保留公开入口名称数组长度。
    pub reserved_entry_names_len: usize,
}

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
    /// Optional request context encoded as JSON text.
    /// 以 JSON 文本编码的可选请求上下文。
    pub request_context_json: *const c_char,
    /// Optional client budget encoded as JSON text.
    /// 以 JSON 文本编码的可选客户端预算。
    pub client_budget_json: *const c_char,
    /// Optional tool config encoded as JSON text.
    /// 以 JSON 文本编码的可选工具配置。
    pub tool_config_json: *const c_char,
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

/// Plain C ABI string-array result.
/// 原生 C ABI 字符串数组结果。
#[repr(C)]
pub struct FfiStringArray {
    /// Owned UTF-8 string item pointers.
    /// 拥有所有权的 UTF-8 字符串指针数组。
    pub items: *mut *mut c_char,
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
    pub name: *mut c_char,
    /// Runtime parameter type string.
    /// 运行时参数类型字符串。
    pub param_type: *mut c_char,
    /// Human-readable parameter description.
    /// 人类可读参数说明。
    pub description: *mut c_char,
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
    pub canonical_name: *mut c_char,
    /// Owning skill id.
    /// 所属技能标识符。
    pub skill_id: *mut c_char,
    /// Local entry name declared by the skill.
    /// 技能声明的局部入口名。
    pub local_name: *mut c_char,
    /// Effective root name.
    /// 生效根名称。
    pub root_name: *mut c_char,
    /// Effective physical skill directory.
    /// 生效物理技能目录。
    pub skill_dir: *mut c_char,
    /// Human-readable entry description.
    /// 人类可读入口描述。
    pub description: *mut c_char,
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
    pub flow_name: *mut c_char,
    /// Human-readable node description.
    /// 人类可读节点描述。
    pub description: *mut c_char,
    /// Related canonical runtime entry names.
    /// 关联的 canonical 运行时入口名称。
    pub related_entries: *mut *mut c_char,
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
    pub skill_id: *mut c_char,
    /// Human-readable skill name.
    /// 人类可读技能名称。
    pub skill_name: *mut c_char,
    /// Semantic skill version.
    /// 语义化技能版本。
    pub skill_version: *mut c_char,
    /// Effective root name.
    /// 生效根名称。
    pub root_name: *mut c_char,
    /// Effective physical skill directory.
    /// 生效物理技能目录。
    pub skill_dir: *mut c_char,
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
    pub skill_id: *mut c_char,
    /// Human-readable skill name.
    /// 人类可读技能名称。
    pub skill_name: *mut c_char,
    /// Semantic skill version.
    /// 语义化技能版本。
    pub skill_version: *mut c_char,
    /// Effective root name.
    /// 生效根名称。
    pub root_name: *mut c_char,
    /// Effective physical skill directory.
    /// 生效物理技能目录。
    pub skill_dir: *mut c_char,
    /// Flow name.
    /// 流程名称。
    pub flow_name: *mut c_char,
    /// Human-readable description.
    /// 人类可读描述。
    pub description: *mut c_char,
    /// Related canonical runtime entries.
    /// 关联的 canonical 运行时入口。
    pub related_entries: *mut *mut c_char,
    /// Number of related canonical runtime entries.
    /// 关联 canonical 运行时入口数量。
    pub related_entries_len: usize,
    /// Whether the node is the main help node.
    /// 当前节点是否为主帮助节点。
    pub is_main: u8,
    /// Structured content type.
    /// 结构化内容类型。
    pub content_type: *mut c_char,
    /// Final rendered help content.
    /// 最终渲染出的帮助内容。
    pub content: *mut c_char,
}

/// Plain C ABI invocation result.
/// 原生 C ABI 调用结果结构。
#[repr(C)]
pub struct FfiRuntimeInvocationResult {
    /// Tool body content.
    /// 工具正文内容。
    pub content: *mut c_char,
    /// Overflow mode encoded as 0 none, 1 truncate, 2 page.
    /// 以 0 无、1 截断、2 分页编码的超限模式。
    pub overflow_mode: i32,
    /// Optional template hint.
    /// 可选模板提示名。
    pub template_hint: *mut c_char,
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
    pub skill_id: *mut c_char,
    /// High-level result status.
    /// 高层结果状态。
    pub status: *mut c_char,
    /// Human-readable message.
    /// 人类可读消息。
    pub message: *mut c_char,
    /// Optional semantic version.
    /// 可选语义化版本。
    pub version: *mut c_char,
    /// Optional source type encoded as integer, where -1 means absent.
    /// 以整数编码的可选来源类型，-1 表示不存在。
    pub source_type: i32,
    /// Optional source locator.
    /// 可选来源定位值。
    pub source_locator: *mut c_char,
}

/// Plain C ABI uninstall result.
/// 原生 C ABI 卸载结果结构。
#[repr(C)]
pub struct FfiSkillUninstallResult {
    /// Stable skill id.
    /// 稳定技能标识符。
    pub skill_id: *mut c_char,
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
    pub message: *mut c_char,
}

/// Write one optional owned error string into the caller-provided error output pointer.
/// 将一段可选拥有所有权的错误字符串写入调用方提供的错误输出指针。
fn set_error_out(error_out: *mut *mut c_char, message: impl Into<String>) {
    if !error_out.is_null() {
        let text = CString::new(message.into())
            .unwrap_or_else(|_| CString::new("FFI error contains NUL byte").expect("static text"));
        unsafe { *error_out = text.into_raw() };
    }
}

/// Clear one caller-provided error output pointer to null.
/// 将调用方提供的错误输出指针清空为 null。
fn clear_error_out(error_out: *mut *mut c_char) {
    if !error_out.is_null() {
        unsafe { *error_out = std::ptr::null_mut() };
    }
}

/// Clear one caller-provided pointer output slot to null.
/// 将调用方提供的指针输出槽位清空为 null。
fn clear_out_ptr<T>(value_out: *mut *mut T) {
    if !value_out.is_null() {
        unsafe { *value_out = std::ptr::null_mut() };
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

/// Convert one optional Rust string into one nullable owned raw C string pointer.
/// 将单个可选 Rust 字符串转换为一个可空拥有所有权的原生 C 字符串指针。
fn alloc_optional_c_string(value: Option<&str>) -> *mut c_char {
    value.map_or(std::ptr::null_mut(), alloc_c_string)
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

/// Parse one optional JSON string into one serde_json value object.
/// 将单个可选 JSON 字符串解析为一个 serde_json 值对象。
fn parse_json_value_or_empty_object(
    value: *const c_char,
    field_name: &str,
) -> Result<Value, String> {
    match parse_optional_string(value, field_name)? {
        Some(text) => serde_json::from_str(&text)
            .map_err(|error| format!("{} contains invalid JSON: {}", field_name, error)),
        None => Ok(Value::Object(serde_json::Map::new())),
    }
}

/// Parse one optional request-context JSON string into one structured request context.
/// 将单个可选请求上下文 JSON 字符串解析为一个结构化请求上下文。
fn parse_request_context(value: *const c_char) -> Result<Option<RuntimeRequestContext>, String> {
    match parse_optional_string(value, "request_context_json")? {
        Some(text) => serde_json::from_str(&text)
            .map(Some)
            .map_err(|error| format!("request_context_json contains invalid JSON: {}", error)),
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
        lancedb_library_path: parse_optional_string(
            value.lancedb_library_path,
            "lancedb_library_path",
        )?
        .map(PathBuf::from),
        cache_config: parse_cache_config(value.cache_config),
        reserved_entry_names: parse_string_array(
            value.reserved_entry_names,
            value.reserved_entry_names_len,
            "reserved_entry_names",
        )?,
    })
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
        parse_request_context(context.request_context_json)?,
        parse_json_value_or_empty_object(context.client_budget_json, "client_budget_json")?,
        parse_json_value_or_empty_object(context.tool_config_json, "tool_config_json")?,
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
    let mut items: Vec<*mut c_char> = values.iter().map(alloc_c_string).collect();
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
        name: alloc_c_string(&value.name),
        param_type: alloc_c_string(&value.param_type),
        description: alloc_c_string(&value.description),
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
        canonical_name: alloc_c_string(&value.canonical_name),
        skill_id: alloc_c_string(&value.skill_id),
        local_name: alloc_c_string(&value.local_name),
        root_name: alloc_c_string(&value.root_name),
        skill_dir: alloc_c_string(&value.skill_dir),
        description: alloc_c_string(&value.description),
        parameters: parameters_ptr,
        parameters_len,
    }
}

/// Convert one help node descriptor into one C ABI descriptor.
/// 将单个帮助节点描述转换为一个 C ABI 描述结构。
fn alloc_help_node_descriptor(value: &RuntimeHelpNodeDescriptor) -> FfiRuntimeHelpNodeDescriptor {
    let related_entries = alloc_string_array(&value.related_entries);
    FfiRuntimeHelpNodeDescriptor {
        flow_name: alloc_c_string(&value.flow_name),
        description: alloc_c_string(&value.description),
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
        skill_id: alloc_c_string(&value.skill_id),
        skill_name: alloc_c_string(&value.skill_name),
        skill_version: alloc_c_string(&value.skill_version),
        root_name: alloc_c_string(&value.root_name),
        skill_dir: alloc_c_string(&value.skill_dir),
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
        skill_id: alloc_c_string(&value.skill_id),
        skill_name: alloc_c_string(&value.skill_name),
        skill_version: alloc_c_string(&value.skill_version),
        root_name: alloc_c_string(&value.root_name),
        skill_dir: alloc_c_string(&value.skill_dir),
        flow_name: alloc_c_string(&value.flow_name),
        description: alloc_c_string(&value.description),
        related_entries: related_entries.items,
        related_entries_len: related_entries.len,
        is_main: u8::from(value.is_main),
        content_type: alloc_c_string(&value.content_type),
        content: alloc_c_string(&value.content),
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
        content: alloc_c_string(&value.content),
        overflow_mode,
        template_hint: alloc_optional_c_string(value.template_hint.as_deref()),
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
        skill_id: alloc_c_string(&value.skill_id),
        status: alloc_c_string(&value.status),
        message: alloc_c_string(&value.message),
        version: alloc_optional_c_string(value.version.as_deref()),
        source_type,
        source_locator: alloc_optional_c_string(value.source_locator.as_deref()),
    }
}

/// Convert one uninstall result into one C ABI result.
/// 将单个卸载结果转换为一个 C ABI 结果结构。
fn alloc_skill_uninstall_result(value: &SkillUninstallResult) -> FfiSkillUninstallResult {
    FfiSkillUninstallResult {
        skill_id: alloc_c_string(&value.skill_id),
        skill_removed: u8::from(value.skill_removed),
        sqlite_removed: u8::from(value.sqlite_removed),
        lancedb_removed: u8::from(value.lancedb_removed),
        sqlite_retained: u8::from(value.sqlite_retained),
        lancedb_retained: u8::from(value.lancedb_retained),
        message: alloc_c_string(&value.message),
    }
}

/// Free one owned C string pointer if it is not null.
/// 如果单个拥有所有权的 C 字符串指针非空，则释放它。
unsafe fn free_c_string(value: *mut c_char) {
    if !value.is_null() {
        let _ = unsafe { CString::from_raw(value) };
    }
}

/// Free one owned string array and all nested string items.
/// 释放单个拥有所有权的字符串数组以及其嵌套字符串条目。
unsafe fn free_string_array_parts(items: *mut *mut c_char, len: usize) {
    if items.is_null() || len == 0 {
        return;
    }
    let values = unsafe { Vec::from_raw_parts(items, len, len) };
    for value in values {
        unsafe { free_c_string(value) };
    }
}

/// Free one owned entry parameter descriptor.
/// 释放单个拥有所有权的入口参数描述结构。
unsafe fn free_entry_parameter_descriptor(value: FfiRuntimeEntryParameterDescriptor) {
    unsafe { free_c_string(value.name) };
    unsafe { free_c_string(value.param_type) };
    unsafe { free_c_string(value.description) };
}

/// Free one owned entry descriptor.
/// 释放单个拥有所有权的入口描述结构。
unsafe fn free_entry_descriptor(value: FfiRuntimeEntryDescriptor) {
    unsafe { free_c_string(value.canonical_name) };
    unsafe { free_c_string(value.skill_id) };
    unsafe { free_c_string(value.local_name) };
    unsafe { free_c_string(value.root_name) };
    unsafe { free_c_string(value.skill_dir) };
    unsafe { free_c_string(value.description) };
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
    unsafe { free_c_string(value.flow_name) };
    unsafe { free_c_string(value.description) };
    unsafe { free_string_array_parts(value.related_entries, value.related_entries_len) };
}

/// Write one successful status code.
/// 写入一个成功状态码。
fn ffi_ok_status(error_out: *mut *mut c_char) -> i32 {
    clear_error_out(error_out);
    FFI_STATUS_OK
}

/// Write one failed status code and error text.
/// 写入一个失败状态码与错误文本。
fn ffi_error_status(error_out: *mut *mut c_char, message: impl Into<String>) -> i32 {
    set_error_out(error_out, message);
    FFI_STATUS_ERROR
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
            unsafe { free_c_string(item.skill_id) };
            unsafe { free_c_string(item.skill_name) };
            unsafe { free_c_string(item.skill_version) };
            unsafe { free_c_string(item.root_name) };
            unsafe { free_c_string(item.skill_dir) };
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
    let value = unsafe { Box::from_raw(value) };
    unsafe { free_c_string(value.skill_id) };
    unsafe { free_c_string(value.skill_name) };
    unsafe { free_c_string(value.skill_version) };
    unsafe { free_c_string(value.root_name) };
    unsafe { free_c_string(value.skill_dir) };
    unsafe { free_c_string(value.flow_name) };
    unsafe { free_c_string(value.description) };
    unsafe { free_string_array_parts(value.related_entries, value.related_entries_len) };
    unsafe { free_c_string(value.content_type) };
    unsafe { free_c_string(value.content) };
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
    let value = unsafe { Box::from_raw(value) };
    unsafe { free_c_string(value.content) };
    unsafe { free_c_string(value.template_hint) };
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
    let value = unsafe { Box::from_raw(value) };
    unsafe { free_c_string(value.skill_id) };
    unsafe { free_c_string(value.status) };
    unsafe { free_c_string(value.message) };
    unsafe { free_c_string(value.version) };
    unsafe { free_c_string(value.source_locator) };
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
    let value = unsafe { Box::from_raw(value) };
    unsafe { free_c_string(value.skill_id) };
    unsafe { free_c_string(value.message) };
}

/// Return the stable FFI version string through the standard C ABI surface.
/// 通过标准 C ABI 接口返回稳定的 FFI 版本字符串。
#[unsafe(no_mangle)]
pub extern "C" fn vulcan_luaskills_ffi_version(
    version_out: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> i32 {
    clear_error_out(error_out);
    clear_out_ptr(version_out);
    if version_out.is_null() {
        return ffi_error_status(error_out, "version_out must not be null");
    }
    unsafe { *version_out = alloc_c_string(crate::ffi::FFI_VERSION) };
    ffi_ok_status(error_out)
}

/// Return the exported FFI entrypoint names through the standard C ABI surface.
/// 通过标准 C ABI 接口返回已导出 FFI 入口点名称。
#[unsafe(no_mangle)]
pub extern "C" fn vulcan_luaskills_ffi_describe(
    functions_out: *mut *mut FfiStringArray,
    error_out: *mut *mut c_char,
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
pub extern "C" fn vulcan_luaskills_ffi_engine_new(
    options: *const FfiLuaEngineOptions,
    engine_id_out: *mut u64,
    error_out: *mut *mut c_char,
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
                    registry.insert(engine_id, crate::ffi::FfiEngineSlot { engine });
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
pub extern "C" fn vulcan_luaskills_ffi_engine_free(
    engine_id: u64,
    error_out: *mut *mut c_char,
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
pub extern "C" fn vulcan_luaskills_ffi_load_from_dirs(
    engine_id: u64,
    base_dir: *const c_char,
    override_dir: *const c_char,
    error_out: *mut *mut c_char,
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
pub extern "C" fn vulcan_luaskills_ffi_load_from_roots(
    engine_id: u64,
    skill_roots: *const FfiRuntimeSkillRoot,
    skill_roots_len: usize,
    error_out: *mut *mut c_char,
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
pub extern "C" fn vulcan_luaskills_ffi_reload_from_dirs(
    engine_id: u64,
    base_dir: *const c_char,
    override_dir: *const c_char,
    error_out: *mut *mut c_char,
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
pub extern "C" fn vulcan_luaskills_ffi_reload_from_roots(
    engine_id: u64,
    skill_roots: *const FfiRuntimeSkillRoot,
    skill_roots_len: usize,
    error_out: *mut *mut c_char,
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
pub extern "C" fn vulcan_luaskills_ffi_list_entries(
    engine_id: u64,
    entries_out: *mut *mut FfiRuntimeEntryDescriptorList,
    error_out: *mut *mut c_char,
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
pub extern "C" fn vulcan_luaskills_ffi_list_skill_help(
    engine_id: u64,
    help_out: *mut *mut FfiRuntimeSkillHelpDescriptorList,
    error_out: *mut *mut c_char,
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
pub extern "C" fn vulcan_luaskills_ffi_render_skill_help_detail(
    engine_id: u64,
    skill_id: *const c_char,
    flow_name: *const c_char,
    request_context_json: *const c_char,
    detail_out: *mut *mut FfiRuntimeHelpDetail,
    error_out: *mut *mut c_char,
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
    let request_context = match parse_request_context(request_context_json) {
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
pub extern "C" fn vulcan_luaskills_ffi_prompt_argument_completions(
    engine_id: u64,
    prompt_name: *const c_char,
    argument_name: *const c_char,
    values_out: *mut *mut FfiStringArray,
    error_out: *mut *mut c_char,
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
pub extern "C" fn vulcan_luaskills_ffi_is_skill(
    engine_id: u64,
    tool_name: *const c_char,
    value_out: *mut u8,
    error_out: *mut *mut c_char,
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
pub extern "C" fn vulcan_luaskills_ffi_skill_name_for_tool(
    engine_id: u64,
    tool_name: *const c_char,
    skill_id_out: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> i32 {
    clear_error_out(error_out);
    clear_out_ptr(skill_id_out);
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
            unsafe { *skill_id_out = alloc_optional_c_string(skill_id.as_deref()) };
            ffi_ok_status(error_out)
        }
        Err(error) => ffi_error_status(error_out, error),
    }
}

/// Call one loaded skill entry through the standard C ABI surface.
/// 通过标准 C ABI 接口调用单个已加载技能入口。
#[unsafe(no_mangle)]
pub extern "C" fn vulcan_luaskills_ffi_call_skill(
    engine_id: u64,
    tool_name: *const c_char,
    args_json: *const c_char,
    invocation_context: *const FfiLuaInvocationContext,
    result_out: *mut *mut FfiRuntimeInvocationResult,
    error_out: *mut *mut c_char,
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
    let args = match parse_json_value_or_empty_object(args_json, "args_json") {
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
pub extern "C" fn vulcan_luaskills_ffi_run_lua(
    engine_id: u64,
    code: *const c_char,
    args_json: *const c_char,
    invocation_context: *const FfiLuaInvocationContext,
    result_json_out: *mut *mut c_char,
    error_out: *mut *mut c_char,
) -> i32 {
    clear_error_out(error_out);
    clear_out_ptr(result_json_out);
    if result_json_out.is_null() {
        return ffi_error_status(error_out, "result_json_out must not be null");
    }
    let code = match parse_required_string(code, "code") {
        Ok(code) => code,
        Err(error) => return ffi_error_status(error_out, error),
    };
    let args = match parse_json_value_or_empty_object(args_json, "args_json") {
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
                unsafe { *result_json_out = alloc_c_string(result_json) };
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
pub extern "C" fn vulcan_luaskills_ffi_disable_skill_in_dirs(
    engine_id: u64,
    base_dir: *const c_char,
    override_dir: *const c_char,
    skill_id: *const c_char,
    reason: *const c_char,
    error_out: *mut *mut c_char,
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
pub extern "C" fn vulcan_luaskills_ffi_disable_skill(
    engine_id: u64,
    skill_roots: *const FfiRuntimeSkillRoot,
    skill_roots_len: usize,
    skill_id: *const c_char,
    reason: *const c_char,
    error_out: *mut *mut c_char,
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
pub extern "C" fn vulcan_luaskills_ffi_system_disable_skill_in_dirs(
    engine_id: u64,
    base_dir: *const c_char,
    override_dir: *const c_char,
    skill_id: *const c_char,
    reason: *const c_char,
    error_out: *mut *mut c_char,
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
pub extern "C" fn vulcan_luaskills_ffi_system_disable_skill(
    engine_id: u64,
    skill_roots: *const FfiRuntimeSkillRoot,
    skill_roots_len: usize,
    skill_id: *const c_char,
    reason: *const c_char,
    error_out: *mut *mut c_char,
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
pub extern "C" fn vulcan_luaskills_ffi_enable_skill(
    engine_id: u64,
    skill_roots: *const FfiRuntimeSkillRoot,
    skill_roots_len: usize,
    skill_id: *const c_char,
    error_out: *mut *mut c_char,
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
pub extern "C" fn vulcan_luaskills_ffi_system_enable_skill(
    engine_id: u64,
    skill_roots: *const FfiRuntimeSkillRoot,
    skill_roots_len: usize,
    skill_id: *const c_char,
    error_out: *mut *mut c_char,
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
pub extern "C" fn vulcan_luaskills_ffi_uninstall_skill(
    engine_id: u64,
    skill_roots: *const FfiRuntimeSkillRoot,
    skill_roots_len: usize,
    skill_id: *const c_char,
    options: *const FfiSkillUninstallOptions,
    result_out: *mut *mut FfiSkillUninstallResult,
    error_out: *mut *mut c_char,
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
pub extern "C" fn vulcan_luaskills_ffi_system_uninstall_skill(
    engine_id: u64,
    skill_roots: *const FfiRuntimeSkillRoot,
    skill_roots_len: usize,
    skill_id: *const c_char,
    options: *const FfiSkillUninstallOptions,
    result_out: *mut *mut FfiSkillUninstallResult,
    error_out: *mut *mut c_char,
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
pub extern "C" fn vulcan_luaskills_ffi_install_skill(
    engine_id: u64,
    skill_roots: *const FfiRuntimeSkillRoot,
    skill_roots_len: usize,
    request: *const FfiSkillInstallRequest,
    result_out: *mut *mut FfiSkillApplyResult,
    error_out: *mut *mut c_char,
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
pub extern "C" fn vulcan_luaskills_ffi_system_install_skill(
    engine_id: u64,
    skill_roots: *const FfiRuntimeSkillRoot,
    skill_roots_len: usize,
    request: *const FfiSkillInstallRequest,
    result_out: *mut *mut FfiSkillApplyResult,
    error_out: *mut *mut c_char,
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
pub extern "C" fn vulcan_luaskills_ffi_update_skill(
    engine_id: u64,
    skill_roots: *const FfiRuntimeSkillRoot,
    skill_roots_len: usize,
    request: *const FfiSkillInstallRequest,
    result_out: *mut *mut FfiSkillApplyResult,
    error_out: *mut *mut c_char,
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
pub extern "C" fn vulcan_luaskills_ffi_system_update_skill(
    engine_id: u64,
    skill_roots: *const FfiRuntimeSkillRoot,
    skill_roots_len: usize,
    request: *const FfiSkillInstallRequest,
    result_out: *mut *mut FfiSkillApplyResult,
    error_out: *mut *mut c_char,
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
