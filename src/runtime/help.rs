use serde::Serialize;

/// One structured help node descriptor returned to host-side system tool wrappers.
/// 返回给宿主侧 system tool 包装层的单个结构化帮助节点描述。
#[derive(Debug, Clone, Serialize)]
pub struct RuntimeHelpNodeDescriptor {
    /// Stable node name. The main node always uses `main`.
    /// 稳定节点名称，主节点固定使用 `main`。
    pub flow_name: String,
    /// Human-readable short description of the current help node.
    /// 当前帮助节点的人类可读简要说明。
    pub description: String,
    /// Canonical runtime entries related to the current help node.
    /// 与当前帮助节点关联的 canonical 运行时入口列表。
    pub related_entries: Vec<String>,
    /// Whether the current node is the main help node of one skill.
    /// 当前节点是否为某个 skill 的主帮助节点。
    pub is_main: bool,
}

/// Structured help tree summary of one skill returned by the runtime core.
/// 由运行时核心返回的单个 skill 结构化帮助树摘要。
#[derive(Debug, Clone, Serialize)]
pub struct RuntimeSkillHelpDescriptor {
    /// Stable skill identifier of the current help tree.
    /// 当前帮助树所属的稳定 skill 标识符。
    pub skill_id: String,
    /// Human-readable internal skill name.
    /// 人类可读的内部 skill 名称。
    pub skill_name: String,
    /// Semantic package version declared by the current skill.
    /// 当前技能声明的语义化包版本。
    pub skill_version: String,
    /// Named skill root that currently owns the effective help tree.
    /// 当前生效帮助树所属的命名技能根。
    pub root_name: String,
    /// Physical skill directory that currently owns the effective help tree.
    /// 当前生效帮助树所属的物理技能目录。
    pub skill_dir: String,
    /// Main help node summary.
    /// 主帮助节点摘要。
    pub main: RuntimeHelpNodeDescriptor,
    /// Topic/workflow help node summaries declared under the current skill.
    /// 当前 skill 声明的 topic/workflow 子帮助节点摘要。
    pub flows: Vec<RuntimeHelpNodeDescriptor>,
}

/// Structured help detail payload returned by the runtime core for one node.
/// 运行时核心为单个帮助节点返回的结构化详情载荷。
#[derive(Debug, Clone, Serialize)]
pub struct RuntimeHelpDetail {
    /// Stable skill identifier that owns the current help node.
    /// 拥有当前帮助节点的稳定 skill 标识符。
    pub skill_id: String,
    /// Human-readable internal skill name.
    /// 人类可读的内部 skill 名称。
    pub skill_name: String,
    /// Semantic package version declared by the current skill.
    /// 当前技能声明的语义化包版本。
    pub skill_version: String,
    /// Named skill root that currently owns the resolved help node.
    /// 当前解析出的帮助节点所属的命名技能根。
    pub root_name: String,
    /// Physical skill directory that currently owns the resolved help node.
    /// 当前解析出的帮助节点所属的物理技能目录。
    pub skill_dir: String,
    /// Stable flow name. The main node always uses `main`.
    /// 稳定流程名称，主节点固定使用 `main`。
    pub flow_name: String,
    /// Human-readable short description of the current help node.
    /// 当前帮助节点的人类可读简要说明。
    pub description: String,
    /// Canonical runtime entries related to the current help node.
    /// 与当前帮助节点关联的 canonical 运行时入口列表。
    pub related_entries: Vec<String>,
    /// Whether the current node is the main help node of one skill.
    /// 当前节点是否为某个 skill 的主帮助节点。
    pub is_main: bool,
    /// Structured content type label of the rendered payload.
    /// 渲染后载荷的结构化内容类型标签。
    pub content_type: String,
    /// Final rendered help content returned by the runtime core.
    /// 运行时核心返回的最终帮助内容。
    pub content: String,
}
