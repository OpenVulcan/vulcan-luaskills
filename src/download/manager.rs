use std::fs;
use std::path::{Path, PathBuf};
use std::thread;

use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::dependency::types::DependencySourceType;
use crate::download::github::{GithubReleaseApiResponse, rewrite_github_download_url};
use crate::runtime_logging::info as log_info;
use crate::skill::dependencies::GithubReleaseSourceSpec;

/// One resolved GitHub release asset with the tag/version metadata needed by install flows.
/// 安装流程需要的单个已解析 GitHub release 资产及其标签/版本元数据。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedGithubReleaseAsset {
    /// Exact GitHub release tag name returned by the upstream API.
    /// 上游 API 返回的精确 GitHub release 标签名。
    pub tag_name: String,
    /// Normalized semantic version string derived from the release tag.
    /// 从 release 标签派生出的标准化语义化版本字符串。
    pub version: String,
    /// Exact asset file name selected from the release payload.
    /// 从 release 载荷中选中的精确资产文件名。
    pub asset_name: String,
    /// Exact browser download URL after optional host-side GitHub URL rewriting.
    /// 经过可选宿主侧 GitHub URL 重写后的精确浏览器下载地址。
    pub download_url: String,
    /// Expected SHA-256 checksum for the selected asset when one checksum manifest is available.
    /// 当存在校验清单时，所选资产对应的期望 SHA-256 校验值。
    pub sha256: Option<String>,
}

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
}

impl DownloadManager {
    /// Create one shared downloader from configuration.
    /// 基于配置创建一个共享下载器。
    pub fn new(config: DownloadManagerConfig) -> Self {
        Self { config }
    }

    /// Download one binary payload into the cache directory and return the cached file path.
    /// 把单个二进制载荷下载到缓存目录并返回缓存文件路径。
    pub fn download(&self, request: &DownloadRequest) -> Result<PathBuf, String> {
        self.ensure_network_allowed()?;
        fs::create_dir_all(&self.config.cache_root).map_err(|error| {
            format!(
                "Failed to create download cache root {}: {}",
                self.config.cache_root.display(),
                error
            )
        })?;

        let file_extension = infer_download_extension(&request.source_locator);
        let target_path = self
            .config
            .cache_root
            .join(format!("{}{}", request.cache_key, file_extension));
        if target_path.exists() {
            return Ok(target_path);
        }

        log_info(format!(
            "[LuaSkills:download] Fetching {} from {}",
            request.cache_key, request.source_locator
        ));
        let source_locator = request.source_locator.clone();
        let bytes = self.run_http_task(move |client| {
            let response = client
                .get(&source_locator)
                .send()
                .map_err(|error| format!("Failed to download {}: {}", source_locator, error))?
                .error_for_status()
                .map_err(|error| format!("Failed to download {}: {}", source_locator, error))?;
            response
                .bytes()
                .map(|value| value.to_vec())
                .map_err(|error| format!("Failed to read {}: {}", source_locator, error))
        })?;
        fs::write(&target_path, &bytes)
            .map_err(|error| format!("Failed to write {}: {}", target_path.display(), error))?;
        Ok(target_path)
    }

    /// Download one binary payload and verify one expected SHA-256 checksum.
    /// 下载单个二进制载荷，并验证其期望的 SHA-256 校验值。
    pub fn download_with_sha256(
        &self,
        request: &DownloadRequest,
        expected_sha256: &str,
    ) -> Result<PathBuf, String> {
        let target_path = self.download(request)?;
        if let Err(error) = verify_file_sha256(&target_path, expected_sha256) {
            let _ = fs::remove_file(&target_path);
            return Err(error);
        }
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
        Ok(self
            .resolve_github_release_asset(source, asset_name_template, expected_version)?
            .download_url)
    }

    /// Resolve one GitHub latest-release asset together with its tag and normalized version.
    /// 解析单个 GitHub 最新 release 资产，并返回其标签与标准化版本。
    pub fn resolve_github_release_asset(
        &self,
        source: &GithubReleaseSourceSpec,
        asset_name_template: &str,
        expected_version: Option<&str>,
    ) -> Result<ResolvedGithubReleaseAsset, String> {
        self.ensure_network_allowed()?;
        let release = self.fetch_github_release(source, expected_version)?;
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
                format!(
                    "GitHub release {} does not contain asset '{}'",
                    release.tag_name, expected_asset_name
                )
            })?;
        Ok(ResolvedGithubReleaseAsset {
            tag_name: release.tag_name.clone(),
            version: normalized_version,
            asset_name: asset.name.clone(),
            download_url: rewrite_github_download_url(
                asset.browser_download_url.as_str(),
                self.config.github_base_url.as_deref(),
            ),
            sha256: None,
        })
    }

    /// Resolve one managed GitHub skill release asset together with its checksum metadata.
    /// 解析单个受管 GitHub 技能 release 资产及其校验和元数据。
    pub fn resolve_github_managed_skill_release_asset(
        &self,
        source: &GithubReleaseSourceSpec,
        skill_id: &str,
        expected_version: Option<&str>,
    ) -> Result<ResolvedGithubReleaseAsset, String> {
        let mut resolved = self.resolve_github_release_asset(
            source,
            &format!("{}-v{{version}}-skill.zip", skill_id),
            expected_version,
        )?;
        let release = self.fetch_github_release(source, Some(resolved.version.as_str()))?;
        let checksum_asset_name = format!("{}-v{}-checksums.txt", skill_id, resolved.version);
        let checksum_asset = release
            .assets
            .iter()
            .find(|asset| asset.name == checksum_asset_name)
            .ok_or_else(|| {
                format!(
                    "GitHub release {} does not contain checksum asset '{}'",
                    release.tag_name, checksum_asset_name
                )
            })?;
        let checksum_url = rewrite_github_download_url(
            checksum_asset.browser_download_url.as_str(),
            self.config.github_base_url.as_deref(),
        );
        let checksum_text = self.fetch_text(
            checksum_url.as_str(),
            &format!(
                "github-checksums-{}-{}",
                sanitize_cache_key_fragment(source.repo.as_str()),
                sanitize_cache_key_fragment(release.tag_name.as_str())
            ),
        )?;
        resolved.sha256 = Some(parse_checksum_manifest_for_asset(
            checksum_text.as_str(),
            resolved.asset_name.as_str(),
        )?);
        Ok(resolved)
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

    /// Fetch one GitHub release payload, preferring an explicit version/tag when provided.
    /// 获取单个 GitHub release 载荷；若提供版本号则优先按显式版本标签解析。
    fn fetch_github_release(
        &self,
        source: &GithubReleaseSourceSpec,
        expected_version: Option<&str>,
    ) -> Result<GithubReleaseApiResponse, String> {
        if let Some(tag_api) = source.tag_api.as_ref() {
            return self.fetch_github_release_from_url(tag_api);
        }

        if let Some(expected_version) = expected_version {
            let trimmed_version = expected_version.trim().trim_start_matches('v');
            if !trimmed_version.is_empty() {
                let candidate_tags = [trimmed_version.to_string(), format!("v{}", trimmed_version)];
                let mut last_not_found = None;
                for candidate_tag in candidate_tags {
                    let api_url = build_github_release_tag_api_url(
                        &self.config,
                        source.repo.as_str(),
                        &candidate_tag,
                    );
                    match self.try_fetch_github_release_from_url(&api_url)? {
                        Some(release) => return Ok(release),
                        None => last_not_found = Some(api_url),
                    }
                }
                return Err(format!(
                    "Failed to resolve GitHub release for repo '{}' and version '{}'; attempted tag endpoints ending with '{}'",
                    source.repo,
                    trimmed_version,
                    last_not_found.unwrap_or_default()
                ));
            }
        }

        let api_url = build_github_release_api_url(&self.config, source.repo.as_str());
        self.fetch_github_release_from_url(&api_url)
    }

    /// Fetch one GitHub release payload from one exact API URL and fail on any non-success status.
    /// 从精确 API URL 获取单个 GitHub release 载荷，并在非成功状态时直接失败。
    fn fetch_github_release_from_url(
        &self,
        api_url: &str,
    ) -> Result<GithubReleaseApiResponse, String> {
        let api_url = api_url.to_string();
        let request_url = api_url.clone();
        let response_text = self.run_http_task(move |client| {
            client
                .get(&request_url)
                .send()
                .map_err(|error| format!("Failed to query {}: {}", request_url, error))?
                .error_for_status()
                .map_err(|error| format!("Failed to query {}: {}", request_url, error))?
                .text()
                .map_err(|error| format!("Failed to read {}: {}", request_url, error))
        })?;
        serde_json::from_str(&response_text)
            .map_err(|error| format!("Failed to parse {}: {}", api_url, error))
    }

    /// Try to fetch one GitHub release payload from one exact API URL, returning `None` on 404.
    /// 尝试从精确 API URL 获取单个 GitHub release 载荷；遇到 404 时返回 `None`。
    fn try_fetch_github_release_from_url(
        &self,
        api_url: &str,
    ) -> Result<Option<GithubReleaseApiResponse>, String> {
        let api_url = api_url.to_string();
        self.run_http_task(move |client| {
            let response = client
                .get(&api_url)
                .send()
                .map_err(|error| format!("Failed to query {}: {}", api_url, error))?;
            if response.status() == StatusCode::NOT_FOUND {
                return Ok(None);
            }
            let response = response
                .error_for_status()
                .map_err(|error| format!("Failed to query {}: {}", api_url, error))?;
            let response_text = response
                .text()
                .map_err(|error| format!("Failed to read {}: {}", api_url, error))?;
            let release = serde_json::from_str(&response_text)
                .map_err(|error| format!("Failed to parse {}: {}", api_url, error))?;
            Ok(Some(release))
        })
    }

    /// Run one blocking HTTP task inside a dedicated OS thread to stay independent from Tokio contexts.
    /// 在专用操作系统线程中运行单个阻塞式 HTTP 任务，以避免依赖 Tokio 上下文。
    fn run_http_task<T, F>(&self, operation: F) -> Result<T, String>
    where
        T: Send + 'static,
        F: FnOnce(reqwest::blocking::Client) -> Result<T, String> + Send + 'static,
    {
        thread::spawn(move || {
            let client = Self::build_http_client()?;
            operation(client)
        })
        .join()
        .map_err(|_| "Blocking HTTP worker thread panicked".to_string())?
    }

    /// Build one blocking HTTP client only when a network operation is actually needed.
    /// 仅在真正需要网络操作时构建一个阻塞式 HTTP 客户端。
    fn build_http_client() -> Result<reqwest::blocking::Client, String> {
        reqwest::blocking::Client::builder()
            .user_agent("vulcan-luaskills/0.1.0")
            .build()
            .map_err(|error| format!("Failed to build HTTP client: {}", error))
    }
}

/// Build the GitHub latest-release API URL for one repository.
/// 为单个仓库构造 GitHub 最新 release API 地址。
fn build_github_release_api_url(config: &DownloadManagerConfig, repo: &str) -> String {
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

/// Build the GitHub release-by-tag API URL for one repository and tag.
/// 为单个仓库和标签构造 GitHub 按标签查询 release 的 API 地址。
fn build_github_release_tag_api_url(
    config: &DownloadManagerConfig,
    repo: &str,
    tag: &str,
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
    format!(
        "{}/repos/{}/releases/tags/{}",
        api_base,
        normalized_repo,
        tag.trim()
    )
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

/// Parse one checksum manifest and return the SHA-256 value matching one asset name.
/// 解析单个校验清单，并返回与某个资产名称匹配的 SHA-256 值。
fn parse_checksum_manifest_for_asset(content: &str, asset_name: &str) -> Result<String, String> {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        let checksum = parts.next().unwrap_or_default().trim();
        let file_name = parts
            .next()
            .unwrap_or_default()
            .trim_start_matches('*')
            .trim();
        if file_name == asset_name {
            if checksum.len() == 64 && checksum.chars().all(|value| value.is_ascii_hexdigit()) {
                return Ok(checksum.to_ascii_lowercase());
            }
            return Err(format!(
                "Checksum entry for '{}' is not one valid SHA-256 value",
                asset_name
            ));
        }
    }
    Err(format!(
        "Checksum manifest does not contain an entry for '{}'",
        asset_name
    ))
}

/// Verify one downloaded file against one expected SHA-256 checksum.
/// 使用单个期望的 SHA-256 校验值验证一个已下载文件。
fn verify_file_sha256(path: &Path, expected_sha256: &str) -> Result<(), String> {
    let expected = expected_sha256.trim().to_ascii_lowercase();
    if expected.len() != 64 || !expected.chars().all(|value| value.is_ascii_hexdigit()) {
        return Err(format!(
            "Expected checksum for {} is not one valid SHA-256 value",
            path.display()
        ));
    }
    let bytes =
        fs::read(path).map_err(|error| format!("Failed to read {}: {}", path.display(), error))?;
    let actual = format!("{:x}", Sha256::digest(&bytes));
    if actual != expected {
        return Err(format!(
            "Checksum mismatch for {}: expected {}, got {}",
            path.display(),
            expected,
            actual
        ));
    }
    Ok(())
}

/// Sanitize one cache-key fragment so it can safely participate in cache file names.
/// 规范化单个缓存键片段，使其可以安全参与缓存文件名构造。
fn sanitize_cache_key_fragment(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' => ch,
            _ => '-',
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{parse_checksum_manifest_for_asset, verify_file_sha256};

    /// Verify that the checksum manifest parser resolves one matching SHA-256 entry.
    /// 验证校验清单解析器能够解析出匹配的 SHA-256 条目。
    #[test]
    fn checksum_manifest_parser_returns_matching_sha256() {
        let checksum = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let manifest = format!(
            "{}  demo-v0.1.0-skill.zip\n{}  other-file.zip\n",
            checksum, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        let parsed = parse_checksum_manifest_for_asset(&manifest, "demo-v0.1.0-skill.zip")
            .expect("checksum should be parsed");
        assert_eq!(parsed, checksum);
    }

    /// Verify that file SHA-256 verification succeeds for one matching payload.
    /// 验证当文件内容匹配时，文件 SHA-256 校验会成功。
    #[test]
    fn file_sha256_verification_succeeds_for_matching_payload() {
        let temp_root = std::env::temp_dir().join(format!(
            "vulcan_luaskills_download_checksum_test_{}",
            std::process::id()
        ));
        if temp_root.exists() {
            let _ = std::fs::remove_dir_all(&temp_root);
        }
        std::fs::create_dir_all(&temp_root).expect("temp root should be created");
        let file_path = temp_root.join("payload.txt");
        std::fs::write(&file_path, b"hello world").expect("payload should be written");
        let checksum = "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";
        verify_file_sha256(&file_path, checksum).expect("checksum should match");
        let _ = std::fs::remove_dir_all(&temp_root);
    }
}
