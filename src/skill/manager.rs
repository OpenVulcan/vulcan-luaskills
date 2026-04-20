use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::lua_skill::validate_luaskills_identifier;

/// English: Lifecycle operations that the LuaSkills manager layer exposes for one skill.
/// LuaSkills 管理层为单个技能公开的生命周期操作类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SkillLifecycleAction {
    Install,
    Update,
    Reload,
    Uninstall,
    Enable,
    Disable,
}

/// English: Logical operation plane used to distinguish host system controls from ordinary skill controls.
/// 用于区分宿主系统控制面与普通技能控制面的逻辑操作平面。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SkillOperationPlane {
    Skills,
    System,
}

/// English: Host-owned protection configuration that reserves specific skill identifiers.
/// 由宿主持有的保护配置，用于保留特定技能标识符。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillProtectionConfig {
    /// English: Reserved protected skill identifiers that cannot be handled through the `skills` plane.
    /// 受保护的保留技能标识符列表，禁止通过 `skills` 平面处理。
    #[serde(default)]
    pub protected_skill_ids: Vec<String>,
}

/// English: High-level manager configuration that defines where installed skills and their state are stored.
/// 定义已安装技能及其状态存放位置的高层管理配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillManagerConfig {
    /// English: Root directory where installed skill packages live.
    /// 已安装 skill 包所在的根目录。
    pub skill_root: PathBuf,
    /// English: Root directory where enabled/disabled state markers are persisted.
    /// 保存启用/停用状态标记的根目录。
    pub state_root: PathBuf,
    /// English: Host-owned protection policy that reserves core skill identifiers.
    /// 宿主拥有的保护策略，用于保留核心技能标识符。
    #[serde(default)]
    pub protection: SkillProtectionConfig,
}

/// English: One install request accepted by the future LuaSkills manager entrypoints.
/// 未来 LuaSkills 管理入口接受的单次安装请求定义。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillInstallRequest {
    /// English: Optional skill id used for install-by-name flows.
    /// 供按名称安装流程使用的可选 skill id。
    pub skill_id: Option<String>,
    /// English: Optional raw source string such as URL or local directory.
    /// 例如 URL 或本地目录一类的可选原始来源字符串。
    pub source: Option<String>,
}

/// English: One install or update result returned by the skill manager.
/// 由技能管理器返回的单次安装或更新结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillApplyResult {
    /// English: Stable skill identifier targeted by the current operation.
    /// 当前操作目标的稳定技能标识符。
    pub skill_id: String,
    /// English: High-level result status such as blocked, already_installed, or not_implemented.
    /// 高层结果状态，例如 blocked、already_installed 或 not_implemented。
    pub status: String,
    /// English: Human-readable explanation of the current result.
    /// 当前结果的人类可读解释文本。
    pub message: String,
}

/// English: Optional database cleanup switches accepted by skill uninstall operations.
/// 技能卸载操作接受的可选数据库清理开关集合。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillUninstallOptions {
    /// English: Remove the SQLite database directory owned by the target skill when true.
    /// 为 true 时删除目标技能拥有的 SQLite 数据目录。
    #[serde(default)]
    pub remove_sqlite: bool,
    /// English: Remove the LanceDB database directory owned by the target skill when true.
    /// 为 true 时删除目标技能拥有的 LanceDB 数据目录。
    #[serde(default)]
    pub remove_lancedb: bool,
}

/// English: Structured uninstall result that reports whether code and databases were removed or retained.
/// 结构化卸载结果，用于报告代码与数据库是被删除还是被保留。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillUninstallResult {
    /// English: Stable skill identifier targeted by the current uninstall action.
    /// 当前卸载动作目标的稳定技能标识符。
    pub skill_id: String,
    /// English: Whether the skill package directory itself was removed.
    /// skill 包目录本身是否已经被删除。
    pub skill_removed: bool,
    /// English: Whether the SQLite database directory was removed explicitly.
    /// SQLite 数据目录是否已被显式删除。
    pub sqlite_removed: bool,
    /// English: Whether the LanceDB database directory was removed explicitly.
    /// LanceDB 数据目录是否已被显式删除。
    pub lancedb_removed: bool,
    /// English: Whether the SQLite database directory was intentionally retained.
    /// SQLite 数据目录是否被有意保留。
    pub sqlite_retained: bool,
    /// English: Whether the LanceDB database directory was intentionally retained.
    /// LanceDB 数据目录是否被有意保留。
    pub lancedb_retained: bool,
    /// English: Human-readable explanation of the uninstall result.
    /// 当前卸载结果的人类可读说明文本。
    pub message: String,
}

/// English: Persistent record written when one skill is explicitly disabled.
/// 显式停用某个技能时写入的持久化记录。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisabledSkillRecord {
    /// English: Stable skill identifier bound to this state record.
    /// 与当前状态记录绑定的稳定 skill 标识符。
    pub skill_id: String,
    /// English: Optional human-readable disable reason.
    /// 可选的人类可读停用原因。
    pub reason: Option<String>,
    /// English: Unix timestamp in milliseconds when the skill was disabled.
    /// 当前技能被停用时的 Unix 毫秒时间戳。
    pub disabled_at_unix_ms: u128,
}

/// English: Skill manager that owns persisted skill enabled/disabled state.
/// 持有技能启用/停用持久状态的技能管理器。
pub struct SkillManager {
    config: SkillManagerConfig,
}

impl SkillManager {
    /// English: Create one skill manager from a shared configuration object.
    /// 基于共享配置对象创建一个技能管理器实例。
    pub fn new(config: SkillManagerConfig) -> Self {
        Self { config }
    }

    /// English: Ensure the skill-state root and its child directories exist.
    /// 确保技能状态根目录及其子目录已经存在。
    pub fn ensure_state_layout(&self) -> Result<(), String> {
        fs::create_dir_all(self.disabled_root()).map_err(|error| {
            format!(
                "Failed to create disabled root {}: {}",
                self.disabled_root().display(),
                error
            )
        })
    }

    /// English: Validate one skill id and enforce the plane-specific protection rules.
    /// 校验单个 skill id 并执行按平面划分的保护规则。
    pub fn guard_operation(
        &self,
        plane: SkillOperationPlane,
        action: SkillLifecycleAction,
        skill_id: &str,
    ) -> Result<(), String> {
        validate_luaskills_identifier(skill_id, "skill_id")?;
        if self.is_protected_skill(skill_id) && plane == SkillOperationPlane::Skills {
            return Err(format!(
                "protected skill '{}' cannot be processed through the skills plane for action {:?} / 受保护技能 '{}' 不能通过 skills 平面执行 {:?} 操作",
                skill_id, action, skill_id, action
            ));
        }
        Ok(())
    }

    /// English: Return whether one skill identifier is reserved by the host protection policy.
    /// 返回单个技能标识符是否被宿主保护策略保留。
    pub fn is_protected_skill(&self, skill_id: &str) -> bool {
        self.config
            .protection
            .protected_skill_ids
            .iter()
            .any(|protected| protected.trim() == skill_id)
    }

    /// English: Return whether one skill is currently enabled.
    /// 返回单个技能当前是否处于启用状态。
    pub fn is_skill_enabled(&self, skill_id: &str) -> Result<bool, String> {
        self.ensure_state_layout()?;
        Ok(!self.disabled_record_path(skill_id).exists())
    }

    /// English: Persist one disabled-state marker for the specified skill.
    /// 为指定技能持久化一份停用状态标记。
    pub fn disable_skill(&self, skill_id: &str, reason: Option<&str>) -> Result<(), String> {
        self.disable_skill_in_plane(SkillOperationPlane::Skills, skill_id, reason)
    }

    /// English: Persist one disabled-state marker for the specified skill in the requested operation plane.
    /// 在指定操作平面为目标技能持久化一份停用状态标记。
    pub fn disable_skill_in_plane(
        &self,
        plane: SkillOperationPlane,
        skill_id: &str,
        reason: Option<&str>,
    ) -> Result<(), String> {
        self.guard_operation(plane, SkillLifecycleAction::Disable, skill_id)?;
        self.ensure_state_layout()?;
        let record = DisabledSkillRecord {
            skill_id: skill_id.to_string(),
            reason: reason.map(|value| value.trim().to_string()).filter(|value| !value.is_empty()),
            disabled_at_unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
        };
        let path = self.disabled_record_path(skill_id);
        let content = serde_json::to_string_pretty(&record)
            .map_err(|error| format!("Failed to serialize disabled record: {}", error))?;
        fs::write(&path, content)
            .map_err(|error| format!("Failed to write {}: {}", path.display(), error))
    }

    /// English: Remove the disabled-state marker for one skill.
    /// 删除单个技能的停用状态标记。
    pub fn enable_skill(&self, skill_id: &str) -> Result<(), String> {
        self.enable_skill_in_plane(SkillOperationPlane::Skills, skill_id)
    }

    /// English: Remove the disabled-state marker for one skill in the requested operation plane.
    /// 在指定操作平面移除单个技能的停用状态标记。
    pub fn enable_skill_in_plane(
        &self,
        plane: SkillOperationPlane,
        skill_id: &str,
    ) -> Result<(), String> {
        self.guard_operation(plane, SkillLifecycleAction::Enable, skill_id)?;
        self.ensure_state_layout()?;
        let path = self.disabled_record_path(skill_id);
        if path.exists() {
            fs::remove_file(&path)
                .map_err(|error| format!("Failed to remove {}: {}", path.display(), error))?;
        }
        Ok(())
    }

    /// English: Read the disabled-state record for one skill when it exists.
    /// 在停用状态记录存在时读取单个技能的停用状态记录。
    pub fn disabled_record(&self, skill_id: &str) -> Result<Option<DisabledSkillRecord>, String> {
        let path = self.disabled_record_path(skill_id);
        if !path.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(&path)
            .map_err(|error| format!("Failed to read {}: {}", path.display(), error))?;
        let record = serde_json::from_str::<DisabledSkillRecord>(&content)
            .map_err(|error| format!("Failed to parse {}: {}", path.display(), error))?;
        Ok(Some(record))
    }

    /// English: Remove one installed skill directory and clear its disabled marker.
    /// 删除单个已安装 skill 目录，并清理其停用标记。
    pub fn uninstall_skill(&self, skill_id: &str) -> Result<SkillUninstallResult, String> {
        self.uninstall_skill_in_plane(SkillOperationPlane::Skills, skill_id)
    }

    /// English: Remove one installed skill directory and clear its disabled marker in the requested operation plane.
    /// 在指定操作平面删除单个已安装技能目录，并清理其停用标记。
    pub fn uninstall_skill_in_plane(
        &self,
        plane: SkillOperationPlane,
        skill_id: &str,
    ) -> Result<SkillUninstallResult, String> {
        self.guard_operation(plane, SkillLifecycleAction::Uninstall, skill_id)?;
        let skill_dir = self.config.skill_root.join(skill_id);
        let skill_removed = if skill_dir.exists() {
            fs::remove_dir_all(&skill_dir)
                .map_err(|error| format!("Failed to remove {}: {}", skill_dir.display(), error))?;
            true
        } else {
            false
        };
        self.enable_skill_in_plane(plane, skill_id)?;
        Ok(SkillUninstallResult {
            skill_id: skill_id.to_string(),
            skill_removed,
            sqlite_removed: false,
            lancedb_removed: false,
            sqlite_retained: false,
            lancedb_retained: false,
            message: if skill_removed {
                "skill package removed / 技能包目录已删除".to_string()
            } else {
                "skill package directory not found / 未找到技能包目录".to_string()
            },
        })
    }

    /// English: Preflight one install request and return a structured placeholder result.
    /// 对单个安装请求执行预检查并返回结构化占位结果。
    pub fn install_skill(
        &self,
        plane: SkillOperationPlane,
        request: &SkillInstallRequest,
    ) -> Result<SkillApplyResult, String> {
        let skill_id = request
            .skill_id
            .as_deref()
            .ok_or_else(|| "install request requires skill_id / install 请求必须提供 skill_id".to_string())?
            .trim()
            .to_string();
        self.guard_operation(plane, SkillLifecycleAction::Install, &skill_id)?;
        let skill_dir = self.config.skill_root.join(&skill_id);
        if skill_dir.exists() {
            return Ok(SkillApplyResult {
                skill_id,
                status: "already_installed".to_string(),
                message: "skill already exists; use update to evaluate upgrade behavior / skill 已存在，请使用 update 评估升级行为".to_string(),
            });
        }
        Ok(SkillApplyResult {
            skill_id,
            status: "not_implemented".to_string(),
            message: "skill install flow is not implemented yet / 技能安装流程尚未实现".to_string(),
        })
    }

    /// English: Preflight one update request and return a structured placeholder result.
    /// 对单个更新请求执行预检查并返回结构化占位结果。
    pub fn update_skill(
        &self,
        plane: SkillOperationPlane,
        request: &SkillInstallRequest,
    ) -> Result<SkillApplyResult, String> {
        let skill_id = request
            .skill_id
            .as_deref()
            .ok_or_else(|| "update request requires skill_id / update 请求必须提供 skill_id".to_string())?
            .trim()
            .to_string();
        self.guard_operation(plane, SkillLifecycleAction::Update, &skill_id)?;
        let skill_dir = self.config.skill_root.join(&skill_id);
        if !skill_dir.exists() {
            return Ok(SkillApplyResult {
                skill_id,
                status: "missing_skill".to_string(),
                message: "skill is not installed; use install first / skill 尚未安装，请先执行 install".to_string(),
            });
        }
        Ok(SkillApplyResult {
            skill_id,
            status: "not_implemented".to_string(),
            message: "skill update flow is not implemented yet / 技能更新流程尚未实现".to_string(),
        })
    }

    /// English: Return the configured installed skill root.
    /// 返回当前配置中的已安装技能根目录。
    pub fn skill_root(&self) -> &Path {
        &self.config.skill_root
    }

    /// English: Return the configured skill-state root.
    /// 返回当前配置中的技能状态根目录。
    pub fn state_root(&self) -> &Path {
        &self.config.state_root
    }

    /// English: Return the root directory used to store disabled-state markers.
    /// 返回用于存放停用状态标记的根目录。
    fn disabled_root(&self) -> PathBuf {
        self.config.state_root.join("disabled")
    }

    /// English: Return the JSON state file path used by one disabled skill.
    /// 返回单个已停用技能对应的 JSON 状态文件路径。
    fn disabled_record_path(&self, skill_id: &str) -> PathBuf {
        self.disabled_root().join(format!("{}.json", skill_id))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        SkillInstallRequest, SkillLifecycleAction, SkillManager, SkillManagerConfig,
        SkillOperationPlane, SkillProtectionConfig,
    };

    /// English: Verify that disable/enable operations persist and clear state markers correctly.
    /// 验证停用/启用操作会正确持久化并清理状态标记。
    #[test]
    fn skill_manager_persists_disabled_state() {
        let temp_root = std::env::temp_dir().join(format!(
            "vulcan_luaskills_skill_manager_test_{}",
            std::process::id()
        ));
        if temp_root.exists() {
            let _ = std::fs::remove_dir_all(&temp_root);
        }
        let manager = SkillManager::new(SkillManagerConfig {
            skill_root: temp_root.join("skills"),
            state_root: temp_root.join("state"),
            protection: SkillProtectionConfig::default(),
        });

        assert!(manager.is_skill_enabled("vulcan-codekit").unwrap());
        manager
            .disable_skill("vulcan-codekit", Some("manual test"))
            .expect("disable should succeed");
        assert!(!manager.is_skill_enabled("vulcan-codekit").unwrap());
        assert_eq!(
            manager
                .disabled_record("vulcan-codekit")
                .unwrap()
                .expect("record should exist")
                .reason
                .as_deref(),
            Some("manual test")
        );

        manager
            .enable_skill("vulcan-codekit")
            .expect("enable should succeed");
        assert!(manager.is_skill_enabled("vulcan-codekit").unwrap());

        let _ = std::fs::remove_dir_all(&temp_root);
    }

    /// English: Verify that protected skills are blocked in the skills plane but still allowed in the system plane.
    /// 验证受保护技能会在 skills 平面被阻止，但在 system 平面仍然允许。
    #[test]
    fn protected_skills_are_blocked_only_in_skills_plane() {
        let temp_root = std::env::temp_dir().join(format!(
            "vulcan_luaskills_protection_test_{}",
            std::process::id()
        ));
        let manager = SkillManager::new(SkillManagerConfig {
            skill_root: temp_root.join("skills"),
            state_root: temp_root.join("state"),
            protection: SkillProtectionConfig {
                protected_skill_ids: vec!["vulcan-runtime".to_string()],
            },
        });

        assert!(manager
            .guard_operation(
                SkillOperationPlane::Skills,
                SkillLifecycleAction::Disable,
                "vulcan-runtime"
            )
            .is_err());
        assert!(manager
            .guard_operation(
                SkillOperationPlane::System,
                SkillLifecycleAction::Disable,
                "vulcan-runtime"
            )
            .is_ok());
    }

    /// English: Verify that install/update placeholders enforce protection and return structured states.
    /// 验证 install/update 占位入口会执行保护判断并返回结构化状态。
    #[test]
    fn install_update_placeholders_return_structured_results() {
        let temp_root = std::env::temp_dir().join(format!(
            "vulcan_luaskills_install_update_test_{}",
            std::process::id()
        ));
        let skill_root = temp_root.join("skills");
        let _ = std::fs::create_dir_all(&skill_root);
        let manager = SkillManager::new(SkillManagerConfig {
            skill_root: skill_root.clone(),
            state_root: temp_root.join("state"),
            protection: SkillProtectionConfig {
                protected_skill_ids: vec!["vulcan-runtime".to_string()],
            },
        });

        assert!(manager
            .install_skill(
                SkillOperationPlane::Skills,
                &SkillInstallRequest {
                    skill_id: Some("vulcan-runtime".to_string()),
                    source: None,
                }
            )
            .is_err());

        let install_result = manager
            .install_skill(
                SkillOperationPlane::Skills,
                &SkillInstallRequest {
                    skill_id: Some("vulcan-codekit".to_string()),
                    source: None,
                },
            )
            .expect("install placeholder should return one structured result");
        assert_eq!(install_result.status, "not_implemented");

        let _ = std::fs::create_dir_all(skill_root.join("vulcan-codekit"));
        let update_result = manager
            .update_skill(
                SkillOperationPlane::Skills,
                &SkillInstallRequest {
                    skill_id: Some("vulcan-codekit".to_string()),
                    source: None,
                },
            )
            .expect("update placeholder should return one structured result");
        assert_eq!(update_result.status, "not_implemented");

        let _ = std::fs::remove_dir_all(&temp_root);
    }

    /// English: Verify that uninstall removes the skill directory but keeps database flags unset by default.
    /// 验证卸载会删除技能目录，同时默认不声明数据库已删除。
    #[test]
    fn uninstall_returns_safe_default_database_flags() {
        let temp_root = std::env::temp_dir().join(format!(
            "vulcan_luaskills_uninstall_result_test_{}",
            std::process::id()
        ));
        if temp_root.exists() {
            let _ = std::fs::remove_dir_all(&temp_root);
        }
        let skill_root = temp_root.join("skills");
        let manager = SkillManager::new(SkillManagerConfig {
            skill_root: skill_root.clone(),
            state_root: temp_root.join("state"),
            protection: SkillProtectionConfig::default(),
        });
        let _ = std::fs::create_dir_all(skill_root.join("vulcan-codekit"));

        let result = manager
            .uninstall_skill("vulcan-codekit")
            .expect("uninstall should succeed");
        assert!(result.skill_removed);
        assert!(!result.sqlite_removed);
        assert!(!result.lancedb_removed);
        assert!(!skill_root.join("vulcan-codekit").exists());

        let _ = std::fs::remove_dir_all(&temp_root);
    }
}
