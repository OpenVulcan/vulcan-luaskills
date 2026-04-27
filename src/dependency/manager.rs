use base64::Engine as _;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::dependency::platform::current_platform_key;
use crate::dependency::types::{
    DependencyDetectionStatus, DependencyScope, DependencySourceType, ResolvedDependencyRequest,
    SkillDependencyKind,
};
use crate::download::archive::install_downloaded_payload;
use crate::download::manager::{DownloadManager, DownloadManagerConfig, DownloadRequest};
use crate::runtime_logging::{info as log_info, warn as log_warn};
use crate::runtime_options::RuntimeSkillRoot;
use crate::skill::dependencies::{
    DependencyExportSpec, FfiDependencySpec, LuaDependencySpec, SkillDependencyManifest,
    SkillListIndexFile, ToolDependencySpec,
};

/// Dependency-manager configuration shared by dependency resolution and installation phases.
/// 供依赖解析与安装阶段共享使用的依赖管理配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyManagerConfig {
    /// Root directory used to install skill-local executable/tool dependencies.
    /// 用于安装技能本地可执行工具依赖的根目录。
    pub tool_root: PathBuf,
    /// Root directory used to probe host-provided executable/tool dependencies.
    /// 用于探测宿主提供可执行工具依赖的根目录。
    pub host_tool_root: PathBuf,
    /// Root directory used to install Lua package dependencies.
    /// 用于安装 Lua 包依赖的根目录。
    pub lua_root: PathBuf,
    /// Root directory used to probe host-provided Lua package dependencies.
    /// 用于探测宿主提供 Lua 包依赖的根目录。
    pub host_lua_root: PathBuf,
    /// Root directory used to install FFI/native library dependencies.
    /// 用于安装 FFI/原生库依赖的根目录。
    pub ffi_root: PathBuf,
    /// Root directory used to probe host-provided FFI/native dependencies.
    /// 用于探测宿主提供 FFI/原生依赖的根目录。
    pub host_ffi_root: PathBuf,
    /// Root directory used for cached downloads and fetched remote manifests.
    /// 用于缓存下载结果和远程清单的根目录。
    pub download_cache_root: PathBuf,
    /// Whether network downloads are allowed during dependency resolution.
    /// 依赖解析过程中是否允许网络下载。
    pub allow_network_download: bool,
    /// Optional GitHub browser base URL override.
    /// 可选的 GitHub 浏览器下载基址覆盖。
    pub github_base_url: Option<String>,
    /// Optional GitHub API base URL override.
    /// 可选的 GitHub API 基址覆盖。
    pub github_api_base_url: Option<String>,
}

/// High-level dependency manager owned by the LuaSkills runtime.
/// 由 LuaSkills 运行时拥有的高层依赖管理器。
pub struct DependencyManager {
    config: DependencyManagerConfig,
    downloader: DownloadManager,
}

impl DependencyManager {
    /// Create one dependency manager from a shared configuration object.
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

    /// Ensure all declared dependencies for one skill are installed and ready.
    /// 确保单个 skill 声明的全部依赖已安装且可用。
    pub fn ensure_skill_dependencies(
        &self,
        skill_id: &str,
        manifest: &SkillDependencyManifest,
    ) -> Result<(), String> {
        let platform_key = current_platform_key();
        if platform_key == "unknown" {
            return Err(
                "current platform is not supported by LuaSkills dependency manager".to_string(),
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

    /// Ensure one tool dependency is installed for the current platform.
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

    /// Ensure one Lua dependency is installed for the current platform.
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

    /// Ensure one FFI dependency is installed for the current platform.
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

    /// Resolve the concrete install/probe root for one dependency kind and scope pair.
    /// 根据依赖类型与作用域解析实际安装/探测根目录。
    fn install_root_for_kind(&self, kind: SkillDependencyKind, scope: DependencyScope) -> &Path {
        match (kind, scope) {
            (SkillDependencyKind::Tool, DependencyScope::Host) => &self.config.host_tool_root,
            (SkillDependencyKind::Tool, _) => &self.config.tool_root,
            (SkillDependencyKind::Lua, DependencyScope::Host) => &self.config.host_lua_root,
            (SkillDependencyKind::Lua, _) => &self.config.lua_root,
            (SkillDependencyKind::Ffi, DependencyScope::Host) => &self.config.host_ffi_root,
            (SkillDependencyKind::Ffi, _) => &self.config.ffi_root,
        }
    }

    /// Shared dependency ensure pipeline used by tool/lua/ffi dependency kinds.
    /// tool
    /// lua
    /// ffi 三类依赖共用的统一安装确保流程。
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
        if self
            .find_existing_local_dependency_request(
                skill_id,
                kind,
                dependency_name,
                version.as_deref(),
                scope,
                package,
                platform_key,
                install_root,
            )?
            .is_some()
        {
            log_info(format!(
                "[LuaSkills:dependency] Skill '{}' reuses existing dependency '{}' on {}",
                skill_id, dependency_name, platform_key
            ));
            return Ok(());
        }

        if scope == DependencyScope::Host {
            if required {
                return Err(format!(
                    "required host dependency '{}' is missing",
                    dependency_name
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
                    "required dependency '{}' is missing and network download is disabled",
                    dependency_name
                ));
            }
            log_warn(format!(
                "[LuaSkills:dependency] Optional dependency '{}' is missing and download is disabled",
                dependency_name
            ));
            return Ok(());
        }

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
                    cache_key: cache_key.clone(),
                })?;
                if let Err(first_error) = install_downloaded_payload(
                    &download_path,
                    resolved_request.archive_type,
                    &resolved_request.install_root,
                    &resolved_request.exports,
                ) {
                    log_warn(format!(
                        "[LuaSkills:dependency] Dependency '{}' install from cached archive failed once, retrying after cache cleanup: {}",
                        dependency_name, first_error
                    ));
                    let _ = fs::remove_file(&download_path);
                    let _ = fs::remove_dir_all(&resolved_request.install_root);
                    let redownloaded_path = self.downloader.download(&DownloadRequest {
                        source_type,
                        source_locator: resolved_request.download_url.clone(),
                        cache_key,
                    })?;
                    install_downloaded_payload(
                        &redownloaded_path,
                        resolved_request.archive_type,
                        &resolved_request.install_root,
                        &resolved_request.exports,
                    )
                    .map_err(|retry_error| {
                        format!(
                            "{}. Automatic redownload and reinstall also failed: {}",
                            first_error, retry_error
                        )
                    })?;
                }
                if matches!(
                    self.detect_dependency(&resolved_request)?,
                    DependencyDetectionStatus::Missing
                ) {
                    return Err(format!(
                        "dependency '{}' was downloaded but exported files are still missing",
                        dependency_name
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

    /// Find one already-installed dependency request without resolving any remote source.
    /// 在不解析任何远程来源的前提下查找一个已安装依赖请求。
    #[allow(clippy::too_many_arguments)]
    fn find_existing_local_dependency_request(
        &self,
        skill_id: &str,
        kind: SkillDependencyKind,
        dependency_name: &str,
        version: Option<&str>,
        scope: DependencyScope,
        package: Option<&crate::skill::dependencies::DependencyPackageSpec>,
        platform_key: &str,
        install_root: &Path,
    ) -> Result<Option<ResolvedDependencyRequest>, String> {
        let Some(package) = package else {
            return Ok(None);
        };
        for request in self.local_dependency_probe_requests(
            skill_id,
            kind,
            dependency_name,
            version,
            scope,
            package,
            platform_key,
            install_root,
        ) {
            if matches!(
                self.detect_dependency(&request)?,
                DependencyDetectionStatus::Present
            ) {
                return Ok(Some(request));
            }
        }
        Ok(None)
    }

    /// Build local-only dependency probe requests before network-backed resolution is needed.
    /// 在需要网络解析前构造仅用于本地探测的依赖请求。
    #[allow(clippy::too_many_arguments)]
    fn local_dependency_probe_requests(
        &self,
        skill_id: &str,
        kind: SkillDependencyKind,
        dependency_name: &str,
        version: Option<&str>,
        scope: DependencyScope,
        package: &crate::skill::dependencies::DependencyPackageSpec,
        platform_key: &str,
        install_root: &Path,
    ) -> Vec<ResolvedDependencyRequest> {
        let mut requests = Vec::new();
        for version_candidate in local_dependency_version_candidates(version) {
            requests.extend(local_dependency_probe_request_variants(
                skill_id,
                kind,
                dependency_name,
                version_candidate.as_deref(),
                scope,
                package,
                platform_key,
                install_root,
            ));
        }

        if version.is_none() {
            requests.extend(self.local_unversioned_dependency_probe_requests(
                skill_id,
                kind,
                dependency_name,
                scope,
                package,
                platform_key,
                install_root,
            ));
        }

        requests
    }

    /// Build probe requests for existing version directories when the manifest omits a version.
    /// 当清单省略版本时，根据已有版本目录构造探测请求。
    #[allow(clippy::too_many_arguments)]
    fn local_unversioned_dependency_probe_requests(
        &self,
        skill_id: &str,
        kind: SkillDependencyKind,
        dependency_name: &str,
        scope: DependencyScope,
        package: &crate::skill::dependencies::DependencyPackageSpec,
        platform_key: &str,
        install_root: &Path,
    ) -> Vec<ResolvedDependencyRequest> {
        let normalized_name = normalize_dependency_path_component(dependency_name);
        let dependency_root = match scope {
            DependencyScope::Host => install_root.join(normalized_name),
            DependencyScope::Skill => install_root.join(skill_id).join(normalized_name),
        };
        let Ok(version_entries) = fs::read_dir(&dependency_root) else {
            return Vec::new();
        };

        let normalized_platform = normalize_dependency_path_component(platform_key);
        version_entries
            .filter_map(Result::ok)
            .flat_map(|entry| {
                let file_type = entry.file_type().ok()?;
                if !file_type.is_dir() {
                    return None;
                }
                let version_component = entry.file_name().to_string_lossy().to_string();
                let candidate_root = entry.path().join(&normalized_platform);
                if !candidate_root.is_dir() {
                    return None;
                }
                Some(local_dependency_probe_request_variants_for_root(
                    kind,
                    dependency_name,
                    Some(version_component.as_str()),
                    scope,
                    platform_key,
                    candidate_root,
                    package,
                ))
            })
            .flatten()
            .collect()
    }

    /// Resolve one dependency declaration into a concrete install request.
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
            DependencySourceType::GithubRelease | DependencySourceType::Url => {
                package.cloned().ok_or_else(|| {
                    format!(
                        "dependency '{}' does not declare package metadata for platform '{}'",
                        dependency_name, platform_key
                    )
                })?
            }
            DependencySourceType::SkillList => {
                let skilllist = source.skilllist.as_ref().ok_or_else(|| {
                    format!(
                        "dependency '{}' declares skilllist source but misses skilllist config",
                        dependency_name
                    )
                })?;
                let index_file = self.fetch_skilllist_index(&skilllist.url)?;
                let package_manifest =
                    index_file.packages.get(&skilllist.package).ok_or_else(|| {
                        format!(
                            "dependency '{}' cannot find package '{}' in skilllist",
                            dependency_name, skilllist.package
                        )
                    })?;
                package_manifest
                    .packages
                    .get(platform_key)
                    .cloned()
                    .ok_or_else(|| {
                        format!(
                            "dependency '{}' skilllist package '{}' does not support platform '{}'",
                            dependency_name, skilllist.package, platform_key
                        )
                    })?
            }
        };

        let mut resolved_version = version.clone();
        let mut resolved_tag_name: Option<String> = None;
        let download_url = match source_type {
            DependencySourceType::GithubRelease => {
                let github_source = source.github.as_ref().ok_or_else(|| {
                    format!(
                        "dependency '{}' declares github_release source but misses github config",
                        dependency_name
                    )
                })?;
                let asset_name = resolved_package.asset_name.as_ref().ok_or_else(|| {
                    format!(
                        "dependency '{}' github package for platform '{}' must declare asset_name",
                        dependency_name, platform_key
                    )
                })?;
                let resolved_asset = self.downloader.resolve_github_release_asset(
                    github_source,
                    asset_name,
                    version.as_deref(),
                )?;
                resolved_version = Some(resolved_asset.version.clone());
                resolved_tag_name = Some(resolved_asset.tag_name.clone());
                resolved_asset.download_url
            }
            DependencySourceType::Url | DependencySourceType::SkillList => {
                resolved_package.url.clone().ok_or_else(|| {
                    format!(
                        "dependency '{}' platform '{}' must declare url",
                        dependency_name, platform_key
                    )
                })?
            }
        };

        Ok(ResolvedDependencyRequest {
            kind,
            name: dependency_name.to_string(),
            scope,
            platform_key: platform_key.to_string(),
            download_url,
            version: resolved_version.clone(),
            install_root: build_dependency_install_root(
                install_root,
                scope,
                skill_id,
                dependency_name,
                resolved_version.as_deref(),
                platform_key,
            ),
            archive_type: resolved_package.archive_type,
            exports: resolve_export_templates(
                &resolved_package.exports,
                resolved_version.as_deref(),
                resolved_tag_name.as_deref(),
            ),
        })
    }

    /// Remove dependencies owned by one uninstalled skill.
    /// 清理一个已卸载技能拥有的依赖。
    pub fn cleanup_uninstalled_skill_dependencies(
        &self,
        base_dir: &Path,
        override_dir: Option<&Path>,
        removed_skill_id: &str,
        removed_manifest: Option<&SkillDependencyManifest>,
    ) -> Result<(), String> {
        let mut roots = vec![RuntimeSkillRoot {
            name: "ROOT".to_string(),
            skills_dir: base_dir.to_path_buf(),
        }];
        if let Some(override_dir) = override_dir {
            roots.push(RuntimeSkillRoot {
                name: "PROJECT".to_string(),
                skills_dir: override_dir.to_path_buf(),
            });
        }
        self.cleanup_uninstalled_skill_dependencies_from_roots(
            &roots,
            removed_skill_id,
            removed_manifest,
        )
    }

    /// Remove all private dependency roots for one uninstalled skill using an ordered root chain.
    /// 使用有序根目录链为单个已卸载技能清理全部私有依赖根。
    pub fn cleanup_uninstalled_skill_dependencies_from_roots(
        &self,
        skill_roots: &[RuntimeSkillRoot],
        removed_skill_id: &str,
        removed_manifest: Option<&SkillDependencyManifest>,
    ) -> Result<(), String> {
        self.remove_skill_private_dependency_roots(removed_skill_id)?;
        let _ = (skill_roots, removed_manifest);
        Ok(())
    }

    /// Remove stale skill-private dependency roots after one successful skill update.
    /// 在单个技能更新成功后移除已经过期的技能私有依赖根目录。
    pub fn cleanup_updated_skill_dependencies(
        &self,
        skill_id: &str,
        previous_manifest: Option<&SkillDependencyManifest>,
        current_manifest: Option<&SkillDependencyManifest>,
    ) -> Result<(), String> {
        let platform_key = current_platform_key();
        if platform_key == "unknown" {
            return Ok(());
        }

        let previous_roots =
            self.collect_skill_local_dependency_roots(skill_id, previous_manifest, platform_key)?;
        let current_roots =
            self.collect_skill_local_dependency_roots(skill_id, current_manifest, platform_key)?;

        for stale_root in previous_roots.difference(&current_roots) {
            if stale_root.exists() {
                fs::remove_dir_all(stale_root).map_err(|error| {
                    format!(
                        "Failed to remove stale dependency root {}: {}",
                        stale_root.display(),
                        error
                    )
                })?;
            }
        }
        Ok(())
    }

    /// Remove all skill-private dependency roots of one skill identifier.
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

    /// Collect all skill-local dependency install roots that are relevant for the current platform.
    /// 收集当前平台下与单个技能相关的全部技能私有依赖安装根目录。
    fn collect_skill_local_dependency_roots(
        &self,
        skill_id: &str,
        manifest: Option<&SkillDependencyManifest>,
        platform_key: &str,
    ) -> Result<BTreeSet<PathBuf>, String> {
        let mut roots = BTreeSet::new();
        let Some(manifest) = manifest else {
            return Ok(roots);
        };

        for spec in &manifest.tool_dependencies {
            self.push_dependency_root_if_applicable(
                &mut roots,
                skill_id,
                SkillDependencyKind::Tool,
                spec.name.as_str(),
                spec.version.as_deref(),
                spec.scope,
                spec.package_for_platform(platform_key).is_some(),
                platform_key,
            );
        }
        for spec in &manifest.lua_dependencies {
            self.push_dependency_root_if_applicable(
                &mut roots,
                skill_id,
                SkillDependencyKind::Lua,
                spec.name.as_str(),
                spec.version.as_deref(),
                spec.scope,
                spec.package_for_platform(platform_key).is_some(),
                platform_key,
            );
        }
        for spec in &manifest.ffi_dependencies {
            self.push_dependency_root_if_applicable(
                &mut roots,
                skill_id,
                SkillDependencyKind::Ffi,
                spec.name.as_str(),
                spec.version.as_deref(),
                spec.scope,
                spec.package_for_platform(platform_key).is_some(),
                platform_key,
            );
        }

        Ok(roots)
    }

    /// Insert one dependency install root when the dependency is skill-local and applicable to the current platform.
    /// 当依赖属于技能私有且适用于当前平台时，把其安装根目录加入集合。
    #[allow(clippy::too_many_arguments)]
    fn push_dependency_root_if_applicable(
        &self,
        roots: &mut BTreeSet<PathBuf>,
        skill_id: &str,
        kind: SkillDependencyKind,
        dependency_name: &str,
        version: Option<&str>,
        scope: DependencyScope,
        has_platform_package: bool,
        platform_key: &str,
    ) {
        if scope != DependencyScope::Skill || !has_platform_package {
            return;
        }

        let root = build_dependency_install_root(
            self.install_root_for_kind(kind, scope),
            scope,
            skill_id,
            dependency_name,
            version,
            platform_key,
        );
        roots.insert(root);
    }

    /// Detect whether one dependency is already installed by checking all declared exports.
    /// 通过检查全部声明的导出文件来判断单个依赖是否已经安装。
    fn detect_dependency(
        &self,
        request: &ResolvedDependencyRequest,
    ) -> Result<DependencyDetectionStatus, String> {
        if request.exports.is_empty() {
            return Err(format!(
                "dependency '{}' must declare at least one export",
                request.name
            ));
        }

        let all_present = request.exports.iter().all(|export| {
            request
                .install_root
                .join(
                    export
                        .target_path
                        .replace('/', std::path::MAIN_SEPARATOR_STR),
                )
                .exists()
        });
        Ok(if all_present {
            DependencyDetectionStatus::Present
        } else {
            DependencyDetectionStatus::Missing
        })
    }

    /// Fetch and parse one remote skilllist index file.
    /// 获取并解析单个远程 skilllist 索引文件。
    fn fetch_skilllist_index(&self, url: &str) -> Result<SkillListIndexFile, String> {
        let cache_key = format!(
            "skilllist-{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(url)
        );
        let cached_text = self.downloader.fetch_text(url, &cache_key)?;
        serde_yaml::from_str::<SkillListIndexFile>(&cached_text)
            .map_err(|error| format!("failed to parse skilllist index {}: {}", url, error))
    }
}

/// Ensure one root directory exists before it is used by the dependency manager.
/// 在依赖管理器使用某个根目录前确保其已经存在。
pub fn ensure_directory(root: &Path) -> Result<(), String> {
    fs::create_dir_all(root)
        .map_err(|error| format!("Failed to create {}: {}", root.display(), error))
}

/// Build the final install root of one dependency according to its scope, name, version, and platform.
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
        DependencyScope::Host => root
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

/// Build local version candidates that may map to one already-installed dependency root.
/// 构造可能映射到已安装依赖根目录的本地版本候选值。
fn local_dependency_version_candidates(version: Option<&str>) -> Vec<Option<String>> {
    let Some(raw_version) = version else {
        return vec![None];
    };
    let trimmed_version = raw_version.trim();
    let normalized_version = trimmed_version.trim_start_matches('v');
    let mut candidates = Vec::new();
    push_unique_optional_candidate(&mut candidates, Some(trimmed_version.to_string()));
    if normalized_version != trimmed_version {
        push_unique_optional_candidate(&mut candidates, Some(normalized_version.to_string()));
    }
    candidates
}

/// Build local tag candidates used for export-template probing without GitHub access.
/// 构造不访问 GitHub 时用于导出模板探测的本地标签候选值。
fn local_dependency_tag_candidates(version: Option<&str>) -> Vec<Option<String>> {
    let Some(raw_version) = version else {
        return vec![None];
    };
    let trimmed_version = raw_version.trim();
    let normalized_version = trimmed_version.trim_start_matches('v');
    let mut candidates = Vec::new();
    push_unique_optional_candidate(&mut candidates, Some(trimmed_version.to_string()));
    if normalized_version != trimmed_version {
        push_unique_optional_candidate(&mut candidates, Some(normalized_version.to_string()));
    }
    if !normalized_version.is_empty() {
        push_unique_optional_candidate(&mut candidates, Some(format!("v{}", normalized_version)));
    }
    candidates
}

/// Append one optional string candidate while preserving insertion order.
/// 在保留插入顺序的同时追加一个可选字符串候选值。
fn push_unique_optional_candidate(candidates: &mut Vec<Option<String>>, candidate: Option<String>) {
    if !candidates.iter().any(|value| value == &candidate) {
        candidates.push(candidate);
    }
}

/// Build local probe request variants for one dependency root derived from a version candidate.
/// 为一个由版本候选值派生的依赖根目录构造本地探测请求变体。
#[allow(clippy::too_many_arguments)]
fn local_dependency_probe_request_variants(
    skill_id: &str,
    kind: SkillDependencyKind,
    dependency_name: &str,
    version: Option<&str>,
    scope: DependencyScope,
    package: &crate::skill::dependencies::DependencyPackageSpec,
    platform_key: &str,
    install_root: &Path,
) -> Vec<ResolvedDependencyRequest> {
    let candidate_root = build_dependency_install_root(
        install_root,
        scope,
        skill_id,
        dependency_name,
        version,
        platform_key,
    );
    local_dependency_probe_request_variants_for_root(
        kind,
        dependency_name,
        version,
        scope,
        platform_key,
        candidate_root,
        package,
    )
}

/// Build local probe request variants for one concrete dependency root and tag candidate set.
/// 为一个具体依赖根目录和标签候选集合构造本地探测请求变体。
#[allow(clippy::too_many_arguments)]
fn local_dependency_probe_request_variants_for_root(
    kind: SkillDependencyKind,
    dependency_name: &str,
    version: Option<&str>,
    scope: DependencyScope,
    platform_key: &str,
    install_root: PathBuf,
    package: &crate::skill::dependencies::DependencyPackageSpec,
) -> Vec<ResolvedDependencyRequest> {
    local_dependency_tag_candidates(version)
        .into_iter()
        .map(|tag| ResolvedDependencyRequest {
            kind,
            name: dependency_name.to_string(),
            scope,
            platform_key: platform_key.to_string(),
            download_url: String::new(),
            version: version.map(str::to_string),
            install_root: install_root.clone(),
            archive_type: package.archive_type,
            exports: resolve_export_templates(&package.exports, version, tag.as_deref()),
        })
        .collect()
}

/// Normalize one dependency path component for stable cross-platform directory generation.
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

/// Resolve export-path templates using the final normalized version and tag values.
/// 使用最终标准化版本号与标签值解析导出路径模板。
fn resolve_export_templates(
    exports: &[DependencyExportSpec],
    version: Option<&str>,
    tag: Option<&str>,
) -> Vec<DependencyExportSpec> {
    exports
        .iter()
        .cloned()
        .map(|mut export| {
            export.archive_path = resolve_export_template_field(&export.archive_path, version, tag);
            export.target_path = resolve_export_template_field(&export.target_path, version, tag);
            export
        })
        .collect()
}

/// Resolve one export template field by replacing `{version}` and `{tag}` when values are available.
/// 在值可用时解析单个导出模板字段中的 `{version}` 与 `{tag}`。
fn resolve_export_template_field(raw: &str, version: Option<&str>, tag: Option<&str>) -> String {
    let mut value = raw.to_string();
    if let Some(version) = version {
        value = value.replace("{version}", version);
    }
    if let Some(tag) = tag {
        value = value.replace("{tag}", tag);
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skill::dependencies::{
        DependencyArchiveType, DependencyExportSpec, DependencyPackageSpec, DependencySourceSpec,
        GithubReleaseSourceSpec, SkillDependencyManifest, ToolDependencySpec, UrlSourceSpec,
    };
    use std::collections::BTreeMap;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// Build one minimal dependency manager rooted under one unique temporary test directory.
    /// 在唯一的临时测试目录下构造一个最小依赖管理器。
    fn test_manager() -> (DependencyManager, PathBuf) {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("luaskills-dependency-test-{}", unique));
        let config = DependencyManagerConfig {
            tool_root: root.join("dependencies").join("tools"),
            host_tool_root: root.join("bin").join("tools"),
            lua_root: root.join("dependencies").join("lua"),
            host_lua_root: root.join("lua_packages"),
            ffi_root: root.join("dependencies").join("ffi"),
            host_ffi_root: root.join("libs"),
            download_cache_root: root.join("temp").join("downloads"),
            allow_network_download: false,
            github_base_url: None,
            github_api_base_url: None,
        };
        (DependencyManager::new(config), root)
    }

    /// Build one minimal tool dependency spec for the current platform.
    /// 为当前平台构造一个最小工具依赖声明。
    fn tool_dependency(name: &str, version: &str, platform_key: &str) -> ToolDependencySpec {
        let mut packages = BTreeMap::new();
        packages.insert(
            platform_key.to_string(),
            DependencyPackageSpec {
                archive_type: DependencyArchiveType::Raw,
                asset_name: None,
                url: Some("https://example.invalid/package".to_string()),
                exports: vec![DependencyExportSpec {
                    archive_path: "demo.bin".to_string(),
                    target_path: "bin/demo.bin".to_string(),
                    executable: false,
                }],
            },
        );
        ToolDependencySpec {
            name: name.to_string(),
            version: Some(version.to_string()),
            required: true,
            scope: DependencyScope::Skill,
            source: DependencySourceSpec {
                source_type: DependencySourceType::Url,
                github: None,
                url: Some(UrlSourceSpec::default()),
                skilllist: None,
            },
            packages,
        }
    }

    /// Build one GitHub-release tool dependency that must not touch GitHub when exports exist.
    /// 构造一个在导出产物已存在时不应访问 GitHub 的 GitHub Release 工具依赖。
    fn github_tool_dependency(
        name: &str,
        version: Option<&str>,
        platform_key: &str,
    ) -> ToolDependencySpec {
        let mut packages = BTreeMap::new();
        packages.insert(
            platform_key.to_string(),
            DependencyPackageSpec {
                archive_type: DependencyArchiveType::Zip,
                asset_name: Some("demo-{version}.zip".to_string()),
                url: None,
                exports: vec![DependencyExportSpec {
                    archive_path: "demo-{version}.bin".to_string(),
                    target_path: "bin/demo-{version}.bin".to_string(),
                    executable: false,
                }],
            },
        );
        ToolDependencySpec {
            name: name.to_string(),
            version: version.map(str::to_string),
            required: true,
            scope: DependencyScope::Skill,
            source: DependencySourceSpec {
                source_type: DependencySourceType::GithubRelease,
                github: Some(GithubReleaseSourceSpec {
                    repo: "OpenVulcan/demo-dependency".to_string(),
                    tag_api: None,
                }),
                url: None,
                skilllist: None,
            },
            packages,
        }
    }

    /// Build one GitHub-release tool dependency whose export target uses the release tag.
    /// 构造一个导出目标使用 release 标签的 GitHub Release 工具依赖。
    fn github_tagged_tool_dependency(
        name: &str,
        version: Option<&str>,
        platform_key: &str,
    ) -> ToolDependencySpec {
        let mut dependency = github_tool_dependency(name, version, platform_key);
        let package = dependency
            .packages
            .get_mut(platform_key)
            .expect("test package should exist");
        package.exports = vec![DependencyExportSpec {
            archive_path: "demo-{tag}.bin".to_string(),
            target_path: "bin/demo-{tag}.bin".to_string(),
            executable: false,
        }];
        dependency
    }

    /// Existing GitHub-release exports should enable the skill without remote resolution.
    /// 已存在的 GitHub Release 导出产物应直接启用 skill 而不进行远程解析。
    #[test]
    fn ensure_dependency_reuses_existing_github_release_exports_without_remote_resolution() {
        let platform_key = current_platform_key();
        if platform_key == "unknown" {
            return;
        }

        let (mut manager, root) = test_manager();
        manager.config.allow_network_download = true;
        manager.config.github_api_base_url = Some("https://example.invalid/github-api".to_string());
        manager.downloader = DownloadManager::new(DownloadManagerConfig {
            cache_root: manager.config.download_cache_root.clone(),
            allow_network_download: manager.config.allow_network_download,
            github_base_url: manager.config.github_base_url.clone(),
            github_api_base_url: manager.config.github_api_base_url.clone(),
        });
        let skill_id = "demo-skill";
        let manifest = SkillDependencyManifest {
            tool_dependencies: vec![github_tool_dependency(
                "demo-tool",
                Some("1.2.3"),
                platform_key,
            )],
            lua_dependencies: Vec::new(),
            ffi_dependencies: Vec::new(),
        };
        let dependency_root = build_dependency_install_root(
            &manager.config.tool_root,
            DependencyScope::Skill,
            skill_id,
            "demo-tool",
            Some("1.2.3"),
            platform_key,
        );
        fs::create_dir_all(dependency_root.join("bin")).unwrap();
        fs::write(dependency_root.join("bin").join("demo-1.2.3.bin"), b"ready").unwrap();

        manager
            .ensure_skill_dependencies(skill_id, &manifest)
            .expect("existing exports should bypass GitHub release lookup");

        let _ = fs::remove_dir_all(root);
    }

    /// Existing version directories should be reused when a GitHub dependency omits version.
    /// 当 GitHub 依赖省略版本时，应复用已有版本目录中的导出产物。
    #[test]
    fn ensure_dependency_reuses_existing_unversioned_github_release_exports() {
        let platform_key = current_platform_key();
        if platform_key == "unknown" {
            return;
        }

        let (mut manager, root) = test_manager();
        manager.config.allow_network_download = true;
        manager.config.github_api_base_url = Some("https://example.invalid/github-api".to_string());
        manager.downloader = DownloadManager::new(DownloadManagerConfig {
            cache_root: manager.config.download_cache_root.clone(),
            allow_network_download: manager.config.allow_network_download,
            github_base_url: manager.config.github_base_url.clone(),
            github_api_base_url: manager.config.github_api_base_url.clone(),
        });
        let skill_id = "demo-skill";
        let manifest = SkillDependencyManifest {
            tool_dependencies: vec![github_tool_dependency("demo-tool", None, platform_key)],
            lua_dependencies: Vec::new(),
            ffi_dependencies: Vec::new(),
        };
        let dependency_root = build_dependency_install_root(
            &manager.config.tool_root,
            DependencyScope::Skill,
            skill_id,
            "demo-tool",
            Some("1.2.3"),
            platform_key,
        );
        fs::create_dir_all(dependency_root.join("bin")).unwrap();
        fs::write(dependency_root.join("bin").join("demo-1.2.3.bin"), b"ready").unwrap();

        manager
            .ensure_skill_dependencies(skill_id, &manifest)
            .expect("existing version directories should bypass GitHub release lookup");

        let _ = fs::remove_dir_all(root);
    }

    /// Existing GitHub-release exports using `{tag}` should reuse the likely `v` tag variant.
    /// 使用 `{tag}` 的 GitHub Release 已有导出产物应复用可能的 `v` 标签变体。
    #[test]
    fn ensure_dependency_reuses_existing_github_release_tag_exports_without_remote_resolution() {
        let platform_key = current_platform_key();
        if platform_key == "unknown" {
            return;
        }

        let (mut manager, root) = test_manager();
        manager.config.allow_network_download = true;
        manager.config.github_api_base_url = Some("https://example.invalid/github-api".to_string());
        manager.downloader = DownloadManager::new(DownloadManagerConfig {
            cache_root: manager.config.download_cache_root.clone(),
            allow_network_download: manager.config.allow_network_download,
            github_base_url: manager.config.github_base_url.clone(),
            github_api_base_url: manager.config.github_api_base_url.clone(),
        });
        let skill_id = "demo-skill";
        let manifest = SkillDependencyManifest {
            tool_dependencies: vec![github_tagged_tool_dependency(
                "demo-tool",
                Some("1.2.3"),
                platform_key,
            )],
            lua_dependencies: Vec::new(),
            ffi_dependencies: Vec::new(),
        };
        let dependency_root = build_dependency_install_root(
            &manager.config.tool_root,
            DependencyScope::Skill,
            skill_id,
            "demo-tool",
            Some("1.2.3"),
            platform_key,
        );
        fs::create_dir_all(dependency_root.join("bin")).unwrap();
        fs::write(
            dependency_root.join("bin").join("demo-v1.2.3.bin"),
            b"ready",
        )
        .unwrap();

        manager
            .ensure_skill_dependencies(skill_id, &manifest)
            .expect("existing tag exports should bypass GitHub release lookup");

        let _ = fs::remove_dir_all(root);
    }

    /// Existing unversioned GitHub exports using `{tag}` should reuse scanned version roots.
    /// 未声明版本且使用 `{tag}` 的 GitHub 已有导出产物应复用扫描到的版本根目录。
    #[test]
    fn ensure_dependency_reuses_existing_unversioned_github_release_tag_exports() {
        let platform_key = current_platform_key();
        if platform_key == "unknown" {
            return;
        }

        let (mut manager, root) = test_manager();
        manager.config.allow_network_download = true;
        manager.config.github_api_base_url = Some("https://example.invalid/github-api".to_string());
        manager.downloader = DownloadManager::new(DownloadManagerConfig {
            cache_root: manager.config.download_cache_root.clone(),
            allow_network_download: manager.config.allow_network_download,
            github_base_url: manager.config.github_base_url.clone(),
            github_api_base_url: manager.config.github_api_base_url.clone(),
        });
        let skill_id = "demo-skill";
        let manifest = SkillDependencyManifest {
            tool_dependencies: vec![github_tagged_tool_dependency(
                "demo-tool",
                None,
                platform_key,
            )],
            lua_dependencies: Vec::new(),
            ffi_dependencies: Vec::new(),
        };
        let dependency_root = build_dependency_install_root(
            &manager.config.tool_root,
            DependencyScope::Skill,
            skill_id,
            "demo-tool",
            Some("1.2.3"),
            platform_key,
        );
        fs::create_dir_all(dependency_root.join("bin")).unwrap();
        fs::write(
            dependency_root.join("bin").join("demo-v1.2.3.bin"),
            b"ready",
        )
        .unwrap();

        manager
            .ensure_skill_dependencies(skill_id, &manifest)
            .expect("existing unversioned tag exports should bypass GitHub release lookup");

        let _ = fs::remove_dir_all(root);
    }

    /// Updated-skill cleanup removes stale private dependency roots while preserving unchanged ones.
    /// 更新后的清理流程会删除过期的私有依赖根，同时保留未变化的依赖。
    #[test]
    fn cleanup_updated_skill_dependencies_removes_stale_roots_and_keeps_reused_roots() {
        let platform_key = current_platform_key();
        if platform_key == "unknown" {
            return;
        }

        let (manager, root) = test_manager();
        let skill_id = "demo-skill";
        let previous_manifest = SkillDependencyManifest {
            tool_dependencies: vec![
                tool_dependency("rg", "14.1.1", platform_key),
                tool_dependency("fd", "9.0.0", platform_key),
            ],
            lua_dependencies: Vec::new(),
            ffi_dependencies: Vec::new(),
        };
        let current_manifest = SkillDependencyManifest {
            tool_dependencies: vec![
                tool_dependency("rg", "14.1.2", platform_key),
                tool_dependency("fd", "9.0.0", platform_key),
            ],
            lua_dependencies: Vec::new(),
            ffi_dependencies: Vec::new(),
        };

        let stale_root = build_dependency_install_root(
            &manager.config.tool_root,
            DependencyScope::Skill,
            skill_id,
            "rg",
            Some("14.1.1"),
            platform_key,
        );
        let kept_root = build_dependency_install_root(
            &manager.config.tool_root,
            DependencyScope::Skill,
            skill_id,
            "fd",
            Some("9.0.0"),
            platform_key,
        );
        let current_root = build_dependency_install_root(
            &manager.config.tool_root,
            DependencyScope::Skill,
            skill_id,
            "rg",
            Some("14.1.2"),
            platform_key,
        );

        fs::create_dir_all(stale_root.join("bin")).unwrap();
        fs::write(stale_root.join("bin").join("demo.bin"), b"old").unwrap();
        fs::create_dir_all(kept_root.join("bin")).unwrap();
        fs::write(kept_root.join("bin").join("demo.bin"), b"keep").unwrap();
        fs::create_dir_all(current_root.join("bin")).unwrap();
        fs::write(current_root.join("bin").join("demo.bin"), b"new").unwrap();

        manager
            .cleanup_updated_skill_dependencies(
                skill_id,
                Some(&previous_manifest),
                Some(&current_manifest),
            )
            .unwrap();

        assert!(
            !stale_root.exists(),
            "stale dependency root should be removed"
        );
        assert!(
            kept_root.exists(),
            "unchanged dependency root should be preserved"
        );
        assert!(
            current_root.exists(),
            "current dependency root should be preserved"
        );

        let _ = fs::remove_dir_all(root);
    }

    /// Updated-skill cleanup keeps identical dependency roots when the manifest does not change.
    /// 当依赖清单没有变化时，更新清理流程会保留完全相同的依赖根目录。
    #[test]
    fn cleanup_updated_skill_dependencies_keeps_identical_roots() {
        let platform_key = current_platform_key();
        if platform_key == "unknown" {
            return;
        }

        let (manager, root) = test_manager();
        let skill_id = "demo-skill";
        let manifest = SkillDependencyManifest {
            tool_dependencies: vec![tool_dependency("rg", "14.1.1", platform_key)],
            lua_dependencies: Vec::new(),
            ffi_dependencies: Vec::new(),
        };

        let dependency_root = build_dependency_install_root(
            &manager.config.tool_root,
            DependencyScope::Skill,
            skill_id,
            "rg",
            Some("14.1.1"),
            platform_key,
        );
        fs::create_dir_all(dependency_root.join("bin")).unwrap();
        fs::write(dependency_root.join("bin").join("demo.bin"), b"keep").unwrap();

        manager
            .cleanup_updated_skill_dependencies(skill_id, Some(&manifest), Some(&manifest))
            .unwrap();

        assert!(
            dependency_root.exists(),
            "unchanged dependency root should remain"
        );

        let _ = fs::remove_dir_all(root);
    }

    /// Updated-skill cleanup removes all old private dependency roots that disappear from the new manifest.
    /// 当新清单移除了旧依赖时，更新清理流程会删除全部过期的私有依赖根目录。
    #[test]
    fn cleanup_updated_skill_dependencies_removes_deleted_dependencies() {
        let platform_key = current_platform_key();
        if platform_key == "unknown" {
            return;
        }

        let (manager, root) = test_manager();
        let skill_id = "demo-skill";
        let previous_manifest = SkillDependencyManifest {
            tool_dependencies: vec![
                tool_dependency("rg", "14.1.1", platform_key),
                tool_dependency("fd", "9.0.0", platform_key),
            ],
            lua_dependencies: Vec::new(),
            ffi_dependencies: Vec::new(),
        };
        let current_manifest = SkillDependencyManifest::default();

        let rg_root = build_dependency_install_root(
            &manager.config.tool_root,
            DependencyScope::Skill,
            skill_id,
            "rg",
            Some("14.1.1"),
            platform_key,
        );
        let fd_root = build_dependency_install_root(
            &manager.config.tool_root,
            DependencyScope::Skill,
            skill_id,
            "fd",
            Some("9.0.0"),
            platform_key,
        );
        fs::create_dir_all(rg_root.join("bin")).unwrap();
        fs::write(rg_root.join("bin").join("demo.bin"), b"old-rg").unwrap();
        fs::create_dir_all(fd_root.join("bin")).unwrap();
        fs::write(fd_root.join("bin").join("demo.bin"), b"old-fd").unwrap();

        manager
            .cleanup_updated_skill_dependencies(
                skill_id,
                Some(&previous_manifest),
                Some(&current_manifest),
            )
            .unwrap();

        assert!(
            !rg_root.exists(),
            "removed dependency root should be deleted"
        );
        assert!(
            !fd_root.exists(),
            "removed dependency root should be deleted"
        );

        let _ = fs::remove_dir_all(root);
    }

    /// Updated-skill cleanup does not remove current roots when the new manifest only adds dependencies.
    /// 当新清单只是新增依赖时，更新清理流程不会误删当前仍然有效的依赖根目录。
    #[test]
    fn cleanup_updated_skill_dependencies_preserves_existing_roots_for_add_only_changes() {
        let platform_key = current_platform_key();
        if platform_key == "unknown" {
            return;
        }

        let (manager, root) = test_manager();
        let skill_id = "demo-skill";
        let previous_manifest = SkillDependencyManifest {
            tool_dependencies: vec![tool_dependency("rg", "14.1.1", platform_key)],
            lua_dependencies: Vec::new(),
            ffi_dependencies: Vec::new(),
        };
        let current_manifest = SkillDependencyManifest {
            tool_dependencies: vec![
                tool_dependency("rg", "14.1.1", platform_key),
                tool_dependency("fd", "9.0.0", platform_key),
            ],
            lua_dependencies: Vec::new(),
            ffi_dependencies: Vec::new(),
        };

        let rg_root = build_dependency_install_root(
            &manager.config.tool_root,
            DependencyScope::Skill,
            skill_id,
            "rg",
            Some("14.1.1"),
            platform_key,
        );
        let fd_root = build_dependency_install_root(
            &manager.config.tool_root,
            DependencyScope::Skill,
            skill_id,
            "fd",
            Some("9.0.0"),
            platform_key,
        );
        fs::create_dir_all(rg_root.join("bin")).unwrap();
        fs::write(rg_root.join("bin").join("demo.bin"), b"keep-rg").unwrap();
        fs::create_dir_all(fd_root.join("bin")).unwrap();
        fs::write(fd_root.join("bin").join("demo.bin"), b"new-fd").unwrap();

        manager
            .cleanup_updated_skill_dependencies(
                skill_id,
                Some(&previous_manifest),
                Some(&current_manifest),
            )
            .unwrap();

        assert!(
            rg_root.exists(),
            "existing dependency root should be preserved"
        );
        assert!(
            fd_root.exists(),
            "new dependency root should remain untouched"
        );

        let _ = fs::remove_dir_all(root);
    }

    /// GitHub-release export templates should resolve `{version}` placeholders before archive extraction.
    /// GitHub Release 导出模板应在归档解包前解析 `{version}` 占位符。
    #[test]
    fn resolve_export_templates_expands_version_placeholder() {
        let exports = vec![DependencyExportSpec {
            archive_path: "ripgrep-{version}-x86_64-pc-windows-msvc/rg.exe".to_string(),
            target_path: "bin/rg-{version}.exe".to_string(),
            executable: false,
        }];

        let resolved = resolve_export_templates(&exports, Some("14.1.1"), Some("14.1.1"));
        assert_eq!(
            resolved[0].archive_path,
            "ripgrep-14.1.1-x86_64-pc-windows-msvc/rg.exe"
        );
        assert_eq!(resolved[0].target_path, "bin/rg-14.1.1.exe");
    }
}
