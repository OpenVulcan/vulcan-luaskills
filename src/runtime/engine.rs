use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use mlua::{Function, HookTriggers, Lua, MultiValue, Table, Value as LuaValue, VmState};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::io::{ErrorKind, Read, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock, TryLockError};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
#[cfg(windows)]
use windows_sys::Win32::System::LibraryLoader::{
    AddDllDirectory, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS, LOAD_LIBRARY_SEARCH_USER_DIRS,
    RemoveDllDirectory, SetDefaultDllDirectories,
};

use crate::dependency::manager::{DependencyManager, DependencyManagerConfig, ensure_directory};
use crate::entry_descriptor::{RuntimeEntryDescriptor, RuntimeEntryParameterDescriptor};
use crate::host::callbacks::{
    RuntimeEntryRegistryDelta, RuntimeHostToolAction, RuntimeHostToolRequest, RuntimeModelCaller,
    RuntimeModelEmbedRequest, RuntimeModelEmbedResponse, RuntimeModelError, RuntimeModelErrorCode,
    RuntimeModelLlmRequest, RuntimeModelLlmResponse, RuntimeModelUsage, RuntimeSkillLifecycleEvent,
    RuntimeSkillManagementAction, RuntimeSkillManagementRequest,
    RuntimeSkillOperationProgressEmitter, dispatch_host_tool_request, dispatch_model_embed_request,
    dispatch_model_llm_request, dispatch_skill_management_request, try_has_host_tool_callback,
    try_has_model_embed_callback, try_has_model_llm_callback, try_has_skill_management_callback,
};
use crate::host::database::RuntimeDatabaseProviderCallbacks;
use crate::lancedb_host::{LanceDbSkillBinding, LanceDbSkillHost, disabled_skill_status_json};
use crate::lua_skill::{SkillMeta, validate_luaskills_identifier, validate_luaskills_version};
use crate::runtime::config::{SkillConfigEntry, SkillConfigStore};
use crate::runtime::encoding::{
    RuntimeTextEncoding, decode_runtime_text, default_runtime_text_encoding, encode_runtime_text,
};
use crate::runtime::managed_io::{create_vulcan_io_table, install_managed_io_compat};
use crate::runtime::process_session::create_process_session_table;
use crate::runtime_context::{RuntimeClientInfo, RuntimeRequestContext};
use crate::runtime_help::{
    RuntimeHelpDetail, RuntimeHelpNodeDescriptor, RuntimeSkillHelpDescriptor,
};
use crate::runtime_logging::{error as log_error, info as log_info, warn as log_warn};
use crate::runtime_options::{LuaInvocationContext, LuaRuntimeHostOptions, RuntimeSkillRoot};
use crate::runtime_result::RuntimeInvocationResult;
use crate::skill::dependencies::SkillDependencyManifest;
use crate::skill::manager::{
    PreparedSkillApply, ResolvedSkillInstance, SkillApplyResult, SkillInstallRequest,
    SkillManagementAuthority, SkillManager, SkillManagerConfig, SkillOperationPlane,
    SkillUninstallOptions, SkillUninstallResult, collect_effective_skill_instances_from_roots,
    resolve_declared_skill_instance_from_roots, resolve_effective_skill_instance_from_roots,
    resolve_requested_skill_id,
};
use crate::sqlite_host::{
    SqliteSkillBinding, SqliteSkillHost,
    disabled_skill_status_json as disabled_sqlite_skill_status_json,
};
use crate::tool_cache::{ToolCacheConfig, configure_global_tool_cache, global_tool_cache};

mod bridge;
mod host_result;
mod lease;
mod runlua;

use self::bridge::{
    create_host_tool_call_fn, create_host_tool_has_fn, create_host_tool_list_fn,
    create_model_embed_fn, create_model_has_fn, create_model_llm_fn, create_model_status_fn,
    create_runtime_skill_layers_fn, create_runtime_skill_management_bridge_fn,
};
use self::host_result::{
    host_result_capability_to_json_value, parse_tool_call_output, resolve_host_result_capability,
};
use self::lease::RuntimeSessionManager;
use self::runlua::{
    default_exec_shell_name, exec_result_to_lua_table, execute_exec_request, optional_u64_arg,
    parse_exec_request, require_path_arg, require_string_arg, require_table_arg,
    resolve_host_default_text_encoding, supported_exec_shell_names,
};

// ============================================================
// Loaded skill (compiled Lua function + metadata)
// ============================================================

#[derive(Clone)]
struct LoadedSkill {
    meta: SkillMeta,
    dir: std::path::PathBuf,
    root_name: String,
    lancedb_binding: Option<Arc<LanceDbSkillBinding>>,
    sqlite_binding: Option<Arc<SqliteSkillBinding>>,
    resolved_entry_names: HashMap<String, String>,
}

/// Normalize one host-visible path string so Windows verbatim prefixes never leak into logs or Lua-visible context.
/// 归一化一个对宿主可见的路径文本，避免 Windows verbatim 前缀泄漏到日志或 Lua 可见上下文中。
fn normalize_host_visible_path_text(rendered: &str) -> String {
    #[cfg(windows)]
    {
        if let Some(stripped) = rendered.strip_prefix(r"\\?\UNC\") {
            return format!(r"\\{}", stripped);
        }
        if let Some(stripped) = rendered.strip_prefix(r"\\?\") {
            return stripped.to_string();
        }
    }
    rendered.to_string()
}

/// Render one filesystem path for host-visible runtime surfaces without Windows verbatim prefixes.
/// 为宿主可见的运行时表面渲染文件系统路径，并去掉 Windows verbatim 前缀。
fn render_host_visible_path(path: &Path) -> String {
    normalize_host_visible_path_text(&path.to_string_lossy())
}

/// Render one filesystem path for user-facing runtime logs without Windows verbatim prefixes.
/// 为面向用户的运行时日志渲染文件系统路径，并去掉 Windows verbatim 前缀。
fn render_log_friendly_path(path: &Path) -> String {
    render_host_visible_path(path)
}

/// Normalize one runtime-root path with stable lexical component folding.
/// 使用稳定的词法组件折叠规则规范化单个运行时根目录路径。
fn normalize_runtime_root_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    let mut can_pop_normal = false;
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => {
                normalized.push(prefix.as_os_str());
                can_pop_normal = false;
            }
            Component::RootDir => {
                normalized.push(component.as_os_str());
                can_pop_normal = false;
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if can_pop_normal && normalized.pop() {
                    can_pop_normal = !matches!(
                        normalized.components().next_back(),
                        Some(Component::Prefix(_)) | Some(Component::RootDir) | None
                    );
                } else if !path.is_absolute() {
                    normalized.push(component.as_os_str());
                    can_pop_normal = false;
                }
            }
            Component::Normal(part) => {
                normalized.push(part);
                can_pop_normal = true;
            }
        }
    }
    normalized
}

/// Structured path table stored in `resources/luaskills-packages-manifest.json`.
/// 存储在 `resources/luaskills-packages-manifest.json` 中的结构化路径表。
#[derive(Debug, Deserialize)]
struct RuntimePackagesManifestPaths {
    install_manifest: String,
    compat_lua_packages_txt: String,
    platform_support: String,
    third_party_licenses: String,
    third_party_notices: String,
    help_index: String,
    package_help_root: String,
    module_help_root: String,
    license_index: String,
}

/// Runtime-facing luaskills-packages manifest embedded inside one packaged runtime.
/// 嵌入在一个打包运行时中的面向运行时的 luaskills-packages 清单。
#[derive(Debug, Deserialize)]
struct RuntimePackagesManifest {
    schema_version: u32,
    layout: String,
    paths: RuntimePackagesManifestPaths,
}

/// Require one runtime-relative path string and reject absolute or traversal-only payloads.
/// 要求一个运行时相对路径字符串，并拒绝绝对路径或纯穿越型载荷。
fn validate_runtime_relative_manifest_path(label: &str, relative_path: &str) -> Result<(), String> {
    let candidate = Path::new(relative_path);
    if candidate.is_absolute() {
        return Err(format!(
            "packaged runtime is invalid: {} must be runtime-relative, got '{}'",
            label, relative_path
        ));
    }
    if candidate.components().next().is_none() {
        return Err(format!("packaged runtime is invalid: {} is empty", label));
    }
    Ok(())
}

/// Validate one required manifest target path inside a packaged runtime root.
/// 校验打包运行时根目录中的一个必需清单目标路径。
fn validate_packaged_runtime_target(
    runtime_root: &Path,
    label: &str,
    relative_path: &str,
) -> Result<(), String> {
    validate_runtime_relative_manifest_path(label, relative_path)?;
    let candidate = runtime_root.join(relative_path);
    if !candidate.exists() {
        return Err(format!(
            "packaged runtime is invalid: missing {}",
            render_log_friendly_path(&candidate)
        ));
    }
    Ok(())
}

/// Validate the luaskills-packages metadata layout embedded under one packaged runtime resources directory.
/// 校验一个打包运行时 resources 目录下嵌入的 luaskills-packages 元数据布局。
fn validate_packaged_runtime_packages_layout(resources_dir: &Path) -> Result<(), String> {
    let runtime_manifest_path = resources_dir.join("lua-runtime-manifest.json");
    if !runtime_manifest_path.exists() {
        return Ok(());
    }

    let runtime_root = resources_dir.parent().ok_or_else(|| {
        format!(
            "packaged runtime is invalid: resources directory has no parent: {}",
            render_log_friendly_path(resources_dir)
        )
    })?;
    let packages_manifest_path = resources_dir.join("luaskills-packages-manifest.json");
    if !packages_manifest_path.exists() {
        return Err(format!(
            "packaged runtime is incomplete: missing {}",
            render_log_friendly_path(&packages_manifest_path)
        ));
    }

    let manifest_text = fs::read_to_string(&packages_manifest_path).map_err(|error| {
        format!(
            "packaged runtime is invalid: failed to read {}: {}",
            render_log_friendly_path(&packages_manifest_path),
            error
        )
    })?;
    let manifest: RuntimePackagesManifest =
        serde_json::from_str(&manifest_text).map_err(|error| {
            format!(
                "packaged runtime is invalid: failed to parse {}: {}",
                render_log_friendly_path(&packages_manifest_path),
                error
            )
        })?;

    if manifest.schema_version != 1 {
        return Err(format!(
            "packaged runtime is invalid: unsupported luaskills-packages manifest schema_version {}",
            manifest.schema_version
        ));
    }
    if manifest.layout != "luaskills-packages-runtime-v1" {
        return Err(format!(
            "packaged runtime is invalid: unsupported luaskills-packages layout '{}'",
            manifest.layout
        ));
    }

    validate_packaged_runtime_target(
        runtime_root,
        "install_manifest",
        &manifest.paths.install_manifest,
    )?;
    validate_packaged_runtime_target(
        runtime_root,
        "compat_lua_packages_txt",
        &manifest.paths.compat_lua_packages_txt,
    )?;
    validate_packaged_runtime_target(
        runtime_root,
        "platform_support",
        &manifest.paths.platform_support,
    )?;
    validate_packaged_runtime_target(
        runtime_root,
        "third_party_licenses",
        &manifest.paths.third_party_licenses,
    )?;
    validate_packaged_runtime_target(
        runtime_root,
        "third_party_notices",
        &manifest.paths.third_party_notices,
    )?;
    validate_packaged_runtime_target(runtime_root, "help_index", &manifest.paths.help_index)?;
    validate_packaged_runtime_target(
        runtime_root,
        "package_help_root",
        &manifest.paths.package_help_root,
    )?;
    validate_packaged_runtime_target(
        runtime_root,
        "module_help_root",
        &manifest.paths.module_help_root,
    )?;
    validate_packaged_runtime_target(runtime_root, "license_index", &manifest.paths.license_index)?;
    Ok(())
}

/// Pool sizing configuration for Lua virtual machines.
/// Lua 虚拟机池的容量配置。
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct LuaVmPoolConfig {
    /// Minimum number of VMs that should stay warm.
    /// 需要常驻保温的最小虚拟机数量。
    pub min_size: usize,
    /// Maximum number of VMs allowed in the pool.
    /// 池内允许存在的最大虚拟机数量。
    pub max_size: usize,
    /// Idle TTL in seconds before an excess VM can be retired.
    /// 多余虚拟机在空闲多少秒后允许回收。
    pub idle_ttl_secs: u64,
}

impl LuaVmPoolConfig {
    /// Return a normalized pool config with safe bounds.
    /// 返回经过安全边界归一化后的池配置。
    fn normalized(self) -> Self {
        let min_size = self.min_size.max(1);
        let max_size = self.max_size.max(min_size);
        let idle_ttl_secs = self.idle_ttl_secs.max(1);
        Self {
            min_size,
            max_size,
            idle_ttl_secs,
        }
    }
}

/// Return the default dedicated pool config used by isolated runlua execution.
/// 返回隔离 runlua 执行使用的默认独立池配置。
fn default_runlua_vm_pool_config() -> LuaVmPoolConfig {
    LuaVmPoolConfig {
        min_size: 1,
        max_size: 4,
        idle_ttl_secs: 60,
    }
}

/// Runtime state of a single Lua VM instance.
/// 单个 Lua 虚拟机实例的运行时状态。
struct LuaVm {
    lua: Lua,
    last_used_at: Instant,
}

/// Shared mutable state for the Lua VM pool.
/// Lua 虚拟机池的共享可变状态。
struct LuaVmPoolState {
    available: Vec<LuaVm>,
    total_count: usize,
}

/// Pool of Lua VM instances with opportunistic scaling.
/// 支持按需扩缩容的 Lua 虚拟机池。
struct LuaVmPool {
    config: LuaVmPoolConfig,
    state: Mutex<LuaVmPoolState>,
    condvar: Condvar,
}

/// Process-level native library search guard owned by one runtime engine.
/// 由单个运行时引擎持有的进程级原生库搜索保护句柄。
#[derive(Debug, Default)]
struct NativeLibrarySearchGuard {
    /// Registered native library directories kept alive for the lifetime of this engine.
    /// 在该引擎生命周期内保持有效的已注册原生库目录集合。
    #[cfg(windows)]
    directories: Vec<NativeLibraryDirectoryCookie>,
}

/// Windows DLL directory cookie returned by `AddDllDirectory`.
/// `AddDllDirectory` 返回的 Windows DLL 目录句柄。
#[cfg(windows)]
#[derive(Debug)]
struct NativeLibraryDirectoryCookie(*mut core::ffi::c_void);

#[cfg(windows)]
impl NativeLibraryDirectoryCookie {
    /// Return whether the cookie points at one registered DLL directory.
    /// 返回该句柄是否指向一个已注册的 DLL 目录。
    fn is_valid(&self) -> bool {
        !self.0.is_null()
    }
}

unsafe impl Send for NativeLibraryDirectoryCookie {}
unsafe impl Sync for NativeLibraryDirectoryCookie {}

#[cfg(windows)]
impl Drop for NativeLibrarySearchGuard {
    /// Drop registered DLL directory cookies before the guard is released.
    /// 在保护句柄释放前丢弃已注册的 DLL 目录句柄。
    fn drop(&mut self) {
        self.directories.clear();
    }
}

#[cfg(windows)]
impl Drop for NativeLibraryDirectoryCookie {
    /// Remove the registered Windows DLL directory when the owning engine is dropped.
    /// 当所属引擎释放时移除已注册的 Windows DLL 目录。
    fn drop(&mut self) {
        if self.is_valid() {
            unsafe {
                RemoveDllDirectory(self.0);
            }
        }
    }
}

impl NativeLibrarySearchGuard {
    /// Register the host-provided FFI/native library root for this process.
    /// 为当前进程注册宿主提供的 FFI/原生库根目录。
    fn new(host_options: &LuaRuntimeHostOptions) -> Result<Self, String> {
        #[cfg(windows)]
        {
            Self::new_windows(host_options)
        }
        #[cfg(not(windows))]
        {
            let _ = host_options;
            Ok(Self::default())
        }
    }

    /// Register Windows DLL search directories without mutating the global PATH variable.
    /// 在不修改全局 PATH 变量的前提下注册 Windows DLL 搜索目录。
    #[cfg(windows)]
    fn new_windows(host_options: &LuaRuntimeHostOptions) -> Result<Self, String> {
        let mut directories = Vec::new();
        let Some(host_provided_ffi_root) = host_options.host_provided_ffi_root.as_ref() else {
            return Ok(Self { directories });
        };
        if !host_provided_ffi_root.is_dir() {
            return Ok(Self { directories });
        }

        // DefaultDllDirectories enables the USER_DIRS search bucket used by AddDllDirectory.
        // DefaultDllDirectories 启用 AddDllDirectory 所依赖的 USER_DIRS 搜索桶。
        let default_directory_result = unsafe {
            SetDefaultDllDirectories(
                LOAD_LIBRARY_SEARCH_DEFAULT_DIRS | LOAD_LIBRARY_SEARCH_USER_DIRS,
            )
        };
        if default_directory_result == 0 {
            return Err(format!(
                "failed to enable Windows DLL directory search for host_provided_ffi_root: {}",
                std::io::Error::last_os_error()
            ));
        }

        let wide_path = windows_wide_null_path(host_provided_ffi_root)?;
        let cookie = NativeLibraryDirectoryCookie(unsafe { AddDllDirectory(wide_path.as_ptr()) });
        if !cookie.is_valid() {
            return Err(format!(
                "failed to add Windows DLL directory {}: {}",
                host_provided_ffi_root.display(),
                std::io::Error::last_os_error()
            ));
        }
        directories.push(cookie);
        Ok(Self { directories })
    }
}

/// Convert one Windows filesystem path into a null-terminated wide string.
/// 将单个 Windows 文件系统路径转换为以空字符结尾的宽字符串。
#[cfg(windows)]
fn windows_wide_null_path(path: &Path) -> Result<Vec<u16>, String> {
    use std::os::windows::ffi::OsStrExt;

    let mut wide_path = path.as_os_str().encode_wide().collect::<Vec<u16>>();
    if wide_path.iter().any(|value| *value == 0) {
        return Err(format!(
            "Windows DLL directory contains an embedded NUL: {}",
            path.display()
        ));
    }
    wide_path.push(0);
    Ok(wide_path)
}

// ============================================================
// LuaEngine — LuaJIT VM wrapper
// ============================================================

pub struct LuaEngine {
    skills: HashMap<String, LoadedSkill>,
    entry_registry: BTreeMap<String, ResolvedEntryTarget>,
    runtime_skill_roots: Vec<RuntimeSkillRoot>,
    pool: Arc<LuaVmPool>,
    runlua_pool: Arc<LuaVmPool>,
    runtime_sessions: Arc<RuntimeSessionManager>,
    skill_config_store: Arc<SkillConfigStore>,
    lancedb_host: Option<Arc<LanceDbSkillHost>>,
    sqlite_host: Option<Arc<SqliteSkillHost>>,
    database_provider_callbacks: Arc<RuntimeDatabaseProviderCallbacks>,
    native_library_search_guard: NativeLibrarySearchGuard,
    host_options: Arc<LuaRuntimeHostOptions>,
}

/// Resolved runtime entry target produced after canonical-name collision indexing.
/// 经过 canonical 名称冲突编号后得到的运行时入口目标。
#[derive(Debug, Clone)]
struct ResolvedEntryTarget {
    /// Final canonical tool name exposed to hosts and Lua dispatch.
    /// 暴露给宿主和 Lua 分发器的最终 canonical 工具名。
    canonical_name: String,
    /// Internal storage key of the owning loaded skill.
    /// 所属已加载 skill 的内部存储键。
    skill_storage_key: String,
    /// Owning stable skill identifier declared in skill metadata.
    /// 在 skill 元数据中声明的所属稳定 skill 标识符。
    skill_id: String,
    /// Stable local entry name declared by the owning skill.
    /// 所属 skill 声明的稳定局部入口名称。
    local_name: String,
}

/// Construction options used by the host to create one LuaSkills runtime engine.
/// 宿主创建单个 LuaSkills 运行时引擎时使用的构造选项。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LuaEngineOptions {
    /// Pool sizing configuration for reusable Lua virtual machines.
    /// 可复用 Lua 虚拟机池的容量配置。
    pub pool_config: LuaVmPoolConfig,
    /// Host-owned runtime paths and external library locations.
    /// 宿主拥有的运行时路径与外部动态库位置配置。
    pub host_options: LuaRuntimeHostOptions,
}

impl LuaEngineOptions {
    /// Build engine options from one pool config and one host option object.
    /// 基于一份虚拟机池配置和一份宿主选项对象构造引擎选项。
    pub fn new(pool_config: LuaVmPoolConfig, host_options: LuaRuntimeHostOptions) -> Self {
        Self {
            pool_config,
            host_options,
        }
    }
}

impl LoadedSkill {
    /// Return the resolved canonical entry name for one local entry name.
    /// 返回某个局部入口名称对应的已解析 canonical 名称。
    fn resolved_tool_name(&self, local_name: &str) -> Option<&str> {
        self.resolved_entry_names
            .get(local_name)
            .map(String::as_str)
    }
}

/// Return a stable human-readable Lua value type name.
/// 返回稳定且可读的 Lua 值类型名称。
fn lua_value_type_name(value: &LuaValue) -> &'static str {
    match value {
        LuaValue::Nil => "nil",
        LuaValue::Boolean(_) => "boolean",
        LuaValue::LightUserData(_) => "lightuserdata",
        LuaValue::Integer(_) => "integer",
        LuaValue::Number(_) => "number",
        LuaValue::String(_) => "string",
        LuaValue::Table(_) => "table",
        LuaValue::Function(_) => "function",
        LuaValue::Thread(_) => "thread",
        LuaValue::UserData(_) => "userdata",
        LuaValue::Error(_) => "error",
        LuaValue::Other(_) => "other",
    }
}

/// Read one optional `recursive` flag from one `vulcan.fs.*` options value.
/// 从单个 `vulcan.fs.*` 选项值中读取可选的 `recursive` 标志。
fn parse_vulcan_fs_recursive_option(value: LuaValue, fn_name: &str) -> mlua::Result<bool> {
    match value {
        LuaValue::Nil => Ok(false),
        other => {
            let options = require_table_arg(other, fn_name, "options")?;
            let recursive_value: LuaValue = options.get("recursive")?;
            match recursive_value {
                LuaValue::Nil => Ok(false),
                LuaValue::Boolean(flag) => Ok(flag),
                other => Err(mlua::Error::runtime(format!(
                    "{fn_name}: options.recursive must be a boolean when provided: {}",
                    lua_value_type_name(&other)
                ))),
            }
        }
    }
}

/// Read one optional `overwrite` flag from one `vulcan.fs.copy` options value.
/// 从单个 `vulcan.fs.copy` 选项值中读取可选的 `overwrite` 标志。
fn parse_vulcan_fs_overwrite_option(value: LuaValue, fn_name: &str) -> mlua::Result<bool> {
    match value {
        LuaValue::Nil => Ok(false),
        other => {
            let options = require_table_arg(other, fn_name, "options")?;
            let overwrite_value: LuaValue = options.get("overwrite")?;
            match overwrite_value {
                LuaValue::Nil => Ok(false),
                LuaValue::Boolean(flag) => Ok(flag),
                other => Err(mlua::Error::runtime(format!(
                    "{fn_name}: options.overwrite must be a boolean when provided: {}",
                    lua_value_type_name(&other)
                ))),
            }
        }
    }
}

/// Resolve one `vulcan.fs.copy` path into a normalized absolute path for relationship checks.
/// 将单个 `vulcan.fs.copy` 路径解析为归一化绝对路径，以便做关系校验。
fn resolve_vulcan_fs_copy_absolute_path(path: &Path) -> Result<PathBuf, String> {
    let cwd = std::env::current_dir().map_err(|error| format!("fs.copy: {}", error))?;
    Ok(if path.is_absolute() {
        normalize_runtime_root_path(path)
    } else {
        normalize_runtime_root_path(&cwd.join(path))
    })
}

/// Resolve one `vulcan.fs.copy` destination into the effective absolute location created after existing parent links are followed.
/// 将单个 `vulcan.fs.copy` 目标解析为跟随现有父级链接后最终会创建到的实际绝对位置。
fn resolve_vulcan_fs_copy_effective_destination_path(
    target: &Path,
    recreate_leaf: bool,
) -> Result<PathBuf, String> {
    let absolute_target = resolve_vulcan_fs_copy_absolute_path(target)?;
    let mut suffix = Vec::<PathBuf>::new();
    let mut cursor = if recreate_leaf {
        let leaf_name = absolute_target.file_name().ok_or_else(|| {
            format!(
                "fs.copy: destination path must not be one filesystem root: {}",
                render_log_friendly_path(&absolute_target)
            )
        })?;
        suffix.push(PathBuf::from(leaf_name));
        absolute_target.parent().ok_or_else(|| {
            format!(
                "fs.copy: destination path must have one parent directory: {}",
                render_log_friendly_path(&absolute_target)
            )
        })?
    } else {
        absolute_target.as_path()
    };
    while !cursor.exists() {
        let missing_name = cursor.file_name().ok_or_else(|| {
            format!(
                "fs.copy: destination path could not resolve one existing ancestor: {}",
                render_log_friendly_path(&absolute_target)
            )
        })?;
        suffix.push(PathBuf::from(missing_name));
        cursor = cursor.parent().ok_or_else(|| {
            format!(
                "fs.copy: destination path must stay under one existing filesystem root: {}",
                render_log_friendly_path(&absolute_target)
            )
        })?;
    }
    let mut resolved = fs::canonicalize(cursor).map_err(|error| format!("fs.copy: {}", error))?;
    for component in suffix.into_iter().rev() {
        resolved.push(component);
    }
    Ok(normalize_runtime_root_path(&resolved))
}

/// Validate that one directory-copy destination is neither equal to nor nested under the source directory.
/// 校验单个目录复制目标既不等于源目录，也不位于源目录内部。
fn validate_vulcan_fs_copy_directory_target(
    source: &Path,
    target: &Path,
    recreate_target_leaf: bool,
) -> Result<(), String> {
    let resolved_source =
        fs::canonicalize(source).map_err(|error| format!("fs.copy: {}", error))?;
    let resolved_target =
        resolve_vulcan_fs_copy_effective_destination_path(target, recreate_target_leaf)?;
    if resolved_source == resolved_target {
        return Err(format!(
            "fs.copy: source and destination must differ: {}",
            render_log_friendly_path(&resolved_source)
        ));
    }
    if resolved_target.starts_with(&resolved_source) {
        return Err(format!(
            "fs.copy: destination directory must not be inside source directory: {}",
            render_log_friendly_path(&resolved_target)
        ));
    }
    Ok(())
}

/// Remove one existing `vulcan.fs.copy` destination so overwrite mode can replace it atomically.
/// 删除单个已存在的 `vulcan.fs.copy` 目标，以便 overwrite 模式能够整体替换它。
fn remove_vulcan_fs_copy_target(target: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(target).map_err(|error| format!("fs.copy: {}", error))?;
    let file_type = metadata.file_type();
    if file_type.is_dir() {
        fs::remove_dir_all(target).map_err(|error| format!("fs.copy: {}", error))?;
    } else {
        fs::remove_file(target).map_err(|error| format!("fs.copy: {}", error))?;
    }
    Ok(())
}

/// Detect whether one filesystem entry exists at the path itself even when the target behind one symlink is missing.
/// 判断单个路径位置本身是否存在文件系统条目，即使其背后的符号链接目标已经缺失。
fn path_entry_exists(path: &Path, error_prefix: &str) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("{error_prefix}: {}", error)),
    }
}

/// Recursively copy one directory tree for `vulcan.fs.copy` while rejecting symbolic links for predictable behavior.
/// 为 `vulcan.fs.copy` 递归复制单个目录树，并拒绝符号链接以保证行为可预测。
fn copy_vulcan_fs_directory_recursive(source: &Path, target: &Path) -> Result<(), String> {
    fs::create_dir_all(target).map_err(|error| format!("fs.copy: {}", error))?;
    for entry in fs::read_dir(source).map_err(|error| format!("fs.copy: {}", error))? {
        let entry = entry.map_err(|error| format!("fs.copy: {}", error))?;
        let entry_path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("fs.copy: {}", error))?;
        let destination = target.join(entry.file_name());
        if file_type.is_symlink() {
            return Err(format!(
                "fs.copy: symbolic-link entries are not supported inside directory trees: {}",
                render_log_friendly_path(&entry_path)
            ));
        }
        if file_type.is_dir() {
            copy_vulcan_fs_directory_recursive(&entry_path, &destination)?;
        } else if file_type.is_file() {
            fs::copy(&entry_path, &destination).map_err(|error| format!("fs.copy: {}", error))?;
        } else {
            return Err(format!(
                "fs.copy: unsupported entry type inside directory tree: {}",
                render_log_friendly_path(&entry_path)
            ));
        }
    }
    Ok(())
}

/// Classify one filesystem type into the stable `vulcan.fs.stat` kind strings.
/// 将单个文件系统类型归类为稳定的 `vulcan.fs.stat` kind 字符串。
fn classify_vulcan_fs_kind(file_type: &fs::FileType) -> &'static str {
    if file_type.is_file() {
        "file"
    } else if file_type.is_dir() {
        "dir"
    } else if file_type.is_symlink() {
        "symlink"
    } else {
        "other"
    }
}

/// Convert one metadata modified time into Unix milliseconds when the timestamp is available.
/// 当时间戳可用时，将单个元数据修改时间转换为 Unix 毫秒值。
fn metadata_modified_unix_ms(metadata: &fs::Metadata) -> Option<i64> {
    let modified = metadata.modified().ok()?;
    let duration = modified.duration_since(UNIX_EPOCH).ok()?;
    let millis = duration.as_millis();
    Some(if millis > i64::MAX as u128 {
        i64::MAX
    } else {
        millis as i64
    })
}

/// Build one Lua table for the current `vulcan.fs.stat` metadata snapshot.
/// 为当前 `vulcan.fs.stat` 元数据快照构造一个 Lua table。
fn create_vulcan_fs_stat_table(lua: &Lua, metadata: &fs::Metadata) -> mlua::Result<Table> {
    let file_type = metadata.file_type();
    let stat = lua.create_table()?;
    stat.set("kind", classify_vulcan_fs_kind(&file_type))?;
    stat.set("is_file", file_type.is_file())?;
    stat.set("is_dir", file_type.is_dir())?;
    stat.set("is_symlink", file_type.is_symlink())?;
    stat.set("readonly", metadata.permissions().readonly())?;
    if file_type.is_file() {
        stat.set("size", metadata.len())?;
    }
    if let Some(modified_unix_ms) = metadata_modified_unix_ms(metadata) {
        stat.set("modified_unix_ms", modified_unix_ms)?;
    }
    Ok(stat)
}

/// Render one `vulcan.path.dirname` result with script-friendly fallback semantics.
/// 以适合脚本使用的兜底语义渲染单个 `vulcan.path.dirname` 结果。
fn render_vulcan_path_dirname(path: &Path) -> String {
    match path.parent() {
        Some(parent) if parent.as_os_str().is_empty() => ".".to_string(),
        Some(parent) => render_host_visible_path(parent),
        None if path.is_absolute() => render_host_visible_path(path),
        None => ".".to_string(),
    }
}

/// Render one normalized path string for `vulcan.path.normalize`.
/// 为 `vulcan.path.normalize` 渲染单个规范化后的路径字符串。
fn render_vulcan_normalized_path(path: &Path) -> String {
    let normalized = normalize_runtime_root_path(path);
    if normalized.as_os_str().is_empty() {
        ".".to_string()
    } else {
        render_host_visible_path(&normalized)
    }
}

/// Detect whether one `vulcan.process.which` input should be treated as an explicit path.
/// 判断单个 `vulcan.process.which` 输入是否应按显式路径处理。
fn is_vulcan_process_explicit_path(program: &str) -> bool {
    Path::new(program).is_absolute() || program.contains('/') || program.contains('\\')
}

/// Resolve one possibly relative process-search path against the current working directory.
/// 将单个可能为相对路径的进程搜索路径相对于当前工作目录解析出来。
fn resolve_vulcan_process_search_path(path: &Path, cwd: &Path) -> PathBuf {
    if path.is_absolute() {
        normalize_runtime_root_path(path)
    } else {
        normalize_runtime_root_path(&cwd.join(path))
    }
}

/// Return the ordered PATHEXT list used by Windows process lookup.
/// 返回 Windows 进程查找使用的有序 PATHEXT 列表。
#[cfg(windows)]
fn vulcan_process_windows_pathexts() -> Vec<String> {
    let from_env = std::env::var("PATHEXT").ok().map(|value| {
        value
            .split(';')
            .filter_map(|entry| {
                let trimmed = entry.trim();
                if trimmed.is_empty() {
                    None
                } else if trimmed.starts_with('.') {
                    Some(trimmed.to_ascii_lowercase())
                } else {
                    Some(format!(".{}", trimmed).to_ascii_lowercase())
                }
            })
            .collect::<Vec<_>>()
    });
    let pathexts = from_env.unwrap_or_default();
    if pathexts.is_empty() {
        vec![
            ".com".to_string(),
            ".exe".to_string(),
            ".bat".to_string(),
            ".cmd".to_string(),
        ]
    } else {
        pathexts
    }
}

/// Expand one process-search base path into platform-specific executable candidates.
/// 将单个进程搜索基路径展开为平台相关的可执行候选路径列表。
#[cfg(windows)]
fn vulcan_process_candidate_paths(base: &Path) -> Vec<PathBuf> {
    let mut candidates = vec![base.to_path_buf()];
    if base.extension().is_some() {
        return candidates;
    }
    let base_text = base.as_os_str().to_string_lossy().to_string();
    for ext in vulcan_process_windows_pathexts() {
        candidates.push(PathBuf::from(format!("{base_text}{ext}")));
    }
    candidates
}

/// Expand one process-search base path into platform-specific executable candidates.
/// 将单个进程搜索基路径展开为平台相关的可执行候选路径列表。
#[cfg(not(windows))]
fn vulcan_process_candidate_paths(base: &Path) -> Vec<PathBuf> {
    vec![base.to_path_buf()]
}

/// Check whether one candidate path is executable on the current platform.
/// 检查单个候选路径在当前平台上是否可执行。
#[cfg(unix)]
fn is_vulcan_process_executable(path: &Path) -> bool {
    fs::metadata(path)
        .map(|metadata| metadata.is_file() && (metadata.permissions().mode() & 0o111) != 0)
        .unwrap_or(false)
}

/// Check whether one candidate path is executable on the current platform.
/// 检查单个候选路径在当前平台上是否可执行。
#[cfg(windows)]
fn is_vulcan_process_executable(path: &Path) -> bool {
    fs::metadata(path)
        .map(|metadata| metadata.is_file())
        .unwrap_or(false)
}

/// Check whether one candidate path is executable on the current platform.
/// 检查单个候选路径在当前平台上是否可执行。
#[cfg(not(any(unix, windows)))]
fn is_vulcan_process_executable(path: &Path) -> bool {
    fs::metadata(path)
        .map(|metadata| metadata.is_file())
        .unwrap_or(false)
}

/// Find the first executable candidate derived from one base process path.
/// 从单个基础进程路径派生的候选项中找出第一个可执行目标。
fn find_vulcan_process_candidate(base: &Path) -> Option<PathBuf> {
    vulcan_process_candidate_paths(base)
        .into_iter()
        .find(|candidate| is_vulcan_process_executable(candidate))
}

/// Resolve one program name into a host-visible executable path using PATH-like lookup semantics.
/// 使用类 PATH 的查找语义，将单个程序名解析为宿主可见的可执行路径。
fn resolve_vulcan_process_which(program: &str) -> Result<Option<PathBuf>, String> {
    let cwd = std::env::current_dir().map_err(|error| format!("process.which: {}", error))?;
    if is_vulcan_process_explicit_path(program) {
        let explicit_path = resolve_vulcan_process_search_path(Path::new(program), &cwd);
        return Ok(find_vulcan_process_candidate(&explicit_path));
    }
    let Some(path_env) = std::env::var_os("PATH") else {
        return Ok(None);
    };
    for search_dir in std::env::split_paths(&path_env) {
        let resolved_dir = resolve_vulcan_process_search_path(&search_dir, &cwd);
        let base = resolved_dir.join(program);
        if let Some(found) = find_vulcan_process_candidate(&base) {
            return Ok(Some(found));
        }
    }
    Ok(None)
}

/// Validate one relative metadata path against a fixed prefix and reject traversal.
/// 按固定目录前缀校验单个 skill 元数据相对路径，并拒绝路径穿越。
fn validate_skill_relative_path(
    relative_path: &str,
    expected_prefix: &str,
    field_label: &str,
) -> Result<(), String> {
    let trimmed = relative_path.trim();
    if trimmed.is_empty() {
        return Err(format!("{field_label} must not be empty"));
    }

    let path = Path::new(trimmed);
    if path.is_absolute() {
        return Err(format!(
            "{field_label} must be a relative path under {expected_prefix}"
        ));
    }

    let normalized = trimmed.replace('\\', "/");
    let required_prefix = format!("{expected_prefix}/");
    if !normalized.starts_with(&required_prefix) {
        return Err(format!("{field_label} must start with {required_prefix}"));
    }

    for component in path.components() {
        if !matches!(component, std::path::Component::Normal(_)) {
            return Err(format!("{field_label} must not contain parent"));
        }
    }

    Ok(())
}

/// Validate one discovered skill directory name against the strict LuaSkills rule.
/// 按严格 LuaSkills 规则校验一个被发现的 skill 目录名。
/// Build the absolute Lua entry file path for a tool.
/// 构建工具 Lua 入口文件的绝对路径。
fn tool_entry_path(skill_dir: &Path, tool: &crate::lua_skill::SkillToolMeta) -> PathBuf {
    skill_dir.join(&tool.lua_entry)
}

/// Internal per-VM Vulcan execution markers used for tool dispatch guards.
/// 用于工具分发保护的每个虚拟机内部 Vulcan 执行标记。
#[derive(Debug, Clone, Default)]
struct VulcanInternalExecutionContext {
    /// Current tool name executing inside this Lua VM.
    /// 当前 Lua 虚拟机内正在执行的工具名称。
    tool_name: Option<String>,
    /// Current owner skill name executing inside this Lua VM.
    /// 当前 Lua 虚拟机内正在执行的所属 skill 名称。
    skill_name: Option<String>,
    /// Current local entry name executing inside this Lua VM.
    /// 当前 Lua 虚拟机内正在执行的局部入口名称。
    entry_name: Option<String>,
    /// Current runtime root name that owns the executing skill.
    /// 拥有当前执行 skill 的运行时根名称。
    root_name: Option<String>,
    /// Whether the current Lua VM is the isolated luaexec runtime environment.
    /// 当前 Lua 虚拟机是否处于隔离的 luaexec 运行环境。
    luaexec_active: bool,
    /// Original tool name that launched the current luaexec request.
    /// 发起当前 luaexec 请求的原始工具名称。
    luaexec_caller_tool_name: Option<String>,
}

/// Capture the current Lua entry context stored on `vulcan`.
/// 捕获当前存放在 `vulcan` 上的 Lua 入口文件上下文。
fn capture_vulcan_file_context(
    lua: &Lua,
) -> Result<(Option<String>, Option<String>, Option<String>), String> {
    let context = get_vulcan_context_table(lua)?;
    let skill_dir: Option<String> = context
        .get("skill_dir")
        .map_err(|error| format!("Failed to read vulcan.context.skill_dir: {}", error))?;
    let entry_dir: Option<String> = context
        .get("entry_dir")
        .map_err(|error| format!("Failed to read vulcan.context.entry_dir: {}", error))?;
    let entry_file: Option<String> = context
        .get("entry_file")
        .map_err(|error| format!("Failed to read vulcan.context.entry_file: {}", error))?;
    Ok((skill_dir, entry_dir, entry_file))
}

/// Populate the current skill directory, entry directory, and entry file onto `vulcan`.
/// 将当前 skill 目录、入口目录与入口文件路径注入到 `vulcan` 模块。
fn populate_vulcan_file_context(
    lua: &Lua,
    skill_dir: Option<&Path>,
    entry_file: Option<&Path>,
) -> Result<(), String> {
    let context = get_vulcan_context_table(lua)?;

    match skill_dir {
        Some(path) => context
            .set("skill_dir", render_host_visible_path(path))
            .map_err(|error| format!("Failed to set vulcan.context.skill_dir: {}", error))?,
        None => context
            .set("skill_dir", LuaValue::Nil)
            .map_err(|error| format!("Failed to clear vulcan.context.skill_dir: {}", error))?,
    }

    match entry_file {
        Some(path) => {
            let entry_dir = path.parent().unwrap_or(path);
            context
                .set("entry_dir", render_host_visible_path(entry_dir))
                .map_err(|error| format!("Failed to set vulcan.context.entry_dir: {}", error))?;
            context
                .set("entry_file", render_host_visible_path(path))
                .map_err(|error| format!("Failed to set vulcan.context.entry_file: {}", error))?;
        }
        None => {
            context
                .set("entry_dir", LuaValue::Nil)
                .map_err(|error| format!("Failed to clear vulcan.context.entry_dir: {}", error))?;
            context
                .set("entry_file", LuaValue::Nil)
                .map_err(|error| format!("Failed to clear vulcan.context.entry_file: {}", error))?;
        }
    }

    Ok(())
}

/// Populate the current skill dependency roots onto `vulcan.deps`.
/// 将当前技能依赖根路径注入到 `vulcan.deps` 中。
fn populate_vulcan_dependency_context(
    lua: &Lua,
    host_options: &LuaRuntimeHostOptions,
    skill_dir: Option<&Path>,
    skill_id: Option<&str>,
) -> Result<(), String> {
    let deps = get_vulcan_deps_table(lua)?;

    let clear_paths = || -> Result<(), String> {
        deps.set("tools_path", LuaValue::Nil)
            .map_err(|error| format!("Failed to clear vulcan.deps.tools_path: {}", error))?;
        deps.set("lua_path", LuaValue::Nil)
            .map_err(|error| format!("Failed to clear vulcan.deps.lua_path: {}", error))?;
        deps.set("ffi_path", LuaValue::Nil)
            .map_err(|error| format!("Failed to clear vulcan.deps.ffi_path: {}", error))?;
        Ok(())
    };

    let Some(skill_dir) = skill_dir else {
        return clear_paths();
    };
    let Some(skill_id) = skill_id.filter(|value| !value.trim().is_empty()) else {
        return clear_paths();
    };

    let skills_root = skill_dir.parent().ok_or_else(|| {
        format!(
            "Failed to derive skills root from skill directory {}",
            skill_dir.display()
        )
    })?;
    let runtime_root = skills_root.parent().ok_or_else(|| {
        format!(
            "Failed to derive runtime root from skill directory {}",
            skill_dir.display()
        )
    })?;
    let dependency_root = runtime_root.join(host_options.dependency_dir_name.as_str());

    deps.set(
        "tools_path",
        render_host_visible_path(&dependency_root.join("tools").join(skill_id)),
    )
    .map_err(|error| format!("Failed to set vulcan.deps.tools_path: {}", error))?;
    deps.set(
        "lua_path",
        render_host_visible_path(&dependency_root.join("lua").join(skill_id)),
    )
    .map_err(|error| format!("Failed to set vulcan.deps.lua_path: {}", error))?;
    deps.set(
        "ffi_path",
        render_host_visible_path(&dependency_root.join("ffi").join(skill_id)),
    )
    .map_err(|error| format!("Failed to set vulcan.deps.ffi_path: {}", error))?;
    Ok(())
}

/// Capture the internal execution markers currently stored on `vulcan`.
/// 捕获当前存放在 `vulcan` 上的内部执行标记。
fn capture_vulcan_internal_execution_context(
    lua: &Lua,
) -> Result<VulcanInternalExecutionContext, String> {
    let internal = get_vulcan_runtime_internal_table(lua)?;
    let tool_name: Option<String> = internal.get("tool_name").map_err(|error| {
        format!(
            "Failed to read vulcan.runtime.internal.tool_name: {}",
            error
        )
    })?;
    let skill_name: Option<String> = internal.get("skill_name").map_err(|error| {
        format!(
            "Failed to read vulcan.runtime.internal.skill_name: {}",
            error
        )
    })?;
    let entry_name: Option<String> = internal.get("entry_name").map_err(|error| {
        format!(
            "Failed to read vulcan.runtime.internal.entry_name: {}",
            error
        )
    })?;
    let root_name: Option<String> = internal.get("root_name").map_err(|error| {
        format!(
            "Failed to read vulcan.runtime.internal.root_name: {}",
            error
        )
    })?;
    let luaexec_active: bool = internal.get("luaexec_active").map_err(|error| {
        format!(
            "Failed to read vulcan.runtime.internal.luaexec_active: {}",
            error
        )
    })?;
    let luaexec_caller_tool_name: Option<String> =
        internal.get("luaexec_caller_tool_name").map_err(|error| {
            format!(
                "Failed to read vulcan.runtime.internal.luaexec_caller_tool_name: {}",
                error
            )
        })?;
    Ok(VulcanInternalExecutionContext {
        tool_name,
        skill_name,
        entry_name,
        root_name,
        luaexec_active,
        luaexec_caller_tool_name,
    })
}

/// Populate the internal execution markers stored on `vulcan`.
/// 填充存放在 `vulcan` 上的内部执行标记。
fn populate_vulcan_internal_execution_context(
    lua: &Lua,
    context: &VulcanInternalExecutionContext,
) -> Result<(), String> {
    let internal = get_vulcan_runtime_internal_table(lua)?;

    match context.tool_name.as_deref() {
        Some(tool_name) => internal.set("tool_name", tool_name).map_err(|error| {
            format!("Failed to set vulcan.runtime.internal.tool_name: {}", error)
        })?,
        None => internal.set("tool_name", LuaValue::Nil).map_err(|error| {
            format!(
                "Failed to clear vulcan.runtime.internal.tool_name: {}",
                error
            )
        })?,
    }

    match context.skill_name.as_deref() {
        Some(skill_name) => internal.set("skill_name", skill_name).map_err(|error| {
            format!(
                "Failed to set vulcan.runtime.internal.skill_name: {}",
                error
            )
        })?,
        None => internal.set("skill_name", LuaValue::Nil).map_err(|error| {
            format!(
                "Failed to clear vulcan.runtime.internal.skill_name: {}",
                error
            )
        })?,
    }

    match context.entry_name.as_deref() {
        Some(entry_name) => internal.set("entry_name", entry_name).map_err(|error| {
            format!(
                "Failed to set vulcan.runtime.internal.entry_name: {}",
                error
            )
        })?,
        None => internal.set("entry_name", LuaValue::Nil).map_err(|error| {
            format!(
                "Failed to clear vulcan.runtime.internal.entry_name: {}",
                error
            )
        })?,
    }

    match context.root_name.as_deref() {
        Some(root_name) => internal.set("root_name", root_name).map_err(|error| {
            format!("Failed to set vulcan.runtime.internal.root_name: {}", error)
        })?,
        None => internal.set("root_name", LuaValue::Nil).map_err(|error| {
            format!(
                "Failed to clear vulcan.runtime.internal.root_name: {}",
                error
            )
        })?,
    }

    internal
        .set("luaexec_active", context.luaexec_active)
        .map_err(|error| {
            format!(
                "Failed to set vulcan.runtime.internal.luaexec_active: {}",
                error
            )
        })?;

    match context.luaexec_caller_tool_name.as_deref() {
        Some(tool_name) => internal
            .set("luaexec_caller_tool_name", tool_name)
            .map_err(|error| {
                format!(
                    "Failed to set vulcan.runtime.internal.luaexec_caller_tool_name: {}",
                    error
                )
            })?,
        None => internal
            .set("luaexec_caller_tool_name", LuaValue::Nil)
            .map_err(|error| {
                format!(
                    "Failed to clear vulcan.runtime.internal.luaexec_caller_tool_name: {}",
                    error
                )
            })?,
    }

    Ok(())
}

/// Resolve the active skill identifier currently stored in the internal `vulcan` execution context.
/// 解析当前存储在内部 `vulcan` 执行上下文中的活动技能标识符。
fn current_vulcan_config_skill_id(lua: &Lua, api_name: &str) -> Result<String, mlua::Error> {
    let internal = get_vulcan_runtime_internal_table(lua)
        .map_err(|error| mlua::Error::runtime(format!("{}: {}", api_name, error)))?;
    let skill_name: Option<String> = internal
        .get("skill_name")
        .map_err(|error| mlua::Error::runtime(format!("{}: {}", api_name, error)))?;
    skill_name
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            mlua::Error::runtime(format!("{} requires one active skill context", api_name))
        })
}

/// Return whether one help payload should be executed as Lua instead of read as plain text.
/// 判断某个帮助载荷是否应按 Lua 执行，而不是按纯文本读取。
fn is_lua_help_file(relative_path: &str) -> bool {
    Path::new(relative_path)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("lua"))
        .unwrap_or(false)
}

/// Read one UTF-8 text file relative to the skill directory.
/// 读取相对于 skill 目录的一份 UTF-8 文本文件。
fn read_skill_text_file(
    skill_dir: &Path,
    relative_path: &str,
    label: &str,
) -> Result<String, String> {
    let file_path = skill_dir.join(relative_path);
    std::fs::read_to_string(&file_path).map_err(|error| {
        format!(
            "Failed to read {label} file {}: {}",
            file_path.display(),
            error
        )
    })
}

/// Return the root `vulcan` Lua table.
/// 返回根级 `vulcan` Lua 表。
fn get_vulcan_table(lua: &Lua) -> Result<Table, String> {
    lua.globals()
        .get("vulcan")
        .map_err(|error| format!("Failed to get vulcan module: {}", error))
}

/// Return the nested `vulcan.context` Lua table.
/// 返回嵌套的 `vulcan.context` Lua 表。
fn get_vulcan_context_table(lua: &Lua) -> Result<Table, String> {
    let vulcan = get_vulcan_table(lua)?;
    vulcan
        .get("context")
        .map_err(|error| format!("Failed to get vulcan.context: {}", error))
}

/// Return the nested `vulcan.deps` Lua table.
/// 返回嵌套的 `vulcan.deps` Lua 表。
fn get_vulcan_deps_table(lua: &Lua) -> Result<Table, String> {
    let vulcan = get_vulcan_table(lua)?;
    vulcan
        .get("deps")
        .map_err(|error| format!("Failed to get vulcan.deps: {}", error))
}

/// Return the nested `vulcan.runtime` Lua table.
/// 返回嵌套的 `vulcan.runtime` Lua 表。
fn get_vulcan_runtime_table(lua: &Lua) -> Result<Table, String> {
    let vulcan = get_vulcan_table(lua)?;
    vulcan
        .get("runtime")
        .map_err(|error| format!("Failed to get vulcan.runtime: {}", error))
}

/// Return the nested `vulcan.runtime.internal` Lua table.
/// 返回嵌套的 `vulcan.runtime.internal` Lua 表。
fn get_vulcan_runtime_internal_table(lua: &Lua) -> Result<Table, String> {
    let runtime = get_vulcan_runtime_table(lua)?;
    runtime
        .get("internal")
        .map_err(|error| format!("Failed to get vulcan.runtime.internal: {}", error))
}

/// Return the nested `vulcan.runtime.lua` Lua table.
/// 返回嵌套的 `vulcan.runtime.lua` Lua 表。
fn get_vulcan_runtime_lua_table(lua: &Lua) -> Result<Table, String> {
    let runtime = get_vulcan_runtime_table(lua)?;
    runtime
        .get("lua")
        .map_err(|error| format!("Failed to get vulcan.runtime.lua: {}", error))
}

/// Snapshot of the mutable core `vulcan` tables that must survive nested-call failures.
/// 会在嵌套调用失败后恢复的 `vulcan` 可变核心表快照。
#[derive(Clone)]
struct VulcanCoreModuleState {
    vulcan: Table,
    call: Function,
    runtime: Table,
    runtime_skills: Table,
    runtime_internal: Table,
    runtime_lua: Table,
    fs: Table,
    io: Table,
    path: Table,
    process: Table,
    os: Table,
    json: Table,
    cache: Table,
    context: Table,
    deps: Table,
    models: Table,
}

impl VulcanCoreModuleState {
    /// Capture the currently installed `vulcan` root tables before one nested skill call mutates them.
    /// 在一次嵌套技能调用可能修改它们之前，捕获当前安装好的 `vulcan` 根表结构。
    fn capture(lua: &Lua) -> Result<Self, String> {
        let vulcan = get_vulcan_table(lua)?;
        let runtime = get_vulcan_runtime_table(lua)?;
        Ok(Self {
            call: vulcan
                .get("call")
                .map_err(|error| format!("Failed to get vulcan.call: {}", error))?,
            runtime_skills: runtime
                .get("skills")
                .map_err(|error| format!("Failed to get vulcan.runtime.skills: {}", error))?,
            runtime_internal: runtime
                .get("internal")
                .map_err(|error| format!("Failed to get vulcan.runtime.internal: {}", error))?,
            runtime_lua: runtime
                .get("lua")
                .map_err(|error| format!("Failed to get vulcan.runtime.lua: {}", error))?,
            fs: vulcan
                .get("fs")
                .map_err(|error| format!("Failed to get vulcan.fs: {}", error))?,
            io: vulcan
                .get("io")
                .map_err(|error| format!("Failed to get vulcan.io: {}", error))?,
            path: vulcan
                .get("path")
                .map_err(|error| format!("Failed to get vulcan.path: {}", error))?,
            process: vulcan
                .get("process")
                .map_err(|error| format!("Failed to get vulcan.process: {}", error))?,
            os: vulcan
                .get("os")
                .map_err(|error| format!("Failed to get vulcan.os: {}", error))?,
            json: vulcan
                .get("json")
                .map_err(|error| format!("Failed to get vulcan.json: {}", error))?,
            cache: vulcan
                .get("cache")
                .map_err(|error| format!("Failed to get vulcan.cache: {}", error))?,
            models: vulcan
                .get("models")
                .map_err(|error| format!("Failed to get vulcan.models: {}", error))?,
            context: vulcan
                .get("context")
                .map_err(|error| format!("Failed to get vulcan.context: {}", error))?,
            deps: vulcan
                .get("deps")
                .map_err(|error| format!("Failed to get vulcan.deps: {}", error))?,
            vulcan,
            runtime,
        })
    }

    /// Reinstall the captured `vulcan` core table topology after one nested call corrupts it.
    /// 在嵌套调用破坏表结构后，重新安装捕获到的 `vulcan` 核心表拓扑。
    fn restore(&self, lua: &Lua) -> Result<(), String> {
        self.runtime
            .set("skills", self.runtime_skills.clone())
            .map_err(|error| format!("Failed to restore vulcan.runtime.skills: {}", error))?;
        self.runtime
            .set("internal", self.runtime_internal.clone())
            .map_err(|error| format!("Failed to restore vulcan.runtime.internal: {}", error))?;
        self.runtime
            .set("lua", self.runtime_lua.clone())
            .map_err(|error| format!("Failed to restore vulcan.runtime.lua: {}", error))?;
        self.vulcan
            .set("call", self.call.clone())
            .map_err(|error| format!("Failed to restore vulcan.call: {}", error))?;
        self.vulcan
            .set("runtime", self.runtime.clone())
            .map_err(|error| format!("Failed to restore vulcan.runtime: {}", error))?;
        self.vulcan
            .set("fs", self.fs.clone())
            .map_err(|error| format!("Failed to restore vulcan.fs: {}", error))?;
        self.vulcan
            .set("io", self.io.clone())
            .map_err(|error| format!("Failed to restore vulcan.io: {}", error))?;
        self.vulcan
            .set("path", self.path.clone())
            .map_err(|error| format!("Failed to restore vulcan.path: {}", error))?;
        self.vulcan
            .set("process", self.process.clone())
            .map_err(|error| format!("Failed to restore vulcan.process: {}", error))?;
        self.vulcan
            .set("os", self.os.clone())
            .map_err(|error| format!("Failed to restore vulcan.os: {}", error))?;
        self.vulcan
            .set("json", self.json.clone())
            .map_err(|error| format!("Failed to restore vulcan.json: {}", error))?;
        self.vulcan
            .set("cache", self.cache.clone())
            .map_err(|error| format!("Failed to restore vulcan.cache: {}", error))?;
        self.vulcan
            .set("models", self.models.clone())
            .map_err(|error| format!("Failed to restore vulcan.models: {}", error))?;
        self.vulcan
            .set("context", self.context.clone())
            .map_err(|error| format!("Failed to restore vulcan.context: {}", error))?;
        self.vulcan
            .set("deps", self.deps.clone())
            .map_err(|error| format!("Failed to restore vulcan.deps: {}", error))?;
        lua.globals()
            .set("vulcan", self.vulcan.clone())
            .map_err(|error| format!("Failed to restore global vulcan module: {}", error))?;
        Ok(())
    }
}

/// Return the non-empty skill identifier string when the captured value is usable.
/// 当捕获到的技能标识可用时，返回其非空字符串引用。
fn non_empty_skill_name(value: &str) -> Option<&str> {
    if value.trim().is_empty() {
        None
    } else {
        Some(value)
    }
}

/// Clear the transient `__runlua_args` global used by pooled VM requests.
/// 清理池化虚拟机请求期间使用的临时 `__runlua_args` 全局变量。
fn clear_runlua_args_global(lua: &Lua) -> Result<(), String> {
    lua.globals()
        .set("__runlua_args", LuaValue::Nil)
        .map_err(|error| format!("Failed to clear __runlua_args: {}", error))
}

/// Reset one pooled Lua VM back to the neutral per-request baseline.
/// 将单个池化 Lua 虚拟机重置回中性的单次请求基线状态。
fn reset_pooled_vm_request_scope(
    lua: &Lua,
    host_options: &LuaRuntimeHostOptions,
) -> Result<(), String> {
    LuaEngine::populate_vulcan_request_context(lua, None)?;
    populate_vulcan_internal_execution_context(lua, &VulcanInternalExecutionContext::default())?;
    populate_vulcan_file_context(lua, None, None)?;
    populate_vulcan_dependency_context(lua, host_options, None, None)?;
    LuaEngine::populate_vulcan_lancedb_context(lua, None, None)?;
    LuaEngine::populate_vulcan_sqlite_context(lua, None, None)?;
    clear_runlua_args_global(lua)?;
    Ok(())
}

/// One RAII guard that keeps pooled Lua VM request-scoped state isolated.
/// 一个用于保持池化 Lua 虚拟机请求级状态隔离的 RAII 守卫。
struct LuaVmRequestScopeGuard<'a> {
    lease: &'a mut LuaVmLease,
    host_options: &'a LuaRuntimeHostOptions,
    active: bool,
}

impl<'a> LuaVmRequestScopeGuard<'a> {
    /// Normalize one pooled VM before use and arm cleanup for all exit paths.
    /// 在使用前归一化单个池化虚拟机，并为全部退出路径启用清理保护。
    fn new(
        lease: &'a mut LuaVmLease,
        host_options: &'a LuaRuntimeHostOptions,
    ) -> Result<Self, String> {
        let mut guard = Self {
            lease,
            host_options,
            active: true,
        };
        if let Err(error) = reset_pooled_vm_request_scope(guard.lua(), host_options) {
            guard.lease.discard();
            guard.active = false;
            return Err(error);
        }
        Ok(guard)
    }

    /// Borrow the guarded Lua VM while the request scope is active.
    /// 在请求作用域激活期间借用受守卫保护的 Lua 虚拟机。
    fn lua(&self) -> &Lua {
        self.lease.lua()
    }

    /// Explicitly finish the request scope and surface cleanup errors to the caller.
    /// 显式结束当前请求作用域，并将清理错误返回给调用方。
    fn finish(mut self) -> Result<(), String> {
        let cleanup_result = reset_pooled_vm_request_scope(self.lua(), self.host_options);
        if let Err(error) = cleanup_result {
            self.lease.discard();
            self.active = false;
            return Err(error);
        }
        self.active = false;
        Ok(())
    }
}

impl Drop for LuaVmRequestScopeGuard<'_> {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        if let Err(error) = reset_pooled_vm_request_scope(self.lua(), self.host_options) {
            log_error(format!(
                "[LuaSkill:error] Failed to reset pooled Lua VM request scope: {}",
                error
            ));
            self.lease.discard();
        }
    }
}

/// RAII guard that restores the outer `vulcan` execution context after one nested `vulcan.call`.
/// 在一次嵌套 `vulcan.call` 之后恢复外层 `vulcan` 执行上下文的 RAII 守卫。
struct LuaNestedCallScopeGuard {
    lua: Lua,
    host_options: Arc<LuaRuntimeHostOptions>,
    lancedb_host: Option<Arc<LanceDbSkillHost>>,
    sqlite_host: Option<Arc<SqliteSkillHost>>,
    core_state: VulcanCoreModuleState,
    previous_context: LuaValue,
    previous_client_info: LuaValue,
    previous_client_capabilities: LuaValue,
    previous_client_budget: LuaValue,
    previous_tool_config: LuaValue,
    previous_host_result: LuaValue,
    previous_lancedb_skill_name: String,
    previous_sqlite_skill_name: String,
    previous_internal_context: VulcanInternalExecutionContext,
    previous_file_context: (Option<String>, Option<String>, Option<String>),
    active: bool,
}

impl LuaNestedCallScopeGuard {
    /// Capture the current outer `vulcan` execution state before entering one nested skill.
    /// 在进入一次嵌套技能调用之前捕获当前外层 `vulcan` 执行状态。
    fn new(
        lua: &Lua,
        host_options: Arc<LuaRuntimeHostOptions>,
        lancedb_host: Option<Arc<LanceDbSkillHost>>,
        sqlite_host: Option<Arc<SqliteSkillHost>>,
    ) -> Result<Self, String> {
        let vulcan = get_vulcan_table(lua)?;
        let context_table = get_vulcan_context_table(lua)?;
        Ok(Self {
            lua: lua.clone(),
            host_options,
            lancedb_host,
            sqlite_host,
            core_state: VulcanCoreModuleState::capture(lua)?,
            previous_context: context_table
                .get("request")
                .map_err(|error| format!("Failed to read vulcan.context.request: {}", error))?,
            previous_client_info: context_table
                .get("client_info")
                .map_err(|error| format!("Failed to read vulcan.context.client_info: {}", error))?,
            previous_client_capabilities: context_table.get("client_capabilities").map_err(
                |error| {
                    format!(
                        "Failed to read vulcan.context.client_capabilities: {}",
                        error
                    )
                },
            )?,
            previous_client_budget: context_table.get("client_budget").map_err(|error| {
                format!("Failed to read vulcan.context.client_budget: {}", error)
            })?,
            previous_tool_config: context_table
                .get("tool_config")
                .map_err(|error| format!("Failed to read vulcan.context.tool_config: {}", error))?,
            previous_host_result: context_table
                .get("host_result")
                .map_err(|error| format!("Failed to read vulcan.context.host_result: {}", error))?,
            previous_lancedb_skill_name: vulcan.get("__lancedb_skill_name").unwrap_or_default(),
            previous_sqlite_skill_name: vulcan.get("__sqlite_skill_name").unwrap_or_default(),
            previous_internal_context: capture_vulcan_internal_execution_context(lua)?,
            previous_file_context: capture_vulcan_file_context(lua)?,
            active: true,
        })
    }

    /// Switch the current Lua VM into the nested skill request context.
    /// 把当前 Lua 虚拟机切换到嵌套技能请求上下文。
    fn enter_nested_call(
        &self,
        dispatch_entry_display_name: &str,
        owner_skill_name: &str,
        owner_local_name: &str,
        owner_root_name: &str,
        owner_skill_dir: &str,
        entry_path: &str,
        nested_invocation_context: &LuaInvocationContext,
        target_lancedb_binding: Option<Arc<LanceDbSkillBinding>>,
        target_sqlite_binding: Option<Arc<SqliteSkillBinding>>,
    ) -> Result<(), String> {
        LuaEngine::populate_vulcan_request_context(&self.lua, Some(nested_invocation_context))?;
        populate_vulcan_internal_execution_context(
            &self.lua,
            &VulcanInternalExecutionContext {
                tool_name: Some(dispatch_entry_display_name.to_string()),
                skill_name: Some(owner_skill_name.to_string()),
                entry_name: Some(owner_local_name.to_string()),
                root_name: Some(owner_root_name.to_string()),
                luaexec_active: self.previous_internal_context.luaexec_active,
                luaexec_caller_tool_name: self
                    .previous_internal_context
                    .luaexec_caller_tool_name
                    .clone(),
            },
        )?;
        populate_vulcan_file_context(
            &self.lua,
            Some(Path::new(owner_skill_dir)),
            Some(Path::new(entry_path)),
        )?;
        populate_vulcan_dependency_context(
            &self.lua,
            self.host_options.as_ref(),
            Some(Path::new(owner_skill_dir)),
            Some(owner_skill_name),
        )?;
        LuaEngine::populate_vulcan_lancedb_context(
            &self.lua,
            target_lancedb_binding,
            Some(owner_skill_name),
        )?;
        LuaEngine::populate_vulcan_sqlite_context(
            &self.lua,
            target_sqlite_binding,
            Some(owner_skill_name),
        )?;
        Ok(())
    }

    /// Restore the outer `vulcan` execution state captured before the nested call began.
    /// 恢复嵌套调用开始前捕获到的外层 `vulcan` 执行状态。
    fn restore_previous_state(&self) -> Result<(), String> {
        self.core_state.restore(&self.lua)?;
        let restore_lancedb_binding = match non_empty_skill_name(&self.previous_lancedb_skill_name)
        {
            Some(skill_name) => self
                .lancedb_host
                .as_ref()
                .map(|host| host.binding_for_skill(skill_name))
                .transpose()?
                .flatten(),
            None => None,
        };
        let restore_sqlite_binding = match non_empty_skill_name(&self.previous_sqlite_skill_name) {
            Some(skill_name) => self
                .sqlite_host
                .as_ref()
                .map(|host| host.binding_for_skill(skill_name))
                .transpose()?
                .flatten(),
            None => None,
        };
        LuaEngine::populate_vulcan_lancedb_context(
            &self.lua,
            restore_lancedb_binding,
            non_empty_skill_name(&self.previous_lancedb_skill_name),
        )?;
        LuaEngine::populate_vulcan_sqlite_context(
            &self.lua,
            restore_sqlite_binding,
            non_empty_skill_name(&self.previous_sqlite_skill_name),
        )?;
        let context_table = get_vulcan_context_table(&self.lua)?;
        context_table
            .set("request", self.previous_context.clone())
            .map_err(|error| format!("Failed to restore vulcan.context.request: {}", error))?;
        context_table
            .set("client_info", self.previous_client_info.clone())
            .map_err(|error| format!("Failed to restore vulcan.context.client_info: {}", error))?;
        context_table
            .set(
                "client_capabilities",
                self.previous_client_capabilities.clone(),
            )
            .map_err(|error| {
                format!(
                    "Failed to restore vulcan.context.client_capabilities: {}",
                    error
                )
            })?;
        context_table
            .set("client_budget", self.previous_client_budget.clone())
            .map_err(|error| {
                format!("Failed to restore vulcan.context.client_budget: {}", error)
            })?;
        context_table
            .set("tool_config", self.previous_tool_config.clone())
            .map_err(|error| format!("Failed to restore vulcan.context.tool_config: {}", error))?;
        context_table
            .set("host_result", self.previous_host_result.clone())
            .map_err(|error| format!("Failed to restore vulcan.context.host_result: {}", error))?;
        populate_vulcan_internal_execution_context(&self.lua, &self.previous_internal_context)?;
        populate_vulcan_file_context(
            &self.lua,
            self.previous_file_context.0.as_deref().map(Path::new),
            self.previous_file_context.2.as_deref().map(Path::new),
        )?;
        populate_vulcan_dependency_context(
            &self.lua,
            self.host_options.as_ref(),
            self.previous_file_context.0.as_deref().map(Path::new),
            self.previous_internal_context.skill_name.as_deref(),
        )?;
        Ok(())
    }

    /// Explicitly finish the nested-call scope and surface any restore failure to the caller.
    /// 显式结束嵌套调用作用域，并把恢复失败信息返回给调用方。
    fn finish(mut self) -> Result<(), String> {
        let restore_result = self.restore_previous_state();
        self.active = false;
        restore_result
    }
}

impl Drop for LuaNestedCallScopeGuard {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        if let Err(error) = self.restore_previous_state() {
            log_error(format!(
                "[LuaSkill:error] Failed to restore nested vulcan.call context: {}",
                error
            ));
        }
    }
}

/// Checked-out VM guard that returns the VM back into the pool on drop.
/// 已借出的虚拟机守卫，在释放时会自动归还到池中。
struct LuaVmLease {
    pool: Arc<LuaVmPool>,
    vm: Option<LuaVm>,
}

impl LuaVmLease {
    /// Borrow the underlying Lua VM immutably for the duration of the lease.
    /// 在租约生命周期内以只读方式借用底层 Lua 虚拟机。
    fn lua(&self) -> &Lua {
        &self.vm.as_ref().expect("lua vm lease missing instance").lua
    }

    /// Permanently retire the currently leased VM instead of returning it to the pool.
    /// 永久淘汰当前租出的虚拟机，而不是把它放回池中。
    fn discard(&mut self) {
        if let Some(vm) = self.vm.take() {
            self.pool.discard(vm);
        }
    }
}

impl Drop for LuaVmLease {
    fn drop(&mut self) {
        if let Some(mut vm) = self.vm.take() {
            vm.last_used_at = Instant::now();
            self.pool.release(vm);
        }
    }
}

impl LuaVmPool {
    /// Create a new empty Lua VM pool.
    /// 创建一个新的空 Lua 虚拟机池。
    fn new(config: LuaVmPoolConfig) -> Self {
        Self {
            config: config.normalized(),
            state: Mutex::new(LuaVmPoolState {
                available: Vec::new(),
                total_count: 0,
            }),
            condvar: Condvar::new(),
        }
    }

    /// Prewarm the pool to the configured minimum size.
    /// 预热到配置要求的最小虚拟机数量。
    fn prewarm<F>(&self, mut factory: F) -> Result<(), String>
    where
        F: FnMut() -> Result<LuaVm, String>,
    {
        while self.total_count() < self.config.min_size {
            {
                let mut state = self.state.lock().unwrap();
                state.total_count += 1;
            }
            match factory() {
                Ok(vm) => self.release(vm),
                Err(error) => {
                    let mut state = self.state.lock().unwrap();
                    state.total_count = state.total_count.saturating_sub(1);
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    /// Acquire a VM from the pool, growing on demand up to the configured limit.
    /// 从池中获取虚拟机，并在未达上限时按需扩容。
    fn acquire<F>(self: &Arc<Self>, mut factory: F) -> Result<LuaVmLease, String>
    where
        F: FnMut() -> Result<LuaVm, String>,
    {
        loop {
            let mut state = self.state.lock().unwrap();
            self.reap_idle_locked(&mut state);

            if let Some(mut vm) = state.available.pop() {
                vm.last_used_at = Instant::now();
                return Ok(LuaVmLease {
                    pool: self.clone(),
                    vm: Some(vm),
                });
            }

            if state.total_count < self.config.max_size {
                state.total_count += 1;
                drop(state);
                match factory() {
                    Ok(vm) => {
                        return Ok(LuaVmLease {
                            pool: self.clone(),
                            vm: Some(vm),
                        });
                    }
                    Err(error) => {
                        let mut state = self.state.lock().unwrap();
                        state.total_count = state.total_count.saturating_sub(1);
                        self.condvar.notify_one();
                        return Err(error);
                    }
                }
            }

            let _guard = self.condvar.wait(state).unwrap();
        }
    }

    /// Return a VM back into the pool.
    /// 将虚拟机归还到池中。
    fn release(&self, vm: LuaVm) {
        let mut state = self.state.lock().unwrap();
        state.available.push(vm);
        self.reap_idle_locked(&mut state);
        self.condvar.notify_one();
    }

    /// Retire one broken VM so later borrowers receive a fresh instance instead of stale state.
    /// 退役一个已损坏的虚拟机，确保后续借用方拿到的是新实例而不是陈旧状态。
    fn discard(&self, _vm: LuaVm) {
        let mut state = self.state.lock().unwrap();
        if state.total_count > 0 {
            state.total_count -= 1;
        }
        self.condvar.notify_one();
    }

    /// Return the current total number of VMs in the pool.
    /// 返回当前池中的虚拟机总数。
    fn total_count(&self) -> usize {
        self.state.lock().unwrap().total_count
    }

    /// Reap idle available VMs while respecting the minimum pool size.
    /// 在保证最小池规模的前提下回收空闲虚拟机。
    fn reap_idle_locked(&self, state: &mut LuaVmPoolState) {
        if state.total_count <= self.config.min_size {
            return;
        }

        let idle_limit = Duration::from_secs(self.config.idle_ttl_secs);
        let now = Instant::now();
        let mut index = 0usize;
        while index < state.available.len() && state.total_count > self.config.min_size {
            let should_remove = now
                .checked_duration_since(state.available[index].last_used_at)
                .map(|idle| idle >= idle_limit)
                .unwrap_or(false);
            if should_remove {
                state.available.swap_remove(index);
                state.total_count = state.total_count.saturating_sub(1);
            } else {
                index += 1;
            }
        }
    }
}

impl LuaEngine {
    /// Return the normalized formal label for one raw runtime skill root name.
    /// 返回单个原始运行时技能根名称的规范化正式标签。
    fn normalized_skill_root_name(root_name: &str) -> String {
        root_name.trim().to_ascii_uppercase()
    }

    /// Return the normalized formal label for one runtime skill root.
    /// 返回单个运行时技能根的规范化正式标签。
    fn normalized_skill_root_label(root: &RuntimeSkillRoot) -> String {
        Self::normalized_skill_root_name(&root.name)
    }

    /// Return the fixed load-priority rank for one formal root label.
    /// 返回单个正式根标签的固定加载优先级排序值。
    fn formal_skill_root_rank(label: &str) -> Option<usize> {
        match label {
            "ROOT" => Some(0),
            "PROJECT" => Some(1),
            "USER" => Some(2),
            _ => None,
        }
    }

    /// Return whether one runtime skill root is the system-controlled ROOT layer.
    /// 返回单个运行时技能根是否为系统控制的 ROOT 层。
    fn is_root_skill_root(root: &RuntimeSkillRoot) -> bool {
        Self::normalized_skill_root_label(root) == "ROOT"
    }

    /// Return whether one runtime skill root is writable through the ordinary skills plane.
    /// 返回单个运行时技能根是否可通过普通 skills 平面写入。
    fn is_user_mutable_skill_root(root: &RuntimeSkillRoot) -> bool {
        matches!(
            Self::normalized_skill_root_label(root).as_str(),
            "PROJECT" | "USER"
        )
    }

    /// Validate that the configured skill root chain uses only ROOT, PROJECT, and USER in fixed order.
    /// 校验已配置技能根链仅使用 ROOT、PROJECT、USER 且顺序固定。
    fn validate_formal_skill_root_chain(skill_roots: &[RuntimeSkillRoot]) -> Result<(), String> {
        if skill_roots.is_empty() {
            return Err(
                "ROOT skill root is required; pass a ROOT layer before starting LuaSkills"
                    .to_string(),
            );
        }
        let mut previous_rank = None;
        let mut seen_labels = BTreeSet::new();
        for root in skill_roots {
            let label = Self::normalized_skill_root_label(root);
            let rank = Self::formal_skill_root_rank(&label).ok_or_else(|| {
                format!(
                    "unsupported skill root label '{}'; expected one of ROOT, PROJECT, USER",
                    root.name
                )
            })?;
            if !seen_labels.insert(label.clone()) {
                return Err(format!(
                    "duplicate skill root label '{}'; only one ROOT, PROJECT, and USER root is supported",
                    label
                ));
            }
            if previous_rank
                .map(|previous_rank| rank < previous_rank)
                .unwrap_or(false)
            {
                return Err(
                    "skill roots must be ordered by fixed priority ROOT -> PROJECT -> USER"
                        .to_string(),
                );
            }
            previous_rank = Some(rank);
        }
        if !seen_labels.contains("ROOT") {
            return Err(
                "ROOT skill root is required; pass a ROOT layer before starting LuaSkills"
                    .to_string(),
            );
        }
        Ok(())
    }

    /// Find one configured skill root by its formal label.
    /// 按正式标签查找单个已配置技能根。
    fn find_skill_root_by_label<'a>(
        skill_roots: &'a [RuntimeSkillRoot],
        label: &str,
    ) -> Option<&'a RuntimeSkillRoot> {
        skill_roots
            .iter()
            .find(|root| Self::normalized_skill_root_label(root) == label)
    }

    /// Resolve the default install target for one operation plane.
    /// 解析单个操作平面的默认安装目标根。
    fn default_install_skill_root<'a>(
        &self,
        plane: SkillOperationPlane,
        skill_roots: &'a [RuntimeSkillRoot],
    ) -> Result<&'a RuntimeSkillRoot, String> {
        match plane {
            SkillOperationPlane::Skills => Self::find_skill_root_by_label(skill_roots, "USER")
                .or_else(|| Self::find_skill_root_by_label(skill_roots, "PROJECT"))
                .filter(|root| Self::is_user_mutable_skill_root(root))
                .ok_or_else(|| {
                    "ordinary skills plane requires a PROJECT or USER skill root; ROOT is system-controlled"
                        .to_string()
                }),
            SkillOperationPlane::System => {
                if let Some(root) = Self::find_skill_root_by_label(skill_roots, "ROOT") {
                    Ok(root)
                } else {
                    Err(
                        "system install requires a configured ROOT skill root; ordinary PROJECT/USER layers must be managed through the skills plane"
                            .to_string(),
                    )
                }
            }
        }
    }

    /// Convert one host-injected authority into the lifecycle operation plane it may use.
    /// 将单个宿主注入权限转换为可使用的生命周期操作平面。
    fn operation_plane_for_authority(authority: SkillManagementAuthority) -> SkillOperationPlane {
        match authority {
            SkillManagementAuthority::System => SkillOperationPlane::System,
            SkillManagementAuthority::DelegatedTool => SkillOperationPlane::Skills,
        }
    }

    /// Resolve the canonical runtime root used by the unified skill-config file.
    /// 解析统一技能配置文件所使用的规范运行时根目录。
    fn canonical_skill_config_runtime_root(
        &self,
        skill_roots: &[RuntimeSkillRoot],
    ) -> Result<PathBuf, String> {
        let mut candidates: Vec<PathBuf> = Vec::new();
        for skill_root in skill_roots {
            let candidate = normalize_runtime_root_path(&self.runtime_root_for(skill_root));
            if !candidates.iter().any(|existing| existing == &candidate) {
                candidates.push(candidate);
            }
        }
        match candidates.len() {
            0 => Err("at least one skill root is required to resolve the unified skill config path".to_string()),
            1 => Ok(candidates.remove(0)),
            _ => Err(
                "multiple runtime roots map to different parents; set host_options.skill_config_file_path explicitly".to_string()
            ),
        }
    }

    /// Create a new LuaEngine with LuaJIT VM and registered globals.
    pub fn new(options: LuaEngineOptions) -> Result<Self, Box<dyn std::error::Error>> {
        let _default_text_encoding = resolve_host_default_text_encoding(&options.host_options)
            .map_err(std::io::Error::other)?;
        let runlua_pool_config = options
            .host_options
            .runlua_pool_config
            .map(|config| LuaVmPoolConfig {
                min_size: config.min_size,
                max_size: config.max_size,
                idle_ttl_secs: config.idle_ttl_secs,
            })
            .unwrap_or_else(default_runlua_vm_pool_config);
        configure_global_tool_cache(
            options
                .host_options
                .cache_config
                .clone()
                .unwrap_or_else(ToolCacheConfig::default),
        );
        let native_library_search_guard =
            NativeLibrarySearchGuard::new(&options.host_options).map_err(std::io::Error::other)?;
        let database_provider_callbacks = Arc::new(
            RuntimeDatabaseProviderCallbacks::capture_process_defaults()
                .map_err(std::io::Error::other)?,
        );
        Ok(Self {
            skills: HashMap::new(),
            entry_registry: BTreeMap::new(),
            runtime_skill_roots: Vec::new(),
            pool: Arc::new(LuaVmPool::new(options.pool_config)),
            runlua_pool: Arc::new(LuaVmPool::new(runlua_pool_config)),
            runtime_sessions: Arc::new(RuntimeSessionManager::new()),
            skill_config_store: Arc::new(
                SkillConfigStore::new(options.host_options.skill_config_file_path.clone())
                    .map_err(std::io::Error::other)?,
            ),
            lancedb_host: None,
            sqlite_host: None,
            database_provider_callbacks,
            native_library_search_guard,
            host_options: Arc::new(options.host_options),
        })
    }

    /// Build the shared runtime root used by host-managed sibling directories.
    /// 构造宿主管理同级目录所使用的共享运行时根目录。
    fn runtime_root_for(&self, skill_root: &RuntimeSkillRoot) -> PathBuf {
        skill_root
            .skills_dir
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| skill_root.skills_dir.clone())
    }

    /// Collect the resources directories that may represent packaged runtime layouts for the active root chain.
    /// 收集当前根目录链中可能代表打包运行时布局的 resources 目录。
    fn packaged_runtime_resources_dirs(&self, skill_roots: &[RuntimeSkillRoot]) -> Vec<PathBuf> {
        let mut deduped = BTreeSet::new();
        if let Some(resources_dir) = self.host_options.resources_dir.as_ref() {
            deduped.insert(normalize_runtime_root_path(resources_dir));
        } else {
            for skill_root in skill_roots {
                deduped.insert(normalize_runtime_root_path(
                    &self.runtime_root_for(skill_root).join("resources"),
                ));
            }
        }
        deduped.into_iter().collect()
    }

    /// Validate the embedded luaskills-packages metadata whenever one packaged runtime layout is detected.
    /// 在检测到打包运行时布局时校验其内嵌的 luaskills-packages 元数据。
    fn validate_packaged_runtime_resources(
        &self,
        skill_roots: &[RuntimeSkillRoot],
    ) -> Result<(), String> {
        for resources_dir in self.packaged_runtime_resources_dirs(skill_roots) {
            validate_packaged_runtime_packages_layout(&resources_dir)?;
        }
        Ok(())
    }

    /// Build the sibling state root for one named skill root.
    /// 为单个命名技能根构造同级状态根目录。
    fn state_root_for(&self, skill_root: &RuntimeSkillRoot) -> PathBuf {
        self.runtime_root_for(skill_root)
            .join(self.host_options.state_dir_name.as_str())
    }

    /// Build the sibling dependency root for one named skill root.
    /// 为单个命名技能根构造同级依赖根目录。
    fn dependency_root_for(&self, skill_root: &RuntimeSkillRoot) -> PathBuf {
        self.runtime_root_for(skill_root)
            .join(self.host_options.dependency_dir_name.as_str())
    }

    /// Return whether the host policy forces one skill identifier to be ignored.
    /// 返回宿主策略是否强制忽略指定技能标识符。
    fn is_host_ignored_skill(&self, skill_id: &str) -> bool {
        self.host_options
            .ignored_skill_ids
            .iter()
            .any(|ignored| ignored.trim() == skill_id)
    }

    /// Build the sibling database root for one named skill root.
    /// 为单个命名技能根构造同级数据库根目录。
    fn database_root_for(&self, skill_root: &RuntimeSkillRoot) -> PathBuf {
        self.runtime_root_for(skill_root)
            .join(self.host_options.database_dir_name.as_str())
    }

    /// Capture the shared runtime root used by the unified skill config file.
    /// 记录统一技能配置文件所使用的共享运行时根目录。
    fn refresh_skill_config_runtime_root(
        &self,
        skill_roots: &[RuntimeSkillRoot],
    ) -> Result<(), String> {
        if self.skill_config_store.has_explicit_file_path() {
            return Ok(());
        }
        let runtime_root = self.canonical_skill_config_runtime_root(skill_roots)?;
        self.skill_config_store
            .set_default_runtime_root(&runtime_root)
    }

    /// Create an empty reload candidate that preserves immutable host policy and callback snapshots.
    /// 创建一个空的重载候选引擎，并保留不可变宿主策略与回调快照。
    fn empty_reload_candidate(&self) -> Result<Self, Box<dyn std::error::Error>> {
        let explicit_skill_config_file_path = if self.skill_config_store.has_explicit_file_path() {
            Some(
                self.skill_config_store
                    .file_path()
                    .map_err(std::io::Error::other)?,
            )
        } else {
            None
        };
        Ok(Self {
            skills: HashMap::new(),
            entry_registry: BTreeMap::new(),
            runtime_skill_roots: Vec::new(),
            pool: Arc::new(LuaVmPool::new(self.pool.config)),
            runlua_pool: Arc::new(LuaVmPool::new(self.runlua_pool.config)),
            runtime_sessions: Arc::new(RuntimeSessionManager::new()),
            skill_config_store: Arc::new(
                SkillConfigStore::new(explicit_skill_config_file_path)
                    .map_err(std::io::Error::other)?,
            ),
            lancedb_host: None,
            sqlite_host: None,
            database_provider_callbacks: self.database_provider_callbacks.clone(),
            native_library_search_guard: NativeLibrarySearchGuard::new(&self.host_options)
                .map_err(std::io::Error::other)?,
            host_options: self.host_options.clone(),
        })
    }

    /// Replace the active runtime state with one fully loaded reload candidate.
    /// 使用一个已完整加载的重载候选引擎替换当前活动运行时状态。
    fn replace_runtime_state_from(&mut self, next: LuaEngine) {
        self.skills = next.skills;
        self.entry_registry = next.entry_registry;
        self.runtime_skill_roots = next.runtime_skill_roots;
        self.pool = next.pool;
        self.runlua_pool = next.runlua_pool;
        self.runtime_sessions = next.runtime_sessions;
        self.skill_config_store = next.skill_config_store;
        self.lancedb_host = next.lancedb_host;
        self.sqlite_host = next.sqlite_host;
        self.database_provider_callbacks = next.database_provider_callbacks;
        self.native_library_search_guard = next.native_library_search_guard;
        self.host_options = next.host_options;
    }

    /// Build the dependency-manager configuration for one named skill root.
    /// 为单个命名技能根构造依赖管理器配置。
    fn dependency_manager_config_for(
        &self,
        skill_root: &RuntimeSkillRoot,
    ) -> Result<DependencyManagerConfig, String> {
        let runtime_root = skill_root
            .skills_dir
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| skill_root.skills_dir.clone());
        let dependency_root = self.dependency_root_for(skill_root);
        let tool_root = dependency_root.join("tools");
        let host_tool_root = self
            .host_options
            .host_provided_tool_root
            .clone()
            .unwrap_or_else(|| runtime_root.join("bin").join("tools"));
        let lua_root = dependency_root.join("lua");
        let host_lua_root = self
            .host_options
            .host_provided_lua_root
            .clone()
            .or_else(|| self.host_options.lua_packages_dir.clone())
            .unwrap_or_else(|| runtime_root.join("lua_packages"));
        let ffi_root = dependency_root.join("ffi");
        let host_ffi_root = self
            .host_options
            .host_provided_ffi_root
            .clone()
            .or_else(|| {
                self.host_options
                    .lancedb_library_path
                    .as_ref()
                    .and_then(|path| path.parent().map(Path::to_path_buf))
            })
            .or_else(|| {
                self.host_options
                    .sqlite_library_path
                    .as_ref()
                    .and_then(|path| path.parent().map(Path::to_path_buf))
            })
            .unwrap_or_else(|| runtime_root.join("libs"));
        let download_cache_root = self
            .host_options
            .download_cache_root
            .clone()
            .unwrap_or_else(|| runtime_root.join("temp").join("downloads"));

        ensure_directory(&tool_root)?;
        ensure_directory(&host_tool_root)?;
        ensure_directory(&lua_root)?;
        ensure_directory(&host_lua_root)?;
        ensure_directory(&ffi_root)?;
        ensure_directory(&host_ffi_root)?;
        ensure_directory(&download_cache_root)?;

        Ok(DependencyManagerConfig {
            tool_root,
            host_tool_root,
            lua_root,
            host_lua_root,
            ffi_root,
            host_ffi_root,
            download_cache_root,
            allow_network_download: self.host_options.allow_network_download,
            github_base_url: self.host_options.github_base_url.clone(),
            github_api_base_url: self.host_options.github_api_base_url.clone(),
        })
    }

    /// Build the skill-manager configuration for one named skill root.
    /// 为单个命名技能根构造技能管理器配置。
    fn skill_manager_for(&self, skill_root: &RuntimeSkillRoot) -> Result<SkillManager, String> {
        let state_root = self.state_root_for(skill_root);
        let dependency_config = self.dependency_manager_config_for(skill_root)?;
        ensure_directory(&state_root)?;
        Ok(SkillManager::new(SkillManagerConfig {
            skill_root: skill_root.clone(),
            lifecycle_root: state_root,
            download_cache_root: dependency_config.download_cache_root,
            allow_network_download: dependency_config.allow_network_download,
            github_base_url: dependency_config.github_base_url,
            github_api_base_url: dependency_config.github_api_base_url,
            official_skill_hub_base_url: self.host_options.official_skill_hub_base_url.clone(),
            enable_private_url_skill_install: self.host_options.enable_private_url_skill_install,
            private_skill_source_allowlist: self
                .host_options
                .private_skill_source_allowlist
                .clone(),
        }))
    }

    /// Build the skill-manager configuration for one operation with progress reporting enabled.
    /// 为启用进度回馈的单次操作构造技能管理器配置。
    fn skill_manager_for_with_progress(
        &self,
        skill_root: &RuntimeSkillRoot,
        progress: RuntimeSkillOperationProgressEmitter,
    ) -> Result<SkillManager, String> {
        let state_root = self.state_root_for(skill_root);
        let dependency_config = self.dependency_manager_config_for(skill_root)?;
        ensure_directory(&state_root)?;
        Ok(SkillManager::new_with_progress(
            SkillManagerConfig {
                skill_root: skill_root.clone(),
                lifecycle_root: state_root,
                download_cache_root: dependency_config.download_cache_root,
                allow_network_download: dependency_config.allow_network_download,
                github_base_url: dependency_config.github_base_url,
                github_api_base_url: dependency_config.github_api_base_url,
                official_skill_hub_base_url: self.host_options.official_skill_hub_base_url.clone(),
                enable_private_url_skill_install: self
                    .host_options
                    .enable_private_url_skill_install,
                private_skill_source_allowlist: self
                    .host_options
                    .private_skill_source_allowlist
                    .clone(),
            },
            Some(progress),
        ))
    }

    /// Ensure dependencies declared by one skill directory are installed before the skill is loaded.
    /// 在真正加载 skill 前确保该目录声明的依赖已经安装完成。
    fn ensure_skill_dependencies(
        &self,
        skill_root: &RuntimeSkillRoot,
        skill_dir: &Path,
    ) -> Result<(), String> {
        let dependencies_path = skill_dir.join("dependencies.yaml");
        if !dependencies_path.exists() {
            return Ok(());
        }

        let manifest = SkillDependencyManifest::load_from_path(&dependencies_path)?;
        if manifest.is_empty() {
            return Ok(());
        }

        let skill_name = skill_dir
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("unknown-skill");
        let manager = DependencyManager::new(self.dependency_manager_config_for(skill_root)?);
        manager.ensure_skill_dependencies(skill_name, &manifest)
    }

    /// Load one optional dependency manifest from one skill directory when the file exists.
    /// 当依赖文件存在时，从单个技能目录加载可选依赖清单。
    fn load_skill_dependency_manifest(
        &self,
        skill_dir: &Path,
    ) -> Result<Option<SkillDependencyManifest>, String> {
        let dependencies_path = skill_dir.join("dependencies.yaml");
        if !dependencies_path.exists() {
            return Ok(None);
        }
        SkillDependencyManifest::load_from_path(&dependencies_path).map(Some)
    }

    /// Return whether two runtime skill roots refer to the same configured root entry.
    /// 返回两个运行时技能根是否指向同一个已配置根条目。
    fn runtime_skill_roots_match(left: &RuntimeSkillRoot, right: &RuntimeSkillRoot) -> bool {
        left.name == right.name && left.skills_dir == right.skills_dir
    }

    /// Return the ordered-chain index of one configured root.
    /// 返回单个已配置根在有序根链中的索引。
    fn runtime_skill_root_index(
        skill_roots: &[RuntimeSkillRoot],
        target_root: &RuntimeSkillRoot,
    ) -> Result<usize, String> {
        skill_roots
            .iter()
            .position(|root| Self::runtime_skill_roots_match(root, target_root))
            .ok_or_else(|| {
                format!(
                    "target root '{}' at {} is not part of the full runtime root chain",
                    target_root.name,
                    target_root.skills_dir.display()
                )
            })
    }

    /// Ensure one delegated ordinary target is a configured PROJECT or USER root.
    /// 确保单个委托普通目标是已配置的 PROJECT 或 USER 根。
    fn validate_ordinary_target_root(
        skill_roots: &[RuntimeSkillRoot],
        target_root: &RuntimeSkillRoot,
        action: crate::skill::manager::SkillLifecycleAction,
    ) -> Result<(), String> {
        Self::runtime_skill_root_index(skill_roots, target_root)?;
        if Self::is_root_skill_root(target_root) {
            return Err(format!(
                "ordinary skills plane cannot {:?} the system-controlled ROOT skill root",
                action
            ));
        }
        if !Self::is_user_mutable_skill_root(target_root) {
            return Err(format!(
                "ordinary skills plane can only {:?} PROJECT or USER skill roots; got '{}'",
                action, target_root.name
            ));
        }
        Ok(())
    }

    /// Ensure one authority may write the requested target root.
    /// 确保单个权限等级可以写入请求的目标根。
    fn validate_authority_for_target_root(
        authority: SkillManagementAuthority,
        target_root: &RuntimeSkillRoot,
        action: crate::skill::manager::SkillLifecycleAction,
    ) -> Result<(), String> {
        if authority == SkillManagementAuthority::DelegatedTool
            && Self::is_root_skill_root(target_root)
        {
            return Err(format!(
                "DelegatedTool authority cannot {:?} the system-controlled ROOT skill root",
                action
            ));
        }
        Ok(())
    }

    /// Return the ROOT-owned declaration for one skill id when the root exists.
    /// 当 ROOT 存在时返回单个 skill id 的 ROOT 层声明。
    fn resolve_root_declared_skill_instance(
        skill_roots: &[RuntimeSkillRoot],
        skill_id: &str,
    ) -> Result<Option<ResolvedSkillInstance>, String> {
        let Some(root) = Self::find_skill_root_by_label(skill_roots, "ROOT") else {
            return Ok(None);
        };
        resolve_declared_skill_instance_from_roots(&[root.clone()], skill_id)
    }

    /// Reject PROJECT or USER install/update when ROOT already owns the same skill id.
    /// 当 ROOT 已拥有同名 skill id 时拒绝 PROJECT 或 USER 的安装与更新。
    fn ensure_root_skill_id_is_not_system_occupied(
        skill_roots: &[RuntimeSkillRoot],
        target_root: &RuntimeSkillRoot,
        skill_id: &str,
        action: crate::skill::manager::SkillLifecycleAction,
    ) -> Result<(), String> {
        if !matches!(
            action,
            crate::skill::manager::SkillLifecycleAction::Install
                | crate::skill::manager::SkillLifecycleAction::Update
        ) || !Self::is_user_mutable_skill_root(target_root)
        {
            return Ok(());
        }
        if let Some(root_instance) =
            Self::resolve_root_declared_skill_instance(skill_roots, skill_id)?
        {
            return Err(format!(
                "skill '{}' is managed by the ROOT system layer at {}; {:?} in '{}' is not allowed until the ROOT skill is removed",
                skill_id,
                root_instance.actual_dir.display(),
                action,
                target_root.name
            ));
        }
        Ok(())
    }

    /// Ensure one explicit target root will be effective after the staged apply operation.
    /// 确保单个显式目标根在暂存应用操作完成后会成为生效根。
    fn ensure_explicit_apply_target_will_be_effective(
        skill_roots: &[RuntimeSkillRoot],
        target_root: Option<&RuntimeSkillRoot>,
        skill_id: &str,
    ) -> Result<(), String> {
        let Some(target_root) = target_root else {
            return Ok(());
        };
        let target_index = Self::runtime_skill_root_index(skill_roots, target_root)?;
        let Some(effective_instance) =
            resolve_declared_skill_instance_from_roots(skill_roots, skill_id)?
        else {
            return Ok(());
        };
        let effective_root = RuntimeSkillRoot {
            name: effective_instance.root_name.clone(),
            skills_dir: effective_instance.skills_root.clone(),
        };
        let effective_index = Self::runtime_skill_root_index(skill_roots, &effective_root)?;
        if effective_index < target_index {
            return Err(format!(
                "skill '{}' in target root '{}' is shadowed by higher-priority root '{}'; update the higher-priority layer or remove that override before changing this fallback root",
                skill_id, target_root.name, effective_instance.root_name
            ));
        }
        Ok(())
    }

    /// Resolve final canonical entry names for all loaded skills with stable collision indexing.
    /// 为全部已加载 skill 解析最终 canonical 入口名，并以稳定顺序处理冲突编号。
    fn rebuild_entry_registry(&mut self) -> Result<(), String> {
        /// One unresolved entry candidate collected before collision indexing.
        /// 冲突编号前收集到的单个未解析入口候选项。
        #[derive(Clone)]
        struct EntrySeed {
            /// Internal storage key of the owning loaded skill.
            /// 所属已加载 skill 的内部存储键。
            skill_storage_key: String,
            /// Stable skill identifier declared in metadata.
            /// 元数据中声明的稳定 skill 标识符。
            skill_id: String,
            /// Stable local entry name declared by the skill.
            /// skill 声明的稳定局部入口名称。
            local_name: String,
            /// Unresolved `skill-entry` base name before numeric suffixing.
            /// 添加数字后缀前的未解析 `skill-entry` 基础名称。
            base_name: String,
            /// Deterministic tie-breaker based on directory basename.
            /// 基于目录基名的确定性并列打破键。
            directory_name: String,
            /// Module name used as the final low-level tie-breaker.
            /// 作为最终并列打破条件的模块名称。
            module_name: String,
        }

        let mut seeds = Vec::new();
        for (skill_storage_key, skill) in &self.skills {
            for tool in skill.meta.entries() {
                let local_name = tool.name.trim().to_string();
                if seeds.iter().any(|seed: &EntrySeed| {
                    seed.skill_storage_key == *skill_storage_key && seed.local_name == local_name
                }) {
                    return Err(format!(
                        "skill '{}' declares duplicate local entry name '{}'",
                        skill.meta.effective_skill_id(),
                        local_name
                    ));
                }

                let directory_name = skill
                    .dir
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or_default()
                    .to_string();
                seeds.push(EntrySeed {
                    skill_storage_key: skill_storage_key.clone(),
                    skill_id: skill.meta.effective_skill_id().to_string(),
                    local_name: local_name.clone(),
                    base_name: skill.meta.tool_base_name(tool),
                    directory_name,
                    module_name: tool.lua_module.clone(),
                });
            }
        }

        seeds.sort_by(|left, right| {
            (
                left.base_name.as_str(),
                left.directory_name.as_str(),
                left.skill_id.as_str(),
                left.local_name.as_str(),
                left.module_name.as_str(),
            )
                .cmp(&(
                    right.base_name.as_str(),
                    right.directory_name.as_str(),
                    right.skill_id.as_str(),
                    right.local_name.as_str(),
                    right.module_name.as_str(),
                ))
        });

        for skill in self.skills.values_mut() {
            skill.resolved_entry_names.clear();
        }

        let mut registry = BTreeMap::new();
        let mut base_name_counters = HashMap::<String, usize>::new();
        let mut occupied_names = self
            .host_options
            .reserved_entry_names
            .iter()
            .cloned()
            .collect::<HashSet<String>>();
        for seed in seeds {
            let mut duplicate_index = *base_name_counters.get(&seed.base_name).unwrap_or(&0usize);
            let canonical_name = loop {
                duplicate_index += 1;
                let candidate_name = if duplicate_index == 1 {
                    seed.base_name.clone()
                } else {
                    format!("{}-{}", seed.base_name, duplicate_index)
                };
                if !occupied_names.contains(&candidate_name) {
                    break candidate_name;
                }
            };
            base_name_counters.insert(seed.base_name.clone(), duplicate_index);
            occupied_names.insert(canonical_name.clone());

            let resolved_target = ResolvedEntryTarget {
                canonical_name: canonical_name.clone(),
                skill_storage_key: seed.skill_storage_key.clone(),
                skill_id: seed.skill_id.clone(),
                local_name: seed.local_name.clone(),
            };
            registry.insert(canonical_name.clone(), resolved_target);

            let skill = self
                .skills
                .get_mut(&seed.skill_storage_key)
                .ok_or_else(|| {
                    format!(
                        "internal error: missing loaded skill '{}' while building entry registry",
                        seed.skill_storage_key
                    )
                })?;
            skill
                .resolved_entry_names
                .insert(seed.local_name.clone(), canonical_name);
        }

        self.entry_registry = registry;
        Ok(())
    }

    /// Load skills from an ordered root chain where earlier roots override later roots.
    /// 从有序根目录覆盖链加载技能，前面的根目录会覆盖后面的同名技能。
    pub fn load_from_roots(
        &mut self,
        skill_roots: &[RuntimeSkillRoot],
    ) -> Result<(), Box<dyn std::error::Error>> {
        Self::validate_formal_skill_root_chain(skill_roots)
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        self.runtime_skill_roots = skill_roots.to_vec();
        if !skill_roots.is_empty() {
            self.refresh_skill_config_runtime_root(skill_roots)
                .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
            self.validate_packaged_runtime_resources(skill_roots)
                .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        }
        if skill_roots.iter().all(|root| !root.skills_dir.exists()) {
            return Ok(());
        }

        for resolved_instance in collect_effective_skill_instances_from_roots(skill_roots)
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?
        {
            let skill_name = resolved_instance.skill_id;
            if self.is_host_ignored_skill(&skill_name) {
                log_info(format!(
                    "[LuaSkill] Skipped host-ignored skill '{}'",
                    skill_name
                ));
                continue;
            }
            let resolved_root = RuntimeSkillRoot {
                name: resolved_instance.root_name.clone(),
                skills_dir: resolved_instance.skills_root.clone(),
            };
            let resolved_skill_manager = self
                .skill_manager_for(&resolved_root)
                .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
            if !resolved_skill_manager.is_skill_enabled(&skill_name)? {
                log_warn(format!(
                    "[LuaSkill] Skipped disabled skill '{}'",
                    skill_name
                ));
                continue;
            }
            let actual_dir = resolved_instance.actual_dir;
            log_info(format!(
                "[LuaSkill] Loaded '{}' from root '{}'",
                skill_name, resolved_instance.root_name
            ));

            if let Err(error) = self.ensure_skill_dependencies(&resolved_root, &actual_dir) {
                log_error(format!(
                    "[LuaSkill] Failed to prepare dependencies for {}: {}",
                    skill_name, error
                ));
                continue;
            }

            if let Err(e) = self.load_single_skill(&actual_dir, &resolved_instance.root_name) {
                log_error(format!("[LuaSkill] Failed to load {}: {}", skill_name, e));
            }
        }

        self.rebuild_entry_registry()
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;

        self.pool
            .prewarm(|| self.create_vm())
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        self.runlua_pool
            .prewarm(|| {
                Self::create_runlua_vm(
                    &self.skills,
                    &self.entry_registry,
                    self.host_options.clone(),
                    self.skill_config_store.clone(),
                    self.runtime_skill_roots.clone(),
                    self.lancedb_host.clone(),
                    self.sqlite_host.clone(),
                )
            })
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;

        log_info(format!("[LuaSkill] {} skills loaded", self.skills.len()));
        Ok(())
    }

    /// Reload all skills from one ordered root chain and rebuild runtime state from scratch.
    /// 从一条有序根目录覆盖链中重载全部技能，并从零重建运行时状态。
    pub fn reload_from_roots(
        &mut self,
        skill_roots: &[RuntimeSkillRoot],
    ) -> Result<(), Box<dyn std::error::Error>> {
        Self::validate_formal_skill_root_chain(skill_roots)
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        let previous_entries = self.list_entries();
        let mut next = self.empty_reload_candidate()?;
        next.load_from_roots(skill_roots)?;
        self.replace_runtime_state_from(next);
        self.emit_entry_registry_delta(previous_entries);
        Ok(())
    }

    /// Execute one mutating skill lifecycle action in the requested operation plane and then reload the runtime view.
    /// 在指定操作平面执行一次会改变状态的技能生命周期动作，并在完成后立即重载运行时视图。
    fn mutate_skill_state_and_reload(
        &mut self,
        plane: SkillOperationPlane,
        action: crate::skill::manager::SkillLifecycleAction,
        skill_roots: &[RuntimeSkillRoot],
        skill_id: &str,
        reason: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        Self::validate_formal_skill_root_chain(skill_roots)
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        validate_luaskills_identifier(skill_id, "skill_id")
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        let resolved_instance = resolve_declared_skill_instance_from_roots(skill_roots, skill_id)
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?
            .ok_or_else(|| -> Box<dyn std::error::Error> {
                format!("declared skill instance '{}' not found", skill_id).into()
            })?;
        let resolved_root = RuntimeSkillRoot {
            name: resolved_instance.root_name.clone(),
            skills_dir: resolved_instance.skills_root.clone(),
        };
        let manager = self
            .skill_manager_for(&resolved_root)
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        let removed_dependency_manifest =
            if action == crate::skill::manager::SkillLifecycleAction::Uninstall {
                let dependencies_path = resolved_instance.actual_dir.join("dependencies.yaml");
                if dependencies_path.exists() {
                    Some(
                        SkillDependencyManifest::load_from_path(&dependencies_path)
                            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?,
                    )
                } else {
                    None
                }
            } else {
                None
            };
        if let Err(error) = manager.guard_operation(plane, action, skill_id) {
            self.emit_skill_lifecycle_event(
                plane,
                action,
                skill_id,
                Some(resolved_instance.root_name.clone()),
                Some(resolved_instance.actual_dir.display().to_string()),
                "blocked",
                Some(error.clone()),
            );
            return Err(error.into());
        }
        let action_result = match action {
            crate::skill::manager::SkillLifecycleAction::Disable => manager
                .disable_skill_in_plane(plane, skill_id, reason)
                .map_err(|error| -> Box<dyn std::error::Error> { error.into() }),
            crate::skill::manager::SkillLifecycleAction::Enable => manager
                .enable_skill_in_plane(plane, skill_id)
                .map_err(|error| -> Box<dyn std::error::Error> { error.into() }),
            crate::skill::manager::SkillLifecycleAction::Uninstall => manager
                .uninstall_skill_at_path_in_plane(plane, skill_id, &resolved_instance.actual_dir)
                .map(|_| ())
                .map_err(|error| -> Box<dyn std::error::Error> { error.into() }),
            _ => {
                return Err(format!("unsupported state mutation action {:?}", action).into());
            }
        };
        if let Err(error) = action_result {
            let message = error.to_string();
            self.emit_skill_lifecycle_event(
                plane,
                action,
                skill_id,
                Some(resolved_instance.root_name.clone()),
                Some(resolved_instance.actual_dir.display().to_string()),
                "failed",
                Some(message),
            );
            return Err(error);
        }
        if action == crate::skill::manager::SkillLifecycleAction::Uninstall {
            let dependency_manager = DependencyManager::new(
                self.dependency_manager_config_for(&resolved_root)
                    .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?,
            );
            dependency_manager
                .cleanup_uninstalled_skill_dependencies_from_roots(
                    skill_roots,
                    skill_id,
                    removed_dependency_manifest.as_ref(),
                )
                .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        }
        self.reload_from_roots(skill_roots)?;
        self.emit_skill_lifecycle_event(
            plane,
            action,
            skill_id,
            Some(resolved_instance.root_name.clone()),
            Some(resolved_instance.actual_dir.display().to_string()),
            "completed",
            None,
        );
        Ok(())
    }

    /// Remove one optional skill-owned database directory when the caller explicitly requests it.
    /// 调用方显式请求时删除单个技能拥有的可选数据库目录。
    fn remove_skill_database_dir(
        &self,
        database_root: &Path,
        skill_id: &str,
        remove_requested: bool,
        database_label: &str,
    ) -> Result<(bool, bool), Box<dyn std::error::Error>> {
        if !remove_requested {
            return Ok((false, true));
        }
        let database_dir = database_root.join(database_label).join(skill_id);
        if !database_dir.exists() {
            return Ok((false, false));
        }
        fs::remove_dir_all(&database_dir).map_err(|error| {
            format!(
                "failed to remove {database_label} directory {}: {}",
                database_dir.display(),
                error
            )
        })?;
        Ok((true, false))
    }

    /// Execute one uninstall action with explicit database-retention semantics and then reload the runtime view.
    /// 以显式数据库保留语义执行一次卸载动作，并在完成后重载运行时视图。
    fn uninstall_skill_and_reload(
        &mut self,
        plane: SkillOperationPlane,
        skill_roots: &[RuntimeSkillRoot],
        skill_id: &str,
        options: &SkillUninstallOptions,
    ) -> Result<SkillUninstallResult, Box<dyn std::error::Error>> {
        self.uninstall_skill_and_reload_in_root(plane, skill_roots, None, skill_id, options)
    }

    /// Execute one uninstall action against an optional explicit target root and then reload the full runtime view.
    /// 针对可选的显式目标根执行一次卸载动作，并随后重载完整运行时视图。
    fn uninstall_skill_and_reload_in_root(
        &mut self,
        plane: SkillOperationPlane,
        skill_roots: &[RuntimeSkillRoot],
        target_root: Option<&RuntimeSkillRoot>,
        skill_id: &str,
        options: &SkillUninstallOptions,
    ) -> Result<SkillUninstallResult, Box<dyn std::error::Error>> {
        Self::validate_formal_skill_root_chain(skill_roots)
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        validate_luaskills_identifier(skill_id, "skill_id")
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        if let Some(target_root) = target_root {
            Self::runtime_skill_root_index(skill_roots, target_root)
                .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        }
        let target_roots = target_root.map(|root| vec![root.clone()]);
        let resolution_roots = target_roots.as_deref().unwrap_or(skill_roots);
        let resolved_instance = if target_root.is_some() {
            resolve_declared_skill_instance_from_roots(resolution_roots, skill_id)
        } else {
            resolve_effective_skill_instance_from_roots(resolution_roots, skill_id)
        }
        .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?
        .ok_or_else(|| -> Box<dyn std::error::Error> {
            match target_root {
                Some(root) => format!(
                    "skill instance '{}' not found in target root '{}'",
                    skill_id, root.name
                )
                .into(),
                None => format!("effective skill instance '{}' not found", skill_id).into(),
            }
        })?;
        let resolved_root = RuntimeSkillRoot {
            name: resolved_instance.root_name.clone(),
            skills_dir: resolved_instance.skills_root.clone(),
        };
        let manager = self
            .skill_manager_for(&resolved_root)
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        let dependencies_path = resolved_instance.actual_dir.join("dependencies.yaml");
        let removed_dependency_manifest = if dependencies_path.exists() {
            Some(
                SkillDependencyManifest::load_from_path(&dependencies_path)
                    .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?,
            )
        } else {
            None
        };
        if let Err(error) = manager.guard_operation(
            plane,
            crate::skill::manager::SkillLifecycleAction::Uninstall,
            skill_id,
        ) {
            self.emit_skill_lifecycle_event(
                plane,
                crate::skill::manager::SkillLifecycleAction::Uninstall,
                skill_id,
                Some(resolved_instance.root_name.clone()),
                Some(resolved_instance.actual_dir.display().to_string()),
                "blocked",
                Some(error.clone()),
            );
            return Err(error.into());
        }
        let prepared_uninstall = match manager.prepare_uninstall_skill_at_path_in_plane(
            plane,
            skill_id,
            &resolved_instance.actual_dir,
        ) {
            Ok(prepared) => prepared,
            Err(error) => {
                let message = error.to_string();
                self.emit_skill_lifecycle_event(
                    plane,
                    crate::skill::manager::SkillLifecycleAction::Uninstall,
                    skill_id,
                    Some(resolved_instance.root_name.clone()),
                    Some(resolved_instance.actual_dir.display().to_string()),
                    "failed",
                    Some(message),
                );
                return Err(error.into());
            }
        };
        if let Err(reload_error) = self.reload_from_roots(skill_roots) {
            let rollback_error = manager.rollback_prepared_skill_uninstall(&prepared_uninstall);
            let restore_error = self.reload_from_roots(skill_roots);
            let rollback_message = rollback_error
                .err()
                .map(|error| format!(" rollback failed: {}", error))
                .unwrap_or_default();
            let restore_message = restore_error
                .err()
                .map(|error| format!(" runtime restore failed: {}", error))
                .unwrap_or_default();
            let message = format!(
                "Failed to reload LuaSkills after uninstall: {}.{}{}",
                reload_error, rollback_message, restore_message
            );
            self.emit_skill_lifecycle_event(
                plane,
                crate::skill::manager::SkillLifecycleAction::Uninstall,
                skill_id,
                Some(resolved_instance.root_name.clone()),
                Some(resolved_instance.actual_dir.display().to_string()),
                "failed",
                Some(message.clone()),
            );
            return Err(message.into());
        }
        let mut result = match manager.commit_prepared_skill_uninstall(&prepared_uninstall) {
            Ok(result) => result,
            Err(error) => {
                let rollback_error = manager.rollback_prepared_skill_uninstall(&prepared_uninstall);
                let restore_error = self.reload_from_roots(skill_roots);
                let rollback_message = rollback_error
                    .err()
                    .map(|rollback| format!(" rollback failed: {}", rollback))
                    .unwrap_or_default();
                let restore_message = restore_error
                    .err()
                    .map(|restore| format!(" runtime restore failed: {}", restore))
                    .unwrap_or_default();
                let message = format!(
                    "Failed to finalize uninstall: {}.{}{}",
                    error, rollback_message, restore_message
                );
                self.emit_skill_lifecycle_event(
                    plane,
                    crate::skill::manager::SkillLifecycleAction::Uninstall,
                    skill_id,
                    Some(resolved_instance.root_name.clone()),
                    Some(resolved_instance.actual_dir.display().to_string()),
                    "failed",
                    Some(message.clone()),
                );
                return Err(message.into());
            }
        };
        let dependency_manager = DependencyManager::new(
            self.dependency_manager_config_for(&resolved_root)
                .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?,
        );
        if let Err(error) = dependency_manager.cleanup_uninstalled_skill_dependencies_from_roots(
            skill_roots,
            skill_id,
            removed_dependency_manifest.as_ref(),
        ) {
            log_warn(format!(
                "[LuaSkills:uninstall] Stale dependency cleanup warning for skill '{}': {}",
                skill_id, error
            ));
            result.message = format!(
                "{} (warning: stale dependency cleanup failed: {})",
                result.message, error
            );
        }
        let (sqlite_removed, sqlite_retained) = match self.remove_skill_database_dir(
            &self.database_root_for(&resolved_root),
            skill_id,
            options.remove_sqlite,
            "sqlite",
        ) {
            Ok(result) => result,
            Err(error) => {
                log_warn(format!(
                    "[LuaSkills:uninstall] SQLite cleanup warning for skill '{}': {}",
                    skill_id, error
                ));
                result.message = format!(
                    "{} (warning: sqlite cleanup failed: {})",
                    result.message, error
                );
                (false, false)
            }
        };
        let (lancedb_removed, lancedb_retained) = match self.remove_skill_database_dir(
            &self.database_root_for(&resolved_root),
            skill_id,
            options.remove_lancedb,
            "lancedb",
        ) {
            Ok(result) => result,
            Err(error) => {
                log_warn(format!(
                    "[LuaSkills:uninstall] LanceDB cleanup warning for skill '{}': {}",
                    skill_id, error
                ));
                result.message = format!(
                    "{} (warning: lancedb cleanup failed: {})",
                    result.message, error
                );
                (false, false)
            }
        };
        result.sqlite_removed = sqlite_removed;
        result.sqlite_retained = sqlite_retained;
        result.lancedb_removed = lancedb_removed;
        result.lancedb_retained = lancedb_retained;
        let summary = format!(
            "skill package removed={} sqlite_removed={} sqlite_retained={} lancedb_removed={} lancedb_retained={}",
            result.skill_removed,
            result.sqlite_removed,
            result.sqlite_retained,
            result.lancedb_removed,
            result.lancedb_retained
        );
        result.message = if result.message.is_empty() {
            summary
        } else {
            format!("{}; {}", summary, result.message)
        };
        self.emit_skill_lifecycle_event(
            plane,
            crate::skill::manager::SkillLifecycleAction::Uninstall,
            skill_id,
            Some(resolved_instance.root_name.clone()),
            Some(resolved_instance.actual_dir.display().to_string()),
            "completed",
            Some(result.message.clone()),
        );
        Ok(result)
    }

    /// Execute one install or update preflight request in the requested operation plane.
    /// 在指定操作平面执行一次安装或更新预检查请求。
    fn apply_skill_request(
        &mut self,
        plane: SkillOperationPlane,
        action: crate::skill::manager::SkillLifecycleAction,
        skill_roots: &[RuntimeSkillRoot],
        request: &SkillInstallRequest,
    ) -> Result<SkillApplyResult, Box<dyn std::error::Error>> {
        self.apply_skill_request_in_root(plane, action, skill_roots, None, request)
    }

    /// Execute one install or update request against an optional explicit target root.
    /// 针对可选的显式目标根执行一次安装或更新请求。
    fn apply_skill_request_in_root(
        &mut self,
        plane: SkillOperationPlane,
        action: crate::skill::manager::SkillLifecycleAction,
        skill_roots: &[RuntimeSkillRoot],
        target_root: Option<&RuntimeSkillRoot>,
        request: &SkillInstallRequest,
    ) -> Result<SkillApplyResult, Box<dyn std::error::Error>> {
        if !matches!(
            action,
            crate::skill::manager::SkillLifecycleAction::Install
                | crate::skill::manager::SkillLifecycleAction::Update
        ) {
            return Err(format!("unsupported apply action {:?}", action).into());
        }
        Self::validate_formal_skill_root_chain(skill_roots)
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        let explicit_target_root = target_root;
        if let Some(target_root) = explicit_target_root {
            Self::runtime_skill_root_index(skill_roots, target_root)
                .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        }
        let requested_skill_id = resolve_requested_skill_id(request)
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        let explicit_target_roots = explicit_target_root.map(|root| vec![root.clone()]);
        let update_resolution_roots = explicit_target_roots.as_deref().unwrap_or(skill_roots);
        let target_root = match action {
            crate::skill::manager::SkillLifecycleAction::Install => {
                if let Some(target_root) = explicit_target_root {
                    target_root.clone()
                } else {
                    self.default_install_skill_root(plane, skill_roots)?.clone()
                }
            }
            crate::skill::manager::SkillLifecycleAction::Update => {
                let resolved_instance = resolve_declared_skill_instance_from_roots(
                    update_resolution_roots,
                    &requested_skill_id,
                )
                .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?
                .ok_or_else(|| -> Box<dyn std::error::Error> {
                    match explicit_target_root {
                        Some(root) => format!(
                            "skill '{}' is not installed in target root '{}'",
                            requested_skill_id, root.name
                        )
                        .into(),
                        None => format!("skill '{}' is not installed", requested_skill_id).into(),
                    }
                })?;
                RuntimeSkillRoot {
                    name: resolved_instance.root_name,
                    skills_dir: resolved_instance.skills_root,
                }
            }
            _ => unreachable!("unsupported apply action should have returned early"),
        };
        Self::ensure_root_skill_id_is_not_system_occupied(
            skill_roots,
            &target_root,
            &requested_skill_id,
            action,
        )
        .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        if explicit_target_root.is_some() {
            Self::ensure_explicit_apply_target_will_be_effective(
                skill_roots,
                explicit_target_root,
                &requested_skill_id,
            )
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        }
        let operation_roots_owned = if let Some(target_root) = explicit_target_root {
            Some(vec![target_root.clone()])
        } else if action == crate::skill::manager::SkillLifecycleAction::Install
            && plane == SkillOperationPlane::System
            && Self::is_root_skill_root(&target_root)
        {
            Some(vec![target_root.clone()])
        } else {
            None
        };
        let operation_roots = operation_roots_owned.as_deref().unwrap_or(skill_roots);
        let previous_dependency_manifest =
            if action == crate::skill::manager::SkillLifecycleAction::Update {
                resolve_declared_skill_instance_from_roots(operation_roots, &requested_skill_id)
                    .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?
                    .and_then(|resolved| {
                        self.load_skill_dependency_manifest(&resolved.actual_dir)
                            .transpose()
                    })
                    .transpose()
                    .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?
            } else {
                None
            };
        let progress = RuntimeSkillOperationProgressEmitter::new(
            plane,
            action,
            Some(target_root.name.clone()),
            Some(requested_skill_id.clone()),
        );
        progress.emit(
            "validating_request",
            "completed",
            Some("skill lifecycle request accepted".to_string()),
        );
        let manager = match self.skill_manager_for_with_progress(&target_root, progress.clone()) {
            Ok(manager) => manager,
            Err(error) => {
                progress.emit("failed", "failed", Some(error.clone()));
                return Err(error.into());
            }
        };
        let prepared = match action {
            crate::skill::manager::SkillLifecycleAction::Install => {
                match manager.prepare_install_skill(plane, operation_roots, request) {
                    Ok(prepared) => prepared,
                    Err(error) => {
                        progress.emit("failed", "failed", Some(error.clone()));
                        return Err(error.into());
                    }
                }
            }
            crate::skill::manager::SkillLifecycleAction::Update => {
                match manager.prepare_update_skill(plane, operation_roots, request) {
                    Ok(prepared) => prepared,
                    Err(error) => {
                        progress.emit("failed", "failed", Some(error.clone()));
                        return Err(error.into());
                    }
                }
            }
            _ => unreachable!("unsupported apply action should have returned early"),
        };
        let mut result = match &prepared {
            PreparedSkillApply::Immediate(result) => result.clone(),
            PreparedSkillApply::Install(_) | PreparedSkillApply::Update(_) => {
                progress.emit(
                    "reloading_runtime",
                    "started",
                    Some("reloading LuaSkills runtime after staged change".to_string()),
                );
                if let Err(reload_error) = self.reload_from_roots(skill_roots) {
                    let rollback_error = manager.rollback_prepared_skill_apply(&prepared);
                    let restore_error = self.reload_from_roots(skill_roots);
                    let rollback_message = rollback_error
                        .err()
                        .map(|error| format!(" rollback failed: {}", error))
                        .unwrap_or_default();
                    let restore_message = restore_error
                        .err()
                        .map(|error| format!(" runtime restore failed: {}", error))
                        .unwrap_or_default();
                    let message = format!(
                        "Failed to reload LuaSkills after {:?}: {}.{}{}",
                        action, reload_error, rollback_message, restore_message
                    );
                    progress.emit("failed", "failed", Some(message.clone()));
                    return Err(message.into());
                }

                progress.emit(
                    "committing",
                    "started",
                    Some("committing staged skill lifecycle change".to_string()),
                );
                let committed = manager.commit_prepared_skill_apply(&prepared).map_err(
                    |error| -> Box<dyn std::error::Error> {
                        let rollback_error = manager.rollback_prepared_skill_apply(&prepared);
                        let restore_error = self.reload_from_roots(skill_roots);
                        let rollback_message = rollback_error
                            .err()
                            .map(|rollback| format!(" rollback failed: {}", rollback))
                            .unwrap_or_default();
                        let restore_message = restore_error
                            .err()
                            .map(|restore| format!(" runtime restore failed: {}", restore))
                            .unwrap_or_default();
                        let message = format!(
                            "Failed to finalize {:?}: {}.{}{}",
                            action, error, rollback_message, restore_message
                        );
                        progress.emit("failed", "failed", Some(message.clone()));
                        message.into()
                    },
                )?;
                progress.emit(
                    "committing",
                    "completed",
                    Some("skill lifecycle change committed".to_string()),
                );
                committed
            }
        };
        if action == crate::skill::manager::SkillLifecycleAction::Update
            && result.status == "updated"
        {
            let current_dependency_manifest =
                resolve_declared_skill_instance_from_roots(operation_roots, &result.skill_id)
                    .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?
                    .and_then(|resolved| {
                        self.load_skill_dependency_manifest(&resolved.actual_dir)
                            .transpose()
                    })
                    .transpose()
                    .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
            let dependency_manager = DependencyManager::new(
                self.dependency_manager_config_for(&target_root)
                    .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?,
            );
            if let Err(error) = dependency_manager.cleanup_updated_skill_dependencies(
                &result.skill_id,
                previous_dependency_manifest.as_ref(),
                current_dependency_manifest.as_ref(),
            ) {
                log_warn(format!(
                    "[LuaSkills:update] Stale dependency cleanup warning for skill '{}': {}",
                    result.skill_id, error
                ));
                result.message = format!(
                    "{} (warning: stale dependency cleanup failed: {})",
                    result.message, error
                );
            }
        }
        let resolved_instance =
            resolve_declared_skill_instance_from_roots(operation_roots, &result.skill_id)
                .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        self.emit_skill_lifecycle_event(
            plane,
            action,
            &result.skill_id,
            resolved_instance
                .as_ref()
                .map(|instance| instance.root_name.clone()),
            resolved_instance
                .as_ref()
                .map(|instance| instance.actual_dir.display().to_string()),
            &result.status,
            Some(result.message.clone()),
        );
        progress.emit("completed", "completed", Some(result.message.clone()));
        Ok(result)
    }

    /// Mark one skill disabled through the ordinary skills plane using an ordered root chain and immediately reload the runtime view.
    /// 通过普通 skills 平面使用有序根目录链将单个技能标记为停用，并立即重载运行时视图。
    pub fn disable_skill_in_roots(
        &mut self,
        skill_roots: &[RuntimeSkillRoot],
        skill_id: &str,
        reason: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.mutate_skill_state_and_reload(
            SkillOperationPlane::Skills,
            crate::skill::manager::SkillLifecycleAction::Disable,
            skill_roots,
            skill_id,
            reason,
        )
    }

    /// Mark one skill disabled through the host-controlled system plane using an ordered root chain and immediately reload the runtime view.
    /// 通过宿主控制的 system 平面使用有序根目录链将单个技能标记为停用，并立即重载运行时视图。
    pub fn system_disable_skill_in_roots(
        &mut self,
        skill_roots: &[RuntimeSkillRoot],
        authority: SkillManagementAuthority,
        skill_id: &str,
        reason: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.mutate_skill_state_and_reload(
            Self::operation_plane_for_authority(authority),
            crate::skill::manager::SkillLifecycleAction::Disable,
            skill_roots,
            skill_id,
            reason,
        )
    }

    /// Remove the disabled marker of one skill through the ordinary skills plane and immediately reload the runtime view.
    /// 通过普通 skills 平面移除单个技能的停用标记，并立即重载运行时视图。
    pub fn enable_skill(
        &mut self,
        skill_roots: &[RuntimeSkillRoot],
        skill_id: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.mutate_skill_state_and_reload(
            SkillOperationPlane::Skills,
            crate::skill::manager::SkillLifecycleAction::Enable,
            skill_roots,
            skill_id,
            None,
        )
    }

    /// Remove the disabled marker of one skill through the host-controlled system plane and immediately reload the runtime view.
    /// 通过宿主控制的 system 平面移除单个技能的停用标记，并立即重载运行时视图。
    pub fn system_enable_skill(
        &mut self,
        skill_roots: &[RuntimeSkillRoot],
        authority: SkillManagementAuthority,
        skill_id: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.mutate_skill_state_and_reload(
            Self::operation_plane_for_authority(authority),
            crate::skill::manager::SkillLifecycleAction::Enable,
            skill_roots,
            skill_id,
            None,
        )
    }

    /// Uninstall one skill directory through the ordinary skills plane and immediately reload the runtime view.
    /// 通过普通 skills 平面卸载单个技能目录，并立即重载运行时视图。
    pub fn uninstall_skill(
        &mut self,
        skill_roots: &[RuntimeSkillRoot],
        skill_id: &str,
        options: &SkillUninstallOptions,
    ) -> Result<SkillUninstallResult, Box<dyn std::error::Error>> {
        self.uninstall_skill_and_reload(SkillOperationPlane::Skills, skill_roots, skill_id, options)
    }

    /// Uninstall one skill through the ordinary skills plane from an explicit PROJECT or USER root.
    /// 通过普通 skills 平面从显式 PROJECT 或 USER 根卸载单个技能。
    pub fn uninstall_skill_in_root(
        &mut self,
        skill_roots: &[RuntimeSkillRoot],
        target_root: &RuntimeSkillRoot,
        skill_id: &str,
        options: &SkillUninstallOptions,
    ) -> Result<SkillUninstallResult, Box<dyn std::error::Error>> {
        Self::validate_formal_skill_root_chain(skill_roots)
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        Self::validate_ordinary_target_root(
            skill_roots,
            target_root,
            crate::skill::manager::SkillLifecycleAction::Uninstall,
        )
        .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        self.uninstall_skill_and_reload_in_root(
            SkillOperationPlane::Skills,
            skill_roots,
            Some(target_root),
            skill_id,
            options,
        )
    }

    /// Uninstall one skill directory through the host-controlled system plane and immediately reload the runtime view.
    /// 通过宿主控制的 system 平面卸载单个技能目录，并立即重载运行时视图。
    pub fn system_uninstall_skill(
        &mut self,
        skill_roots: &[RuntimeSkillRoot],
        authority: SkillManagementAuthority,
        skill_id: &str,
        options: &SkillUninstallOptions,
    ) -> Result<SkillUninstallResult, Box<dyn std::error::Error>> {
        self.uninstall_skill_and_reload(
            Self::operation_plane_for_authority(authority),
            skill_roots,
            skill_id,
            options,
        )
    }

    /// Uninstall one skill through the host-controlled system plane from an explicit target root.
    /// 通过宿主控制的 system 平面从显式目标根卸载单个技能。
    pub fn system_uninstall_skill_in_root(
        &mut self,
        skill_roots: &[RuntimeSkillRoot],
        target_root: &RuntimeSkillRoot,
        authority: SkillManagementAuthority,
        skill_id: &str,
        options: &SkillUninstallOptions,
    ) -> Result<SkillUninstallResult, Box<dyn std::error::Error>> {
        Self::validate_formal_skill_root_chain(skill_roots)
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        Self::runtime_skill_root_index(skill_roots, target_root)
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        Self::validate_authority_for_target_root(
            authority,
            target_root,
            crate::skill::manager::SkillLifecycleAction::Uninstall,
        )
        .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        let plane = Self::operation_plane_for_authority(authority);
        self.uninstall_skill_and_reload_in_root(
            plane,
            skill_roots,
            Some(target_root),
            skill_id,
            options,
        )
    }

    /// Preflight one install request through the ordinary skills plane and return a structured result.
    /// 通过普通 skills 平面对一次安装请求执行预检查，并返回结构化结果。
    pub fn install_skill(
        &mut self,
        skill_roots: &[RuntimeSkillRoot],
        request: &SkillInstallRequest,
    ) -> Result<SkillApplyResult, Box<dyn std::error::Error>> {
        self.apply_skill_request(
            SkillOperationPlane::Skills,
            crate::skill::manager::SkillLifecycleAction::Install,
            skill_roots,
            request,
        )
    }

    /// Preflight one install request through the ordinary skills plane into an explicit PROJECT or USER root.
    /// 通过普通 skills 平面将一次安装请求预检查并写入显式 PROJECT 或 USER 根。
    pub fn install_skill_in_root(
        &mut self,
        skill_roots: &[RuntimeSkillRoot],
        target_root: &RuntimeSkillRoot,
        request: &SkillInstallRequest,
    ) -> Result<SkillApplyResult, Box<dyn std::error::Error>> {
        Self::validate_formal_skill_root_chain(skill_roots)
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        Self::validate_ordinary_target_root(
            skill_roots,
            target_root,
            crate::skill::manager::SkillLifecycleAction::Install,
        )
        .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        self.apply_skill_request_in_root(
            SkillOperationPlane::Skills,
            crate::skill::manager::SkillLifecycleAction::Install,
            skill_roots,
            Some(target_root),
            request,
        )
    }

    /// Preflight one install request through the host-controlled system plane and return a structured result.
    /// 通过宿主控制的 system 平面对一次安装请求执行预检查，并返回结构化结果。
    pub fn system_install_skill(
        &mut self,
        skill_roots: &[RuntimeSkillRoot],
        authority: SkillManagementAuthority,
        request: &SkillInstallRequest,
    ) -> Result<SkillApplyResult, Box<dyn std::error::Error>> {
        self.apply_skill_request(
            Self::operation_plane_for_authority(authority),
            crate::skill::manager::SkillLifecycleAction::Install,
            skill_roots,
            request,
        )
    }

    /// Preflight one install request through the host-controlled system plane into an explicit target root.
    /// 通过宿主控制的 system 平面将一次安装请求预检查并写入显式目标根。
    pub fn system_install_skill_in_root(
        &mut self,
        skill_roots: &[RuntimeSkillRoot],
        target_root: &RuntimeSkillRoot,
        authority: SkillManagementAuthority,
        request: &SkillInstallRequest,
    ) -> Result<SkillApplyResult, Box<dyn std::error::Error>> {
        Self::validate_formal_skill_root_chain(skill_roots)
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        Self::runtime_skill_root_index(skill_roots, target_root)
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        Self::validate_authority_for_target_root(
            authority,
            target_root,
            crate::skill::manager::SkillLifecycleAction::Install,
        )
        .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        let plane = Self::operation_plane_for_authority(authority);
        self.apply_skill_request_in_root(
            plane,
            crate::skill::manager::SkillLifecycleAction::Install,
            skill_roots,
            Some(target_root),
            request,
        )
    }

    /// Preflight one update request through the ordinary skills plane and return a structured result.
    /// 通过普通 skills 平面对一次更新请求执行预检查，并返回结构化结果。
    pub fn update_skill(
        &mut self,
        skill_roots: &[RuntimeSkillRoot],
        request: &SkillInstallRequest,
    ) -> Result<SkillApplyResult, Box<dyn std::error::Error>> {
        self.apply_skill_request(
            SkillOperationPlane::Skills,
            crate::skill::manager::SkillLifecycleAction::Update,
            skill_roots,
            request,
        )
    }

    /// Preflight one update request through the ordinary skills plane against an explicit PROJECT or USER root.
    /// 通过普通 skills 平面对显式 PROJECT 或 USER 根执行一次更新预检查。
    pub fn update_skill_in_root(
        &mut self,
        skill_roots: &[RuntimeSkillRoot],
        target_root: &RuntimeSkillRoot,
        request: &SkillInstallRequest,
    ) -> Result<SkillApplyResult, Box<dyn std::error::Error>> {
        Self::validate_formal_skill_root_chain(skill_roots)
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        Self::validate_ordinary_target_root(
            skill_roots,
            target_root,
            crate::skill::manager::SkillLifecycleAction::Update,
        )
        .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        self.apply_skill_request_in_root(
            SkillOperationPlane::Skills,
            crate::skill::manager::SkillLifecycleAction::Update,
            skill_roots,
            Some(target_root),
            request,
        )
    }

    /// Preflight one update request through the host-controlled system plane and return a structured result.
    /// 通过宿主控制的 system 平面对一次更新请求执行预检查，并返回结构化结果。
    pub fn system_update_skill(
        &mut self,
        skill_roots: &[RuntimeSkillRoot],
        authority: SkillManagementAuthority,
        request: &SkillInstallRequest,
    ) -> Result<SkillApplyResult, Box<dyn std::error::Error>> {
        self.apply_skill_request(
            Self::operation_plane_for_authority(authority),
            crate::skill::manager::SkillLifecycleAction::Update,
            skill_roots,
            request,
        )
    }

    /// Preflight one update request through the host-controlled system plane against an explicit target root.
    /// 通过宿主控制的 system 平面对显式目标根执行一次更新预检查。
    pub fn system_update_skill_in_root(
        &mut self,
        skill_roots: &[RuntimeSkillRoot],
        target_root: &RuntimeSkillRoot,
        authority: SkillManagementAuthority,
        request: &SkillInstallRequest,
    ) -> Result<SkillApplyResult, Box<dyn std::error::Error>> {
        Self::validate_formal_skill_root_chain(skill_roots)
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        Self::runtime_skill_root_index(skill_roots, target_root)
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        Self::validate_authority_for_target_root(
            authority,
            target_root,
            crate::skill::manager::SkillLifecycleAction::Update,
        )
        .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        let plane = Self::operation_plane_for_authority(authority);
        self.apply_skill_request_in_root(
            plane,
            crate::skill::manager::SkillLifecycleAction::Update,
            skill_roots,
            Some(target_root),
            request,
        )
    }

    /// Emit one structured lifecycle event through the host callback bridge.
    /// 通过宿主回调桥发送一条结构化生命周期事件。
    fn emit_skill_lifecycle_event(
        &self,
        plane: SkillOperationPlane,
        action: crate::skill::manager::SkillLifecycleAction,
        skill_id: &str,
        root_name: Option<String>,
        skill_dir: Option<String>,
        status: &str,
        message: Option<String>,
    ) {
        crate::host::callbacks::emit_skill_lifecycle_event(&RuntimeSkillLifecycleEvent {
            plane,
            action,
            skill_id: skill_id.to_string(),
            root_name,
            skill_dir,
            status: status.to_string(),
            message,
        });
    }

    /// Compare pre-reload and post-reload entry snapshots and emit one host-visible registry delta.
    /// 对比重载前后入口快照并发出一条宿主可见的注册表差异事件。
    fn emit_entry_registry_delta(&self, previous_entries: Vec<RuntimeEntryDescriptor>) {
        let current_entries = self.list_entries();
        let previous_map = previous_entries
            .into_iter()
            .map(|entry| (entry.canonical_name.clone(), entry))
            .collect::<BTreeMap<String, RuntimeEntryDescriptor>>();
        let current_map = current_entries
            .into_iter()
            .map(|entry| (entry.canonical_name.clone(), entry))
            .collect::<BTreeMap<String, RuntimeEntryDescriptor>>();

        let mut added_entries = Vec::new();
        let mut updated_entries = Vec::new();
        let mut removed_entry_names = Vec::new();

        for (canonical_name, current_entry) in &current_map {
            match previous_map.get(canonical_name) {
                None => added_entries.push(current_entry.clone()),
                Some(previous_entry) if previous_entry != current_entry => {
                    updated_entries.push(current_entry.clone());
                }
                Some(_) => {}
            }
        }

        for canonical_name in previous_map.keys() {
            if !current_map.contains_key(canonical_name) {
                removed_entry_names.push(canonical_name.clone());
            }
        }

        if added_entries.is_empty() && updated_entries.is_empty() && removed_entry_names.is_empty()
        {
            return;
        }

        crate::host::callbacks::emit_entry_registry_delta(&RuntimeEntryRegistryDelta {
            added_entries,
            removed_entry_names,
            updated_entries,
        });
    }

    /// Load a single skill from its directory.
    fn load_single_skill(
        &mut self,
        dir: &Path,
        root_name: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let skill_yaml = dir.join("skill.yaml");
        if !skill_yaml.exists() {
            return Err(format!("skill.yaml not found in {}", dir.display()).into());
        }

        let yaml_str = std::fs::read_to_string(&skill_yaml)?;
        let yaml_value: serde_yaml::Value = serde_yaml::from_str(&yaml_str)?;
        if yaml_value.as_mapping().is_some_and(|mapping| {
            mapping.contains_key(serde_yaml::Value::String("skill_id".to_string()))
        }) {
            return Err(format!("skill {} must not declare skill_id in skill.yaml; directory name is the only skill_id", dir.display())
            .into());
        }
        let mut meta: SkillMeta = serde_yaml::from_value(yaml_value)?;
        let directory_skill_id = dir
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| format!("invalid skill directory name: {}", dir.display()))?
            .trim()
            .to_string();
        validate_luaskills_identifier(&directory_skill_id, "skill directory name")
            .map_err(|error| format!("skill {}: {}", dir.display(), error))?;
        meta.bind_directory_skill_id(directory_skill_id.clone());

        if !meta.is_enabled() {
            log_info(format!(
                "[LuaSkill] Skip disabled skill '{}'",
                meta.effective_skill_id()
            ));
            return Ok(());
        }
        meta.resolve_entry_input_schemas(dir)
            .map_err(|error| format!("skill {}: {}", meta.name, error))?;
        validate_luaskills_identifier(meta.effective_skill_id(), "skill_id")
            .map_err(|error| format!("skill {}: {}", meta.name, error))?;
        validate_luaskills_version(meta.version(), "version")
            .map_err(|error| format!("skill {}: {}", meta.effective_skill_id(), error))?;

        if meta.entries.is_empty() {
            return Err(format!("skill {} must declare at least one entry", meta.name).into());
        }

        for tool in meta.entries() {
            validate_luaskills_identifier(tool.name.trim(), "entry.name").map_err(|error| {
                format!("skill {} entry {}: {}", meta.name, tool.name.trim(), error)
            })?;
            if tool.lua_entry.trim().is_empty() || tool.lua_module.trim().is_empty() {
                return Err(format!(
                    "skill {} declares entry {} but lua_entry/lua_module is missing",
                    meta.name, tool.name
                )
                .into());
            }

            validate_skill_relative_path(&tool.lua_entry, "runtime", "entry.lua_entry")
                .map_err(|error| format!("skill {} entry {}: {}", meta.name, tool.name, error))?;

            let lua_path = tool_entry_path(dir, tool);
            if !lua_path.exists() {
                return Err(format!(
                    "Lua entry {} not found in {}",
                    tool.lua_entry,
                    dir.display()
                )
                .into());
            }
        }

        if !meta.help.main.file.trim().is_empty() {
            validate_skill_relative_path(&meta.help.main.file, "help", "help.main.file")
                .map_err(|error| format!("skill {} help main: {}", meta.name, error))?;
        }
        for topic in &meta.help.topics {
            validate_skill_relative_path(&topic.file, "help", "help.topic.file").map_err(
                |error| {
                    format!(
                        "skill {} help topic {}: {}",
                        meta.name,
                        topic.name.trim(),
                        error
                    )
                },
            )?;
        }

        let effective_lancedb = meta.effective_lancedb();
        let lancedb_binding = if effective_lancedb.enable {
            if self.lancedb_host.is_none() {
                self.lancedb_host = Some(Arc::new(
                    LanceDbSkillHost::new(
                        self.host_options.as_ref().clone(),
                        self.database_provider_callbacks.clone(),
                    )
                    .map_err(|error| {
                        format!("Failed to initialize LanceDB skill host: {}", error)
                    })?,
                ));
            }

            let host = self
                .lancedb_host
                .as_ref()
                .ok_or("LanceDB skill host missing after initialization")?
                .clone();

            Some(
                host.register_skill(root_name, meta.effective_skill_id(), dir, effective_lancedb)
                    .map_err(|error| {
                        format!(
                            "Failed to register LanceDB for skill {}: {}",
                            meta.effective_skill_id(),
                            error
                        )
                    })?,
            )
        } else {
            None
        };

        let effective_sqlite = meta.effective_sqlite();
        let sqlite_binding = if effective_sqlite.enable {
            if self.sqlite_host.is_none() {
                self.sqlite_host = Some(Arc::new(
                    SqliteSkillHost::new(
                        self.host_options.as_ref().clone(),
                        self.database_provider_callbacks.clone(),
                    )
                    .map_err(|error| {
                        format!("Failed to initialize SQLite skill host: {}", error)
                    })?,
                ));
            }

            let host = self
                .sqlite_host
                .as_ref()
                .ok_or("SQLite skill host missing after initialization")?
                .clone();

            Some(
                host.register_skill(root_name, meta.effective_skill_id(), dir, effective_sqlite)
                    .map_err(|error| {
                        format!(
                            "Failed to register SQLite for skill {}: {}",
                            meta.effective_skill_id(),
                            error
                        )
                    })?,
            )
        } else {
            None
        };

        self.skills.insert(
            meta.effective_skill_id().to_string(),
            LoadedSkill {
                meta,
                dir: dir.to_path_buf(),
                root_name: root_name.to_string(),
                lancedb_binding,
                sqlite_binding,
                resolved_entry_names: HashMap::new(),
            },
        );

        Ok(())
    }

    /// Build a fresh Lua VM instance from one explicit runtime state snapshot.
    /// 基于一份显式运行时状态快照创建全新的 Lua 虚拟机实例。
    fn create_vm_with_runtime_state(
        &self,
        skills: HashMap<String, LoadedSkill>,
        entry_registry: BTreeMap<String, ResolvedEntryTarget>,
    ) -> Result<LuaVm, String> {
        let skills = Arc::new(skills);
        let entry_registry = Arc::new(entry_registry);
        let lua = unsafe { Lua::unsafe_new() };
        Self::setup_package_paths(&lua, self.host_options.as_ref())
            .map_err(|error| error.to_string())?;
        Self::register_vulcan_module(
            &lua,
            self.host_options.as_ref(),
            self.skill_config_store.clone(),
            &self.runtime_skill_roots,
        )
        .map_err(|error| error.to_string())?;
        Self::populate_vulcan_luaexec_bridge(
            &lua,
            self.host_options.clone(),
            self.runlua_pool.clone(),
            self.skill_config_store.clone(),
            skills.clone(),
            entry_registry.clone(),
            self.runtime_skill_roots.clone(),
            self.lancedb_host.clone(),
            self.sqlite_host.clone(),
        )?;
        Self::register_skill_functions(&lua, skills.as_ref())?;
        Self::populate_vulcan_call_for_lua(
            &lua,
            skills.as_ref(),
            entry_registry.as_ref(),
            self.host_options.clone(),
            self.lancedb_host.clone(),
            self.sqlite_host.clone(),
        )?;
        Ok(LuaVm {
            lua,
            last_used_at: Instant::now(),
        })
    }

    /// Build a fresh Lua VM instance with all loaded skills registered.
    /// 创建一个全新的 Lua 虚拟机实例，并注册当前已加载的全部技能。
    fn create_vm(&self) -> Result<LuaVm, String> {
        self.create_vm_with_runtime_state(self.skills.clone(), self.entry_registry.clone())
    }

    /// Borrow a Lua VM from the pool for one operation.
    /// 从虚拟机池借出一个 Lua 实例执行一次操作。
    fn acquire_vm(&self) -> Result<LuaVmLease, String> {
        self.pool.acquire(|| self.create_vm())
    }

    /// Build a fresh isolated runlua VM instance with current runtime state registered.
    /// 创建一个带有当前运行时状态注册信息的全新隔离 runlua 虚拟机实例。
    fn create_runlua_vm(
        skills: &HashMap<String, LoadedSkill>,
        entry_registry: &BTreeMap<String, ResolvedEntryTarget>,
        host_options: Arc<LuaRuntimeHostOptions>,
        skill_config_store: Arc<SkillConfigStore>,
        runtime_skill_roots: Vec<RuntimeSkillRoot>,
        lancedb_host: Option<Arc<LanceDbSkillHost>>,
        sqlite_host: Option<Arc<SqliteSkillHost>>,
    ) -> Result<LuaVm, String> {
        let lua = unsafe { Lua::unsafe_new() };
        Self::setup_package_paths(&lua, host_options.as_ref())
            .map_err(|error| error.to_string())?;
        Self::register_vulcan_module(
            &lua,
            host_options.as_ref(),
            skill_config_store,
            &runtime_skill_roots,
        )
        .map_err(|error| error.to_string())?;
        Self::register_skill_functions(&lua, skills)?;
        Self::populate_vulcan_call_for_lua(
            &lua,
            skills,
            entry_registry,
            host_options,
            lancedb_host,
            sqlite_host,
        )?;
        Ok(LuaVm {
            lua,
            last_used_at: Instant::now(),
        })
    }

    /// Register all tool-bearing skill entries into a specific Lua VM.
    /// 将所有声明了工具入口的 skill 条目注册到指定 Lua 虚拟机中。
    fn register_skill_functions(
        lua: &Lua,
        skills: &HashMap<String, LoadedSkill>,
    ) -> Result<(), String> {
        for skill in skills.values() {
            for tool in skill.meta.entries() {
                Self::compile_skill_into_lua(lua, skill, tool, false)?;
            }
        }
        Ok(())
    }

    /// Compile one tool entry into the target Lua VM.
    /// 将单个工具入口编译并注册到目标 Lua 虚拟机中。
    fn compile_skill_into_lua(
        lua: &Lua,
        skill: &LoadedSkill,
        tool: &crate::lua_skill::SkillToolMeta,
        always_reload: bool,
    ) -> Result<(), String> {
        let lua_path = tool_entry_path(&skill.dir, tool);
        let source = std::fs::read_to_string(&lua_path)
            .map_err(|error| format!("Failed to read {}: {}", lua_path.display(), error))?;
        if always_reload {
            log_info(format!(
                "[LuaSkill] Hot reload {}: {}",
                tool.lua_module,
                render_log_friendly_path(&lua_path)
            ));
        }

        let chunk = lua.load(&source).set_name(&tool.lua_module);
        let outer: Function = chunk.into_function().map_err(|error| {
            format!(
                "Failed to compile skill '{}::{}': {}",
                skill.meta.name, tool.lua_module, error
            )
        })?;
        let handler: Function = outer.call(()).map_err(|error| {
            format!(
                "Failed to initialize skill '{}::{}': {}",
                skill.meta.name, tool.lua_module, error
            )
        })?;
        lua.globals()
            .set(format!("__skill_{}", tool.lua_module), handler)
            .map_err(|error| {
                format!(
                    "Failed to register skill '{}::{}': {}",
                    skill.meta.name, tool.lua_module, error
                )
            })?;
        Ok(())
    }

    /// Return generic runtime entry descriptors for all loaded skills.
    /// 返回当前已加载全部 skill 的通用运行时入口描述。
    pub fn list_entries(&self) -> Vec<RuntimeEntryDescriptor> {
        self.entry_registry
            .values()
            .filter_map(|target| {
                let skill = self.skills.get(&target.skill_storage_key)?;
                let tool = skill.meta.find_tool_by_local_name(&target.local_name)?;
                Some(RuntimeEntryDescriptor {
                    canonical_name: target.canonical_name.clone(),
                    skill_id: target.skill_id.clone(),
                    local_name: target.local_name.clone(),
                    root_name: skill.root_name.clone(),
                    skill_dir: skill.dir.display().to_string(),
                    description: tool.description.clone(),
                    parameters: tool
                        .parameters
                        .iter()
                        .map(|parameter| RuntimeEntryParameterDescriptor {
                            name: parameter.name.clone(),
                            param_type: parameter.param_type.clone(),
                            description: parameter.description.clone(),
                            required: parameter.required,
                        })
                        .collect(),
                    input_schema: tool.resolved_input_schema().clone(),
                })
            })
            .collect()
    }

    /// Return runtime entry descriptors visible to one host-injected skill-management authority.
    /// 返回单个宿主注入技能管理权限可见的运行时入口描述。
    pub fn list_entries_for_authority(
        &self,
        authority: SkillManagementAuthority,
    ) -> Vec<RuntimeEntryDescriptor> {
        self.list_entries()
            .into_iter()
            .filter(|entry| {
                authority == SkillManagementAuthority::System
                    || Self::normalized_skill_root_name(&entry.root_name) != "ROOT"
            })
            .collect()
    }

    /// List all structured help trees currently registered in the runtime.
    /// 列出当前运行时中已注册的全部结构化帮助树。
    pub fn list_skill_help(&self) -> Vec<RuntimeSkillHelpDescriptor> {
        let mut descriptors = self
            .skills
            .values()
            .map(|skill| RuntimeSkillHelpDescriptor {
                skill_id: skill.meta.effective_skill_id().to_string(),
                skill_name: skill.meta.name.clone(),
                skill_version: skill.meta.version().to_string(),
                root_name: skill.root_name.clone(),
                skill_dir: skill.dir.display().to_string(),
                main: self.build_help_node_descriptor(skill, skill.meta.main_help(), true),
                flows: skill
                    .meta
                    .help_topics()
                    .map(|topic| self.build_help_node_descriptor(skill, topic, false))
                    .collect::<Vec<RuntimeHelpNodeDescriptor>>(),
            })
            .collect::<Vec<RuntimeSkillHelpDescriptor>>();

        descriptors.sort_by(|left, right| left.skill_id.cmp(&right.skill_id));
        descriptors
    }

    /// List structured help trees visible to one host-injected skill-management authority.
    /// 列出单个宿主注入技能管理权限可见的结构化帮助树。
    pub fn list_skill_help_for_authority(
        &self,
        authority: SkillManagementAuthority,
    ) -> Vec<RuntimeSkillHelpDescriptor> {
        self.list_skill_help()
            .into_iter()
            .filter(|help| {
                authority == SkillManagementAuthority::System
                    || Self::normalized_skill_root_name(&help.root_name) != "ROOT"
            })
            .collect()
    }

    /// Render one structured help detail payload for one skill flow node.
    /// 为单个 skill 流程节点渲染一份结构化帮助详情载荷。
    pub fn render_skill_help_detail(
        &self,
        skill_id: &str,
        flow_name: &str,
        request_context: Option<&RuntimeRequestContext>,
    ) -> Result<Option<RuntimeHelpDetail>, String> {
        let Some(skill) = self
            .skills
            .values()
            .find(|skill| skill.meta.effective_skill_id() == skill_id)
        else {
            return Ok(None);
        };

        let normalized_flow_name = flow_name.trim();
        if normalized_flow_name.is_empty() {
            return Err("Help flow name must not be empty".to_string());
        }

        let (selected_help, is_main) = if normalized_flow_name == "main" {
            (skill.meta.main_help(), true)
        } else {
            (
                skill
                    .meta
                    .find_help_topic(normalized_flow_name)
                    .ok_or_else(|| {
                        format!(
                            "Skill '{}' does not declare help flow '{}'",
                            skill.meta.effective_skill_id(),
                            normalized_flow_name
                        )
                    })?,
                false,
            )
        };

        let rendered_body =
            self.render_help_payload(skill, &selected_help.file, request_context)?;
        let descriptor = self.build_help_node_descriptor(skill, selected_help, is_main);
        Ok(Some(RuntimeHelpDetail {
            skill_id: skill.meta.effective_skill_id().to_string(),
            skill_name: skill.meta.name.clone(),
            skill_version: skill.meta.version().to_string(),
            root_name: skill.root_name.clone(),
            skill_dir: skill.dir.display().to_string(),
            flow_name: descriptor.flow_name,
            description: descriptor.description,
            related_entries: descriptor.related_entries,
            is_main: descriptor.is_main,
            content_type: "markdown".to_string(),
            content: rendered_body,
        }))
    }

    /// Render help detail only when visible to one host-injected skill-management authority.
    /// 仅在单个宿主注入技能管理权限可见时渲染帮助详情。
    pub fn render_skill_help_detail_for_authority(
        &self,
        authority: SkillManagementAuthority,
        skill_id: &str,
        flow_name: &str,
        request_context: Option<&RuntimeRequestContext>,
    ) -> Result<Option<RuntimeHelpDetail>, String> {
        if authority == SkillManagementAuthority::DelegatedTool
            && self
                .skills
                .values()
                .find(|skill| skill.meta.effective_skill_id() == skill_id)
                .map(|skill| Self::normalized_skill_root_name(&skill.root_name) == "ROOT")
                .unwrap_or(false)
        {
            return Ok(None);
        }
        self.render_skill_help_detail(skill_id, flow_name, request_context)
    }

    /// Build one structured help node descriptor with related canonical entries.
    /// 构建单个结构化帮助节点描述及其关联 canonical 入口列表。
    fn build_help_node_descriptor(
        &self,
        skill: &LoadedSkill,
        help_node: &crate::lua_skill::SkillHelpNodeMeta,
        is_main: bool,
    ) -> RuntimeHelpNodeDescriptor {
        let flow_name = if is_main {
            "main".to_string()
        } else {
            help_node.name.trim().to_string()
        };
        let related_entries = if is_main {
            skill
                .meta
                .entries()
                .filter_map(|entry| {
                    skill
                        .resolved_tool_name(entry.name.trim())
                        .map(str::to_string)
                })
                .collect::<Vec<String>>()
        } else {
            skill
                .meta
                .entries_for_help_topic(help_node.name.trim())
                .filter_map(|entry| {
                    skill
                        .resolved_tool_name(entry.name.trim())
                        .map(str::to_string)
                })
                .collect::<Vec<String>>()
        };

        RuntimeHelpNodeDescriptor {
            flow_name,
            description: help_node.description.trim().to_string(),
            related_entries,
            is_main,
        }
    }

    /// Return configured completion candidates for a prompt argument, if declared by a skill.
    /// 返回某个提示词参数在 skill 元数据中声明的候选补全项。
    pub fn prompt_argument_completions(
        &self,
        prompt_name: &str,
        argument_name: &str,
    ) -> Option<Vec<String>> {
        let _ = prompt_name;
        let _ = argument_name;
        None
    }

    /// Return configured completion candidates visible to one host-injected authority.
    /// 返回单个宿主注入权限可见的提示词参数补全候选项。
    pub fn prompt_argument_completions_for_authority(
        &self,
        authority: SkillManagementAuthority,
        prompt_name: &str,
        argument_name: &str,
    ) -> Option<Vec<String>> {
        let _ = authority;
        self.prompt_argument_completions(prompt_name, argument_name)
    }

    /// Return whether one resolved entry target is visible to one host-injected authority.
    /// 返回单个已解析入口目标是否对某个宿主注入权限可见。
    fn entry_target_visible_to_authority(
        &self,
        authority: SkillManagementAuthority,
        target: &ResolvedEntryTarget,
    ) -> bool {
        if authority == SkillManagementAuthority::System {
            return true;
        }
        self.skills
            .get(&target.skill_storage_key)
            .map(|skill| Self::normalized_skill_root_name(&skill.root_name) != "ROOT")
            .unwrap_or(false)
    }

    /// Check if a tool_name is a Lua skill.
    /// 检查单个工具名是否为 Lua skill 入口。
    pub fn is_skill(&self, name: &str) -> bool {
        self.entry_registry.contains_key(name)
    }

    /// Check if a tool_name is visible as a Lua skill under one host-injected authority.
    /// 检查单个工具名在某个宿主注入权限下是否可见为 Lua skill 入口。
    pub fn is_skill_for_authority(&self, authority: SkillManagementAuthority, name: &str) -> bool {
        self.entry_registry
            .get(name)
            .map(|target| self.entry_target_visible_to_authority(authority, target))
            .unwrap_or(false)
    }

    /// Return the owning skill name for an MCP tool name; return `None` when the tool is not provided by a Lua skill.
    /// 根据 MCP 工具名返回所属 skill 名称；未命中时返回 `None`。
    pub fn skill_name_for_tool(&self, tool_name: &str) -> Option<String> {
        self.entry_registry
            .get(tool_name)
            .map(|target| target.skill_id.clone())
    }

    /// Return the visible owning skill name for one tool under one host-injected authority.
    /// 返回某个工具在单个宿主注入权限下可见的所属 skill 名称。
    pub fn skill_name_for_tool_for_authority(
        &self,
        authority: SkillManagementAuthority,
        tool_name: &str,
    ) -> Option<String> {
        self.entry_registry.get(tool_name).and_then(|target| {
            self.entry_target_visible_to_authority(authority, target)
                .then(|| target.skill_id.clone())
        })
    }

    /// List flattened skill config records for one optional skill namespace.
    /// 列出某个可选技能命名空间下的扁平化技能配置记录。
    pub fn list_skill_config_entries(
        &self,
        skill_id: Option<&str>,
    ) -> Result<Vec<SkillConfigEntry>, String> {
        self.skill_config_store.list_entries(skill_id)
    }

    /// Read one optional string config value for one `(skill_id, key)` pair.
    /// 读取某个 `(skill_id, key)` 对下的可选字符串配置值。
    pub fn get_skill_config_value(
        &self,
        skill_id: &str,
        key: &str,
    ) -> Result<Option<String>, String> {
        self.skill_config_store.get_value(skill_id, key)
    }

    /// Insert or replace one string config value for one `(skill_id, key)` pair.
    /// 为某个 `(skill_id, key)` 对插入或替换一个字符串配置值。
    pub fn set_skill_config_value(
        &mut self,
        skill_id: &str,
        key: &str,
        value: &str,
    ) -> Result<(), String> {
        self.skill_config_store.set_value(skill_id, key, value)
    }

    /// Delete one config key from one skill namespace and report whether one value was removed.
    /// 从某个技能命名空间删除单个配置键，并返回是否移除了一个值。
    pub fn delete_skill_config_value(&mut self, skill_id: &str, key: &str) -> Result<bool, String> {
        self.skill_config_store.delete_value(skill_id, key)
    }

    /// Populate per-request context into the `vulcan` module.
    /// 将单次请求的上下文注入到 `vulcan` 模块中。
    fn populate_vulcan_request_context(
        lua: &Lua,
        invocation_context: Option<&LuaInvocationContext>,
    ) -> Result<(), String> {
        let context_table = get_vulcan_context_table(lua)?;
        let request_context =
            invocation_context.and_then(|context| context.request_context.as_ref());
        let context_value = match request_context {
            Some(context) => serde_json::to_value(context)
                .map_err(|error| format!("Failed to serialize request context: {}", error))?,
            None => Value::Object(serde_json::Map::new()),
        };
        let context_lua = json_value_to_lua(lua, &context_value)
            .map_err(|error| format!("Failed to convert request context to Lua: {}", error))?;
        let client_info_value = match &context_value {
            Value::Object(object) => object.get("client_info").cloned().unwrap_or(Value::Null),
            _ => Value::Null,
        };
        let client_capabilities_value = match &context_value {
            Value::Object(object) => object
                .get("client_capabilities")
                .cloned()
                .unwrap_or_else(|| Value::Object(serde_json::Map::new())),
            _ => Value::Object(serde_json::Map::new()),
        };
        let client_info_lua = json_value_to_lua(lua, &client_info_value)
            .map_err(|error| format!("Failed to convert client_info to Lua: {}", error))?;
        let client_capabilities_lua = json_value_to_lua(lua, &client_capabilities_value)
            .map_err(|error| format!("Failed to convert client_capabilities to Lua: {}", error))?;
        let client_budget_value = invocation_context
            .map(|context| context.client_budget.clone())
            .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
        let client_budget_lua = json_value_to_lua(lua, &client_budget_value)
            .map_err(|error| format!("Failed to convert client_budget to Lua: {}", error))?;
        let tool_config_value = invocation_context
            .map(|context| context.tool_config.clone())
            .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
        let tool_config_lua = json_value_to_lua(lua, &tool_config_value)
            .map_err(|error| format!("Failed to convert tool_config to Lua: {}", error))?;
        let host_result_capability = resolve_host_result_capability(invocation_context);
        let host_result_value = host_result_capability_to_json_value(&host_result_capability);
        let host_result_lua = json_value_to_lua(lua, &host_result_value)
            .map_err(|error| format!("Failed to convert host_result helper to Lua: {}", error))?;

        context_table
            .set("request", context_lua)
            .map_err(|error| format!("Failed to set vulcan.context.request: {}", error))?;
        context_table
            .set("client_info", client_info_lua)
            .map_err(|error| format!("Failed to set vulcan.context.client_info: {}", error))?;
        context_table
            .set("client_capabilities", client_capabilities_lua)
            .map_err(|error| {
                format!(
                    "Failed to set vulcan.context.client_capabilities: {}",
                    error
                )
            })?;
        context_table
            .set("client_budget", client_budget_lua)
            .map_err(|error| format!("Failed to set vulcan.context.client_budget: {}", error))?;
        context_table
            .set("tool_config", tool_config_lua)
            .map_err(|error| format!("Failed to set vulcan.context.tool_config: {}", error))?;
        context_table
            .set("host_result", host_result_lua)
            .map_err(|error| format!("Failed to set vulcan.context.host_result: {}", error))?;
        Ok(())
    }

    /// Populate the skill-scoped LanceDB host interface into the `vulcan` module.
    /// 将按 skill 作用域隔离的 LanceDB 宿主接口注入到 `vulcan` 模块中。
    fn populate_vulcan_lancedb_context(
        lua: &Lua,
        binding: Option<Arc<LanceDbSkillBinding>>,
        current_skill_name: Option<&str>,
    ) -> Result<(), String> {
        let vulcan: Table = lua
            .globals()
            .get("vulcan")
            .map_err(|error| format!("Failed to get vulcan module: {}", error))?;

        let lancedb_table = lua
            .create_table()
            .map_err(|error| format!("Failed to create vulcan.lancedb table: {}", error))?;

        let current_skill = current_skill_name.unwrap_or("");
        vulcan
            .set("__lancedb_skill_name", current_skill)
            .map_err(|error| format!("Failed to set vulcan.__lancedb_skill_name: {}", error))?;

        if let Some(binding) = binding {
            lancedb_table
                .set("enabled", true)
                .map_err(|error| format!("Failed to set vulcan.lancedb.enabled: {}", error))?;
            let info_binding = binding.clone();
            let info_fn = lua
                .create_function(move |lua, ()| {
                    json_value_to_lua(lua, &info_binding.info_json()).map_err(mlua::Error::external)
                })
                .map_err(|error| format!("Failed to create vulcan.lancedb.info: {}", error))?;
            lancedb_table
                .set("info", info_fn)
                .map_err(|error| format!("Failed to set vulcan.lancedb.info: {}", error))?;

            let status_binding = binding.clone();
            let status_fn = lua
                .create_function(move |lua, ()| {
                    json_value_to_lua(lua, &status_binding.status_json())
                        .map_err(mlua::Error::external)
                })
                .map_err(|error| format!("Failed to create vulcan.lancedb.status: {}", error))?;
            lancedb_table
                .set("status", status_fn)
                .map_err(|error| format!("Failed to set vulcan.lancedb.status: {}", error))?;

            let create_binding = binding.clone();
            let create_table_fn = lua
                .create_function(move |lua, input: LuaValue| {
                    let input_table = require_table_arg(input, "lancedb.create_table", "input")?;
                    let input_json = lua_value_to_json(&LuaValue::Table(input_table))
                        .map_err(mlua::Error::runtime)?;
                    let result = create_binding
                        .create_table_json(&input_json)
                        .map_err(mlua::Error::runtime)?;
                    json_value_to_lua(lua, &result).map_err(mlua::Error::external)
                })
                .map_err(|error| {
                    format!("Failed to create vulcan.lancedb.create_table: {}", error)
                })?;
            lancedb_table
                .set("create_table", create_table_fn)
                .map_err(|error| format!("Failed to set vulcan.lancedb.create_table: {}", error))?;

            let upsert_binding = binding.clone();
            let vector_upsert_fn = lua
                .create_function(move |lua, input: LuaValue| {
                    let input_table = require_table_arg(input, "lancedb.vector_upsert", "input")?;
                    let mut input_json = lua_value_to_json(&LuaValue::Table(input_table))
                        .map_err(mlua::Error::runtime)?;
                    let input_object = input_json.as_object_mut().ok_or_else(|| {
                        mlua::Error::runtime("lancedb.vector_upsert input must be an object")
                    })?;

                    let payload_value = if let Some(rows) = input_object.remove("rows") {
                        input_object
                            .entry("input_format".to_string())
                            .or_insert_with(|| Value::String("json".to_string()));
                        rows
                    } else if let Some(data) = input_object.remove("data") {
                        data
                    } else {
                        return Err(mlua::Error::runtime(
                            "lancedb.vector_upsert requires rows or data",
                        ));
                    };

                    let payload_bytes = match payload_value {
                        Value::String(text) => {
                            if !input_object.contains_key("input_format") {
                                input_object.insert(
                                    "input_format".to_string(),
                                    Value::String("arrow_ipc".to_string()),
                                );
                            }
                            text.into_bytes()
                        }
                        Value::Array(_) | Value::Object(_) => {
                            if !input_object.contains_key("input_format") {
                                input_object.insert(
                                    "input_format".to_string(),
                                    Value::String("json".to_string()),
                                );
                            }
                            serde_json::to_vec(&payload_value).map_err(|error| {
                                mlua::Error::runtime(format!(
                                    "failed to encode lancedb upsert payload: {}",
                                    error
                                ))
                            })?
                        }
                        _ => {
                            return Err(mlua::Error::runtime(
                                "lancedb.vector_upsert payload must be string",
                            ));
                        }
                    };

                    let result = upsert_binding
                        .vector_upsert_json(&input_json, &payload_bytes)
                        .map_err(mlua::Error::runtime)?;
                    json_value_to_lua(lua, &result).map_err(mlua::Error::external)
                })
                .map_err(|error| {
                    format!("Failed to create vulcan.lancedb.vector_upsert: {}", error)
                })?;
            lancedb_table
                .set("vector_upsert", vector_upsert_fn)
                .map_err(|error| {
                    format!("Failed to set vulcan.lancedb.vector_upsert: {}", error)
                })?;

            let search_binding = binding.clone();
            let vector_search_fn = lua
                .create_function(move |lua, input: LuaValue| {
                    let input_table = require_table_arg(input, "lancedb.vector_search", "input")?;
                    let mut input_json = lua_value_to_json(&LuaValue::Table(input_table))
                        .map_err(mlua::Error::runtime)?;
                    let input_object = input_json.as_object_mut().ok_or_else(|| {
                        mlua::Error::runtime("lancedb.vector_search input must be an object")
                    })?;
                    input_object
                        .entry("output_format".to_string())
                        .or_insert_with(|| Value::String("json".to_string()));

                    let (meta, raw_bytes) = search_binding
                        .vector_search_json(&input_json)
                        .map_err(mlua::Error::runtime)?;
                    let result_table =
                        json_to_lua_table_inner(lua, &meta).map_err(mlua::Error::external)?;

                    if meta
                        .get("format")
                        .and_then(Value::as_str)
                        .map(|value| value == "json")
                        .unwrap_or(false)
                    {
                        let rows_json: Value =
                            serde_json::from_slice(&raw_bytes).map_err(|error| {
                                mlua::Error::runtime(format!(
                                    "failed to parse LanceDB JSON rows: {}",
                                    error
                                ))
                            })?;
                        result_table
                            .set(
                                "data_json",
                                json_value_to_lua(lua, &rows_json)
                                    .map_err(mlua::Error::external)?,
                            )
                            .map_err(mlua::Error::external)?;
                    } else {
                        result_table
                            .set(
                                "data",
                                LuaValue::String(
                                    lua.create_string(&raw_bytes)
                                        .map_err(mlua::Error::external)?,
                                ),
                            )
                            .map_err(mlua::Error::external)?;
                    }
                    Ok(LuaValue::Table(result_table))
                })
                .map_err(|error| {
                    format!("Failed to create vulcan.lancedb.vector_search: {}", error)
                })?;
            lancedb_table
                .set("vector_search", vector_search_fn)
                .map_err(|error| {
                    format!("Failed to set vulcan.lancedb.vector_search: {}", error)
                })?;

            let delete_binding = binding.clone();
            let delete_fn = lua
                .create_function(move |lua, input: LuaValue| {
                    let input_table = require_table_arg(input, "lancedb.delete", "input")?;
                    let input_json = lua_value_to_json(&LuaValue::Table(input_table))
                        .map_err(mlua::Error::runtime)?;
                    let result = delete_binding
                        .delete_json(&input_json)
                        .map_err(mlua::Error::runtime)?;
                    json_value_to_lua(lua, &result).map_err(mlua::Error::external)
                })
                .map_err(|error| format!("Failed to create vulcan.lancedb.delete: {}", error))?;
            lancedb_table
                .set("delete", delete_fn)
                .map_err(|error| format!("Failed to set vulcan.lancedb.delete: {}", error))?;

            let drop_binding = binding;
            let drop_table_fn = lua
                .create_function(move |lua, input: LuaValue| {
                    let input_table = require_table_arg(input, "lancedb.drop_table", "input")?;
                    let input_json = lua_value_to_json(&LuaValue::Table(input_table))
                        .map_err(mlua::Error::runtime)?;
                    let result = drop_binding
                        .drop_table_json(&input_json)
                        .map_err(mlua::Error::runtime)?;
                    json_value_to_lua(lua, &result).map_err(mlua::Error::external)
                })
                .map_err(|error| {
                    format!("Failed to create vulcan.lancedb.drop_table: {}", error)
                })?;
            lancedb_table
                .set("drop_table", drop_table_fn)
                .map_err(|error| format!("Failed to set vulcan.lancedb.drop_table: {}", error))?;
        } else {
            let disabled_status = disabled_skill_status_json(current_skill_name);
            lancedb_table
                .set("enabled", false)
                .map_err(|error| format!("Failed to set vulcan.lancedb.enabled: {}", error))?;
            let status_value = disabled_status.clone();
            let status_fn = lua
                .create_function(move |lua, ()| {
                    json_value_to_lua(lua, &status_value).map_err(mlua::Error::external)
                })
                .map_err(|error| {
                    format!("Failed to create disabled vulcan.lancedb.status: {}", error)
                })?;
            lancedb_table
                .set("status", status_fn)
                .map_err(|error| format!("Failed to set vulcan.lancedb.status: {}", error))?;
            let info_value = disabled_status.clone();
            let info_fn = lua
                .create_function(move |lua, ()| {
                    json_value_to_lua(lua, &info_value).map_err(mlua::Error::external)
                })
                .map_err(|error| {
                    format!("Failed to create disabled vulcan.lancedb.info: {}", error)
                })?;
            lancedb_table.set("info", info_fn).map_err(|error| {
                format!("Failed to set disabled vulcan.lancedb.info: {}", error)
            })?;
            let disabled_error = "current skill has not enabled lancedb".to_string();
            for method_name in [
                "create_table",
                "vector_upsert",
                "vector_search",
                "delete",
                "drop_table",
            ] {
                let error_text = disabled_error.clone();
                let fn_value = lua
                    .create_function(move |_, _: MultiValue| {
                        Err::<LuaValue, _>(mlua::Error::runtime(error_text.clone()))
                    })
                    .map_err(|error| {
                        format!("Failed to create disabled vulcan.lancedb proxy: {}", error)
                    })?;
                lancedb_table.set(method_name, fn_value).map_err(|error| {
                    format!("Failed to set disabled method {}: {}", method_name, error)
                })?;
            }
        }

        vulcan
            .set("lancedb", lancedb_table)
            .map_err(|error| format!("Failed to set vulcan.lancedb: {}", error))?;
        Ok(())
    }

    /// Populate the skill-scoped SQLite host interface into the `vulcan` module.
    /// 将按 skill 作用域隔离的 SQLite 宿主接口注入到 `vulcan` 模块中。
    fn populate_vulcan_sqlite_context(
        lua: &Lua,
        binding: Option<Arc<SqliteSkillBinding>>,
        current_skill_name: Option<&str>,
    ) -> Result<(), String> {
        let vulcan: Table = lua
            .globals()
            .get("vulcan")
            .map_err(|error| format!("Failed to get vulcan module: {}", error))?;

        let sqlite_table = lua
            .create_table()
            .map_err(|error| format!("Failed to create vulcan.sqlite table: {}", error))?;

        let current_skill = current_skill_name.unwrap_or("");
        vulcan
            .set("__sqlite_skill_name", current_skill)
            .map_err(|error| format!("Failed to set vulcan.__sqlite_skill_name: {}", error))?;

        if let Some(binding) = binding {
            sqlite_table
                .set("enabled", true)
                .map_err(|error| format!("Failed to set vulcan.sqlite.enabled: {}", error))?;

            let info_binding = binding.clone();
            let info_fn = lua
                .create_function(move |lua, ()| {
                    json_value_to_lua(lua, &info_binding.info_json()).map_err(mlua::Error::external)
                })
                .map_err(|error| format!("Failed to create vulcan.sqlite.info: {}", error))?;
            sqlite_table
                .set("info", info_fn)
                .map_err(|error| format!("Failed to set vulcan.sqlite.info: {}", error))?;

            let status_binding = binding.clone();
            let status_fn = lua
                .create_function(move |lua, ()| {
                    json_value_to_lua(lua, &status_binding.status_json())
                        .map_err(mlua::Error::external)
                })
                .map_err(|error| format!("Failed to create vulcan.sqlite.status: {}", error))?;
            sqlite_table
                .set("status", status_fn)
                .map_err(|error| format!("Failed to set vulcan.sqlite.status: {}", error))?;

            let tokenize_binding = binding.clone();
            let tokenize_fn = lua
                .create_function(move |lua, input: LuaValue| {
                    let input_table = require_table_arg(input, "sqlite.tokenize_text", "input")?;
                    let input_json = lua_value_to_json(&LuaValue::Table(input_table))
                        .map_err(mlua::Error::runtime)?;
                    let result = tokenize_binding
                        .tokenize_text_json(&input_json)
                        .map_err(mlua::Error::runtime)?;
                    json_value_to_lua(lua, &result).map_err(mlua::Error::external)
                })
                .map_err(|error| {
                    format!("Failed to create vulcan.sqlite.tokenize_text: {}", error)
                })?;
            sqlite_table
                .set("tokenize_text", tokenize_fn)
                .map_err(|error| format!("Failed to set vulcan.sqlite.tokenize_text: {}", error))?;

            let execute_script_binding = binding.clone();
            let execute_script_fn = lua
                .create_function(move |lua, input: LuaValue| {
                    let input_table = require_table_arg(input, "sqlite.execute_script", "input")?;
                    let input_json = lua_value_to_json(&LuaValue::Table(input_table))
                        .map_err(mlua::Error::runtime)?;
                    let result = execute_script_binding
                        .execute_script(&input_json)
                        .map_err(mlua::Error::runtime)?;
                    json_value_to_lua(lua, &result).map_err(mlua::Error::external)
                })
                .map_err(|error| {
                    format!("Failed to create vulcan.sqlite.execute_script: {}", error)
                })?;
            sqlite_table
                .set("execute_script", execute_script_fn)
                .map_err(|error| {
                    format!("Failed to set vulcan.sqlite.execute_script: {}", error)
                })?;

            let execute_batch_binding = binding.clone();
            let execute_batch_fn = lua
                .create_function(move |lua, input: LuaValue| {
                    let input_table = require_table_arg(input, "sqlite.execute_batch", "input")?;
                    let input_json = lua_value_to_json(&LuaValue::Table(input_table))
                        .map_err(mlua::Error::runtime)?;
                    let result = execute_batch_binding
                        .execute_batch(&input_json)
                        .map_err(mlua::Error::runtime)?;
                    json_value_to_lua(lua, &result).map_err(mlua::Error::external)
                })
                .map_err(|error| {
                    format!("Failed to create vulcan.sqlite.execute_batch: {}", error)
                })?;
            sqlite_table
                .set("execute_batch", execute_batch_fn)
                .map_err(|error| format!("Failed to set vulcan.sqlite.execute_batch: {}", error))?;

            let query_json_binding = binding.clone();
            let query_json_fn = lua
                .create_function(move |lua, input: LuaValue| {
                    let input_table = require_table_arg(input, "sqlite.query_json", "input")?;
                    let input_json = lua_value_to_json(&LuaValue::Table(input_table))
                        .map_err(mlua::Error::runtime)?;
                    let result = query_json_binding
                        .query_json(&input_json)
                        .map_err(mlua::Error::runtime)?;
                    json_value_to_lua(lua, &result).map_err(mlua::Error::external)
                })
                .map_err(|error| format!("Failed to create vulcan.sqlite.query_json: {}", error))?;
            sqlite_table
                .set("query_json", query_json_fn)
                .map_err(|error| format!("Failed to set vulcan.sqlite.query_json: {}", error))?;

            let query_stream_binding = binding.clone();
            let query_stream_fn = lua
                .create_function(move |lua, input: LuaValue| {
                    let input_table = require_table_arg(input, "sqlite.query_stream", "input")?;
                    let input_json = lua_value_to_json(&LuaValue::Table(input_table))
                        .map_err(mlua::Error::runtime)?;
                    let result = query_stream_binding
                        .query_stream(&input_json)
                        .map_err(mlua::Error::runtime)?;
                    json_value_to_lua(lua, &result).map_err(mlua::Error::external)
                })
                .map_err(|error| {
                    format!("Failed to create vulcan.sqlite.query_stream: {}", error)
                })?;
            sqlite_table
                .set("query_stream", query_stream_fn)
                .map_err(|error| format!("Failed to set vulcan.sqlite.query_stream: {}", error))?;

            let query_stream_wait_metrics_binding = binding.clone();
            let query_stream_wait_metrics_fn = lua
                .create_function(move |lua, input: LuaValue| {
                    let input_table =
                        require_table_arg(input, "sqlite.query_stream_wait_metrics", "input")?;
                    let input_json = lua_value_to_json(&LuaValue::Table(input_table))
                        .map_err(mlua::Error::runtime)?;
                    let result = query_stream_wait_metrics_binding
                        .query_stream_wait_metrics(&input_json)
                        .map_err(mlua::Error::runtime)?;
                    json_value_to_lua(lua, &result).map_err(mlua::Error::external)
                })
                .map_err(|error| {
                    format!(
                        "Failed to create vulcan.sqlite.query_stream_wait_metrics: {}",
                        error
                    )
                })?;
            sqlite_table
                .set("query_stream_wait_metrics", query_stream_wait_metrics_fn)
                .map_err(|error| {
                    format!(
                        "Failed to set vulcan.sqlite.query_stream_wait_metrics: {}",
                        error
                    )
                })?;

            let query_stream_chunk_binding = binding.clone();
            let query_stream_chunk_fn = lua
                .create_function(move |lua, input: LuaValue| {
                    let input_table =
                        require_table_arg(input, "sqlite.query_stream_chunk", "input")?;
                    let input_json = lua_value_to_json(&LuaValue::Table(input_table))
                        .map_err(mlua::Error::runtime)?;
                    let result = query_stream_chunk_binding
                        .query_stream_chunk(&input_json)
                        .map_err(mlua::Error::runtime)?;
                    json_value_to_lua(lua, &result).map_err(mlua::Error::external)
                })
                .map_err(|error| {
                    format!(
                        "Failed to create vulcan.sqlite.query_stream_chunk: {}",
                        error
                    )
                })?;
            sqlite_table
                .set("query_stream_chunk", query_stream_chunk_fn)
                .map_err(|error| {
                    format!("Failed to set vulcan.sqlite.query_stream_chunk: {}", error)
                })?;

            let query_stream_close_binding = binding.clone();
            let query_stream_close_fn = lua
                .create_function(move |lua, input: LuaValue| {
                    let input_table =
                        require_table_arg(input, "sqlite.query_stream_close", "input")?;
                    let input_json = lua_value_to_json(&LuaValue::Table(input_table))
                        .map_err(mlua::Error::runtime)?;
                    let result = query_stream_close_binding
                        .query_stream_close(&input_json)
                        .map_err(mlua::Error::runtime)?;
                    json_value_to_lua(lua, &result).map_err(mlua::Error::external)
                })
                .map_err(|error| {
                    format!(
                        "Failed to create vulcan.sqlite.query_stream_close: {}",
                        error
                    )
                })?;
            sqlite_table
                .set("query_stream_close", query_stream_close_fn)
                .map_err(|error| {
                    format!("Failed to set vulcan.sqlite.query_stream_close: {}", error)
                })?;

            let upsert_word_binding = binding.clone();
            let upsert_word_fn = lua
                .create_function(move |lua, input: LuaValue| {
                    let input_table =
                        require_table_arg(input, "sqlite.upsert_custom_word", "input")?;
                    let input_json = lua_value_to_json(&LuaValue::Table(input_table))
                        .map_err(mlua::Error::runtime)?;
                    let result = upsert_word_binding
                        .upsert_custom_word_json(&input_json)
                        .map_err(mlua::Error::runtime)?;
                    json_value_to_lua(lua, &result).map_err(mlua::Error::external)
                })
                .map_err(|error| {
                    format!(
                        "Failed to create vulcan.sqlite.upsert_custom_word: {}",
                        error
                    )
                })?;
            sqlite_table
                .set("upsert_custom_word", upsert_word_fn)
                .map_err(|error| {
                    format!("Failed to set vulcan.sqlite.upsert_custom_word: {}", error)
                })?;

            let remove_word_binding = binding.clone();
            let remove_word_fn = lua
                .create_function(move |lua, input: LuaValue| {
                    let input_table =
                        require_table_arg(input, "sqlite.remove_custom_word", "input")?;
                    let input_json = lua_value_to_json(&LuaValue::Table(input_table))
                        .map_err(mlua::Error::runtime)?;
                    let result = remove_word_binding
                        .remove_custom_word_json(&input_json)
                        .map_err(mlua::Error::runtime)?;
                    json_value_to_lua(lua, &result).map_err(mlua::Error::external)
                })
                .map_err(|error| {
                    format!(
                        "Failed to create vulcan.sqlite.remove_custom_word: {}",
                        error
                    )
                })?;
            sqlite_table
                .set("remove_custom_word", remove_word_fn)
                .map_err(|error| {
                    format!("Failed to set vulcan.sqlite.remove_custom_word: {}", error)
                })?;

            let list_words_binding = binding.clone();
            let list_words_fn = lua
                .create_function(move |lua, ()| {
                    let result = list_words_binding
                        .list_custom_words_json()
                        .map_err(mlua::Error::runtime)?;
                    json_value_to_lua(lua, &result).map_err(mlua::Error::external)
                })
                .map_err(|error| {
                    format!(
                        "Failed to create vulcan.sqlite.list_custom_words: {}",
                        error
                    )
                })?;
            sqlite_table
                .set("list_custom_words", list_words_fn)
                .map_err(|error| {
                    format!("Failed to set vulcan.sqlite.list_custom_words: {}", error)
                })?;

            let ensure_index_binding = binding.clone();
            let ensure_index_fn = lua
                .create_function(move |lua, input: LuaValue| {
                    let input_table = require_table_arg(input, "sqlite.ensure_fts_index", "input")?;
                    let input_json = lua_value_to_json(&LuaValue::Table(input_table))
                        .map_err(mlua::Error::runtime)?;
                    let result = ensure_index_binding
                        .ensure_fts_index_json(&input_json)
                        .map_err(mlua::Error::runtime)?;
                    json_value_to_lua(lua, &result).map_err(mlua::Error::external)
                })
                .map_err(|error| {
                    format!("Failed to create vulcan.sqlite.ensure_fts_index: {}", error)
                })?;
            sqlite_table
                .set("ensure_fts_index", ensure_index_fn)
                .map_err(|error| {
                    format!("Failed to set vulcan.sqlite.ensure_fts_index: {}", error)
                })?;

            let rebuild_index_binding = binding.clone();
            let rebuild_index_fn = lua
                .create_function(move |lua, input: LuaValue| {
                    let input_table =
                        require_table_arg(input, "sqlite.rebuild_fts_index", "input")?;
                    let input_json = lua_value_to_json(&LuaValue::Table(input_table))
                        .map_err(mlua::Error::runtime)?;
                    let result = rebuild_index_binding
                        .rebuild_fts_index_json(&input_json)
                        .map_err(mlua::Error::runtime)?;
                    json_value_to_lua(lua, &result).map_err(mlua::Error::external)
                })
                .map_err(|error| {
                    format!(
                        "Failed to create vulcan.sqlite.rebuild_fts_index: {}",
                        error
                    )
                })?;
            sqlite_table
                .set("rebuild_fts_index", rebuild_index_fn)
                .map_err(|error| {
                    format!("Failed to set vulcan.sqlite.rebuild_fts_index: {}", error)
                })?;

            let upsert_doc_binding = binding.clone();
            let upsert_doc_fn = lua
                .create_function(move |lua, input: LuaValue| {
                    let input_table =
                        require_table_arg(input, "sqlite.upsert_fts_document", "input")?;
                    let input_json = lua_value_to_json(&LuaValue::Table(input_table))
                        .map_err(mlua::Error::runtime)?;
                    let result = upsert_doc_binding
                        .upsert_fts_document_json(&input_json)
                        .map_err(mlua::Error::runtime)?;
                    json_value_to_lua(lua, &result).map_err(mlua::Error::external)
                })
                .map_err(|error| {
                    format!(
                        "Failed to create vulcan.sqlite.upsert_fts_document: {}",
                        error
                    )
                })?;
            sqlite_table
                .set("upsert_fts_document", upsert_doc_fn)
                .map_err(|error| {
                    format!("Failed to set vulcan.sqlite.upsert_fts_document: {}", error)
                })?;

            let delete_doc_binding = binding.clone();
            let delete_doc_fn = lua
                .create_function(move |lua, input: LuaValue| {
                    let input_table =
                        require_table_arg(input, "sqlite.delete_fts_document", "input")?;
                    let input_json = lua_value_to_json(&LuaValue::Table(input_table))
                        .map_err(mlua::Error::runtime)?;
                    let result = delete_doc_binding
                        .delete_fts_document_json(&input_json)
                        .map_err(mlua::Error::runtime)?;
                    json_value_to_lua(lua, &result).map_err(mlua::Error::external)
                })
                .map_err(|error| {
                    format!(
                        "Failed to create vulcan.sqlite.delete_fts_document: {}",
                        error
                    )
                })?;
            sqlite_table
                .set("delete_fts_document", delete_doc_fn)
                .map_err(|error| {
                    format!("Failed to set vulcan.sqlite.delete_fts_document: {}", error)
                })?;

            let search_binding = binding;
            let search_fn = lua
                .create_function(move |lua, input: LuaValue| {
                    let input_table = require_table_arg(input, "sqlite.search_fts", "input")?;
                    let input_json = lua_value_to_json(&LuaValue::Table(input_table))
                        .map_err(mlua::Error::runtime)?;
                    let result = search_binding
                        .search_fts_json(&input_json)
                        .map_err(mlua::Error::runtime)?;
                    json_value_to_lua(lua, &result).map_err(mlua::Error::external)
                })
                .map_err(|error| format!("Failed to create vulcan.sqlite.search_fts: {}", error))?;
            sqlite_table
                .set("search_fts", search_fn)
                .map_err(|error| format!("Failed to set vulcan.sqlite.search_fts: {}", error))?;
        } else {
            let disabled_status = disabled_sqlite_skill_status_json(current_skill_name);
            sqlite_table
                .set("enabled", false)
                .map_err(|error| format!("Failed to set vulcan.sqlite.enabled: {}", error))?;
            let status_value = disabled_status.clone();
            let status_fn = lua
                .create_function(move |lua, ()| {
                    json_value_to_lua(lua, &status_value).map_err(mlua::Error::external)
                })
                .map_err(|error| {
                    format!("Failed to create disabled vulcan.sqlite.status: {}", error)
                })?;
            sqlite_table
                .set("status", status_fn)
                .map_err(|error| format!("Failed to set vulcan.sqlite.status: {}", error))?;
            let info_value = disabled_status.clone();
            let info_fn = lua
                .create_function(move |lua, ()| {
                    json_value_to_lua(lua, &info_value).map_err(mlua::Error::external)
                })
                .map_err(|error| {
                    format!("Failed to create disabled vulcan.sqlite.info: {}", error)
                })?;
            sqlite_table
                .set("info", info_fn)
                .map_err(|error| format!("Failed to set disabled vulcan.sqlite.info: {}", error))?;
            let disabled_error = "current skill has not enabled sqlite".to_string();
            for method_name in [
                "execute_script",
                "execute_batch",
                "query_json",
                "query_stream",
                "query_stream_wait_metrics",
                "query_stream_chunk",
                "query_stream_close",
                "tokenize_text",
                "upsert_custom_word",
                "remove_custom_word",
                "list_custom_words",
                "ensure_fts_index",
                "rebuild_fts_index",
                "upsert_fts_document",
                "delete_fts_document",
                "search_fts",
            ] {
                let error_text = disabled_error.clone();
                let fn_value = lua
                    .create_function(move |_, _: MultiValue| {
                        Err::<LuaValue, _>(mlua::Error::runtime(error_text.clone()))
                    })
                    .map_err(|error| {
                        format!("Failed to create disabled vulcan.sqlite proxy: {}", error)
                    })?;
                sqlite_table.set(method_name, fn_value).map_err(|error| {
                    format!("Failed to set disabled method {}: {}", method_name, error)
                })?;
            }
        }

        vulcan
            .set("sqlite", sqlite_table)
            .map_err(|error| format!("Failed to set vulcan.sqlite: {}", error))?;
        Ok(())
    }

    /// Call one active loaded Lua skill with the given JSON arguments.
    /// 使用给定 JSON 参数调用单个已激活的已加载 Lua skill。
    /// Calls are runtime execution, not skill-management authority checks.
    /// 调用属于运行时执行，不属于技能管理权限校验。
    pub fn call_skill(
        &self,
        tool_name: &str,
        args: &Value,
        invocation_context: Option<&LuaInvocationContext>,
    ) -> Result<RuntimeInvocationResult, String> {
        let resolved_target = self
            .entry_registry
            .get(tool_name)
            .ok_or_else(|| format!("Lua skill '{}' not found", tool_name))?;
        let skill = self
            .skills
            .get(&resolved_target.skill_storage_key)
            .ok_or_else(|| format!("Lua skill '{}' not found", tool_name))?;
        let tool = skill
            .meta
            .find_tool_by_local_name(&resolved_target.local_name)
            .ok_or_else(|| format!("Lua skill '{}' not found", tool_name))?;
        let display_tool_name = resolved_target.canonical_name.clone();

        let module_name = tool.lua_module.clone();
        let func_name = format!("__skill_{}", module_name);

        let mut lease = self.acquire_vm()?;
        let scope_guard = LuaVmRequestScopeGuard::new(&mut lease, self.host_options.as_ref())?;
        let lua = scope_guard.lua();

        if skill.meta.debug {
            Self::compile_skill_into_lua(lua, skill, tool, true)?;
        }

        Self::populate_vulcan_request_context(lua, invocation_context)?;
        populate_vulcan_internal_execution_context(
            lua,
            &VulcanInternalExecutionContext {
                tool_name: Some(display_tool_name.clone()),
                skill_name: Some(skill.meta.effective_skill_id().to_string()),
                entry_name: Some(resolved_target.local_name.clone()),
                root_name: Some(skill.root_name.clone()),
                luaexec_active: false,
                luaexec_caller_tool_name: None,
            },
        )?;
        let entry_path = tool_entry_path(&skill.dir, tool);
        populate_vulcan_file_context(lua, Some(&skill.dir), Some(&entry_path))?;
        populate_vulcan_dependency_context(
            lua,
            self.host_options.as_ref(),
            Some(&skill.dir),
            Some(skill.meta.effective_skill_id()),
        )?;
        Self::populate_vulcan_lancedb_context(
            lua,
            skill.lancedb_binding.clone(),
            Some(skill.meta.effective_skill_id()),
        )?;
        Self::populate_vulcan_sqlite_context(
            lua,
            skill.sqlite_binding.clone(),
            Some(skill.meta.effective_skill_id()),
        )?;

        let handler: Function = lua
            .globals()
            .get(func_name.as_str())
            .map_err(|e| format!("Skill function '{}' not found: {}", module_name, e))?;

        // Convert JSON args to Lua table
        let args_table = json_to_lua_table(lua, args)?;

        let call_result = (|| {
            // Call the function
            let result: MultiValue = handler.call(args_table).map_err(|e| {
                let msg = format!("Lua skill '{}' error: {}", display_tool_name, e);
                log_error(format!("[LuaSkill:error] {}", msg));
                msg
            })?;

            parse_tool_call_output(result, &display_tool_name, invocation_context).map_err(|e| {
                log_error(format!("[LuaSkill:error] {}", e));
                e
            })
        })();
        let cleanup_result = scope_guard.finish();
        match (call_result, cleanup_result) {
            (Ok(result), Ok(())) => Ok(result),
            (Ok(_), Err(cleanup_error)) => Err(cleanup_error),
            (Err(call_error), Ok(())) => Err(call_error),
            (Err(call_error), Err(cleanup_error)) => Err(format!(
                "{}; pooled Lua VM cleanup failed: {}",
                call_error, cleanup_error
            )),
        }
    }

    /// Render one help payload from either Markdown or Lua.
    /// 从 Markdown 或 Lua 渲染单个帮助载荷。
    fn render_help_payload(
        &self,
        skill: &LoadedSkill,
        relative_path: &str,
        request_context: Option<&RuntimeRequestContext>,
    ) -> Result<String, String> {
        if !is_lua_help_file(relative_path) {
            return read_skill_text_file(&skill.dir, relative_path, "help");
        }

        let helper_path = skill.dir.join(relative_path);
        let helper_source = std::fs::read_to_string(&helper_path).map_err(|error| {
            format!(
                "Failed to read help file {}: {}",
                helper_path.display(),
                error
            )
        })?;
        let mut lease = self.acquire_vm()?;
        let scope_guard = LuaVmRequestScopeGuard::new(&mut lease, self.host_options.as_ref())?;
        let lua = scope_guard.lua();
        let help_invocation_context = LuaInvocationContext::new(
            request_context.cloned(),
            Value::Object(serde_json::Map::new()),
            Value::Object(serde_json::Map::new()),
        );
        Self::populate_vulcan_request_context(lua, Some(&help_invocation_context))?;
        populate_vulcan_internal_execution_context(
            lua,
            &VulcanInternalExecutionContext {
                tool_name: Some("vulcan-help".to_string()),
                skill_name: Some(skill.meta.effective_skill_id().to_string()),
                entry_name: Some(relative_path.to_string()),
                root_name: Some(skill.root_name.clone()),
                luaexec_active: false,
                luaexec_caller_tool_name: None,
            },
        )?;
        populate_vulcan_file_context(lua, Some(&skill.dir), Some(&helper_path))?;
        populate_vulcan_dependency_context(
            lua,
            self.host_options.as_ref(),
            Some(&skill.dir),
            Some(skill.meta.effective_skill_id()),
        )?;
        Self::populate_vulcan_lancedb_context(
            lua,
            skill.lancedb_binding.clone(),
            Some(skill.meta.effective_skill_id()),
        )?;
        Self::populate_vulcan_sqlite_context(
            lua,
            skill.sqlite_binding.clone(),
            Some(skill.meta.effective_skill_id()),
        )?;

        let chunk_name = format!("{}-{}", skill.meta.effective_skill_id(), relative_path);
        let chunk = lua.load(&helper_source).set_name(&chunk_name);
        let rendered_result = (|| {
            let exported: LuaValue = chunk
                .into_function()
                .map_err(|error| {
                    format!(
                        "Help compile error for {}: {}",
                        helper_path.display(),
                        error
                    )
                })?
                .call(())
                .map_err(|error| {
                    format!("Help init error for {}: {}", helper_path.display(), error)
                })?;

            let rendered_value = match exported {
                LuaValue::Function(function) => function.call(()).map_err(|error| {
                    format!(
                        "Help runtime error for {}: {}",
                        helper_path.display(),
                        error
                    )
                })?,
                other => other,
            };

            match rendered_value {
                LuaValue::String(text) => {
                    text.to_str()
                        .map(|value| value.to_string())
                        .map_err(|error| {
                            format!(
                                "Help {} returned invalid UTF-8 text: {}",
                                helper_path.display(),
                                error
                            )
                        })
                }
                other => Err(format!(
                    "Help {} must return a plain string, actual_type='{}'",
                    helper_path.display(),
                    lua_value_type_name(&other)
                )),
            }
        })();
        let cleanup_result = scope_guard.finish();
        match (rendered_result, cleanup_result) {
            (Ok(rendered), Ok(())) => Ok(rendered),
            (Ok(_), Err(cleanup_error)) => Err(cleanup_error),
            (Err(render_error), Ok(())) => Err(render_error),
            (Err(render_error), Err(cleanup_error)) => Err(format!(
                "{}; pooled Lua VM cleanup failed: {}",
                render_error, cleanup_error
            )),
        }
    }

    /// Populate the vulcan.call function to dispatch to loaded skills.
    fn populate_vulcan_call_for_lua(
        lua: &Lua,
        skills_map: &HashMap<String, LoadedSkill>,
        entry_registry: &BTreeMap<String, ResolvedEntryTarget>,
        host_options: Arc<LuaRuntimeHostOptions>,
        lancedb_host: Option<Arc<LanceDbSkillHost>>,
        sqlite_host: Option<Arc<SqliteSkillHost>>,
    ) -> Result<(), String> {
        let vulcan: Table = lua
            .globals()
            .get("vulcan")
            .map_err(|e| format!("vulcan module not found: {}", e))?;

        /// Resolved dispatcher metadata for one strict LuaSkills entry.
        /// 单个严格 LuaSkills 入口的已解析分发元数据。
        #[derive(Clone)]
        struct DispatchEntry {
            /// Canonical display name used as the active tool name.
            /// 作为当前活动工具名使用的 canonical 显示名称。
            display_name: String,
            /// Lua module name registered in the VM globals.
            /// 注册到虚拟机全局表中的 Lua 模块名。
            module_name: String,
            /// Owning skill id of the current entry.
            /// 当前入口所属的 skill id。
            owner_skill_id: String,
            /// Stable local entry name declared by the owning skill.
            /// 所属 skill 声明的稳定局部入口名称。
            local_name: String,
            /// Runtime root name that owns the current entry.
            /// 拥有当前入口的运行时根名称。
            root_name: String,
            /// Owning skill directory used to restore file context.
            /// 用于恢复文件上下文的所属 skill 目录。
            owner_skill_dir: String,
            /// Absolute entry file path used to restore file context.
            /// 用于恢复文件上下文的绝对入口文件路径。
            entry_path: String,
        }

        // Create the call dispatcher
        let dispatch_entries: Vec<DispatchEntry> = entry_registry
            .values()
            .filter_map(|target| {
                let skill = skills_map.get(&target.skill_storage_key)?;
                let tool = skill.meta.find_tool_by_local_name(&target.local_name)?;
                let entry_path = tool_entry_path(&skill.dir, tool);
                Some(DispatchEntry {
                    display_name: target.canonical_name.clone(),
                    module_name: tool.lua_module.clone(),
                    owner_skill_id: target.skill_id.clone(),
                    local_name: target.local_name.clone(),
                    root_name: skill.root_name.clone(),
                    owner_skill_dir: skill.dir.to_string_lossy().to_string(),
                    entry_path: entry_path.to_string_lossy().to_string(),
                })
            })
            .collect();

        let dispatcher = lua
            .create_function(move |lua, (name, args): (LuaValue, LuaValue)| {
                let name = require_string_arg(name, "call", "name", false)?;
                let args = require_table_arg(args, "call", "args")?;
                let dispatch_entry = dispatch_entries
                    .iter()
                    .find(|entry| entry.display_name == name)
                    .ok_or_else(|| mlua::Error::runtime(format!("Skill '{}' not found", name)))?;
                let module = &dispatch_entry.module_name;
                let owner_skill_name = &dispatch_entry.owner_skill_id;
                let func_name = format!("__skill_{}", module);
                let func: Function = lua.globals().get(func_name.as_str()).map_err(|_| {
                    mlua::Error::runtime(format!("Skill function '{}' not found", module))
                })?;
                let nested_scope_guard = LuaNestedCallScopeGuard::new(
                    lua,
                    host_options.clone(),
                    lancedb_host.clone(),
                    sqlite_host.clone(),
                )
                .map_err(mlua::Error::runtime)?;
                let current_request_context_json =
                    lua_value_to_json(&nested_scope_guard.previous_context)
                        .map_err(mlua::Error::runtime)?;
                let current_request_context = match &current_request_context_json {
                    Value::Object(object) if object.is_empty() => None,
                    _ => serde_json::from_value::<RuntimeRequestContext>(
                        current_request_context_json,
                    )
                    .ok(),
                };
                let current_client_budget =
                    lua_value_to_json(&nested_scope_guard.previous_client_budget)
                        .map_err(mlua::Error::runtime)?;
                let current_tool_config =
                    lua_value_to_json(&nested_scope_guard.previous_tool_config)
                        .map_err(mlua::Error::runtime)?;
                if nested_scope_guard.previous_internal_context.luaexec_active {
                    if nested_scope_guard
                        .previous_internal_context
                        .luaexec_caller_tool_name
                        .as_deref()
                        == Some(dispatch_entry.display_name.as_str())
                    {
                        return Err(mlua::Error::runtime(format!(
                            "vulcan.call cannot call the current luaexec caller tool '{}'",
                            dispatch_entry.display_name
                        )));
                    }
                    if dispatch_entry.owner_skill_id == "vulcan-runtime"
                        && (dispatch_entry.local_name == "lua-exec"
                            || dispatch_entry.local_name == "lua-file")
                    {
                        return Err(mlua::Error::runtime(format!(
                            "vulcan.call cannot invoke '{}' inside luaexec",
                            dispatch_entry.display_name
                        )));
                    }
                }
                let target_binding = match lancedb_host.as_ref() {
                    Some(host) => host
                        .binding_for_skill(owner_skill_name)
                        .map_err(mlua::Error::runtime)?,
                    None => None,
                };
                let target_sqlite_binding = match sqlite_host.as_ref() {
                    Some(host) => host
                        .binding_for_skill(owner_skill_name)
                        .map_err(mlua::Error::runtime)?,
                    None => None,
                };
                let nested_invocation_context = LuaInvocationContext::new(
                    current_request_context,
                    current_client_budget,
                    current_tool_config,
                );
                nested_scope_guard
                    .enter_nested_call(
                        &dispatch_entry.display_name,
                        owner_skill_name,
                        &dispatch_entry.local_name,
                        &dispatch_entry.root_name,
                        &dispatch_entry.owner_skill_dir,
                        &dispatch_entry.entry_path,
                        &nested_invocation_context,
                        target_binding,
                        target_sqlite_binding,
                    )
                    .map_err(mlua::Error::runtime)?;
                let call_result = func.call::<MultiValue>(args);
                let restore_result = nested_scope_guard.finish().map_err(mlua::Error::runtime);
                match (call_result, restore_result) {
                    (Ok(result), Ok(())) => Ok(result),
                    (Ok(_), Err(restore_error)) => Err(restore_error),
                    (Err(call_error), Ok(())) => Err(call_error),
                    (Err(call_error), Err(restore_error)) => Err(mlua::Error::runtime(format!(
                        "{}; nested vulcan.call restore failed: {}",
                        call_error, restore_error
                    ))),
                }
            })
            .map_err(|e| format!("Failed to create vulcan.call dispatcher: {}", e))?;

        vulcan
            .set("call", dispatcher)
            .map_err(|e| format!("Failed to set vulcan.call: {}", e))?;

        Ok(())
    }

    /// Configure package.path and package.cpath to include project-local luarocks tree.
    /// 配置 package.path 与 package.cpath，使其只依赖项目内统一的 lua 目录布局。
    ///
    /// This keeps runtime resolution aligned with the deployed layout under
    /// `lua_packages/share/lua/` and `lua_packages/lib/lua/`, instead of relying on
    /// versioned `5.1` subdirectories that may not exist in the shipped bundle.
    /// This keeps runtime resolution aligned with the deployed layout under
    /// 这会让运行时与已部署的目录布局保持一致，
    /// `lua_packages/share/lua/` and `lua_packages/lib/lua/`, instead of relying on
    /// 即仅依赖 `lua_packages/share/lua/` 与 `lua_packages/lib/lua/`，
    /// versioned `5.1` subdirectories that may not exist in the shipped bundle.
    /// 而不再依赖发布包中可能并不存在的带版本 `5.1` 子目录。
    fn setup_package_paths(
        lua: &Lua,
        host_options: &LuaRuntimeHostOptions,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let Some(lua_packages) = host_options.lua_packages_dir.as_ref() else {
            return Ok(());
        };
        if !lua_packages.exists() {
            return Ok(());
        }

        let host_provided_ffi_root = host_options
            .host_provided_ffi_root
            .as_ref()
            .filter(|root| root.exists());

        // Build package.cpath entries for C modules (.dll on Windows)
        // 统一使用宿主提供的 lua_packages/lib/lua 目录，并补充宿主提供的 FFI/原生库根目录。
        #[cfg(windows)]
        let cpath_pattern = {
            let mut pattern = format!(
                "{}\\lib\\lua\\?.dll;{}\\lib\\lua\\?\\init.dll;{}\\lib\\lua\\loadall.dll;{}\\?\\?.dll;",
                lua_packages.display(),
                lua_packages.display(),
                lua_packages.display(),
                lua_packages.display()
            );
            if let Some(root) = host_provided_ffi_root {
                pattern.push_str(&format!(
                    "{}\\?.dll;{}\\?\\init.dll;",
                    root.display(),
                    root.display()
                ));
            }
            pattern
        };

        // Build package.cpath entries for C modules (.so on Linux)
        // Linux 下同样严格依赖宿主传入的 lua_packages 根目录，并补充宿主提供的 FFI/原生库根目录。
        #[cfg(target_os = "linux")]
        let cpath_pattern = {
            let mut pattern = format!(
                "{}/lib/lua/?.so;{}/lib/lua/?/init.so;{}/lib/lua/loadall.so;{}/?.so;",
                lua_packages.display(),
                lua_packages.display(),
                lua_packages.display(),
                lua_packages.display()
            );
            if let Some(root) = host_provided_ffi_root {
                pattern.push_str(&format!(
                    "{}/?.so;{}/?/init.so;",
                    root.display(),
                    root.display()
                ));
            }
            pattern
        };

        // Build package.cpath entries for C modules (.dylib on macOS)
        // macOS 下同样严格依赖宿主传入的 lua_packages 根目录，并补充宿主提供的 FFI/原生库根目录。
        #[cfg(target_os = "macos")]
        let cpath_pattern = {
            let mut pattern = format!(
                "{}/lib/lua/?.dylib;{}/lib/lua/?/init.dylib;{}/lib/lua/loadall.dylib;{}/?.dylib;",
                lua_packages.display(),
                lua_packages.display(),
                lua_packages.display(),
                lua_packages.display()
            );
            if let Some(root) = host_provided_ffi_root {
                pattern.push_str(&format!(
                    "{}/?.dylib;{}/?/init.dylib;",
                    root.display(),
                    root.display()
                ));
            }
            pattern
        };

        // Build package.path entries for Lua modules
        // 统一使用宿主提供的 lua_packages/share/lua 目录。
        #[cfg(windows)]
        let path_pattern = format!(
            "{}\\share\\lua\\?.lua;{}\\share\\lua\\?\\init.lua;{}\\?.lua;",
            lua_packages.display(),
            lua_packages.display(),
            lua_packages.display()
        );

        // Build package.path entries for Lua modules on Unix-like systems
        // 类 Unix 平台同样严格依赖宿主传入的 lua_packages 根目录。
        #[cfg(unix)]
        let path_pattern = format!(
            "{}/share/lua/?.lua;{}/share/lua/?/init.lua;{}/?.lua;",
            lua_packages.display(),
            lua_packages.display(),
            lua_packages.display()
        );

        // Prepend to existing paths
        // 将宿主指定路径前置到现有 package 搜索链，避免覆盖 Lua 默认行为。
        let package: Table = lua.globals().get("package")?;
        let old_cpath: mlua::String = package.get("cpath")?;
        let new_cpath = format!("{}{}", cpath_pattern, old_cpath.to_str()?.to_string());
        package.set("cpath", lua.create_string(&new_cpath)?)?;

        let old_path: mlua::String = package.get("path")?;
        let new_path = format!("{}{}", path_pattern, old_path.to_str()?.to_string());
        package.set("path", lua.create_string(&new_path)?)?;
        Ok(())
    }

    /// Register the strict `vulcan` module in the Lua VM.
    /// 在 Lua 虚拟机中注册严格版 `vulcan` 模块。
    fn register_vulcan_module(
        lua: &Lua,
        host_options: &LuaRuntimeHostOptions,
        skill_config_store: Arc<SkillConfigStore>,
        runtime_skill_roots: &[RuntimeSkillRoot],
    ) -> Result<(), Box<dyn std::error::Error>> {
        let vulcan = lua.create_table()?;
        let runtime = lua.create_table()?;
        let runtime_skills = lua.create_table()?;
        let runtime_internal = lua.create_table()?;
        let runtime_lua = lua.create_table()?;
        let fs = lua.create_table()?;
        let path = lua.create_table()?;
        let process = lua.create_table()?;
        let os = lua.create_table()?;
        let json = lua.create_table()?;
        let cache = lua.create_table()?;
        let config = lua.create_table()?;
        let host = lua.create_table()?;
        let models = lua.create_table()?;
        let context = lua.create_table()?;
        let deps = lua.create_table()?;
        let default_text_encoding = resolve_host_default_text_encoding(host_options)?;
        let vulcan_io = create_vulcan_io_table(lua, default_text_encoding)?;

        let runtime_log_fn = lua.create_function(|_, (level, msg): (LuaValue, LuaValue)| {
            let level = require_string_arg(level, "runtime.log", "level", false)?;
            let msg = require_string_arg(msg, "runtime.log", "message", true)?;
            let normalized_level = level.trim().to_ascii_lowercase();
            let rendered = format!("[LuaSkill:{}] {}", level, msg);
            if normalized_level.contains("error") || normalized_level.contains("fatal") {
                log_error(rendered);
            } else if normalized_level.contains("warn") {
                log_warn(rendered);
            } else {
                log_info(rendered);
            }
            Ok(())
        })?;
        runtime.set("log", runtime_log_fn)?;

        let print_fn = lua.create_function(|_, args: MultiValue| {
            let mut parts = Vec::new();
            for val in args.into_iter() {
                let s = match val {
                    LuaValue::String(s) => s.to_str().map(|b| b.to_string()).unwrap_or_default(),
                    LuaValue::Integer(i) => i.to_string(),
                    LuaValue::Number(f) => f.to_string(),
                    LuaValue::Boolean(b) => b.to_string(),
                    LuaValue::Nil => "nil".to_string(),
                    _ => format!("{:?}", val),
                };
                parts.push(s);
            }
            log_info(format!("[LuaSkill:info] {}", parts.join("\t")));
            Ok(())
        })?;
        lua.globals().set("print", print_fn)?;

        let fs_list_fn = lua.create_function(|_, dir: LuaValue| {
            let dir = require_path_arg(dir, "fs.list", "dir")?;
            let mut entries = Vec::new();
            for entry in std::fs::read_dir(&dir)
                .map_err(|e| mlua::Error::runtime(format!("fs.list: {}", e)))?
            {
                let entry = entry.map_err(|e| mlua::Error::runtime(format!("fs.list: {}", e)))?;
                let file_name = entry.file_name().into_string().map_err(|name| {
                    mlua::Error::runtime(format!(
                        "fs.list: non-UTF-8 file name under {}: {:?}",
                        Path::new(&dir).display(),
                        name
                    ))
                })?;
                entries.push(file_name);
            }
            Ok(entries)
        })?;
        fs.set("list", fs_list_fn)?;

        let fs_read_fn = lua.create_function(|_, path: LuaValue| {
            let path = require_path_arg(path, "fs.read", "path")?;
            std::fs::read_to_string(&path)
                .map_err(|e| mlua::Error::runtime(format!("fs.read: {}", e)))
        })?;
        fs.set("read", fs_read_fn)?;

        let fs_write_fn = lua.create_function(|_, (path, content): (LuaValue, LuaValue)| {
            let path = require_path_arg(path, "fs.write", "path")?;
            let content = require_string_arg(content, "fs.write", "content", true)?;
            std::fs::write(&path, content)
                .map_err(|e| mlua::Error::runtime(format!("fs.write: {}", e)))
        })?;
        fs.set("write", fs_write_fn)?;

        let fs_write_bytes_fn =
            lua.create_function(|_, (path, content): (LuaValue, LuaValue)| {
                let path = require_path_arg(path, "fs.write_bytes", "path")?;
                let content = require_string_arg(content, "fs.write_bytes", "content", false)?;
                let bytes = BASE64_STANDARD
                    .decode(content.as_bytes())
                    .map_err(|error| {
                        mlua::Error::runtime(format!(
                            "fs.write_bytes: base64 decode failed: {error}"
                        ))
                    })?;
                fs::write(&path, bytes)
                    .map_err(|error| mlua::Error::runtime(format!("fs.write_bytes: {}", error)))?;
                Ok(true)
            })?;
        fs.set("write_bytes", fs_write_bytes_fn)?;

        let fs_rename_fn =
            lua.create_function(|_, (old_path, new_path): (LuaValue, LuaValue)| {
                let old_path = require_path_arg(old_path, "fs.rename", "old_path")?;
                let new_path = require_path_arg(new_path, "fs.rename", "new_path")?;
                fs::rename(&old_path, &new_path)
                    .map_err(|error| mlua::Error::runtime(format!("fs.rename: {}", error)))?;
                Ok(true)
            })?;
        fs.set("rename", fs_rename_fn)?;

        let fs_remove_fn = lua.create_function(|_, args: MultiValue| {
            let mut values = args.into_iter();
            let path =
                require_path_arg(values.next().unwrap_or(LuaValue::Nil), "fs.remove", "path")?;
            let recursive = parse_vulcan_fs_recursive_option(
                values.next().unwrap_or(LuaValue::Nil),
                "fs.remove",
            )?;
            let target_path = Path::new(&path);
            let metadata = match fs::symlink_metadata(target_path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
                Err(error) => {
                    return Err(mlua::Error::runtime(format!("fs.remove: {}", error)));
                }
            };
            let file_type = metadata.file_type();
            if file_type.is_dir() {
                if recursive {
                    fs::remove_dir_all(target_path)
                        .map_err(|error| mlua::Error::runtime(format!("fs.remove: {}", error)))?;
                } else {
                    fs::remove_dir(target_path)
                        .map_err(|error| mlua::Error::runtime(format!("fs.remove: {}", error)))?;
                }
            } else {
                fs::remove_file(target_path)
                    .map_err(|error| mlua::Error::runtime(format!("fs.remove: {}", error)))?;
            }
            Ok(true)
        })?;
        fs.set("remove", fs_remove_fn)?;

        let fs_mkdir_fn = lua.create_function(|_, args: MultiValue| {
            let mut values = args.into_iter();
            let path =
                require_path_arg(values.next().unwrap_or(LuaValue::Nil), "fs.mkdir", "path")?;
            let recursive = parse_vulcan_fs_recursive_option(
                values.next().unwrap_or(LuaValue::Nil),
                "fs.mkdir",
            )?;
            let target_path = Path::new(&path);
            if target_path.exists() {
                if target_path.is_dir() {
                    return Ok(false);
                }
                return Err(mlua::Error::runtime(format!(
                    "fs.mkdir: target already exists and is not a directory: {}",
                    render_log_friendly_path(target_path)
                )));
            }
            if recursive {
                fs::create_dir_all(target_path)
                    .map_err(|error| mlua::Error::runtime(format!("fs.mkdir: {}", error)))?;
            } else {
                fs::create_dir(target_path)
                    .map_err(|error| mlua::Error::runtime(format!("fs.mkdir: {}", error)))?;
            }
            Ok(true)
        })?;
        fs.set("mkdir", fs_mkdir_fn)?;

        let fs_copy_fn = lua.create_function(|_, args: MultiValue| {
            let mut values = args.into_iter();
            let source_path = require_path_arg(
                values.next().unwrap_or(LuaValue::Nil),
                "fs.copy",
                "src_path",
            )?;
            let target_path = require_path_arg(
                values.next().unwrap_or(LuaValue::Nil),
                "fs.copy",
                "dst_path",
            )?;
            let overwrite = parse_vulcan_fs_overwrite_option(
                values.next().unwrap_or(LuaValue::Nil),
                "fs.copy",
            )?;
            let source = Path::new(&source_path);
            let target = Path::new(&target_path);
            let source_metadata = fs::metadata(source)
                .map_err(|error| mlua::Error::runtime(format!("fs.copy: {}", error)))?;
            let target_exists =
                path_entry_exists(target, "fs.copy").map_err(mlua::Error::runtime)?;
            if target_exists && !overwrite {
                return Ok(false);
            }
            if source_metadata.is_dir() {
                validate_vulcan_fs_copy_directory_target(
                    source,
                    target,
                    overwrite && target_exists,
                )
                .map_err(mlua::Error::runtime)?;
            }
            if target_exists {
                remove_vulcan_fs_copy_target(target).map_err(mlua::Error::runtime)?;
            }
            if source_metadata.is_dir() {
                copy_vulcan_fs_directory_recursive(source, target).map_err(mlua::Error::runtime)?;
            } else {
                fs::copy(source, target)
                    .map_err(|error| mlua::Error::runtime(format!("fs.copy: {}", error)))?;
            }
            Ok(true)
        })?;
        fs.set("copy", fs_copy_fn)?;

        let fs_stat_fn = lua.create_function(|lua, path: LuaValue| {
            let path = require_path_arg(path, "fs.stat", "path")?;
            match fs::symlink_metadata(&path) {
                Ok(metadata) => Ok(LuaValue::Table(create_vulcan_fs_stat_table(
                    lua, &metadata,
                )?)),
                Err(error) if error.kind() == ErrorKind::NotFound => Ok(LuaValue::Nil),
                Err(error) => Err(mlua::Error::runtime(format!("fs.stat: {}", error))),
            }
        })?;
        fs.set("stat", fs_stat_fn)?;

        let fs_read_bytes_fn = lua.create_function(|lua, path: LuaValue| {
            let path = require_path_arg(path, "fs.read_bytes", "path")?;
            let bytes = fs::read(&path)
                .map_err(|error| mlua::Error::runtime(format!("fs.read_bytes: {}", error)))?;
            lua.create_string(BASE64_STANDARD.encode(bytes))
        })?;
        fs.set("read_bytes", fs_read_bytes_fn)?;

        let fs_exists_fn = lua.create_function(|_, path: LuaValue| {
            let path = require_path_arg(path, "fs.exists", "path")?;
            Ok(Path::new(&path).exists())
        })?;
        fs.set("exists", fs_exists_fn)?;

        let fs_is_dir_fn = lua.create_function(|_, path: LuaValue| {
            let path = require_path_arg(path, "fs.is_dir", "path")?;
            Ok(Path::new(&path).is_dir())
        })?;
        fs.set("is_dir", fs_is_dir_fn)?;

        let path_join_fn = lua.create_function(|lua, parts: MultiValue| {
            if parts.is_empty() {
                return Err(mlua::Error::runtime(
                    "path.join: expected at least one path segment",
                ));
            }
            let mut joined = PathBuf::new();
            for (index, val) in parts.into_iter().enumerate() {
                let param_name = format!("part[{}]", index + 1);
                let part = require_path_arg(val, "path.join", &param_name)?;
                joined.push(part);
            }
            let result = render_host_visible_path(&joined);
            lua.create_string(&result)
        })?;
        path.set("join", path_join_fn)?;

        let path_dirname_fn = lua.create_function(|lua, path: LuaValue| {
            let path = require_path_arg(path, "path.dirname", "path")?;
            let rendered = render_vulcan_path_dirname(Path::new(&path));
            lua.create_string(&rendered)
        })?;
        path.set("dirname", path_dirname_fn)?;

        let path_basename_fn = lua.create_function(|lua, path: LuaValue| {
            let path = require_path_arg(path, "path.basename", "path")?;
            let rendered = Path::new(&path)
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_default();
            lua.create_string(&rendered)
        })?;
        path.set("basename", path_basename_fn)?;

        let path_stem_fn = lua.create_function(|lua, path: LuaValue| {
            let path = require_path_arg(path, "path.stem", "path")?;
            let rendered = Path::new(&path)
                .file_stem()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_default();
            lua.create_string(&rendered)
        })?;
        path.set("stem", path_stem_fn)?;

        let path_extname_fn = lua.create_function(|lua, path: LuaValue| {
            let path = require_path_arg(path, "path.extname", "path")?;
            let rendered = Path::new(&path)
                .extension()
                .map(|ext| format!(".{}", ext.to_string_lossy()))
                .unwrap_or_default();
            lua.create_string(&rendered)
        })?;
        path.set("extname", path_extname_fn)?;

        let path_normalize_fn = lua.create_function(|lua, path: LuaValue| {
            let path = require_path_arg(path, "path.normalize", "path")?;
            let rendered = render_vulcan_normalized_path(Path::new(&path));
            lua.create_string(&rendered)
        })?;
        path.set("normalize", path_normalize_fn)?;

        let path_is_abs_fn = lua.create_function(|_, path: LuaValue| {
            let path = require_path_arg(path, "path.is_abs", "path")?;
            Ok(Path::new(&path).is_absolute())
        })?;
        path.set("is_abs", path_is_abs_fn)?;

        let cwd_fn = lua.create_function(|lua, ()| {
            let current_dir = std::env::current_dir()
                .map_err(|error| mlua::Error::runtime(format!("runtime.cwd: {}", error)))?;
            let current_dir_text = render_host_visible_path(&current_dir);
            lua.create_string(&current_dir_text)
        })?;
        runtime.set("cwd", cwd_fn)?;

        match host_options.temp_dir.as_ref() {
            Some(path_buf) => runtime.set("temp_dir", render_host_visible_path(path_buf))?,
            None => runtime.set("temp_dir", LuaValue::Nil)?,
        }

        match host_options.resources_dir.as_ref() {
            Some(path_buf) => runtime.set("resources_dir", render_host_visible_path(path_buf))?,
            None => runtime.set("resources_dir", LuaValue::Nil)?,
        }

        let exec_default_encoding = default_text_encoding;
        let launchers_fn = lua.create_function(|lua, ()| {
            let info = lua.create_table()?;
            let shells = lua.create_table()?;
            for (index, shell_name) in supported_exec_shell_names().into_iter().enumerate() {
                shells.set(index + 1, shell_name)?;
            }
            info.set("default", default_exec_shell_name())?;
            info.set("shells", shells)?;
            Ok(info)
        })?;
        process.set("launchers", launchers_fn)?;
        let exec_fn = lua.create_function(move |lua, spec: LuaValue| {
            let request = parse_exec_request(spec, "process.exec", exec_default_encoding)?;
            let result = execute_exec_request(request);
            exec_result_to_lua_table(lua, result)
        })?;
        process.set("exec", exec_fn)?;
        let which_fn = lua.create_function(|lua, program: LuaValue| {
            let program = require_string_arg(program, "process.which", "program", false)?;
            match resolve_vulcan_process_which(&program) {
                Ok(Some(found)) => {
                    let rendered = render_host_visible_path(&found);
                    Ok(LuaValue::String(lua.create_string(&rendered)?))
                }
                Ok(None) => Ok(LuaValue::Nil),
                Err(error) => Err(mlua::Error::runtime(error)),
            }
        })?;
        process.set("which", which_fn)?;
        process.set(
            "session",
            create_process_session_table(lua, default_text_encoding)?,
        )?;

        let os_info_fn = lua.create_function(|lua, ()| {
            let current_os = match std::env::consts::OS {
                "windows" => "windows",
                "linux" => "linux",
                "macos" => "macos",
                _ => std::env::consts::OS,
            };
            let arch = match std::env::consts::ARCH {
                "x86_64" => "x86_64",
                "x86" => "i686",
                "aarch64" => "aarch64",
                "arm" => "armv7l",
                _ => std::env::consts::ARCH,
            };
            let info = lua.create_table()?;
            info.set("os", current_os)?;
            info.set("arch", arch)?;
            Ok(info)
        })?;
        os.set("info", os_info_fn)?;

        let json_encode_fn =
            lua.create_function(|lua, val: LuaValue| match lua_value_to_json(&val) {
                Ok(value) => lua.create_string(serde_json::to_string(&value).unwrap_or_default()),
                Err(error) => Err(mlua::Error::runtime(format!("json.encode: {}", error))),
            })?;
        json.set("encode", json_encode_fn)?;

        let json_decode_fn = lua.create_function(|lua, s: LuaValue| {
            let s = require_string_arg(s, "json.decode", "text", false)?;
            match serde_json::from_str::<Value>(&s) {
                Ok(value) => json_value_to_lua(lua, &value),
                Err(error) => Err(mlua::Error::runtime(format!("json.decode: {}", error))),
            }
        })?;
        json.set("decode", json_decode_fn)?;

        let cache_put_fn = lua.create_function(|lua, (value, ttl_sec): (LuaValue, LuaValue)| {
            let internal = get_vulcan_runtime_internal_table(lua).map_err(mlua::Error::runtime)?;
            let tool_name: Option<String> =
                internal.get("tool_name").map_err(mlua::Error::runtime)?;
            let skill_name: Option<String> =
                internal.get("skill_name").map_err(mlua::Error::runtime)?;
            let scope = tool_name
                .or(skill_name)
                .unwrap_or_else(|| "__runtime".to_string());
            let ttl_secs = optional_u64_arg(ttl_sec, "cache.put", "ttl_sec")?;
            let payload = lua_value_to_json(&value)
                .map_err(|error| mlua::Error::runtime(format!("cache.put: {}", error)))?;
            Ok(global_tool_cache().create(&scope, payload, ttl_secs))
        })?;
        cache.set("put", cache_put_fn)?;

        let cache_get_fn = lua.create_function(|lua, cache_id: LuaValue| {
            let internal = get_vulcan_runtime_internal_table(lua).map_err(mlua::Error::runtime)?;
            let tool_name: Option<String> =
                internal.get("tool_name").map_err(mlua::Error::runtime)?;
            let skill_name: Option<String> =
                internal.get("skill_name").map_err(mlua::Error::runtime)?;
            let scope = tool_name
                .or(skill_name)
                .unwrap_or_else(|| "__runtime".to_string());
            let cache_id = require_string_arg(cache_id, "cache.get", "cache_id", false)?;
            match global_tool_cache().get(&scope, &cache_id) {
                Some(value) => json_value_to_lua(lua, &value),
                None => Ok(LuaValue::Nil),
            }
        })?;
        cache.set("get", cache_get_fn)?;

        let cache_delete_fn = lua.create_function(|lua, cache_id: LuaValue| {
            let internal = get_vulcan_runtime_internal_table(lua).map_err(mlua::Error::runtime)?;
            let tool_name: Option<String> =
                internal.get("tool_name").map_err(mlua::Error::runtime)?;
            let skill_name: Option<String> =
                internal.get("skill_name").map_err(mlua::Error::runtime)?;
            let scope = tool_name
                .or(skill_name)
                .unwrap_or_else(|| "__runtime".to_string());
            let cache_id = require_string_arg(cache_id, "cache.delete", "cache_id", false)?;
            Ok(global_tool_cache().delete(&scope, &cache_id))
        })?;
        cache.set("delete", cache_delete_fn)?;

        let config_get_store = skill_config_store.clone();
        let config_get_fn = lua.create_function(move |lua, key: LuaValue| {
            let key = require_string_arg(key, "config.get", "key", false)?;
            let skill_id = current_vulcan_config_skill_id(lua, "vulcan.config.get")?;
            match config_get_store
                .get_value(&skill_id, &key)
                .map_err(mlua::Error::runtime)?
            {
                Some(value) => Ok(LuaValue::String(
                    lua.create_string(&value).map_err(mlua::Error::runtime)?,
                )),
                None => Ok(LuaValue::Nil),
            }
        })?;
        config.set("get", config_get_fn)?;

        let config_has_store = skill_config_store.clone();
        let config_has_fn = lua.create_function(move |lua, key: LuaValue| {
            let key = require_string_arg(key, "config.has", "key", false)?;
            let skill_id = current_vulcan_config_skill_id(lua, "vulcan.config.has")?;
            config_has_store
                .has_value(&skill_id, &key)
                .map_err(mlua::Error::runtime)
        })?;
        config.set("has", config_has_fn)?;

        let config_set_store = skill_config_store.clone();
        let config_set_fn =
            lua.create_function(move |lua, (key, value): (LuaValue, LuaValue)| {
                let key = require_string_arg(key, "config.set", "key", false)?;
                let value = require_string_arg(value, "config.set", "value", true)?;
                let skill_id = current_vulcan_config_skill_id(lua, "vulcan.config.set")?;
                config_set_store
                    .set_value(&skill_id, &key, &value)
                    .map_err(mlua::Error::runtime)?;
                Ok(true)
            })?;
        config.set("set", config_set_fn)?;

        let config_delete_store = skill_config_store.clone();
        let config_delete_fn = lua.create_function(move |lua, key: LuaValue| {
            let key = require_string_arg(key, "config.delete", "key", false)?;
            let skill_id = current_vulcan_config_skill_id(lua, "vulcan.config.delete")?;
            config_delete_store
                .delete_value(&skill_id, &key)
                .map_err(mlua::Error::runtime)
        })?;
        config.set("delete", config_delete_fn)?;

        let config_list_store = skill_config_store.clone();
        let config_list_fn = lua.create_function(move |lua, ()| {
            let skill_id = current_vulcan_config_skill_id(lua, "vulcan.config.list")?;
            let items = config_list_store
                .list_skill_values(&skill_id)
                .map_err(mlua::Error::runtime)?;
            let table = lua.create_table().map_err(mlua::Error::runtime)?;
            for (key, value) in items {
                table
                    .set(
                        key,
                        LuaValue::String(lua.create_string(&value).map_err(mlua::Error::runtime)?),
                    )
                    .map_err(mlua::Error::runtime)?;
            }
            Ok(LuaValue::Table(table))
        })?;
        config.set("list", config_list_fn)?;

        host.set("list", create_host_tool_list_fn(lua)?)?;
        let host_has_fn = create_host_tool_has_fn(lua)?;
        host.set("has", host_has_fn.clone())?;
        host.set("has_tool", host_has_fn)?;
        host.set("call", create_host_tool_call_fn(lua)?)?;

        models.set("status", create_model_status_fn(lua)?)?;
        models.set("has", create_model_has_fn(lua)?)?;
        models.set("embed", create_model_embed_fn(lua)?)?;
        models.set("llm", create_model_llm_fn(lua)?)?;

        context.set("request", lua.create_table()?)?;
        context.set("client_info", LuaValue::Nil)?;
        context.set("client_capabilities", lua.create_table()?)?;
        context.set("client_budget", lua.create_table()?)?;
        context.set("tool_config", lua.create_table()?)?;
        context.set("host_result", lua.create_table()?)?;
        context.set("skill_dir", LuaValue::Nil)?;
        context.set("entry_dir", LuaValue::Nil)?;
        context.set("entry_file", LuaValue::Nil)?;
        deps.set("tools_path", LuaValue::Nil)?;
        deps.set("lua_path", LuaValue::Nil)?;
        deps.set("ffi_path", LuaValue::Nil)?;

        let skill_management_enabled = host_options.capabilities.enable_skill_management_bridge;
        runtime_skills.set("enabled", skill_management_enabled)?;

        let runtime_skills_status_fn = lua.create_function(move |lua, ()| {
            let status = lua.create_table()?;
            let callback_registered =
                try_has_skill_management_callback().map_err(mlua::Error::runtime)?;
            status.set("enabled", skill_management_enabled)?;
            status.set("callback_registered", callback_registered)?;
            status.set("mode", "host_callback")?;
            let message = if !skill_management_enabled {
                "Skill management bridge is disabled by host policy"
            } else if callback_registered {
                "Skill management bridge is enabled and ready"
            } else {
                "Skill management bridge is enabled but no host callback is registered"
            };
            status.set("message", message)?;
            Ok(status)
        })?;
        runtime_skills.set("status", runtime_skills_status_fn)?;
        runtime_skills.set(
            "layers",
            create_runtime_skill_layers_fn(lua, runtime_skill_roots, skill_management_enabled)?,
        )?;
        runtime_skills.set(
            "install",
            create_runtime_skill_management_bridge_fn(
                lua,
                skill_management_enabled,
                RuntimeSkillManagementAction::Install,
                "install",
            )?,
        )?;
        runtime_skills.set(
            "update",
            create_runtime_skill_management_bridge_fn(
                lua,
                skill_management_enabled,
                RuntimeSkillManagementAction::Update,
                "update",
            )?,
        )?;
        runtime_skills.set(
            "uninstall",
            create_runtime_skill_management_bridge_fn(
                lua,
                skill_management_enabled,
                RuntimeSkillManagementAction::Uninstall,
                "uninstall",
            )?,
        )?;
        runtime_skills.set(
            "enable",
            create_runtime_skill_management_bridge_fn(
                lua,
                skill_management_enabled,
                RuntimeSkillManagementAction::Enable,
                "enable",
            )?,
        )?;
        runtime_skills.set(
            "disable",
            create_runtime_skill_management_bridge_fn(
                lua,
                skill_management_enabled,
                RuntimeSkillManagementAction::Disable,
                "disable",
            )?,
        )?;

        let overflow_type = lua.create_table()?;
        overflow_type.set("truncate", "truncate")?;
        overflow_type.set("page", "page")?;
        runtime.set("overflow_type", overflow_type)?;

        runtime_internal.set("tool_name", LuaValue::Nil)?;
        runtime_internal.set("skill_name", LuaValue::Nil)?;
        runtime_internal.set("entry_name", LuaValue::Nil)?;
        runtime_internal.set("root_name", LuaValue::Nil)?;
        runtime_internal.set("luaexec_active", false)?;
        runtime_internal.set("luaexec_caller_tool_name", LuaValue::Nil)?;
        runtime.set("internal", runtime_internal)?;
        runtime.set("skills", runtime_skills)?;
        runtime.set("lua", runtime_lua)?;

        let call_stub = lua.create_function(|_, _: (LuaValue, LuaValue)| {
            Err::<(), _>(mlua::Error::runtime("vulcan.call not initialized"))
        })?;
        vulcan.set("call", call_stub)?;
        vulcan.set("runtime", runtime)?;
        vulcan.set("fs", fs)?;
        vulcan.set("io", vulcan_io)?;
        vulcan.set("path", path)?;
        vulcan.set("process", process)?;
        vulcan.set("os", os)?;
        vulcan.set("json", json)?;
        vulcan.set("cache", cache)?;
        vulcan.set("config", config)?;
        vulcan.set("host", host)?;
        vulcan.set("models", models)?;
        vulcan.set("context", context)?;
        vulcan.set("deps", deps)?;

        lua.globals().set("vulcan", vulcan)?;
        Ok(())
    }
}

// ============================================================
// JSON ↔ Lua Value conversion
// ============================================================

fn json_to_lua_table(lua: &Lua, json: &Value) -> Result<Table, String> {
    json_to_lua_table_inner(lua, json).map_err(|e| e.to_string())
}

fn json_to_lua_table_inner(lua: &Lua, json: &Value) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    if let Value::Object(obj) = json {
        for (k, v) in obj {
            table.set(k.as_str(), json_value_to_lua(lua, v)?)?;
        }
    } else if let Value::Array(arr) = json {
        for (i, v) in arr.iter().enumerate() {
            table.set(i + 1, json_value_to_lua(lua, v)?)?;
        }
    }
    Ok(table)
}

fn json_value_to_lua(lua: &Lua, json: &Value) -> mlua::Result<LuaValue> {
    match json {
        Value::Null => Ok(LuaValue::Nil),
        Value::Bool(b) => Ok(LuaValue::Boolean(*b)),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(LuaValue::Integer(i))
            } else {
                Ok(LuaValue::Number(n.as_f64().unwrap_or(0.0)))
            }
        }
        Value::String(s) => Ok(LuaValue::String(lua.create_string(s)?)),
        Value::Array(_) | Value::Object(_) => {
            Ok(LuaValue::Table(json_to_lua_table_inner(lua, json)?))
        }
    }
}

fn lua_value_to_json(val: &LuaValue) -> Result<Value, String> {
    match val {
        LuaValue::Nil => Ok(Value::Null),
        LuaValue::Boolean(b) => Ok(Value::Bool(*b)),
        LuaValue::Integer(i) => Ok(Value::Number((*i).into())),
        LuaValue::Number(f) => {
            if let Some(n) = serde_json::Number::from_f64(*f) {
                Ok(Value::Number(n))
            } else {
                Ok(Value::Null)
            }
        }
        LuaValue::String(s) => Ok(Value::String(
            s.to_str().map(|b| b.to_string()).unwrap_or_default(),
        )),
        LuaValue::Table(t) => {
            // Heuristic: if raw_len() > 0, treat as array. Otherwise as object.
            if t.raw_len() > 0 {
                let arr = lua_table_to_array(t)?;
                Ok(Value::Array(arr))
            } else {
                lua_table_to_object(t)
            }
        }
        LuaValue::Function(_) => Err("Cannot convert Lua function to JSON".to_string()),
        LuaValue::Thread(_) => Err("Cannot convert Lua thread to JSON".to_string()),
        LuaValue::UserData(_) => Err("Cannot convert Lua userdata to JSON".to_string()),
        LuaValue::LightUserData(_) => Err("Cannot convert light userdata to JSON".to_string()),
        _ => Err("Unknown Lua value type".to_string()),
    }
}

fn lua_table_to_array(t: &Table) -> Result<Vec<Value>, String> {
    let len = t.raw_len();
    if len == 0 {
        // Could be empty object or empty array, default to array
        return Ok(Vec::new());
    }
    let mut arr = Vec::with_capacity(len);
    for i in 1..=len {
        let val: LuaValue = t.get(i).map_err(|e| format!("Array index {}: {}", i, e))?;
        arr.push(lua_value_to_json(&val)?);
    }
    Ok(arr)
}

fn lua_table_to_object(t: &Table) -> Result<Value, String> {
    let mut obj = serde_json::Map::new();
    for pair in t.pairs::<String, LuaValue>() {
        let (k, v) = pair.map_err(|e| format!("Table key: {}", e))?;
        obj.insert(k, lua_value_to_json(&v)?);
    }
    // Empty Lua table has no distinction between array and object.
    // If there are no string keys, treat as empty array.
    if obj.is_empty() && t.raw_len() == 0 {
        return Ok(Value::Array(Vec::new()));
    }
    Ok(Value::Object(obj))
}

#[cfg(test)]
mod tests;
