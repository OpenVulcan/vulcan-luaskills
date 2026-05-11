use mlua::{MultiValue, Value as LuaValue};
use serde_json::{Value, json};
use std::path::Path;

use crate::runtime_options::LuaInvocationContext;
use crate::runtime_result::{
    NON_STRING_TOOL_RESULT_ERROR, RuntimeHostResult, RuntimeInvocationResult, ToolOverflowMode,
};

use super::{lua_value_to_json, lua_value_type_name};

/// Structured host-result capability snapshot derived from request context.
/// 从请求上下文导出的结构化宿主结果能力快照。
#[derive(Debug, Clone)]
pub(super) struct RuntimeHostResultCapability {
    /// Whether the host explicitly enables structured host results for the current request.
    /// 宿主是否为当前请求显式开启结构化宿主结果。
    enabled: bool,
    /// Allowed result kinds when the host wants to restrict the bridge surface.
    /// 当宿主希望限制桥接面时允许的结果类型集合。
    allowed_kinds: Vec<String>,
    /// Optional maximum payload byte size accepted by the host.
    /// 宿主接受的可选最大载荷字节数。
    max_payload_bytes: Option<usize>,
}

impl RuntimeHostResultCapability {
    /// Return whether one result kind is accepted by the current host capability snapshot.
    /// 返回当前宿主能力快照是否接受某个结果类型。
    fn allows_kind(&self, kind: &str) -> bool {
        self.allowed_kinds.is_empty() || self.allowed_kinds.iter().any(|item| item == kind)
    }
}

/// Resolve one host-result capability snapshot from one optional invocation context.
/// 从可选调用上下文中解析一份宿主结果能力快照。
pub(super) fn resolve_host_result_capability(
    invocation_context: Option<&LuaInvocationContext>,
) -> RuntimeHostResultCapability {
    let Some(request_context) =
        invocation_context.and_then(|context| context.request_context.as_ref())
    else {
        return RuntimeHostResultCapability {
            enabled: false,
            allowed_kinds: Vec::new(),
            max_payload_bytes: None,
        };
    };
    let Value::Object(capabilities) = &request_context.client_capabilities else {
        return RuntimeHostResultCapability {
            enabled: false,
            allowed_kinds: Vec::new(),
            max_payload_bytes: None,
        };
    };
    let Value::Object(host_result) = capabilities
        .get("host_result")
        .cloned()
        .unwrap_or_else(|| Value::Object(serde_json::Map::new()))
    else {
        return RuntimeHostResultCapability {
            enabled: false,
            allowed_kinds: Vec::new(),
            max_payload_bytes: None,
        };
    };
    let enabled = host_result
        .get("enabled")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let allowed_kinds = host_result
        .get("allowed_kinds")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(ToString::to_string)
                .collect::<Vec<String>>()
        })
        .unwrap_or_default();
    let max_payload_bytes = host_result
        .get("max_payload_bytes")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value > 0);
    RuntimeHostResultCapability {
        enabled,
        allowed_kinds,
        max_payload_bytes,
    }
}

/// Convert one host-result capability snapshot into one Lua helper table payload.
/// 将宿主结果能力快照转换为一份 Lua helper 表载荷。
pub(super) fn host_result_capability_to_json_value(
    capability: &RuntimeHostResultCapability,
) -> Value {
    let allowed_kinds = capability
        .allowed_kinds
        .iter()
        .cloned()
        .map(Value::String)
        .collect::<Vec<Value>>();
    json!({
        "enabled": capability.enabled,
        "allowed_kinds": allowed_kinds,
        "max_payload_bytes": capability.max_payload_bytes,
    })
}

/// Parse one optional fourth Lua return value into one structured host result.
/// 将可选的第四个 Lua 返回值解析为结构化宿主结果。
fn parse_host_result_value(
    value: Option<&LuaValue>,
    display_name: &str,
    capability: &RuntimeHostResultCapability,
) -> Result<Option<RuntimeHostResult>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    if matches!(value, LuaValue::Nil) {
        return Ok(None);
    }
    if !capability.enabled {
        return Ok(None);
    }
    let host_result_json = lua_value_to_json(value)?;
    let Value::Object(object) = host_result_json else {
        return Err(format!(
            "Lua skill '{}' must return host_result as an object with kind and payload",
            display_name
        ));
    };
    let kind = object
        .get("kind")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            format!(
                "Lua skill '{}' must return host_result.kind as one non-empty string",
                display_name
            )
        })?
        .to_string();
    if !capability.allows_kind(&kind) {
        return Err(format!(
            "Lua skill '{}' returned host_result.kind '{}' which is not allowed by the host",
            display_name, kind
        ));
    }
    let payload = object.get("payload").cloned().ok_or_else(|| {
        format!(
            "Lua skill '{}' must return host_result.payload when host_result is enabled",
            display_name
        )
    })?;
    validate_host_result_payload(display_name, &kind, &payload, capability.max_payload_bytes)?;
    Ok(Some(RuntimeHostResult { kind, payload }))
}

/// Validate one structured host-result payload against common and kind-specific rules.
/// 按通用规则与特定 kind 规则校验结构化宿主结果载荷。
fn validate_host_result_payload(
    display_name: &str,
    kind: &str,
    payload: &Value,
    max_payload_bytes: Option<usize>,
) -> Result<(), String> {
    let payload_json = serde_json::to_vec(payload).map_err(|error| {
        format!(
            "Lua skill '{}' returned one host_result payload that cannot be serialized: {}",
            display_name, error
        )
    })?;
    if let Some(limit) = max_payload_bytes {
        if payload_json.len() > limit {
            return Err(format!(
                "Lua skill '{}' returned host_result payload {} bytes larger than host limit {}",
                display_name,
                payload_json.len(),
                limit
            ));
        }
    }
    if kind == "change_set" {
        validate_change_set_payload(display_name, payload)?;
    }
    Ok(())
}

/// Validate the canonical `change_set` payload contract used by IDE-oriented edit results.
/// 校验面向 IDE 编辑结果使用的 canonical `change_set` 载荷协议。
pub(super) fn validate_change_set_payload(
    display_name: &str,
    payload: &Value,
) -> Result<(), String> {
    let Value::Object(object) = payload else {
        return Err(format!(
            "Lua skill '{}' must return change_set payload as an object",
            display_name
        ));
    };
    let mode = object.get("mode").and_then(Value::as_str).ok_or_else(|| {
        format!(
            "Lua skill '{}' change_set.mode must be a string",
            display_name
        )
    })?;
    if !matches!(mode, "preview" | "applied") {
        return Err(format!(
            "Lua skill '{}' change_set.mode must be 'preview' or 'applied'",
            display_name
        ));
    }
    if let Some(summary) = object.get("summary") {
        if !summary.is_string() && !summary.is_null() {
            return Err(format!(
                "Lua skill '{}' change_set.summary must be a string when present",
                display_name
            ));
        }
    }
    let files = object.get("files").ok_or_else(|| {
        format!(
            "Lua skill '{}' change_set.files must be present as one array",
            display_name
        )
    })?;
    let Value::Array(files) = files else {
        return Err(format!(
            "Lua skill '{}' change_set.files must be an array",
            display_name
        ));
    };
    for (index, file) in files.iter().enumerate() {
        validate_change_set_file_payload(display_name, index, file)?;
    }
    if let Some(diagnostics) = object.get("diagnostics") {
        let Value::Array(diagnostics) = diagnostics else {
            return Err(format!(
                "Lua skill '{}' change_set.diagnostics must be an array when present",
                display_name
            ));
        };
        for (index, diagnostic) in diagnostics.iter().enumerate() {
            let Value::Object(diagnostic) = diagnostic else {
                return Err(format!(
                    "Lua skill '{}' change_set.diagnostics[{}] must be an object",
                    display_name, index
                ));
            };
            let _level = diagnostic
                .get("level")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    format!(
                        "Lua skill '{}' change_set.diagnostics[{}].level must be a string",
                        display_name, index
                    )
                })?;
            let _message = diagnostic
                .get("message")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    format!(
                        "Lua skill '{}' change_set.diagnostics[{}].message must be a string",
                        display_name, index
                    )
                })?;
        }
    }
    Ok(())
}

/// Validate one file-level record inside one canonical `change_set` payload.
/// 校验 canonical `change_set` 载荷中的单个文件级记录。
fn validate_change_set_file_payload(
    display_name: &str,
    file_index: usize,
    file: &Value,
) -> Result<(), String> {
    let Value::Object(file) = file else {
        return Err(format!(
            "Lua skill '{}' change_set.files[{}] must be an object",
            display_name, file_index
        ));
    };
    let change = file.get("change").and_then(Value::as_str).ok_or_else(|| {
        format!(
            "Lua skill '{}' change_set.files[{}].change must be a string",
            display_name, file_index
        )
    })?;
    if let Some(patch) = file.get("patch") {
        if !patch.is_string() && !patch.is_null() {
            return Err(format!(
                "Lua skill '{}' change_set.files[{}].patch must be a string when present",
                display_name, file_index
            ));
        }
    }
    match change {
        "modify" => {
            let _path =
                validate_change_set_absolute_path_field(display_name, file_index, file, "path")?;
            let hunks = file.get("hunks").ok_or_else(|| {
                format!(
                    "Lua skill '{}' change_set.files[{}].hunks must be present as one non-empty array for modify changes",
                    display_name, file_index
                )
            })?;
            let Value::Array(hunks) = hunks else {
                return Err(format!(
                    "Lua skill '{}' change_set.files[{}].hunks must be a non-empty array for modify changes",
                    display_name, file_index
                ));
            };
            if hunks.is_empty() {
                return Err(format!(
                    "Lua skill '{}' change_set.files[{}].hunks must be a non-empty array for modify changes",
                    display_name, file_index
                ));
            }
            for (hunk_index, hunk) in hunks.iter().enumerate() {
                validate_change_set_modify_hunk(display_name, file_index, hunk_index, hunk)?;
            }
        }
        "create" => {
            let _path =
                validate_change_set_absolute_path_field(display_name, file_index, file, "path")?;
            validate_change_set_required_string_field(
                display_name,
                &format!("change_set.files[{}].content", file_index),
                file.get("content"),
            )?;
        }
        "delete" => {
            let _path =
                validate_change_set_absolute_path_field(display_name, file_index, file, "path")?;
            validate_change_set_required_string_field(
                display_name,
                &format!("change_set.files[{}].content", file_index),
                file.get("content"),
            )?;
        }
        "rename" => {
            let _old_path = validate_change_set_absolute_path_field(
                display_name,
                file_index,
                file,
                "old_path",
            )?;
            let _new_path = validate_change_set_absolute_path_field(
                display_name,
                file_index,
                file,
                "new_path",
            )?;
        }
        _ => {
            return Err(format!(
                "Lua skill '{}' change_set.files[{}].change is unsupported",
                display_name, file_index
            ));
        }
    }
    Ok(())
}

/// Validate one absolute path field inside one canonical `change_set` file record.
/// 校验 canonical `change_set` 文件记录中的单个绝对路径字段。
fn validate_change_set_absolute_path_field(
    display_name: &str,
    file_index: usize,
    file: &serde_json::Map<String, Value>,
    field_name: &str,
) -> Result<String, String> {
    let path = file
        .get(field_name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            format!(
                "Lua skill '{}' change_set.files[{}].{} must be one non-empty string",
                display_name, file_index, field_name
            )
        })?;
    if !Path::new(path).is_absolute() {
        return Err(format!(
            "Lua skill '{}' change_set.files[{}].{} must be absolute",
            display_name, file_index, field_name
        ));
    }
    Ok(path.to_string())
}

/// Validate one required string field inside one canonical `change_set` object path.
/// 校验 canonical `change_set` 对象路径中的单个必填字符串字段。
fn validate_change_set_required_string_field(
    display_name: &str,
    field_path: &str,
    value: Option<&Value>,
) -> Result<(), String> {
    match value {
        Some(Value::String(_)) => Ok(()),
        _ => Err(format!(
            "Lua skill '{}' {} must be a string",
            display_name, field_path
        )),
    }
}

/// Validate one modify hunk record inside one canonical `change_set` file.
/// 校验 canonical `change_set` 文件中的单个 modify hunk 记录。
fn validate_change_set_modify_hunk(
    display_name: &str,
    file_index: usize,
    hunk_index: usize,
    hunk: &Value,
) -> Result<(), String> {
    let Value::Object(hunk) = hunk else {
        return Err(format!(
            "Lua skill '{}' change_set.files[{}].hunks[{}] must be an object",
            display_name, file_index, hunk_index
        ));
    };
    validate_change_set_required_string_field(
        display_name,
        &format!(
            "change_set.files[{}].hunks[{}].before",
            file_index, hunk_index
        ),
        hunk.get("before"),
    )?;
    validate_change_set_required_string_field(
        display_name,
        &format!(
            "change_set.files[{}].hunks[{}].after",
            file_index, hunk_index
        ),
        hunk.get("after"),
    )?;
    let deleted_count = validate_change_set_hunk_line_entries(
        display_name,
        file_index,
        hunk_index,
        "delete",
        hunk.get("delete"),
    )?;
    let inserted_count = validate_change_set_hunk_line_entries(
        display_name,
        file_index,
        hunk_index,
        "insert",
        hunk.get("insert"),
    )?;
    if deleted_count == 0 && inserted_count == 0 {
        return Err(format!(
            "Lua skill '{}' change_set.files[{}].hunks[{}] must include at least one deleted or inserted line",
            display_name, file_index, hunk_index
        ));
    }
    Ok(())
}

/// Validate one ordered delete/insert line list inside one canonical `change_set` hunk.
/// 校验 canonical `change_set` hunk 中的单个有序 delete/insert 行列表。
fn validate_change_set_hunk_line_entries(
    display_name: &str,
    file_index: usize,
    hunk_index: usize,
    entry_name: &str,
    value: Option<&Value>,
) -> Result<usize, String> {
    let value = value.ok_or_else(|| {
        format!(
            "Lua skill '{}' change_set.files[{}].hunks[{}].{} must be an array",
            display_name, file_index, hunk_index, entry_name
        )
    })?;
    let Value::Array(entries) = value else {
        return Err(format!(
            "Lua skill '{}' change_set.files[{}].hunks[{}].{} must be an array",
            display_name, file_index, hunk_index, entry_name
        ));
    };
    let mut previous_line = 0_i64;
    for (entry_index, entry) in entries.iter().enumerate() {
        let Value::Object(entry) = entry else {
            return Err(format!(
                "Lua skill '{}' change_set.files[{}].hunks[{}].{}[{}] must be an object",
                display_name, file_index, hunk_index, entry_name, entry_index
            ));
        };
        let line = entry
            .get("line")
            .and_then(Value::as_i64)
            .filter(|line| *line > 0)
            .ok_or_else(|| {
                format!(
                    "Lua skill '{}' change_set.files[{}].hunks[{}].{}[{}].line must be one positive integer",
                    display_name, file_index, hunk_index, entry_name, entry_index
                )
            })?;
        if entry_index > 0 && line <= previous_line {
            return Err(format!(
                "Lua skill '{}' change_set.files[{}].hunks[{}].{} line numbers must be strictly increasing",
                display_name, file_index, hunk_index, entry_name
            ));
        }
        previous_line = line;
        validate_change_set_required_string_field(
            display_name,
            &format!(
                "change_set.files[{}].hunks[{}].{}[{}].content",
                file_index, hunk_index, entry_name, entry_index
            ),
            entry.get("content"),
        )?;
    }
    Ok(entries.len())
}

/// Parse Lua multi-return values into the host's unified string-result protocol.
/// 把 Lua 工具的多返回值解析为宿主统一字符串结果协议。
pub(super) fn parse_tool_call_output(
    values: MultiValue,
    display_name: &str,
    invocation_context: Option<&LuaInvocationContext>,
) -> Result<RuntimeInvocationResult, String> {
    let host_result_capability = resolve_host_result_capability(invocation_context);
    let values_vec: Vec<LuaValue> = values.into_vec();
    if values_vec.is_empty() {
        return Err(format!(
            "Lua skill '{}' must return a plain string result",
            display_name
        ));
    }

    if values_vec.len() > 4 {
        return Err(format!(
            "Lua skill '{}' must return content[, overflow_mode[, template_hint[, host_result]]]",
            display_name
        ));
    }

    let content = match &values_vec[0] {
        LuaValue::String(text) => text
            .to_str()
            .map_err(|error| {
                format!(
                    "Lua skill '{}' returned an invalid UTF-8 string: {}",
                    display_name, error
                )
            })?
            .to_string(),
        other => {
            return Err(format!(
                "{} (skill='{}', actual_type='{}')",
                NON_STRING_TOOL_RESULT_ERROR,
                display_name,
                lua_value_type_name(other)
            ));
        }
    };

    let overflow_mode = match values_vec.get(1) {
        None | Some(LuaValue::Nil) => None,
        Some(LuaValue::String(text)) => {
            let mode_text = text.to_str().map_err(|error| {
                format!(
                    "Lua skill '{}' returned an invalid overflow mode string: {}",
                    display_name, error
                )
            })?;
            Some(ToolOverflowMode::parse(&mode_text).ok_or_else(|| {
                format!(
                    "Lua skill '{}' returned an unsupported overflow mode: {}",
                    display_name, mode_text
                )
            })?)
        }
        Some(_) => {
            return Err(format!(
                "Lua skill '{}' must return overflow mode as a string constant",
                display_name
            ));
        }
    };

    let template_hint = match values_vec.get(2) {
        None | Some(LuaValue::Nil) => None,
        Some(LuaValue::String(text)) => {
            let name = text.to_str().map_err(|error| {
                format!(
                    "Lua skill '{}' returned an invalid template name: {}",
                    display_name, error
                )
            })?;
            let trimmed = name.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        Some(_) => {
            return Err(format!(
                "Lua skill '{}' must return template_hint as a string",
                display_name
            ));
        }
    };

    let host_result =
        parse_host_result_value(values_vec.get(3), display_name, &host_result_capability)?;

    Ok(RuntimeInvocationResult::from_content_parts(
        content,
        overflow_mode,
        template_hint,
        host_result,
    ))
}
