use mlua::{Function, HookTriggers, Lua, MultiValue, Table, Value as LuaValue, VmState};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use crate::dependency::manager::{DependencyManager, DependencyManagerConfig, ensure_directory};
use crate::entry_descriptor::{RuntimeEntryDescriptor, RuntimeEntryParameterDescriptor};
use crate::host::callbacks::{
    RuntimeEntryRegistryDelta, RuntimeSkillLifecycleEvent, RuntimeSkillManagementAction,
    RuntimeSkillManagementRequest, dispatch_skill_management_request,
    try_has_skill_management_callback,
};
use crate::host::database::RuntimeDatabaseProviderCallbacks;
use crate::lancedb_host::{LanceDbSkillBinding, LanceDbSkillHost, disabled_skill_status_json};
use crate::lua_skill::{SkillMeta, validate_luaskills_identifier, validate_luaskills_version};
use crate::runtime::config::{SkillConfigEntry, SkillConfigStore};
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
    PreparedSkillApply, SkillApplyResult, SkillInstallRequest, SkillManager, SkillManagerConfig,
    SkillOperationPlane, SkillUninstallOptions, SkillUninstallResult,
    collect_effective_skill_instances_from_roots, resolve_declared_skill_instance_from_roots,
    resolve_effective_skill_instance_from_roots,
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
    pool: Arc<LuaVmPool>,
    runlua_pool: Arc<LuaVmPool>,
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
        let result = dispatch_skill_management_request(&RuntimeSkillManagementRequest {
            action: action.clone(),
            input: payload,
        })
        .map_err(|error| {
            mlua::Error::runtime(format!("vulcan.runtime.skills.{}: {}", action_name, error))
        })?;
        json_value_to_lua(lua, &result)
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

/// Return the default empty args object for runlua execution.
/// 返回 runlua 执行默认使用的空参数对象。
fn default_runlua_exec_args() -> Value {
    Value::Object(serde_json::Map::new())
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
    mode: ExecMode,
    cwd: Option<String>,
    env: HashMap<String, String>,
    stdin: Option<String>,
    timeout_ms: Option<u64>,
}

/// Process execution result returned back to Lua.
/// 返回给 Lua 的进程执行结果。
struct ExecResult {
    ok: bool,
    success: bool,
    code: Option<i32>,
    stdout: String,
    stderr: String,
    timed_out: bool,
    error: Option<String>,
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

/// Parse Lua input into an executable process request.
/// 将 Lua 输入解析为可执行的进程请求。
fn parse_exec_request(value: LuaValue, fn_name: &str) -> mlua::Result<ExecRequest> {
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

/// Spawn a background reader for a child process output pipe.
/// 为子进程输出管道启动后台读取线程。
fn spawn_pipe_reader<R>(mut reader: R) -> thread::JoinHandle<String>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut buffer = Vec::new();
        let _ = reader.read_to_end(&mut buffer);
        String::from_utf8_lossy(&buffer).to_string()
    })
}

/// Spawn a background writer for a child process stdin pipe.
/// 为子进程标准输入管道启动后台写入线程。
fn spawn_stdin_writer<W>(mut writer: W, input: String) -> thread::JoinHandle<()>
where
    W: Write + Send + 'static,
{
    thread::spawn(move || {
        let _ = writer.write_all(input.as_bytes());
        let _ = writer.flush();
    })
}

/// Execute a process request and capture its structured result.
/// 执行进程请求并捕获结构化结果。
fn execute_exec_request(request: ExecRequest) -> ExecResult {
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
    command.stdin(if request.stdin.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            let error_text = format!("failed to spawn process: {}", error);
            return ExecResult {
                ok: false,
                success: false,
                code: None,
                stdout: String::new(),
                stderr: error_text.clone(),
                timed_out: false,
                error: Some(error_text),
            };
        }
    };

    let stdout_handle = child.stdout.take().map(spawn_pipe_reader);
    let stderr_handle = child.stderr.take().map(spawn_pipe_reader);
    let stdin_handle = match (request.stdin.clone(), child.stdin.take()) {
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
                return ExecResult {
                    ok: false,
                    success: false,
                    code: None,
                    stdout: String::new(),
                    stderr: error_text.clone(),
                    timed_out,
                    error: Some(error_text),
                };
            }
        }
    };

    if let Some(handle) = stdin_handle {
        let _ = handle.join();
    }

    let stdout = stdout_handle
        .map(|handle| handle.join().unwrap_or_default())
        .unwrap_or_default();
    let mut stderr = stderr_handle
        .map(|handle| handle.join().unwrap_or_default())
        .unwrap_or_default();

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
    path: Table,
    process: Table,
    os: Table,
    json: Table,
    cache: Table,
    context: Table,
    deps: Table,
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
    /// Return the reference root used for host-managed fallback directories when no explicit host path is provided.
    /// 在未显式提供宿主管理路径时返回用于回退目录计算的参考根目录。
    fn reference_skill_root<'a>(
        &self,
        skill_roots: &'a [RuntimeSkillRoot],
    ) -> Result<&'a RuntimeSkillRoot, String> {
        skill_roots
            .iter()
            .rev()
            .find(|root| root.skills_dir.exists() && root.skills_dir.is_dir())
            .ok_or_else(|| "at least one skill root is required".to_string())
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
            pool: Arc::new(LuaVmPool::new(options.pool_config)),
            runlua_pool: Arc::new(LuaVmPool::new(runlua_pool_config)),
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
            protection: self.host_options.protection.clone(),
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

    /// Load skills from directories. `base_dir` is the system skill directory,
    /// `override_dir` is the user override directory (if any).
    pub fn load_from_dirs(
        &mut self,
        base_dir: &Path,
        override_dir: Option<&Path>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut skill_roots = Vec::new();
        if let Some(override_dir) = override_dir {
            skill_roots.push(RuntimeSkillRoot {
                name: "OVERRIDE".to_string(),
                skills_dir: override_dir.to_path_buf(),
            });
        }
        skill_roots.push(RuntimeSkillRoot {
            name: "ROOT".to_string(),
            skills_dir: base_dir.to_path_buf(),
        });
        self.load_from_roots(&skill_roots)
    }

    /// Load skills from an ordered root chain where earlier roots override later roots.
    /// 从有序根目录覆盖链加载技能，前面的根目录会覆盖后面的同名技能。
    pub fn load_from_roots(
        &mut self,
        skill_roots: &[RuntimeSkillRoot],
    ) -> Result<(), Box<dyn std::error::Error>> {
        if !skill_roots.is_empty() {
            self.refresh_skill_config_runtime_root(skill_roots)
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
                    self.lancedb_host.clone(),
                    self.sqlite_host.clone(),
                )
            })
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;

        log_info(format!("[LuaSkill] {} skills loaded", self.skills.len()));
        Ok(())
    }

    /// Reload all skills from the given directories and rebuild runtime state from scratch.
    /// 从给定目录重新加载全部技能，并从零重建运行时状态。
    pub fn reload_from_dirs(
        &mut self,
        base_dir: &Path,
        override_dir: Option<&Path>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut skill_roots = Vec::new();
        if let Some(override_dir) = override_dir {
            skill_roots.push(RuntimeSkillRoot {
                name: "OVERRIDE".to_string(),
                skills_dir: override_dir.to_path_buf(),
            });
        }
        skill_roots.push(RuntimeSkillRoot {
            name: "ROOT".to_string(),
            skills_dir: base_dir.to_path_buf(),
        });
        self.reload_from_roots(&skill_roots)
    }

    /// Reload all skills from one ordered root chain and rebuild runtime state from scratch.
    /// 从一条有序根目录覆盖链中重载全部技能，并从零重建运行时状态。
    pub fn reload_from_roots(
        &mut self,
        skill_roots: &[RuntimeSkillRoot],
    ) -> Result<(), Box<dyn std::error::Error>> {
        let previous_entries = self.list_entries();
        self.reset_runtime_state();
        self.load_from_roots(skill_roots)?;
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
        validate_luaskills_identifier(skill_id, "skill_id")
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        let resolved_instance = resolve_effective_skill_instance_from_roots(skill_roots, skill_id)
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?
            .ok_or_else(|| -> Box<dyn std::error::Error> {
                format!("effective skill instance '{}' not found", skill_id).into()
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
        if !matches!(
            action,
            crate::skill::manager::SkillLifecycleAction::Install
                | crate::skill::manager::SkillLifecycleAction::Update
        ) {
            return Err(format!("unsupported apply action {:?}", action).into());
        }
        let target_root = match action {
            crate::skill::manager::SkillLifecycleAction::Install => {
                self.reference_skill_root(skill_roots)?.clone()
            }
            crate::skill::manager::SkillLifecycleAction::Update => {
                let target_skill_id = request
                    .skill_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned)
                    .or_else(|| {
                        request
                            .source
                            .as_deref()
                            .and_then(|value| value.trim().rsplit('/').next())
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                            .map(ToOwned::to_owned)
                    })
                    .ok_or_else(|| -> Box<dyn std::error::Error> {
                        "update request requires skill_id or one derivable source".into()
                    })?;
                let resolved_instance =
                    resolve_declared_skill_instance_from_roots(skill_roots, &target_skill_id)
                        .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?
                        .ok_or_else(|| -> Box<dyn std::error::Error> {
                            format!("skill '{}' is not installed", target_skill_id).into()
                        })?;
                RuntimeSkillRoot {
                    name: resolved_instance.root_name,
                    skills_dir: resolved_instance.skills_root,
                }
            }
            _ => unreachable!("unsupported apply action should have returned early"),
        };
        let previous_dependency_manifest =
            if action == crate::skill::manager::SkillLifecycleAction::Update {
                let target_skill_id = request
                    .skill_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned)
                    .or_else(|| {
                        request
                            .source
                            .as_deref()
                            .and_then(|value| value.trim().rsplit('/').next())
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                            .map(ToOwned::to_owned)
                    });
                if let Some(target_skill_id) = target_skill_id {
                    resolve_declared_skill_instance_from_roots(skill_roots, &target_skill_id)
                        .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?
                        .and_then(|resolved| {
                            self.load_skill_dependency_manifest(&resolved.actual_dir)
                                .transpose()
                        })
                        .transpose()
                        .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?
                } else {
                    None
                }
            } else {
                None
            };
        let manager = self
            .skill_manager_for(&target_root)
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        let prepared = match action {
            crate::skill::manager::SkillLifecycleAction::Install => manager
                .prepare_install_skill(plane, skill_roots, request)
                .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?,
            crate::skill::manager::SkillLifecycleAction::Update => manager
                .prepare_update_skill(plane, skill_roots, request)
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
                resolve_declared_skill_instance_from_roots(skill_roots, &result.skill_id)
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
            resolve_declared_skill_instance_from_roots(skill_roots, &result.skill_id)
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

    /// Mark one skill disabled through the ordinary skills plane and immediately reload the runtime view.
    /// 通过普通 skills 平面将单个技能标记为停用，并立即重载运行时视图。
    pub fn disable_skill(
        &mut self,
        base_dir: &Path,
        override_dir: Option<&Path>,
        skill_id: &str,
        reason: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut skill_roots = Vec::new();
        if let Some(override_dir) = override_dir {
            skill_roots.push(RuntimeSkillRoot {
                name: "OVERRIDE".to_string(),
                skills_dir: override_dir.to_path_buf(),
            });
        }
        skill_roots.push(RuntimeSkillRoot {
            name: "ROOT".to_string(),
            skills_dir: base_dir.to_path_buf(),
        });
        self.disable_skill_in_roots(&skill_roots, skill_id, reason)
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

    /// Mark one skill disabled through the host-controlled system plane and immediately reload the runtime view.
    /// 通过宿主控制的 system 平面将单个技能标记为停用，并立即重载运行时视图。
    pub fn system_disable_skill(
        &mut self,
        base_dir: &Path,
        override_dir: Option<&Path>,
        skill_id: &str,
        reason: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut skill_roots = Vec::new();
        if let Some(override_dir) = override_dir {
            skill_roots.push(RuntimeSkillRoot {
                name: "OVERRIDE".to_string(),
                skills_dir: override_dir.to_path_buf(),
            });
        }
        skill_roots.push(RuntimeSkillRoot {
            name: "ROOT".to_string(),
            skills_dir: base_dir.to_path_buf(),
        });
        self.system_disable_skill_in_roots(&skill_roots, skill_id, reason)
    }

    /// Mark one skill disabled through the host-controlled system plane using an ordered root chain and immediately reload the runtime view.
    /// 通过宿主控制的 system 平面使用有序根目录链将单个技能标记为停用，并立即重载运行时视图。
    pub fn system_disable_skill_in_roots(
        &mut self,
        skill_roots: &[RuntimeSkillRoot],
        skill_id: &str,
        reason: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.mutate_skill_state_and_reload(
            SkillOperationPlane::System,
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
        skill_id: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.mutate_skill_state_and_reload(
            SkillOperationPlane::System,
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

    /// Uninstall one skill directory through the host-controlled system plane and immediately reload the runtime view.
    /// 通过宿主控制的 system 平面卸载单个技能目录，并立即重载运行时视图。
    pub fn system_uninstall_skill(
        &mut self,
        skill_roots: &[RuntimeSkillRoot],
        skill_id: &str,
        options: &SkillUninstallOptions,
    ) -> Result<SkillUninstallResult, Box<dyn std::error::Error>> {
        self.uninstall_skill_and_reload(SkillOperationPlane::System, skill_roots, skill_id, options)
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

    /// Preflight one install request through the host-controlled system plane and return a structured result.
    /// 通过宿主控制的 system 平面对一次安装请求执行预检查，并返回结构化结果。
    pub fn system_install_skill(
        &mut self,
        skill_roots: &[RuntimeSkillRoot],
        request: &SkillInstallRequest,
    ) -> Result<SkillApplyResult, Box<dyn std::error::Error>> {
        self.apply_skill_request(
            SkillOperationPlane::System,
            crate::skill::manager::SkillLifecycleAction::Install,
            skill_roots,
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

    /// Preflight one update request through the host-controlled system plane and return a structured result.
    /// 通过宿主控制的 system 平面对一次更新请求执行预检查，并返回结构化结果。
    pub fn system_update_skill(
        &mut self,
        skill_roots: &[RuntimeSkillRoot],
        request: &SkillInstallRequest,
    ) -> Result<SkillApplyResult, Box<dyn std::error::Error>> {
        self.apply_skill_request(
            SkillOperationPlane::System,
            crate::skill::manager::SkillLifecycleAction::Update,
            skill_roots,
            request,
        )
    }

    /// Reset all loaded skills, providers, and pooled VMs before one full reload.
    /// 在执行一次完整重载前重置全部已加载技能、provider 与虚拟机池。
    fn reset_runtime_state(&mut self) {
        let pool_config = self.pool.config;
        let runlua_pool_config = self.runlua_pool.config;
        self.skills.clear();
        self.entry_registry.clear();
        self.lancedb_host = None;
        self.sqlite_host = None;
        self.pool = Arc::new(LuaVmPool::new(pool_config));
        self.runlua_pool = Arc::new(LuaVmPool::new(runlua_pool_config));
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

    /// Build a fresh Lua VM instance with all loaded skills registered.
    /// 创建一个全新的 Lua 虚拟机实例，并注册当前已加载的全部技能。
    fn create_vm(&self) -> Result<LuaVm, String> {
        let skills = Arc::new(self.skills.clone());
        let entry_registry = Arc::new(self.entry_registry.clone());
        let lua = unsafe { Lua::unsafe_new() };
        Self::setup_package_paths(&lua, self.host_options.as_ref())
            .map_err(|error| error.to_string())?;
        Self::register_vulcan_module(
            &lua,
            self.host_options.as_ref(),
            self.skill_config_store.clone(),
        )
        .map_err(|error| error.to_string())?;
        Self::populate_vulcan_luaexec_bridge(
            &lua,
            self.host_options.clone(),
            self.runlua_pool.clone(),
            self.skill_config_store.clone(),
            skills.clone(),
            entry_registry.clone(),
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
        lancedb_host: Option<Arc<LanceDbSkillHost>>,
        sqlite_host: Option<Arc<SqliteSkillHost>>,
    ) -> Result<LuaVm, String> {
        let lua = unsafe { Lua::unsafe_new() };
        Self::setup_package_paths(&lua, host_options.as_ref())
            .map_err(|error| error.to_string())?;
        Self::register_vulcan_module(&lua, host_options.as_ref(), skill_config_store)
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

    /// Check if a tool_name is a Lua skill.
    pub fn is_skill(&self, name: &str) -> bool {
        self.entry_registry.contains_key(name)
    }

    /// Return the owning skill name for an MCP tool name; return `None` when the tool is not provided by a Lua skill.
    /// 根据 MCP 工具名返回所属 skill 名称；未命中时返回 `None`。
    pub fn skill_name_for_tool(&self, tool_name: &str) -> Option<String> {
        self.entry_registry
            .get(tool_name)
            .map(|target| target.skill_id.clone())
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

    /// Call a loaded Lua skill with the given JSON arguments.
    /// This is synchronous — wrap in spawn_blocking for async contexts.
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

    /// Execute arbitrary Lua code and return the result.
    pub fn run_lua(
        &self,
        code: &str,
        args: &Value,
        invocation_context: Option<&LuaInvocationContext>,
    ) -> Result<Value, String> {
        let mut lease = self.acquire_vm()?;
        let scope_guard = LuaVmRequestScopeGuard::new(&mut lease, self.host_options.as_ref())?;
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

        // Build a wrapper that passes args as a local variable
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

    /// Acquire one isolated runlua VM from the dedicated pool.
    /// 从独立池中获取一个隔离 runlua 虚拟机。
    fn acquire_runlua_vm(
        runlua_pool: Arc<LuaVmPool>,
        skills: Arc<HashMap<String, LoadedSkill>>,
        entry_registry: Arc<BTreeMap<String, ResolvedEntryTarget>>,
        host_options: Arc<LuaRuntimeHostOptions>,
        skill_config_store: Arc<SkillConfigStore>,
        lancedb_host: Option<Arc<LanceDbSkillHost>>,
        sqlite_host: Option<Arc<SqliteSkillHost>>,
    ) -> Result<LuaVmLease, String> {
        runlua_pool.acquire(move || {
            Self::create_runlua_vm(
                skills.as_ref(),
                entry_registry.as_ref(),
                host_options.clone(),
                skill_config_store.clone(),
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
                luaexec_active: true,
                luaexec_caller_tool_name: request.caller_tool_name.clone(),
            },
        )?;
        populate_vulcan_file_context(lua, None, entry_file.as_deref())?;
        Self::populate_vulcan_lancedb_context(lua, None, None)?;
        Self::populate_vulcan_sqlite_context(lua, None, None)?;

        let captured_output: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        Self::configure_runlua_execution_environment(lua, captured_output.clone())?;

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
    ) -> Result<(), String> {
        let runtime = get_vulcan_runtime_table(lua)?;
        let runtime_lua = get_vulcan_runtime_lua_table(lua)?;
        let cache = get_vulcan_table(lua)?
            .get::<Table>("cache")
            .map_err(|error| format!("Failed to get vulcan.cache: {}", error))?;

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
        let context = lua.create_table()?;
        let deps = lua.create_table()?;

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

        let exec_fn = lua.create_function(|lua, spec: LuaValue| {
            let request = parse_exec_request(spec, "process.exec")?;
            let result = execute_exec_request(request);
            exec_result_to_lua_table(lua, result)
        })?;
        process.set("exec", exec_fn)?;

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
        vulcan.set("path", path)?;
        vulcan.set("process", process)?;
        vulcan.set("os", os)?;
        vulcan.set("json", json)?;
        vulcan.set("cache", cache)?;
        vulcan.set("config", config)?;
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
        SkillConfigStore, VulcanInternalExecutionContext, default_runlua_vm_pool_config,
        get_vulcan_context_table, get_vulcan_deps_table, get_vulcan_runtime_internal_table,
        get_vulcan_table, json_to_lua_table, normalize_host_visible_path_text,
        populate_vulcan_dependency_context, populate_vulcan_file_context,
        populate_vulcan_internal_execution_context,
    };
    use crate::host::database::RuntimeDatabaseProviderCallbacks;
    use crate::lua_skill::SkillMeta;
    use crate::runtime_options::LuaRuntimeRunLuaPoolConfig;
    use crate::{LuaEngineOptions, LuaRuntimeHostOptions};
    use mlua::{Table, Value as LuaValue};
    use serde_json::json;
    use std::collections::HashMap;
    use std::fs;
    use std::path::Path;
    use std::path::PathBuf;
    use std::sync::{Arc, Condvar, Mutex};

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
        LuaEngine::new(LuaEngineOptions {
            host_options: LuaRuntimeHostOptions::default(),
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
            "vulcan_luaskills_{}_{}_{}",
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
                name: "ROOT_FIRST".to_string(),
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
                name: "ROOT_SECOND".to_string(),
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
                    name: "ROOT_A".to_string(),
                    skills_dir: first_runtime_root.join("skills"),
                },
                crate::host::options::RuntimeSkillRoot {
                    name: "ROOT_B".to_string(),
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
                    name: "ROOT_A".to_string(),
                    skills_dir: first_runtime_root.join("skills"),
                },
                crate::host::options::RuntimeSkillRoot {
                    name: "ROOT_B".to_string(),
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
                    name: "ROOT_CANONICAL".to_string(),
                    skills_dir: runtime_root.join("skills"),
                },
                crate::host::options::RuntimeSkillRoot {
                    name: "ROOT_EQUIVALENT".to_string(),
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
            "vulcan_luaskills_reject_skill_id_test_{}",
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
            "vulcan_luaskills_ignored_skill_test_{}",
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
            "vulcan_luaskills_nested_call_restore_test_{}",
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
