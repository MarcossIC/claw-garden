//! Input validation for native tools.
//!
//! Validates JSON input against the tool's declared JSON Schema. Scope is
//! deliberately narrow — we cover the schema subset we actually use in
//! `builtin_tools()`:
//!
//!   - required fields
//!   - top-level `type: "object"` expectation
//!   - per-property `type` checks (string, integer, number, boolean, array, object)
//!   - `enum` membership
//!
//! Going beyond this subset (patterns, formats, `$ref`, composition keywords)
//! would mean pulling in a full JSON Schema crate. For the tool input shapes
//! we actually declare, this covers the real bugs the model introduces
//! (wrong type, missing field, unknown enum value) without a new dependency.

use serde_json::Value;
use super::errors::ToolErrorMessage;

/// Validate `input` against `schema`. Returns an `InvalidInput` error with a
/// model-actionable message on the first failure.
///
/// Fails fast — the error names the exact field and what was expected so the
/// model can fix its next tool call instead of looping on the same mistake.
pub fn validate_input(
    tool_name: &str,
    schema: &Value,
    input: &Value,
) -> Result<(), ToolErrorMessage> {
    let obj = input.as_object().ok_or_else(|| {
        ToolErrorMessage::invalid_input(tool_name, "input must be a JSON object")
    })?;

    if let Some(required) = schema.get("required").and_then(|r| r.as_array()) {
        let mut missing = Vec::new();
        for req in required {
            if let Some(name) = req.as_str() {
                if !obj.contains_key(name) || obj[name].is_null() {
                    missing.push(name);
                }
            }
        }
        if !missing.is_empty() {
            return Err(ToolErrorMessage::invalid_input(
                tool_name,
                &format!("missing required field(s): {}", missing.join(", ")),
            ));
        }
    }

    if let Some(props) = schema.get("properties").and_then(|p| p.as_object()) {
        for (field_name, field_schema) in props {
            let Some(field_value) = obj.get(field_name) else {
                continue; // Optional field absent — already covered by `required` above.
            };
            // Treat explicit null as "field omitted" for optional fields —
            // the model frequently sends null to mean "use default".
            if field_value.is_null() {
                continue;
            }

            if let Some(expected_type) = field_schema.get("type").and_then(|t| t.as_str()) {
                if !matches_type(expected_type, field_value) {
                    return Err(ToolErrorMessage::invalid_input(
                        tool_name,
                        &format!(
                            "field '{field_name}' must be {expected_type}, got {}",
                            type_name_of(field_value)
                        ),
                    ));
                }
            }

            if let Some(allowed) = field_schema.get("enum").and_then(|e| e.as_array()) {
                if !allowed.iter().any(|a| a == field_value) {
                    let allowed_str: Vec<String> =
                        allowed.iter().map(|v| v.to_string()).collect();
                    return Err(ToolErrorMessage::invalid_input(
                        tool_name,
                        &format!(
                            "field '{field_name}' must be one of [{}], got {}",
                            allowed_str.join(", "),
                            field_value
                        ),
                    ));
                }
            }
        }
    }

    Ok(())
}

/// True if `value` matches the JSON Schema `type` keyword.
///
/// Note: per JSON Schema, `"integer"` accepts any number with no fractional part
/// (including floats like 1.0). We follow that rule via `.is_i64() || is_u64()`
/// plus the `.as_f64().map(|f| f.fract() == 0.0)` fallback.
fn matches_type(expected: &str, value: &Value) -> bool {
    match expected {
        "string" => value.is_string(),
        "integer" => {
            value.is_i64()
                || value.is_u64()
                || value.as_f64().map_or(false, |f| f.is_finite() && f.fract() == 0.0)
        }
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        "null" => value.is_null(),
        _ => true, // Unknown type keyword — skip (don't reject).
    }
}

fn type_name_of(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(n) if n.is_i64() || n.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn passes_when_all_required_present() {
        let schema = json!({"required": ["path"]});
        let input = json!({"path": "src/main.rs"});
        assert!(validate_input("read_file", &schema, &input).is_ok());
    }

    #[test]
    fn fails_when_required_missing() {
        let schema = json!({"required": ["path"]});
        let input = json!({"offset": 10});
        let err = validate_input("read_file", &schema, &input).unwrap_err();
        assert!(err.detail.contains("path"));
    }

    #[test]
    fn passes_when_no_required() {
        let schema = json!({"type": "object"});
        let input = json!({});
        assert!(validate_input("list_directory", &schema, &input).is_ok());
    }

    #[test]
    fn rejects_non_object_input() {
        let schema = json!({"type": "object"});
        let err = validate_input("tool", &schema, &json!("not an object")).unwrap_err();
        assert!(err.detail.contains("JSON object"));
    }

    #[test]
    fn rejects_wrong_type_for_string_field() {
        let schema = json!({
            "properties": { "path": { "type": "string" } },
            "required": ["path"]
        });
        let err = validate_input("read_file", &schema, &json!({"path": 42})).unwrap_err();
        assert!(err.detail.contains("path"));
        assert!(err.detail.contains("string"));
        assert!(err.detail.contains("integer"));
    }

    #[test]
    fn accepts_integer_passed_as_whole_number() {
        // JSON Schema: integer accepts 1.0, because its fractional part is zero.
        let schema = json!({
            "properties": { "offset": { "type": "integer" } }
        });
        assert!(validate_input("read_file", &schema, &json!({"offset": 1.0})).is_ok());
        assert!(validate_input("read_file", &schema, &json!({"offset": 10})).is_ok());
    }

    #[test]
    fn rejects_non_integer_float() {
        let schema = json!({
            "properties": { "offset": { "type": "integer" } }
        });
        let err = validate_input("read_file", &schema, &json!({"offset": 1.5})).unwrap_err();
        assert!(err.detail.contains("offset"));
    }

    #[test]
    fn rejects_unknown_enum_value() {
        let schema = json!({
            "properties": {
                "level": { "type": "string", "enum": ["debug", "info", "error"] }
            }
        });
        let err = validate_input("log", &schema, &json!({"level": "trace"})).unwrap_err();
        assert!(err.detail.contains("level"));
        assert!(err.detail.contains("one of"));
    }

    #[test]
    fn accepts_valid_enum_value() {
        let schema = json!({
            "properties": {
                "level": { "type": "string", "enum": ["debug", "info", "error"] }
            }
        });
        assert!(validate_input("log", &schema, &json!({"level": "info"})).is_ok());
    }

    #[test]
    fn null_value_treated_as_absent_for_optional_field() {
        // The model often sends null to mean "omit" — allow it for optional fields.
        let schema = json!({
            "properties": { "offset": { "type": "integer" } }
        });
        assert!(validate_input("read_file", &schema, &json!({"offset": null})).is_ok());
    }

    #[test]
    fn null_still_rejected_for_required_field() {
        // But null in a required field still fails — the field is effectively missing.
        let schema = json!({
            "properties": { "path": { "type": "string" } },
            "required": ["path"]
        });
        let err = validate_input("read_file", &schema, &json!({"path": null})).unwrap_err();
        assert!(err.detail.contains("path"));
    }
}
