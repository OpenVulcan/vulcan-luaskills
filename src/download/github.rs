use serde::Deserialize;

/// English: Minimal GitHub release API asset record used to resolve browser download URLs.
/// 用于解析浏览器下载地址的最小 GitHub release API 资产记录。
#[derive(Debug, Clone, Deserialize)]
pub struct GithubReleaseAssetRecord {
    /// English: Exact asset file name exposed by GitHub release metadata.
    /// GitHub release 元数据中暴露的精确资产文件名。
    pub name: String,
    /// English: Browser download URL exposed by GitHub release metadata.
    /// GitHub release 元数据中暴露的浏览器下载地址。
    pub browser_download_url: String,
}

/// English: Minimal GitHub release API response used by the downloader.
/// 下载器使用的最小 GitHub release API 响应。
#[derive(Debug, Clone, Deserialize)]
pub struct GithubReleaseApiResponse {
    /// English: Release tag name such as `v14.1.0`.
    /// 例如 `v14.1.0` 一类的 release 标签名。
    pub tag_name: String,
    /// English: Asset list carried by the release.
    /// 当前 release 携带的资产列表。
    #[serde(default)]
    pub assets: Vec<GithubReleaseAssetRecord>,
}

/// English: Rewrite one GitHub browser download URL through the host-provided site base URL override.
/// 通过宿主提供的站点基址覆盖重写单个 GitHub 浏览器下载地址。
pub fn rewrite_github_download_url(download_url: &str, github_base_url: Option<&str>) -> String {
    let Some(base_url) = github_base_url.map(str::trim).filter(|value| !value.is_empty()) else {
        return download_url.to_string();
    };
    let normalized_base = base_url.trim_end_matches('/');
    if download_url.starts_with("https://github.com/") {
        format!(
            "{}/{}",
            normalized_base,
            download_url.trim_start_matches("https://github.com/")
        )
    } else if download_url.starts_with("http://github.com/") {
        format!(
            "{}/{}",
            normalized_base,
            download_url.trim_start_matches("http://github.com/")
        )
    } else {
        download_url.to_string()
    }
}
