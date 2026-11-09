//! Bounded validation for the native graphics allowlist's serialized fields.
//! Unknown fields are refused; native defaulted fields may be absent in older data.
use serde_json::Value;

pub(super) const MAX_BYTES: usize = 4096;

pub(super) fn known_fields(input: &Value, typed: &Value) -> bool {
    fn walk(input: &Value, typed: &Value, depth: usize) -> bool {
        if depth > 16 {
            return false;
        }
        match (input, typed) {
            (Value::Object(input), Value::Object(typed)) => input
                .iter()
                .all(|(key, value)| typed.get(key).is_some_and(|other| walk(value, other, depth + 1))),
            (Value::Array(input), Value::Array(typed)) => {
                input.len() == typed.len() && input.iter().zip(typed).all(|(a, b)| walk(a, b, depth + 1))
            }
            (Value::Object(_) | Value::Array(_), _) | (_, Value::Object(_) | Value::Array(_)) => false,
            _ => true,
        }
    }
    walk(input, typed, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn workspace_graphics_codec_accepts_known_defaults_but_refuses_unknown_nested_fields() {
        let typed = json!({"quality":{"glow":true,"draw_distance":128.0},"exposure":1.0});
        assert!(known_fields(&json!({"quality":{"glow":false}}), &typed));
        assert!(!known_fields(&json!({"account":"private"}), &typed));
        assert!(!known_fields(&json!({"quality":{"unknown":true}}), &typed));
    }
    #[test]
    fn workspace_graphics_codec_checks_every_array_element_and_shape() {
        let typed = json!({"array":[{"known":1},{"known":2}]});
        assert!(known_fields(&typed, &typed));
        assert!(!known_fields(&json!({"array":[{"known":1},{"unknown":2}]}), &typed));
        assert!(!known_fields(&json!({"array":[{"known":1}]}), &typed));
        assert!(!known_fields(&json!({"array":{}}), &typed));
    }
    #[test]
    fn workspace_graphics_codec_refuses_excessive_nesting() {
        let mut input = json!(1);
        for _ in 0..18 {
            input = json!({"nested":input});
        }
        assert!(!known_fields(&input, &input));
    }
}
