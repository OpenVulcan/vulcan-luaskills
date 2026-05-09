use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Unified English error message returned when a tool emits a non-string result.
/// 当工具返回非字符串结果时，统一返回的英文错误提示。
pub const NON_STRING_TOOL_RESULT_ERROR: &str = "Tool results must be returned as plain strings. Structured JSON or table results are not supported.";

/// Stable overflow-mode enum returned from the Lua runtime to the host.
/// Lua runtime 返回给宿主的稳定超限模式枚举。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolOverflowMode {
    /// Suggest that the host handles overflow in truncate mode.
    /// 超限时建议宿主按截断模式处理。
    Truncate,
    /// Suggest that the host handles overflow in page mode.
    /// 超限时建议宿主按分页模式处理。
    Page,
}

impl ToolOverflowMode {
    /// Parse an overflow mode string returned from Lua.
    /// 解析来自 Lua 的超限模式字符串。
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "truncate" => Some(Self::Truncate),
            "page" => Some(Self::Page),
            _ => None,
        }
    }
}

/// Unified intermediate result object returned from the Lua runtime to the host.
/// Lua runtime 返回给宿主的统一中间结果对象。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeInvocationResult {
    /// Tool body content, which must always be a string.
    /// 工具正文内容，必须始终为字符串。
    pub content: String,
    /// Optional overflow mode; when absent the host applies its own default policy.
    /// 可选超限模式；为空时由宿主按自身默认策略处理。
    pub overflow_mode: Option<ToolOverflowMode>,
    /// Optional template hint used only as a host-side suggestion, never rendered directly by the runtime.
    /// 可选模板建议名，仅作为宿主层提示，不在 runtime 中直接渲染。
    pub template_hint: Option<String>,
    /// Normalized body byte count used by the host to decide pagination, truncation, or compression.
    /// 正文规范化后的字节数，供宿主判断是否需要分页、截断或压缩。
    pub content_bytes: usize,
    /// Normalized body line count used by the host to decide whether the line budget is exceeded.
    /// 正文规范化后的行数，供宿主判断是否命中行预算。
    pub content_lines: usize,
    /// Optional structured host-side result returned as the fourth Lua value when the host explicitly enables the bridge.
    /// 当宿主显式开启桥接后，作为 Lua 第四返回值传回的可选宿主结构化结果。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_result: Option<RuntimeHostResult>,
}

/// Structured host-side result emitted by one Lua tool call when the host bridge is enabled.
/// 在宿主桥接开启时由单次 Lua 工具调用发出的结构化宿主结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeHostResult {
    /// Stable host-consumed result kind such as `change_set` or one host-private contract name.
    /// 供宿主消费的稳定结果类型，例如 `change_set` 或宿主私有协议名称。
    pub kind: String,
    /// JSON-compatible structured payload consumed by the host as one independent signal source.
    /// 由宿主作为独立信号源消费的 JSON 兼容结构化载荷。
    pub payload: Value,
}

impl RuntimeInvocationResult {
    /// Build the unified runtime result from content and optional overflow hints while computing byte and line metrics at creation time.
    /// 根据正文和可选超限提示构造统一运行时结果，并在创建时计算字节与行数。
    pub fn from_content_parts(
        content: String,
        overflow_mode: Option<ToolOverflowMode>,
        template_hint: Option<String>,
        host_result: Option<RuntimeHostResult>,
    ) -> Self {
        let normalized = normalize_text(&content);
        let content_bytes = normalized.len();
        let content_lines = split_lines(&normalized).len();
        Self {
            content,
            overflow_mode,
            template_hint,
            content_bytes,
            content_lines,
            host_result,
        }
    }

    /// Build a content-only string result.
    /// 构造只包含正文的字符串返回值。
    pub fn plain(content: String) -> Self {
        Self::from_content_parts(content, None, None, None)
    }
}

/// Normalize line endings so byte and line metrics are computed consistently.
/// 规范化文本中的换行，统一统计字节与行数。
fn normalize_text(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// Split text into lines after newline normalization.
/// 按规范化后的换行拆分文本行。
fn split_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        Vec::new()
    } else {
        text.split('\n').collect()
    }
}
