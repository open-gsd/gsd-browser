//! Moonshot/Kimi-compatible JSON Schema sanitizer for MCP tool inputSchema output.
//!
//! Moonshot rejects tools.function.parameters schemas that use anyOf/oneOf/allOf
//! with parent-level `type`. Ported from gsd-pi's `sanitizeSchemaForMoonshot`.

use serde_json::{json, Map, Value};
use std::collections::HashSet;

const FORBIDDEN_UNION_KEYS: [&str; 3] = ["anyOf", "oneOf", "allOf"];

fn is_record(value: &Value) -> bool {
    value.is_object()
}

fn infer_json_schema_type(value: &Value) -> &'static str {
    match value {
        Value::Number(n) if n.as_i64().is_some() || n.as_u64().is_some() => "integer",
        Value::Number(_) => "number",
        Value::Bool(_) => "boolean",
        _ => "string",
    }
}

fn is_const_only_schema(schema: &Value) -> bool {
    schema
        .as_object()
        .is_some_and(|obj| obj.contains_key("const"))
}

fn is_unsupported_schema_key(key: &str) -> bool {
    matches!(
        key,
        "$schema"
            | "$id"
            | "$anchor"
            | "$dynamicAnchor"
            | "$vocabulary"
            | "$comment"
            | "$defs"
            | "definitions"
            | "unevaluatedProperties"
            | "$ref"
            | "nullable"
            | "examples"
            | "example"
            | "readOnly"
            | "writeOnly"
    )
}

fn merge_property_schemas(a: Option<&Value>, b: Option<&Value>) -> Value {
    match (a, b) {
        (None, None) => Value::Null,
        (None, Some(b)) => b.clone(),
        (Some(a), None) => a.clone(),
        (Some(a), Some(b)) => {
            let (Some(left), Some(right)) = (a.as_object(), b.as_object()) else {
                return b.clone();
            };

            if left.contains_key("const") && right.contains_key("const") {
                let mut enum_values = vec![left["const"].clone(), right["const"].clone()];
                enum_values.sort_by(|x, y| {
                    serde_json::to_string(x)
                        .unwrap_or_default()
                        .cmp(&serde_json::to_string(y).unwrap_or_default())
                });
                enum_values.dedup();

                let mut merged = left.clone();
                merged.extend(right.clone());
                merged.remove("const");
                merged.insert("enum".to_string(), Value::Array(enum_values.clone()));
                if enum_values.iter().all(|v| v.is_string()) {
                    merged.insert("type".to_string(), json!("string"));
                } else {
                    merged.insert(
                        "type".to_string(),
                        right
                            .get("type")
                            .or_else(|| left.get("type"))
                            .cloned()
                            .unwrap_or(json!("string")),
                    );
                }
                return Value::Object(merged);
            }

            if left.contains_key("enum") || right.contains_key("enum") {
                let mut enum_values: Vec<Value> = left
                    .get("enum")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                enum_values.extend(
                    right
                        .get("enum")
                        .and_then(|v| v.as_array())
                        .cloned()
                        .unwrap_or_default(),
                );
                enum_values.sort_by(|x, y| {
                    serde_json::to_string(x)
                        .unwrap_or_default()
                        .cmp(&serde_json::to_string(y).unwrap_or_default())
                });
                enum_values.dedup();

                let mut merged = left.clone();
                merged.extend(right.clone());
                merged.insert("enum".to_string(), Value::Array(enum_values));
                return Value::Object(merged);
            }

            let mut merged = left.clone();
            merged.extend(right.clone());
            Value::Object(merged)
        }
    }
}

fn collapse_const_union(obj: &Map<String, Value>, union_key: &str) -> Map<String, Value> {
    let variants = obj
        .get(union_key)
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let enum_values: Vec<Value> = variants
        .iter()
        .filter_map(|variant| variant.get("const").cloned())
        .collect();
    let enum_types: HashSet<&str> = enum_values.iter().map(infer_json_schema_type).collect();

    let mut rest = obj.clone();
    rest.remove(union_key);
    rest.insert(
        "type".to_string(),
        json!(if enum_types.len() == 1 {
            enum_types.into_iter().next().unwrap_or("string")
        } else {
            "string"
        }),
    );
    rest.insert("enum".to_string(), Value::Array(enum_values));
    rest
}

fn normalize_additional_properties(value: &Value) -> Option<Value> {
    if value == &json!(false) {
        return None;
    }
    if value.as_object().is_some_and(|obj| obj.is_empty()) {
        return Some(json!(true));
    }
    Some(value.clone())
}

fn convert_pattern_properties_to_additional_properties(
    obj: &Map<String, Value>,
) -> Map<String, Value> {
    if !obj.contains_key("patternProperties") {
        return obj.clone();
    }

    let mut rest = obj.clone();
    let pattern_properties = rest.remove("patternProperties");
    if rest.contains_key("additionalProperties") {
        return rest;
    }

    let Some(pattern_properties) = pattern_properties.and_then(|v| v.as_object().cloned()) else {
        return rest;
    };

    let value_schemas: Vec<Value> = pattern_properties.values().cloned().collect();
    match value_schemas.len() {
        1 => {
            rest.insert("additionalProperties".to_string(), value_schemas[0].clone());
        }
        n if n > 1 => {
            rest.insert("additionalProperties".to_string(), json!(true));
        }
        _ => {}
    }
    rest
}

/// Normalize root-level object unions before deep sanitization.
pub fn normalize_claude_tool_schema_for_google(schema: &Value) -> Value {
    let Some(json_schema) = schema.as_object() else {
        return json!({
            "type": "object",
            "properties": {},
            "required": []
        });
    };

    let variants = json_schema
        .get("anyOf")
        .or_else(|| json_schema.get("oneOf"))
        .and_then(|v| v.as_array());

    let object_variants: Vec<&Map<String, Value>> = variants
        .map(|variants| {
            variants
                .iter()
                .filter_map(|candidate| {
                    candidate.as_object().and_then(|obj| {
                        if obj.get("type") == Some(&json!("object"))
                            && obj.get("properties").and_then(|p| p.as_object()).is_some()
                        {
                            Some(obj)
                        } else {
                            None
                        }
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    if object_variants.is_empty() {
        let mut without_unions = json_schema.clone();
        without_unions.remove("anyOf");
        without_unions.remove("oneOf");
        without_unions.remove("allOf");

        let properties = without_unions
            .get("properties")
            .and_then(|p| p.as_object())
            .cloned()
            .unwrap_or_default();
        let required: Vec<Value> = without_unions
            .get("required")
            .and_then(|r| r.as_array())
            .map(|arr| arr.iter().filter(|key| key.is_string()).cloned().collect())
            .unwrap_or_default();

        without_unions.insert("type".to_string(), json!("object"));
        without_unions.insert("properties".to_string(), Value::Object(properties));
        without_unions.insert("required".to_string(), Value::Array(required));
        return Value::Object(without_unions);
    }

    let mut properties = Map::new();
    for candidate in &object_variants {
        if let Some(candidate_props) = candidate.get("properties").and_then(|p| p.as_object()) {
            for (key, value) in candidate_props {
                let merged = merge_property_schemas(properties.get(key), Some(value));
                properties.insert(key.clone(), merged);
            }
        }
    }

    let mut required_seen: HashSet<String> = HashSet::new();
    let mut required_order: Vec<String> = Vec::new();
    for candidate in &object_variants {
        if let Some(req) = candidate.get("required").and_then(|r| r.as_array()) {
            for key in req {
                if let Some(key) = key.as_str() {
                    if required_seen.insert(key.to_string()) {
                        required_order.push(key.to_string());
                    }
                }
            }
        }
    }

    let mut normalized = Map::new();
    normalized.insert("type".to_string(), json!("object"));
    normalized.insert("properties".to_string(), Value::Object(properties));
    if !required_order.is_empty() {
        normalized.insert(
            "required".to_string(),
            Value::Array(required_order.into_iter().map(Value::String).collect()),
        );
    }
    Value::Object(normalized)
}

fn union_key(obj: &Map<String, Value>) -> Option<&'static str> {
    if obj.get("oneOf").and_then(|v| v.as_array()).is_some() {
        Some("oneOf")
    } else if obj.get("anyOf").and_then(|v| v.as_array()).is_some() {
        Some("anyOf")
    } else if obj.get("allOf").and_then(|v| v.as_array()).is_some() {
        Some("allOf")
    } else {
        None
    }
}

fn simplify_non_const_union(obj: &Map<String, Value>, union_key_name: &str) -> Map<String, Value> {
    let variants: Vec<Value> = obj
        .get(union_key_name)
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(sanitize_for_claude_input_schema_deep)
                .collect()
        })
        .unwrap_or_default();

    let description = obj
        .get("description")
        .and_then(|d| d.as_str())
        .map(str::to_string);
    let mut rest = obj.clone();
    rest.remove(union_key_name);
    rest.remove("type");
    rest.remove("description");

    let object_variants: Vec<&Map<String, Value>> = variants
        .iter()
        .filter_map(|variant| {
            variant
                .as_object()
                .filter(|obj| obj.get("type") == Some(&json!("object")))
        })
        .collect();

    if !object_variants.is_empty() && object_variants.len() == variants.len() {
        let mut properties = Map::new();
        for candidate in &object_variants {
            if let Some(candidate_props) = candidate.get("properties").and_then(|p| p.as_object()) {
                for (key, value) in candidate_props {
                    let merged = merge_property_schemas(properties.get(key), Some(value));
                    properties.insert(key.clone(), merged);
                }
            }
        }

        let merged = sanitize_for_claude_input_schema_deep(&json!({
            "type": "object",
            "properties": properties,
        }));
        let mut result = merged
            .as_object()
            .cloned()
            .unwrap_or_else(|| Map::from_iter([("type".to_string(), json!("object"))]));
        result.extend(rest);
        if let Some(desc) = description {
            result.insert("description".to_string(), json!(desc));
        }
        return result;
    }

    let array_variant = variants.iter().find(|variant| {
        variant
            .as_object()
            .is_some_and(|obj| obj.get("type") == Some(&json!("array")))
    });
    let string_variant = variants.iter().find(|variant| {
        variant
            .as_object()
            .is_some_and(|obj| obj.get("type") == Some(&json!("string")))
    });

    if array_variant.is_some() && string_variant.is_some() && variants.len() == 2 {
        let mut result = array_variant
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();
        result.extend(rest);
        if let Some(desc) = description {
            result.insert("description".to_string(), json!(desc));
        }
        return result;
    }

    if let (Some(object_variant), Some(_)) = (object_variants.first(), string_variant) {
        let mut result = (*object_variant).clone();
        result.extend(rest);
        let desc = description
            .map(|desc| format!("{desc}. A plain string fallback is also accepted."))
            .unwrap_or_else(|| {
                "Structured object preferred; a plain string fallback is also accepted.".to_string()
            });
        result.insert("description".to_string(), json!(desc));
        return result;
    }

    if let Some(first_variant) = variants.iter().find_map(|v| v.as_object()) {
        let mut result = first_variant.clone();
        result.extend(rest);
        if let Some(desc) = description {
            result.insert("description".to_string(), json!(desc));
        }
        return result;
    }

    let mut result = Map::new();
    result.insert("type".to_string(), json!("string"));
    result.extend(rest);
    if let Some(desc) = description {
        result.insert("description".to_string(), json!(desc));
    }
    result
}

fn sanitize_for_claude_input_schema_deep(schema: &Value) -> Value {
    if let Some(arr) = schema.as_array() {
        return Value::Array(
            arr.iter()
                .map(sanitize_for_claude_input_schema_deep)
                .collect(),
        );
    }

    let Some(obj) = schema.as_object() else {
        return schema.clone();
    };

    let obj = convert_pattern_properties_to_additional_properties(obj);

    if let Some(union_key_name) = union_key(&obj) {
        let variants = obj
            .get(union_key_name)
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        if !variants.is_empty() && variants.iter().all(is_const_only_schema) {
            let collapsed_union_key = if union_key_name == "allOf" {
                "anyOf"
            } else {
                union_key_name
            };
            let mut with_union = obj.clone();
            with_union.remove(union_key_name);
            with_union.remove("type");
            with_union.insert(collapsed_union_key.to_string(), Value::Array(variants));
            let collapsed = collapse_const_union(&with_union, collapsed_union_key);
            return sanitize_for_claude_input_schema_deep(&Value::Object(collapsed));
        }

        let simplified = simplify_non_const_union(&obj, union_key_name);
        return sanitize_for_claude_input_schema_deep(&Value::Object(simplified));
    }

    if obj.contains_key("const") {
        let mut next = obj.clone();
        let const_value = next.remove("const").unwrap_or(Value::Null);
        if !next.contains_key("enum") {
            next.insert("enum".to_string(), json!([const_value]));
        }
        if !next.contains_key("type") {
            next.insert(
                "type".to_string(),
                json!(infer_json_schema_type(&const_value)),
            );
        }
        return sanitize_for_claude_input_schema_deep(&Value::Object(next));
    }

    let mut result = Map::new();
    for (key, value) in &obj {
        if is_unsupported_schema_key(key) {
            continue;
        }
        if key == "additionalProperties" {
            if let Some(normalized) = normalize_additional_properties(value) {
                result.insert(
                    key.clone(),
                    sanitize_for_claude_input_schema_deep(&normalized),
                );
            }
            continue;
        }
        if key == "required" {
            if value.as_array().is_some_and(|arr| arr.is_empty()) {
                continue;
            }
        }
        result.insert(key.clone(), sanitize_for_claude_input_schema_deep(value));
    }
    Value::Object(result)
}

fn to_claude_input_schema_root(schema: &Value) -> Value {
    let sanitized =
        sanitize_for_claude_input_schema_deep(&normalize_claude_tool_schema_for_google(schema));
    let sanitized_obj = sanitized.as_object().cloned().unwrap_or_default();

    let mut root = Map::new();
    root.insert("type".to_string(), json!("object"));
    root.insert(
        "properties".to_string(),
        sanitized_obj
            .get("properties")
            .cloned()
            .unwrap_or_else(|| json!({})),
    );
    if let Some(required) = sanitized_obj.get("required").and_then(|r| r.as_array()) {
        let filtered: Vec<Value> = required
            .iter()
            .filter(|key| key.is_string())
            .cloned()
            .collect();
        if !filtered.is_empty() {
            root.insert("required".to_string(), Value::Array(filtered));
        }
    }
    Value::Object(root)
}

/// Sanitize a JSON Schema for Moonshot/Kimi tool parameter compatibility.
pub fn sanitize_schema_for_moonshot(schema: &Value) -> Value {
    to_claude_input_schema_root(schema)
}

/// Test helper — returns paths of forbidden union keywords in a schema tree.
pub fn collect_forbidden_union_schema_paths(value: &Value, path: &str) -> Vec<String> {
    if value.is_null() || !value.is_object() && !value.is_array() {
        return Vec::new();
    }

    if let Some(arr) = value.as_array() {
        return arr
            .iter()
            .enumerate()
            .flat_map(|(index, item)| {
                collect_forbidden_union_schema_paths(item, &format!("{path}[{index}]"))
            })
            .collect();
    }

    let mut violations = Vec::new();
    if let Some(obj) = value.as_object() {
        for (key, nested) in obj {
            if FORBIDDEN_UNION_KEYS.contains(&key.as_str()) {
                violations.push(format!("{path}.{key}"));
            }
            violations.extend(collect_forbidden_union_schema_paths(
                nested,
                &format!("{path}.{key}"),
            ));
        }
    }
    violations
}

/// Apply Moonshot-safe inputSchema to every tool in an MCP tools/list payload.
pub fn sanitize_tool_list_for_moonshot(tools: Vec<Value>) -> Vec<Value> {
    tools
        .into_iter()
        .map(|mut tool| {
            if let Some(obj) = tool.as_object_mut() {
                if let Some(input_schema) = obj.get("inputSchema") {
                    let sanitized = sanitize_schema_for_moonshot(input_schema);
                    obj.insert("inputSchema".to_string(), sanitized);
                }
                if let Some(output_schema) = obj.get("outputSchema") {
                    let sanitized = sanitize_schema_for_moonshot(output_schema);
                    obj.insert("outputSchema".to_string(), sanitized);
                }
            }
            tool
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flattens_root_any_of_object_unions_to_single_object_schema() {
        let schema = json!({
            "anyOf": [
                {
                    "type": "object",
                    "properties": { "kind": { "const": "milestone" }, "content": { "type": "string" } },
                    "required": ["kind", "content"]
                },
                {
                    "type": "object",
                    "properties": { "kind": { "const": "project" }, "content": { "type": "string" } },
                    "required": ["kind", "content"]
                }
            ]
        });

        let sanitized = sanitize_schema_for_moonshot(&schema);
        assert_eq!(
            sanitized,
            json!({
                "type": "object",
                "properties": {
                    "kind": { "type": "string", "enum": ["milestone", "project"] },
                    "content": { "type": "string" }
                },
                "required": ["kind", "content"]
            })
        );
        assert!(collect_forbidden_union_schema_paths(&sanitized, "$").is_empty());
    }

    #[test]
    fn removes_parent_type_alongside_any_of_by_flattening_heterogeneous_unions() {
        let schema = json!({
            "type": "object",
            "properties": {
                "keyFiles": {
                    "type": "string",
                    "anyOf": [{ "type": "array", "items": { "type": "string" } }, { "type": "string" }],
                    "description": "Key files"
                }
            }
        });

        let sanitized = sanitize_schema_for_moonshot(&schema);
        assert!(collect_forbidden_union_schema_paths(&sanitized, "$").is_empty());
        assert_eq!(
            sanitized["properties"]["keyFiles"],
            json!({
                "type": "array",
                "items": { "type": "string" },
                "description": "Key files"
            })
        );
    }

    #[test]
    fn collapses_nested_one_of_string_or_array_to_array() {
        let schema = json!({
            "type": "object",
            "properties": {
                "option": {
                    "description": "Option text/value to select",
                    "oneOf": [
                        { "type": "string" },
                        { "type": "array", "items": { "type": "string" } }
                    ]
                }
            },
            "required": ["option"]
        });

        let sanitized = sanitize_schema_for_moonshot(&schema);
        assert!(collect_forbidden_union_schema_paths(&sanitized, "$").is_empty());
        assert_eq!(sanitized["properties"]["option"]["type"], "array");
    }

    #[test]
    fn strips_root_any_of_required_only_variants() {
        let schema = json!({
            "type": "object",
            "properties": {
                "recordingId": { "type": "string" },
                "bundlePath": { "type": "string" }
            },
            "anyOf": [
                { "required": ["recordingId"] },
                { "required": ["bundlePath"] }
            ]
        });

        let sanitized = sanitize_schema_for_moonshot(&schema);
        assert!(collect_forbidden_union_schema_paths(&sanitized, "$").is_empty());
        assert_eq!(sanitized["type"], "object");
        assert!(sanitized["properties"]["recordingId"].is_object());
        assert!(sanitized["properties"]["bundlePath"].is_object());
    }
}
