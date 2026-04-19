use serde::{Deserialize, Serialize};

/// English: Skill-list download descriptor that resolves one dependency through an indexed package list.
/// 通过索引包列表解析单个依赖的 skilllist 下载描述对象。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillListDownloadSource {
    pub list_url: String,
    pub package_name: String,
}
