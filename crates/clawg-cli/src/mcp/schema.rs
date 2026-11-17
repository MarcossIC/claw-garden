//! The tool schemas as the model reads them.
//!
//! rmcp generates every input schema with schemars' 2020-12 settings, and a
//! part of what that writes tells the model nothing it acts on. A loaded tool
//! travels in every request from then on, so that part is paid again on each.

use serde_json::{Map, Value};

/// Removes from an input schema what the generator writes and the model gains
/// nothing from: the dialect URI, `"default": null`, `null` as a second type,
/// and the numeric formats, with the `minimum: 0` an unsigned one implies.
///
/// Every keyword left means the same in draft-07 and in 2020-12, so the schema
/// needs no dialect. An optional argument stays `Option<T>`, so an explicit
/// `null` from a client still reads as absent.
pub fn strip_noise(schema: &mut Map<String, Value>) {
    schema.remove("$schema");
    strip_node(schema);
}

fn strip_node(node: &mut Map<String, Value>) {
    if node.get("default").is_some_and(Value::is_null) {
        node.remove("default");
    }
    if let Some(Value::Array(types)) = node.get_mut("type") {
        types.retain(|t| t != "null");
        if let [only] = types.as_slice() {
            let only = only.clone();
            node.insert("type".into(), only);
        }
    }
    let numeric = node.get("type").and_then(Value::as_str).is_some_and(|t| t == "integer" || t == "number");
    if numeric {
        let unsigned = node.remove("format").and_then(|f| f.as_str().map(|f| f.starts_with("uint"))).unwrap_or(false);
        if unsigned && node.get("minimum").and_then(Value::as_u64) == Some(0) {
            node.remove("minimum");
        }
    }
    if let Some(Value::Object(properties)) = node.get_mut("properties") {
        for property in properties.values_mut() {
            if let Value::Object(property) = property {
                strip_node(property);
            }
        }
    }
    if let Some(Value::Object(items)) = node.get_mut("items") {
        strip_node(items);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn stripped(schema: Value) -> Value {
        let Value::Object(mut map) = schema else { panic!("a schema is an object") };
        strip_noise(&mut map);
        Value::Object(map)
    }

    #[test]
    fn strip_noise_drops_what_the_generator_adds() {
        let generated = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {
                "query": { "description": "What to look for.", "type": "string" },
                "limit": { "default": null, "description": "How many.", "format": "uint", "minimum": 0, "type": ["integer", "null"] },
                "id": { "description": "The id.", "format": "int64", "type": "integer" },
                "importance": { "default": null, "format": "double", "type": ["number", "null"] },
                "derived_from": { "default": null, "items": { "format": "int64", "type": "integer" }, "type": ["array", "null"] },
                "memory_type": { "default": null, "enum": ["semantic_fact", "user_preference"], "type": ["string", "null"] }
            },
            "required": ["query"]
        });
        let expected = json!({
            "type": "object",
            "properties": {
                "query": { "description": "What to look for.", "type": "string" },
                "limit": { "description": "How many.", "type": "integer" },
                "id": { "description": "The id.", "type": "integer" },
                "importance": { "type": "number" },
                "derived_from": { "items": { "type": "integer" }, "type": "array" },
                "memory_type": { "enum": ["semantic_fact", "user_preference"], "type": "string" }
            },
            "required": ["query"]
        });
        assert_eq!(stripped(generated), expected);
    }

    /// A default other than `null`, a bound other than the one an unsigned
    /// type implies, or a string format all tell the model something.
    #[test]
    fn strip_noise_keeps_what_the_model_reads() {
        let schema = json!({
            "type": "object",
            "properties": {
                "user_answered": { "default": false, "description": "True only when the user said so.", "type": "boolean" },
                "budget": { "format": "int32", "minimum": 1, "type": "integer" },
                "page": { "format": "uint32", "minimum": 1, "type": "integer" },
                "link": { "format": "uri", "type": "string" },
                "either": { "type": ["string", "integer"] }
            }
        });
        let expected = json!({
            "type": "object",
            "properties": {
                "user_answered": { "default": false, "description": "True only when the user said so.", "type": "boolean" },
                "budget": { "minimum": 1, "type": "integer" },
                "page": { "minimum": 1, "type": "integer" },
                "link": { "format": "uri", "type": "string" },
                "either": { "type": ["string", "integer"] }
            }
        });
        assert_eq!(stripped(schema), expected);
    }

    #[test]
    fn strip_noise_leaves_a_schema_with_no_properties_alone() {
        let schema = json!({ "properties": {}, "type": "object" });
        assert_eq!(stripped(schema.clone()), schema);
    }
}
