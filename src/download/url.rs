use serde::{Deserialize, Serialize};

/// Direct URL download descriptor used by LuaSkills dependency downloads.
/// LuaSkills 依赖下载使用的直接 URL 描述对象。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UrlDownloadSource {
    pub url: String,
}
