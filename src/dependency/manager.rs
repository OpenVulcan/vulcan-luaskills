use std::fs;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use serde::{Deserialize, Serialize};

use crate::dependency::platform::current_platform_key;
use crate::dependency::types::{
    DependencyDetectionStatus, DependencyScope, DependencySourceType,
    ResolvedDependencyRequest, SkillDependencyKind,
};
use crate::download::archive::install_downloaded_payload;
use crate::download::manager::{DownloadManager, DownloadManagerConfig, DownloadRequest};
use crate::runtime_logging::{info as log_info, warn as log_warn};
use crate::skill::dependencies::{
    FfiDependencySpec, LuaDependencySpec, SkillDependencyManifest, SkillListIndexFile,
    ToolDependencySpec,
};
use crate::skill::manifest::validate_luaskills_identifier;
use std::collections::BTreeSet;

/// English: Dependency-manager configuration shared by dependency resolution and installation phases.
/// 供依赖解析与安装阶段共享使用的依赖管理配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyManagerConfig {
    /// English: Root directory used to install shared executable/tool dependencies.
    /// 用于安装共享可执行工具依赖的根目录。
    pub tool_root: PathBuf,
    /// English: Root directory used to probe host-provided executable/tool dependencies.
    /// 用于探测宿主提供可执行工具依赖的根目录。
    pub host_tool_root: PathBuf,
    /// English: Root directory used to install Lua package dependencies.
    /// 用于安装 Lua 包依赖的根目录。
    pub lua_root: PathBuf,
    /// English: Root directory used to probe host-provided Lua package dependencies.
    /// 用于探测宿主提供 Lua 包依赖的根目录。
    pub host_lua_root: PathBuf,
    /// English: Root directory used to install FFI/native library dependencies.
    /// 用于安装 FFI/原生库依赖的根目录。
    pub ffi_root: PathBuf,
    /// English: Root directory used to probe host-provided FFI/native dependencies.
    /// 用于探测宿主提供 FFI/原生依赖的根目录。
    pub host_ffi_root: PathBuf,
    /// English: Root directory used for cached downloads and fetched remote manifests.
    /// 用于缓存下载结果和远程清单的根目录。
    pub download_cache_root: PathBuf,
    /// English: Whether network downloads are allowed during dependency resolution.
    /// 依赖解析过程中是否允许网络下载。
    pub allow_network_download: bool,
    /// English: Optional GitHub browser base URL override.
    /// 可选的 GitHub 浏览器下载基址覆盖。
    pub github_base_url: Option<String>,
    /// English: Optional GitHub API base URL override.
    /// 可选的 GitHub API 基址覆盖。
    pub github_api_base_url: Option<String>,
}

/// English: High-level dependency manager owned by the LuaSkills runtime.
/// 由 LuaSkills 运行时拥有的高层依赖管理器。
pub struct DependencyManager {
    config: DependencyManagerConfig,
    downloader: DownloadManager,
}

impl DependencyManager {
    /// English: Create one dependency manager from a shared configuration object.
    /// 基于共享配置对象创建一个依赖管理器实例。
    pub fn new(config: DependencyManagerConfig) -> Self {
        let downloader = DownloadManager::new(DownloadManagerConfig {
            cache_root: config.download_cache_root.clone(),
            allow_network_download: config.allow_network_download,
            github_base_url: config.github_base_url.clone(),
            github_api_base_url: config.github_api_base_url.clone(),
        });
        Self { config, downloader }
    }

    /// English: Ensure all declared dependencies for one skill are installed and ready.
    /// 确保单个 skill 声明的全部依赖已安装且可用。
    pub fn ensure_skill_dependencies(
        &self,
        skill_id: &str,
        manifest: &SkillDependencyManifest,
    ) -> Result<(), String> {
        let platform_key = current_platform_key();
        if platform_key == "unknown" {
            return Err(
                "current platform is not supported by LuaSkills dependency manager / 当前平台不受 LuaSkills 依赖管理器支持"
                    .to_string(),
            );
        }

        for dependency in &manifest.tool_dependencies {
            self.ensure_tool_dependency(skill_id, dependency, platform_key)?;
        }
        for dependency in &manifest.lua_dependencies {
            self.ensure_lua_dependency(skill_id, dependency, platform_key)?;
        }
        for dependency in &manifest.ffi_dependencies {
            self.ensure_ffi_dependency(skill_id, dependency, platform_key)?;
        }
        Ok(())
    }

    /// English: Ensure one tool dependency is installed for the current platform.
    /// 确保单个工具依赖在当前平台上已安装完成。
    fn ensure_tool_dependency(
        &self,
        skill_id: &str,
        spec: &ToolDependencySpec,
        platform_key: &str,
    ) -> Result<(), String> {
        self.ensure_dependency(
            skill_id,
            SkillDependencyKind::Tool,
            spec.name.as_str(),
            spec.version.clone(),
            spec.scope,
            spec.required,
            spec.source.source_type,
            &spec.source,
            spec.package_for_platform(platform_key),
            platform_key,
            self.install_root_for_kind(SkillDependencyKind::Tool, spec.scope),
        )
    }

    /// English: Ensure one Lua dependency is installed for the current platform.
    /// 确保单个 Lua 依赖在当前平台上已安装完成。
    fn ensure_lua_dependency(
        &self,
        skill_id: &str,
        spec: &LuaDependencySpec,
        platform_key: &str,
    ) -> Result<(), String> {
        self.ensure_dependency(
            skill_id,
            SkillDependencyKind::Lua,
            spec.name.as_str(),
            spec.version.clone(),
            spec.scope,
            spec.required,
            spec.source.source_type,
            &spec.source,
            spec.package_for_platform(platform_key),
            platform_key,
            self.install_root_for_kind(SkillDependencyKind::Lua, spec.scope),
        )
    }

    /// English: Ensure one FFI dependency is installed for the current platform.
    /// 确保单个 FFI 依赖在当前平台上已安装完成。
    fn ensure_ffi_dependency(
        &self,
        skill_id: &str,
        spec: &FfiDependencySpec,
        platform_key: &str,
    ) -> Result<(), String> {
        self.ensure_dependency(
            skill_id,
            SkillDependencyKind::Ffi,
            spec.name.as_str(),
            spec.version.clone(),
            spec.scope,
            spec.required,
            spec.source.source_type,
            &spec.source,
            spec.package_for_platform(platform_key),
            platform_key,
            self.install_root_for_kind(SkillDependencyKind::Ffi, spec.scope),
        )
    }

    /// English: Resolve the concrete install/probe root for one dependency kind and scope pair.
    /// 根据依赖类型与作用域解析实际安装/探测根目录。
    fn install_root_for_kind(
        &self,
        kind: SkillDependencyKind,
        scope: DependencyScope,
    ) -> &Path {
        match (kind, scope) {
            (SkillDependencyKind::Tool, DependencyScope::Host) => &self.config.host_tool_root,
            (SkillDependencyKind::Tool, _) => &self.config.tool_root,
            (SkillDependencyKind::Lua, DependencyScope::Host) => &self.config.host_lua_root,
            (SkillDependencyKind::Lua, _) => &self.config.lua_root,
            (SkillDependencyKind::Ffi, DependencyScope::Host) => &self.config.host_ffi_root,
            (SkillDependencyKind::Ffi, _) => &self.config.ffi_root,
        }
    }

    /// English: Shared dependency ensure pipeline used by tool/lua/ffi dependency kinds.
    /// tool/lua/ffi 三类依赖共用的统一安装确保流程。
    #[allow(clippy::too_many_arguments)]
    fn ensure_dependency(
        &self,
        skill_id: &str,
        kind: SkillDependencyKind,
        dependency_name: &str,
        version: Option<String>,
        scope: DependencyScope,
        required: bool,
        source_type: DependencySourceType,
        source: &crate::skill::dependencies::DependencySourceSpec,
        package: Option<&crate::skill::dependencies::DependencyPackageSpec>,
        platform_key: &str,
        install_root: &Path,
    ) -> Result<(), String> {
        let resolved_request = self.resolve_dependency_request(
            skill_id,
            kind,
            dependency_name,
            version,
            scope,
            source_type,
            source,
            package,
            platform_key,
            install_root,
        )?;

        match self.detect_dependency(&resolved_request)? {
            DependencyDetectionStatus::Present => {
                log_info(format!(
                    "[LuaSkills:dependency] Skill '{}' reuses existing dependency '{}' on {}",
                    skill_id, dependency_name, platform_key
                ));
                Ok(())
            }
            DependencyDetectionStatus::Missing => {
                if scope == DependencyScope::Host {
                    if required {
                        return Err(format!(
                            "required host dependency '{}' is missing / 必需的宿主依赖 '{}' 缺失",
                            dependency_name, dependency_name
                        ));
                    }
                    log_warn(format!(
                        "[LuaSkills:dependency] Optional host dependency '{}' is missing",
                        dependency_name
                    ));
                    return Ok(());
                }
                if !self.config.allow_network_download {
                    if required {
                        return Err(format!(
                            "required dependency '{}' is missing and network download is disabled / 必需依赖 '{}' 缺失且当前禁止网络下载",
                            dependency_name, dependency_name
                        ));
                    }
                    log_warn(format!(
                        "[LuaSkills:dependency] Optional dependency '{}' is missing and download is disabled",
                        dependency_name
                    ));
                    return Ok(());
                }

                let cache_key = format!(
                    "{}-{}-{}",
                    match kind {
                        SkillDependencyKind::Tool => "tool",
                        SkillDependencyKind::Lua => "lua",
                        SkillDependencyKind::Ffi => "ffi",
                    },
                    dependency_name,
                    platform_key
                );
                let download_path = self.downloader.download(&DownloadRequest {
                    source_type,
                    source_locator: resolved_request.download_url.clone(),
                    cache_key,
                })?;
                install_downloaded_payload(
                    &download_path,
                    resolved_request.archive_type,
                    &resolved_request.install_root,
                    &resolved_request.exports,
                )?;
                if matches!(
                    self.detect_dependency(&resolved_request)?,
                    DependencyDetectionStatus::Missing
                ) {
                    return Err(format!(
                        "dependency '{}' was downloaded but exported files are still missing / 依赖 '{}' 已下载，但导出文件仍然缺失",
                        dependency_name, dependency_name
                    ));
                }

                log_info(format!(
                    "[LuaSkills:dependency] Installed dependency '{}' for skill '{}' on {}",
                    dependency_name, skill_id, platform_key
                ));
                Ok(())
            }
        }
    }

    /// English: Resolve one dependency declaration into a concrete install request.
    /// 把单个依赖声明解析成具体可执行的安装请求。
    #[allow(clippy::too_many_arguments)]
    fn resolve_dependency_request(
        &self,
        skill_id: &str,
        kind: SkillDependencyKind,
        dependency_name: &str,
        version: Option<String>,
        scope: DependencyScope,
        source_type: DependencySourceType,
        source: &crate::skill::dependencies::DependencySourceSpec,
        package: Option<&crate::skill::dependencies::DependencyPackageSpec>,
        platform_key: &str,
        install_root: &Path,
    ) -> Result<ResolvedDependencyRequest, String> {
        let resolved_package = match source_type {
            DependencySourceType::GithubRelease | DependencySourceType::Url => package
                .cloned()
                .ok_or_else(|| {
                    format!(
                        "dependency '{}' does not declare package metadata for platform '{}' / 依赖 '{}' 没有为平台 '{}' 声明包元数据",
                        dependency_name, platform_key, dependency_name, platform_key
                    )
                })?,
            DependencySourceType::SkillList => {
                let skilllist = source.skilllist.as_ref().ok_or_else(|| {
                    format!(
                        "dependency '{}' declares skilllist source but misses skilllist config / 依赖 '{}' 声明了 skilllist 来源但缺少配置",
                        dependency_name, dependency_name
                    )
                })?;
                let index_file = self.fetch_skilllist_index(&skilllist.url)?;
                let package_manifest = index_file
                    .packages
                    .get(&skilllist.package)
                    .ok_or_else(|| {
                        format!(
                            "dependency '{}' cannot find package '{}' in skilllist / 依赖 '{}' 无法在 skilllist 中找到包 '{}'",
                            dependency_name, skilllist.package, dependency_name, skilllist.package
                        )
                    })?;
                package_manifest
                    .packages
                    .get(platform_key)
                    .cloned()
                    .ok_or_else(|| {
                        format!(
                            "dependency '{}' skilllist package '{}' does not support platform '{}' / 依赖 '{}' 的 skilllist 包 '{}' 不支持平台 '{}'",
                            dependency_name,
                            skilllist.package,
                            platform_key,
                            dependency_name,
                            skilllist.package,
                            platform_key
                        )
                    })?
            }
        };

        let download_url = match source_type {
            DependencySourceType::GithubRelease => {
                let github_source = source.github.as_ref().ok_or_else(|| {
                    format!(
                        "dependency '{}' declares github_release source but misses github config / 依赖 '{}' 声明了 github_release 来源但缺少 github 配置",
                        dependency_name, dependency_name
                    )
                })?;
                let asset_name = resolved_package.asset_name.as_ref().ok_or_else(|| {
                    format!(
                        "dependency '{}' github package for platform '{}' must declare asset_name / 依赖 '{}' 的 GitHub 平台包 '{}' 必须声明 asset_name",
                        dependency_name, platform_key, dependency_name, platform_key
                    )
                })?;
                self.downloader.resolve_github_release_asset_url(
                    github_source,
                    asset_name,
                    version.as_deref(),
                )?
            }
            DependencySourceType::Url | DependencySourceType::SkillList => resolved_package
                .url
                .clone()
                .ok_or_else(|| {
                    format!(
                        "dependency '{}' platform '{}' must declare url / 依赖 '{}' 的平台 '{}' 必须声明 url",
                        dependency_name, platform_key, dependency_name, platform_key
                    )
                })?,
        };

        Ok(ResolvedDependencyRequest {
            kind,
            name: dependency_name.to_string(),
            scope,
            platform_key: platform_key.to_string(),
            download_url,
            version: version.clone(),
            install_root: build_dependency_install_root(
                install_root,
                scope,
                skill_id,
                dependency_name,
                version.as_deref(),
                platform_key,
            ),
            archive_type: resolved_package.archive_type,
            exports: resolved_package.exports,
        })
    }

    /// English: Remove dependencies owned by one uninstalled skill and clean up unused shared dependencies using real-time manifest scans.
    /// 清理一个已卸载技能拥有的依赖，并通过实时扫描清除不再使用的共享依赖。
    pub fn cleanup_uninstalled_skill_dependencies(
        &self,
        base_dir: &Path,
        override_dir: Option<&Path>,
        removed_skill_id: &str,
        removed_manifest: Option<&SkillDependencyManifest>,
    ) -> Result<(), String> {
        self.remove_skill_private_dependency_roots(removed_skill_id)?;
        let Some(removed_manifest) = removed_manifest else {
            return Ok(());
        };
        let remaining_shared_roots = self.collect_live_shared_dependency_roots(base_dir, override_dir)?;
        for root in self.shared_dependency_roots_for_manifest(removed_skill_id, removed_manifest)? {
            if remaining_shared_roots.contains(&root) {
                continue;
            }
            if root.exists() {
                fs::remove_dir_all(&root)
                    .map_err(|error| format!("Failed to remove {}: {}", root.display(), error))?;
            }
        }
        Ok(())
    }

    /// English: Return all shared dependency install roots declared by one manifest for the current platform.
    /// 返回当前平台下由单个依赖清单声明的全部共享依赖安装根目录。
    pub fn shared_dependency_roots_for_manifest(
        &self,
        skill_id: &str,
        manifest: &SkillDependencyManifest,
    ) -> Result<Vec<PathBuf>, String> {
        let platform_key = current_platform_key();
        if platform_key == "unknown" {
            return Ok(Vec::new());
        }
        let mut roots = BTreeSet::new();
        for dependency in &manifest.tool_dependencies {
            if dependency.scope != DependencyScope::Shared {
                continue;
            }
            if dependency.package_for_platform(platform_key).is_none() {
                continue;
            }
            roots.insert(build_dependency_install_root(
                &self.config.tool_root,
                dependency.scope,
                skill_id,
                dependency.name.as_str(),
                dependency.version.as_deref(),
                platform_key,
            ));
        }
        for dependency in &manifest.lua_dependencies {
            if dependency.scope != DependencyScope::Shared {
                continue;
            }
            if dependency.package_for_platform(platform_key).is_none() {
                continue;
            }
            roots.insert(build_dependency_install_root(
                &self.config.lua_root,
                dependency.scope,
                skill_id,
                dependency.name.as_str(),
                dependency.version.as_deref(),
                platform_key,
            ));
        }
        for dependency in &manifest.ffi_dependencies {
            if dependency.scope != DependencyScope::Shared {
                continue;
            }
            if dependency.package_for_platform(platform_key).is_none() {
                continue;
            }
            roots.insert(build_dependency_install_root(
                &self.config.ffi_root,
                dependency.scope,
                skill_id,
                dependency.name.as_str(),
                dependency.version.as_deref(),
                platform_key,
            ));
        }
        Ok(roots.into_iter().collect())
    }

    /// English: Scan the current effective skill set and collect all shared dependency install roots.
    /// 扫描当前生效技能集合，并收集全部共享依赖安装根目录。
    pub fn collect_live_shared_dependency_roots(
        &self,
        base_dir: &Path,
        override_dir: Option<&Path>,
    ) -> Result<BTreeSet<PathBuf>, String> {
        let platform_key = current_platform_key();
        if platform_key == "unknown" {
            return Ok(BTreeSet::new());
        }

        let mut roots = BTreeSet::new();
        for skill_dir in collect_effective_skill_dirs(base_dir, override_dir)? {
            let skill_id = skill_dir
                .file_name()
                .and_then(|value| value.to_str())
                .ok_or_else(|| format!("Invalid skill directory name: {}", skill_dir.display()))?;
            let dependencies_path = skill_dir.join("dependencies.yaml");
            if !dependencies_path.exists() {
                continue;
            }
            let manifest = SkillDependencyManifest::load_from_path(&dependencies_path)?;
            for root in self.shared_dependency_roots_for_manifest(skill_id, &manifest)? {
                roots.insert(root);
            }
        }
        Ok(roots)
    }

    /// English: Remove orphan shared dependency directories by rescanning current live skills instead of trusting persisted state files.
    /// 通过重新扫描当前生效技能删除孤立共享依赖目录，而不是依赖持久化状态文件。
    pub fn cleanup_orphaned_shared_dependencies(
        &self,
        base_dir: &Path,
        override_dir: Option<&Path>,
    ) -> Result<(), String> {
        let live_shared_roots = self.collect_live_shared_dependency_roots(base_dir, override_dir)?;
        for root in self.enumerate_shared_dependency_install_roots()? {
            if live_shared_roots.contains(&root) {
                continue;
            }
            if root.exists() {
                fs::remove_dir_all(&root)
                    .map_err(|error| format!("Failed to remove {}: {}", root.display(), error))?;
            }
        }
        Ok(())
    }

    /// English: Remove all skill-private dependency roots of one skill identifier.
    /// 删除单个技能标识符对应的全部技能私有依赖根目录。
    fn remove_skill_private_dependency_roots(&self, skill_id: &str) -> Result<(), String> {
        for root in [
            self.config.tool_root.join(skill_id),
            self.config.lua_root.join(skill_id),
            self.config.ffi_root.join(skill_id),
        ] {
            if root.exists() {
                fs::remove_dir_all(&root)
                    .map_err(|error| format!("Failed to remove {}: {}", root.display(), error))?;
            }
        }
        Ok(())
    }

    /// English: Enumerate all currently installed shared dependency leaf directories across tool/lua/ffi roots.
    /// 枚举当前在 tool/lua/ffi 根目录下已安装的全部共享依赖叶子目录。
    fn enumerate_shared_dependency_install_roots(&self) -> Result<BTreeSet<PathBuf>, String> {
        let mut roots = BTreeSet::new();
        for shared_root in [&self.config.tool_root, &self.config.lua_root, &self.config.ffi_root] {
            for dependency_dir in read_child_dirs(shared_root)? {
                for version_dir in read_child_dirs(&dependency_dir)? {
                    for platform_dir in read_child_dirs(&version_dir)? {
                        roots.insert(platform_dir);
                    }
                }
            }
        }
        Ok(roots)
    }

    /// English: Detect whether one dependency is already installed by checking all declared exports.
    /// 通过检查全部声明的导出文件来判断单个依赖是否已经安装。
    fn detect_dependency(
        &self,
        request: &ResolvedDependencyRequest,
    ) -> Result<DependencyDetectionStatus, String> {
        if request.exports.is_empty() {
            return Err(format!(
                "dependency '{}' must declare at least one export / 依赖 '{}' 必须至少声明一个导出文件",
                request.name, request.name
            ));
        }

        let all_present = request.exports.iter().all(|export| {
            request
                .install_root
                .join(export.target_path.replace('/', std::path::MAIN_SEPARATOR_STR))
                .exists()
        });
        Ok(if all_present {
            DependencyDetectionStatus::Present
        } else {
            DependencyDetectionStatus::Missing
        })
    }

    /// English: Fetch and parse one remote skilllist index file.
    /// 获取并解析单个远程 skilllist 索引文件。
    fn fetch_skilllist_index(&self, url: &str) -> Result<SkillListIndexFile, String> {
        let cache_key = format!(
            "skilllist-{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(url)
        );
        let cached_text = self.downloader.fetch_text(url, &cache_key)?;
        serde_yaml::from_str::<SkillListIndexFile>(&cached_text).map_err(|error| {
            format!(
                "failed to parse skilllist index {}: {} / 解析 skilllist 索引 {} 失败: {}",
                url, error, url, error
            )
        })
    }
}

/// English: Build one fallback shared tool root under the skill directory parent.
/// 在 skill 目录上级下构造一个兜底共享工具根目录。
pub fn fallback_tool_root(skill_base_dir: &Path) -> PathBuf {
    skill_base_dir.join("__tools")
}

/// English: Build one fallback download cache root under the skill directory parent.
/// 在 skill 目录上级下构造一个兜底下载缓存根目录。
pub fn fallback_download_cache_root(skill_base_dir: &Path) -> PathBuf {
    skill_base_dir.join("__download_cache")
}

/// English: Ensure one root directory exists before it is used by the dependency manager.
/// 在依赖管理器使用某个根目录前确保其已经存在。
pub fn ensure_directory(root: &Path) -> Result<(), String> {
    fs::create_dir_all(root)
        .map_err(|error| format!("Failed to create {}: {}", root.display(), error))
}

/// English: Build the final install root of one dependency according to its scope, name, version, and platform.
/// 根据依赖的作用域、名称、版本和平台构造最终安装根目录。
fn build_dependency_install_root(
    root: &Path,
    scope: DependencyScope,
    skill_id: &str,
    dependency_name: &str,
    version: Option<&str>,
    platform_key: &str,
) -> PathBuf {
    let normalized_version = normalize_dependency_path_component(version.unwrap_or("unversioned"));
    let normalized_name = normalize_dependency_path_component(dependency_name);
    let normalized_platform = normalize_dependency_path_component(platform_key);
    match scope {
        DependencyScope::Shared | DependencyScope::Host => root
            .join(normalized_name)
            .join(normalized_version)
            .join(normalized_platform),
        DependencyScope::Skill => root
            .join(skill_id)
            .join(normalized_name)
            .join(normalized_version)
            .join(normalized_platform),
    }
}

/// English: Normalize one dependency path component for stable cross-platform directory generation.
/// 归一化单个依赖路径片段，以生成稳定的跨平台目录结构。
fn normalize_dependency_path_component(raw: &str) -> String {
    let mut output = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
            output.push(ch);
        } else {
            output.push('_');
        }
    }
    if output.is_empty() {
        "unnamed".to_string()
    } else {
        output
    }
}

/// English: Minimal manifest subset used to determine whether one scanned skill is enabled.
/// 用于判断单个扫描到的技能是否启用的最小清单子集。
#[derive(Debug, Deserialize)]
struct SkillEnableProbe {
    /// English: When omitted the skill is treated as enabled.
    /// 省略时表示技能默认启用。
    #[serde(default = "default_skill_enable")]
    enable: bool,
}

/// English: Return the default enable flag used by dependency scanning.
/// 返回依赖扫描时使用的默认启用标记。
fn default_skill_enable() -> bool {
    true
}

/// English: Collect the current effective skill directories after applying base/override precedence rules.
/// 在应用 base/override 优先级规则后收集当前生效的 skill 目录。
fn collect_effective_skill_dirs(
    base_dir: &Path,
    override_dir: Option<&Path>,
) -> Result<Vec<PathBuf>, String> {
    let mut effective_dirs = Vec::new();
    let base_iter = fs::read_dir(base_dir)
        .map_err(|error| format!("Failed to read {}: {}", base_dir.display(), error))?;
    for entry in base_iter {
        let entry = entry.map_err(|error| format!("Failed to read skill entry: {}", error))?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("Failed to inspect skill entry type: {}", error))?;
        if !file_type.is_dir() {
            continue;
        }
        let skill_name = match entry.file_name().to_str() {
            Some(value) => value.to_string(),
            None => continue,
        };
        if validate_luaskills_identifier(&skill_name, "skill_id").is_err() {
            continue;
        }

        let actual_dir = if let Some(override_dir) = override_dir {
            let override_skill_dir = override_dir.join(&skill_name);
            if override_skill_dir.exists() {
                if override_skill_dir
                    .read_dir()
                    .map_err(|error| {
                        format!(
                            "Failed to read override dir {}: {}",
                            override_skill_dir.display(),
                            error
                        )
                    })?
                    .next()
                    .is_none()
                {
                    continue;
                }
                override_skill_dir
            } else {
                entry.path()
            }
        } else {
            entry.path()
        };

        if !is_skill_manifest_enabled(&actual_dir)? {
            continue;
        }
        effective_dirs.push(actual_dir);
    }
    Ok(effective_dirs)
}

/// English: Read immediate child directories of one root path and skip files or missing roots.
/// 读取单个根目录的直接子目录，并跳过文件或不存在的根目录。
fn read_child_dirs(root: &Path) -> Result<Vec<PathBuf>, String> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut output = Vec::new();
    for entry in fs::read_dir(root)
        .map_err(|error| format!("Failed to read {}: {}", root.display(), error))?
    {
        let entry = entry.map_err(|error| format!("Failed to read child entry: {}", error))?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("Failed to inspect child entry type: {}", error))?;
        if file_type.is_dir() {
            output.push(entry.path());
        }
    }
    Ok(output)
}

/// English: Return whether one scanned skill directory is enabled by its manifest.
/// 返回单个扫描到的技能目录是否在清单中被启用。
fn is_skill_manifest_enabled(skill_dir: &Path) -> Result<bool, String> {
    let skill_yaml = skill_dir.join("skill.yaml");
    if !skill_yaml.exists() {
        return Ok(true);
    }
    let yaml_text = fs::read_to_string(&skill_yaml)
        .map_err(|error| format!("Failed to read {}: {}", skill_yaml.display(), error))?;
    let probe: SkillEnableProbe = serde_yaml::from_str(&yaml_text)
        .map_err(|error| format!("Failed to parse {}: {}", skill_yaml.display(), error))?;
    Ok(probe.enable)
}
