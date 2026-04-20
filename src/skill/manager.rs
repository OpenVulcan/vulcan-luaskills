use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use semver::Version;
use serde::{Deserialize, Serialize};

use crate::download::archive::extract_skill_package_zip;
use crate::download::manager::{DownloadManager, DownloadManagerConfig};
use crate::host::options::RuntimeSkillRoot;
use crate::lua_skill::{SkillMeta, validate_luaskills_identifier, validate_luaskills_version};
use crate::skill::source::{
    InstalledSkillRecord, InstalledSkillSourceRecord, SkillInstallSourceType,
};

/// Lifecycle operations that the LuaSkills manager layer exposes for one skill.
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

/// Logical operation plane used to distinguish host system controls from ordinary skill controls.
/// 用于区分宿主系统控制面与普通技能控制面的逻辑操作平面。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SkillOperationPlane {
    Skills,
    System,
}

/// Host-owned protection configuration that reserves specific skill identifiers.
/// 由宿主持有的保护配置，用于保留特定技能标识符。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillProtectionConfig {
    /// Reserved protected skill identifiers that cannot be handled through the `skills` plane.
    /// 受保护的保留技能标识符列表，禁止通过 `skills` 平面处理。
    #[serde(default)]
    pub protected_skill_ids: Vec<String>,
}

/// High-level manager configuration that defines where installed skills and their state are stored.
/// 定义已安装技能及其状态存放位置的高层管理配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillManagerConfig {
    /// Named skill root whose lifecycle state is managed by the current manager instance.
    /// 当前管理器实例所管理的命名技能根。
    pub skill_root: RuntimeSkillRoot,
    /// Root directory where lifecycle sidecar state of the current named skill root is persisted.
    /// 当前命名技能根生命周期旁路状态的持久化根目录。
    pub lifecycle_root: PathBuf,
    /// Host-owned protection policy that reserves core skill identifiers.
    /// 宿主拥有的保护策略，用于保留核心技能标识符。
    #[serde(default)]
    pub protection: SkillProtectionConfig,
    /// Root directory used to cache downloaded skill packages and remote manifests.
    /// 用于缓存下载技能包与远程清单的根目录。
    pub download_cache_root: PathBuf,
    /// Whether managed skill install/update flows may access the network.
    /// 受管技能安装/更新流程是否允许访问网络。
    pub allow_network_download: bool,
    /// Optional GitHub site base URL override used by managed GitHub installs.
    /// 受管 GitHub 安装使用的可选 GitHub 站点基址覆盖。
    #[serde(default)]
    pub github_base_url: Option<String>,
    /// Optional GitHub API base URL override used by managed GitHub installs.
    /// 受管 GitHub 安装使用的可选 GitHub API 基址覆盖。
    #[serde(default)]
    pub github_api_base_url: Option<String>,
}

/// One install request accepted by the future LuaSkills manager entrypoints.
/// 未来 LuaSkills 管理入口接受的单次安装请求定义。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillInstallRequest {
    /// Optional skill id used for install-by-name flows.
    /// 供按名称安装流程使用的可选 skill id。
    pub skill_id: Option<String>,
    /// Optional raw source string such as URL or local directory.
    /// 例如 URL 或本地目录一类的可选原始来源字符串。
    pub source: Option<String>,
    /// Source type used to interpret the source locator. Defaults to GitHub.
    /// 用于解释来源定位值的来源类型，默认使用 GitHub。
    #[serde(default)]
    pub source_type: SkillInstallSourceType,
}

/// One install or update result returned by the skill manager.
/// 由技能管理器返回的单次安装或更新结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillApplyResult {
    /// Stable skill identifier targeted by the current operation.
    /// 当前操作目标的稳定技能标识符。
    pub skill_id: String,
    /// High-level result status such as blocked, already_installed, or not_implemented.
    /// 高层结果状态，例如 blocked、already_installed 或 not_implemented。
    pub status: String,
    /// Human-readable explanation of the current result.
    /// 当前结果的人类可读解释文本。
    pub message: String,
    /// Optional semantic version involved in the current install/update result.
    /// 当前安装/更新结果涉及的可选语义化版本。
    #[serde(default)]
    pub version: Option<String>,
    /// Optional managed install source type involved in the current result.
    /// 当前结果涉及的可选受管安装来源类型。
    #[serde(default)]
    pub source_type: Option<SkillInstallSourceType>,
    /// Optional stable source locator involved in the current result.
    /// 当前结果涉及的可选稳定来源定位值。
    #[serde(default)]
    pub source_locator: Option<String>,
}

/// One staged install/update mutation that is not committed until runtime reload succeeds.
/// 单个尚未提交的安装/更新变更，只有运行时重载成功后才会最终提交。
#[derive(Debug, Clone)]
pub enum PreparedSkillApply {
    /// One immediate result that does not mutate disk state.
    /// 一个不会修改磁盘状态的即时结果。
    Immediate(SkillApplyResult),
    /// One staged install mutation waiting for commit or rollback.
    /// 一个等待提交或回滚的已暂存安装变更。
    Install(PreparedSkillInstall),
    /// One staged update mutation waiting for commit or rollback.
    /// 一个等待提交或回滚的已暂存更新变更。
    Update(PreparedSkillUpdate),
}

/// One staged install mutation prepared before the runtime reload is attempted.
/// 在尝试运行时重载之前准备好的单次安装暂存变更。
#[derive(Debug, Clone)]
pub struct PreparedSkillInstall {
    /// Structured install result returned after the staged install succeeds.
    /// 暂存安装成功后返回的结构化安装结果。
    pub result: SkillApplyResult,
    /// Final target directory where the installed skill has been staged.
    /// 已暂存安装技能的最终目标目录。
    pub target_dir: PathBuf,
    /// Install record that should be persisted only after runtime reload succeeds.
    /// 只有运行时重载成功后才应持久化的安装记录。
    pub install_record: InstalledSkillRecord,
}

/// One staged update mutation prepared before the runtime reload is attempted.
/// 在尝试运行时重载之前准备好的单次更新暂存变更。
#[derive(Debug, Clone)]
pub struct PreparedSkillUpdate {
    /// Structured update result returned after the staged update succeeds.
    /// 暂存更新成功后返回的结构化更新结果。
    pub result: SkillApplyResult,
    /// Final target directory currently holding the staged new skill package.
    /// 当前持有已暂存新技能包的最终目标目录。
    pub target_dir: PathBuf,
    /// Backup directory that still contains the previous skill package until commit completes.
    /// 在提交完成前仍保存旧技能包的备份目录。
    pub backup_dir: PathBuf,
    /// Updated install record that should be persisted only after runtime reload succeeds.
    /// 只有运行时重载成功后才应持久化的更新后安装记录。
    pub install_record: InstalledSkillRecord,
    /// Previous install record that should be restored if the update commit partially fails.
    /// 如果更新提交发生部分失败则需要恢复的旧安装记录。
    pub previous_install_record: InstalledSkillRecord,
}

/// One staged uninstall mutation prepared before the runtime reload is attempted.
/// 在尝试运行时重载之前准备好的单次卸载暂存变更。
#[derive(Debug, Clone)]
pub struct PreparedSkillUninstall {
    /// Structured uninstall result returned after the staged uninstall succeeds.
    /// 暂存卸载成功后返回的结构化卸载结果。
    pub result: SkillUninstallResult,
    /// Final target directory currently reserved for the installed skill.
    /// 当前为已安装技能保留的最终目标目录。
    pub target_dir: PathBuf,
    /// Backup directory that still contains the previous skill package until commit completes.
    /// 在提交完成前仍保存旧技能包的备份目录。
    pub backup_dir: Option<PathBuf>,
    /// Previous disabled-state record that should be restored if uninstall rollback is needed.
    /// 如果需要回滚卸载则应恢复的旧停用状态记录。
    pub previous_disabled_record: Option<DisabledSkillRecord>,
    /// Previous managed install record that should be restored if uninstall rollback is needed.
    /// 如果需要回滚卸载则应恢复的旧受管安装记录。
    pub previous_install_record: Option<InstalledSkillRecord>,
}

/// Optional database cleanup switches accepted by skill uninstall operations.
/// 技能卸载操作接受的可选数据库清理开关集合。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillUninstallOptions {
    /// Remove the SQLite database directory owned by the target skill when true.
    /// 为 true 时删除目标技能拥有的 SQLite 数据目录。
    #[serde(default)]
    pub remove_sqlite: bool,
    /// Remove the LanceDB database directory owned by the target skill when true.
    /// 为 true 时删除目标技能拥有的 LanceDB 数据目录。
    #[serde(default)]
    pub remove_lancedb: bool,
}

/// Structured uninstall result that reports whether code and databases were removed or retained.
/// 结构化卸载结果，用于报告代码与数据库是被删除还是被保留。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillUninstallResult {
    /// Stable skill identifier targeted by the current uninstall action.
    /// 当前卸载动作目标的稳定技能标识符。
    pub skill_id: String,
    /// Whether the skill package directory itself was removed.
    /// skill 包目录本身是否已经被删除。
    pub skill_removed: bool,
    /// Whether the SQLite database directory was removed explicitly.
    /// SQLite 数据目录是否已被显式删除。
    pub sqlite_removed: bool,
    /// Whether the LanceDB database directory was removed explicitly.
    /// LanceDB 数据目录是否已被显式删除。
    pub lancedb_removed: bool,
    /// Whether the SQLite database directory was intentionally retained.
    /// SQLite 数据目录是否被有意保留。
    pub sqlite_retained: bool,
    /// Whether the LanceDB database directory was intentionally retained.
    /// LanceDB 数据目录是否被有意保留。
    pub lancedb_retained: bool,
    /// Human-readable explanation of the uninstall result.
    /// 当前卸载结果的人类可读说明文本。
    pub message: String,
}

/// One resolved effective skill instance after applying root precedence rules.
/// 应用根目录优先级规则后得到的单个生效技能实例。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedSkillInstance {
    /// Stable skill identifier resolved from the directory name.
    /// 从目录名称解析出的稳定技能标识符。
    pub skill_id: String,
    /// Named skill root that currently owns the effective skill instance.
    /// 当前生效技能实例所属的命名技能根。
    pub root_name: String,
    /// Physical skills root directory that currently owns the effective skill instance.
    /// 当前生效技能实例所属的物理 skills 根目录。
    pub skills_root: PathBuf,
    /// Physical skill directory that is currently effective for the resolved skill id.
    /// 当前针对该技能标识符实际生效的物理技能目录。
    pub actual_dir: PathBuf,
}

/// Persistent record written when one skill is explicitly disabled.
/// 显式停用某个技能时写入的持久化记录。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisabledSkillRecord {
    /// Stable skill identifier bound to this state record.
    /// 与当前状态记录绑定的稳定 skill 标识符。
    pub skill_id: String,
    /// Optional human-readable disable reason.
    /// 可选的人类可读停用原因。
    pub reason: Option<String>,
    /// Unix timestamp in milliseconds when the skill was disabled.
    /// 当前技能被停用时的 Unix 毫秒时间戳。
    pub disabled_at_unix_ms: u128,
}

/// Skill manager that owns persisted skill enabled/disabled state.
/// 持有技能启用/停用持久状态的技能管理器。
pub struct SkillManager {
    config: SkillManagerConfig,
}

impl SkillManager {
    /// Create one skill manager from a shared configuration object.
    /// 基于共享配置对象创建一个技能管理器实例。
    pub fn new(config: SkillManagerConfig) -> Self {
        Self { config }
    }

    /// Ensure the skill-state root and its child directories exist.
    /// 确保技能状态根目录及其子目录已经存在。
    pub fn ensure_state_layout(&self) -> Result<(), String> {
        fs::create_dir_all(self.disabled_root()).map_err(|error| {
            format!(
                "Failed to create disabled root {}: {}",
                self.disabled_root().display(),
                error
            )
        })?;
        fs::create_dir_all(self.install_record_root()).map_err(|error| {
            format!(
                "Failed to create install-record root {}: {}",
                self.install_record_root().display(),
                error
            )
        })
    }

    /// Validate one skill id and enforce the plane-specific protection rules.
    /// 校验单个 skill id 并执行按平面划分的保护规则。
    pub fn guard_operation(
        &self,
        plane: SkillOperationPlane,
        action: SkillLifecycleAction,
        skill_id: &str,
    ) -> Result<(), String> {
        validate_luaskills_identifier(skill_id, "skill_id")?;
        if self.is_protected_skill(skill_id) && plane == SkillOperationPlane::Skills {
            return Err(format!("protected skill '{}' cannot be processed through the skills plane for action {:?}", skill_id, action));
        }
        Ok(())
    }

    /// Return whether one skill identifier is reserved by the host protection policy.
    /// 返回单个技能标识符是否被宿主保护策略保留。
    pub fn is_protected_skill(&self, skill_id: &str) -> bool {
        self.config
            .protection
            .protected_skill_ids
            .iter()
            .any(|protected| protected.trim() == skill_id)
    }

    /// Return whether one skill is currently enabled.
    /// 返回单个技能当前是否处于启用状态。
    pub fn is_skill_enabled(&self, skill_id: &str) -> Result<bool, String> {
        self.ensure_state_layout()?;
        Ok(!self.disabled_record_path(skill_id).exists())
    }

    /// Persist one disabled-state marker for the specified skill.
    /// 为指定技能持久化一份停用状态标记。
    pub fn disable_skill(&self, skill_id: &str, reason: Option<&str>) -> Result<(), String> {
        self.disable_skill_in_plane(SkillOperationPlane::Skills, skill_id, reason)
    }

    /// Persist one disabled-state marker for the specified skill in the requested operation plane.
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

    /// Remove the disabled-state marker for one skill.
    /// 删除单个技能的停用状态标记。
    pub fn enable_skill(&self, skill_id: &str) -> Result<(), String> {
        self.enable_skill_in_plane(SkillOperationPlane::Skills, skill_id)
    }

    /// Remove the disabled-state marker for one skill in the requested operation plane.
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

    /// Read the disabled-state record for one skill when it exists.
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

    /// Remove one installed skill directory and clear its disabled marker.
    /// 删除单个已安装 skill 目录，并清理其停用标记。
    pub fn uninstall_skill(&self, skill_id: &str) -> Result<SkillUninstallResult, String> {
        self.uninstall_skill_in_plane(SkillOperationPlane::Skills, skill_id)
    }

    /// Remove one installed skill directory and clear its disabled marker in the requested operation plane.
    /// 在指定操作平面删除单个已安装技能目录，并清理其停用标记。
    pub fn uninstall_skill_in_plane(
        &self,
        plane: SkillOperationPlane,
        skill_id: &str,
    ) -> Result<SkillUninstallResult, String> {
        let skill_dir = self.config.skill_root.skills_dir.join(skill_id);
        let prepared = self.prepare_uninstall_skill_at_path_in_plane(plane, skill_id, &skill_dir)?;
        self.commit_prepared_skill_uninstall(&prepared).map_err(|error| {
            let rollback_error = self.rollback_prepared_skill_uninstall(&prepared);
            let rollback_message = rollback_error
                .err()
                .map(|rollback| format!(" rollback failed: {}", rollback))
                .unwrap_or_default();
            format!("Failed to finalize uninstall: {}.{}", error, rollback_message)
        })
    }

    /// Remove one installed skill directory at an explicitly resolved path and clear its disabled marker.
    /// 删除单个已解析物理路径上的技能目录，并清理其停用标记。
    pub fn uninstall_skill_at_path_in_plane(
        &self,
        plane: SkillOperationPlane,
        skill_id: &str,
        skill_dir: &Path,
    ) -> Result<SkillUninstallResult, String> {
        let prepared = self.prepare_uninstall_skill_at_path_in_plane(plane, skill_id, skill_dir)?;
        self.commit_prepared_skill_uninstall(&prepared).map_err(|error| {
            let rollback_error = self.rollback_prepared_skill_uninstall(&prepared);
            let rollback_message = rollback_error
                .err()
                .map(|rollback| format!(" rollback failed: {}", rollback))
                .unwrap_or_default();
            format!("Failed to finalize uninstall: {}.{}", error, rollback_message)
        })
    }

    /// Prepare one uninstall request and stage filesystem changes without committing state deletions yet.
    /// 预处理单个卸载请求并暂存文件系统变更，但暂不提交状态删除。
    pub fn prepare_uninstall_skill_at_path_in_plane(
        &self,
        plane: SkillOperationPlane,
        skill_id: &str,
        skill_dir: &Path,
    ) -> Result<PreparedSkillUninstall, String> {
        self.guard_operation(plane, SkillLifecycleAction::Uninstall, skill_id)?;
        self.ensure_state_layout()?;
        let previous_disabled_record = self.disabled_record(skill_id)?;
        let previous_install_record = self.install_record(skill_id)?;
        let (skill_removed, backup_dir) = if skill_dir.exists() {
            let backup_dir = self
                .config
                .lifecycle_root
                .join("uninstall_backup")
                .join(format!("{}-{}", skill_id, current_unix_millis()));
            if let Some(parent) = backup_dir.parent() {
                fs::create_dir_all(parent)
                    .map_err(|error| format!("Failed to create {}: {}", parent.display(), error))?;
            }
            fs::rename(skill_dir, &backup_dir).map_err(|error| {
                format!(
                    "Failed to move current skill {} into uninstall backup {}: {}",
                    skill_dir.display(),
                    backup_dir.display(),
                    error
                )
            })?;
            (true, Some(backup_dir))
        } else {
            (false, None)
        };
        Ok(PreparedSkillUninstall {
            result: SkillUninstallResult {
                skill_id: skill_id.to_string(),
                skill_removed,
                sqlite_removed: false,
                lancedb_removed: false,
                sqlite_retained: false,
                lancedb_retained: false,
                message: if skill_removed {
                    "skill package removed".to_string()
                } else {
                    "skill package directory not found".to_string()
                },
            },
            target_dir: skill_dir.to_path_buf(),
            backup_dir,
            previous_disabled_record,
            previous_install_record,
        })
    }

    /// Prepare one install request and stage filesystem changes without committing the install record yet.
    /// 预处理单个安装请求并暂存文件系统变更，但暂不提交安装记录。
    pub fn prepare_install_skill(
        &self,
        plane: SkillOperationPlane,
        skill_roots: &[RuntimeSkillRoot],
        request: &SkillInstallRequest,
    ) -> Result<PreparedSkillApply, String> {
        let skill_id = resolve_requested_skill_id(request)?;
        self.guard_operation(plane, SkillLifecycleAction::Install, &skill_id)?;
        if resolve_declared_skill_instance_from_roots(skill_roots, &skill_id)?.is_some() {
            return Ok(PreparedSkillApply::Immediate(SkillApplyResult {
                skill_id,
                status: "already_installed".to_string(),
                message: "skill already exists; use update to evaluate upgrade behavior".to_string(),
                version: None,
                source_type: None,
                source_locator: None,
            }));
        }
        match request.source_type {
            SkillInstallSourceType::Github => self.prepare_install_skill_from_github(&skill_id, request),
            SkillInstallSourceType::Url => Err(
                "managed URL install is not implemented yet; GitHub install is currently the only supported install source"
                    .to_string(),
            ),
        }
    }

    /// Prepare one update request and stage filesystem changes without committing the new install record yet.
    /// 预处理单个更新请求并暂存文件系统变更，但暂不提交新的安装记录。
    pub fn prepare_update_skill(
        &self,
        plane: SkillOperationPlane,
        skill_roots: &[RuntimeSkillRoot],
        request: &SkillInstallRequest,
    ) -> Result<PreparedSkillApply, String> {
        let skill_id = resolve_requested_skill_id(request)?;
        self.guard_operation(plane, SkillLifecycleAction::Update, &skill_id)?;
        if resolve_declared_skill_instance_from_roots(skill_roots, &skill_id)?.is_none() {
            return Ok(PreparedSkillApply::Immediate(SkillApplyResult {
                skill_id,
                status: "missing_skill".to_string(),
                message: "skill is not installed; use install first".to_string(),
                version: None,
                source_type: None,
                source_locator: None,
            }));
        }
        self.prepare_github_managed_skill_update(&skill_id)
    }

    /// Stage one skill package install from the latest GitHub release of the declared repository.
    /// 从声明仓库的最新 GitHub release 暂存单个技能包安装。
    fn prepare_install_skill_from_github(
        &self,
        skill_id: &str,
        request: &SkillInstallRequest,
    ) -> Result<PreparedSkillApply, String> {
        let repo = normalize_github_repo_locator(
            request
                .source
                .as_deref()
                .ok_or_else(|| "github install requires source repository".to_string())?,
        )?;
        let repo_skill_id = github_repo_skill_id(&repo)?;
        if repo_skill_id != skill_id {
            return Err(format!(
                "github repository '{}' resolves to skill_id '{}' but the request targets '{}'",
                repo, repo_skill_id, skill_id
            ));
        }

        let downloader = self.downloader();
        let asset = downloader.resolve_github_release_asset(
            &crate::skill::dependencies::GithubReleaseSourceSpec {
                repo: repo.clone(),
                tag_api: None,
            },
            &format!("{}-v{{version}}-skill.zip", skill_id),
            None,
        )?;
        let archive_path = downloader.download(&crate::download::manager::DownloadRequest {
            source_type: crate::dependency::types::DependencySourceType::GithubRelease,
            source_locator: asset.download_url.clone(),
            cache_key: managed_skill_cache_key(skill_id, asset.version.as_str()),
        })?;

        let install_temp_root = self
            .config
            .lifecycle_root
            .join("install_tmp")
            .join(format!("{}-{}", skill_id, current_unix_millis()));
        if install_temp_root.exists() {
            fs::remove_dir_all(&install_temp_root).map_err(|error| {
                format!(
                    "Failed to remove stale temp install root {}: {}",
                    install_temp_root.display(),
                    error
                )
            })?;
        }
        fs::create_dir_all(&install_temp_root).map_err(|error| {
            format!(
                "Failed to create temp install root {}: {}",
                install_temp_root.display(),
                error
            )
        })?;

        let extracted_skill_dir =
            extract_skill_package_zip(&archive_path, &install_temp_root, skill_id)?;
        let installed_meta = read_skill_manifest_from_directory(&extracted_skill_dir)?;
        if installed_meta.effective_skill_id() != skill_id {
            return Err(format!(
                "downloaded skill package resolves to skill_id '{}' instead of '{}'",
                installed_meta.effective_skill_id(),
                skill_id
            ));
        }
        if installed_meta.version() != asset.version {
            return Err(format!(
                "downloaded skill package version '{}' does not match release version '{}'",
                installed_meta.version(),
                asset.version
            ));
        }

        let target_dir = self.skill_root().join(skill_id);
        if target_dir.exists() {
            return Err(format!(
                "target skill directory {} already exists",
                target_dir.display()
            ));
        }
        fs::rename(&extracted_skill_dir, &target_dir).map_err(|error| {
            format!(
                "Failed to move extracted skill {} into {}: {}",
                extracted_skill_dir.display(),
                target_dir.display(),
                error
            )
        })?;
        let _ = fs::remove_dir_all(&install_temp_root);

        let record = InstalledSkillRecord {
            skill_id: skill_id.to_string(),
            version: asset.version.clone(),
            managed: true,
            source: InstalledSkillSourceRecord {
                source_type: SkillInstallSourceType::Github,
                locator: repo.clone(),
                tag: Some(asset.tag_name.clone()),
            },
            installed_at_unix_ms: current_unix_millis(),
        };
        Ok(PreparedSkillApply::Install(PreparedSkillInstall {
            result: SkillApplyResult {
                skill_id: skill_id.to_string(),
                status: "installed".to_string(),
                message: format!(
                    "skill '{}' version {} was installed from GitHub repository '{}'",
                    skill_id, asset.version, repo
                ),
                version: Some(asset.version),
                source_type: Some(SkillInstallSourceType::Github),
                source_locator: Some(repo),
            },
            target_dir,
            install_record: record,
        }))
    }

    /// Stage one managed GitHub-installed skill update by comparing the latest release tag with the current installed version.
    /// 通过比较最新 release 标签与当前已安装版本来暂存单个 GitHub 受管技能更新。
    fn prepare_github_managed_skill_update(&self, skill_id: &str) -> Result<PreparedSkillApply, String> {
        let record = self
            .install_record(skill_id)?
            .ok_or_else(|| {
                format!(
                    "skill '{}' is not managed by the install workflow; automatic update is unavailable",
                    skill_id
                )
            })?;
        if !record.managed {
            return Err(format!(
                "skill '{}' is not managed by the install workflow; automatic update is unavailable",
                skill_id
            ));
        }
        if record.source.source_type != SkillInstallSourceType::Github {
            return Err(format!(
                "skill '{}' uses source type '{:?}', but update currently supports only github",
                skill_id, record.source.source_type
            ));
        }

        let current_version = Version::parse(record.version.as_str()).map_err(|error| {
            format!(
                "installed version '{}' of skill '{}' is invalid: {}",
                record.version, skill_id, error
            )
        })?;
        let downloader = self.downloader();
        let asset = downloader.resolve_github_release_asset(
            &crate::skill::dependencies::GithubReleaseSourceSpec {
                repo: record.source.locator.clone(),
                tag_api: None,
            },
            &format!("{}-v{{version}}-skill.zip", skill_id),
            None,
        )?;
        let latest_version = Version::parse(asset.version.as_str()).map_err(|error| {
            format!(
                "latest GitHub release version '{}' of skill '{}' is invalid: {}",
                asset.version, skill_id, error
            )
        })?;
        if latest_version <= current_version {
            return Ok(PreparedSkillApply::Immediate(SkillApplyResult {
                skill_id: skill_id.to_string(),
                status: "up_to_date".to_string(),
                message: format!(
                    "skill '{}' is already on version {}",
                    skill_id, record.version
                ),
                version: Some(record.version),
                source_type: Some(SkillInstallSourceType::Github),
                source_locator: Some(record.source.locator),
            }));
        }

        let archive_path = downloader.download(&crate::download::manager::DownloadRequest {
            source_type: crate::dependency::types::DependencySourceType::GithubRelease,
            source_locator: asset.download_url.clone(),
            cache_key: managed_skill_cache_key(skill_id, asset.version.as_str()),
        })?;
        let temp_root = self
            .config
            .lifecycle_root
            .join("update_tmp")
            .join(format!("{}-{}", skill_id, current_unix_millis()));
        if temp_root.exists() {
            fs::remove_dir_all(&temp_root).map_err(|error| {
                format!(
                    "Failed to remove stale temp update root {}: {}",
                    temp_root.display(),
                    error
                )
            })?;
        }
        fs::create_dir_all(&temp_root).map_err(|error| {
            format!(
                "Failed to create temp update root {}: {}",
                temp_root.display(),
                error
            )
        })?;
        let extracted_skill_dir = extract_skill_package_zip(&archive_path, &temp_root, skill_id)?;
        let updated_meta = read_skill_manifest_from_directory(&extracted_skill_dir)?;
        if updated_meta.version() != asset.version {
            return Err(format!(
                "downloaded update package version '{}' does not match release version '{}'",
                updated_meta.version(),
                asset.version
            ));
        }

        let target_dir = self.skill_root().join(skill_id);
        if !target_dir.exists() {
            return Err(format!(
                "installed skill directory {} does not exist",
                target_dir.display()
            ));
        }
        let backup_dir = self
            .config
            .lifecycle_root
            .join("update_backup")
            .join(format!("{}-{}", skill_id, current_unix_millis()));
        if let Some(parent) = backup_dir.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!("Failed to create {}: {}", parent.display(), error)
            })?;
        }
        fs::rename(&target_dir, &backup_dir).map_err(|error| {
            format!(
                "Failed to move current skill {} into backup {}: {}",
                target_dir.display(),
                backup_dir.display(),
                error
            )
        })?;
        if let Err(error) = fs::rename(&extracted_skill_dir, &target_dir) {
            let _ = fs::rename(&backup_dir, &target_dir);
            return Err(format!(
                "Failed to move updated skill {} into {}: {}",
                extracted_skill_dir.display(),
                target_dir.display(),
                error
            ));
        }
        let _ = fs::remove_dir_all(&temp_root);

        let updated_record = InstalledSkillRecord {
            skill_id: skill_id.to_string(),
            version: asset.version.clone(),
            managed: true,
            source: InstalledSkillSourceRecord {
                source_type: SkillInstallSourceType::Github,
                locator: record.source.locator.clone(),
                tag: Some(asset.tag_name.clone()),
            },
            installed_at_unix_ms: current_unix_millis(),
        };
        Ok(PreparedSkillApply::Update(PreparedSkillUpdate {
            result: SkillApplyResult {
                skill_id: skill_id.to_string(),
                status: "updated".to_string(),
                message: format!(
                    "skill '{}' was updated from version {} to {}",
                    skill_id, record.version, asset.version
                ),
                version: Some(asset.version),
                source_type: Some(SkillInstallSourceType::Github),
                source_locator: Some(record.source.locator.clone()),
            },
            target_dir,
            backup_dir,
            install_record: updated_record,
            previous_install_record: record,
        }))
    }

    /// Return the configured installed skill root.
    /// 返回当前配置中的已安装技能根目录。
    pub fn skill_root(&self) -> &Path {
        &self.config.skill_root.skills_dir
    }

    /// Return the configured skill-state root.
    /// 返回当前配置中的技能状态根目录。
    pub fn state_root(&self) -> &Path {
        self.config.lifecycle_root.as_path()
    }

    /// Return the root directory used to store managed install records.
    /// 返回用于存放受管安装记录的根目录。
    fn install_record_root(&self) -> PathBuf {
        self.config.lifecycle_root.join("installs")
    }

    /// Return the root directory used to store disabled-state markers.
    /// 返回用于存放停用状态标记的根目录。
    fn disabled_root(&self) -> PathBuf {
        self.config
            .lifecycle_root
            .join("skills")
            .join("disabled")
    }

    /// Return the JSON state file path used by one disabled skill.
    /// 返回单个已停用技能对应的 JSON 状态文件路径。
    fn disabled_record_path(&self, skill_id: &str) -> PathBuf {
        self.disabled_root().join(format!("{}.json", skill_id))
    }

    /// Return the YAML install-record path used by one managed skill.
    /// 返回单个受管技能使用的 YAML 安装记录路径。
    fn install_record_path(&self, skill_id: &str) -> PathBuf {
        self.install_record_root().join(format!("{}.yaml", skill_id))
    }

    /// Read one managed install record from disk when it exists.
    /// 在受管安装记录存在时从磁盘读取该记录。
    pub fn install_record(&self, skill_id: &str) -> Result<Option<InstalledSkillRecord>, String> {
        validate_luaskills_identifier(skill_id, "skill_id")?;
        let path = self.install_record_path(skill_id);
        if !path.exists() {
            return Ok(None);
        }
        let yaml = fs::read_to_string(&path)
            .map_err(|error| format!("Failed to read {}: {}", path.display(), error))?;
        let record: InstalledSkillRecord = serde_yaml::from_str(&yaml)
            .map_err(|error| format!("Failed to parse {}: {}", path.display(), error))?;
        Ok(Some(record))
    }

    /// Persist one managed install record to disk.
    /// 将单个受管安装记录持久化到磁盘。
    fn persist_install_record(&self, record: &InstalledSkillRecord) -> Result<(), String> {
        self.ensure_state_layout()?;
        let path = self.install_record_path(&record.skill_id);
        let yaml = serde_yaml::to_string(record)
            .map_err(|error| format!("Failed to serialize install record: {}", error))?;
        fs::write(&path, yaml)
            .map_err(|error| format!("Failed to write {}: {}", path.display(), error))
    }

    /// Remove one managed install record from disk and report whether it existed.
    /// 从磁盘删除单个受管安装记录，并返回它是否存在。
    fn remove_install_record(&self, skill_id: &str) -> Result<bool, String> {
        validate_luaskills_identifier(skill_id, "skill_id")?;
        let path = self.install_record_path(skill_id);
        if !path.exists() {
            return Ok(false);
        }
        fs::remove_file(&path)
            .map_err(|error| format!("Failed to remove {}: {}", path.display(), error))?;
        Ok(true)
    }

    /// Persist one disabled-state record exactly as captured before a staged mutation.
    /// 按暂存变更前捕获的原样持久化单个停用状态记录。
    fn persist_disabled_record(&self, record: &DisabledSkillRecord) -> Result<(), String> {
        self.ensure_state_layout()?;
        let path = self.disabled_record_path(&record.skill_id);
        let content = serde_json::to_string_pretty(record)
            .map_err(|error| format!("Failed to serialize disabled record: {}", error))?;
        fs::write(&path, content)
            .map_err(|error| format!("Failed to write {}: {}", path.display(), error))
    }

    /// Remove one disabled-state record from disk and report whether it existed.
    /// 从磁盘删除单个停用状态记录，并返回它是否存在。
    fn remove_disabled_record(&self, skill_id: &str) -> Result<bool, String> {
        validate_luaskills_identifier(skill_id, "skill_id")?;
        self.ensure_state_layout()?;
        let path = self.disabled_record_path(skill_id);
        if !path.exists() {
            return Ok(false);
        }
        fs::remove_file(&path)
            .map_err(|error| format!("Failed to remove {}: {}", path.display(), error))?;
        Ok(true)
    }

    /// Restore one previous disabled-state snapshot or remove the current record when no snapshot existed.
    /// 恢复单个旧停用状态快照，若原先不存在快照则删除当前记录。
    fn restore_disabled_record(
        &self,
        skill_id: &str,
        record: Option<&DisabledSkillRecord>,
    ) -> Result<(), String> {
        match record {
            Some(record) => self.persist_disabled_record(record),
            None => {
                self.remove_disabled_record(skill_id)?;
                Ok(())
            }
        }
    }

    /// Restore one previous install-record snapshot or remove the current record when no snapshot existed.
    /// 恢复单个旧安装记录快照，若原先不存在快照则删除当前记录。
    fn restore_install_record(
        &self,
        skill_id: &str,
        record: Option<&InstalledSkillRecord>,
    ) -> Result<(), String> {
        match record {
            Some(record) => self.persist_install_record(record),
            None => {
                self.remove_install_record(skill_id)?;
                Ok(())
            }
        }
    }

    /// Persist the final install record and remove transitional backup data after runtime reload succeeds.
    /// 在运行时重载成功后持久化最终安装记录，并移除过渡备份数据。
    pub fn commit_prepared_skill_apply(
        &self,
        prepared: &PreparedSkillApply,
    ) -> Result<SkillApplyResult, String> {
        match prepared {
            PreparedSkillApply::Immediate(result) => Ok(result.clone()),
            PreparedSkillApply::Install(prepared_install) => {
                self.persist_install_record(&prepared_install.install_record)?;
                Ok(prepared_install.result.clone())
            }
            PreparedSkillApply::Update(prepared_update) => {
                self.persist_install_record(&prepared_update.install_record)?;
                if prepared_update.backup_dir.exists() {
                    fs::remove_dir_all(&prepared_update.backup_dir).map_err(|error| {
                        let restore_error =
                            self.persist_install_record(&prepared_update.previous_install_record);
                        match restore_error {
                            Ok(()) => format!(
                                "Failed to remove update backup {}: previous install record was restored: {}",
                                prepared_update.backup_dir.display(),
                                error
                            ),
                            Err(restore_error) => format!(
                                "Failed to remove update backup {}: {}. Failed to restore previous install record: {}",
                                prepared_update.backup_dir.display(),
                                error,
                                restore_error
                            ),
                        }
                    })?;
                }
                Ok(prepared_update.result.clone())
            }
        }
    }

    /// Roll back one staged install/update mutation after reload or commit fails.
    /// 在重载或提交失败后回滚一次已暂存的安装或更新变更。
    pub fn rollback_prepared_skill_apply(&self, prepared: &PreparedSkillApply) -> Result<(), String> {
        match prepared {
            PreparedSkillApply::Immediate(_) => Ok(()),
            PreparedSkillApply::Install(prepared_install) => {
                if prepared_install.target_dir.exists() {
                    fs::remove_dir_all(&prepared_install.target_dir).map_err(|error| {
                        format!(
                            "Failed to roll back installed skill directory {}: {}",
                            prepared_install.target_dir.display(),
                            error
                        )
                    })?;
                }
                Ok(())
            }
            PreparedSkillApply::Update(prepared_update) => {
                if prepared_update.target_dir.exists() {
                    fs::remove_dir_all(&prepared_update.target_dir).map_err(|error| {
                        format!(
                            "Failed to remove staged updated skill directory {}: {}",
                            prepared_update.target_dir.display(),
                            error
                        )
                    })?;
                }
                if prepared_update.backup_dir.exists() {
                    fs::rename(&prepared_update.backup_dir, &prepared_update.target_dir).map_err(|error| {
                        format!(
                            "Failed to restore backup {} into {}: {}",
                            prepared_update.backup_dir.display(),
                            prepared_update.target_dir.display(),
                            error
                        )
                    })?;
                }
                Ok(())
            }
        }
    }

    /// Persist the final uninstall state and remove transitional backup data after runtime reload succeeds.
    /// 在运行时重载成功后持久化最终卸载状态，并移除过渡备份数据。
    pub fn commit_prepared_skill_uninstall(
        &self,
        prepared: &PreparedSkillUninstall,
    ) -> Result<SkillUninstallResult, String> {
        if prepared.previous_disabled_record.is_some() {
            self.remove_disabled_record(&prepared.result.skill_id)?;
        }
        if prepared.previous_install_record.is_some() {
            self.remove_install_record(&prepared.result.skill_id)?;
        }
        if let Some(backup_dir) = &prepared.backup_dir {
            fs::remove_dir_all(backup_dir).map_err(|error| {
                let disabled_restore_error = self.restore_disabled_record(
                    &prepared.result.skill_id,
                    prepared.previous_disabled_record.as_ref(),
                );
                let install_restore_error = self.restore_install_record(
                    &prepared.result.skill_id,
                    prepared.previous_install_record.as_ref(),
                );
                let mut message = format!(
                    "Failed to remove uninstall backup {}: {}",
                    backup_dir.display(),
                    error
                );
                if let Err(restore_error) = disabled_restore_error {
                    message.push_str(&format!(
                        ". Failed to restore previous disabled record: {}",
                        restore_error
                    ));
                }
                if let Err(restore_error) = install_restore_error {
                    message.push_str(&format!(
                        ". Failed to restore previous install record: {}",
                        restore_error
                    ));
                }
                message
            })?;
        }
        Ok(prepared.result.clone())
    }

    /// Roll back one staged uninstall mutation after reload or commit fails.
    /// 在重载或提交失败后回滚一次已暂存的卸载变更。
    pub fn rollback_prepared_skill_uninstall(
        &self,
        prepared: &PreparedSkillUninstall,
    ) -> Result<(), String> {
        if let Some(backup_dir) = &prepared.backup_dir {
            if prepared.target_dir.exists() {
                fs::remove_dir_all(&prepared.target_dir).map_err(|error| {
                    format!(
                        "Failed to remove staged uninstall target directory {}: {}",
                        prepared.target_dir.display(),
                        error
                    )
                })?;
            }
            if backup_dir.exists() {
                fs::rename(backup_dir, &prepared.target_dir).map_err(|error| {
                    format!(
                        "Failed to restore uninstall backup {} into {}: {}",
                        backup_dir.display(),
                        prepared.target_dir.display(),
                        error
                    )
                })?;
            }
        }
        self.restore_disabled_record(
            &prepared.result.skill_id,
            prepared.previous_disabled_record.as_ref(),
        )?;
        self.restore_install_record(
            &prepared.result.skill_id,
            prepared.previous_install_record.as_ref(),
        )?;
        Ok(())
    }

    /// Build one downloader configured for managed install and update flows.
    /// 为受管安装与更新流程构造单个下载器。
    fn downloader(&self) -> DownloadManager {
        DownloadManager::new(DownloadManagerConfig {
            cache_root: self.config.download_cache_root.clone(),
            allow_network_download: self.config.allow_network_download,
            github_base_url: self.config.github_base_url.clone(),
            github_api_base_url: self.config.github_api_base_url.clone(),
        })
    }
}

/// Resolve the effective request skill id, deriving it from the source locator when needed.
/// 解析当前请求的生效技能标识符，并在需要时从来源定位值派生。
fn resolve_requested_skill_id(request: &SkillInstallRequest) -> Result<String, String> {
    let explicit_skill_id = request
        .skill_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let derived_skill_id = match request.source_type {
        SkillInstallSourceType::Github => request
            .source
            .as_deref()
            .map(normalize_github_repo_locator)
            .transpose()?
            .map(|repo| github_repo_skill_id(&repo))
            .transpose()?,
        SkillInstallSourceType::Url => None,
    };
    let skill_id = explicit_skill_id
        .or(derived_skill_id)
        .ok_or_else(|| "install/update request requires skill_id or one source that can derive it".to_string())?;
    validate_luaskills_identifier(&skill_id, "skill_id")?;
    Ok(skill_id)
}

/// Normalize one GitHub repository locator into `owner/repo` form.
/// 将单个 GitHub 仓库定位值规范化为 `owner/repo` 形式。
fn normalize_github_repo_locator(source: &str) -> Result<String, String> {
    let normalized = source
        .trim()
        .trim_start_matches("https://github.com/")
        .trim_start_matches("http://github.com/")
        .trim_matches('/')
        .to_string();
    let mut segments = normalized.split('/');
    let owner = segments.next().unwrap_or_default().trim();
    let repo = segments.next().unwrap_or_default().trim();
    if owner.is_empty() || repo.is_empty() || segments.next().is_some() {
        return Err(format!(
            "github source '{}' must be one repository locator in owner/repo form",
            source
        ));
    }
    Ok(format!("{}/{}", owner, repo))
}

/// Derive one skill id from the repository segment of a GitHub locator.
/// 从 GitHub 定位值的仓库段派生单个技能标识符。
fn github_repo_skill_id(repo: &str) -> Result<String, String> {
    let skill_id = repo
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .trim()
        .to_string();
    validate_luaskills_identifier(&skill_id, "derived github skill_id")?;
    Ok(skill_id)
}

/// Build one stable download-cache key for a managed skill package.
/// 为受管技能包构造单个稳定的下载缓存键。
fn managed_skill_cache_key(skill_id: &str, version: &str) -> String {
    format!("skill-{}-{}", skill_id, version)
}

/// Return the current Unix timestamp in milliseconds.
/// 返回当前 Unix 毫秒时间戳。
fn current_unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

/// Read one extracted skill manifest from disk and bind the directory-derived skill id.
/// 从磁盘读取单个已解包技能清单，并绑定从目录派生的技能标识符。
fn read_skill_manifest_from_directory(skill_dir: &Path) -> Result<SkillMeta, String> {
    let skill_id = skill_dir
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| format!("Failed to resolve skill id from directory {}", skill_dir.display()))?
        .trim()
        .to_string();
    validate_luaskills_identifier(&skill_id, "skill_id")?;
    let skill_yaml_path = skill_dir.join("skill.yaml");
    let yaml_text = fs::read_to_string(&skill_yaml_path)
        .map_err(|error| format!("Failed to read {}: {}", skill_yaml_path.display(), error))?;
    let yaml_value: serde_yaml::Value = serde_yaml::from_str(&yaml_text)
        .map_err(|error| format!("Failed to parse {}: {}", skill_yaml_path.display(), error))?;
    if yaml_value
        .as_mapping()
        .and_then(|mapping| mapping.get(serde_yaml::Value::String("skill_id".to_string())))
        .is_some()
    {
        return Err(format!(
            "skill {} must not declare skill_id in skill.yaml; directory name is the only skill_id",
            skill_dir.display()
        ));
    }
    let mut meta: SkillMeta = serde_yaml::from_value(yaml_value)
        .map_err(|error| format!("Failed to decode {}: {}", skill_yaml_path.display(), error))?;
    meta.bind_directory_skill_id(skill_id.clone());
    validate_luaskills_version(meta.version(), "skill.yaml version")?;
    if meta.effective_skill_id() != skill_id {
        return Err(format!(
            "skill manifest in {} resolved to skill_id '{}' instead of '{}'",
            skill_yaml_path.display(),
            meta.effective_skill_id(),
            skill_id
        ));
    }
    Ok(meta)
}

/// Resolve the currently effective skill directories after applying override precedence and empty-directory disable semantics.
/// 在应用 override 优先级与空目录禁用语义后解析当前实际生效的技能目录集合。
pub fn collect_effective_skill_instances(
    base_dir: &Path,
    override_dir: Option<&Path>,
) -> Result<Vec<ResolvedSkillInstance>, String> {
    let mut roots = Vec::new();
    if let Some(override_dir) = override_dir {
        roots.push(RuntimeSkillRoot {
            name: "OVERRIDE".to_string(),
            skills_dir: override_dir.to_path_buf(),
        });
    }
    roots.push(RuntimeSkillRoot {
        name: "ROOT".to_string(),
        skills_dir: base_dir.to_path_buf(),
    });
    collect_effective_skill_instances_from_roots(&roots)
}

/// Resolve the currently effective skill directories after applying ordered root precedence rules.
/// 在应用有序根目录优先级规则后解析当前实际生效的技能目录集合。
pub fn collect_effective_skill_instances_from_roots(
    roots: &[RuntimeSkillRoot],
) -> Result<Vec<ResolvedSkillInstance>, String> {
    let mut all_skill_ids = BTreeSet::new();
    let mut root_maps = Vec::new();
    for root in roots {
        let root_map = collect_named_skill_dirs(&root.skills_dir)?;
        all_skill_ids.extend(root_map.keys().cloned());
        root_maps.push((root.clone(), root_map));
    }

    let mut resolved = Vec::new();
    for skill_id in all_skill_ids {
        for (root, root_map) in &root_maps {
            let Some(skill_dir) = root_map.get(&skill_id) else {
                continue;
            };
            if is_effective_disable_override(skill_dir)? {
                break;
            }
            if !is_skill_manifest_enabled(skill_dir)? {
                break;
            }
            resolved.push(ResolvedSkillInstance {
                skill_id: skill_id.clone(),
                root_name: root.name.clone(),
                skills_root: root.skills_dir.clone(),
                actual_dir: skill_dir.clone(),
            });
            break;
        }
    }
    Ok(resolved)
}

/// Resolve one effective skill instance by skill id after applying root precedence.
/// 在应用根目录优先级后按技能标识符解析单个生效技能实例。
pub fn resolve_effective_skill_instance(
    base_dir: &Path,
    override_dir: Option<&Path>,
    skill_id: &str,
) -> Result<Option<ResolvedSkillInstance>, String> {
    validate_luaskills_identifier(skill_id, "skill_id")?;
    Ok(collect_effective_skill_instances(base_dir, override_dir)?
        .into_iter()
        .find(|instance| instance.skill_id == skill_id))
}

/// Resolve one effective skill instance by skill id from an ordered root chain.
/// 从有序根目录覆盖链中按技能标识符解析单个生效技能实例。
pub fn resolve_effective_skill_instance_from_roots(
    roots: &[RuntimeSkillRoot],
    skill_id: &str,
) -> Result<Option<ResolvedSkillInstance>, String> {
    validate_luaskills_identifier(skill_id, "skill_id")?;
    Ok(collect_effective_skill_instances_from_roots(roots)?
        .into_iter()
        .find(|instance| instance.skill_id == skill_id))
}

/// Resolve the highest-priority declared skill directory by skill id without applying enable-state filtering.
/// 在不应用启用状态过滤的前提下，按技能标识符解析最高优先级的已声明技能目录。
pub fn resolve_declared_skill_instance_from_roots(
    roots: &[RuntimeSkillRoot],
    skill_id: &str,
) -> Result<Option<ResolvedSkillInstance>, String> {
    validate_luaskills_identifier(skill_id, "skill_id")?;
    for root in roots {
        let root_map = collect_named_skill_dirs(&root.skills_dir)?;
        if let Some(actual_dir) = root_map.get(skill_id) {
            return Ok(Some(ResolvedSkillInstance {
                skill_id: skill_id.to_string(),
                root_name: root.name.clone(),
                skills_root: root.skills_dir.clone(),
                actual_dir: actual_dir.clone(),
            }));
        }
    }
    Ok(None)
}

/// Read one root directory into a validated skill-id -> path map.
/// 把单个根目录读取为经过校验的 skill-id -> 路径映射。
fn collect_named_skill_dirs(root: &Path) -> Result<std::collections::BTreeMap<String, PathBuf>, String> {
    let mut output = std::collections::BTreeMap::new();
    if !root.exists() {
        return Ok(output);
    }
    for entry in fs::read_dir(root)
        .map_err(|error| format!("Failed to read {}: {}", root.display(), error))?
    {
        let entry = entry.map_err(|error| format!("Failed to read skill entry: {}", error))?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("Failed to inspect skill entry type: {}", error))?;
        if !file_type.is_dir() {
            continue;
        }
        let skill_id = match entry.file_name().to_str() {
            Some(value) => value.to_string(),
            None => continue,
        };
        if validate_luaskills_identifier(&skill_id, "skill_id").is_err() {
            continue;
        }
        output.insert(skill_id, entry.path());
    }
    Ok(output)
}

/// Return whether one override skill directory should disable lower-priority instances because it is intentionally empty.
/// 返回单个 override 技能目录是否因为有意留空而应禁用更低优先级实例。
fn is_effective_disable_override(skill_dir: &Path) -> Result<bool, String> {
    Ok(fs::read_dir(skill_dir)
        .map_err(|error| format!("Failed to read override dir {}: {}", skill_dir.display(), error))?
        .next()
        .is_none())
}

/// Return whether one resolved skill directory is enabled by its manifest.
/// 返回单个已解析技能目录是否在其清单中启用。
fn is_skill_manifest_enabled(skill_dir: &Path) -> Result<bool, String> {
    let skill_yaml = skill_dir.join("skill.yaml");
    if !skill_yaml.exists() {
        return Ok(true);
    }
    let yaml_text = fs::read_to_string(&skill_yaml)
        .map_err(|error| format!("Failed to read {}: {}", skill_yaml.display(), error))?;
    let yaml_value: serde_yaml::Value = serde_yaml::from_str(&yaml_text)
        .map_err(|error| format!("Failed to parse {}: {}", skill_yaml.display(), error))?;
    if yaml_value
        .as_mapping()
        .is_some_and(|mapping| mapping.contains_key(serde_yaml::Value::String("skill_id".to_string())))
    {
        return Err(format!("skill manifest {} must not declare skill_id; directory name is the only skill_id", skill_yaml.display()));
    }
    #[derive(Debug, Deserialize)]
    struct SkillEnableProbe {
        /// When omitted the skill is treated as enabled.
        /// 省略时表示技能默认启用。
        #[serde(default = "default_skill_enable")]
        enable: bool,
    }
    /// Return the default enable flag used by lightweight manifest probes.
    /// 返回轻量清单探针使用的默认启用标记。
    fn default_skill_enable() -> bool {
        true
    }
    let probe: SkillEnableProbe = serde_yaml::from_value(yaml_value)
        .map_err(|error| format!("Failed to parse {}: {}", skill_yaml.display(), error))?;
    Ok(probe.enable)
}

#[cfg(test)]
mod tests {
    use super::{
        SkillInstallRequest, SkillInstallSourceType, SkillLifecycleAction, SkillManager,
        SkillManagerConfig, SkillOperationPlane, SkillProtectionConfig,
        collect_effective_skill_instances, resolve_effective_skill_instance,
    };
    use crate::runtime_options::RuntimeSkillRoot;

    /// Build one test skill-manager configuration rooted under the provided temporary directory.
    /// 基于给定临时目录构造单个测试用技能管理器配置。
    fn test_manager_config(temp_root: &std::path::Path, skill_root: RuntimeSkillRoot) -> SkillManagerConfig {
        SkillManagerConfig {
            skill_root,
            lifecycle_root: temp_root.join("state"),
            protection: SkillProtectionConfig::default(),
            download_cache_root: temp_root.join("downloads"),
            allow_network_download: false,
            github_base_url: None,
            github_api_base_url: None,
        }
    }

    /// Verify that disable/enable operations persist and clear state markers correctly.
    /// 验证停用/启用操作会正确持久化并清理状态标记。
    #[test]
    fn skill_manager_persists_disabled_state() {
        let temp_root = std::env::temp_dir().join(format!("vulcan_luaskills_skill_manager_test_{}", std::process::id()));
        if temp_root.exists() {
            let _ = std::fs::remove_dir_all(&temp_root);
        }
        let skill_root = temp_root.join("skills");
        let manager = SkillManager::new(SkillManagerConfig {
            protection: SkillProtectionConfig::default(),
            ..test_manager_config(
                &temp_root,
                RuntimeSkillRoot {
                    name: "ROOT".to_string(),
                    skills_dir: skill_root,
                },
            )
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

    /// Verify that protected skills are blocked in the skills plane but still allowed in the system plane.
    /// 验证受保护技能会在 skills 平面被阻止，但在 system 平面仍然允许。
    #[test]
    fn protected_skills_are_blocked_only_in_skills_plane() {
        let temp_root = std::env::temp_dir().join(format!("vulcan_luaskills_protection_test_{}", std::process::id()));
        let skill_root = temp_root.join("skills");
        let manager = SkillManager::new(SkillManagerConfig {
            protection: SkillProtectionConfig {
                protected_skill_ids: vec!["vulcan-runtime".to_string()],
            },
            ..test_manager_config(
                &temp_root,
                RuntimeSkillRoot {
                    name: "ROOT".to_string(),
                    skills_dir: skill_root,
                },
            )
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

    /// Verify that install/update entrypoints still enforce protection and return strict structured states before networking succeeds.
    /// 验证 install/update 入口在真正下载前依旧会执行保护判断，并返回严格的结构化状态。
    #[test]
    fn install_update_entrypoints_return_strict_structured_results() {
        let temp_root = std::env::temp_dir().join(format!("vulcan_luaskills_install_update_test_{}", std::process::id()));
        let skill_root = temp_root.join("skills");
        let skill_roots = vec![RuntimeSkillRoot {
            name: "ROOT".to_string(),
            skills_dir: skill_root.clone(),
        }];
        let _ = std::fs::create_dir_all(&skill_root);
        let manager = SkillManager::new(SkillManagerConfig {
            protection: SkillProtectionConfig {
                protected_skill_ids: vec!["vulcan-runtime".to_string()],
            },
            ..test_manager_config(&temp_root, skill_roots[0].clone())
        });

        assert!(manager
            .prepare_install_skill(
                SkillOperationPlane::Skills,
                &skill_roots,
                &SkillInstallRequest {
                    skill_id: Some("vulcan-runtime".to_string()),
                    source: None,
                    source_type: SkillInstallSourceType::Github,
                }
            )
            .is_err());

        let install_result = manager
            .prepare_install_skill(
                SkillOperationPlane::Skills,
                &skill_roots,
                &SkillInstallRequest {
                    skill_id: Some("vulcan-codekit".to_string()),
                    source: None,
                    source_type: SkillInstallSourceType::Github,
                },
            )
            .expect_err("install without source should fail strictly");
        assert!(install_result.contains("github install requires source repository"));

        let _ = std::fs::create_dir_all(skill_root.join("vulcan-codekit"));
        let update_result = manager
            .prepare_update_skill(
                SkillOperationPlane::Skills,
                &skill_roots,
                &SkillInstallRequest {
                    skill_id: Some("vulcan-codekit".to_string()),
                    source: None,
                    source_type: SkillInstallSourceType::Github,
                },
            )
            .expect_err("update without install record should fail strictly");
        assert!(update_result.contains("is not managed by the install workflow"));

        let _ = std::fs::remove_dir_all(&temp_root);
    }

    /// Verify that uninstall removes the skill directory but keeps database flags unset by default.
    /// 验证卸载会删除技能目录，同时默认不声明数据库已删除。
    #[test]
    fn uninstall_returns_safe_default_database_flags() {
        let temp_root = std::env::temp_dir().join(format!("vulcan_luaskills_uninstall_result_test_{}", std::process::id()));
        if temp_root.exists() {
            let _ = std::fs::remove_dir_all(&temp_root);
        }
        let skill_root = temp_root.join("skills");
        let manager = SkillManager::new(SkillManagerConfig {
            protection: SkillProtectionConfig::default(),
            ..test_manager_config(
                &temp_root,
                RuntimeSkillRoot {
                    name: "ROOT".to_string(),
                    skills_dir: skill_root.clone(),
                },
            )
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

    /// Verify that override roots can contribute standalone skills and shadow lower-priority roots.
    /// 验证 override 根目录既可以独立提供技能，也可以覆盖更低优先级根目录。
    #[test]
    fn collect_effective_skill_instances_supports_override_add_and_shadow() {
        let temp_root = std::env::temp_dir().join(format!("vulcan_luaskills_collect_effective_instances_test_{}", std::process::id()));
        if temp_root.exists() {
            let _ = std::fs::remove_dir_all(&temp_root);
        }
        let base_dir = temp_root.join("base");
        let override_dir = temp_root.join("override");
        let _ = std::fs::create_dir_all(base_dir.join("vulcan-codekit"));
        let _ = std::fs::create_dir_all(override_dir.join("vulcan-codekit"));
        let _ = std::fs::create_dir_all(override_dir.join("vulcan-runtime"));
        let _ = std::fs::write(
            base_dir.join("vulcan-codekit").join("skill.yaml"),
            "name: vulcan-codekit\nversion: 0.1.0\n",
        );
        let _ = std::fs::write(
            override_dir.join("vulcan-codekit").join("skill.yaml"),
            "name: vulcan-codekit\nversion: 0.2.0\n",
        );
        let _ = std::fs::write(
            override_dir.join("vulcan-runtime").join("skill.yaml"),
            "name: vulcan-runtime\nversion: 0.1.0\n",
        );

        let resolved = collect_effective_skill_instances(&base_dir, Some(&override_dir))
            .expect("effective skill collection should succeed");
        assert_eq!(resolved.len(), 2);
        let codekit = resolved
            .iter()
            .find(|value| value.skill_id == "vulcan-codekit")
            .expect("vulcan-codekit should exist");
        assert!(codekit.actual_dir.starts_with(&override_dir));
        let runtime = resolved
            .iter()
            .find(|value| value.skill_id == "vulcan-runtime")
            .expect("override-only vulcan-runtime should exist");
        assert!(runtime.actual_dir.starts_with(&override_dir));

        let _ = std::fs::remove_dir_all(&temp_root);
    }

    /// Verify that resolving one effective skill instance returns the highest-priority existing directory.
    /// 验证解析单个生效技能实例时会返回最高优先级的现有目录。
    #[test]
    fn resolve_effective_skill_instance_prefers_override_directory() {
        let temp_root = std::env::temp_dir().join(format!("vulcan_luaskills_resolve_effective_instance_test_{}", std::process::id()));
        if temp_root.exists() {
            let _ = std::fs::remove_dir_all(&temp_root);
        }
        let base_dir = temp_root.join("base");
        let override_dir = temp_root.join("override");
        let _ = std::fs::create_dir_all(base_dir.join("vulcan-codekit"));
        let _ = std::fs::create_dir_all(override_dir.join("vulcan-codekit"));
        let _ = std::fs::write(
            base_dir.join("vulcan-codekit").join("skill.yaml"),
            "name: vulcan-codekit\nversion: 0.1.0\n",
        );
        let _ = std::fs::write(
            override_dir.join("vulcan-codekit").join("skill.yaml"),
            "name: vulcan-codekit\nversion: 0.2.0\n",
        );

        let resolved = resolve_effective_skill_instance(&base_dir, Some(&override_dir), "vulcan-codekit")
            .expect("resolution should succeed")
            .expect("instance should exist");
        assert!(resolved.actual_dir.starts_with(&override_dir));

        let _ = std::fs::remove_dir_all(&temp_root);
    }
}
