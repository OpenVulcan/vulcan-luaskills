use serde::Deserialize;

// ============================================================
// Lua Skill metadata (loaded from skill.yaml only)
// ============================================================

/// Skill-scoped LanceDB logging level.
/// Skill 级 LanceDB 宿主日志级别配置。
#[derive(Deserialize, Debug, Clone, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SkillLanceDbLogLevel {
    /// Disable host-side LanceDB logs except hard failures.
    /// 除硬错误外关闭宿主侧 LanceDB 日志。
    Off,
    /// Emit informational host-side LanceDB logs.
    /// 输出信息级宿主 LanceDB 日志。
    #[default]
    Info,
    /// Emit only warning/error host-side LanceDB logs.
    /// 仅输出告警/错误级宿主 LanceDB 日志。
    Warning,
}

impl SkillLanceDbLogLevel {
    /// Return the stable wire name of the current log level.
    /// 返回当前日志级别对应的稳定名称。
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Info => "info",
            Self::Warning => "warning",
        }
    }
}

/// Skill-scoped SQLite logging level.
/// Skill 级 SQLite 宿主日志级别配置。
#[derive(Deserialize, Debug, Clone, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SkillSqliteLogLevel {
    /// Disable host-side SQLite logs except hard failures.
    /// 除硬错误外关闭宿主侧 SQLite 日志。
    Off,
    /// Emit informational host-side SQLite logs.
    /// 输出信息级宿主 SQLite 日志。
    #[default]
    Info,
    /// Emit only warning/error host-side SQLite logs.
    /// 仅输出告警/错误级宿主 SQLite 日志。
    Warning,
}

impl SkillSqliteLogLevel {
    /// Return the stable wire name of the current log level.
    /// 返回当前日志级别对应的稳定名称。
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Info => "info",
            Self::Warning => "warning",
        }
    }
}

/// Skill-scoped LanceDB configuration object.
/// Skill 级 LanceDB 配置对象。
#[derive(Deserialize, Debug, Clone, Default)]
pub struct SkillLanceDbMeta {
    /// Whether the current skill should receive a dedicated host-managed LanceDB instance.
    /// 当前 skill 是否需要启用宿主管理的专属 LanceDB 实例。
    #[serde(default)]
    pub enable: bool,
    /// Host-side LanceDB log level.
    /// 宿主侧 LanceDB 日志级别。
    #[serde(default)]
    pub log_level: SkillLanceDbLogLevel,
    /// Whether slow-operation logging is enabled.
    /// 是否开启慢操作日志。
    #[serde(default)]
    pub slow_log_enabled: bool,
    /// Slow-operation threshold in milliseconds.
    /// 慢操作阈值（毫秒）。
    #[serde(default = "default_lancedb_slow_log_threshold_ms")]
    pub slow_log_threshold_ms: u64,
}

/// Skill-scoped SQLite configuration object.
/// Skill 级 SQLite 配置对象。
#[derive(Deserialize, Debug, Clone, Default)]
pub struct SkillSqliteMeta {
    /// Whether the current skill should receive a dedicated host-managed SQLite instance.
    /// 当前 skill 是否需要启用宿主管理的专属 SQLite 实例。
    #[serde(default)]
    pub enable: bool,
    /// Host-side SQLite log level.
    /// 宿主侧 SQLite 日志级别。
    #[serde(default)]
    pub log_level: SkillSqliteLogLevel,
    /// Whether slow-operation logging is enabled.
    /// 是否开启慢操作日志。
    #[serde(default)]
    pub slow_log_enabled: bool,
    /// Slow-operation threshold in milliseconds.
    /// 慢操作阈值（毫秒）。
    #[serde(default = "default_sqlite_slow_log_threshold_ms")]
    pub slow_log_threshold_ms: u64,
}

/// Default slow-operation threshold for host-side LanceDB logs.
/// 宿主侧 LanceDB 慢操作日志默认阈值。
fn default_lancedb_slow_log_threshold_ms() -> u64 {
    800
}

/// Default slow-operation threshold for host-side SQLite logs.
/// 宿主侧 SQLite 慢操作日志默认阈值。
fn default_sqlite_slow_log_threshold_ms() -> u64 {
    500
}

/// Help node metadata used by the new help structure.
/// 新 help 结构使用的帮助节点元数据。
#[derive(Deserialize, Debug, Clone, Default)]
pub struct SkillHelpNodeMeta {
    /// Optional node name used by topic/workflow nodes.
    /// 供 topic/workflow 节点使用的可选名称。
    #[serde(default)]
    pub name: String,
    /// Optional human-readable description of the node.
    /// 当前节点的人类可读描述。
    #[serde(default)]
    pub description: String,
    /// Relative file path of the help payload under `help/`.
    /// 位于 `help/` 目录下的帮助载荷相对路径。
    #[serde(default)]
    pub file: String,
}

/// Top-level help metadata declared by a skill.
/// skill 顶层声明的 help 元数据。
#[derive(Deserialize, Debug, Clone, Default)]
pub struct SkillHelpMeta {
    /// Main help node used for skill-level overview.
    /// 用于 skill 级总览的主帮助节点。
    #[serde(default)]
    pub main: SkillHelpNodeMeta,
    /// Topic or workflow help nodes listed under the main help.
    /// 挂在主帮助下的 topic 或 workflow 子节点。
    #[serde(default)]
    pub topics: Vec<SkillHelpNodeMeta>,
}

/// Shared parameter metadata used by tool entries.
/// 工具入口共用的参数元数据。
#[derive(Deserialize, Debug, Clone)]
pub struct SkillParam {
    /// Parameter name.
    /// 参数名称。
    pub name: String,
    /// Parameter type string used by JSON Schema.
    /// JSON Schema 使用的参数类型字符串。
    #[serde(rename = "type")]
    pub param_type: String,
    /// Parameter description.
    /// 参数描述。
    pub description: String,
    /// Whether the parameter is required.
    /// 参数是否必填。
    #[serde(default)]
    pub required: bool,
}

/// Strict top-level entry metadata used by the new LuaSkills package layout.
/// 新 LuaSkills 包结构使用的严格顶层入口元数据。
#[derive(Deserialize, Debug, Clone)]
pub struct SkillToolMeta {
    /// Local entry name used inside the skill namespace.
    /// skill 命名空间内部使用的局部入口名称。
    pub name: String,
    /// Human-readable entry description shown in tools/list.
    /// 展示在 tools/list 中的人类可读描述。
    #[serde(default)]
    pub description: String,
    /// Relative Lua entry filename under `runtime/`.
    /// 位于 `runtime/` 目录下的相对 Lua 入口文件路径。
    pub lua_entry: String,
    /// Lua module registration name.
    /// Lua 模块注册名称。
    pub lua_module: String,
    /// Parameter definitions specific to this entry.
    /// 当前入口独有的参数定义。
    #[serde(default)]
    pub parameters: Vec<SkillParam>,
    /// Optional help topic/workflow reference name.
    /// 可选的帮助 topic/workflow 引用名称。
    #[serde(default)]
    pub help: String,
}

/// Strict skill-level metadata loaded only from skill.yaml.
/// 仅从 skill.yaml 加载的严格 skill 级元数据。
#[derive(Deserialize, Debug, Clone)]
pub struct SkillMeta {
    /// Internal skill name, for example "vulcan-codekit".
    /// 内部 skill 名称，例如 "vulcan-codekit"。
    pub name: String,
    /// Stable skill namespace used to build canonical entry ids.
    /// 用于生成 canonical 入口 id 的稳定 skill 命名空间。
    pub skill_id: String,
    /// Debug mode: reload Lua source from disk on each invocation.
    /// 调试模式：每次调用时都从磁盘热加载 Lua 源文件。
    #[serde(default)]
    pub debug: bool,
    /// Structured LanceDB configuration used by the host-managed binding.
    /// 宿主管理的 LanceDB 绑定所使用的结构化配置对象。
    #[serde(default)]
    pub lancedb: SkillLanceDbMeta,
    /// Structured SQLite configuration used by the host-managed binding.
    /// 宿主管理的 SQLite 绑定所使用的结构化配置对象。
    #[serde(default)]
    pub sqlite: SkillSqliteMeta,
    /// Top-level entry declarations used by the strict LuaSkills package layout.
    /// 严格 LuaSkills 包结构使用的顶层入口声明。
    #[serde(default)]
    pub entries: Vec<SkillToolMeta>,
    /// New help metadata used to replace prompt-based guidance.
    /// 用于替代 prompt 说明的新 help 元数据。
    #[serde(default)]
    pub help: SkillHelpMeta,
}

/// Return whether one LuaSkills identifier follows the strict lowercase-hyphen rule.
/// 判断某个 LuaSkills 标识符是否满足严格的小写短横线规则。
pub fn is_valid_luaskills_identifier(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return false;
    }

    let mut chars = trimmed.chars();
    let Some(first_char) = chars.next() else {
        return false;
    };
    if !first_char.is_ascii_lowercase() {
        return false;
    }
    if trimmed.ends_with('-') {
        return false;
    }

    chars.all(|character| character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-')
}

/// Validate one LuaSkills identifier and return a bilingual error when it is invalid.
/// 校验单个 LuaSkills 标识符，并在非法时返回双语错误文本。
pub fn validate_luaskills_identifier(value: &str, label: &str) -> Result<(), String> {
    if is_valid_luaskills_identifier(value) {
        return Ok(());
    }

    Err(format!(
        "{label} must match ^[a-z]([a-z0-9-]*[a-z0-9])?$ / {label} 必须匹配 ^[a-z]([a-z0-9-]*[a-z0-9])?$"
    ))
}

impl SkillMeta {
    /// Return the effective LanceDB configuration.
    /// 返回生效的 LanceDB 配置。
    pub fn effective_lancedb(&self) -> SkillLanceDbMeta {
        self.lancedb.clone()
    }

    /// Return the effective SQLite configuration.
    /// 返回生效的 SQLite 配置。
    pub fn effective_sqlite(&self) -> SkillSqliteMeta {
        self.sqlite.clone()
    }

    /// Return the effective skill id used by canonical entry names.
    /// 返回用于 canonical 入口名的生效 skill id。
    pub fn effective_skill_id(&self) -> &str {
        self.skill_id.trim()
    }

    /// Iterate over all top-level entries.
    /// 遍历当前 skill 下的全部顶层入口。
    pub fn entries(&self) -> impl Iterator<Item = &SkillToolMeta> {
        self.entries.iter()
    }

    /// Build the unresolved base name of one entry before conflict indexing.
    /// 构建单个入口在冲突编号前的未解析基础名称。
    pub fn tool_base_name(&self, tool: &SkillToolMeta) -> String {
        format!("{}-{}", self.effective_skill_id(), tool.name.trim())
    }

    /// Find one entry by its strict local name.
    /// 根据严格局部入口名查找单个入口。
    pub fn find_tool_by_local_name(&self, tool_name: &str) -> Option<&SkillToolMeta> {
        self.entries()
            .find(|tool| tool.name.trim() == tool_name)
    }

    /// Return the main help node declared by the skill.
    /// 返回 skill 声明的主帮助节点。
    pub fn main_help(&self) -> &SkillHelpNodeMeta {
        &self.help.main
    }

    /// Iterate over all topic or workflow help nodes declared by the skill.
    /// 遍历当前 skill 声明的全部 topic 或 workflow 帮助节点。
    pub fn help_topics(&self) -> impl Iterator<Item = &SkillHelpNodeMeta> {
        self.help.topics.iter()
    }

    /// Find one help topic or workflow node by its declared name.
    /// 根据声明名称查找单个 help topic 或 workflow 节点。
    pub fn find_help_topic(&self, topic_name: &str) -> Option<&SkillHelpNodeMeta> {
        self.help_topics().find(|topic| topic.name.trim() == topic_name)
    }

    /// Return all entries that reference one help topic/workflow name.
    /// 返回引用某个 help topic/workflow 名称的全部入口。
    pub fn entries_for_help_topic<'a>(
        &'a self,
        topic_name: &'a str,
    ) -> impl Iterator<Item = &'a SkillToolMeta> + 'a {
        self.entries()
            .filter(move |tool| tool.help.trim() == topic_name)
    }
}

#[cfg(test)]
mod tests {
    use super::{is_valid_luaskills_identifier, validate_luaskills_identifier};

    /// Verify that legal lowercase-hyphen identifiers are accepted.
    /// 验证合法的小写短横线标识符会被接受。
    #[test]
    fn valid_luaskills_identifiers_are_accepted() {
        assert!(is_valid_luaskills_identifier("vulcan-codekit"));
        assert!(is_valid_luaskills_identifier("codekit2"));
        assert!(is_valid_luaskills_identifier("vulcan-runtime-tools"));
    }

    /// Verify that invalid identifiers are rejected by both helper functions.
    /// 验证非法标识符会被两个辅助函数共同拒绝。
    #[test]
    fn invalid_luaskills_identifiers_are_rejected() {
        for candidate in [
            "",
            "2codekit",
            "Vulcan-codekit",
            "vulcan_codekit",
            "vulcan-codekit-",
            "__demo",
        ] {
            assert!(!is_valid_luaskills_identifier(candidate));
            assert!(validate_luaskills_identifier(candidate, "skill_id").is_err());
        }
    }
}
