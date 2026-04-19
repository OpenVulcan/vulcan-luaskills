/// 中文：当工具返回非字符串结果时，统一返回的英文错误提示。
/// English: Unified English error message returned when a tool emits a non-string result.
pub const NON_STRING_TOOL_RESULT_ERROR: &str =
    "Tool results must be returned as plain strings. Structured JSON or table results are not supported.";

/// 中文：Lua runtime 返回给宿主的稳定超限模式枚举。
/// English: Stable overflow-mode enum returned from the Lua runtime to the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolOverflowMode {
    /// 中文：超限时建议宿主按截断模式处理。
    /// English: Suggest that the host handles overflow in truncate mode.
    Truncate,
    /// 中文：超限时建议宿主按分页模式处理。
    /// English: Suggest that the host handles overflow in page mode.
    Page,
}

impl ToolOverflowMode {
    /// 中文：解析来自 Lua 的超限模式字符串。
    /// English: Parse an overflow mode string returned from Lua.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "truncate" => Some(Self::Truncate),
            "page" => Some(Self::Page),
            _ => None,
        }
    }
}

/// 中文：Lua runtime 返回给宿主的统一中间结果对象。
/// English: Unified intermediate result object returned from the Lua runtime to the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeInvocationResult {
    /// 中文：工具正文内容，必须始终为字符串。
    /// English: Tool body content, which must always be a string.
    pub content: String,
    /// 中文：可选超限模式；为空时由宿主按自身默认策略处理。
    /// English: Optional overflow mode; when absent the host applies its own default policy.
    pub overflow_mode: Option<ToolOverflowMode>,
    /// 中文：可选模板建议名，仅作为宿主层提示，不在 runtime 中直接渲染。
    /// English: Optional template hint used only as a host-side suggestion, never rendered directly by the runtime.
    pub template_hint: Option<String>,
    /// 中文：正文规范化后的字节数，供宿主判断是否需要分页、截断或压缩。
    /// English: Normalized body byte count used by the host to decide pagination, truncation, or compression.
    pub content_bytes: usize,
    /// 中文：正文规范化后的行数，供宿主判断是否命中行预算。
    /// English: Normalized body line count used by the host to decide whether the line budget is exceeded.
    pub content_lines: usize,
}

impl RuntimeInvocationResult {
    /// 中文：根据正文和可选超限提示构造统一运行时结果，并在创建时计算字节与行数。
    /// English: Build the unified runtime result from content and optional overflow hints while computing byte and line metrics at creation time.
    pub fn from_content_parts(
        content: String,
        overflow_mode: Option<ToolOverflowMode>,
        template_hint: Option<String>,
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
        }
    }

    /// 中文：构造只包含正文的字符串返回值。
    /// English: Build a content-only string result.
    pub fn plain(content: String) -> Self {
        Self::from_content_parts(content, None, None)
    }
}

/// 中文：规范化文本中的换行，统一统计字节与行数。
/// English: Normalize line endings so byte and line metrics are computed consistently.
fn normalize_text(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// 中文：按规范化后的换行拆分文本行。
/// English: Split text into lines after newline normalization.
fn split_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        Vec::new()
    } else {
        text.split('\n').collect()
    }
}

