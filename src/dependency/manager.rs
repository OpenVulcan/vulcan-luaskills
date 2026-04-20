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
use crate::runtime_options::RuntimeSkillRoot;

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
                "current platform is not supported by LuaSkills dependency manager"
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
                log_info(format!("[LuaSkills:dependency] Skill '{}' reuses existing dependency '{}' on {}", skill_id, dependency_name, platform_key));
                Ok(())
            }
            DependencyDetectionStatus::Missing => {
                if scope == DependencyScope::Host {
                    if required {
                        return Err(format!("required host dependency '{}' is missing", dependency_name));
                    }
                    log_warn(format!("[LuaSkills:dependency] Optional host dependency '{}' is missing", dependency_name));
                    return Ok(());
                }
                if !self.config.allow_network_download {
                    if required {
                        return Err(format!("required dependency '{}' is missing and network download is disabled", dependency_name));
                    }
                    log_warn(format!("[LuaSkills:dependency] Optional dependency '{}' is missing and download is disabled", dependency_name));
                    return Ok(());
                }

                let cache_key = format!("{}-{}-{}", match kind {
                        SkillDependencyKind::Tool => "tool",
                        SkillDependencyKind::Lua => "lua",
                        SkillDependencyKind::Ffi => "ffi",
                    }, dependency_name, platform_key);
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
                    return Err(format!("dependency '{}' was downloaded but exported files are still missing", dependency_name));
                }

                log_info(format!("[LuaSkills:dependency] Installed dependency '{}' for skill '{}' on {}", dependency_name, skill_id, platform_key));
                Ok(())
            }
        }
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
            DependencySourceType::GithubRelease | DependencySourceType::Url => package
                .cloned()
                .ok_or_else(|| {
                    format!("dependency '{}' does not declare package metadata for platform '{}'", dependency_name, platform_key)
                })?,
            DependencySourceType::SkillList => {
                let skilllist = source.skilllist.as_ref().ok_or_else(|| {
                    format!("dependency '{}' declares skilllist source but misses skilllist config", dependency_name)
                })?;
                let index_file = self.fetch_skilllist_index(&skilllist.url)?;
                let package_manifest = index_file
                    .packages
                    .get(&skilllist.package)
                    .ok_or_else(|| {
                        format!("dependency '{}' cannot find package '{}' in skilllist", dependency_name, skilllist.package)
                    })?;
                package_manifest
                    .packages
                    .get(platform_key)
                    .cloned()
                    .ok_or_else(|| {
                        format!("dependency '{}' skilllist package '{}' does not support platform '{}'", dependency_name, skilllist.package, platform_key)
                    })?
            }
        };

        let download_url = match source_type {
            DependencySourceType::GithubRelease => {
                let github_source = source.github.as_ref().ok_or_else(|| {
                    format!("dependency '{}' declares github_release source but misses github config", dependency_name)
                })?;
                let asset_name = resolved_package.asset_name.as_ref().ok_or_else(|| {
                    format!("dependency '{}' github package for platform '{}' must declare asset_name", dependency_name, platform_key)
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
                    format!("dependency '{}' platform '{}' must declare url", dependency_name, platform_key)
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

    /// Remove dependencies owned by one uninstalled skill.
    /// 清理一个已卸载技能拥有的依赖。
    pub fn cleanup_uninstalled_skill_dependencies(
        &self,
        base_dir: &Path,
        override_dir: Option<&Path>,
        removed_skill_id: &str,
        removed_manifest: Option<&SkillDependencyManifest>,
    ) -> Result<(), String> {
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

    /// Detect whether one dependency is already installed by checking all declared exports.
    /// 通过检查全部声明的导出文件来判断单个依赖是否已经安装。
    fn detect_dependency(
        &self,
        request: &ResolvedDependencyRequest,
    ) -> Result<DependencyDetectionStatus, String> {
        if request.exports.is_empty() {
            return Err(format!("dependency '{}' must declare at least one export", request.name));
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

    /// Fetch and parse one remote skilllist index file.
    /// 获取并解析单个远程 skilllist 索引文件。
    fn fetch_skilllist_index(&self, url: &str) -> Result<SkillListIndexFile, String> {
        let cache_key = format!("skilllist-{}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(url));
        let cached_text = self.downloader.fetch_text(url, &cache_key)?;
        serde_yaml::from_str::<SkillListIndexFile>(&cached_text).map_err(|error| {
            format!("failed to parse skilllist index {}: {}", url, error)
        })
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
