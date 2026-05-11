use super::parse_single_sql_params;
use serde_json::json;

/// Verify legacy params_json payloads are rejected in favor of formal params arrays.
/// 验证旧版 params_json 载荷会被拒绝，并要求改用正式 params 数组。
#[test]
fn parse_single_sql_params_rejects_legacy_params_json() {
    let error = match parse_single_sql_params(&json!({
        "params_json": "[1, 2, 3]"
    })) {
        Ok(_) => panic!("legacy params_json input should be rejected"),
        Err(error) => error,
    };
    assert_eq!(error, "params_json is no longer supported; use params");
}
