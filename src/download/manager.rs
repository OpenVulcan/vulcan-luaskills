use std::fs;
use std::path::{Path, PathBuf};

use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};

use crate::dependency::types::DependencySourceType;
use crate::download::github::{GithubReleaseApiResponse, rewrite_github_download_url};
use crate::runtime_logging::info as log_info;
use crate::skill::dependencies::GithubReleaseSourceSpec;

/// Download-manager configuration that describes cache roots and upstream policy.
/// 描述缓存根目录与上游策略的下载管理配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadManagerConfig {
    /// Root directory used to cache downloaded archives and remote manifests.
    /// 用于缓存下载归档与远程清单的根目录。
    pub cache_root: PathBuf,
    /// Whether network downloads are allowed.
    /// 是否允许网络下载。
    pub allow_network_download: bool,
    /// Optional GitHub site base URL override.
    /// 可选的 GitHub 站点基址覆盖。
    pub github_base_url: Option<String>,
    /// Optional GitHub API base URL override.
    /// 可选的 GitHub API 基址覆盖。
    pub github_api_base_url: Option<String>,
}

/// One normalized download request consumed by the shared download layer.
/// 由共享下载层消费的单次标准化下载请求。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadRequest {
    /// Source type of the current download request.
    /// 当前下载请求的来源类型。
    pub source_type: DependencySourceType,
    /// Exact source locator, usually one URL.
    /// 精确来源定位值，通常为一个 URL。
    pub source_locator: String,
    /// Stable cache key used to derive one cache file path.
    /// 用于派生缓存文件路径的稳定缓存键。
    pub cache_key: String,
}

/// Shared downloader used by dependency resolution and install flows.
/// 供依赖解析与安装流程共用的共享下载器。
pub struct DownloadManager {
    config: DownloadManagerConfig,
    client: Client,
}

impl DownloadManager {
    /// Create one shared downloader from configuration.
    /// 基于配置创建一个共享下载器。
    pub fn new(config: DownloadManagerConfig) -> Self {
        let client = Client::builder()
            .user_agent("vulcan-luaskills/0.1.0")
            .build()
            .expect("reqwest client should build");
        Self { config, client }
    }

    /// Download one binary payload into the cache directory and return the cached file path.
    /// 把单个二进制载荷下载到缓存目录并返回缓存文件路径。
    pub fn download(&self, request: &DownloadRequest) -> Result<PathBuf, String> {
        self.ensure_network_allowed()?;
        fs::create_dir_all(&self.config.cache_root).map_err(|error| {
            format!("Failed to create download cache root {}: {}", self.config.cache_root.display(), error)
        })?;

        let file_extension = infer_download_extension(&request.source_locator);
        let target_path = self
            .config
            .cache_root
            .join(format!("{}{}", request.cache_key, file_extension));
        if target_path.exists() {
            return Ok(target_path);
        }

        log_info(format!("[LuaSkills:download] Fetching {} from {}", request.cache_key, request.source_locator));
        let response = self
            .client
            .get(&request.source_locator)
            .send()
            .map_err(|error| format!("Failed to download {}: {}", request.source_locator, error))?
            .error_for_status()
            .map_err(|error| format!("Failed to download {}: {}", request.source_locator, error))?;
        let bytes = response
            .bytes()
            .map_err(|error| format!("Failed to read {}: {}", request.source_locator, error))?;
        fs::write(&target_path, &bytes)
            .map_err(|error| format!("Failed to write {}: {}", target_path.display(), error))?;
        Ok(target_path)
    }

    /// Fetch one UTF-8 text resource over HTTP.
    /// 通过 HTTP 获取单个 UTF-8 文本资源。
    pub fn fetch_text(&self, url: &str, cache_key: &str) -> Result<String, String> {
        let cached_path = self.download(&DownloadRequest {
            source_type: DependencySourceType::Url,
            source_locator: url.to_string(),
            cache_key: cache_key.to_string(),
        })?;
        fs::read_to_string(&cached_path)
            .map_err(|error| format!("Failed to read {}: {}", cached_path.display(), error))
    }

    /// Resolve one GitHub release asset into an exact browser download URL.
    /// 把单个 GitHub Release 资产解析为精确浏览器下载地址。
    pub fn resolve_github_release_asset_url(
        &self,
        source: &GithubReleaseSourceSpec,
        asset_name_template: &str,
        expected_version: Option<&str>,
    ) -> Result<String, String> {
        self.ensure_network_allowed()?;
        let api_url = source
            .tag_api
            .clone()
            .unwrap_or_else(|| build_github_release_api_url(&self.config, source.repo.as_str()));
        let response_text = self
            .client
            .get(&api_url)
            .send()
            .map_err(|error| format!("Failed to query {}: {}", api_url, error))?
            .error_for_status()
            .map_err(|error| format!("Failed to query {}: {}", api_url, error))?
            .text()
            .map_err(|error| format!("Failed to read {}: {}", api_url, error))?;
        let release: GithubReleaseApiResponse = serde_json::from_str(&response_text)
            .map_err(|error| format!("Failed to parse {}: {}", api_url, error))?;
        let normalized_version = normalize_release_version(
            expected_version.unwrap_or(release.tag_name.as_str()),
            release.tag_name.as_str(),
        );
        let expected_asset_name = asset_name_template
            .replace("{version}", normalized_version.as_str())
            .replace("{tag}", release.tag_name.as_str());
        let asset = release
            .assets
            .iter()
            .find(|asset| asset.name == expected_asset_name)
            .ok_or_else(|| {
                format!("GitHub release {} does not contain asset '{}'", release.tag_name, expected_asset_name)
            })?;
        Ok(rewrite_github_download_url(
            asset.browser_download_url.as_str(),
            self.config.github_base_url.as_deref(),
        ))
    }

    /// Ensure the downloader is allowed to hit the network.
    /// 确保当前下载器允许访问网络。
    fn ensure_network_allowed(&self) -> Result<(), String> {
        if self.config.allow_network_download {
            Ok(())
        } else {
            Err("network download is disabled by host policy".to_string())
        }
    }
}

/// Build the GitHub latest-release API URL for one repository.
/// 为单个仓库构造 GitHub 最新 release API 地址。
fn build_github_release_api_url(
    config: &DownloadManagerConfig,
    repo: &str,
) -> String {
    let normalized_repo = repo
        .trim()
        .trim_start_matches("https://github.com/")
        .trim_start_matches("http://github.com/")
        .trim_matches('/');
    let api_base = config
        .github_api_base_url
        .as_deref()
        .unwrap_or("https://api.github.com")
        .trim_end_matches('/');
    format!("{}/repos/{}/releases/latest", api_base, normalized_repo)
}

/// Normalize the effective release version used for asset name interpolation.
/// 归一化用于资产名插值的生效 release 版本字符串。
fn normalize_release_version(expected_version: &str, tag_name: &str) -> String {
    let trimmed_expected = expected_version.trim();
    if !trimmed_expected.is_empty() {
        return trimmed_expected.trim_start_matches('v').to_string();
    }
    tag_name.trim().trim_start_matches('v').to_string()
}

/// Infer a cache file extension from one download URL.
/// 根据下载 URL 推断缓存文件扩展名。
fn infer_download_extension(url: &str) -> &'static str {
    let lower = url.to_ascii_lowercase();
    if lower.ends_with(".tar.gz") {
        ".tar.gz"
    } else if lower.ends_with(".zip") {
        ".zip"
    } else if let Some(extension) = Path::new(url)
        .extension()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
    {
        if extension.eq_ignore_ascii_case("gz") && lower.ends_with(".tar.gz") {
            ".tar.gz"
        } else {
            match extension {
                "txt" => ".txt",
                "yaml" => ".yaml",
                "yml" => ".yml",
                "json" => ".json",
                "dll" => ".dll",
                "so" => ".so",
                "dylib" => ".dylib",
                "lua" => ".lua",
                _ => "",
            }
        }
    } else {
        ""
    }
}
