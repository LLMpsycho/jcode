//! Render read-only language-server hover and signature responses.

use serde_json::Value;

pub(super) fn render_hover(value: &Value) -> String {
    let Some(contents) = value.get("contents") else {
        return "No hover information.".to_owned();
    };
    if let Some(text) = contents.as_str() {
        return text.to_owned();
    }
    if let Some(text) = contents.get("value").and_then(Value::as_str) {
        return text.to_owned();
    }
    if let Some(items) = contents.as_array() {
        let text = items
            .iter()
            .filter_map(|item| {
                item.as_str()
                    .or_else(|| item.get("value").and_then(Value::as_str))
            })
            .collect::<Vec<_>>()
            .join("\n");
        if !text.is_empty() {
            return text;
        }
    }
    "Hover information was returned in an unsupported shape.".to_owned()
}

pub(super) fn render_signature_help(value: &Value) -> (String, usize) {
    let signatures = value
        .get("signatures")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    let lines = signatures
        .iter()
        .filter_map(|signature| signature.get("label").and_then(Value::as_str))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let count = lines.len();
    if lines.is_empty() {
        ("No signature help.".to_owned(), 0)
    } else {
        (lines.join("\n"), count)
    }
}
