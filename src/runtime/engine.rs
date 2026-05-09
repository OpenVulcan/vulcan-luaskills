use mlua::{Function, HookTriggers, Lua, MultiValue, Table, Value as LuaValue, VmState};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock, TryLockError};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::dependency::manager::{DependencyManager, DependencyManagerConfig, ensure_directory};
use crate::entry_descriptor::{RuntimeEntryDescriptor, RuntimeEntryParameterDescriptor};
use crate::host::callbacks::{
    RuntimeEntryRegistryDelta, RuntimeHostToolAction, RuntimeHostToolRequest, RuntimeModelCaller,
    RuntimeModelEmbedRequest, RuntimeModelEmbedResponse, RuntimeModelError, RuntimeModelErrorCode,
    RuntimeModelLlmRequest, RuntimeModelLlmResponse, RuntimeModelUsage, RuntimeSkillLifecycleEvent,
    RuntimeSkillManagementAction, RuntimeSkillManagementRequest, dispatch_host_tool_request,
    dispatch_model_embed_request, dispatch_model_llm_request, dispatch_skill_management_request,
    try_has_host_tool_callback, try_has_model_embed_callback, try_has_model_llm_callback,
    try_has_skill_management_callback,
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
use crate::runtime_result::{
    NON_STRING_TOOL_RESULT_ERROR, RuntimeInvocationResult, ToolOverflowMode,
};
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

/// Create one Lua-facing runtime skill-management bridge function.
/// 创建一个面向 Lua 的运行时技能管理桥接函数。
fn create_runtime_skill_management_bridge_fn(
    lua: &Lua,
    enabled: bool,
    action: RuntimeSkillManagementAction,
    function_name: &'static str,
) -> mlua::Result<Function> {
    let action_name = function_name.to_string();
    lua.create_function(move |lua, input: LuaValue| {
        if !enabled {
            return Err(mlua::Error::runtime(format!(
                "vulcan.runtime.skills.{} is disabled by host policy",
                action_name
            )));
        }

        let payload = lua_value_to_json(&input).map_err(|error| {
            mlua::Error::runtime(format!("vulcan.runtime.skills.{}: {}", action_name, error))
        })?;
        if management_payload_targets_root_layer(&payload) {
            return Err(mlua::Error::runtime(format!(
                "vulcan.runtime.skills.{} cannot target the system-controlled ROOT layer",
                action_name
            )));
        }
        let result = dispatch_skill_management_request(&RuntimeSkillManagementRequest {
            action: action.clone(),
            authority: SkillManagementAuthority::DelegatedTool,
            input: payload,
        })
        .map_err(|error| {
            mlua::Error::runtime(format!("vulcan.runtime.skills.{}: {}", action_name, error))
        })?;
        json_value_to_lua(lua, &result)
    })
}

/// Convert one host-tool bridge error into the stable Lua table result envelope.
/// 将单个宿主工具桥接错误转换为稳定的 Lua table 结果包络。
fn host_tool_error_value(code: &str, message: impl Into<String>) -> Value {
    json!({
        "ok": false,
        "error": {
            "code": code,
            "message": message.into(),
        },
    })
}

/// Convert one host-tool callback response into a Lua table-friendly result value.
/// 将单个宿主工具回调响应转换为便于 Lua table 通讯的结果值。
fn normalize_host_tool_call_response(value: Value) -> Value {
    match value {
        Value::Object(_) => value,
        other => json!({
            "ok": true,
            "value": other,
        }),
    }
}

/// Parse the host-tool `has` callback response into one boolean.
/// 将宿主工具 `has` 回调响应解析为布尔值。
fn parse_host_tool_has_response(value: &Value) -> Result<bool, String> {
    match value {
        Value::Bool(value) => Ok(*value),
        Value::Object(object) => {
            for key in ["exists", "has", "available"] {
                if let Some(Value::Bool(value)) = object.get(key) {
                    return Ok(*value);
                }
            }
            Err("host tool has callback must return a boolean or an object with boolean exists/has/available".to_string())
        }
        _ => Err("host tool has callback must return a boolean".to_string()),
    }
}

/// Convert a Lua host-tool args table into JSON while preserving empty args as an object.
/// 将 Lua 宿主工具参数表转换为 JSON，并把空参数保持为空对象。
fn host_tool_args_table_to_json(args_table: Table) -> Result<Value, String> {
    if args_table.raw_len() == 0 && args_table.pairs::<String, LuaValue>().next().is_none() {
        return Ok(Value::Object(serde_json::Map::new()));
    }
    lua_value_to_json(&LuaValue::Table(args_table))
}

/// Create the Lua-facing `vulcan.host.list` function.
/// 创建面向 Lua 的 `vulcan.host.list` 函数。
fn create_host_tool_list_fn(lua: &Lua) -> mlua::Result<Function> {
    lua.create_function(move |lua, ()| {
        if !try_has_host_tool_callback().map_err(mlua::Error::runtime)? {
            return Ok(LuaValue::Table(lua.create_table()?));
        }
        let result = dispatch_host_tool_request(&RuntimeHostToolRequest {
            action: RuntimeHostToolAction::List,
            tool_name: None,
            args: json!({}),
        })
        .map_err(|error| mlua::Error::runtime(format!("vulcan.host.list: {}", error)))?;
        json_value_to_lua(lua, &result)
    })
}

/// Create the Lua-facing `vulcan.host.has` and `vulcan.host.has_tool` function.
/// 创建面向 Lua 的 `vulcan.host.has` 与 `vulcan.host.has_tool` 函数。
fn create_host_tool_has_fn(lua: &Lua) -> mlua::Result<Function> {
    lua.create_function(move |_, tool_name: LuaValue| {
        let tool_name = require_string_arg(tool_name, "host.has", "tool_name", false)?;
        if !try_has_host_tool_callback().map_err(mlua::Error::runtime)? {
            return Ok(false);
        }
        let result = dispatch_host_tool_request(&RuntimeHostToolRequest {
            action: RuntimeHostToolAction::Has,
            tool_name: Some(tool_name),
            args: json!({}),
        })
        .map_err(|error| mlua::Error::runtime(format!("vulcan.host.has: {}", error)))?;
        parse_host_tool_has_response(&result)
            .map_err(|error| mlua::Error::runtime(format!("vulcan.host.has: {}", error)))
    })
}

/// Create the Lua-facing `vulcan.host.call` function.
/// 创建面向 Lua 的 `vulcan.host.call` 函数。
fn create_host_tool_call_fn(lua: &Lua) -> mlua::Result<Function> {
    lua.create_function(move |lua, (tool_name, args): (LuaValue, LuaValue)| {
        let tool_name = require_string_arg(tool_name, "host.call", "tool_name", false)?;
        let args_table = require_table_arg(args, "host.call", "args")?;
        let args_value = host_tool_args_table_to_json(args_table).map_err(|error| {
            mlua::Error::runtime(format!("vulcan.host.call: invalid args table: {}", error))
        })?;
        let result = if try_has_host_tool_callback().map_err(mlua::Error::runtime)? {
            match dispatch_host_tool_request(&RuntimeHostToolRequest {
                action: RuntimeHostToolAction::Call,
                tool_name: Some(tool_name.clone()),
                args: args_value,
            }) {
                Ok(value) => normalize_host_tool_call_response(value),
                Err(error) => host_tool_error_value("host_tool_callback_error", error),
            }
        } else {
            host_tool_error_value(
                "host_tool_callback_missing",
                format!(
                    "host tool bridge has no registered callback for '{}'",
                    tool_name
                ),
            )
        };
        json_value_to_lua(lua, &result)
    })
}

/// Convert one optional model usage object into a JSON object.
/// 将单个可选模型用量对象转换为 JSON 对象。
fn model_usage_value(usage: RuntimeModelUsage) -> Value {
    let mut usage_object = serde_json::Map::new();
    if let Some(input_tokens) = usage.input_tokens {
        usage_object.insert("input_tokens".to_string(), json!(input_tokens));
    }
    if let Some(output_tokens) = usage.output_tokens {
        usage_object.insert("output_tokens".to_string(), json!(output_tokens));
    }
    Value::Object(usage_object)
}

/// Convert one structured model error into the stable Lua table result envelope.
/// 将单个结构化模型错误转换为稳定的 Lua table 返回包络。
fn runtime_model_error_value(error: RuntimeModelError) -> Value {
    let mut error_object = serde_json::Map::new();
    error_object.insert(
        "code".to_string(),
        Value::String(error.code.as_str().to_string()),
    );
    error_object.insert("message".to_string(), Value::String(error.message));
    if let Some(provider_message) = error.provider_message {
        error_object.insert(
            "provider_message".to_string(),
            Value::String(provider_message),
        );
    }
    if let Some(provider_code) = error.provider_code {
        error_object.insert("provider_code".to_string(), Value::String(provider_code));
    }
    if let Some(provider_status) = error.provider_status {
        error_object.insert("provider_status".to_string(), json!(provider_status));
    }
    json!({
        "ok": false,
        "error": Value::Object(error_object),
    })
}

/// Build one structured model error without provider-specific fields.
/// 构造一个不带 provider 特定字段的结构化模型错误。
fn runtime_model_error(
    code: RuntimeModelErrorCode,
    message: impl Into<String>,
) -> RuntimeModelError {
    RuntimeModelError {
        code,
        message: message.into(),
        provider_message: None,
        provider_code: None,
        provider_status: None,
    }
}

/// Convert one successful embedding callback response into the Lua result envelope.
/// 将单个成功的 embedding 回调响应转换为 Lua 返回包络。
fn runtime_model_embed_response_value(response: RuntimeModelEmbedResponse) -> Value {
    let mut result = json!({
        "ok": true,
        "vector": response.vector,
        "dimensions": response.dimensions,
    });
    if let Some(usage) = response.usage
        && let Value::Object(object) = &mut result
    {
        object.insert("usage".to_string(), model_usage_value(usage));
    }
    result
}

/// Convert one successful LLM callback response into the Lua result envelope.
/// 将单个成功的 LLM 回调响应转换为 Lua 返回包络。
fn runtime_model_llm_response_value(response: RuntimeModelLlmResponse) -> Value {
    let mut result = json!({
        "ok": true,
        "assistant": response.assistant,
    });
    if let Some(usage) = response.usage
        && let Value::Object(object) = &mut result
    {
        object.insert("usage".to_string(), model_usage_value(usage));
    }
    result
}

/// Read one exact non-empty UTF-8 string argument for a model function.
/// 为模型函数读取一个精确的非空 UTF-8 字符串参数。
fn runtime_model_string_arg(
    values: &[LuaValue],
    index: usize,
    fn_name: &str,
    param_name: &str,
) -> Result<String, RuntimeModelError> {
    let value = values.get(index).ok_or_else(|| {
        runtime_model_error(
            RuntimeModelErrorCode::InvalidArgument,
            format!("{fn_name}: {param_name} is required"),
        )
    })?;
    let text = match value {
        LuaValue::String(text) => text
            .to_str()
            .map_err(|_| {
                runtime_model_error(
                    RuntimeModelErrorCode::InvalidArgument,
                    format!("{fn_name}: {param_name} must be a valid UTF-8 string"),
                )
            })?
            .to_string(),
        other => {
            return Err(runtime_model_error(
                RuntimeModelErrorCode::InvalidArgument,
                format!(
                    "{fn_name}: {param_name} must be a string, got {}",
                    lua_value_type_name(other)
                ),
            ));
        }
    };
    if text.trim().is_empty() {
        return Err(runtime_model_error(
            RuntimeModelErrorCode::InvalidArgument,
            format!("{fn_name}: {param_name} must not be empty"),
        ));
    }
    if text.contains('\0') {
        return Err(runtime_model_error(
            RuntimeModelErrorCode::InvalidArgument,
            format!("{fn_name}: {param_name} must not contain NUL bytes"),
        ));
    }
    Ok(text)
}

/// Validate the exact argument count for one fixed model API.
/// 校验单个固定模型 API 的精确参数数量。
fn validate_runtime_model_arg_count(
    actual: usize,
    expected: usize,
    fn_name: &str,
) -> Result<(), RuntimeModelError> {
    if actual == expected {
        return Ok(());
    }
    Err(runtime_model_error(
        RuntimeModelErrorCode::InvalidArgument,
        format!("{fn_name}: expected {expected} argument(s), got {actual}"),
    ))
}

/// Capture the current runtime caller context for one host model callback.
/// 为单个宿主模型回调捕获当前运行时调用方上下文。
fn current_runtime_model_caller(lua: &Lua) -> Result<RuntimeModelCaller, String> {
    let internal = get_vulcan_runtime_internal_table(lua)?;
    let context = get_vulcan_context_table(lua)?;
    let request_value: LuaValue = context
        .get("request")
        .map_err(|error| format!("Failed to read vulcan.context.request: {}", error))?;
    let request_json = lua_value_to_json(&request_value)
        .map_err(|error| format!("Failed to convert request context to JSON: {}", error))?;
    let request_context = match &request_json {
        Value::Object(object) if object.is_empty() => None,
        _ => serde_json::from_value::<RuntimeRequestContext>(request_json).ok(),
    };
    let client_name = request_context
        .as_ref()
        .and_then(|context| context.client_name.clone())
        .or_else(|| {
            request_context
                .as_ref()
                .and_then(|context| context.client_info.as_ref())
                .and_then(|client_info| client_info.name.clone())
        });
    let request_id = request_context
        .as_ref()
        .and_then(|context| context.request_id.clone());
    Ok(RuntimeModelCaller {
        skill_id: internal.get("skill_name").map_err(|error| {
            format!(
                "Failed to read vulcan.runtime.internal.skill_name: {}",
                error
            )
        })?,
        entry_name: internal.get("entry_name").map_err(|error| {
            format!(
                "Failed to read vulcan.runtime.internal.entry_name: {}",
                error
            )
        })?,
        canonical_tool_name: internal.get("tool_name").map_err(|error| {
            format!(
                "Failed to read vulcan.runtime.internal.tool_name: {}",
                error
            )
        })?,
        root_name: internal.get("root_name").map_err(|error| {
            format!(
                "Failed to read vulcan.runtime.internal.root_name: {}",
                error
            )
        })?,
        skill_dir: context
            .get("skill_dir")
            .map_err(|error| format!("Failed to read vulcan.context.skill_dir: {}", error))?,
        client_name,
        request_id,
    })
}

/// Create the Lua-facing `vulcan.models.status` function.
/// 创建面向 Lua 的 `vulcan.models.status` 函数。
fn create_model_status_fn(lua: &Lua) -> mlua::Result<Function> {
    lua.create_function(move |lua, _: MultiValue| {
        let result = json!({
            "ok": true,
            "capabilities": {
                "embed": try_has_model_embed_callback().unwrap_or(false),
                "llm": try_has_model_llm_callback().unwrap_or(false),
            },
        });
        json_value_to_lua(lua, &result)
    })
}

/// Create the Lua-facing `vulcan.models.has` function.
/// 创建面向 Lua 的 `vulcan.models.has` 函数。
fn create_model_has_fn(lua: &Lua) -> mlua::Result<Function> {
    lua.create_function(move |_, args: MultiValue| {
        let values = args.into_vec();
        if values.len() != 1 {
            return Ok(false);
        }
        let capability = match &values[0] {
            LuaValue::String(text) => text.to_str().map(|text| text.to_string()).ok(),
            _ => None,
        };
        let available = match capability.as_deref() {
            Some("embed") => try_has_model_embed_callback().unwrap_or(false),
            Some("llm") => try_has_model_llm_callback().unwrap_or(false),
            _ => false,
        };
        Ok(available)
    })
}

/// Create the Lua-facing `vulcan.models.embed` function.
/// 创建面向 Lua 的 `vulcan.models.embed` 函数。
fn create_model_embed_fn(lua: &Lua) -> mlua::Result<Function> {
    lua.create_function(move |lua, args: MultiValue| {
        let values = args.into_vec();
        let result = (|| -> Result<Value, RuntimeModelError> {
            validate_runtime_model_arg_count(values.len(), 1, "vulcan.models.embed")?;
            let text = runtime_model_string_arg(&values, 0, "vulcan.models.embed", "text")?;
            let caller = current_runtime_model_caller(lua).map_err(|error| {
                runtime_model_error(RuntimeModelErrorCode::InternalError, error)
            })?;
            dispatch_model_embed_request(&RuntimeModelEmbedRequest { text, caller })
                .map(runtime_model_embed_response_value)
        })();
        let value = match result {
            Ok(value) => value,
            Err(error) => runtime_model_error_value(error),
        };
        json_value_to_lua(lua, &value)
    })
}

/// Create the Lua-facing `vulcan.models.llm` function.
/// 创建面向 Lua 的 `vulcan.models.llm` 函数。
fn create_model_llm_fn(lua: &Lua) -> mlua::Result<Function> {
    lua.create_function(move |lua, args: MultiValue| {
        let values = args.into_vec();
        let result = (|| -> Result<Value, RuntimeModelError> {
            validate_runtime_model_arg_count(values.len(), 2, "vulcan.models.llm")?;
            let system = runtime_model_string_arg(&values, 0, "vulcan.models.llm", "system")?;
            let user = runtime_model_string_arg(&values, 1, "vulcan.models.llm", "user")?;
            let caller = current_runtime_model_caller(lua).map_err(|error| {
                runtime_model_error(RuntimeModelErrorCode::InternalError, error)
            })?;
            dispatch_model_llm_request(&RuntimeModelLlmRequest {
                system,
                user,
                caller,
            })
            .map(runtime_model_llm_response_value)
        })();
        let value = match result {
            Ok(value) => value,
            Err(error) => runtime_model_error_value(error),
        };
        json_value_to_lua(lua, &value)
    })
}

/// Return whether one JSON value is exactly the ROOT layer label.
/// 返回单个 JSON 值是否正好是 ROOT 层标签。
fn payload_string_is_root_layer(value: &Value) -> bool {
    value
        .as_str()
        .map(|value| value.trim().eq_ignore_ascii_case("ROOT"))
        .unwrap_or(false)
}

/// Return whether one target-root payload value identifies the ROOT layer.
/// 返回单个目标根载荷值是否指向 ROOT 层。
fn root_target_payload_value_targets_root(value: &Value) -> bool {
    match value {
        Value::Object(object) => object.iter().any(|(key, value)| {
            let normalized_key = key.replace(['_', '-'], "").to_ascii_lowercase();
            let is_identity_key = matches!(
                normalized_key.as_str(),
                "name" | "label" | "rootname" | "layer" | "targetlayer"
            );
            (is_identity_key && payload_string_is_root_layer(value))
                || root_target_payload_value_targets_root(value)
        }),
        Value::Array(items) => items.iter().any(root_target_payload_value_targets_root),
        _ => payload_string_is_root_layer(value),
    }
}

/// Return whether one Lua skill-management payload explicitly requests the ROOT layer.
/// 返回单个 Lua 技能管理载荷是否显式请求 ROOT 层。
fn management_payload_targets_root_layer(payload: &Value) -> bool {
    match payload {
        Value::Object(object) => object.iter().any(|(key, value)| {
            let normalized_key = key.replace(['_', '-'], "").to_ascii_lowercase();
            let is_layer_key = matches!(
                normalized_key.as_str(),
                "layer" | "targetlayer" | "root" | "rootname" | "targetroot" | "targetrootname"
            );
            let targets_root = is_layer_key && root_target_payload_value_targets_root(value);
            targets_root || management_payload_targets_root_layer(value)
        }),
        Value::Array(items) => items.iter().any(management_payload_targets_root_layer),
        _ => false,
    }
}

/// Return whether a root chain contains one formal layer label.
/// 返回根链中是否包含指定正式层级标签。
fn runtime_skill_roots_contain_label(skill_roots: &[RuntimeSkillRoot], label: &str) -> bool {
    skill_roots
        .iter()
        .any(|root| root.name.trim().eq_ignore_ascii_case(label))
}

/// Build the Lua-visible layer discovery response for ordinary skill management.
/// 构造普通技能管理在 Lua 侧可见的层级发现响应。
fn create_runtime_skill_layers_fn(
    lua: &Lua,
    skill_roots: &[RuntimeSkillRoot],
    skill_management_enabled: bool,
) -> mlua::Result<Function> {
    let mut available_layers = Vec::new();
    for label in ["PROJECT", "USER"] {
        if runtime_skill_roots_contain_label(skill_roots, label) {
            available_layers.push(label.to_string());
        }
    }
    let default_layer = if available_layers.iter().any(|label| label == "USER") {
        Some("USER".to_string())
    } else if available_layers.iter().any(|label| label == "PROJECT") {
        Some("PROJECT".to_string())
    } else {
        None
    };
    let any_layer_writable = skill_management_enabled && !available_layers.is_empty();
    lua.create_function(move |lua, ()| {
        let result = lua.create_table()?;
        if let Some(default_layer) = default_layer.as_deref() {
            result.set("default", default_layer)?;
        }
        result.set("writable", any_layer_writable)?;

        let labels = lua.create_table()?;
        for (index, label) in available_layers.iter().enumerate() {
            labels.set(index + 1, label.as_str())?;
        }
        result.set("labels", labels)?;

        let layers = lua.create_table()?;
        for (index, label) in available_layers.iter().enumerate() {
            let layer = lua.create_table()?;
            layer.set("label", label.as_str())?;
            layer.set("writable", skill_management_enabled)?;
            let description = match label.as_str() {
                "PROJECT" => "Project skill layer",
                "USER" => "User skill layer",
                _ => "Skill layer",
            };
            layer.set("description", description)?;
            layers.set(index + 1, layer)?;
        }
        result.set("layers", layers)?;

        Ok(result)
    })
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

/// RunLua execution request accepted by `vulcan.runtime.lua.exec`.
/// `vulcan.runtime.lua.exec` 接收的 RunLua 执行请求结构。
#[derive(Debug, Deserialize, Serialize)]
struct RunLuaExecRequest {
    /// Human-readable task summary echoed in the result header.
    /// 展示在结果头部的人类可读任务摘要。
    #[serde(default)]
    task: String,
    /// Inline Lua source code executed inside the isolated runtime VM.
    /// 在隔离运行时虚拟机中执行的内联 Lua 源代码。
    #[serde(default)]
    code: Option<String>,
    /// Lua file path executed inside the isolated runtime VM.
    /// 在隔离运行时虚拟机中执行的 Lua 文件路径。
    #[serde(default)]
    file: Option<String>,
    /// Structured arguments exposed to Lua as `args`.
    /// 以 `args` 变量形式暴露给 Lua 的结构化参数。
    #[serde(default = "default_runlua_exec_args")]
    args: Value,
    /// Maximum execution time in milliseconds. Defaults to 60 seconds.
    /// 最大执行时长（毫秒），默认 60 秒。
    #[serde(default = "default_runlua_timeout_ms")]
    timeout_ms: u64,
    /// Internal caller tool name used to enforce luaexec reentrancy guards.
    /// 用于执行 luaexec 重入保护的内部调用者工具名称。
    #[serde(default)]
    caller_tool_name: Option<String>,
}

/// Runtime session creation request accepted by the host-facing JSON API.
/// 面向宿主 JSON API 的运行时会话创建请求。
#[derive(Debug, Deserialize)]
struct RuntimeSessionCreateRequest {
    /// Stable session identifier supplied by the host or AI task.
    /// 宿主或 AI 任务提供的稳定会话标识。
    sid: String,
    /// Requested lease TTL in seconds.
    /// 请求的租约有效期秒数。
    #[serde(default = "default_runtime_session_ttl_sec")]
    ttl_sec: u64,
    /// Whether an existing session with the same SID should be replaced.
    /// 是否替换同一 SID 下已经存在的会话。
    #[serde(default)]
    replace: bool,
}

/// Runtime session eval request accepted by the host-facing JSON API.
/// 面向宿主 JSON API 的运行时会话执行请求。
#[derive(Debug, Deserialize)]
struct RuntimeSessionEvalRequest {
    /// Opaque lease identifier returned by create.
    /// create 返回的不透明租约标识。
    lease_id: String,
    /// Optional stable session identifier echoed by the host wrapper.
    /// 由宿主包装层回传的可选稳定会话标识。
    #[serde(default)]
    sid: Option<String>,
    /// Optional SID-local generation echoed by the host wrapper.
    /// 由宿主包装层回传的可选 SID 内 generation。
    #[serde(default)]
    generation: Option<u64>,
    /// Inline Lua source code executed inside the persistent VM.
    /// 在持久 VM 内执行的内联 Lua 源码。
    code: String,
    /// Structured arguments exposed to Lua as `args`.
    /// 以 `args` 形式暴露给 Lua 的结构化参数。
    #[serde(default = "default_runlua_exec_args")]
    args: Value,
    /// Maximum execution time in milliseconds.
    /// 最大执行时长（毫秒）。
    #[serde(default = "default_runlua_timeout_ms")]
    timeout_ms: u64,
}

/// Runtime session identifier request accepted by status and close APIs.
/// status 与 close API 接收的运行时会话标识请求。
#[derive(Debug, Deserialize)]
struct RuntimeSessionLeaseRequest {
    /// Opaque lease identifier returned by create.
    /// create 返回的不透明租约标识。
    lease_id: String,
    /// Optional stable session identifier echoed by the host wrapper.
    /// 由宿主包装层回传的可选稳定会话标识。
    #[serde(default)]
    sid: Option<String>,
    /// Optional SID-local generation echoed by the host wrapper.
    /// 由宿主包装层回传的可选 SID 内 generation。
    #[serde(default)]
    generation: Option<u64>,
}

/// Runtime session list request accepted by the host-facing JSON API.
/// 面向宿主 JSON API 的运行时会话列表请求。
#[derive(Debug, Deserialize)]
struct RuntimeSessionListRequest {
    /// Optional stable session identifier used to filter the active lease list.
    /// 用于过滤活跃租约列表的可选稳定会话标识。
    #[serde(default)]
    sid: Option<String>,
}

/// Manager for persistent runtime sessions owned by one LuaEngine.
/// 单个 LuaEngine 拥有的持久运行时会话管理器。
struct RuntimeSessionManager {
    /// Mutable lease maps protected for cross-call coordination.
    /// 用于跨调用协调的可变租约映射。
    state: Mutex<RuntimeSessionManagerState>,
}

/// Mutable state inside the runtime session manager.
/// 运行时会话管理器内部的可变状态。
struct RuntimeSessionManagerState {
    /// Active or recently closed leases keyed by opaque lease id.
    /// 按不透明租约 id 索引的活跃或刚关闭租约。
    leases: HashMap<String, RuntimeSessionEntry>,
    /// Current lease id keyed by stable SID.
    /// 按稳定 SID 索引的当前租约 id。
    sid_index: HashMap<String, String>,
    /// Terminal lease tombstones retained for stable post-close and post-replace errors.
    /// 为关闭后与替换后稳定错误而保留的终态租约墓碑。
    tombstones: HashMap<String, RuntimeSessionTombstone>,
    /// Last issued generation for each SID.
    /// 每个 SID 已签发的最新 generation。
    generations: HashMap<String, u64>,
    /// Monotonic local sequence used to build lease ids.
    /// 用于构造租约 id 的本地单调序号。
    next_sequence: u64,
}

/// One persistent Lua VM runtime session.
/// 单个持久 Lua VM 运行时会话。
struct RuntimeSession {
    /// Stable session identifier supplied by the caller.
    /// 调用方提供的稳定会话标识。
    sid: String,
    /// Opaque lease identifier used for subsequent calls.
    /// 后续调用使用的不透明租约标识。
    lease_id: String,
    /// SID-local generation number.
    /// SID 内部的 generation 编号。
    generation: u64,
    /// Lease TTL in seconds refreshed by successful calls.
    /// 成功调用会刷新的租约 TTL 秒数。
    ttl_sec: u64,
    /// Monotonic expiration timestamp used for local cleanup.
    /// 用于本地清理的单调过期时间戳。
    expires_at: Instant,
    /// Host-visible expiration timestamp in Unix milliseconds.
    /// 面向宿主可见的 Unix 毫秒过期时间戳。
    expires_at_unix_ms: u128,
    /// Persistent Lua VM retained by this session.
    /// 此会话保留的持久 Lua VM。
    vm: LuaVm,
    /// Shared terminal-state marker visible across stale handles and manager retirement paths.
    /// 在陈旧句柄与管理器退役路径之间共享可见的终态状态标记。
    terminal_state: Arc<AtomicU8>,
    /// Whether the lease has been explicitly closed.
    /// 租约是否已经被显式关闭。
    closed: bool,
}

/// Active runtime-session entry stored in the manager table.
/// 存储在管理器表中的活跃运行时会话条目。
struct RuntimeSessionEntry {
    /// Locked runtime session state and retained VM.
    /// 已加锁的运行时会话状态与保留 VM。
    session: Arc<Mutex<RuntimeSession>>,
    /// Shared terminal-state marker that can be flipped without taking the session VM lock.
    /// 可在不获取会话 VM 锁的前提下切换的共享终态状态标记。
    terminal_state: Arc<AtomicU8>,
    /// Lock-free snapshot used by list operations.
    /// 供列表操作使用的无锁快照。
    snapshot: Value,
}

/// Stable runtime-session terminal states stored in the shared atomic marker.
/// 存储在共享原子标记中的稳定运行时会话终态状态。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
enum RuntimeSessionTerminalState {
    /// Lease is still active.
    /// 租约仍然处于活跃状态。
    Active = 0,
    /// Lease has been explicitly closed.
    /// 租约已被显式关闭。
    Closed = 1,
    /// Lease has expired.
    /// 租约已经过期。
    Expired = 2,
    /// Lease has been replaced by a newer SID generation.
    /// 租约已被同 SID 的更新 generation 替换。
    Replaced = 3,
}

/// Runtime session operation error with a stable code.
/// 带稳定错误码的运行时会话操作错误。
#[derive(Debug)]
struct RuntimeSessionError {
    /// Stable error code for host recovery logic.
    /// 供宿主恢复逻辑使用的稳定错误码。
    code: &'static str,
    /// Human-readable error message.
    /// 面向人的错误消息。
    message: String,
}

/// Terminal lease record retained after one session leaves the active pool.
/// 单个会话离开活跃池后保留的终态租约记录。
struct RuntimeSessionTombstone {
    /// Stable session identifier originally bound to the lease.
    /// 原本绑定到该租约的稳定会话标识。
    sid: String,
    /// Opaque lease identifier preserved for post-terminal lookups.
    /// 为终态后续查询保留的不透明租约标识。
    lease_id: String,
    /// SID-local generation number preserved for diagnostics.
    /// 用于诊断的 SID 内 generation 编号。
    generation: u64,
    /// Stable terminal error code reported after the lease leaves the active pool.
    /// 租约离开活跃池后返回的稳定终态错误码。
    code: &'static str,
    /// Monotonic retirement timestamp used to evict stale tombstones.
    /// 用于清理陈旧墓碑的单调退役时间戳。
    retired_at: Instant,
}

/// Return the default empty args object for runlua execution.
/// 返回 runlua 执行默认使用的空参数对象。
fn default_runlua_exec_args() -> Value {
    Value::Object(serde_json::Map::new())
}

/// Return the default TTL used by persistent runtime sessions.
/// 返回持久运行时会话使用的默认 TTL。
fn default_runtime_session_ttl_sec() -> u64 {
    600
}

/// Return the default timeout for runlua execution in milliseconds.
/// 返回 runlua 执行的默认超时时间（毫秒）。
fn default_runlua_timeout_ms() -> u64 {
    60_000
}

/// Return the process-wide current-directory guard used by lua file execution.
/// 返回 Lua 文件执行期间用于保护进程工作目录切换的全局互斥锁。
fn runlua_cwd_guard() -> &'static Mutex<()> {
    static RUNLUA_CWD_GUARD: OnceLock<Mutex<()>> = OnceLock::new();
    RUNLUA_CWD_GUARD.get_or_init(|| Mutex::new(()))
}

/// Build the restricted simulated request context used by internal luaexec tool calls.
/// 构建内部 luaexec 工具调用使用的受限模拟请求上下文。
fn build_luaexec_call_request_context() -> RuntimeRequestContext {
    RuntimeRequestContext {
        request_id: None,
        client_name: None,
        transport_name: Some("luaexec_call".to_string()),
        session_id: Some("luaexec-call-internal".to_string()),
        client_info: Some(RuntimeClientInfo {
            kind: Some("runtime".to_string()),
            name: Some("luaexec_call".to_string()),
            version: Some("internal-runtime".to_string()),
        }),
        client_capabilities: json!({}),
    }
}

/// One captured renderable runlua return item.
/// 一项已捕获并可渲染的 runlua 返回值。
#[derive(Debug)]
struct RunLuaRenderedValue {
    /// Render format of the current item, such as `text` or `json`.
    /// 当前项的渲染格式，例如 `text` 或 `json`。
    format: &'static str,
    /// Rendered payload already formatted for Markdown code fences.
    /// 已格式化好的载荷文本，可直接写入 Markdown 代码块。
    content: String,
}

/// Detect whether a string looks like Lua's debug-style coercion output.
/// 检测字符串是否像 Lua 对象被 `tostring` 后生成的调试文本。
fn looks_like_lua_debug_value(text: &str) -> bool {
    ["table: 0x", "function: 0x", "thread: 0x", "userdata: 0x"]
        .iter()
        .any(|prefix| text.starts_with(prefix))
}

/// Validate Windows-specific path syntax conservatively before touching the filesystem.
/// 在真正访问文件系统之前，对 Windows 路径语法做保守校验。
#[cfg(windows)]
fn has_invalid_windows_path_syntax(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.starts_with(r"\\?\") {
        return false;
    }

    let first_char = trimmed.chars().next();
    for (index, ch) in trimmed.char_indices() {
        if ch.is_control() {
            return true;
        }
        if matches!(ch, '<' | '>' | '"' | '|' | '?' | '*') {
            return true;
        }
        if ch == ':' {
            let is_drive_prefix =
                index == 1 && first_char.map(|c| c.is_ascii_alphabetic()).unwrap_or(false);
            if !is_drive_prefix {
                return true;
            }
        }
    }
    false
}

/// Require an exact UTF-8 Lua string and reject empty/blank values when needed.
/// 要求参数必须是精确的 UTF-8 Lua 字符串，并在需要时拒绝空值或纯空白值。
fn require_string_arg(
    value: LuaValue,
    fn_name: &str,
    param_name: &str,
    allow_blank: bool,
) -> mlua::Result<String> {
    let raw = match value {
        LuaValue::String(text) => text
            .to_str()
            .map_err(|_| {
                mlua::Error::runtime(format!(
                    "{fn_name}: {param_name} must be a valid UTF-8 string"
                ))
            })?
            .to_string(),
        other => {
            return Err(mlua::Error::runtime(format!(
                "{fn_name}: {param_name} must be a string, got {}",
                lua_value_type_name(&other)
            )));
        }
    };

    if !allow_blank && raw.trim().is_empty() {
        return Err(mlua::Error::runtime(format!(
            "{fn_name}: {param_name} must not be empty"
        )));
    }
    if raw.contains('\0') {
        return Err(mlua::Error::runtime(format!(
            "{fn_name}: {param_name} must not contain NUL bytes"
        )));
    }
    Ok(raw)
}

/// Validate path-like text before using it in filesystem operations.
/// 在文件系统函数真正使用路径文本前，先进行统一校验。
fn validate_path_text(text: &str, fn_name: &str, param_name: &str) -> mlua::Result<()> {
    if looks_like_lua_debug_value(text) {
        return Err(mlua::Error::runtime(format!(
            "{fn_name}: {param_name} looks like a coerced Lua object string `{text}`"
        )));
    }

    #[cfg(windows)]
    if has_invalid_windows_path_syntax(text) {
        return Err(mlua::Error::runtime(format!(
            "{fn_name}: {param_name} contains invalid Windows path syntax"
        )));
    }

    Ok(())
}

/// Require a validated path string from Lua input.
/// 从 Lua 输入中提取并校验路径字符串参数。
fn require_path_arg(value: LuaValue, fn_name: &str, param_name: &str) -> mlua::Result<String> {
    let text = require_string_arg(value, fn_name, param_name, false)?;
    validate_path_text(&text, fn_name, param_name)?;
    Ok(text)
}

/// Read an optional non-negative integer argument from Lua.
/// 从 Lua 读取可选的非负整数参数。
fn optional_u64_arg(value: LuaValue, fn_name: &str, param_name: &str) -> mlua::Result<Option<u64>> {
    match value {
        LuaValue::Nil => Ok(None),
        LuaValue::Integer(v) if v >= 0 => Ok(Some(v as u64)),
        LuaValue::Number(v) if v.is_finite() && v >= 0.0 && v.fract() == 0.0 => Ok(Some(v as u64)),
        other => Err(mlua::Error::runtime(format!(
            "{fn_name}: {param_name} must be a non-negative integer: {}",
            lua_value_type_name(&other)
        ))),
    }
}

/// Require a Lua table argument without silent coercion.
/// 要求参数必须是 Lua table，禁止静默类型转换。
fn require_table_arg(value: LuaValue, fn_name: &str, param_name: &str) -> mlua::Result<Table> {
    match value {
        LuaValue::Table(table) => Ok(table),
        other => Err(mlua::Error::runtime(format!(
            "{fn_name}: {param_name} must be a table, got {}",
            lua_value_type_name(&other)
        ))),
    }
}

/// Execution mode supported by `vulcan.exec`.
/// `vulcan.exec` 支持的执行模式。
enum ExecMode {
    Shell { command: String },
    Program { program: String, args: Vec<String> },
}

/// Parsed process execution request from Lua.
/// 从 Lua 解析得到的进程执行请求。
struct ExecRequest {
    /// Process launch mode requested by Lua.
    /// Lua 请求的进程启动模式。
    mode: ExecMode,
    /// Optional process working directory.
    /// 可选的进程工作目录。
    cwd: Option<String>,
    /// Environment variables applied to the child process.
    /// 应用到子进程的环境变量。
    env: HashMap<String, String>,
    /// Optional text written to child process stdin.
    /// 可选的子进程标准输入文本。
    stdin: Option<String>,
    /// Optional process timeout in milliseconds.
    /// 可选的进程超时时间（毫秒）。
    timeout_ms: Option<u64>,
    /// Encoding used to decode captured stdout bytes.
    /// 用于解码已捕获 stdout 字节的编码。
    stdout_encoding: RuntimeTextEncoding,
    /// Encoding used to decode captured stderr bytes.
    /// 用于解码已捕获 stderr 字节的编码。
    stderr_encoding: RuntimeTextEncoding,
    /// Encoding used to encode stdin text bytes.
    /// 用于编码 stdin 文本字节的编码。
    stdin_encoding: RuntimeTextEncoding,
}

/// Process execution result returned back to Lua.
/// 返回给 Lua 的进程执行结果。
struct ExecResult {
    /// Whether the process completed successfully.
    /// 进程是否成功完成。
    ok: bool,
    /// Whether the process completed successfully without timeout.
    /// 进程是否未超时且成功完成。
    success: bool,
    /// Process exit code when available.
    /// 可用时的进程退出码。
    code: Option<i32>,
    /// Decoded stdout text or Base64 text in byte-preserving mode.
    /// 已解码 stdout 文本，或字节保留模式下的 Base64 文本。
    stdout: String,
    /// Decoded stderr text or Base64 text in byte-preserving mode.
    /// 已解码 stderr 文本，或字节保留模式下的 Base64 文本。
    stderr: String,
    /// Whether the process timed out.
    /// 进程是否超时。
    timed_out: bool,
    /// Process-level error summary when execution failed.
    /// 执行失败时的进程级错误摘要。
    error: Option<String>,
    /// Actual stdout encoding used by the decoder.
    /// 解码器实际使用的 stdout 编码。
    stdout_encoding: String,
    /// Actual stderr encoding used by the decoder.
    /// 解码器实际使用的 stderr 编码。
    stderr_encoding: String,
    /// Whether stdout decoding used replacement or fallback behavior.
    /// stdout 解码是否使用了替换或兜底行为。
    stdout_lossy: bool,
    /// Whether stderr decoding used replacement or fallback behavior.
    /// stderr 解码是否使用了替换或兜底行为。
    stderr_lossy: bool,
    /// Byte-preserving stdout payload when available.
    /// 可用时的 stdout 字节保留载荷。
    stdout_base64: Option<String>,
    /// Byte-preserving stderr payload when available.
    /// 可用时的 stderr 字节保留载荷。
    stderr_base64: Option<String>,
}

/// Require a scalar text-like value for exec arguments and environment values.
/// 为 exec 的参数和环境变量值提取标量文本，拒绝 table/function 等复杂类型。
fn require_exec_scalar_text(
    value: LuaValue,
    fn_name: &str,
    param_name: &str,
    allow_blank: bool,
) -> mlua::Result<String> {
    match value {
        LuaValue::String(_) => require_string_arg(value, fn_name, param_name, allow_blank),
        LuaValue::Integer(number) => Ok(number.to_string()),
        LuaValue::Number(number) => {
            if !number.is_finite() {
                return Err(mlua::Error::runtime(format!(
                    "{fn_name}: {param_name} must be a finite number"
                )));
            }
            Ok(number.to_string())
        }
        LuaValue::Boolean(flag) => Ok(flag.to_string()),
        other => Err(mlua::Error::runtime(format!(
            "{fn_name}: {param_name} must be a string: {}",
            lua_value_type_name(&other)
        ))),
    }
}

/// Read an optional string field from a Lua table with strict validation.
/// 从 Lua table 中读取可选字符串字段，并执行严格校验。
fn table_get_optional_string_field(
    table: &Table,
    fn_name: &str,
    field_name: &str,
    allow_blank: bool,
) -> mlua::Result<Option<String>> {
    let value: LuaValue = table.get(field_name)?;
    match value {
        LuaValue::Nil => Ok(None),
        other => Ok(Some(require_string_arg(
            other,
            fn_name,
            field_name,
            allow_blank,
        )?)),
    }
}

/// Read an optional boolean field from a Lua table.
/// 从 Lua table 中读取可选布尔字段。
fn table_get_optional_bool_field(
    table: &Table,
    fn_name: &str,
    field_name: &str,
) -> mlua::Result<Option<bool>> {
    let value: LuaValue = table.get(field_name)?;
    match value {
        LuaValue::Nil => Ok(None),
        LuaValue::Boolean(flag) => Ok(Some(flag)),
        other => Err(mlua::Error::runtime(format!(
            "{fn_name}: {field_name} must be a boolean when provided: {}",
            lua_value_type_name(&other)
        ))),
    }
}

/// Read an optional timeout field in milliseconds from a Lua table.
/// 从 Lua table 中读取可选的毫秒级超时字段。
fn table_get_optional_timeout_field(
    table: &Table,
    fn_name: &str,
    field_name: &str,
) -> mlua::Result<Option<u64>> {
    let value: LuaValue = table.get(field_name)?;
    match value {
        LuaValue::Nil => Ok(None),
        LuaValue::Integer(number) if number > 0 => Ok(Some(number as u64)),
        LuaValue::Number(number) if number.is_finite() && number.fract() == 0.0 && number > 0.0 => {
            Ok(Some(number as u64))
        }
        other => Err(mlua::Error::runtime(format!(
            "{fn_name}: {field_name} must be a positive integer in milliseconds: {}",
            lua_value_type_name(&other)
        ))),
    }
}

/// Read an optional runtime text encoding field from a Lua table.
/// 从 Lua table 中读取可选的运行时文本编码字段。
fn table_get_optional_encoding_field(
    table: &Table,
    fn_name: &str,
    field_name: &str,
) -> mlua::Result<Option<RuntimeTextEncoding>> {
    let Some(label) = table_get_optional_string_field(table, fn_name, field_name, false)? else {
        return Ok(None);
    };
    RuntimeTextEncoding::parse(&label)
        .map(Some)
        .map_err(|error| mlua::Error::runtime(format!("{fn_name}: {field_name}: {error}")))
}

/// Read an optional string-like array field from a Lua table.
/// 从 Lua table 中读取可选的字符串类数组字段。
fn table_get_string_list_field(
    table: &Table,
    fn_name: &str,
    field_name: &str,
) -> mlua::Result<Vec<String>> {
    let value: LuaValue = table.get(field_name)?;
    match value {
        LuaValue::Nil => Ok(Vec::new()),
        other => {
            let list = require_table_arg(other, fn_name, field_name)?;
            let mut items = Vec::new();
            for (index, item) in list.sequence_values::<LuaValue>().enumerate() {
                let item = item.map_err(|error| {
                    mlua::Error::runtime(format!(
                        "{fn_name}: failed to read {field_name}[{}]: {}, {}",
                        index + 1,
                        index + 1,
                        error
                    ))
                })?;
                items.push(require_exec_scalar_text(
                    item,
                    fn_name,
                    &format!("{field_name}[{}]", index + 1),
                    true,
                )?);
            }
            Ok(items)
        }
    }
}

/// Read an optional string map field from a Lua table.
/// 从 Lua table 中读取可选的字符串映射字段。
fn table_get_string_map_field(
    table: &Table,
    fn_name: &str,
    field_name: &str,
) -> mlua::Result<HashMap<String, String>> {
    let value: LuaValue = table.get(field_name)?;
    match value {
        LuaValue::Nil => Ok(HashMap::new()),
        other => {
            let map_table = require_table_arg(other, fn_name, field_name)?;
            let mut items = HashMap::new();
            for pair in map_table.pairs::<LuaValue, LuaValue>() {
                let (key_value, field_value) = pair.map_err(|_error| {
                    mlua::Error::runtime(format!("{fn_name}: failed to read {field_name}"))
                })?;
                let key =
                    require_string_arg(key_value, fn_name, &format!("{field_name}.<key>"), false)?;
                let value_text = require_exec_scalar_text(
                    field_value,
                    fn_name,
                    &format!("{field_name}.{key}"),
                    true,
                )?;
                items.insert(key, value_text);
            }
            Ok(items)
        }
    }
}

/// Resolve the host-configured default runtime text encoding.
/// 解析宿主配置的默认运行时文本编码。
fn resolve_host_default_text_encoding(
    host_options: &LuaRuntimeHostOptions,
) -> Result<RuntimeTextEncoding, String> {
    match host_options.default_text_encoding.as_deref() {
        Some(label) if !label.trim().is_empty() => RuntimeTextEncoding::parse(label),
        _ => Ok(default_runtime_text_encoding()),
    }
}

/// Parse Lua input into an executable process request.
/// 将 Lua 输入解析为可执行的进程请求。
fn parse_exec_request(
    value: LuaValue,
    fn_name: &str,
    default_encoding: RuntimeTextEncoding,
) -> mlua::Result<ExecRequest> {
    match value {
        LuaValue::String(command_text) => Ok(ExecRequest {
            mode: ExecMode::Shell {
                command: require_string_arg(
                    LuaValue::String(command_text),
                    fn_name,
                    "command",
                    false,
                )?,
            },
            cwd: None,
            env: HashMap::new(),
            stdin: None,
            timeout_ms: None,
            stdout_encoding: default_encoding,
            stderr_encoding: default_encoding,
            stdin_encoding: default_encoding,
        }),
        LuaValue::Table(spec) => {
            let command = table_get_optional_string_field(&spec, fn_name, "command", false)?;
            let program = table_get_optional_string_field(&spec, fn_name, "program", false)?;
            let args = table_get_string_list_field(&spec, fn_name, "args")?;
            let cwd = table_get_optional_string_field(&spec, fn_name, "cwd", false)?;
            let env = table_get_string_map_field(&spec, fn_name, "env")?;
            let stdin = table_get_optional_string_field(&spec, fn_name, "stdin", true)?;
            let timeout_ms = table_get_optional_timeout_field(&spec, fn_name, "timeout_ms")?;
            let shell_override = table_get_optional_bool_field(&spec, fn_name, "shell")?;
            let encoding = table_get_optional_encoding_field(&spec, fn_name, "encoding")?
                .unwrap_or(default_encoding);
            let stdout_encoding =
                table_get_optional_encoding_field(&spec, fn_name, "stdout_encoding")?
                    .unwrap_or(encoding);
            let stderr_encoding =
                table_get_optional_encoding_field(&spec, fn_name, "stderr_encoding")?
                    .unwrap_or(encoding);
            let stdin_encoding =
                table_get_optional_encoding_field(&spec, fn_name, "stdin_encoding")?
                    .unwrap_or(encoding);

            if let Some(current_dir) = cwd.as_deref() {
                validate_path_text(current_dir, fn_name, "cwd")?;
            }

            let mode = match (command, program) {
                (Some(command_text), None) => {
                    if matches!(shell_override, Some(false)) {
                        return Err(mlua::Error::runtime(format!(
                            "{fn_name}: shell=false cannot be used with command mode"
                        )));
                    }
                    if !args.is_empty() {
                        return Err(mlua::Error::runtime(format!(
                            "{fn_name}: args is only supported with program mode"
                        )));
                    }
                    ExecMode::Shell {
                        command: command_text,
                    }
                }
                (None, Some(program_path)) => {
                    if matches!(shell_override, Some(true)) {
                        return Err(mlua::Error::runtime(format!(
                            "{fn_name}: shell=true requires command mode"
                        )));
                    }
                    ExecMode::Program {
                        program: program_path,
                        args,
                    }
                }
                (Some(_), Some(_)) => {
                    return Err(mlua::Error::runtime(format!(
                        "{fn_name}: command and program are mutually exclusive"
                    )));
                }
                (None, None) => {
                    return Err(mlua::Error::runtime(format!(
                        "{fn_name}: expected a string command or a table with command"
                    )));
                }
            };

            Ok(ExecRequest {
                mode,
                cwd,
                env,
                stdin,
                timeout_ms,
                stdout_encoding,
                stderr_encoding,
                stdin_encoding,
            })
        }
        other => Err(mlua::Error::runtime(format!(
            "{fn_name}: expected a string or table, got {}",
            lua_value_type_name(&other)
        ))),
    }
}

/// Return the default shell program and command flag for the current platform.
/// 返回当前平台默认的 shell 程序及命令参数开关。
#[cfg(windows)]
fn default_shell_launcher() -> (&'static str, &'static str) {
    ("cmd.exe", "/C")
}

/// Return the default shell program and command flag for the current platform.
/// 返回当前平台默认的 shell 程序及命令参数开关。
#[cfg(not(windows))]
fn default_shell_launcher() -> (&'static str, &'static str) {
    ("sh", "-c")
}

/// Spawn a background reader for a child process output pipe as raw bytes.
/// 为子进程输出管道启动后台读取线程，并以原始字节形式返回。
fn spawn_pipe_reader<R>(mut reader: R) -> thread::JoinHandle<Vec<u8>>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut buffer = Vec::new();
        let _ = reader.read_to_end(&mut buffer);
        buffer
    })
}

/// Spawn a background writer for a child process stdin pipe.
/// 为子进程标准输入管道启动后台写入线程。
fn spawn_stdin_writer<W>(mut writer: W, input: Vec<u8>) -> thread::JoinHandle<()>
where
    W: Write + Send + 'static,
{
    thread::spawn(move || {
        let _ = writer.write_all(&input);
        let _ = writer.flush();
    })
}

/// Build a structured process error result before stdout/stderr bytes are available.
/// 在 stdout/stderr 字节可用之前构建结构化进程错误结果。
fn exec_error_result(error_text: String, request: &ExecRequest, timed_out: bool) -> ExecResult {
    ExecResult {
        ok: false,
        success: false,
        code: None,
        stdout: String::new(),
        stderr: error_text.clone(),
        timed_out,
        error: Some(error_text),
        stdout_encoding: request.stdout_encoding.requested_label().to_string(),
        stderr_encoding: request.stderr_encoding.requested_label().to_string(),
        stdout_lossy: false,
        stderr_lossy: false,
        stdout_base64: None,
        stderr_base64: None,
    }
}

/// Execute a process request and capture its structured result.
/// 执行进程请求并捕获结构化结果。
fn execute_exec_request(request: ExecRequest) -> ExecResult {
    let stdin_bytes = match request.stdin.as_deref() {
        Some(input) => match encode_runtime_text(input, request.stdin_encoding) {
            Ok(bytes) => Some(bytes),
            Err(error) => {
                let error_text = format!("failed to encode process stdin: {error}");
                return exec_error_result(error_text, &request, false);
            }
        },
        None => None,
    };

    let mut command = match &request.mode {
        ExecMode::Shell { command } => {
            let (shell_program, shell_flag) = default_shell_launcher();
            let mut process = Command::new(shell_program);
            process.arg(shell_flag).arg(command);
            process
        }
        ExecMode::Program { program, args } => {
            let mut process = Command::new(program);
            process.args(args);
            process
        }
    };

    if let Some(current_dir) = &request.cwd {
        command.current_dir(current_dir);
    }
    if !request.env.is_empty() {
        command.envs(&request.env);
    }
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command.stdin(if stdin_bytes.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            let error_text = format!("failed to spawn process: {}", error);
            return exec_error_result(error_text, &request, false);
        }
    };

    let stdout_handle = child.stdout.take().map(spawn_pipe_reader);
    let stderr_handle = child.stderr.take().map(spawn_pipe_reader);
    let stdin_handle = match (stdin_bytes, child.stdin.take()) {
        (Some(input), Some(stdin)) => Some(spawn_stdin_writer(stdin, input)),
        _ => None,
    };

    let mut timed_out = false;
    let timeout = request.timeout_ms.map(Duration::from_millis);
    let started_at = Instant::now();

    let final_status = loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                break Some(status);
            }
            Ok(None) => {
                if let Some(limit) = timeout {
                    if started_at.elapsed() >= limit {
                        timed_out = true;
                        let _ = child.kill();
                        break child.wait().ok();
                    }
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => {
                let error_text = format!("failed to wait for process: {}", error);
                return exec_error_result(error_text, &request, timed_out);
            }
        }
    };

    if let Some(handle) = stdin_handle {
        let _ = handle.join();
    }

    let stdout_bytes = stdout_handle
        .map(|handle| handle.join().unwrap_or_default())
        .unwrap_or_default();
    let stderr_bytes = stderr_handle
        .map(|handle| handle.join().unwrap_or_default())
        .unwrap_or_default();
    let decoded_stdout = decode_runtime_text(&stdout_bytes, request.stdout_encoding);
    let decoded_stderr = decode_runtime_text(&stderr_bytes, request.stderr_encoding);
    let stdout = decoded_stdout.text;
    let mut stderr = decoded_stderr.text;

    let status = match final_status {
        Some(status) => status,
        None => {
            let error_text = "process finished without status".to_string();
            return ExecResult {
                ok: false,
                success: false,
                code: None,
                stdout,
                stderr: error_text.clone(),
                timed_out,
                error: Some(error_text),
                stdout_encoding: decoded_stdout.encoding,
                stderr_encoding: decoded_stderr.encoding,
                stdout_lossy: decoded_stdout.lossy,
                stderr_lossy: decoded_stderr.lossy,
                stdout_base64: decoded_stdout.base64,
                stderr_base64: decoded_stderr.base64,
            };
        }
    };

    let code = status.code();
    let success = !timed_out && status.success();
    let mut error = None;

    if timed_out {
        let timeout_value = request.timeout_ms.unwrap_or_default();
        let timeout_text = format!("process execution timed out after {} ms", timeout_value);
        if !stderr.is_empty() {
            stderr.push('\n');
        }
        stderr.push_str(&timeout_text);
        error = Some(timeout_text);
    } else if !success {
        error = Some(match code {
            Some(exit_code) => format!("process exited with code {}", exit_code),
            None => "process terminated without an exit code".to_string(),
        });
    }

    ExecResult {
        ok: success,
        success,
        code,
        stdout,
        stderr,
        timed_out,
        error,
        stdout_encoding: decoded_stdout.encoding,
        stderr_encoding: decoded_stderr.encoding,
        stdout_lossy: decoded_stdout.lossy,
        stderr_lossy: decoded_stderr.lossy,
        stdout_base64: decoded_stdout.base64,
        stderr_base64: decoded_stderr.base64,
    }
}

/// Convert an exec result into a Lua table for skill consumption.
/// 将 exec 结果转换为供 skill 消费的 Lua table。
fn exec_result_to_lua_table(lua: &Lua, result: ExecResult) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    table.set("ok", result.ok)?;
    table.set("success", result.success)?;
    table.set("stdout", result.stdout)?;
    table.set("stderr", result.stderr)?;
    table.set("stdout_encoding", result.stdout_encoding)?;
    table.set("stderr_encoding", result.stderr_encoding)?;
    table.set("stdout_lossy", result.stdout_lossy)?;
    table.set("stderr_lossy", result.stderr_lossy)?;
    match result.stdout_base64 {
        Some(stdout_base64) => table.set("stdout_base64", stdout_base64)?,
        None => table.set("stdout_base64", LuaValue::Nil)?,
    }
    match result.stderr_base64 {
        Some(stderr_base64) => table.set("stderr_base64", stderr_base64)?,
        None => table.set("stderr_base64", LuaValue::Nil)?,
    }
    table.set("timed_out", result.timed_out)?;
    match result.code {
        Some(code) => table.set("code", code)?,
        None => table.set("code", LuaValue::Nil)?,
    }
    match result.error {
        Some(error_text) => table.set("error", error_text)?,
        None => table.set("error", LuaValue::Nil)?,
    }
    Ok(table)
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

impl RuntimeSessionManager {
    /// Create an empty persistent runtime session manager.
    /// 创建空的持久运行时会话管理器。
    fn new() -> Self {
        Self {
            state: Mutex::new(RuntimeSessionManagerState {
                leases: HashMap::new(),
                sid_index: HashMap::new(),
                tombstones: HashMap::new(),
                generations: HashMap::new(),
                next_sequence: 0,
            }),
        }
    }

    /// Insert one new runtime session while enforcing SID uniqueness.
    /// 插入一个新的运行时会话并强制 SID 唯一。
    fn insert(
        &self,
        sid: String,
        ttl_sec: u64,
        replace: bool,
        vm: LuaVm,
    ) -> Result<Value, RuntimeSessionError> {
        let mut state = self.lock_state()?;
        Self::prune_inactive_locked(&mut state);
        if let Some(existing_lease_id) = state.sid_index.get(&sid).cloned() {
            if let Some(existing_session) = state
                .leases
                .get(&existing_lease_id)
                .map(|entry| Arc::clone(&entry.session))
            {
                match existing_session.try_lock() {
                    Ok(existing_session) => {
                        if let Some(error) = existing_session.inactive_error() {
                            Self::retire_active_lease_locked(
                                &mut state,
                                &existing_lease_id,
                                error.code,
                            );
                        } else if !replace {
                            return Err(RuntimeSessionError {
                                code: "lease_exists",
                                message: format!(
                                    "runtime session SID `{sid}` already has lease `{existing_lease_id}`"
                                ),
                            });
                        } else {
                            Self::retire_active_lease_locked(
                                &mut state,
                                &existing_lease_id,
                                "lease_replaced",
                            );
                        }
                    }
                    Err(TryLockError::WouldBlock) => {
                        if replace {
                            return Err(RuntimeSessionError {
                                code: "lease_busy",
                                message: format!(
                                    "runtime session SID `{sid}` cannot replace busy lease `{existing_lease_id}`"
                                ),
                            });
                        }
                        return Err(RuntimeSessionError {
                            code: "lease_exists",
                            message: format!(
                                "runtime session SID `{sid}` already has lease `{existing_lease_id}`"
                            ),
                        });
                    }
                    Err(TryLockError::Poisoned(_)) => {
                        return Err(RuntimeSessionError {
                            code: "lease_busy",
                            message: format!(
                                "runtime session lease `{existing_lease_id}` is unavailable because its lock is poisoned"
                            ),
                        });
                    }
                }
            } else {
                state.sid_index.remove(&sid);
            }
        }
        if state.leases.len() >= 8 {
            return Err(RuntimeSessionError {
                code: "lease_limit_exceeded",
                message: "runtime session lease limit exceeded".to_string(),
            });
        }

        state.next_sequence = state.next_sequence.saturating_add(1);
        let generation = state
            .generations
            .get(&sid)
            .copied()
            .unwrap_or(0)
            .saturating_add(1);
        state.generations.insert(sid.clone(), generation);
        let lease_id = format!("rt_{}_{}", unix_time_millis(), state.next_sequence);
        let ttl_sec = ttl_sec.clamp(1, 3_600);
        let (expires_at, expires_at_unix_ms) = runtime_session_expiry(ttl_sec);
        let terminal_state = Arc::new(AtomicU8::new(RuntimeSessionTerminalState::Active as u8));
        let session = RuntimeSession {
            sid: sid.clone(),
            lease_id: lease_id.clone(),
            generation,
            ttl_sec,
            expires_at,
            expires_at_unix_ms,
            vm,
            terminal_state: Arc::clone(&terminal_state),
            closed: false,
        };
        let snapshot = session.status_payload();
        state.leases.insert(
            lease_id.clone(),
            RuntimeSessionEntry {
                session: Arc::new(Mutex::new(session)),
                terminal_state,
                snapshot,
            },
        );
        state.sid_index.insert(sid.clone(), lease_id.clone());

        Ok(json!({
            "ok": true,
            "sid": sid,
            "lease_id": lease_id,
            "generation": generation,
            "ttl_sec": ttl_sec,
            "expires_at_unix_ms": expires_at_unix_ms
        }))
    }

    /// Get one session handle by lease id.
    /// 按租约 id 获取一个会话句柄。
    fn get(
        &self,
        lease_id: &str,
        expected_sid: Option<&str>,
        expected_generation: Option<u64>,
    ) -> Result<Arc<Mutex<RuntimeSession>>, RuntimeSessionError> {
        let mut state = self.lock_state()?;
        Self::prune_inactive_locked(&mut state);
        if let Some(entry) = state.leases.get(lease_id) {
            let session = Arc::clone(&entry.session);
            let session_guard = session.try_lock().map_err(|_| RuntimeSessionError {
                code: "lease_busy",
                message: format!("runtime session lease `{lease_id}` is busy"),
            })?;
            Self::validate_session_identity(&session_guard, expected_sid, expected_generation)?;
            drop(session_guard);
            return Ok(session);
        }
        if let Some(tombstone) = state.tombstones.get(lease_id) {
            Self::validate_tombstone_identity(tombstone, expected_sid, expected_generation)?;
            return Err(tombstone.as_error());
        }
        Err(RuntimeSessionError {
            code: "lease_not_found",
            message: format!("runtime session lease `{lease_id}` was not found"),
        })
    }

    /// Return a compact status payload for one runtime session.
    /// 返回单个运行时会话的紧凑状态载荷。
    fn status(
        &self,
        lease_id: &str,
        expected_sid: Option<&str>,
        expected_generation: Option<u64>,
    ) -> Result<Value, RuntimeSessionError> {
        let session = self.get(lease_id, expected_sid, expected_generation)?;
        let session = session.try_lock().map_err(|_| RuntimeSessionError {
            code: "lease_busy",
            message: format!("runtime session lease `{lease_id}` is busy"),
        })?;
        if let Some(error) = session.inactive_error() {
            return Err(error);
        }
        Ok(session.status_payload())
    }

    /// Return a stable active-lease listing payload.
    /// 返回稳定的活跃租约列表载荷。
    fn list(&self, sid: Option<&str>) -> Result<Value, RuntimeSessionError> {
        let mut state = self.lock_state()?;
        Self::prune_inactive_locked(&mut state);
        let mut leases = Vec::new();
        for entry in state.leases.values() {
            if sid.is_some_and(|expected_sid| entry.snapshot["sid"].as_str() != Some(expected_sid))
            {
                continue;
            }
            leases.push(entry.snapshot.clone());
        }
        leases.sort_by(compare_runtime_session_payloads);
        Ok(json!({
            "ok": true,
            "leases": leases,
        }))
    }

    /// Mark one runtime session closed.
    /// 将一个运行时会话标记为已关闭。
    fn close(
        &self,
        lease_id: &str,
        expected_sid: Option<&str>,
        expected_generation: Option<u64>,
    ) -> Result<Value, RuntimeSessionError> {
        let mut state = self.lock_state()?;
        Self::prune_inactive_locked(&mut state);
        let Some((session, terminal_state)) = state.leases.get(lease_id).map(|entry| {
            (
                Arc::clone(&entry.session),
                Arc::clone(&entry.terminal_state),
            )
        }) else {
            if let Some(tombstone) = state.tombstones.get(lease_id) {
                Self::validate_tombstone_identity(tombstone, expected_sid, expected_generation)?;
                return Err(tombstone.as_error());
            }
            return Err(RuntimeSessionError {
                code: "lease_not_found",
                message: format!("runtime session lease `{lease_id}` was not found"),
            });
        };
        let mut session = session.try_lock().map_err(|_| RuntimeSessionError {
            code: "lease_busy",
            message: format!("runtime session lease `{lease_id}` is busy"),
        })?;
        Self::validate_session_identity(&session, expected_sid, expected_generation)?;
        terminal_state.store(RuntimeSessionTerminalState::Closed as u8, Ordering::Release);
        session.closed = true;
        let payload = session.close_payload();
        let tombstone = RuntimeSessionTombstone::from_session(&session, "lease_closed");
        let sid = session.sid.clone();
        drop(session);
        state.leases.remove(lease_id);
        if state
            .sid_index
            .get(&sid)
            .is_some_and(|current| current == lease_id)
        {
            state.sid_index.remove(&sid);
        }
        state.tombstones.insert(lease_id.to_string(), tombstone);
        Ok(payload)
    }

    /// Update the cached active snapshot for one runtime session lease when it is still active.
    /// 当运行时会话租约仍然活跃时更新其缓存快照。
    fn update_active_snapshot(
        &self,
        lease_id: &str,
        snapshot: Value,
    ) -> Result<(), RuntimeSessionError> {
        let mut state = self.lock_state()?;
        if let Some(entry) = state.leases.get_mut(lease_id) {
            entry.snapshot = snapshot;
        }
        Ok(())
    }

    /// Lock the manager state with a stable runtime error.
    /// 使用稳定运行时错误锁定管理器状态。
    fn lock_state(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, RuntimeSessionManagerState>, RuntimeSessionError> {
        self.state.lock().map_err(|_| RuntimeSessionError {
            code: "lease_manager_poisoned",
            message: "runtime session manager lock poisoned".to_string(),
        })
    }

    /// Remove expired or closed sessions from the active indexes.
    /// 从活跃索引中移除已过期或已关闭的会话。
    fn prune_inactive_locked(state: &mut RuntimeSessionManagerState) {
        let now = Instant::now();
        let mut removed = Vec::new();
        for (lease_id, entry) in &state.leases {
            let should_remove = entry
                .session
                .try_lock()
                .map(|session| now >= session.expires_at)
                .unwrap_or(false);
            if should_remove {
                removed.push(lease_id.clone());
            }
        }
        for lease_id in removed {
            Self::retire_active_lease_locked(state, &lease_id, "lease_expired");
        }
        let tombstone_ttl = runtime_session_tombstone_ttl();
        state
            .tombstones
            .retain(|_, tombstone| now.duration_since(tombstone.retired_at) < tombstone_ttl);
    }

    /// Move one active lease into the terminal tombstone table.
    /// 将单个活跃租约移动到终态墓碑表中。
    fn retire_active_lease_locked(
        state: &mut RuntimeSessionManagerState,
        lease_id: &str,
        code: &'static str,
    ) {
        if let Some(entry) = state.leases.get(lease_id) {
            entry.terminal_state.store(
                runtime_session_terminal_state_from_code(code) as u8,
                Ordering::Release,
            );
        }
        let Some(entry) = state.leases.remove(lease_id) else {
            return;
        };
        let tombstone = RuntimeSessionTombstone::from_snapshot(&entry.snapshot, code);
        if state
            .sid_index
            .get(&tombstone.sid)
            .is_some_and(|current| current == lease_id)
        {
            state.sid_index.remove(&tombstone.sid);
        }
        state.tombstones.insert(lease_id.to_string(), tombstone);
    }

    /// Validate one active runtime session against optional host-echoed identity fields.
    /// 使用可选宿主回传身份字段校验单个活跃运行时会话。
    fn validate_session_identity(
        session: &RuntimeSession,
        expected_sid: Option<&str>,
        expected_generation: Option<u64>,
    ) -> Result<(), RuntimeSessionError> {
        Self::validate_identity_parts(
            &session.lease_id,
            &session.sid,
            session.generation,
            expected_sid,
            expected_generation,
        )
    }

    /// Validate one terminal runtime-session tombstone against optional host-echoed identity fields.
    /// 使用可选宿主回传身份字段校验单个终态运行时会话墓碑。
    fn validate_tombstone_identity(
        tombstone: &RuntimeSessionTombstone,
        expected_sid: Option<&str>,
        expected_generation: Option<u64>,
    ) -> Result<(), RuntimeSessionError> {
        Self::validate_identity_parts(
            &tombstone.lease_id,
            &tombstone.sid,
            tombstone.generation,
            expected_sid,
            expected_generation,
        )
    }

    /// Validate the stable SID and generation of one runtime-session record.
    /// 校验单个运行时会话记录的稳定 SID 与 generation。
    fn validate_identity_parts(
        lease_id: &str,
        actual_sid: &str,
        actual_generation: u64,
        expected_sid: Option<&str>,
        expected_generation: Option<u64>,
    ) -> Result<(), RuntimeSessionError> {
        if let Some(expected_sid) = expected_sid {
            if actual_sid != expected_sid {
                return Err(RuntimeSessionError {
                    code: "lease_sid_mismatch",
                    message: format!(
                        "runtime session lease `{lease_id}` belongs to sid `{actual_sid}`, not `{expected_sid}`"
                    ),
                });
            }
        }
        if let Some(expected_generation) = expected_generation {
            if actual_generation != expected_generation {
                return Err(RuntimeSessionError {
                    code: "lease_generation_mismatch",
                    message: format!(
                        "runtime session lease `{lease_id}` generation mismatch: expected {expected_generation}, actual {actual_generation}"
                    ),
                });
            }
        }
        Ok(())
    }
}

impl RuntimeSession {
    /// Return the stable non-active error when this session can no longer serve host calls.
    /// 当当前会话不再能够服务宿主调用时返回稳定的非活跃错误。
    fn inactive_error(&self) -> Option<RuntimeSessionError> {
        if let Some(code) =
            runtime_session_terminal_code_from_state(self.terminal_state.load(Ordering::Acquire))
        {
            return Some(RuntimeSessionError {
                code,
                message: format!(
                    "{} (sid `{}`, generation {})",
                    runtime_session_terminal_message(code, &self.lease_id),
                    self.sid,
                    self.generation
                ),
            });
        }
        if self.closed {
            return Some(RuntimeSessionError {
                code: "lease_closed",
                message: format!("runtime session lease `{}` is closed", self.lease_id),
            });
        }
        if self.is_expired() {
            return Some(RuntimeSessionError {
                code: "lease_expired",
                message: format!("runtime session lease `{}` is expired", self.lease_id),
            });
        }
        None
    }

    /// Refresh the lease expiration after one accepted operation.
    /// 在一次已接受操作后刷新租约过期时间。
    fn refresh(&mut self) {
        let (expires_at, expires_at_unix_ms) = runtime_session_expiry(self.ttl_sec);
        self.expires_at = expires_at;
        self.expires_at_unix_ms = expires_at_unix_ms;
    }

    /// Return whether this runtime session is expired.
    /// 返回此运行时会话是否已经过期。
    fn is_expired(&self) -> bool {
        Instant::now() >= self.expires_at
    }

    /// Return a JSON status payload for this runtime session.
    /// 返回此运行时会话的 JSON 状态载荷。
    fn status_payload(&self) -> Value {
        json!({
            "ok": runtime_session_terminal_code_from_state(
                self.terminal_state.load(Ordering::Acquire),
            ).is_none() && !self.closed && !self.is_expired(),
            "sid": self.sid.clone(),
            "lease_id": self.lease_id.clone(),
            "generation": self.generation,
            "ttl_sec": self.ttl_sec,
            "expires_at_unix_ms": self.expires_at_unix_ms,
            "closed": self.closed,
            "expired": self.is_expired()
        })
    }

    /// Return a JSON payload for one successful close operation.
    /// 返回一次成功关闭操作的 JSON 载荷。
    fn close_payload(&self) -> Value {
        json!({
            "ok": true,
            "sid": self.sid.clone(),
            "lease_id": self.lease_id.clone(),
            "generation": self.generation,
            "ttl_sec": self.ttl_sec,
            "expires_at_unix_ms": self.expires_at_unix_ms,
            "closed": self.closed,
            "expired": self.is_expired()
        })
    }
}

impl RuntimeSessionTombstone {
    /// Build one terminal tombstone from one active runtime session snapshot.
    /// 基于单个活跃运行时会话快照构建终态墓碑。
    fn from_session(session: &RuntimeSession, code: &'static str) -> Self {
        Self {
            sid: session.sid.clone(),
            lease_id: session.lease_id.clone(),
            generation: session.generation,
            code,
            retired_at: Instant::now(),
        }
    }

    /// Build one terminal tombstone from one cached active snapshot.
    /// 基于一份缓存的活跃快照构建终态墓碑。
    fn from_snapshot(snapshot: &Value, code: &'static str) -> Self {
        Self {
            sid: snapshot
                .get("sid")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            lease_id: snapshot
                .get("lease_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            generation: snapshot
                .get("generation")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            code,
            retired_at: Instant::now(),
        }
    }

    /// Convert this tombstone into one stable runtime-session error.
    /// 将当前墓碑转换为稳定的运行时会话错误。
    fn as_error(&self) -> RuntimeSessionError {
        RuntimeSessionError {
            code: self.code,
            message: format!(
                "{} (sid `{}`, generation {})",
                runtime_session_terminal_message(self.code, &self.lease_id),
                self.sid,
                self.generation
            ),
        }
    }
}

/// Return the current Unix timestamp in milliseconds.
/// 返回当前 Unix 毫秒时间戳。
fn unix_time_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

/// Calculate monotonic and host-visible expiration timestamps.
/// 计算单调时间与宿主可见的过期时间戳。
fn runtime_session_expiry(ttl_sec: u64) -> (Instant, u128) {
    let ttl = Duration::from_secs(ttl_sec);
    (
        Instant::now() + ttl,
        unix_time_millis().saturating_add(ttl.as_millis()),
    )
}

/// Return the tombstone retention window for terminal runtime session records.
/// 返回终态运行时会话记录的墓碑保留时间窗口。
fn runtime_session_tombstone_ttl() -> Duration {
    Duration::from_secs(3_600)
}

/// Convert one stable terminal error code into its shared atomic terminal-state value.
/// 将稳定终态错误码转换为共享原子终态状态值。
fn runtime_session_terminal_state_from_code(code: &'static str) -> RuntimeSessionTerminalState {
    match code {
        "lease_closed" => RuntimeSessionTerminalState::Closed,
        "lease_expired" => RuntimeSessionTerminalState::Expired,
        "lease_replaced" => RuntimeSessionTerminalState::Replaced,
        _ => RuntimeSessionTerminalState::Active,
    }
}

/// Convert one shared atomic terminal-state value back into its stable terminal error code.
/// 将共享原子终态状态值转换回稳定终态错误码。
fn runtime_session_terminal_code_from_state(state: u8) -> Option<&'static str> {
    match state {
        value if value == RuntimeSessionTerminalState::Closed as u8 => Some("lease_closed"),
        value if value == RuntimeSessionTerminalState::Expired as u8 => Some("lease_expired"),
        value if value == RuntimeSessionTerminalState::Replaced as u8 => Some("lease_replaced"),
        _ => None,
    }
}

/// Compare two runtime-session payloads for stable host-visible listing order.
/// 比较两个运行时会话载荷以生成稳定的宿主可见列表顺序。
fn compare_runtime_session_payloads(left: &Value, right: &Value) -> std::cmp::Ordering {
    let left_sid = left.get("sid").and_then(Value::as_str).unwrap_or_default();
    let right_sid = right.get("sid").and_then(Value::as_str).unwrap_or_default();
    left_sid
        .cmp(right_sid)
        .then_with(|| {
            left.get("generation")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                .cmp(&right.get("generation").and_then(Value::as_u64).unwrap_or(0))
        })
        .then_with(|| {
            left.get("lease_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .cmp(
                    right
                        .get("lease_id")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                )
        })
}

/// Build one stable human-readable message for one terminal runtime-session state.
/// 为单个运行时会话终态构建稳定的人类可读消息。
fn runtime_session_terminal_message(code: &'static str, lease_id: &str) -> String {
    match code {
        "lease_closed" => format!("runtime session lease `{lease_id}` is closed"),
        "lease_expired" => format!("runtime session lease `{lease_id}` is expired"),
        "lease_replaced" => format!("runtime session lease `{lease_id}` was replaced"),
        _ => format!("runtime session lease `{lease_id}` is not active"),
    }
}

/// Build a stable JSON error payload for runtime session operations.
/// 为运行时会话操作构建稳定 JSON 错误载荷。
fn runtime_session_error_payload(error: RuntimeSessionError) -> Value {
    json!({
        "ok": false,
        "error_code": error.code,
        "message": error.message
    })
}

/// Validate and normalize one runtime session SID.
/// 校验并归一化单个运行时会话 SID。
fn normalize_runtime_session_sid(value: &str) -> Result<String, String> {
    let sid = value.trim();
    if sid.is_empty() {
        return Err("runtime session sid must not be empty".to_string());
    }
    if sid.len() > 128 {
        return Err("runtime session sid must not exceed 128 bytes".to_string());
    }
    if sid.contains('\0') {
        return Err("runtime session sid must not contain NUL bytes".to_string());
    }
    Ok(sid.to_string())
}

/// Parse Lua multi-return values into the host's unified string-result protocol.
/// 把 Lua 工具的多返回值解析为宿主统一字符串结果协议。
fn parse_tool_call_output(
    values: MultiValue,
    display_name: &str,
) -> Result<RuntimeInvocationResult, String> {
    let values_vec: Vec<LuaValue> = values.into_vec();
    if values_vec.is_empty() {
        return Err(format!(
            "Lua skill '{}' must return a plain string result",
            display_name
        ));
    }

    if values_vec.len() > 3 {
        return Err(format!(
            "Lua skill '{}' must return content[, overflow_mode[, template_hint]]",
            display_name
        ));
    }

    let content = match &values_vec[0] {
        LuaValue::String(text) => text
            .to_str()
            .map_err(|error| {
                format!(
                    "Lua skill '{}' returned an invalid UTF-8 string: {}",
                    display_name, error
                )
            })?
            .to_string(),
        other => {
            return Err(format!(
                "{} (skill='{}', actual_type='{}')",
                NON_STRING_TOOL_RESULT_ERROR,
                display_name,
                lua_value_type_name(other)
            ));
        }
    };

    let overflow_mode = match values_vec.get(1) {
        None | Some(LuaValue::Nil) => None,
        Some(LuaValue::String(text)) => {
            let mode_text = text.to_str().map_err(|error| {
                format!(
                    "Lua skill '{}' returned an invalid overflow mode string: {}",
                    display_name, error
                )
            })?;
            Some(ToolOverflowMode::parse(&mode_text).ok_or_else(|| {
                format!(
                    "Lua skill '{}' returned an unsupported overflow mode: {}",
                    display_name, mode_text
                )
            })?)
        }
        Some(_) => {
            return Err(format!(
                "Lua skill '{}' must return overflow mode as a string constant",
                display_name
            ));
        }
    };

    let template_hint = match values_vec.get(2) {
        None | Some(LuaValue::Nil) => None,
        Some(LuaValue::String(text)) => {
            let name = text.to_str().map_err(|error| {
                format!(
                    "Lua skill '{}' returned an invalid template name: {}",
                    display_name, error
                )
            })?;
            let trimmed = name.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        Some(_) => {
            return Err(format!(
                "Lua skill '{}' must return template_hint as a string",
                display_name
            ));
        }
    };

    Ok(RuntimeInvocationResult::from_content_parts(
        content,
        overflow_mode,
        template_hint,
    ))
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
        }))
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
        let manager = self
            .skill_manager_for(&target_root)
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        let prepared = match action {
            crate::skill::manager::SkillLifecycleAction::Install => manager
                .prepare_install_skill(plane, operation_roots, request)
                .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?,
            crate::skill::manager::SkillLifecycleAction::Update => manager
                .prepare_update_skill(plane, operation_roots, request)
                .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?,
            _ => unreachable!("unsupported apply action should have returned early"),
        };
        let mut result = match &prepared {
            PreparedSkillApply::Immediate(result) => result.clone(),
            PreparedSkillApply::Install(_) | PreparedSkillApply::Update(_) => {
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
                    return Err(format!(
                        "Failed to reload LuaSkills after {:?}: {}.{}{}",
                        action, reload_error, rollback_message, restore_message
                    )
                    .into());
                }

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
                        format!(
                            "Failed to finalize {:?}: {}.{}{}",
                            action, error, rollback_message, restore_message
                        )
                        .into()
                    },
                )?;
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

    /// Populate the `vulcan.runtime.lua.exec` bridge for normal skill VMs.
    /// 为普通 skill 虚拟机注入 `vulcan.runtime.lua.exec` 桥接函数。
    fn populate_vulcan_luaexec_bridge(
        lua: &Lua,
        host_options: Arc<LuaRuntimeHostOptions>,
        runlua_pool: Arc<LuaVmPool>,
        skill_config_store: Arc<SkillConfigStore>,
        skills: Arc<HashMap<String, LoadedSkill>>,
        entry_registry: Arc<BTreeMap<String, ResolvedEntryTarget>>,
        runtime_skill_roots: Vec<RuntimeSkillRoot>,
        lancedb_host: Option<Arc<LanceDbSkillHost>>,
        sqlite_host: Option<Arc<SqliteSkillHost>>,
    ) -> Result<(), String> {
        let runtime_lua = get_vulcan_runtime_lua_table(lua)?;

        let exec_fn = lua
            .create_function(move |lua, input: LuaValue| {
                let input_table = require_table_arg(input, "runtime.lua.exec", "input")?;
                let input_json = lua_value_to_json(&LuaValue::Table(input_table))
                    .map_err(mlua::Error::runtime)?;
                let mut request: RunLuaExecRequest =
                    serde_json::from_value(input_json).map_err(|error| {
                        mlua::Error::runtime(format!("luaexec input is invalid: {}", error))
                    })?;
                let internal =
                    get_vulcan_runtime_internal_table(lua).map_err(mlua::Error::runtime)?;
                let caller_tool_name: Option<String> =
                    internal.get("tool_name").map_err(mlua::Error::runtime)?;
                request.caller_tool_name = caller_tool_name
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty());
                let rendered = LuaEngine::execute_runlua_request_inline_with_runtime(
                    &request,
                    runlua_pool.clone(),
                    skills.clone(),
                    entry_registry.clone(),
                    host_options.clone(),
                    skill_config_store.clone(),
                    runtime_skill_roots.clone(),
                    lancedb_host.clone(),
                    sqlite_host.clone(),
                )
                .map_err(mlua::Error::runtime)?;
                Ok(LuaValue::String(
                    lua.create_string(&rendered).map_err(mlua::Error::runtime)?,
                ))
            })
            .map_err(|error| format!("Failed to create vulcan.runtime.lua.exec: {}", error))?;
        runtime_lua
            .set("exec", exec_fn)
            .map_err(|error| format!("Failed to set vulcan.runtime.lua.exec: {}", error))?;
        Ok(())
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

            parse_tool_call_output(result, &display_tool_name).map_err(|e| {
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

    /// Execute arbitrary Lua code inside one already selected VM lease.
    /// 在一个已经选定的虚拟机租约中执行任意 Lua 代码。
    fn run_lua_with_lease(
        &self,
        lease: &mut LuaVmLease,
        code: &str,
        args: &Value,
        invocation_context: Option<&LuaInvocationContext>,
    ) -> Result<Value, String> {
        let scope_guard = LuaVmRequestScopeGuard::new(lease, self.host_options.as_ref())?;
        let lua = scope_guard.lua();
        Self::populate_vulcan_request_context(lua, invocation_context)?;
        populate_vulcan_internal_execution_context(
            lua,
            &VulcanInternalExecutionContext::default(),
        )?;
        populate_vulcan_file_context(lua, None, None)?;
        populate_vulcan_dependency_context(lua, self.host_options.as_ref(), None, None)?;
        Self::populate_vulcan_lancedb_context(lua, None, None)?;
        Self::populate_vulcan_sqlite_context(lua, None, None)?;

        // Build a wrapper that passes args as a local variable.
        // 构造包装代码，将 args 作为局部变量传入 Lua 片段。
        let args_table = json_to_lua_table(lua, args)?;
        lua.globals()
            .set("__runlua_args", args_table)
            .map_err(|e| format!("Failed to set args: {}", e))?;

        let wrapper = format!(
            "return (function()\n  local args = __runlua_args\n  {}\nend)()",
            code
        );

        let run_result = (|| {
            let result = lua.load(&wrapper).eval::<LuaValue>().map_err(|e| {
                let msg = format!("Lua run_lua error: {}", e);
                log_error(format!("[LuaSkill:error] {}", msg));
                msg
            })?;

            lua_value_to_json(&result)
        })();
        let cleanup_result = scope_guard.finish();
        match (run_result, cleanup_result) {
            (Ok(result), Ok(())) => Ok(result),
            (Ok(_), Err(cleanup_error)) => Err(cleanup_error),
            (Err(run_error), Ok(())) => Err(run_error),
            (Err(run_error), Err(cleanup_error)) => Err(format!(
                "{}; pooled Lua VM cleanup failed: {}",
                run_error, cleanup_error
            )),
        }
    }

    /// Execute arbitrary Lua code against the current active runtime view and return the result.
    /// 针对当前已激活运行时视图执行任意 Lua 代码并返回结果。
    pub fn run_lua(
        &self,
        code: &str,
        args: &Value,
        invocation_context: Option<&LuaInvocationContext>,
    ) -> Result<Value, String> {
        let mut lease = self.acquire_vm()?;
        self.run_lua_with_lease(&mut lease, code, args, invocation_context)
    }

    /// Create one persistent runtime session and return a stable JSON response.
    /// 创建一个持久运行时会话并返回稳定 JSON 响应。
    pub fn create_runtime_session_json(&self, request_json: &str) -> Result<String, String> {
        let mut request: RuntimeSessionCreateRequest = serde_json::from_str(request_json)
            .map_err(|error| format!("Invalid runtime session create JSON: {}", error))?;
        request.sid = normalize_runtime_session_sid(&request.sid)?;
        let vm = Self::create_runlua_vm(
            &self.skills,
            &self.entry_registry,
            self.host_options.clone(),
            self.skill_config_store.clone(),
            self.runtime_skill_roots.clone(),
            self.lancedb_host.clone(),
            self.sqlite_host.clone(),
        )?;
        Self::install_managed_io_compat_for_runtime(&vm.lua, self.host_options.as_ref())?;
        let payload = self
            .runtime_sessions
            .insert(request.sid, request.ttl_sec, request.replace, vm)
            .unwrap_or_else(runtime_session_error_payload);
        serde_json::to_string(&payload)
            .map_err(|error| format!("Runtime session create JSON encode failed: {}", error))
    }

    /// Evaluate Lua code inside one persistent runtime session and return a stable JSON response.
    /// 在一个持久运行时会话中执行 Lua 代码并返回稳定 JSON 响应。
    pub fn eval_runtime_session_json(&self, request_json: &str) -> Result<String, String> {
        let mut request: RuntimeSessionEvalRequest = serde_json::from_str(request_json)
            .map_err(|error| format!("Invalid runtime session eval JSON: {}", error))?;
        if let Some(sid) = request.sid.as_mut() {
            *sid = normalize_runtime_session_sid(sid)?;
        }
        if request.timeout_ms == 0 {
            return Err("runtime session eval timeout_ms must be greater than 0".to_string());
        }
        let session = match self.runtime_sessions.get(
            &request.lease_id,
            request.sid.as_deref(),
            request.generation,
        ) {
            Ok(session) => session,
            Err(error) => {
                return serde_json::to_string(&runtime_session_error_payload(error)).map_err(
                    |encode_error| {
                        format!("Runtime session eval JSON encode failed: {}", encode_error)
                    },
                );
            }
        };
        let mut session = match session.try_lock() {
            Ok(session) => session,
            Err(_) => {
                let payload = runtime_session_error_payload(RuntimeSessionError {
                    code: "lease_busy",
                    message: format!("runtime session lease `{}` is busy", request.lease_id),
                });
                return serde_json::to_string(&payload).map_err(|error| {
                    format!("Runtime session eval JSON encode failed: {}", error)
                });
            }
        };
        let (payload, refreshed_snapshot) = match Self::ensure_runtime_session_active(&mut session)
        {
            Ok(()) => match self.eval_runtime_session_locked(&mut session, &request) {
                Ok(result) => {
                    session.refresh();
                    (
                        json!({
                            "ok": true,
                            "sid": session.sid.clone(),
                            "lease_id": session.lease_id.clone(),
                            "generation": session.generation,
                            "expires_at_unix_ms": session.expires_at_unix_ms,
                            "result": result
                        }),
                        Some(session.status_payload()),
                    )
                }
                Err(message) => (
                    runtime_session_error_payload(RuntimeSessionError {
                        code: "eval_failed",
                        message,
                    }),
                    Some(session.status_payload()),
                ),
            },
            Err(error) => (runtime_session_error_payload(error), None),
        };
        drop(session);
        if let Some(snapshot) = refreshed_snapshot {
            let _ = self
                .runtime_sessions
                .update_active_snapshot(&request.lease_id, snapshot);
        }
        serde_json::to_string(&payload)
            .map_err(|error| format!("Runtime session eval JSON encode failed: {}", error))
    }

    /// Return status for one persistent runtime session as JSON.
    /// 以 JSON 返回一个持久运行时会话的状态。
    pub fn runtime_session_status_json(&self, request_json: &str) -> Result<String, String> {
        let mut request: RuntimeSessionLeaseRequest = serde_json::from_str(request_json)
            .map_err(|error| format!("Invalid runtime session status JSON: {}", error))?;
        if let Some(sid) = request.sid.as_mut() {
            *sid = normalize_runtime_session_sid(sid)?;
        }
        let payload = self
            .runtime_sessions
            .status(
                &request.lease_id,
                request.sid.as_deref(),
                request.generation,
            )
            .unwrap_or_else(runtime_session_error_payload);
        serde_json::to_string(&payload)
            .map_err(|error| format!("Runtime session status JSON encode failed: {}", error))
    }

    /// List active persistent runtime sessions and return a stable JSON response.
    /// 列出活跃的持久运行时会话并返回稳定 JSON 响应。
    pub fn list_runtime_sessions_json(&self, request_json: &str) -> Result<String, String> {
        let mut request: RuntimeSessionListRequest = serde_json::from_str(request_json)
            .map_err(|error| format!("Invalid runtime session list JSON: {}", error))?;
        if let Some(sid) = request.sid.as_mut() {
            *sid = normalize_runtime_session_sid(sid)?;
        }
        let payload = self
            .runtime_sessions
            .list(request.sid.as_deref())
            .unwrap_or_else(runtime_session_error_payload);
        serde_json::to_string(&payload)
            .map_err(|error| format!("Runtime session list JSON encode failed: {}", error))
    }

    /// Close one persistent runtime session and return its final status as JSON.
    /// 关闭一个持久运行时会话并以 JSON 返回其最终状态。
    pub fn close_runtime_session_json(&self, request_json: &str) -> Result<String, String> {
        let mut request: RuntimeSessionLeaseRequest = serde_json::from_str(request_json)
            .map_err(|error| format!("Invalid runtime session close JSON: {}", error))?;
        if let Some(sid) = request.sid.as_mut() {
            *sid = normalize_runtime_session_sid(sid)?;
        }
        let payload = self
            .runtime_sessions
            .close(
                &request.lease_id,
                request.sid.as_deref(),
                request.generation,
            )
            .unwrap_or_else(runtime_session_error_payload);
        serde_json::to_string(&payload)
            .map_err(|error| format!("Runtime session close JSON encode failed: {}", error))
    }

    /// Install the managed Lua `io` compatibility table in a persistent runtime VM.
    /// 在持久运行时 VM 中安装托管 Lua `io` 兼容表。
    fn install_managed_io_compat_for_runtime(
        lua: &Lua,
        host_options: &LuaRuntimeHostOptions,
    ) -> Result<(), String> {
        if !host_options.capabilities.enable_managed_io_compat {
            return Ok(());
        }
        let default_encoding = resolve_host_default_text_encoding(host_options)?;
        let vulcan = get_vulcan_table(lua)?;
        let vulcan_io = vulcan
            .get::<Table>("io")
            .map_err(|error| format!("Failed to get vulcan.io: {}", error))?;
        install_managed_io_compat(lua, &vulcan_io, default_encoding).map_err(|error| {
            format!(
                "Failed to install managed io compatibility for runtime session: {}",
                error
            )
        })
    }

    /// Ensure one locked runtime session can still execute.
    /// 确保一个已锁定运行时会话仍可执行。
    fn ensure_runtime_session_active(
        session: &mut RuntimeSession,
    ) -> Result<(), RuntimeSessionError> {
        if let Some(error) = session.inactive_error() {
            return Err(error);
        }
        session.refresh();
        Ok(())
    }

    /// Evaluate one request while holding the selected runtime session lock.
    /// 持有所选运行时会话锁时执行一个请求。
    fn eval_runtime_session_locked(
        &self,
        session: &mut RuntimeSession,
        request: &RuntimeSessionEvalRequest,
    ) -> Result<Value, String> {
        reset_pooled_vm_request_scope(&session.vm.lua, self.host_options.as_ref())?;
        Self::populate_vulcan_request_context(&session.vm.lua, None)?;
        populate_vulcan_internal_execution_context(
            &session.vm.lua,
            &VulcanInternalExecutionContext {
                tool_name: None,
                skill_name: None,
                entry_name: None,
                root_name: None,
                luaexec_active: true,
                luaexec_caller_tool_name: None,
            },
        )?;
        populate_vulcan_file_context(&session.vm.lua, None, None)?;
        populate_vulcan_dependency_context(
            &session.vm.lua,
            self.host_options.as_ref(),
            None,
            None,
        )?;
        Self::populate_vulcan_lancedb_context(&session.vm.lua, None, None)?;
        Self::populate_vulcan_sqlite_context(&session.vm.lua, None, None)?;

        let args_table = json_to_lua_table(&session.vm.lua, &request.args)?;
        session
            .vm
            .lua
            .globals()
            .set("__runlua_args", args_table)
            .map_err(|error| format!("Failed to set runtime session args: {}", error))?;
        let wrapper = format!(
            "return (function()\n  local args = __runlua_args\n  {}\nend)()",
            request.code
        );
        Self::install_runlua_timeout_guard(&session.vm.lua, request.timeout_ms)
            .map_err(|error| error.to_string())?;
        let eval_result = session.vm.lua.load(&wrapper).eval::<LuaValue>();
        Self::remove_runlua_timeout_guard(&session.vm.lua);
        let result = eval_result.map_err(|error| {
            let msg = format!("Runtime session eval error: {}", error);
            log_error(format!("[LuaSkill:error] {}", msg));
            msg
        })?;
        let json_result = lua_value_to_json(&result)?;
        clear_runlua_args_global(&session.vm.lua)?;
        Ok(json_result)
    }

    /// Acquire one isolated runlua VM from the dedicated pool.
    /// 从独立池中获取一个隔离 runlua 虚拟机。
    fn acquire_runlua_vm(
        runlua_pool: Arc<LuaVmPool>,
        skills: Arc<HashMap<String, LoadedSkill>>,
        entry_registry: Arc<BTreeMap<String, ResolvedEntryTarget>>,
        host_options: Arc<LuaRuntimeHostOptions>,
        skill_config_store: Arc<SkillConfigStore>,
        runtime_skill_roots: Vec<RuntimeSkillRoot>,
        lancedb_host: Option<Arc<LanceDbSkillHost>>,
        sqlite_host: Option<Arc<SqliteSkillHost>>,
    ) -> Result<LuaVmLease, String> {
        runlua_pool.acquire(move || {
            Self::create_runlua_vm(
                skills.as_ref(),
                entry_registry.as_ref(),
                host_options.clone(),
                skill_config_store.clone(),
                runtime_skill_roots.clone(),
                lancedb_host.clone(),
                sqlite_host.clone(),
            )
        })
    }

    /// Execute one isolated runlua request through the dedicated pooled runtime.
    /// 通过独立的池化运行时执行一次隔离 runlua 请求。
    fn execute_runlua_request_inline_with_runtime(
        request: &RunLuaExecRequest,
        runlua_pool: Arc<LuaVmPool>,
        skills: Arc<HashMap<String, LoadedSkill>>,
        entry_registry: Arc<BTreeMap<String, ResolvedEntryTarget>>,
        host_options: Arc<LuaRuntimeHostOptions>,
        skill_config_store: Arc<SkillConfigStore>,
        runtime_skill_roots: Vec<RuntimeSkillRoot>,
        lancedb_host: Option<Arc<LanceDbSkillHost>>,
        sqlite_host: Option<Arc<SqliteSkillHost>>,
    ) -> Result<String, String> {
        if request.timeout_ms == 0 {
            return Err("luaexec timeout_ms must be greater than 0".to_string());
        }
        let (resolved_code, entry_file) = Self::resolve_runlua_source(request)?;
        let mut lease = Self::acquire_runlua_vm(
            runlua_pool,
            skills,
            entry_registry,
            host_options.clone(),
            skill_config_store,
            runtime_skill_roots,
            lancedb_host,
            sqlite_host,
        )?;
        let scope_guard = LuaVmRequestScopeGuard::new(&mut lease, host_options.as_ref())?;
        let lua = scope_guard.lua();
        let simulated_request_context = build_luaexec_call_request_context();
        let simulated_invocation_context = LuaInvocationContext::new(
            Some(simulated_request_context),
            Value::Object(serde_json::Map::new()),
            Value::Object(serde_json::Map::new()),
        );
        Self::populate_vulcan_request_context(lua, Some(&simulated_invocation_context))?;
        populate_vulcan_internal_execution_context(
            lua,
            &VulcanInternalExecutionContext {
                tool_name: None,
                skill_name: None,
                entry_name: None,
                root_name: None,
                luaexec_active: true,
                luaexec_caller_tool_name: request.caller_tool_name.clone(),
            },
        )?;
        populate_vulcan_file_context(lua, None, entry_file.as_deref())?;
        Self::populate_vulcan_lancedb_context(lua, None, None)?;
        Self::populate_vulcan_sqlite_context(lua, None, None)?;

        let captured_output: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        Self::configure_runlua_execution_environment(
            lua,
            captured_output.clone(),
            host_options.as_ref(),
        )?;

        let args_table = json_to_lua_table(lua, &request.args)?;
        lua.globals()
            .set("__runlua_args", args_table)
            .map_err(|error| format!("Failed to set runlua args: {}", error))?;

        let wrapper = format!(
            "return (function()\n  local args = __runlua_args\n  return table.pack((function()\n{}\nend)())\nend)()",
            resolved_code
        );

        Self::install_runlua_timeout_guard(lua, request.timeout_ms)
            .map_err(|error| error.to_string())?;
        let execution_result = Self::execute_runlua_wrapper(lua, &wrapper, entry_file.as_deref());
        Self::remove_runlua_timeout_guard(lua);
        let printed_output = captured_output
            .lock()
            .map_err(|_| "Failed to lock runlua output capture".to_string())?
            .clone();

        let render_result = match execution_result {
            Ok(returned_values) => {
                let rendered_values = Self::collect_runlua_return_values(&returned_values)?;
                Ok(Self::render_runlua_success_markdown(
                    request,
                    &printed_output,
                    &rendered_values,
                ))
            }
            Err(error) => Ok(Self::render_runlua_error_markdown(
                request,
                &printed_output,
                error.to_string().as_str(),
            )),
        };
        let cleanup_result = scope_guard.finish();
        match (render_result, cleanup_result) {
            (Ok(rendered), Ok(())) => Ok(rendered),
            (Ok(_), Err(cleanup_error)) => Err(cleanup_error),
            (Err(render_error), Ok(())) => Err(render_error),
            (Err(render_error), Err(cleanup_error)) => Err(format!(
                "{}; pooled runlua VM cleanup failed: {}",
                render_error, cleanup_error
            )),
        }
    }

    /// Execute one isolated runlua request using the current engine snapshots.
    /// 使用当前引擎快照执行一次隔离 runlua 请求。
    fn execute_runlua_request_inline(&self, request: &RunLuaExecRequest) -> Result<String, String> {
        Self::execute_runlua_request_inline_with_runtime(
            request,
            self.runlua_pool.clone(),
            Arc::new(self.skills.clone()),
            Arc::new(self.entry_registry.clone()),
            self.host_options.clone(),
            self.skill_config_store.clone(),
            self.runtime_skill_roots.clone(),
            self.lancedb_host.clone(),
            self.sqlite_host.clone(),
        )
    }

    /// Resolve one runlua request into concrete source text and optional entry file context.
    /// 将一次 runlua 请求解析成具体源代码文本及可选入口文件上下文。
    fn resolve_runlua_source(
        request: &RunLuaExecRequest,
    ) -> Result<(String, Option<PathBuf>), String> {
        let inline_code = request
            .code
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string());
        let file_path = request
            .file
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string());

        match (inline_code, file_path) {
            (Some(_), Some(_)) => {
                Err("luaexec accepts either code or file, but not both".to_string())
            }
            (None, None) => Err("luaexec requires code or file".to_string()),
            (Some(code), None) => Ok((code, None)),
            (None, Some(file_text)) => {
                validate_path_text(&file_text, "luaexec", "file")
                    .map_err(|error| error.to_string())?;
                let raw_file_path = PathBuf::from(&file_text);
                let file_path = if raw_file_path.is_absolute() {
                    raw_file_path
                } else {
                    std::env::current_dir()
                        .map_err(|error| {
                            format!("Failed to resolve luaexec relative file path: {}", error)
                        })?
                        .join(raw_file_path)
                };
                let source = std::fs::read_to_string(&file_path).map_err(|error| {
                    format!(
                        "Failed to read luaexec file {}: {}: {}",
                        file_path.display(),
                        error,
                        error
                    )
                })?;
                Ok((source, Some(file_path)))
            }
        }
    }

    /// Execute one inline runlua request from raw JSON text.
    /// 从原始 JSON 文本执行一次进程内 runlua 请求。
    pub fn execute_runlua_request_json_inline(&self, request_json: &str) -> Result<String, String> {
        let request: RunLuaExecRequest = serde_json::from_str(request_json)
            .map_err(|error| format!("Invalid luaexec request JSON: {}", error))?;
        self.execute_runlua_request_inline(&request)
    }

    /// Execute the runlua wrapper, optionally switching the process current directory to the entry file directory.
    /// 执行 runlua 包装器，并在需要时临时切换进程工作目录到入口文件目录。
    fn execute_runlua_wrapper(
        lua: &Lua,
        wrapper: &str,
        entry_file: Option<&Path>,
    ) -> Result<Table, mlua::Error> {
        match entry_file.and_then(Path::parent) {
            Some(entry_dir) => {
                let _cwd_guard = runlua_cwd_guard()
                    .lock()
                    .map_err(|_| mlua::Error::runtime("luaexec cwd guard lock poisoned"))?;
                let original_dir = std::env::current_dir()
                    .map_err(|error| mlua::Error::runtime(format!("luaexec cwd: {}", error)))?;
                std::env::set_current_dir(entry_dir)
                    .map_err(|error| mlua::Error::runtime(format!("luaexec set cwd: {}", error)))?;
                let execution = lua.load(wrapper).eval::<Table>();
                let restore_result = std::env::set_current_dir(&original_dir).map_err(|error| {
                    mlua::Error::runtime(format!("luaexec restore cwd: {}", error))
                });
                match (execution, restore_result) {
                    (Ok(table), Ok(())) => Ok(table),
                    (Err(error), Ok(())) => Err(error),
                    (_, Err(error)) => Err(error),
                }
            }
            None => lua.load(wrapper).eval::<Table>(),
        }
    }

    /// Configure the isolated runlua execution VM.
    /// 配置隔离 runlua 执行虚拟机的运行时环境。
    fn configure_runlua_execution_environment(
        lua: &Lua,
        captured_output: Arc<Mutex<Vec<String>>>,
        host_options: &LuaRuntimeHostOptions,
    ) -> Result<(), String> {
        let runtime = get_vulcan_runtime_table(lua)?;
        let runtime_lua = get_vulcan_runtime_lua_table(lua)?;
        let vulcan = get_vulcan_table(lua)?;
        let cache = vulcan
            .get::<Table>("cache")
            .map_err(|error| format!("Failed to get vulcan.cache: {}", error))?;
        let vulcan_io = vulcan
            .get::<Table>("io")
            .map_err(|error| format!("Failed to get vulcan.io: {}", error))?;

        let print_capture = captured_output.clone();
        let print_fn = lua
            .create_function(move |_, args: MultiValue| {
                let mut parts = Vec::new();
                for value in args.into_iter() {
                    parts.push(LuaEngine::render_lua_value_inline(&value));
                }
                let mut guard = print_capture
                    .lock()
                    .map_err(|_| mlua::Error::runtime("runlua print capture lock poisoned"))?;
                guard.push(parts.join("\t"));
                Ok(())
            })
            .map_err(|error| format!("Failed to create runlua print capture: {}", error))?;
        lua.globals()
            .set("print", print_fn)
            .map_err(|error| format!("Failed to override global print for runlua: {}", error))?;

        lua.load(
            r#"
if jit and type(jit.off) == "function" then
    jit.off(true, true)
end
if jit and type(jit.flush) == "function" then
    jit.flush()
end
"#,
        )
        .exec()
        .map_err(|error| format!("Failed to disable JIT for runlua: {}", error))?;

        runtime
            .set("log", LuaValue::Nil)
            .map_err(|error| format!("Failed to clear vulcan.runtime.log for runlua: {}", error))?;
        cache
            .set("put", LuaValue::Nil)
            .map_err(|error| format!("Failed to clear vulcan.cache.put for runlua: {}", error))?;
        cache
            .set("get", LuaValue::Nil)
            .map_err(|error| format!("Failed to clear vulcan.cache.get for runlua: {}", error))?;
        cache.set("delete", LuaValue::Nil).map_err(|error| {
            format!("Failed to clear vulcan.cache.delete for runlua: {}", error)
        })?;
        runtime_lua.set("exec", LuaValue::Nil).map_err(|error| {
            format!(
                "Failed to clear vulcan.runtime.lua.exec for runlua: {}",
                error
            )
        })?;
        if host_options.capabilities.enable_managed_io_compat {
            let default_encoding = resolve_host_default_text_encoding(host_options)?;
            install_managed_io_compat(lua, &vulcan_io, default_encoding).map_err(|error| {
                format!(
                    "Failed to install managed io compatibility for runlua: {}",
                    error
                )
            })?;
        }
        Ok(())
    }

    /// Install a hard timeout guard for the isolated luaexec VM.
    /// 为隔离 luaexec 虚拟机安装硬超时保护。
    fn install_runlua_timeout_guard(lua: &Lua, timeout_ms: u64) -> mlua::Result<()> {
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        let timeout_text = format!("luaexec execution timed out after {} ms", timeout_ms);

        lua.set_hook(
            HookTriggers::new().every_nth_instruction(1_000),
            move |_, _| {
                if Instant::now() >= deadline {
                    return Err(mlua::Error::runtime(timeout_text.clone()));
                }
                Ok(VmState::Continue)
            },
        )
    }

    /// Remove the previously installed timeout guard from the isolated luaexec VM.
    /// 移除隔离 luaexec 虚拟机上已安装的超时保护。
    fn remove_runlua_timeout_guard(lua: &Lua) {
        lua.remove_hook();
    }

    /// Collect packed Lua return values from the isolated runlua wrapper.
    /// 从隔离 runlua 包装器返回的打包结果中提取所有返回值。
    fn collect_runlua_return_values(
        result_table: &Table,
    ) -> Result<Vec<RunLuaRenderedValue>, String> {
        let value_count = result_table
            .get::<i64>("n")
            .map_err(|error| format!("Failed to read runlua return count: {}", error))?
            .max(0) as usize;

        let mut rendered_values = Vec::new();
        if value_count == 0 {
            rendered_values.push(RunLuaRenderedValue {
                format: "json",
                content: "null".to_string(),
            });
            return Ok(rendered_values);
        }

        for index in 1..=value_count {
            let value: LuaValue = result_table.raw_get(index).map_err(|error| {
                format!("Failed to read runlua return value {}: {}", index, error)
            })?;
            rendered_values.push(Self::render_runlua_value(&value));
        }

        Ok(rendered_values)
    }

    /// Render one Lua return value into a Markdown-ready block payload.
    /// 将单个 Lua 返回值渲染为可直接写入 Markdown 代码块的载荷。
    fn render_runlua_value(value: &LuaValue) -> RunLuaRenderedValue {
        match value {
            LuaValue::String(text) => RunLuaRenderedValue {
                format: "text",
                content: text
                    .to_str()
                    .map(|value| value.to_string())
                    .unwrap_or_default(),
            },
            _ => match lua_value_to_json(value) {
                Ok(json_value) => RunLuaRenderedValue {
                    format: "json",
                    content: serde_json::to_string_pretty(&json_value)
                        .unwrap_or_else(|_| "null".to_string()),
                },
                Err(_) => RunLuaRenderedValue {
                    format: "text",
                    content: Self::render_lua_value_inline(value),
                },
            },
        }
    }

    /// Render one Lua value into a compact single-line textual form.
    /// 将单个 Lua 值渲染为紧凑的单行文本形式。
    fn render_lua_value_inline(value: &LuaValue) -> String {
        match value {
            LuaValue::String(text) => text
                .to_str()
                .map(|value| value.to_string())
                .unwrap_or_default(),
            LuaValue::Integer(number) => number.to_string(),
            LuaValue::Number(number) => number.to_string(),
            LuaValue::Boolean(flag) => flag.to_string(),
            LuaValue::Nil => "nil".to_string(),
            _ => format!("{:?}", value),
        }
    }

    /// Render a successful runlua execution result into Markdown text.
    /// 将成功的 runlua 执行结果渲染为 Markdown 文本。
    fn render_runlua_success_markdown(
        request: &RunLuaExecRequest,
        printed_output: &[String],
        rendered_values: &[RunLuaRenderedValue],
    ) -> String {
        let mut lines = vec![
            "# Runtime Execution Result".to_string(),
            "".to_string(),
            "## Task".to_string(),
            if request.task.trim().is_empty() {
                "Execute Lua runtime code".to_string()
            } else {
                request.task.trim().to_string()
            },
            "".to_string(),
            "## Status".to_string(),
            "SUCCESS".to_string(),
        ];

        if !printed_output.is_empty() {
            lines.extend([
                "".to_string(),
                "## Printed Output".to_string(),
                "```text".to_string(),
                printed_output.join("\n"),
                "```".to_string(),
            ]);
        }

        lines.extend(["".to_string(), "## Returned Values".to_string()]);

        for (index, value) in rendered_values.iter().enumerate() {
            lines.push(format!("{}. ", index + 1));
            lines.push(format!("```{}", value.format));
            lines.push(value.content.clone());
            lines.push("```".to_string());
            if index + 1 < rendered_values.len() {
                lines.push("".to_string());
            }
        }

        lines.join("\n")
    }

    /// Render a failed runlua execution result into Markdown text.
    /// 将失败的 runlua 执行结果渲染为 Markdown 文本。
    fn render_runlua_error_markdown(
        request: &RunLuaExecRequest,
        printed_output: &[String],
        error_text: &str,
    ) -> String {
        let mut lines = vec![
            "# Runtime Execution Error".to_string(),
            "".to_string(),
            "## Task".to_string(),
            if request.task.trim().is_empty() {
                "Execute Lua runtime code".to_string()
            } else {
                request.task.trim().to_string()
            },
            "".to_string(),
            "## Status".to_string(),
            "FAILED".to_string(),
            "".to_string(),
            "## Error".to_string(),
            "```text".to_string(),
            error_text.to_string(),
            "```".to_string(),
        ];

        if !printed_output.is_empty() {
            lines.extend([
                "".to_string(),
                "## Printed Output".to_string(),
                "```text".to_string(),
                printed_output.join("\n"),
                "```".to_string(),
            ]);
        }

        lines.join("\n")
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

        // Build package.cpath entries for C modules (.dll on Windows)
        // 统一使用宿主提供的 lua_packages/lib/lua 目录，不再自行推导可执行文件相对路径。
        #[cfg(windows)]
        let cpath_pattern = format!(
            "{}\\lib\\lua\\?.dll;{}\\lib\\lua\\?\\init.dll;{}\\lib\\lua\\loadall.dll;{}\\?\\?.dll;",
            lua_packages.display(),
            lua_packages.display(),
            lua_packages.display(),
            lua_packages.display()
        );

        // Build package.cpath entries for C modules (.so on Linux)
        // Linux 下同样严格依赖宿主传入的 lua_packages 根目录。
        #[cfg(target_os = "linux")]
        let cpath_pattern = format!(
            "{}/lib/lua/?.so;{}/lib/lua/?/init.so;{}/lib/lua/loadall.so;{}/?.so;",
            lua_packages.display(),
            lua_packages.display(),
            lua_packages.display(),
            lua_packages.display()
        );

        // Build package.cpath entries for C modules (.dylib on macOS)
        // macOS 下同样严格依赖宿主传入的 lua_packages 根目录。
        #[cfg(target_os = "macos")]
        let cpath_pattern = format!(
            "{}/lib/lua/?.dylib;{}/lib/lua/?/init.dylib;{}/lib/lua/loadall.dylib;{}/?.dylib;",
            lua_packages.display(),
            lua_packages.display(),
            lua_packages.display(),
            lua_packages.display()
        );

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
        let exec_fn = lua.create_function(move |lua, spec: LuaValue| {
            let request = parse_exec_request(spec, "process.exec", exec_default_encoding)?;
            let result = execute_exec_request(request);
            exec_result_to_lua_table(lua, result)
        })?;
        process.set("exec", exec_fn)?;
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
mod tests {
    use super::{
        LoadedSkill, LuaEngine, LuaVmPool, LuaVmPoolConfig, LuaVmPoolState, LuaVmRequestScopeGuard,
        RuntimeSessionManager, SkillConfigStore, VulcanInternalExecutionContext,
        default_runlua_vm_pool_config, get_vulcan_context_table, get_vulcan_deps_table,
        get_vulcan_runtime_internal_table, get_vulcan_table, json_to_lua_table,
        normalize_host_visible_path_text, populate_vulcan_dependency_context,
        populate_vulcan_file_context, populate_vulcan_internal_execution_context, runlua_cwd_guard,
    };
    use crate::host::callbacks::runtime_model_callback_test_guard;
    use crate::host::database::RuntimeDatabaseProviderCallbacks;
    use crate::lua_skill::SkillMeta;
    use crate::runtime::encoding::{RuntimeTextEncoding, encode_runtime_text};
    use crate::runtime_options::LuaRuntimeRunLuaPoolConfig;
    use crate::{
        LuaEngineOptions, LuaRuntimeHostOptions, RuntimeClientInfo, RuntimeHostToolAction,
        RuntimeModelEmbedRequest, RuntimeModelEmbedResponse, RuntimeModelError,
        RuntimeModelErrorCode, RuntimeModelLlmRequest, RuntimeModelLlmResponse, RuntimeModelUsage,
        RuntimeRequestContext, RuntimeSkillRoot, SkillInstallRequest, SkillInstallSourceType,
        SkillManagementAuthority, SkillUninstallOptions, set_host_tool_callback,
        set_model_embed_callback, set_model_llm_callback,
    };
    use mlua::{Table, Value as LuaValue};
    use serde_json::{Value, json};
    use std::collections::HashMap;
    use std::fs;
    use std::path::Path;
    use std::path::PathBuf;
    use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};

    /// Guard one process-wide host-tool callback test and clear global callback state on drop.
    /// 保护单个进程级宿主工具回调测试，并在释放时清理全局回调状态。
    struct HostToolCallbackTestGuard {
        /// Hold the process-wide mutex guard until the current test finishes.
        /// 持有进程级互斥锁直到当前测试结束。
        _guard: MutexGuard<'static, ()>,
    }

    impl Drop for HostToolCallbackTestGuard {
        /// Clear the global host-tool callback when one guarded test finishes.
        /// 当受保护测试结束时清理全局宿主工具回调。
        fn drop(&mut self) {
            set_host_tool_callback(None);
        }
    }

    /// Acquire the process-wide host-tool callback test guard.
    /// 获取进程级宿主工具回调测试保护锁。
    fn host_tool_callback_test_guard() -> HostToolCallbackTestGuard {
        static GUARD: OnceLock<Mutex<()>> = OnceLock::new();
        let guard = GUARD
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("lock host tool callback test guard");
        set_host_tool_callback(None);
        HostToolCallbackTestGuard { _guard: guard }
    }

    /// Build one minimal loaded skill for collision-index tests.
    /// 为冲突编号测试构造一个最小已加载 skill。
    fn make_loaded_skill(
        directory_name: &str,
        skill_id: &str,
        local_entry_name: &str,
        lua_module: &str,
    ) -> LoadedSkill {
        let mut meta: SkillMeta = serde_yaml::from_str(&format!("name: {skill_id}\nversion: 0.1.0\nenable: true\ndebug: false\nentries:\n  - name: {local_entry_name}\n    lua_entry: runtime/test.lua\n    lua_module: {lua_module}\n"))
            .expect("deserialize minimal skill meta");
        meta.bind_directory_skill_id(skill_id.to_string());
        LoadedSkill {
            meta,
            dir: PathBuf::from(format!("D:/tests/{directory_name}")),
            root_name: "ROOT".to_string(),
            lancedb_binding: None,
            sqlite_binding: None,
            resolved_entry_names: HashMap::new(),
        }
    }

    /// Verify host-visible path normalization strips the Windows drive-letter verbatim prefix.
    /// 验证对宿主可见的路径归一化会去掉 Windows 盘符 verbatim 前缀。
    #[cfg(windows)]
    #[test]
    fn normalize_host_visible_path_text_strips_windows_drive_verbatim_prefix() {
        assert_eq!(
            normalize_host_visible_path_text(r"\\?\C:\runtime-test-root\skill.lua"),
            r"C:\runtime-test-root\skill.lua"
        );
    }

    /// Verify host-visible path normalization strips the Windows UNC verbatim prefix.
    /// 验证对宿主可见的路径归一化会去掉 Windows UNC verbatim 前缀。
    #[cfg(windows)]
    #[test]
    fn normalize_host_visible_path_text_strips_windows_unc_verbatim_prefix() {
        assert_eq!(
            normalize_host_visible_path_text(r"\\?\UNC\server\share\skill.lua"),
            r"\\server\share\skill.lua"
        );
    }

    /// Build one minimal engine instance used only for registry tests.
    /// 构造仅用于入口注册表测试的最小引擎实例。
    fn make_test_engine(skills: HashMap<String, LoadedSkill>) -> LuaEngine {
        LuaEngine {
            skills,
            entry_registry: Default::default(),
            runtime_skill_roots: Vec::new(),
            pool: Arc::new(LuaVmPool {
                config: LuaVmPoolConfig {
                    min_size: 1,
                    max_size: 1,
                    idle_ttl_secs: 60,
                },
                state: Mutex::new(LuaVmPoolState {
                    available: Vec::new(),
                    total_count: 0,
                }),
                condvar: Condvar::new(),
            }),
            runlua_pool: Arc::new(LuaVmPool::new(default_runlua_vm_pool_config())),
            runtime_sessions: Arc::new(RuntimeSessionManager::new()),
            skill_config_store: Arc::new(
                SkillConfigStore::new(None).expect("create runtime test skill config store"),
            ),
            lancedb_host: None,
            sqlite_host: None,
            database_provider_callbacks: Arc::new(RuntimeDatabaseProviderCallbacks::default()),
            host_options: Arc::new(LuaRuntimeHostOptions::default()),
        }
    }

    /// Build one minimal runtime engine that can execute pooled-VM isolation tests.
    /// 构造一个可用于池化虚拟机隔离测试的最小运行时引擎。
    fn make_runtime_test_engine() -> LuaEngine {
        make_runtime_test_engine_with_host_options(LuaRuntimeHostOptions::default())
    }

    /// Build one minimal runtime engine with explicit host options.
    /// 使用显式宿主选项构造一个最小运行时引擎。
    fn make_runtime_test_engine_with_host_options(
        host_options: LuaRuntimeHostOptions,
    ) -> LuaEngine {
        LuaEngine::new(LuaEngineOptions {
            host_options,
            pool_config: LuaVmPoolConfig {
                min_size: 1,
                max_size: 1,
                idle_ttl_secs: 60,
            },
        })
        .expect("create runtime test engine")
    }

    /// Build one temporary runtime root path for one isolated skill-config test case.
    /// 为单个隔离技能配置测试用例构造一条临时运行时根目录路径。
    fn make_temp_runtime_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "luaskills_{}_{}_{}",
            label,
            std::process::id(),
            label.len()
        ))
    }

    /// Create one minimal runtime directory layout used by skill-config tests.
    /// 创建技能配置测试使用的最小运行时目录结构。
    fn create_runtime_test_layout(runtime_root: &Path) {
        for relative_path in [
            "skills",
            "temp",
            "resources",
            "lua_packages",
            "bin/tools",
            "libs",
        ] {
            fs::create_dir_all(runtime_root.join(relative_path))
                .expect("create runtime test layout path");
        }
    }

    /// Write one minimal packaged-runtime luaskills-packages metadata tree for runtime validation tests.
    /// 为运行时校验测试写入一个最小打包运行时 luaskills-packages 元数据目录树。
    fn write_runtime_packages_test_metadata(runtime_root: &Path) {
        let resources_dir = runtime_root.join("resources");
        let packages_root = resources_dir.join("luaskills-packages");
        let help_packages_dir = packages_root.join("help").join("packages");
        let help_modules_dir = packages_root.join("help").join("modules");
        let packages_licenses_dir = runtime_root.join("licenses").join("luaskills-packages");
        fs::create_dir_all(&help_packages_dir).expect("create package help test dir");
        fs::create_dir_all(&help_modules_dir).expect("create module help test dir");
        fs::create_dir_all(&packages_licenses_dir).expect("create package license test dir");

        fs::write(
            resources_dir.join("lua-runtime-manifest.json"),
            "{\n  \"schema_version\": 1,\n  \"layout\": \"luaskills-runtime-v1\"\n}\n",
        )
        .expect("write runtime manifest test file");
        fs::write(
            packages_root.join("lua_packages.txt"),
            "pkg demo-package 0.1.0\n",
        )
        .expect("write package compatibility file");
        fs::write(
            packages_root.join("install-manifest.json"),
            "{\n  \"schema_version\": 1,\n  \"packages\": []\n}\n",
        )
        .expect("write package install manifest");
        fs::write(
            packages_root.join("platform-support.json"),
            "{\n  \"schema_version\": 1,\n  \"supported_targets\": [\"windows-x64\", \"linux-x64\", \"linux-arm64\", \"macos-x64\", \"macos-arm64\"]\n}\n",
        )
        .expect("write package platform support");
        fs::write(
            packages_root.join("THIRD_PARTY_LICENSES.json"),
            "{\n  \"schema_version\": 1,\n  \"luarocks_packages\": []\n}\n",
        )
        .expect("write package third-party licenses");
        fs::write(
            packages_root.join("THIRD_PARTY_NOTICES.md"),
            "# Third-Party Notices\n",
        )
        .expect("write package third-party notices");
        fs::write(
            packages_root.join("help").join("index.json"),
            "{\n  \"schema_version\": 1,\n  \"packages\": [],\n  \"modules\": []\n}\n",
        )
        .expect("write package help index");
        fs::write(
            help_packages_dir.join("demo-package.json"),
            "{\n  \"schema_version\": 1,\n  \"package_name\": \"demo-package\"\n}\n",
        )
        .expect("write package help document");
        fs::write(
            packages_licenses_dir.join("index.json"),
            "{\n  \"schema_version\": 1,\n  \"luarocks_packages\": []\n}\n",
        )
        .expect("write package license index");
        fs::write(
            resources_dir.join("luaskills-packages-manifest.json"),
            "{\n  \"schema_version\": 1,\n  \"layout\": \"luaskills-packages-runtime-v1\",\n  \"paths\": {\n    \"install_manifest\": \"resources/luaskills-packages/install-manifest.json\",\n    \"compat_lua_packages_txt\": \"resources/luaskills-packages/lua_packages.txt\",\n    \"platform_support\": \"resources/luaskills-packages/platform-support.json\",\n    \"third_party_licenses\": \"resources/luaskills-packages/THIRD_PARTY_LICENSES.json\",\n    \"third_party_notices\": \"resources/luaskills-packages/THIRD_PARTY_NOTICES.md\",\n    \"help_index\": \"resources/luaskills-packages/help/index.json\",\n    \"package_help_root\": \"resources/luaskills-packages/help/packages\",\n    \"module_help_root\": \"resources/luaskills-packages/help/modules\",\n    \"license_index\": \"licenses/luaskills-packages/index.json\"\n  }\n}\n",
        )
        .expect("write runtime packages manifest");
    }

    /// Write one minimal skill fixture that reads one value from `vulcan.config`.
    /// 写入一个最小技能夹具，用于从 `vulcan.config` 读取单个值。
    fn write_skill_config_test_skill(runtime_root: &Path, skill_id: &str) -> PathBuf {
        let skill_dir = runtime_root.join("skills").join(skill_id);
        fs::create_dir_all(skill_dir.join("runtime")).expect("create config test runtime dir");
        fs::write(
            skill_dir.join("skill.yaml"),
            format!(
                "name: {skill_id}\nversion: 0.1.0\nenable: true\ndebug: false\nentries:\n  - name: ping\n    description: Config ping entry.\n    lua_entry: runtime/ping.lua\n    lua_module: {skill_id}.ping\n"
            ),
        )
        .expect("write config test skill yaml");
        fs::write(
            skill_dir.join("runtime").join("ping.lua"),
            "return function(args)\n  local value = vulcan.config.get(\"api_token\")\n  if value == nil then\n    return \"missing\"\n  end\n  return value\nend\n",
        )
        .expect("write config test runtime entry");
        skill_dir
    }

    /// Write one minimal enabled skill fixture into a specific skills root.
    /// 将一个最小启用技能夹具写入指定 skills 根目录。
    fn write_minimal_skill_to_root(skill_root: &Path, skill_id: &str) -> PathBuf {
        write_minimal_skill_to_root_with_response(skill_root, skill_id, "ok")
    }

    /// Write one minimal enabled skill fixture with a deterministic response into a specific skills root.
    /// 将带有确定响应的最小启用技能夹具写入指定 skills 根目录。
    fn write_minimal_skill_to_root_with_response(
        skill_root: &Path,
        skill_id: &str,
        response: &str,
    ) -> PathBuf {
        let skill_dir = skill_root.join(skill_id);
        fs::create_dir_all(skill_dir.join("runtime")).expect("create minimal skill runtime dir");
        fs::write(
            skill_dir.join("skill.yaml"),
            format!(
                "name: {skill_id}\nversion: 0.1.0\nenable: true\ndebug: false\nentries:\n  - name: ping\n    description: Minimal ping entry.\n    lua_entry: runtime/ping.lua\n    lua_module: {skill_id}.ping\n"
            ),
        )
        .expect("write minimal skill yaml");
        fs::write(
            skill_dir.join("runtime").join("ping.lua"),
            format!("return function(args)\n  return '{response}'\nend\n"),
        )
        .expect("write minimal skill runtime entry");
        skill_dir
    }

    /// Write one model-capability test skill with caller-provided Lua source.
    /// 写入一个使用调用方提供 Lua 源码的模型能力测试 skill。
    fn write_model_test_skill_to_root(
        skill_root: &Path,
        skill_id: &str,
        lua_source: &str,
    ) -> PathBuf {
        let skill_dir = skill_root.join(skill_id);
        fs::create_dir_all(skill_dir.join("runtime")).expect("create model test skill runtime dir");
        fs::write(
            skill_dir.join("skill.yaml"),
            format!(
                "name: {skill_id}\nversion: 0.1.0\nenable: true\ndebug: false\nentries:\n  - name: ping\n    description: Model test entry.\n    lua_entry: runtime/ping.lua\n    lua_module: {skill_id}.ping\n"
            ),
        )
        .expect("write model test skill yaml");
        fs::write(skill_dir.join("runtime").join("ping.lua"), lua_source)
            .expect("write model test runtime entry");
        skill_dir
    }

    /// Verify ROOT keeps priority over PROJECT and USER for identical skill ids.
    /// 验证 ROOT 对同名 skill 始终高于 PROJECT 与 USER。
    #[test]
    fn load_from_roots_keeps_root_priority_over_project_and_user() {
        let runtime_root = make_temp_runtime_root("formal-root-load-priority");
        if runtime_root.exists() {
            let _ = fs::remove_dir_all(&runtime_root);
        }
        let root_root = RuntimeSkillRoot {
            name: "ROOT".to_string(),
            skills_dir: runtime_root.join("root_skills"),
        };
        let project_root = RuntimeSkillRoot {
            name: "PROJECT".to_string(),
            skills_dir: runtime_root.join("project_skills"),
        };
        let user_root = RuntimeSkillRoot {
            name: "USER".to_string(),
            skills_dir: runtime_root.join("user_skills"),
        };
        write_minimal_skill_to_root_with_response(&root_root.skills_dir, "vulcan-codekit", "root");
        write_minimal_skill_to_root_with_response(
            &project_root.skills_dir,
            "vulcan-codekit",
            "project",
        );
        write_minimal_skill_to_root_with_response(&user_root.skills_dir, "vulcan-codekit", "user");
        let mut engine = make_runtime_test_engine();
        engine
            .load_from_roots(&[root_root, project_root, user_root])
            .expect("formal root chain should load");

        let result = engine
            .call_skill("vulcan-codekit-ping", &json!({}), None)
            .expect("call root-priority skill");
        assert_eq!(result.content, "root");

        let _ = fs::remove_dir_all(&runtime_root);
    }

    /// Verify one packaged runtime loads successfully when the embedded luaskills-packages metadata tree is complete.
    /// 验证在内嵌 luaskills-packages 元数据目录树完整时，一个打包运行时能够成功加载。
    #[test]
    fn load_from_roots_accepts_packaged_runtime_with_packages_metadata() {
        let runtime_root = make_temp_runtime_root("packaged-runtime-packages-ok");
        if runtime_root.exists() {
            let _ = fs::remove_dir_all(&runtime_root);
        }
        create_runtime_test_layout(&runtime_root);
        write_runtime_packages_test_metadata(&runtime_root);
        write_minimal_skill_to_root(&runtime_root.join("skills"), "demo-packaged-skill");

        let mut host_options = LuaRuntimeHostOptions::default();
        host_options.resources_dir = Some(runtime_root.join("resources"));
        host_options.lua_packages_dir = Some(runtime_root.join("lua_packages"));
        host_options.host_provided_lua_root = Some(runtime_root.join("lua_packages"));
        let mut engine = make_runtime_test_engine_with_host_options(host_options);
        engine
            .load_from_roots(&[RuntimeSkillRoot {
                name: "ROOT".to_string(),
                skills_dir: runtime_root.join("skills"),
            }])
            .expect("packaged runtime with package metadata should load");

        let _ = fs::remove_dir_all(&runtime_root);
    }

    /// Verify one packaged runtime fails with a clear error when the top-level luaskills-packages manifest is missing.
    /// 验证当顶层 luaskills-packages 清单缺失时，一个打包运行时会给出清晰错误并加载失败。
    #[test]
    fn load_from_roots_rejects_packaged_runtime_without_packages_manifest() {
        let runtime_root = make_temp_runtime_root("packaged-runtime-missing-manifest");
        if runtime_root.exists() {
            let _ = fs::remove_dir_all(&runtime_root);
        }
        create_runtime_test_layout(&runtime_root);
        fs::write(
            runtime_root
                .join("resources")
                .join("lua-runtime-manifest.json"),
            "{\n  \"schema_version\": 1,\n  \"layout\": \"luaskills-runtime-v1\"\n}\n",
        )
        .expect("write runtime manifest trigger file");
        write_minimal_skill_to_root(&runtime_root.join("skills"), "demo-missing-manifest");

        let mut host_options = LuaRuntimeHostOptions::default();
        host_options.resources_dir = Some(runtime_root.join("resources"));
        host_options.lua_packages_dir = Some(runtime_root.join("lua_packages"));
        host_options.host_provided_lua_root = Some(runtime_root.join("lua_packages"));
        let mut engine = make_runtime_test_engine_with_host_options(host_options);
        let error_text = engine
            .load_from_roots(&[RuntimeSkillRoot {
                name: "ROOT".to_string(),
                skills_dir: runtime_root.join("skills"),
            }])
            .expect_err("packaged runtime without package manifest should fail")
            .to_string();
        assert!(
            error_text.contains("luaskills-packages-manifest.json"),
            "unexpected error text: {}",
            error_text
        );

        let _ = fs::remove_dir_all(&runtime_root);
    }

    /// Verify one packaged runtime fails with a clear error when one manifest-declared packages file is missing.
    /// 验证当清单声明的某个 packages 文件缺失时，一个打包运行时会给出清晰错误并加载失败。
    #[test]
    fn load_from_roots_rejects_packaged_runtime_when_declared_packages_file_is_missing() {
        let runtime_root = make_temp_runtime_root("packaged-runtime-missing-help-index");
        if runtime_root.exists() {
            let _ = fs::remove_dir_all(&runtime_root);
        }
        create_runtime_test_layout(&runtime_root);
        write_runtime_packages_test_metadata(&runtime_root);
        fs::remove_file(
            runtime_root
                .join("resources")
                .join("luaskills-packages")
                .join("help")
                .join("index.json"),
        )
        .expect("remove package help index");
        write_minimal_skill_to_root(&runtime_root.join("skills"), "demo-missing-help-index");

        let mut host_options = LuaRuntimeHostOptions::default();
        host_options.resources_dir = Some(runtime_root.join("resources"));
        host_options.lua_packages_dir = Some(runtime_root.join("lua_packages"));
        host_options.host_provided_lua_root = Some(runtime_root.join("lua_packages"));
        let mut engine = make_runtime_test_engine_with_host_options(host_options);
        let error_text = engine
            .load_from_roots(&[RuntimeSkillRoot {
                name: "ROOT".to_string(),
                skills_dir: runtime_root.join("skills"),
            }])
            .expect_err("packaged runtime with missing declared file should fail")
            .to_string();
        assert!(
            error_text.contains("luaskills-packages\\help\\index.json")
                || error_text.contains("luaskills-packages/help/index.json"),
            "unexpected error text: {}",
            error_text
        );

        let _ = fs::remove_dir_all(&runtime_root);
    }

    /// Verify delegated query helpers hide ROOT-owned metadata while runtime calls still use active skills.
    /// 验证委托查询辅助函数会隐藏 ROOT 元数据，同时运行时调用仍使用已激活技能。
    #[test]
    fn delegated_authority_query_helpers_hide_root_skills() {
        let runtime_root = make_temp_runtime_root("delegated-query-hides-root");
        if runtime_root.exists() {
            let _ = fs::remove_dir_all(&runtime_root);
        }
        let root_root = RuntimeSkillRoot {
            name: " root ".to_string(),
            skills_dir: runtime_root.join("root_skills"),
        };
        let user_root = RuntimeSkillRoot {
            name: "USER".to_string(),
            skills_dir: runtime_root.join("user_skills"),
        };
        write_minimal_skill_to_root(&root_root.skills_dir, "vulcan-root-skill");
        write_minimal_skill_to_root(&user_root.skills_dir, "vulcan-user-skill");
        let mut engine = make_runtime_test_engine();
        engine
            .load_from_roots(&[root_root, user_root])
            .expect("root and user runtime should load");

        let system_entries = engine.list_entries_for_authority(SkillManagementAuthority::System);
        let delegated_entries =
            engine.list_entries_for_authority(SkillManagementAuthority::DelegatedTool);
        assert!(
            system_entries
                .iter()
                .any(|entry| entry.root_name == " root ")
        );
        assert!(
            delegated_entries
                .iter()
                .all(|entry| entry.root_name.trim().to_ascii_uppercase() != "ROOT")
        );

        let system_help = engine.list_skill_help_for_authority(SkillManagementAuthority::System);
        let delegated_help =
            engine.list_skill_help_for_authority(SkillManagementAuthority::DelegatedTool);
        assert!(system_help.iter().any(|help| help.root_name == " root "));
        assert!(
            delegated_help
                .iter()
                .all(|help| help.root_name.trim().to_ascii_uppercase() != "ROOT")
        );

        let delegated_detail = engine
            .render_skill_help_detail_for_authority(
                SkillManagementAuthority::DelegatedTool,
                "vulcan-root-skill",
                "main",
                None,
            )
            .expect("delegated detail should be filtered");
        assert!(delegated_detail.is_none());

        let root_call = engine
            .call_skill("vulcan-root-skill-ping", &json!({}), None)
            .expect("runtime call should reach any active skill");
        assert_eq!(root_call.content, "ok");

        let root_run_lua = engine
            .run_lua(
                "return vulcan.call('vulcan-root-skill-ping', {})",
                &json!({}),
                None,
            )
            .expect("runtime Lua execution should use the active runtime view");
        assert_eq!(root_run_lua, json!("ok"));

        let _ = fs::remove_dir_all(&runtime_root);
    }

    /// Verify formal root chains reject unknown labels and reversed priority order.
    /// 验证正式根链会拒绝未知标签和反向优先级顺序。
    #[test]
    fn load_from_roots_rejects_unknown_or_reversed_formal_layers() {
        let runtime_root = make_temp_runtime_root("formal-root-chain-validation");
        if runtime_root.exists() {
            let _ = fs::remove_dir_all(&runtime_root);
        }
        let mut engine = make_runtime_test_engine();
        let reversed_error = engine
            .load_from_roots(&[
                RuntimeSkillRoot {
                    name: "USER".to_string(),
                    skills_dir: runtime_root.join("user_skills"),
                },
                RuntimeSkillRoot {
                    name: "ROOT".to_string(),
                    skills_dir: runtime_root.join("root_skills"),
                },
            ])
            .expect_err("reversed formal root order should fail");
        assert!(
            reversed_error
                .to_string()
                .contains("ROOT -> PROJECT -> USER")
        );

        let unknown_error = engine
            .load_from_roots(&[RuntimeSkillRoot {
                name: "WORKSPACE".to_string(),
                skills_dir: runtime_root.join("workspace_skills"),
            }])
            .expect_err("unknown formal root label should fail");
        assert!(
            unknown_error
                .to_string()
                .contains("unsupported skill root label")
        );

        let missing_root_error = engine
            .load_from_roots(&[RuntimeSkillRoot {
                name: "USER".to_string(),
                skills_dir: runtime_root.join("user_skills"),
            }])
            .expect_err("missing ROOT layer should fail");
        assert!(
            missing_root_error
                .to_string()
                .contains("ROOT skill root is required")
        );

        let _ = fs::remove_dir_all(&runtime_root);
    }

    /// Verify ordinary skills installs do not fall back to the system-controlled ROOT layer.
    /// 验证普通 skills 安装不会回落到系统控制的 ROOT 层。
    #[test]
    fn install_skill_rejects_root_only_runtime() {
        let runtime_root = make_temp_runtime_root("ordinary-install-root-only");
        if runtime_root.exists() {
            let _ = fs::remove_dir_all(&runtime_root);
        }
        let root_root = RuntimeSkillRoot {
            name: "ROOT".to_string(),
            skills_dir: runtime_root.join("root_skills"),
        };
        fs::create_dir_all(&root_root.skills_dir).expect("create root skills root");
        let mut engine = make_runtime_test_engine();

        let error = engine
            .install_skill(
                &[root_root],
                &SkillInstallRequest {
                    skill_id: Some("vulcan-codekit".to_string()),
                    source: None,
                    source_type: SkillInstallSourceType::Github,
                },
            )
            .expect_err("ordinary install must reject root-only runtime");
        assert!(error.to_string().contains("ROOT is system-controlled"));

        let _ = fs::remove_dir_all(&runtime_root);
    }

    /// Verify system installs do not fall back to ordinary layers when ROOT is absent.
    /// 验证 system 安装在缺少 ROOT 时不会回退到普通层。
    #[test]
    fn system_install_skill_rejects_runtime_without_root() {
        let runtime_root = make_temp_runtime_root("system-install-without-root");
        if runtime_root.exists() {
            let _ = fs::remove_dir_all(&runtime_root);
        }
        let user_root = RuntimeSkillRoot {
            name: "USER".to_string(),
            skills_dir: runtime_root.join("user_skills"),
        };
        fs::create_dir_all(&user_root.skills_dir).expect("create user skills root");
        let mut engine = make_runtime_test_engine();

        let error = engine
            .system_install_skill(
                &[user_root],
                SkillManagementAuthority::System,
                &SkillInstallRequest {
                    skill_id: Some("vulcan-codekit".to_string()),
                    source: None,
                    source_type: SkillInstallSourceType::Github,
                },
            )
            .expect_err("system install without ROOT should fail");
        assert!(error.to_string().contains("ROOT skill root is required"));

        let _ = fs::remove_dir_all(&runtime_root);
    }

    /// Verify the Lua-visible ordinary skill-management layer list excludes ROOT.
    /// 验证 Lua 可见的普通技能管理层级列表不包含 ROOT。
    #[test]
    fn runtime_skills_layers_excludes_root() {
        let runtime_root = make_temp_runtime_root("runtime-skills-layers-root-only");
        if runtime_root.exists() {
            let _ = fs::remove_dir_all(&runtime_root);
        }
        let root_root = RuntimeSkillRoot {
            name: "ROOT".to_string(),
            skills_dir: runtime_root.join("root_skills"),
        };
        let mut host_options = LuaRuntimeHostOptions::default();
        host_options.capabilities.enable_skill_management_bridge = true;
        let mut engine = LuaEngine::new(LuaEngineOptions {
            host_options,
            pool_config: LuaVmPoolConfig {
                min_size: 1,
                max_size: 1,
                idle_ttl_secs: 60,
            },
        })
        .expect("create root-only layer test engine");
        engine
            .load_from_roots(&[root_root])
            .expect("root-only runtime should load");
        let result = engine
            .run_lua("return vulcan.runtime.skills.layers()", &json!({}), None)
            .expect("layers function should run");

        assert_eq!(result["labels"], json!([]));
        assert_eq!(result["layers"], json!([]));
        assert_eq!(result["writable"], json!(false));
        assert!(result["default"].is_null());

        let _ = fs::remove_dir_all(&runtime_root);
    }

    /// Verify layers reflects loaded PROJECT and USER roots and the bridge writable policy.
    /// 验证 layers 会反映已加载 PROJECT/USER 根以及桥接写入策略。
    #[test]
    fn runtime_skills_layers_reflects_loaded_roots_and_bridge_policy() {
        let runtime_root = make_temp_runtime_root("runtime-skills-layers-dynamic");
        if runtime_root.exists() {
            let _ = fs::remove_dir_all(&runtime_root);
        }
        let root_root = RuntimeSkillRoot {
            name: "ROOT".to_string(),
            skills_dir: runtime_root.join("root_skills"),
        };
        let user_root = RuntimeSkillRoot {
            name: "USER".to_string(),
            skills_dir: runtime_root.join("user_skills"),
        };
        let mut engine = make_runtime_test_engine();
        engine
            .load_from_roots(&[root_root.clone(), user_root])
            .expect("root and user runtime should load");
        let disabled_result = engine
            .run_lua("return vulcan.runtime.skills.layers()", &json!({}), None)
            .expect("layers function should run when bridge is disabled");
        assert_eq!(disabled_result["default"], json!("USER"));
        assert_eq!(disabled_result["labels"], json!(["USER"]));
        assert_eq!(disabled_result["writable"], json!(false));
        assert_eq!(disabled_result["layers"][0]["writable"], json!(false));

        let mut host_options = LuaRuntimeHostOptions::default();
        host_options.capabilities.enable_skill_management_bridge = true;
        let mut enabled_engine = LuaEngine::new(LuaEngineOptions {
            host_options,
            pool_config: LuaVmPoolConfig {
                min_size: 1,
                max_size: 1,
                idle_ttl_secs: 60,
            },
        })
        .expect("create enabled layer test engine");
        let project_root = RuntimeSkillRoot {
            name: "PROJECT".to_string(),
            skills_dir: runtime_root.join("project_skills"),
        };
        enabled_engine
            .load_from_roots(&[
                root_root,
                project_root,
                RuntimeSkillRoot {
                    name: "USER".to_string(),
                    skills_dir: runtime_root.join("enabled_user_skills"),
                },
            ])
            .expect("root, project, user runtime should load");
        let enabled_result = enabled_engine
            .run_lua("return vulcan.runtime.skills.layers()", &json!({}), None)
            .expect("layers function should run when bridge is enabled");
        assert_eq!(enabled_result["default"], json!("USER"));
        assert_eq!(enabled_result["labels"], json!(["PROJECT", "USER"]));
        assert_eq!(enabled_result["writable"], json!(true));
        assert_eq!(enabled_result["layers"][0]["writable"], json!(true));
        assert!(
            enabled_result["labels"]
                .as_array()
                .unwrap()
                .iter()
                .all(|value| value != "ROOT")
        );

        let _ = fs::remove_dir_all(&runtime_root);
    }

    /// Verify the ordinary Lua bridge rejects ROOT targets before dispatching to the host callback.
    /// 验证普通 Lua 桥接会在分发到宿主回调前拒绝 ROOT 目标。
    #[test]
    fn runtime_skills_bridge_rejects_root_payload_before_callback() {
        let mut host_options = LuaRuntimeHostOptions::default();
        host_options.capabilities.enable_skill_management_bridge = true;
        let engine = LuaEngine::new(LuaEngineOptions {
            host_options,
            pool_config: LuaVmPoolConfig {
                min_size: 1,
                max_size: 1,
                idle_ttl_secs: 60,
            },
        })
        .expect("create bridge test engine");

        let error = engine
            .run_lua(
                "return vulcan.runtime.skills.install({ layer = 'ROOT', skill_id = 'vulcan-codekit' })",
                &json!({}),
                None,
            )
            .expect_err("root target should be rejected by bridge");
        assert!(error.contains("cannot target the system-controlled ROOT layer"));
        assert!(!error.contains("no host callback"));

        let object_error = engine
            .run_lua(
                "return vulcan.runtime.skills.install({ target_root = { name = 'ROOT', skills_dir = 'C:/tmp/root-skills' }, skill_id = 'vulcan-codekit' })",
                &json!({}),
                None,
            )
            .expect_err("root target object should be rejected by bridge");
        assert!(object_error.contains("cannot target the system-controlled ROOT layer"));
        assert!(!object_error.contains("no host callback"));
    }

    /// Verify ordinary explicit-root APIs reject ROOT write targets before lifecycle work starts.
    /// 验证普通显式根 API 会在生命周期工作开始前拒绝 ROOT 写入目标。
    #[test]
    fn ordinary_explicit_root_apis_reject_root_target() {
        let runtime_root = make_temp_runtime_root("ordinary-explicit-root-rejects-root");
        if runtime_root.exists() {
            let _ = fs::remove_dir_all(&runtime_root);
        }
        let root_root = RuntimeSkillRoot {
            name: "ROOT".to_string(),
            skills_dir: runtime_root.join("root_skills"),
        };
        let user_root = RuntimeSkillRoot {
            name: "USER".to_string(),
            skills_dir: runtime_root.join("user_skills"),
        };
        fs::create_dir_all(&root_root.skills_dir).expect("create root skills root");
        fs::create_dir_all(&user_root.skills_dir).expect("create user skills root");
        let skill_roots = vec![root_root.clone(), user_root];
        let mut engine = make_runtime_test_engine();

        let error = engine
            .install_skill_in_root(
                &skill_roots,
                &root_root,
                &SkillInstallRequest {
                    skill_id: Some("vulcan-codekit".to_string()),
                    source: None,
                    source_type: SkillInstallSourceType::Github,
                },
            )
            .expect_err("ordinary explicit root install should reject ROOT");
        assert!(error.to_string().contains("ordinary skills plane cannot"));

        let _ = fs::remove_dir_all(&runtime_root);
    }

    /// Verify ROOT-owned skill ids cannot be installed or updated in ordinary layers by any authority.
    /// 验证 ROOT 拥有的 skill id 不能被任何权限安装或更新到普通层。
    #[test]
    fn root_owned_skill_id_blocks_project_user_install_update_for_all_authorities() {
        let runtime_root = make_temp_runtime_root("root-owned-skill-id-blocks-ordinary");
        if runtime_root.exists() {
            let _ = fs::remove_dir_all(&runtime_root);
        }
        let root_root = RuntimeSkillRoot {
            name: "ROOT".to_string(),
            skills_dir: runtime_root.join("root_skills"),
        };
        let project_root = RuntimeSkillRoot {
            name: "PROJECT".to_string(),
            skills_dir: runtime_root.join("project_skills"),
        };
        write_minimal_skill_to_root(&root_root.skills_dir, "vulcan-codekit");
        write_minimal_skill_to_root(&project_root.skills_dir, "vulcan-codekit");
        let skill_roots = vec![root_root, project_root.clone()];
        let mut engine = make_runtime_test_engine();

        let ordinary_install_error = engine
            .install_skill_in_root(
                &skill_roots,
                &project_root,
                &SkillInstallRequest {
                    skill_id: Some("vulcan-codekit".to_string()),
                    source: None,
                    source_type: SkillInstallSourceType::Github,
                },
            )
            .expect_err("ordinary install must reject ROOT-owned skill id");
        assert!(
            ordinary_install_error
                .to_string()
                .contains("ROOT system layer")
        );

        let system_install_error = engine
            .system_install_skill_in_root(
                &skill_roots,
                &project_root,
                SkillManagementAuthority::System,
                &SkillInstallRequest {
                    skill_id: Some("vulcan-codekit".to_string()),
                    source: None,
                    source_type: SkillInstallSourceType::Github,
                },
            )
            .expect_err("system install must reject ROOT-owned skill id in PROJECT");
        assert!(
            system_install_error
                .to_string()
                .contains("ROOT system layer")
        );

        let system_update_error = engine
            .system_update_skill_in_root(
                &skill_roots,
                &project_root,
                SkillManagementAuthority::System,
                &SkillInstallRequest {
                    skill_id: Some("vulcan-codekit".to_string()),
                    source: None,
                    source_type: SkillInstallSourceType::Github,
                },
            )
            .expect_err("system update must also reject ROOT-owned skill id in PROJECT");
        assert!(
            system_update_error
                .to_string()
                .contains("ROOT system layer")
        );

        let delegated_update_error = engine
            .system_update_skill_in_root(
                &skill_roots,
                &project_root,
                SkillManagementAuthority::DelegatedTool,
                &SkillInstallRequest {
                    skill_id: Some("vulcan-codekit".to_string()),
                    source: None,
                    source_type: SkillInstallSourceType::Github,
                },
            )
            .expect_err("delegated update must reject ROOT-owned skill id in PROJECT");
        assert!(
            delegated_update_error
                .to_string()
                .contains("ROOT system layer")
        );

        let _ = fs::remove_dir_all(&runtime_root);
    }

    /// Verify ordinary explicit-root uninstall may clean a USER residual shadowed by ROOT.
    /// 验证普通显式根卸载可以清理被 ROOT 遮蔽的 USER 残留。
    #[test]
    fn ordinary_uninstall_in_root_cleans_user_residual_when_root_owns_same_skill_id() {
        let runtime_root = make_temp_runtime_root("ordinary-uninstall-cleans-root-shadow");
        if runtime_root.exists() {
            let _ = fs::remove_dir_all(&runtime_root);
        }
        let root_root = RuntimeSkillRoot {
            name: "ROOT".to_string(),
            skills_dir: runtime_root.join("root_skills"),
        };
        let user_root = RuntimeSkillRoot {
            name: "USER".to_string(),
            skills_dir: runtime_root.join("user_skills"),
        };
        let root_skill_dir = write_minimal_skill_to_root(&root_root.skills_dir, "vulcan-codekit");
        let user_skill_dir = write_minimal_skill_to_root(&user_root.skills_dir, "vulcan-codekit");
        let skill_roots = vec![root_root, user_root.clone()];
        let mut engine = make_runtime_test_engine();

        let result = engine
            .uninstall_skill_in_root(
                &skill_roots,
                &user_root,
                "vulcan-codekit",
                &SkillUninstallOptions::default(),
            )
            .expect("ordinary uninstall should clean USER residual");
        assert_eq!(result.skill_id, "vulcan-codekit");
        assert!(!user_skill_dir.exists());
        assert!(root_skill_dir.exists());

        let _ = fs::remove_dir_all(&runtime_root);
    }

    /// Verify delegated authority cannot use a system explicit-root API to write ROOT.
    /// 验证委托权限不能借助 system 显式根 API 写入 ROOT。
    #[test]
    fn delegated_authority_rejects_system_root_write() {
        let runtime_root = make_temp_runtime_root("delegated-system-root-write-reject");
        if runtime_root.exists() {
            let _ = fs::remove_dir_all(&runtime_root);
        }
        let root_root = RuntimeSkillRoot {
            name: "ROOT".to_string(),
            skills_dir: runtime_root.join("root_skills"),
        };
        fs::create_dir_all(&root_root.skills_dir).expect("create root skills root");
        let skill_roots = vec![root_root.clone()];
        let mut engine = make_runtime_test_engine();

        let error = engine
            .system_install_skill_in_root(
                &skill_roots,
                &root_root,
                SkillManagementAuthority::DelegatedTool,
                &SkillInstallRequest {
                    skill_id: Some("vulcan-codekit".to_string()),
                    source: None,
                    source_type: SkillInstallSourceType::Github,
                },
            )
            .expect_err("delegated authority must reject ROOT writes");
        assert!(error.to_string().contains("DelegatedTool authority"));

        let _ = fs::remove_dir_all(&runtime_root);
    }

    /// Verify explicit-root system updates fail instead of returning a successful missing-skill result.
    /// 验证显式根 system 更新在缺少目标技能时会失败，而不是返回成功的 missing-skill 结果。
    #[test]
    fn system_update_skill_in_root_missing_target_returns_error() {
        let runtime_root = make_temp_runtime_root("system-update-target-missing");
        if runtime_root.exists() {
            let _ = fs::remove_dir_all(&runtime_root);
        }
        let user_root = RuntimeSkillRoot {
            name: "USER".to_string(),
            skills_dir: runtime_root.join("user").join("skills"),
        };
        let root_root = RuntimeSkillRoot {
            name: "ROOT".to_string(),
            skills_dir: runtime_root.join("root").join("skills"),
        };
        fs::create_dir_all(&user_root.skills_dir).expect("create user skills root");
        fs::create_dir_all(&root_root.skills_dir).expect("create root skills root");
        let skill_roots = vec![root_root, user_root.clone()];
        let mut engine = make_runtime_test_engine();

        let error = engine
            .system_update_skill_in_root(
                &skill_roots,
                &user_root,
                SkillManagementAuthority::System,
                &SkillInstallRequest {
                    skill_id: Some("vulcan-codekit".to_string()),
                    source: None,
                    source_type: SkillInstallSourceType::Github,
                },
            )
            .expect_err("missing explicit-root update target should fail");
        let rendered = error.to_string();

        assert!(rendered.contains("not installed in target root 'USER'"));
        let _ = fs::remove_dir_all(&runtime_root);
    }

    /// Verify explicit-root apply rejects PROJECT changes when ROOT owns the same skill id.
    /// 验证明确定根应用会在 ROOT 拥有同名 skill 时拒绝 PROJECT 变更。
    #[test]
    fn system_update_skill_in_root_rejects_shadowed_fallback_target() {
        let runtime_root = make_temp_runtime_root("system-update-shadowed-root");
        if runtime_root.exists() {
            let _ = fs::remove_dir_all(&runtime_root);
        }
        let root_root = RuntimeSkillRoot {
            name: "ROOT".to_string(),
            skills_dir: runtime_root.join("root_skills"),
        };
        let project_root = RuntimeSkillRoot {
            name: "PROJECT".to_string(),
            skills_dir: runtime_root.join("project_skills"),
        };
        write_minimal_skill_to_root(&root_root.skills_dir, "vulcan-codekit");
        write_minimal_skill_to_root(&project_root.skills_dir, "vulcan-codekit");
        let skill_roots = vec![root_root, project_root.clone()];
        let mut engine = make_runtime_test_engine();

        let error = engine
            .system_update_skill_in_root(
                &skill_roots,
                &project_root,
                SkillManagementAuthority::System,
                &SkillInstallRequest {
                    skill_id: Some("vulcan-codekit".to_string()),
                    source: None,
                    source_type: SkillInstallSourceType::Github,
                },
            )
            .expect_err("shadowed fallback target should fail before update");
        let rendered = error.to_string();

        assert!(rendered.contains("ROOT system layer"));
        let _ = fs::remove_dir_all(&runtime_root);
    }

    /// Verify explicit-root install derives skill ids with the same GitHub locator rules as the manager.
    /// 验证明确定根安装使用与管理器一致的 GitHub 定位规则推导技能标识。
    #[test]
    fn system_install_skill_in_root_accepts_trailing_slash_github_url_for_shadow_check() {
        let runtime_root = make_temp_runtime_root("system-install-trailing-slash-source");
        if runtime_root.exists() {
            let _ = fs::remove_dir_all(&runtime_root);
        }
        let user_root = RuntimeSkillRoot {
            name: "USER".to_string(),
            skills_dir: runtime_root.join("user_skills"),
        };
        let root_root = RuntimeSkillRoot {
            name: "ROOT".to_string(),
            skills_dir: runtime_root.join("root_skills"),
        };
        write_minimal_skill_to_root(&user_root.skills_dir, "vulcan-codekit");
        fs::create_dir_all(&root_root.skills_dir).expect("create root skills root");
        let skill_roots = vec![root_root.clone(), user_root];
        let mut engine = make_runtime_test_engine();

        let error = engine
            .system_install_skill_in_root(
                &skill_roots,
                &root_root,
                SkillManagementAuthority::System,
                &SkillInstallRequest {
                    skill_id: None,
                    source: Some("https://github.com/LuaSkills/vulcan-codekit/".to_string()),
                    source_type: SkillInstallSourceType::Github,
                },
            )
            .expect_err("root install should derive source skill id before managed download");
        let rendered = error.to_string();

        assert!(!rendered.contains("shadowed by higher-priority root"));
        assert!(!rendered.contains("requires skill_id"));
        let _ = fs::remove_dir_all(&runtime_root);
    }

    /// Verify explicit-root system updates reject unlisted targets before probing target contents.
    /// 验证明确定根 system 更新会在探测目标内容前拒绝链外目标。
    #[test]
    fn system_update_skill_in_root_rejects_unlisted_target_before_missing_target() {
        let runtime_root = make_temp_runtime_root("system-update-unlisted-root");
        if runtime_root.exists() {
            let _ = fs::remove_dir_all(&runtime_root);
        }
        let user_root = RuntimeSkillRoot {
            name: "USER".to_string(),
            skills_dir: runtime_root.join("user_skills"),
        };
        let root_root = RuntimeSkillRoot {
            name: "ROOT".to_string(),
            skills_dir: runtime_root.join("root_skills"),
        };
        let rogue_root = RuntimeSkillRoot {
            name: "ROOT".to_string(),
            skills_dir: runtime_root.join("rogue_skills"),
        };
        fs::create_dir_all(&root_root.skills_dir).expect("create root skills root");
        fs::create_dir_all(&user_root.skills_dir).expect("create user skills root");
        let skill_roots = vec![root_root, user_root];
        let mut engine = make_runtime_test_engine();

        let error = engine
            .system_update_skill_in_root(
                &skill_roots,
                &rogue_root,
                SkillManagementAuthority::System,
                &SkillInstallRequest {
                    skill_id: Some("vulcan-codekit".to_string()),
                    source: None,
                    source_type: SkillInstallSourceType::Github,
                },
            )
            .expect_err("unlisted explicit update target root should be rejected");
        let rendered = error.to_string();

        assert!(rendered.contains("not part of the full runtime root chain"));
        assert!(!rendered.contains("not installed in target root"));
        let _ = fs::remove_dir_all(&runtime_root);
    }

    /// Verify explicit-root uninstall rejects target roots outside the active runtime chain.
    /// 验证明确定根卸载会拒绝当前运行时根链之外的目标根。
    #[test]
    fn system_uninstall_skill_in_root_rejects_unlisted_target_root() {
        let runtime_root = make_temp_runtime_root("system-uninstall-unlisted-root");
        if runtime_root.exists() {
            let _ = fs::remove_dir_all(&runtime_root);
        }
        let user_root = RuntimeSkillRoot {
            name: "USER".to_string(),
            skills_dir: runtime_root.join("user").join("skills"),
        };
        let root_root = RuntimeSkillRoot {
            name: "ROOT".to_string(),
            skills_dir: runtime_root.join("root").join("skills"),
        };
        let rogue_root = RuntimeSkillRoot {
            name: "ROGUE".to_string(),
            skills_dir: runtime_root.join("rogue").join("skills"),
        };
        fs::create_dir_all(&root_root.skills_dir).expect("create root skills root");
        fs::create_dir_all(&user_root.skills_dir).expect("create user skills root");
        let rogue_skill_dir = write_minimal_skill_to_root(&rogue_root.skills_dir, "vulcan-codekit");
        let skill_roots = vec![root_root, user_root];
        let mut engine = make_runtime_test_engine();

        let error = engine
            .system_uninstall_skill_in_root(
                &skill_roots,
                &rogue_root,
                SkillManagementAuthority::System,
                "vulcan-codekit",
                &SkillUninstallOptions::default(),
            )
            .expect_err("unlisted explicit target root should be rejected");
        let rendered = error.to_string();

        assert!(rendered.contains("not part of the full runtime root chain"));
        assert!(
            rogue_skill_dir.exists(),
            "unlisted target skill directory should not be removed"
        );
        let _ = fs::remove_dir_all(&runtime_root);
    }

    /// Verify the isolated runlua pool uses the documented default sizing when the host does not override it.
    /// 验证宿主未覆盖时隔离 runlua 池会使用文档声明的默认容量配置。
    #[test]
    fn runlua_pool_uses_default_config_when_host_does_not_override() {
        let engine = make_runtime_test_engine();
        assert_eq!(engine.runlua_pool.config.min_size, 1);
        assert_eq!(engine.runlua_pool.config.max_size, 4);
        assert_eq!(engine.runlua_pool.config.idle_ttl_secs, 60);
    }

    /// Verify the host can override the isolated runlua pool sizing with the same shape as the main VM pool.
    /// 验证宿主可以使用与主虚拟机池相同的参数形状覆盖隔离 runlua 池容量。
    #[test]
    fn runlua_pool_honors_host_override_config() {
        let mut host_options = LuaRuntimeHostOptions::default();
        host_options.runlua_pool_config = Some(LuaRuntimeRunLuaPoolConfig {
            min_size: 2,
            max_size: 5,
            idle_ttl_secs: 90,
        });
        let engine = LuaEngine::new(LuaEngineOptions {
            host_options,
            pool_config: LuaVmPoolConfig {
                min_size: 1,
                max_size: 1,
                idle_ttl_secs: 60,
            },
        })
        .expect("create runtime test engine with custom runlua pool");
        assert_eq!(engine.runlua_pool.config.min_size, 2);
        assert_eq!(engine.runlua_pool.config.max_size, 5);
        assert_eq!(engine.runlua_pool.config.idle_ttl_secs, 90);
    }

    /// Verify the engine host API persists string skill config values into one explicit config file.
    /// 验证引擎宿主 API 会把字符串技能配置值持久化到显式配置文件中。
    #[test]
    fn skill_config_engine_api_persists_values_into_explicit_file() {
        let runtime_root = make_temp_runtime_root("skill_config_explicit_path");
        if runtime_root.exists() {
            let _ = fs::remove_dir_all(&runtime_root);
        }
        create_runtime_test_layout(&runtime_root);
        let config_file = runtime_root.join("custom").join("skill_config.json");

        let mut host_options = LuaRuntimeHostOptions::default();
        host_options.skill_config_file_path = Some(config_file.clone());
        let mut engine = LuaEngine::new(LuaEngineOptions {
            host_options,
            pool_config: LuaVmPoolConfig {
                min_size: 1,
                max_size: 1,
                idle_ttl_secs: 60,
            },
        })
        .expect("create skill config test engine");

        engine
            .set_skill_config_value("demo-skill", "api_token", "sk-explicit")
            .expect("set explicit skill config");
        assert_eq!(
            engine
                .get_skill_config_value("demo-skill", "api_token")
                .expect("read explicit skill config"),
            Some("sk-explicit".to_string())
        );
        let entries = engine
            .list_skill_config_entries(Some("demo-skill"))
            .expect("list explicit skill config");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].skill_id, "demo-skill");
        assert_eq!(entries[0].key, "api_token");
        assert_eq!(entries[0].value, "sk-explicit");
        assert!(config_file.exists());

        let deleted = engine
            .delete_skill_config_value("demo-skill", "api_token")
            .expect("delete explicit skill config");
        assert!(deleted);
        assert_eq!(
            engine
                .get_skill_config_value("demo-skill", "api_token")
                .expect("read deleted explicit skill config"),
            None
        );

        let _ = fs::remove_dir_all(&runtime_root);
    }

    /// Verify the unified skill config store falls back to `<runtime_root>/config/skill_config.json` after roots load.
    /// 验证统一技能配置存储会在加载根目录后回退到 `<runtime_root>/config/skill_config.json`。
    #[test]
    fn skill_config_store_uses_default_runtime_config_file_after_load() {
        let runtime_root = make_temp_runtime_root("skill_config_default_path");
        if runtime_root.exists() {
            let _ = fs::remove_dir_all(&runtime_root);
        }
        create_runtime_test_layout(&runtime_root);

        let mut engine = LuaEngine::new(LuaEngineOptions {
            host_options: LuaRuntimeHostOptions::default(),
            pool_config: LuaVmPoolConfig {
                min_size: 1,
                max_size: 1,
                idle_ttl_secs: 60,
            },
        })
        .expect("create default skill config test engine");

        engine
            .load_from_roots(&[crate::host::options::RuntimeSkillRoot {
                name: "ROOT".to_string(),
                skills_dir: runtime_root.join("skills"),
            }])
            .expect("load empty roots for default skill config path");

        let expected_path = runtime_root.join("config").join("skill_config.json");
        assert_eq!(
            engine
                .skill_config_store
                .file_path()
                .expect("resolve default skill config file path"),
            expected_path
        );

        engine
            .set_skill_config_value("demo-skill", "endpoint", "https://example.test")
            .expect("write default skill config");
        assert!(expected_path.exists());

        let _ = fs::remove_dir_all(&runtime_root);
    }

    /// Verify the unified skill config store resolves the default config path even before the skills directory exists.
    /// 验证统一技能配置存储会在技能目录尚未创建前解析默认配置路径。
    #[test]
    fn skill_config_store_initializes_default_path_before_skills_dir_exists() {
        let runtime_root = make_temp_runtime_root("skill_config_without_skills_dir");
        if runtime_root.exists() {
            let _ = fs::remove_dir_all(&runtime_root);
        }
        fs::create_dir_all(&runtime_root).expect("create runtime root without skills dir");

        let missing_skills_dir = runtime_root.join("skills");
        let mut engine = LuaEngine::new(LuaEngineOptions {
            host_options: LuaRuntimeHostOptions::default(),
            pool_config: LuaVmPoolConfig {
                min_size: 1,
                max_size: 1,
                idle_ttl_secs: 60,
            },
        })
        .expect("create config path initialization test engine");

        engine
            .load_from_roots(&[crate::host::options::RuntimeSkillRoot {
                name: "ROOT".to_string(),
                skills_dir: missing_skills_dir,
            }])
            .expect("load roots without an existing skills directory");

        let expected_path = runtime_root.join("config").join("skill_config.json");
        assert_eq!(
            engine
                .skill_config_store
                .file_path()
                .expect("resolve config path without skills directory"),
            expected_path
        );

        engine
            .set_skill_config_value("demo-skill", "api_token", "sk-before-install")
            .expect("write config before any skills directory exists");
        assert!(expected_path.exists());

        let _ = fs::remove_dir_all(&runtime_root);
    }

    /// Verify invalid reload requests fail before clearing the active runtime view.
    /// 验证无效重载请求会在清空当前运行时视图前失败。
    #[test]
    fn reload_from_roots_rejects_invalid_chain_before_resetting_runtime_state() {
        let runtime_root = make_temp_runtime_root("reload-invalid-chain-preserves-state");
        if runtime_root.exists() {
            let _ = fs::remove_dir_all(&runtime_root);
        }
        let root_root = RuntimeSkillRoot {
            name: "ROOT".to_string(),
            skills_dir: runtime_root.join("root_skills"),
        };
        let user_root = RuntimeSkillRoot {
            name: "USER".to_string(),
            skills_dir: runtime_root.join("user_skills"),
        };
        fs::create_dir_all(&root_root.skills_dir).expect("create root skills root");
        write_minimal_skill_to_root_with_response(&user_root.skills_dir, "vulcan-codekit", "user");
        let mut engine = make_runtime_test_engine();
        engine
            .load_from_roots(&[root_root, user_root.clone()])
            .expect("initial root and user runtime should load");

        let invalid_reload_error = engine
            .reload_from_roots(&[user_root])
            .expect_err("missing ROOT reload should fail");
        assert!(
            invalid_reload_error
                .to_string()
                .contains("ROOT skill root is required")
        );

        let result = engine
            .call_skill("vulcan-codekit-ping", &json!({}), None)
            .expect("old entry should remain callable after failed reload");
        assert_eq!(result.content, "user");

        let layers = engine
            .run_lua("return vulcan.runtime.skills.layers()", &json!({}), None)
            .expect("layers should still use the previously loaded root chain");
        assert_eq!(layers["labels"], json!(["USER"]));
        assert_eq!(layers["default"], json!("USER"));

        let _ = fs::remove_dir_all(&runtime_root);
    }

    /// Verify reload failures after formal validation still preserve the active runtime view.
    /// 验证 formal 校验之后发生的重载失败仍会保留当前活动运行时视图。
    #[test]
    fn reload_from_roots_preserves_state_after_ambiguous_config_root_error() {
        let runtime_root = make_temp_runtime_root("reload-ambiguous-preserves-state");
        let first_ambiguous_root = make_temp_runtime_root("reload-ambiguous-first");
        let second_ambiguous_root = make_temp_runtime_root("reload-ambiguous-second");
        for path in [&runtime_root, &first_ambiguous_root, &second_ambiguous_root] {
            if path.exists() {
                let _ = fs::remove_dir_all(path);
            }
        }
        let root_root = RuntimeSkillRoot {
            name: "ROOT".to_string(),
            skills_dir: runtime_root.join("root_skills"),
        };
        let user_root = RuntimeSkillRoot {
            name: "USER".to_string(),
            skills_dir: runtime_root.join("user_skills"),
        };
        fs::create_dir_all(&root_root.skills_dir).expect("create root skills root");
        write_minimal_skill_to_root_with_response(&user_root.skills_dir, "vulcan-codekit", "user");
        let mut engine = make_runtime_test_engine();
        engine
            .load_from_roots(&[root_root, user_root])
            .expect("initial root and user runtime should load");
        let previous_config_path = engine
            .skill_config_store
            .file_path()
            .expect("resolve previous skill config path");

        let ambiguous_reload_error = engine
            .reload_from_roots(&[
                RuntimeSkillRoot {
                    name: "ROOT".to_string(),
                    skills_dir: first_ambiguous_root.join("skills"),
                },
                RuntimeSkillRoot {
                    name: "PROJECT".to_string(),
                    skills_dir: second_ambiguous_root.join("skills"),
                },
            ])
            .expect_err("ambiguous config root reload should fail");
        assert!(
            ambiguous_reload_error
                .to_string()
                .contains("multiple runtime roots map to different parents")
        );

        let result = engine
            .call_skill("vulcan-codekit-ping", &json!({}), None)
            .expect("old entry should remain callable after ambiguous reload failure");
        assert_eq!(result.content, "user");
        assert_eq!(
            engine
                .skill_config_store
                .file_path()
                .expect("resolve config path after failed reload"),
            previous_config_path
        );

        let layers = engine
            .run_lua("return vulcan.runtime.skills.layers()", &json!({}), None)
            .expect("layers should still use the previous root chain");
        assert_eq!(layers["labels"], json!(["USER"]));
        assert_eq!(layers["default"], json!("USER"));

        let _ = fs::remove_dir_all(&runtime_root);
        let _ = fs::remove_dir_all(&first_ambiguous_root);
        let _ = fs::remove_dir_all(&second_ambiguous_root);
    }

    /// Verify reloading a different runtime root updates the default unified skill-config path.
    /// 验证重新加载另一套运行时根目录时会同步更新默认统一技能配置路径。
    #[test]
    fn reload_from_roots_updates_default_skill_config_path() {
        let first_runtime_root = make_temp_runtime_root("skill_config_reload_first");
        let second_runtime_root = make_temp_runtime_root("skill_config_reload_second");
        if first_runtime_root.exists() {
            let _ = fs::remove_dir_all(&first_runtime_root);
        }
        if second_runtime_root.exists() {
            let _ = fs::remove_dir_all(&second_runtime_root);
        }
        create_runtime_test_layout(&first_runtime_root);
        create_runtime_test_layout(&second_runtime_root);

        let mut engine = LuaEngine::new(LuaEngineOptions {
            host_options: LuaRuntimeHostOptions::default(),
            pool_config: LuaVmPoolConfig {
                min_size: 1,
                max_size: 1,
                idle_ttl_secs: 60,
            },
        })
        .expect("create reload skill config test engine");

        engine
            .load_from_roots(&[crate::host::options::RuntimeSkillRoot {
                name: "ROOT".to_string(),
                skills_dir: first_runtime_root.join("skills"),
            }])
            .expect("load first runtime root");
        assert_eq!(
            engine
                .skill_config_store
                .file_path()
                .expect("resolve first config path"),
            first_runtime_root.join("config").join("skill_config.json")
        );

        engine
            .reload_from_roots(&[crate::host::options::RuntimeSkillRoot {
                name: "ROOT".to_string(),
                skills_dir: second_runtime_root.join("skills"),
            }])
            .expect("reload second runtime root");
        assert_eq!(
            engine
                .skill_config_store
                .file_path()
                .expect("resolve second config path"),
            second_runtime_root.join("config").join("skill_config.json")
        );

        let _ = fs::remove_dir_all(&first_runtime_root);
        let _ = fs::remove_dir_all(&second_runtime_root);
    }

    /// Verify reload keeps the initially resolved explicit relative skill-config path.
    /// 验证重载会保持初始解析后的显式相对技能配置路径。
    #[test]
    fn reload_from_roots_keeps_frozen_relative_explicit_skill_config_path() {
        let _cwd_guard = runlua_cwd_guard()
            .lock()
            .expect("lock cwd guard for explicit config reload test");
        let original_cwd = std::env::current_dir().expect("resolve original cwd");
        /// Restore the process current directory when the test exits.
        /// 在测试退出时恢复进程当前工作目录。
        struct CwdRestoreGuard(PathBuf);
        impl Drop for CwdRestoreGuard {
            fn drop(&mut self) {
                let _ = std::env::set_current_dir(&self.0);
            }
        }
        let _cwd_restore = CwdRestoreGuard(original_cwd);
        let first_cwd = make_temp_runtime_root("skill_config_reload_relative_cwd_first");
        let second_cwd = make_temp_runtime_root("skill_config_reload_relative_cwd_second");
        let runtime_root = make_temp_runtime_root("skill_config_reload_relative_runtime");
        for path in [&first_cwd, &second_cwd, &runtime_root] {
            if path.exists() {
                let _ = fs::remove_dir_all(path);
            }
            fs::create_dir_all(path).expect("create explicit config reload test directory");
        }
        let relative_config_path = PathBuf::from("config").join("skill_config.json");
        std::env::set_current_dir(&first_cwd).expect("switch to first cwd");
        let expected_config_path = first_cwd.join(&relative_config_path);

        let mut host_options = LuaRuntimeHostOptions::default();
        host_options.skill_config_file_path = Some(relative_config_path);
        let mut engine = LuaEngine::new(LuaEngineOptions {
            host_options,
            pool_config: LuaVmPoolConfig {
                min_size: 1,
                max_size: 1,
                idle_ttl_secs: 60,
            },
        })
        .expect("create explicit relative config reload test engine");
        engine
            .load_from_roots(&[RuntimeSkillRoot {
                name: "ROOT".to_string(),
                skills_dir: runtime_root.join("root_skills"),
            }])
            .expect("load initial root for explicit relative config reload test");
        assert_eq!(
            engine
                .skill_config_store
                .file_path()
                .expect("resolve explicit config path before reload"),
            expected_config_path
        );

        std::env::set_current_dir(&second_cwd).expect("switch to second cwd before reload");
        engine
            .reload_from_roots(&[RuntimeSkillRoot {
                name: "ROOT".to_string(),
                skills_dir: runtime_root.join("other_root_skills"),
            }])
            .expect("reload should preserve frozen explicit config path");
        assert_eq!(
            engine
                .skill_config_store
                .file_path()
                .expect("resolve explicit config path after reload"),
            expected_config_path
        );

        let _ = fs::remove_dir_all(&first_cwd);
        let _ = fs::remove_dir_all(&second_cwd);
        let _ = fs::remove_dir_all(&runtime_root);
    }

    /// Verify explicit unified config file paths bypass ambiguous runtime-root inference.
    /// 验证显式统一配置文件路径会绕过歧义运行时根目录推导。
    #[test]
    fn load_from_roots_accepts_explicit_skill_config_path_for_ambiguous_runtime_roots() {
        let first_runtime_root = make_temp_runtime_root("skill_config_explicit_ambiguous_first");
        let second_runtime_root = make_temp_runtime_root("skill_config_explicit_ambiguous_second");
        if first_runtime_root.exists() {
            let _ = fs::remove_dir_all(&first_runtime_root);
        }
        if second_runtime_root.exists() {
            let _ = fs::remove_dir_all(&second_runtime_root);
        }
        fs::create_dir_all(&first_runtime_root)
            .expect("create first explicit ambiguous runtime root");
        fs::create_dir_all(&second_runtime_root)
            .expect("create second explicit ambiguous runtime root");
        let explicit_config_file = first_runtime_root.join("custom").join("skill_config.json");

        let mut host_options = LuaRuntimeHostOptions::default();
        host_options.skill_config_file_path = Some(explicit_config_file.clone());
        let mut engine = LuaEngine::new(LuaEngineOptions {
            host_options,
            pool_config: LuaVmPoolConfig {
                min_size: 1,
                max_size: 1,
                idle_ttl_secs: 60,
            },
        })
        .expect("create explicit ambiguous root test engine");

        engine
            .load_from_roots(&[
                crate::host::options::RuntimeSkillRoot {
                    name: "ROOT".to_string(),
                    skills_dir: first_runtime_root.join("skills"),
                },
                crate::host::options::RuntimeSkillRoot {
                    name: "PROJECT".to_string(),
                    skills_dir: second_runtime_root.join("skills"),
                },
            ])
            .expect("explicit config path should bypass ambiguous runtime roots");

        assert_eq!(
            engine
                .skill_config_store
                .file_path()
                .expect("resolve explicit config path"),
            explicit_config_file
        );

        let _ = fs::remove_dir_all(&first_runtime_root);
        let _ = fs::remove_dir_all(&second_runtime_root);
    }

    /// Verify divergent runtime roots require one explicit unified skill config file path.
    /// 验证运行时根目录分叉时必须显式提供统一技能配置文件路径。
    #[test]
    fn load_from_roots_rejects_ambiguous_default_skill_config_runtime_root() {
        let first_runtime_root = make_temp_runtime_root("skill_config_ambiguous_first");
        let second_runtime_root = make_temp_runtime_root("skill_config_ambiguous_second");
        if first_runtime_root.exists() {
            let _ = fs::remove_dir_all(&first_runtime_root);
        }
        if second_runtime_root.exists() {
            let _ = fs::remove_dir_all(&second_runtime_root);
        }
        fs::create_dir_all(&first_runtime_root).expect("create first ambiguous runtime root");
        fs::create_dir_all(&second_runtime_root).expect("create second ambiguous runtime root");

        let mut engine = LuaEngine::new(LuaEngineOptions {
            host_options: LuaRuntimeHostOptions::default(),
            pool_config: LuaVmPoolConfig {
                min_size: 1,
                max_size: 1,
                idle_ttl_secs: 60,
            },
        })
        .expect("create ambiguous root test engine");

        let error = engine
            .load_from_roots(&[
                crate::host::options::RuntimeSkillRoot {
                    name: "ROOT".to_string(),
                    skills_dir: first_runtime_root.join("skills"),
                },
                crate::host::options::RuntimeSkillRoot {
                    name: "PROJECT".to_string(),
                    skills_dir: second_runtime_root.join("skills"),
                },
            ])
            .expect_err("ambiguous runtime roots should require an explicit config file path");
        assert!(
            error
                .to_string()
                .contains("set host_options.skill_config_file_path explicitly"),
            "unexpected ambiguous root error: {error}"
        );

        let _ = fs::remove_dir_all(&first_runtime_root);
        let _ = fs::remove_dir_all(&second_runtime_root);
    }

    /// Verify lexically equivalent runtime roots do not get misclassified as ambiguous.
    /// 验证词法等价的运行时根目录不会被误判为歧义根目录。
    #[test]
    fn canonical_skill_config_runtime_root_normalizes_equivalent_runtime_roots() {
        let runtime_root = make_temp_runtime_root("skill_config_equivalent_runtime_root");
        if runtime_root.exists() {
            let _ = fs::remove_dir_all(&runtime_root);
        }
        create_runtime_test_layout(&runtime_root);

        let engine = LuaEngine::new(LuaEngineOptions {
            host_options: LuaRuntimeHostOptions::default(),
            pool_config: LuaVmPoolConfig {
                min_size: 1,
                max_size: 1,
                idle_ttl_secs: 60,
            },
        })
        .expect("create equivalent runtime root test engine");

        let equivalent_root = runtime_root.join("nested").join("..").join("skills");
        let resolved_runtime_root = engine
            .canonical_skill_config_runtime_root(&[
                crate::host::options::RuntimeSkillRoot {
                    name: "ROOT".to_string(),
                    skills_dir: runtime_root.join("skills"),
                },
                crate::host::options::RuntimeSkillRoot {
                    name: "PROJECT".to_string(),
                    skills_dir: equivalent_root,
                },
            ])
            .expect("equivalent runtime roots should resolve to one canonical root");

        assert_eq!(resolved_runtime_root, runtime_root);

        let _ = fs::remove_dir_all(&runtime_root);
    }

    /// Verify one loaded skill can read its own namespaced config through `vulcan.config.get`.
    /// 验证单个已加载技能可以通过 `vulcan.config.get` 读取自己的命名空间配置。
    #[test]
    fn call_skill_reads_own_skill_config_namespace() {
        let runtime_root = make_temp_runtime_root("skill_config_call_skill");
        if runtime_root.exists() {
            let _ = fs::remove_dir_all(&runtime_root);
        }
        create_runtime_test_layout(&runtime_root);
        write_skill_config_test_skill(&runtime_root, "demo-skill");

        let mut engine = LuaEngine::new(LuaEngineOptions {
            host_options: LuaRuntimeHostOptions::default(),
            pool_config: LuaVmPoolConfig {
                min_size: 1,
                max_size: 1,
                idle_ttl_secs: 60,
            },
        })
        .expect("create call_skill config test engine");
        engine
            .load_from_roots(&[crate::host::options::RuntimeSkillRoot {
                name: "ROOT".to_string(),
                skills_dir: runtime_root.join("skills"),
            }])
            .expect("load config test skill");
        engine
            .set_skill_config_value("demo-skill", "api_token", "sk-from-config")
            .expect("seed skill config value");

        let result = engine
            .call_skill("demo-skill-ping", &json!({}), None)
            .expect("call skill with config");
        assert_eq!(result.content, "sk-from-config");

        let _ = fs::remove_dir_all(&runtime_root);
    }

    /// Verify `vulcan.config.*` rejects calls that execute without one active skill context.
    /// 验证 `vulcan.config.*` 会拒绝在没有活动技能上下文时执行的调用。
    #[test]
    fn run_lua_config_api_requires_active_skill_context() {
        let engine = make_runtime_test_engine();
        let error = engine
            .run_lua("return vulcan.config.get('api_token')", &json!({}), None)
            .expect_err("run_lua config access should require active skill context");
        assert!(error.contains("vulcan.config.get requires one active skill context"));
    }

    /// Verify `vulcan.models.*` reports disabled capabilities and structured unavailable errors by default.
    /// 验证 `vulcan.models.*` 默认报告能力未开启，并返回结构化不可用错误。
    #[test]
    fn vulcan_models_defaults_without_callbacks() {
        let _guard = runtime_model_callback_test_guard();
        let engine = make_runtime_test_engine();
        let result = engine
            .run_lua(
                r#"
local status = vulcan.models.status()
local embed = vulcan.models.embed("x")
local llm = vulcan.models.llm("s", "u")
return {
  status_ok = status.ok,
  embed_capability = status.capabilities.embed,
  llm_capability = status.capabilities.llm,
  has_embed = vulcan.models.has("embed"),
  has_llm = vulcan.models.has("llm"),
  has_unknown = vulcan.models.has("rerank"),
  embed_ok = embed.ok,
  embed_code = embed.error.code,
  llm_ok = llm.ok,
  llm_code = llm.error.code,
}
"#,
                &json!({}),
                None,
            )
            .expect("run model defaults lua");

        assert_eq!(result["status_ok"], true);
        assert_eq!(result["embed_capability"], false);
        assert_eq!(result["llm_capability"], false);
        assert_eq!(result["has_embed"], false);
        assert_eq!(result["has_llm"], false);
        assert_eq!(result["has_unknown"], false);
        assert_eq!(result["embed_ok"], false);
        assert_eq!(result["embed_code"], "model_unavailable");
        assert_eq!(result["llm_ok"], false);
        assert_eq!(result["llm_code"], "model_unavailable");
    }

    /// Verify model APIs return structured invalid-argument errors instead of throwing to Lua.
    /// 验证模型 API 会返回结构化非法参数错误，而不是向 Lua 抛出异常。
    #[test]
    fn vulcan_models_validate_arguments() {
        let _guard = runtime_model_callback_test_guard();
        let engine = make_runtime_test_engine();
        let result = engine
            .run_lua(
                r#"
local embed_empty = vulcan.models.embed("")
local embed_table = vulcan.models.embed({ "a", "b" })
local embed_extra = vulcan.models.embed("x", "extra")
local llm_empty_system = vulcan.models.llm("", "u")
local llm_empty_user = vulcan.models.llm("s", "")
local llm_extra = vulcan.models.llm("s", "u", "extra")
return {
  embed_empty = embed_empty.error.code,
  embed_table = embed_table.error.code,
  embed_extra = embed_extra.error.code,
  llm_empty_system = llm_empty_system.error.code,
  llm_empty_user = llm_empty_user.error.code,
  llm_extra = llm_extra.error.code,
}
"#,
                &json!({}),
                None,
            )
            .expect("run model argument validation lua");

        assert_eq!(result["embed_empty"], "invalid_argument");
        assert_eq!(result["embed_table"], "invalid_argument");
        assert_eq!(result["embed_extra"], "invalid_argument");
        assert_eq!(result["llm_empty_system"], "invalid_argument");
        assert_eq!(result["llm_empty_user"], "invalid_argument");
        assert_eq!(result["llm_extra"], "invalid_argument");
    }

    /// Verify registered embedding callbacks receive text and full caller context.
    /// 验证已注册的 embedding 回调会收到文本和完整调用方上下文。
    #[test]
    fn vulcan_models_embed_dispatches_registered_callback_with_context() {
        let _guard = runtime_model_callback_test_guard();
        let captured_request: Arc<Mutex<Option<RuntimeModelEmbedRequest>>> =
            Arc::new(Mutex::new(None));
        let captured_request_for_callback = captured_request.clone();
        set_model_embed_callback(Some(Arc::new(move |request| {
            *captured_request_for_callback
                .lock()
                .expect("lock captured embed request") = Some(request.clone());
            Ok(RuntimeModelEmbedResponse {
                vector: vec![0.25, 0.5, 0.75],
                dimensions: 3,
                usage: Some(RuntimeModelUsage {
                    input_tokens: Some(2),
                    output_tokens: None,
                }),
            })
        })));

        let engine = make_runtime_test_engine();
        let has_embed = engine
            .run_lua("return vulcan.models.has('embed')", &json!({}), None)
            .expect("run has embed lua");
        assert_eq!(has_embed, json!(true));

        let runtime_root = make_temp_runtime_root("model-embed-context");
        if runtime_root.exists() {
            let _ = fs::remove_dir_all(&runtime_root);
        }
        create_runtime_test_layout(&runtime_root);
        let skill_dir = write_model_test_skill_to_root(
            &runtime_root.join("skills"),
            "model-skill",
            "return function(args)\n  local result = vulcan.models.embed(\"hello\")\n  return vulcan.json.encode(result)\nend\n",
        );
        let mut engine = make_runtime_test_engine();
        engine
            .load_from_roots(&[RuntimeSkillRoot {
                name: "ROOT".to_string(),
                skills_dir: runtime_root.join("skills"),
            }])
            .expect("load model embed test skill");
        let invocation_context = crate::runtime_options::LuaInvocationContext::new(
            Some(RuntimeRequestContext {
                request_id: Some("req-embed-1".to_string()),
                client_name: Some("Codex Desktop".to_string()),
                transport_name: Some("mcp".to_string()),
                session_id: Some("session-embed".to_string()),
                client_info: Some(RuntimeClientInfo {
                    kind: Some("desktop".to_string()),
                    name: Some("Codex Desktop".to_string()),
                    version: Some("test".to_string()),
                }),
                client_capabilities: json!({"models": true}),
            }),
            json!({"budget": "test"}),
            json!({"tool": "config"}),
        );
        let result = engine
            .call_skill("model-skill-ping", &json!({}), Some(&invocation_context))
            .expect("call model embed skill");
        let result_json: Value =
            serde_json::from_str(&result.content).expect("parse embed result json");
        let captured = captured_request
            .lock()
            .expect("lock captured embed request")
            .clone()
            .expect("embed request captured");

        assert_eq!(result_json["ok"], true);
        assert_eq!(result_json["vector"], json!([0.25, 0.5, 0.75]));
        assert_eq!(result_json["dimensions"], 3);
        assert_eq!(result_json["usage"]["input_tokens"], 2);
        assert_eq!(captured.text, "hello");
        assert_eq!(captured.caller.skill_id.as_deref(), Some("model-skill"));
        assert_eq!(captured.caller.entry_name.as_deref(), Some("ping"));
        assert_eq!(
            captured.caller.canonical_tool_name.as_deref(),
            Some("model-skill-ping")
        );
        assert_eq!(captured.caller.root_name.as_deref(), Some("ROOT"));
        assert_eq!(
            captured.caller.skill_dir.as_deref(),
            Some(skill_dir.to_string_lossy().as_ref())
        );
        assert_eq!(
            captured.caller.client_name.as_deref(),
            Some("Codex Desktop")
        );
        assert_eq!(captured.caller.request_id.as_deref(), Some("req-embed-1"));

        let _ = fs::remove_dir_all(&runtime_root);
    }

    /// Verify registered LLM callbacks receive prompts and full caller context.
    /// 验证已注册的 LLM 回调会收到提示词和完整调用方上下文。
    #[test]
    fn vulcan_models_llm_dispatches_registered_callback_with_context() {
        let _guard = runtime_model_callback_test_guard();
        let captured_request: Arc<Mutex<Option<RuntimeModelLlmRequest>>> =
            Arc::new(Mutex::new(None));
        let captured_request_for_callback = captured_request.clone();
        set_model_llm_callback(Some(Arc::new(move |request| {
            *captured_request_for_callback
                .lock()
                .expect("lock captured llm request") = Some(request.clone());
            Ok(RuntimeModelLlmResponse {
                assistant: "assistant text".to_string(),
                usage: Some(RuntimeModelUsage {
                    input_tokens: Some(5),
                    output_tokens: Some(7),
                }),
            })
        })));

        let engine = make_runtime_test_engine();
        let has_llm = engine
            .run_lua("return vulcan.models.has('llm')", &json!({}), None)
            .expect("run has llm lua");
        assert_eq!(has_llm, json!(true));

        let runtime_root = make_temp_runtime_root("model-llm-context");
        if runtime_root.exists() {
            let _ = fs::remove_dir_all(&runtime_root);
        }
        create_runtime_test_layout(&runtime_root);
        let skill_dir = write_model_test_skill_to_root(
            &runtime_root.join("skills"),
            "llm-skill",
            "return function(args)\n  local result = vulcan.models.llm(\"system\", \"user\")\n  return vulcan.json.encode(result)\nend\n",
        );
        let mut engine = make_runtime_test_engine();
        engine
            .load_from_roots(&[RuntimeSkillRoot {
                name: "ROOT".to_string(),
                skills_dir: runtime_root.join("skills"),
            }])
            .expect("load model llm test skill");
        let result = engine
            .call_skill("llm-skill-ping", &json!({}), None)
            .expect("call model llm skill");
        let result_json: Value =
            serde_json::from_str(&result.content).expect("parse llm result json");
        let captured = captured_request
            .lock()
            .expect("lock captured llm request")
            .clone()
            .expect("llm request captured");

        assert_eq!(result_json["ok"], true);
        assert_eq!(result_json["assistant"], "assistant text");
        assert_eq!(result_json["usage"]["input_tokens"], 5);
        assert_eq!(result_json["usage"]["output_tokens"], 7);
        assert_eq!(captured.system, "system");
        assert_eq!(captured.user, "user");
        assert_eq!(captured.caller.skill_id.as_deref(), Some("llm-skill"));
        assert_eq!(captured.caller.entry_name.as_deref(), Some("ping"));
        assert_eq!(
            captured.caller.canonical_tool_name.as_deref(),
            Some("llm-skill-ping")
        );
        assert_eq!(captured.caller.root_name.as_deref(), Some("ROOT"));
        assert_eq!(
            captured.caller.skill_dir.as_deref(),
            Some(skill_dir.to_string_lossy().as_ref())
        );

        let _ = fs::remove_dir_all(&runtime_root);
    }

    /// Verify callback errors preserve standard codes and provider raw fields.
    /// 验证回调错误会保留标准错误码和 provider 原始字段。
    #[test]
    fn vulcan_models_wrap_callback_errors_and_provider_fields() {
        let _guard = runtime_model_callback_test_guard();
        set_model_embed_callback(Some(Arc::new(|_| {
            Err(RuntimeModelError {
                code: RuntimeModelErrorCode::ProviderError,
                message: "provider failed".to_string(),
                provider_message: Some("raw provider message".to_string()),
                provider_code: Some("model_not_found".to_string()),
                provider_status: Some(400),
            })
        })));
        set_model_llm_callback(Some(Arc::new(|_| {
            Err(RuntimeModelError {
                code: RuntimeModelErrorCode::Timeout,
                message: "llm timed out".to_string(),
                provider_message: None,
                provider_code: None,
                provider_status: None,
            })
        })));

        let engine = make_runtime_test_engine();
        let result = engine
            .run_lua(
                r#"
local embed = vulcan.models.embed("hello")
local llm = vulcan.models.llm("system", "user")
return {
  embed_ok = embed.ok,
  embed_code = embed.error.code,
  embed_message = embed.error.message,
  provider_message = embed.error.provider_message,
  provider_code = embed.error.provider_code,
  provider_status = embed.error.provider_status,
  llm_ok = llm.ok,
  llm_code = llm.error.code,
  llm_message = llm.error.message,
}
"#,
                &json!({}),
                None,
            )
            .expect("run model error wrapping lua");

        assert_eq!(result["embed_ok"], false);
        assert_eq!(result["embed_code"], "provider_error");
        assert_eq!(result["embed_message"], "provider failed");
        assert_eq!(result["provider_message"], "raw provider message");
        assert_eq!(result["provider_code"], "model_not_found");
        assert_eq!(result["provider_status"], 400);
        assert_eq!(result["llm_ok"], false);
        assert_eq!(result["llm_code"], "timeout");
        assert_eq!(result["llm_message"], "llm timed out");
    }

    /// Verify `vulcan.host.*` returns safe defaults when no host callback is registered.
    /// 验证未注册宿主回调时 `vulcan.host.*` 会返回安全默认值。
    #[test]
    fn vulcan_host_bridge_defaults_without_callback() {
        let _guard = host_tool_callback_test_guard();
        let engine = make_runtime_test_engine();
        let result = engine
            .run_lua(
                r#"
local tools = vulcan.host.list()
local called = vulcan.host.call("model.embed", {})
return {
  list_len = #tools,
  has = vulcan.host.has("model.embed"),
  has_tool = vulcan.host.has_tool("model.embed"),
  call_ok = called.ok,
  call_code = called.error.code,
}
"#,
                &json!({}),
                None,
            )
            .expect("run host bridge default lua");

        assert_eq!(result["list_len"], 0);
        assert_eq!(result["has"], false);
        assert_eq!(result["has_tool"], false);
        assert_eq!(result["call_ok"], false);
        assert_eq!(result["call_code"], "host_tool_callback_missing");
    }

    /// Verify `vulcan.host.*` dispatches list, has, and call requests through the host callback.
    /// 验证 `vulcan.host.*` 会通过宿主回调分发 list、has 与 call 请求。
    #[test]
    fn vulcan_host_bridge_dispatches_registered_callback() {
        let _guard = host_tool_callback_test_guard();
        set_host_tool_callback(Some(Arc::new(|request| match request.action {
            RuntimeHostToolAction::List => Ok(json!([
                {
                    "name": "model.echo",
                    "description": "Echo test host tool",
                    "input_schema": {
                        "type": "object",
                    },
                }
            ])),
            RuntimeHostToolAction::Has => {
                Ok(json!(request.tool_name.as_deref() == Some("model.echo")))
            }
            RuntimeHostToolAction::Call => {
                let tool_name = request.tool_name.as_deref().unwrap_or_default();
                if tool_name != "model.echo" {
                    return Err(format!("host tool not found: {}", tool_name));
                }
                Ok(json!({
                    "ok": true,
                    "value": {
                        "echo": request.args["text"].clone(),
                    },
                    "meta": {
                        "tool": tool_name,
                    },
                }))
            }
        })));

        let engine = make_runtime_test_engine();
        let result = engine
            .run_lua(
                r#"
local tools = vulcan.host.list()
local called = vulcan.host.call("model.echo", { text = "hello" })
return {
  first = tools[1].name,
  has = vulcan.host.has("model.echo"),
  missing = vulcan.host.has_tool("missing.tool"),
  ok = called.ok,
  echo = called.value.echo,
  tool = called.meta.tool,
}
"#,
                &json!({}),
                None,
            )
            .expect("run host bridge callback lua");

        assert_eq!(result["first"], "model.echo");
        assert_eq!(result["has"], true);
        assert_eq!(result["missing"], false);
        assert_eq!(result["ok"], true);
        assert_eq!(result["echo"], "hello");
        assert_eq!(result["tool"], "model.echo");
    }

    /// Verify `vulcan.host.call` converts callback failures into table error envelopes.
    /// 验证 `vulcan.host.call` 会把回调失败转换为 table 错误包络。
    #[test]
    fn vulcan_host_call_wraps_callback_errors() {
        let _guard = host_tool_callback_test_guard();
        set_host_tool_callback(Some(Arc::new(|request| match request.action {
            RuntimeHostToolAction::List => Ok(json!([])),
            RuntimeHostToolAction::Has => Ok(json!(true)),
            RuntimeHostToolAction::Call => {
                assert!(request.args.as_object().is_some());
                assert!(request.args.as_object().unwrap().is_empty());
                Err("model provider failed".to_string())
            }
        })));

        let engine = make_runtime_test_engine();
        let result = engine
            .run_lua(
                r#"
local called = vulcan.host.call("model.fail", {})
return {
  ok = called.ok,
  code = called.error.code,
  message = called.error.message,
}
"#,
                &json!({}),
                None,
            )
            .expect("run host bridge callback error lua");

        assert_eq!(result["ok"], false);
        assert_eq!(result["code"], "host_tool_callback_error");
        assert_eq!(result["message"], "model provider failed");
    }

    /// Assert that one pooled Lua VM has returned to the neutral request baseline.
    /// 断言单个池化 Lua 虚拟机已经回到中性的请求基线状态。
    fn assert_vm_scope_is_clean(lua: &mlua::Lua) {
        let context = get_vulcan_context_table(lua).expect("get vulcan.context");
        let request: Table = context.get("request").expect("get request table");
        assert_eq!(request.raw_len(), 0);
        assert_eq!(request.pairs::<String, LuaValue>().count(), 0);
        assert!(matches!(
            context
                .get::<LuaValue>("client_info")
                .expect("get client_info"),
            LuaValue::Nil
        ));
        assert!(matches!(
            context
                .get::<LuaValue>("client_capabilities")
                .expect("get client_capabilities"),
            LuaValue::Table(_)
        ));
        assert!(matches!(
            context
                .get::<LuaValue>("client_budget")
                .expect("get client_budget"),
            LuaValue::Table(_)
        ));
        assert!(matches!(
            context
                .get::<LuaValue>("tool_config")
                .expect("get tool_config"),
            LuaValue::Table(_)
        ));
        assert!(matches!(
            context.get::<LuaValue>("skill_dir").expect("get skill_dir"),
            LuaValue::Nil
        ));
        assert!(matches!(
            context.get::<LuaValue>("entry_dir").expect("get entry_dir"),
            LuaValue::Nil
        ));
        assert!(matches!(
            context
                .get::<LuaValue>("entry_file")
                .expect("get entry_file"),
            LuaValue::Nil
        ));

        let deps = get_vulcan_deps_table(lua).expect("get vulcan.deps");
        assert!(matches!(
            deps.get::<LuaValue>("tools_path").expect("get tools_path"),
            LuaValue::Nil
        ));
        assert!(matches!(
            deps.get::<LuaValue>("lua_path").expect("get lua_path"),
            LuaValue::Nil
        ));
        assert!(matches!(
            deps.get::<LuaValue>("ffi_path").expect("get ffi_path"),
            LuaValue::Nil
        ));

        let internal = get_vulcan_runtime_internal_table(lua).expect("get runtime internal");
        assert!(matches!(
            internal
                .get::<LuaValue>("tool_name")
                .expect("get tool_name"),
            LuaValue::Nil
        ));
        assert!(matches!(
            internal
                .get::<LuaValue>("skill_name")
                .expect("get skill_name"),
            LuaValue::Nil
        ));
        assert!(matches!(
            internal
                .get::<LuaValue>("entry_name")
                .expect("get entry_name"),
            LuaValue::Nil
        ));
        assert!(matches!(
            internal
                .get::<LuaValue>("root_name")
                .expect("get root_name"),
            LuaValue::Nil
        ));
        assert!(
            !internal
                .get::<bool>("luaexec_active")
                .expect("get luaexec_active")
        );
        assert!(matches!(
            internal
                .get::<LuaValue>("luaexec_caller_tool_name")
                .expect("get luaexec_caller_tool_name"),
            LuaValue::Nil
        ));

        let vulcan = get_vulcan_table(lua).expect("get vulcan");
        let lancedb: Table = vulcan.get("lancedb").expect("get lancedb");
        assert!(!lancedb.get::<bool>("enabled").expect("get lancedb enabled"));
        let sqlite: Table = vulcan.get("sqlite").expect("get sqlite");
        assert!(!sqlite.get::<bool>("enabled").expect("get sqlite enabled"));
        assert!(matches!(
            lua.globals()
                .get::<LuaValue>("__runlua_args")
                .expect("get __runlua_args"),
            LuaValue::Nil
        ));
    }

    /// Verify that skill manifests must not declare skill_id explicitly.
    /// 验证 skill 清单不允许再显式声明 skill_id 字段。
    #[test]
    fn load_from_roots_rejects_explicit_skill_id_field() {
        let temp_root = std::env::temp_dir().join(format!(
            "luaskills_reject_skill_id_test_{}",
            std::process::id()
        ));
        if temp_root.exists() {
            let _ = fs::remove_dir_all(&temp_root);
        }
        let skill_root = temp_root.join("skills");
        let skill_dir = skill_root.join("vulcan-codekit");
        fs::create_dir_all(skill_dir.join("runtime")).expect("create runtime dir");
        fs::write(
            skill_dir.join("skill.yaml"),
            "name: vulcan-codekit\nversion: 0.1.0\nskill_id: vulcan-codekit\nentries:\n  - name: ast-tree\n    lua_entry: runtime/test.lua\n    lua_module: vulcan-codekit.ast-tree\n",
        )
        .expect("write skill yaml");
        fs::write(skill_dir.join("runtime").join("test.lua"), "return 'ok'\n")
            .expect("write runtime entry");

        let mut engine = LuaEngine::new(LuaEngineOptions {
            host_options: LuaRuntimeHostOptions::default(),
            pool_config: LuaVmPoolConfig {
                min_size: 1,
                max_size: 1,
                idle_ttl_secs: 60,
            },
        })
        .expect("create engine");

        let error = engine
            .load_from_roots(&[crate::host::options::RuntimeSkillRoot {
                name: "ROOT".to_string(),
                skills_dir: skill_root,
            }])
            .expect_err("explicit skill_id should be rejected");
        let rendered = error.to_string();
        assert!(rendered.contains("must not declare skill_id"));

        let _ = fs::remove_dir_all(&temp_root);
    }

    /// Verify that host-ignored skills are skipped before dependency, database, or entry setup.
    /// 验证宿主忽略的 skill 会在依赖、数据库与入口初始化之前被跳过。
    #[test]
    fn load_from_roots_skips_host_ignored_skill_before_resource_setup() {
        let temp_root = std::env::temp_dir().join(format!(
            "luaskills_ignored_skill_test_{}",
            std::process::id()
        ));
        if temp_root.exists() {
            let _ = fs::remove_dir_all(&temp_root);
        }
        let skill_root = temp_root.join("skills");
        let skill_dir = skill_root.join("grpc-memory");
        fs::create_dir_all(skill_dir.join("runtime")).expect("create runtime dir");
        fs::write(
            skill_dir.join("skill.yaml"),
            "name: grpc-memory\nversion: 0.1.0\nenable: true\ndebug: false\nsqlite:\n  enable: true\nlancedb:\n  enable: true\nentries:\n  - name: remember\n    lua_entry: runtime/remember.lua\n    lua_module: grpc-memory.remember\n",
        )
        .expect("write skill yaml");
        fs::write(
            skill_dir.join("runtime").join("remember.lua"),
            "return function(args)\n  return 'unexpected-load'\nend\n",
        )
        .expect("write runtime entry");

        let mut host_options = LuaRuntimeHostOptions::default();
        host_options.dependency_dir_name = "dependencies".to_string();
        host_options.state_dir_name = "state".to_string();
        host_options.database_dir_name = "databases".to_string();
        host_options.ignored_skill_ids = vec!["grpc-memory".to_string()];
        let mut engine = LuaEngine::new(LuaEngineOptions {
            host_options,
            pool_config: LuaVmPoolConfig {
                min_size: 1,
                max_size: 1,
                idle_ttl_secs: 60,
            },
        })
        .expect("create engine");

        engine
            .load_from_roots(&[crate::host::options::RuntimeSkillRoot {
                name: "ROOT".to_string(),
                skills_dir: skill_root,
            }])
            .expect("ignored skill should not fail loading");

        assert!(engine.skills.is_empty());
        assert!(engine.entry_registry.is_empty());
        assert!(!temp_root.join("dependencies").exists());
        assert!(!temp_root.join("state").exists());
        assert!(!temp_root.join("databases").exists());

        let _ = fs::remove_dir_all(&temp_root);
    }

    /// Verify that colliding `skill-entry` names receive deterministic numeric suffixes.
    /// 验证发生冲突的 `skill-entry` 名称会收到稳定且可预测的数字后缀。
    #[test]
    fn rebuild_entry_registry_appends_numeric_suffixes_for_collisions() {
        let mut skills = HashMap::new();
        skills.insert(
            "alpha".to_string(),
            make_loaded_skill("alpha", "foo-bar", "baz", "alpha_module"),
        );
        skills.insert(
            "beta".to_string(),
            make_loaded_skill("beta", "foo", "bar-baz", "beta_module"),
        );
        skills.insert(
            "gamma".to_string(),
            make_loaded_skill("gamma", "foo-bar", "baz", "gamma_module"),
        );

        let mut engine = make_test_engine(skills);
        engine
            .rebuild_entry_registry()
            .expect("entry registry should rebuild successfully");

        assert!(engine.entry_registry.contains_key("foo-bar-baz"));
        assert!(engine.entry_registry.contains_key("foo-bar-baz-2"));
        assert!(engine.entry_registry.contains_key("foo-bar-baz-3"));

        let alpha_skill = engine
            .skills
            .get("alpha")
            .expect("alpha skill should exist");
        let beta_skill = engine.skills.get("beta").expect("beta skill should exist");
        let gamma_skill = engine
            .skills
            .get("gamma")
            .expect("gamma skill should exist");

        assert_eq!(alpha_skill.resolved_tool_name("baz"), Some("foo-bar-baz"));
        assert_eq!(
            beta_skill.resolved_tool_name("bar-baz"),
            Some("foo-bar-baz-2")
        );
        assert_eq!(gamma_skill.resolved_tool_name("baz"), Some("foo-bar-baz-3"));
    }

    /// Verify that host-reserved public tool names are treated as occupied during canonical-name generation.
    /// 验证宿主保留的公开工具名称会在 canonical 名称生成阶段被视为已占用名称。
    #[test]
    fn rebuild_entry_registry_skips_host_reserved_names() {
        let mut skills = HashMap::new();
        skills.insert(
            "alpha".to_string(),
            make_loaded_skill("alpha", "vulcan", "help-list", "alpha_module"),
        );

        let mut engine = make_test_engine(skills);
        Arc::get_mut(&mut engine.host_options)
            .expect("host options should be uniquely owned in test")
            .reserved_entry_names = vec!["vulcan-help-list".to_string()];

        engine
            .rebuild_entry_registry()
            .expect("entry registry should rebuild successfully");

        assert!(!engine.entry_registry.contains_key("vulcan-help-list"));
        assert!(engine.entry_registry.contains_key("vulcan-help-list-2"));

        let alpha_skill = engine
            .skills
            .get("alpha")
            .expect("alpha skill should exist");
        assert_eq!(
            alpha_skill.resolved_tool_name("help-list"),
            Some("vulcan-help-list-2")
        );
    }

    /// Verify that the pooled VM scope guard clears request state even when setup exits early.
    /// 验证池化虚拟机作用域守卫即使在安装阶段提前退出也会清理请求状态。
    #[test]
    fn pooled_vm_scope_guard_cleans_state_after_early_exit() {
        let engine = make_runtime_test_engine();
        let scope_result: Result<(), String> = (|| {
            let mut lease = engine.acquire_vm()?;
            let _scope_guard =
                LuaVmRequestScopeGuard::new(&mut lease, engine.host_options.as_ref())?;
            let lua = _scope_guard.lua();
            LuaEngine::populate_vulcan_request_context(
                lua,
                Some(&crate::runtime_options::LuaInvocationContext::new(
                    None,
                    json!({"budget":"test"}),
                    json!({"tool":"config"}),
                )),
            )?;
            populate_vulcan_internal_execution_context(
                lua,
                &VulcanInternalExecutionContext {
                    tool_name: Some("test-tool".to_string()),
                    skill_name: Some("test-skill".to_string()),
                    entry_name: Some("test".to_string()),
                    root_name: Some("ROOT".to_string()),
                    luaexec_active: false,
                    luaexec_caller_tool_name: None,
                },
            )?;
            let skill_dir = Path::new("D:/runtime-test-root/skills/test-skill");
            let entry_file = Path::new("D:/runtime-test-root/skills/test-skill/runtime/test.lua");
            populate_vulcan_file_context(lua, Some(skill_dir), Some(entry_file))?;
            populate_vulcan_dependency_context(
                lua,
                engine.host_options.as_ref(),
                Some(skill_dir),
                Some("test-skill"),
            )?;
            lua.globals()
                .set(
                    "__runlua_args",
                    json_to_lua_table(lua, &json!({"stale":"value"}))
                        .expect("build runlua args table"),
                )
                .expect("set stale runlua args");
            Err("simulated setup failure".to_string())
        })();
        assert_eq!(
            scope_result.expect_err("scope should fail"),
            "simulated setup failure"
        );

        let lease = engine.acquire_vm().expect("reacquire pooled vm");
        assert_vm_scope_is_clean(lease.lua());
    }

    /// Verify that a pooled VM with broken core tables is discarded before it can be reused.
    /// 验证当池化虚拟机的核心表被破坏时，该实例会在复用前被直接丢弃。
    #[test]
    fn pooled_vm_scope_guard_discards_vm_when_entry_reset_fails() {
        let engine = make_runtime_test_engine();
        {
            let lease = engine.acquire_vm().expect("borrow pooled vm");
            let vulcan = get_vulcan_table(lease.lua()).expect("get vulcan");
            vulcan
                .set("context", LuaValue::Nil)
                .expect("break vulcan.context");
        }

        let mut broken_lease = engine.acquire_vm().expect("reacquire broken pooled vm");
        let error =
            match LuaVmRequestScopeGuard::new(&mut broken_lease, engine.host_options.as_ref()) {
                Ok(_) => panic!("broken pooled vm should fail normalization"),
                Err(error) => error,
            };
        assert!(error.contains("vulcan.context"));

        let mut fresh_lease = engine.acquire_vm().expect("borrow fresh pooled vm");
        let fresh_scope =
            LuaVmRequestScopeGuard::new(&mut fresh_lease, engine.host_options.as_ref())
                .expect("normalize fresh pooled vm");
        assert_vm_scope_is_clean(fresh_scope.lua());
    }

    /// Verify that cleanup failures retire the current pooled VM instead of returning dirty state.
    /// 验证当清理阶段失败时，当前池化虚拟机会被退役，而不是带着脏状态返回池中。
    #[test]
    fn pooled_vm_scope_guard_discards_vm_when_exit_reset_fails() {
        let engine = make_runtime_test_engine();
        let mut lease = engine.acquire_vm().expect("borrow pooled vm");
        let scope_guard = LuaVmRequestScopeGuard::new(&mut lease, engine.host_options.as_ref())
            .expect("normalize pooled vm");
        let vulcan = get_vulcan_table(scope_guard.lua()).expect("get vulcan");
        vulcan
            .set("context", LuaValue::Nil)
            .expect("break vulcan.context");
        let error = scope_guard
            .finish()
            .expect_err("cleanup should fail after context corruption");
        assert!(error.contains("vulcan.context"));

        let mut fresh_lease = engine.acquire_vm().expect("borrow fresh pooled vm");
        let fresh_scope =
            LuaVmRequestScopeGuard::new(&mut fresh_lease, engine.host_options.as_ref())
                .expect("normalize fresh pooled vm");
        assert_vm_scope_is_clean(fresh_scope.lua());
    }

    /// Verify that run_lua clears transient args after one successful execution.
    /// 验证 run_lua 在成功执行后会清理临时参数状态。
    #[test]
    fn run_lua_clears_args_after_success() {
        let engine = make_runtime_test_engine();
        let result = engine
            .run_lua("return args.value", &json!({"value":"hello"}), None)
            .expect("run_lua should succeed");
        assert_eq!(result, json!("hello"));

        let lease = engine.acquire_vm().expect("reacquire pooled vm");
        assert_vm_scope_is_clean(lease.lua());
    }

    /// Verify isolated `vulcan.runtime.lua.exec` calls reuse the dedicated runlua VM pool.
    /// 验证隔离 `vulcan.runtime.lua.exec` 调用会复用独立的 runlua 虚拟机池。
    #[test]
    fn execute_runlua_request_inline_reuses_dedicated_pool() {
        let engine = make_runtime_test_engine();
        assert_eq!(engine.runlua_pool.total_count(), 0);

        let first = engine
            .execute_runlua_request_json_inline(r#"{"code":"return 1"}"#)
            .expect("first inline runlua should succeed");
        assert!(!first.trim().is_empty());
        assert_eq!(engine.runlua_pool.total_count(), 1);

        let second = engine
            .execute_runlua_request_json_inline(r#"{"code":"return 2"}"#)
            .expect("second inline runlua should succeed");
        assert!(!second.trim().is_empty());
        assert_eq!(engine.runlua_pool.total_count(), 1);
    }

    /// Verify isolated runlua redirects Lua `io.open` to the Rust-backed managed IO table.
    /// 验证隔离 runlua 会把 Lua `io.open` 重定向到 Rust 托管 IO 表。
    #[test]
    fn execute_runlua_request_inline_uses_managed_io_open() {
        let engine = make_runtime_test_engine();
        let path = std::env::temp_dir().join(format!(
            "luaskills_runlua_managed_io_{}.txt",
            std::process::id()
        ));
        fs::write(&path, "managed-io-ok").expect("write managed io test file");
        let request = json!({
            "code": "local f = io.open(args.path, 'r'); local value = f:read('*a'); f:close(); return value",
            "args": {
                "path": path.to_string_lossy().to_string()
            }
        });

        let result = engine
            .execute_runlua_request_json_inline(&request.to_string())
            .expect("inline runlua should read through managed io");

        assert!(result.contains("SUCCESS"));
        assert!(result.contains("managed-io-ok"));
        let _ = fs::remove_file(path);
    }

    /// Verify isolated runlua supports default managed `io.input` and `io.read`.
    /// 验证隔离 runlua 支持托管默认 `io.input` 与 `io.read`。
    #[test]
    fn execute_runlua_request_inline_uses_managed_io_default_input() {
        let engine = make_runtime_test_engine();
        let path = std::env::temp_dir().join(format!(
            "luaskills_runlua_managed_io_input_{}.txt",
            std::process::id()
        ));
        fs::write(&path, "managed-default-input").expect("write managed input test file");
        let request = json!({
            "code": "io.input(args.path); return io.read('*a')",
            "args": {
                "path": path.to_string_lossy().to_string()
            }
        });

        let result = engine
            .execute_runlua_request_json_inline(&request.to_string())
            .expect("inline runlua should read through managed default input");

        assert!(result.contains("SUCCESS"));
        assert!(result.contains("managed-default-input"));
        let _ = fs::remove_file(path);
    }

    /// Verify isolated runlua supports default managed `io.output` and `io.write`.
    /// 验证隔离 runlua 支持托管默认 `io.output` 与 `io.write`。
    #[test]
    fn execute_runlua_request_inline_uses_managed_io_default_output() {
        let engine = make_runtime_test_engine();
        let path = std::env::temp_dir().join(format!(
            "luaskills_runlua_managed_io_output_{}.txt",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        let request = json!({
            "code": "io.output(args.path); io.write('managed', '-', 'default-output'); io.close(); return vulcan.io.read_text(args.path, { encoding = 'utf-8' })",
            "args": {
                "path": path.to_string_lossy().to_string()
            }
        });

        let result = engine
            .execute_runlua_request_json_inline(&request.to_string())
            .expect("inline runlua should write through managed default output");

        assert!(result.contains("SUCCESS"));
        assert!(result.contains("managed-default-output"));
        let _ = fs::remove_file(path);
    }

    /// Verify isolated runlua redirects Lua `io.popen` to the Rust-backed read implementation.
    /// 验证隔离 runlua 会把 Lua `io.popen` 重定向到 Rust 托管读取实现。
    #[test]
    fn execute_runlua_request_inline_uses_managed_io_popen() {
        let engine = make_runtime_test_engine();
        let result = engine
            .execute_runlua_request_json_inline(
                r#"{"code":"local f = io.popen('echo managed-popen-ok', 'r'); local value = f:read('*a'); local ok = f:close(); return value, ok"}"#,
            )
            .expect("inline runlua should read through managed io.popen");

        assert!(result.contains("SUCCESS"));
        assert!(result.contains("managed-popen-ok"));
        assert!(result.contains("true"));
    }

    /// Verify isolated runlua rejects the unsupported managed `io.popen` write mode.
    /// 验证隔离 runlua 会拒绝暂不支持的托管 `io.popen` 写入模式。
    #[test]
    fn execute_runlua_request_inline_rejects_io_popen_write_mode() {
        let engine = make_runtime_test_engine();
        let result = engine
            .execute_runlua_request_json_inline(r#"{"code":"return io.popen('echo hello', 'w')"}"#)
            .expect("inline runlua should render the managed io.popen mode error");

        assert!(result.contains("FAILED"));
        assert!(result.contains("write mode is not implemented yet"));
    }

    /// Verify host default text encoding is used by managed IO when Lua omits encoding options.
    /// 验证 Lua 省略编码选项时托管 IO 会使用宿主默认文本编码。
    #[test]
    fn execute_runlua_request_inline_uses_host_default_text_encoding() {
        let mut host_options = LuaRuntimeHostOptions::default();
        host_options.default_text_encoding = Some("gb18030".to_string());
        let engine = make_runtime_test_engine_with_host_options(host_options);
        let path = std::env::temp_dir().join(format!(
            "luaskills_runlua_default_encoding_{}.txt",
            std::process::id()
        ));
        let bytes = encode_runtime_text("宿主默认编码", RuntimeTextEncoding::Gb18030)
            .expect("encode host default gb18030 test file");
        fs::write(&path, bytes).expect("write host default encoding file");
        let request = json!({
            "code": "return vulcan.io.read_text(args.path)",
            "args": {
                "path": path.to_string_lossy().to_string()
            }
        });

        let result = engine
            .execute_runlua_request_json_inline(&request.to_string())
            .expect("inline runlua should read through host default encoding");

        assert!(result.contains("SUCCESS"));
        assert!(result.contains("宿主默认编码"));
        let _ = fs::remove_file(path);
    }

    /// Verify hosts can disable the managed global `io` compatibility layer for luaexec.
    /// 验证宿主可以为 luaexec 关闭托管全局 `io` 兼容层。
    #[test]
    fn execute_runlua_request_inline_can_disable_managed_io_compat() {
        let mut host_options = LuaRuntimeHostOptions::default();
        host_options.capabilities.enable_managed_io_compat = false;
        let engine = make_runtime_test_engine_with_host_options(host_options);
        let result = engine
            .execute_runlua_request_json_inline(
                r#"{"code":"local preload = package and package.preload and package.preload.io; return type(preload) == 'function' and 'managed' or 'native'"}"#,
            )
            .expect("inline runlua should keep native io when managed compat is disabled");

        assert!(result.contains("SUCCESS"));
        assert!(result.contains("native"));
    }

    /// Verify `vulcan.process.exec` exposes explicit encoding metadata after byte-based capture.
    /// 验证 `vulcan.process.exec` 在按字节捕获后会暴露明确的编码元数据。
    #[test]
    fn execute_runlua_request_inline_reports_process_exec_encoding_metadata() {
        let engine = make_runtime_test_engine();
        let result = engine
            .execute_runlua_request_json_inline(
                r#"{"code":"local info = vulcan.os.info(); local spec; if info.os == 'windows' then spec = { program = 'cmd', args = { '/C', 'echo exec-encoding-ok' }, encoding = 'utf-8' } else spec = { program = 'sh', args = { '-c', 'printf exec-encoding-ok' }, encoding = 'utf-8' } end; local result = vulcan.process.exec(spec); return result.stdout, result.stdout_encoding, result.stdout_lossy"}"#,
            )
            .expect("inline runlua should execute process.exec");

        assert!(result.contains("SUCCESS"));
        assert!(result.contains("exec-encoding-ok"));
        assert!(result.contains("utf-8"));
        assert!(result.contains("false"));
    }

    /// Verify `vulcan.process.session` can write to stdin and read captured stdout.
    /// 验证 `vulcan.process.session` 可以写入 stdin 并读取捕获的 stdout。
    #[test]
    fn execute_runlua_request_inline_uses_process_session_write_read() {
        let engine = make_runtime_test_engine();
        let result = engine
            .execute_runlua_request_json_inline(
                r#"{"code":"local info = vulcan.os.info(); local spec; if info.os == 'windows' then spec = { program = 'cmd', args = { '/V:ON', '/C', 'set /P line=&echo session:!line!' }, encoding = 'utf-8' } else spec = { program = 'sh', args = { '-c', 'read line; echo session:$line' }, encoding = 'utf-8' } end; local session = vulcan.process.session.open(spec); session:write('ok\\n'); local status = session:close({ timeout_ms = 3000 }); local output = session:read({ timeout_ms = 3000 }); return output.stdout, status.exited, status.success"}"#,
            )
            .expect("inline runlua should exercise process session");

        assert!(result.contains("SUCCESS"));
        assert!(result.contains("session:ok"));
        assert!(result.contains("true"));
    }

    /// Verify persistent runtime sessions keep Lua VM globals across eval calls.
    /// 验证持久运行时会话会在多次 eval 调用之间保留 Lua VM 全局状态。
    #[test]
    fn runtime_session_eval_preserves_vm_state_across_calls() {
        let engine = make_runtime_test_engine();
        let created: Value = serde_json::from_str(
            &engine
                .create_runtime_session_json(r#"{"sid":"stateful-test","ttl_sec":60}"#)
                .expect("create runtime session"),
        )
        .expect("create response json");
        assert_eq!(created["ok"], true);
        let lease_id = created["lease_id"]
            .as_str()
            .expect("lease id should be present")
            .to_string();

        let first_request = json!({
            "lease_id": lease_id,
            "code": "counter = (counter or 0) + 1; return counter"
        });
        let first: Value = serde_json::from_str(
            &engine
                .eval_runtime_session_json(&first_request.to_string())
                .expect("first runtime session eval"),
        )
        .expect("first eval response json");
        assert_eq!(first["ok"], true);
        assert_eq!(first["result"], json!(1));

        let second_request = json!({
            "lease_id": lease_id,
            "code": "counter = (counter or 0) + 1; return counter"
        });
        let second: Value = serde_json::from_str(
            &engine
                .eval_runtime_session_json(&second_request.to_string())
                .expect("second runtime session eval"),
        )
        .expect("second eval response json");
        assert_eq!(second["ok"], true);
        assert_eq!(second["result"], json!(2));
    }

    /// Verify closed runtime sessions return a stable lease_closed error.
    /// 验证已关闭的运行时会话会返回稳定的 lease_closed 错误。
    #[test]
    fn runtime_session_eval_reports_closed_lease() {
        let engine = make_runtime_test_engine();
        let created: Value = serde_json::from_str(
            &engine
                .create_runtime_session_json(r#"{"sid":"closed-test","ttl_sec":60}"#)
                .expect("create runtime session"),
        )
        .expect("create response json");
        let lease_id = created["lease_id"]
            .as_str()
            .expect("lease id should be present")
            .to_string();
        let close_request = json!({ "lease_id": lease_id });
        let closed: Value = serde_json::from_str(
            &engine
                .close_runtime_session_json(&close_request.to_string())
                .expect("close runtime session"),
        )
        .expect("close response json");
        assert_eq!(closed["ok"], true);
        assert_eq!(closed["closed"], true);

        let eval_request = json!({
            "lease_id": lease_id,
            "code": "return 1"
        });
        let eval: Value = serde_json::from_str(
            &engine
                .eval_runtime_session_json(&eval_request.to_string())
                .expect("eval closed runtime session"),
        )
        .expect("eval response json");
        assert_eq!(eval["ok"], false);
        assert_eq!(eval["error_code"], "lease_closed");
    }

    /// Verify closed runtime sessions return a stable lease_closed error from status.
    /// 验证已关闭的运行时会话在 status 中会返回稳定的 lease_closed 错误。
    #[test]
    fn runtime_session_status_reports_closed_lease() {
        let engine = make_runtime_test_engine();
        let created: Value = serde_json::from_str(
            &engine
                .create_runtime_session_json(r#"{"sid":"closed-status-test","ttl_sec":60}"#)
                .expect("create runtime session"),
        )
        .expect("create response json");
        let lease_id = created["lease_id"]
            .as_str()
            .expect("lease id should be present")
            .to_string();
        let close_request = json!({ "lease_id": lease_id.clone() });
        let closed: Value = serde_json::from_str(
            &engine
                .close_runtime_session_json(&close_request.to_string())
                .expect("close runtime session"),
        )
        .expect("close response json");
        assert_eq!(closed["ok"], true);

        let status_request = json!({ "lease_id": lease_id });
        let status: Value = serde_json::from_str(
            &engine
                .runtime_session_status_json(&status_request.to_string())
                .expect("status closed runtime session"),
        )
        .expect("status response json");
        assert_eq!(status["ok"], false);
        assert_eq!(status["error_code"], "lease_closed");
    }

    /// Verify replaced runtime sessions keep a stable lease_replaced terminal error.
    /// 验证被替换的运行时会话会保留稳定的 lease_replaced 终态错误。
    #[test]
    fn runtime_session_eval_reports_replaced_lease() {
        let engine = make_runtime_test_engine();
        let first_created: Value = serde_json::from_str(
            &engine
                .create_runtime_session_json(r#"{"sid":"replace-test","ttl_sec":60}"#)
                .expect("create first runtime session"),
        )
        .expect("first create response json");
        let first_lease_id = first_created["lease_id"]
            .as_str()
            .expect("first lease id should be present")
            .to_string();

        let second_created: Value = serde_json::from_str(
            &engine
                .create_runtime_session_json(
                    r#"{"sid":"replace-test","ttl_sec":60,"replace":true}"#,
                )
                .expect("create second runtime session"),
        )
        .expect("second create response json");
        assert_eq!(second_created["ok"], true);
        assert_ne!(second_created["lease_id"], first_created["lease_id"]);

        let eval_request = json!({
            "lease_id": first_lease_id,
            "code": "return 1"
        });
        let eval: Value = serde_json::from_str(
            &engine
                .eval_runtime_session_json(&eval_request.to_string())
                .expect("eval replaced runtime session"),
        )
        .expect("replaced eval response json");
        assert_eq!(eval["ok"], false);
        assert_eq!(eval["error_code"], "lease_replaced");
    }

    /// Verify replaced runtime sessions return a stable lease_replaced error from status.
    /// 验证被替换的运行时会话在 status 中会返回稳定的 lease_replaced 错误。
    #[test]
    fn runtime_session_status_reports_replaced_lease() {
        let engine = make_runtime_test_engine();
        let first_created: Value = serde_json::from_str(
            &engine
                .create_runtime_session_json(r#"{"sid":"replace-status-test","ttl_sec":60}"#)
                .expect("create first runtime session"),
        )
        .expect("first create response json");
        let first_lease_id = first_created["lease_id"]
            .as_str()
            .expect("first lease id should be present")
            .to_string();

        let second_created: Value = serde_json::from_str(
            &engine
                .create_runtime_session_json(
                    r#"{"sid":"replace-status-test","ttl_sec":60,"replace":true}"#,
                )
                .expect("create second runtime session"),
        )
        .expect("second create response json");
        assert_eq!(second_created["ok"], true);

        let status_request = json!({ "lease_id": first_lease_id });
        let status: Value = serde_json::from_str(
            &engine
                .runtime_session_status_json(&status_request.to_string())
                .expect("status replaced runtime session"),
        )
        .expect("status response json");
        assert_eq!(status["ok"], false);
        assert_eq!(status["error_code"], "lease_replaced");
    }

    /// Verify a stale runtime-session handle observes lease_replaced after another caller replaces the SID lease.
    /// 验证陈旧运行时会话句柄会在另一个调用方替换同 SID 租约后观察到 lease_replaced。
    #[test]
    fn runtime_session_stale_handle_reports_replaced_after_manager_get() {
        let engine = make_runtime_test_engine();
        let first_created: Value = serde_json::from_str(
            &engine
                .create_runtime_session_json(r#"{"sid":"replace-race-test","ttl_sec":60}"#)
                .expect("create first runtime session"),
        )
        .expect("first create response json");
        let first_lease_id = first_created["lease_id"]
            .as_str()
            .expect("first lease id should be present")
            .to_string();
        let stale_session = engine
            .runtime_sessions
            .get(&first_lease_id, None, None)
            .expect("capture stale runtime session handle");

        let replaced: Value = serde_json::from_str(
            &engine
                .create_runtime_session_json(
                    r#"{"sid":"replace-race-test","ttl_sec":60,"replace":true}"#,
                )
                .expect("replace runtime session"),
        )
        .expect("replace response json");
        assert_eq!(replaced["ok"], true);

        let mut stale_session = stale_session.lock().expect("lock stale runtime session");
        let error = LuaEngine::ensure_runtime_session_active(&mut stale_session)
            .expect_err("stale handle should fail");
        assert_eq!(error.code, "lease_replaced");
    }

    /// Verify replace=true rejects one busy lease before creating a second VM for the same SID.
    /// 验证 replace=true 会在同一 SID 的旧租约忙碌时拒绝替换，而不会创建第二个虚拟机。
    #[test]
    fn runtime_session_replace_rejects_busy_lease() {
        let engine = make_runtime_test_engine();
        let created: Value = serde_json::from_str(
            &engine
                .create_runtime_session_json(r#"{"sid":"busy-replace-test","ttl_sec":60}"#)
                .expect("create busy replace runtime session"),
        )
        .expect("busy replace create response json");
        let lease_id = created["lease_id"]
            .as_str()
            .expect("busy replace lease id should be present")
            .to_string();

        let session = engine
            .runtime_sessions
            .get(&lease_id, None, None)
            .expect("get busy replace runtime session");
        let guard = session.lock().expect("lock busy replace runtime session");

        let blocked_replace: Value = serde_json::from_str(
            &engine
                .create_runtime_session_json(
                    r#"{"sid":"busy-replace-test","ttl_sec":60,"replace":true}"#,
                )
                .expect("replace busy runtime session"),
        )
        .expect("busy replace response json");
        assert_eq!(blocked_replace["ok"], false);
        assert_eq!(blocked_replace["error_code"], "lease_busy");
        assert!(
            blocked_replace["message"]
                .as_str()
                .expect("busy replace message should be present")
                .contains("cannot replace busy lease")
        );

        let listed: Value = serde_json::from_str(
            &engine
                .list_runtime_sessions_json(r#"{"sid":"busy-replace-test"}"#)
                .expect("list busy replace runtime sessions"),
        )
        .expect("busy replace list response json");
        assert_eq!(listed["ok"], true);
        assert_eq!(listed["leases"].as_array().map(Vec::len), Some(1));
        assert_eq!(listed["leases"][0]["lease_id"], lease_id);

        drop(guard);

        let replaced: Value = serde_json::from_str(
            &engine
                .create_runtime_session_json(
                    r#"{"sid":"busy-replace-test","ttl_sec":60,"replace":true}"#,
                )
                .expect("replace idle runtime session"),
        )
        .expect("idle replace response json");
        assert_eq!(replaced["ok"], true);
        assert_ne!(replaced["lease_id"], created["lease_id"]);
    }

    /// Verify runtime sessions reject a mismatched echoed SID before executing code.
    /// 验证运行时会话会在执行前拒绝不匹配的回传 SID。
    #[test]
    fn runtime_session_eval_rejects_sid_mismatch() {
        let engine = make_runtime_test_engine();
        let created: Value = serde_json::from_str(
            &engine
                .create_runtime_session_json(r#"{"sid":"identity-test","ttl_sec":60}"#)
                .expect("create identity runtime session"),
        )
        .expect("identity create response json");
        let lease_id = created["lease_id"]
            .as_str()
            .expect("identity lease id should be present")
            .to_string();

        let eval_request = json!({
            "lease_id": lease_id,
            "sid": "wrong-sid",
            "code": "return 1"
        });
        let eval: Value = serde_json::from_str(
            &engine
                .eval_runtime_session_json(&eval_request.to_string())
                .expect("eval runtime session with wrong sid"),
        )
        .expect("wrong sid eval response json");
        assert_eq!(eval["ok"], false);
        assert_eq!(eval["error_code"], "lease_sid_mismatch");
    }

    /// Verify runtime sessions reject a mismatched echoed generation before executing code.
    /// 验证运行时会话会在执行前拒绝不匹配的回传 generation。
    #[test]
    fn runtime_session_eval_rejects_generation_mismatch() {
        let engine = make_runtime_test_engine();
        let created: Value = serde_json::from_str(
            &engine
                .create_runtime_session_json(r#"{"sid":"generation-test","ttl_sec":60}"#)
                .expect("create generation runtime session"),
        )
        .expect("generation create response json");
        let lease_id = created["lease_id"]
            .as_str()
            .expect("generation lease id should be present")
            .to_string();
        let sid = created["sid"]
            .as_str()
            .expect("generation sid should be present")
            .to_string();

        let eval_request = json!({
            "lease_id": lease_id,
            "sid": sid,
            "generation": 999_u64,
            "code": "return 1"
        });
        let eval: Value = serde_json::from_str(
            &engine
                .eval_runtime_session_json(&eval_request.to_string())
                .expect("eval runtime session with wrong generation"),
        )
        .expect("wrong generation eval response json");
        assert_eq!(eval["ok"], false);
        assert_eq!(eval["error_code"], "lease_generation_mismatch");
    }

    /// Verify runtime-session list only returns active leases and supports SID filtering.
    /// 验证运行时会话列表仅返回活跃租约并支持 SID 过滤。
    #[test]
    fn runtime_session_list_returns_only_active_leases() {
        let engine = make_runtime_test_engine();
        let alpha_created: Value = serde_json::from_str(
            &engine
                .create_runtime_session_json(r#"{"sid":"alpha-test","ttl_sec":60}"#)
                .expect("create alpha runtime session"),
        )
        .expect("alpha create response json");
        let beta_created: Value = serde_json::from_str(
            &engine
                .create_runtime_session_json(r#"{"sid":"beta-test","ttl_sec":60}"#)
                .expect("create beta runtime session"),
        )
        .expect("beta create response json");
        let beta_lease_id = beta_created["lease_id"]
            .as_str()
            .expect("beta lease id should be present")
            .to_string();

        let all_list: Value = serde_json::from_str(
            &engine
                .list_runtime_sessions_json(r#"{}"#)
                .expect("list runtime sessions"),
        )
        .expect("list response json");
        assert_eq!(all_list["ok"], true);
        assert_eq!(all_list["leases"].as_array().map(Vec::len), Some(2),);

        let alpha_only: Value = serde_json::from_str(
            &engine
                .list_runtime_sessions_json(r#"{"sid":"alpha-test"}"#)
                .expect("list alpha runtime sessions"),
        )
        .expect("alpha list response json");
        assert_eq!(alpha_only["ok"], true);
        assert_eq!(alpha_only["leases"].as_array().map(Vec::len), Some(1),);
        assert_eq!(alpha_only["leases"][0]["sid"], alpha_created["sid"]);

        let beta_close_request = json!({ "lease_id": beta_lease_id });
        let beta_closed: Value = serde_json::from_str(
            &engine
                .close_runtime_session_json(&beta_close_request.to_string())
                .expect("close beta runtime session"),
        )
        .expect("beta close response json");
        assert_eq!(beta_closed["ok"], true);

        let remaining: Value = serde_json::from_str(
            &engine
                .list_runtime_sessions_json(r#"{}"#)
                .expect("list remaining runtime sessions"),
        )
        .expect("remaining list response json");
        assert_eq!(remaining["ok"], true);
        assert_eq!(remaining["leases"].as_array().map(Vec::len), Some(1),);
        assert_eq!(remaining["leases"][0]["sid"], alpha_created["sid"]);
    }

    /// Verify list requests still return busy active leases while a caller is holding the session lock.
    /// 验证当调用方持有会话锁时列表请求仍然会返回忙碌但活跃的租约。
    #[test]
    fn runtime_session_list_keeps_busy_active_leases_visible() {
        let engine = make_runtime_test_engine();
        let created: Value = serde_json::from_str(
            &engine
                .create_runtime_session_json(r#"{"sid":"busy-list-test","ttl_sec":60}"#)
                .expect("create busy runtime session"),
        )
        .expect("busy create response json");
        let lease_id = created["lease_id"]
            .as_str()
            .expect("busy lease id should be present")
            .to_string();
        let session = engine
            .runtime_sessions
            .get(&lease_id, None, None)
            .expect("get busy runtime session");
        let _guard = session.lock().expect("lock busy runtime session");

        let listed: Value = serde_json::from_str(
            &engine
                .list_runtime_sessions_json(r#"{"sid":"busy-list-test"}"#)
                .expect("list busy runtime sessions"),
        )
        .expect("busy list response json");
        assert_eq!(listed["ok"], true);
        assert_eq!(listed["leases"].as_array().map(Vec::len), Some(1));
        assert_eq!(listed["leases"][0]["lease_id"], lease_id);
    }

    /// Verify that run_lua clears transient args after one failed execution.
    /// 验证 run_lua 在失败执行后同样会清理临时参数状态。
    #[test]
    fn run_lua_clears_args_after_failure() {
        let engine = make_runtime_test_engine();
        let error = engine
            .run_lua("error('boom')", &json!({"value":"hello"}), None)
            .expect_err("run_lua should fail");
        assert!(error.contains("Lua run_lua error"));

        let lease = engine.acquire_vm().expect("reacquire pooled vm");
        assert_vm_scope_is_clean(lease.lua());
    }

    /// Verify that `vulcan.call` restores the outer execution context even when the nested skill corrupts it.
    /// 验证当嵌套技能破坏上下文时，`vulcan.call` 仍会恢复外层执行上下文。
    #[test]
    fn vulcan_call_restores_outer_context_after_nested_failure() {
        let temp_root = std::env::temp_dir().join(format!(
            "luaskills_nested_call_restore_test_{}",
            std::process::id()
        ));
        if temp_root.exists() {
            let _ = fs::remove_dir_all(&temp_root);
        }
        let skill_root = temp_root.join("skills");
        let skill_dir = skill_root.join("test-skill");
        fs::create_dir_all(skill_dir.join("runtime")).expect("create runtime dir");
        fs::write(
            skill_dir.join("skill.yaml"),
            "name: test-skill\nversion: 0.1.0\nenable: true\ndebug: false\nentries:\n  - name: outer\n    lua_entry: runtime/outer.lua\n    lua_module: test-skill.outer\n  - name: nested\n    lua_entry: runtime/nested.lua\n    lua_module: test-skill.nested\n",
        )
        .expect("write skill yaml");
        fs::write(
            skill_dir.join("runtime").join("outer.lua"),
            "return function(args)\n  local ok, err = pcall(vulcan.call, \"test-skill-nested\", {})\n  if ok then\n    return \"nested-call-unexpected-success\"\n  end\n  local tool_name = (vulcan.runtime and vulcan.runtime.internal and vulcan.runtime.internal.tool_name) or \"tool-nil\"\n  local entry_file = (vulcan.context and vulcan.context.entry_file) or \"entry-nil\"\n  local deps_path = (vulcan.deps and vulcan.deps.lua_path) or \"deps-nil\"\n  return tool_name .. \"|\" .. entry_file .. \"|\" .. deps_path\nend\n",
        )
        .expect("write outer runtime entry");
        fs::write(
            skill_dir.join("runtime").join("nested.lua"),
            "return function(args)\n  vulcan.runtime = nil\n  vulcan.context = nil\n  vulcan.deps = nil\n  error(\"boom\")\nend\n",
        )
        .expect("write nested runtime entry");

        let mut engine = LuaEngine::new(LuaEngineOptions {
            host_options: LuaRuntimeHostOptions::default(),
            pool_config: LuaVmPoolConfig {
                min_size: 1,
                max_size: 1,
                idle_ttl_secs: 60,
            },
        })
        .expect("create engine");
        engine
            .load_from_roots(&[crate::host::options::RuntimeSkillRoot {
                name: "ROOT".to_string(),
                skills_dir: skill_root.clone(),
            }])
            .expect("load nested-call test skill");

        let result = engine
            .call_skill("test-skill-outer", &json!({}), None)
            .expect("outer skill should succeed after nested failure");
        assert!(result.content.starts_with("test-skill-outer|"));
        assert!(result.content.contains("outer.lua"));
        assert!(!result.content.contains("|entry-nil|"));
        assert!(!result.content.ends_with("|deps-nil"));
        assert!(result.content.contains("test-skill"));

        let _ = fs::remove_dir_all(&temp_root);
    }
}
