//! Python's semantics where the output format was first defined by the Python
//! builder: changing any of these changes published files or what gets fetched.

use serde_json::Value;

/// Python truthiness of a JSON value.
pub fn is_truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(m) => !m.is_empty(),
    }
}

/// `value or …`: the value when truthy, so the caller's fallback applies otherwise.
pub fn truthy(value: Option<&Value>) -> Option<&Value> {
    value.filter(|v| is_truthy(v))
}

/// `a or b` on strings.
pub fn or_str<'a>(a: Option<&'a str>, b: &'a str) -> &'a str {
    a.filter(|s| !s.is_empty()).unwrap_or(b)
}

/// A JSON string literal as `json.dumps(s, ensure_ascii=False)` writes it.
pub fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn json_str_matches_python() {
        assert_eq!(
            json_str("a\u{1}\u{7f}\u{2028}\\\t"),
            "\"a\\u0001\u{7f}\u{2028}\\\\\\t\""
        );
    }

    #[test]
    fn truthiness_matches_python() {
        let falsy = [
            json!(null),
            json!(false),
            json!(0),
            json!(""),
            json!([]),
            json!({}),
        ];
        assert!(falsy.iter().all(|v| !is_truthy(v)));
        assert!([
            json!(true),
            json!(1),
            json!("x"),
            json!([0]),
            json!({"a": 0})
        ]
        .iter()
        .all(is_truthy));
        assert_eq!(or_str(Some(""), "b"), "b");
        assert_eq!(or_str(None, "b"), "b");
        assert_eq!(or_str(Some("a"), "b"), "a");
    }
}
