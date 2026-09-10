//! UTF-8-safe output budgets. All size limits in this module are bytes unless noted.

/// Longest UTF-8 prefix fitting `max_bytes` (never splits a code point).
pub fn byte_safe_prefix(text: &str, max_bytes: usize) -> &str {
    &text[..text.floor_char_boundary(max_bytes.min(text.len()))]
}

/// Prefix limited by Unicode scalar values, not bytes or grapheme clusters.
pub fn char_safe_prefix(text: &str, max_chars: usize) -> &str {
    let end = text
        .char_indices()
        .nth(max_chars)
        .map_or(text.len(), |(i, _)| i);
    &text[..end]
}

/// Hard byte cap for text, including the marker; tiny budgets omit the marker.
pub fn cap_bytes(text: &str, max_bytes: usize) -> String {
    const MARKER: &str = "\n[truncated]";
    if text.len() <= max_bytes {
        return text.to_owned();
    }
    if max_bytes < MARKER.len() {
        return byte_safe_prefix(text, max_bytes).to_owned();
    }
    format!(
        "{}{MARKER}",
        byte_safe_prefix(text, max_bytes - MARKER.len())
    )
}

/// Cap JSON without returning a broken document. Oversized JSON becomes an
/// explicit envelope with an escaped source preview, never a partial value that
/// could be mistaken for a complete result. Serialization overhead counts toward
/// the budget. Tiny budgets use a marker-only object, `null`, or `0`; a zero-byte
/// budget returns empty text because no valid JSON document can fit.
/// Non-JSON input falls back to the text cap.
pub fn truncate_json(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_owned();
    }
    if serde_json::from_str::<serde_json::Value>(text).is_err() {
        return cap_bytes(text, max_bytes);
    }
    const EMPTY: &str = r#"{"_truncated":true,"preview":""}"#;
    if max_bytes < EMPTY.len() {
        return [r#"{"_truncated":true}"#, "null", "0", ""]
            .into_iter()
            .find(|marker| marker.len() <= max_bytes)
            .unwrap_or_default()
            .to_owned();
    }
    // Bound the candidate before scanning: JSON escaping never shrinks a prefix.
    let candidate = byte_safe_prefix(text, max_bytes - EMPTY.len());
    let mut used = EMPTY.len();
    let mut chars = 0;
    for c in candidate.chars() {
        let escaped_bytes = match c {
            '"' | '\\' | '\n' | '\r' | '\t' | '\u{08}' | '\u{0c}' => 2,
            '\u{00}'..='\u{1f}' => 6,
            _ => c.len_utf8(),
        };
        if escaped_bytes > max_bytes - used {
            break;
        }
        used += escaped_bytes;
        chars += 1;
    }
    serde_json::json!({
        "_truncated": true,
        "preview": char_safe_prefix(candidate, chars),
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_caps_are_parseable_and_marked() {
        let input = serde_json::json!({"items": ["🦀\n\"".repeat(100), "tail"]}).to_string();
        for budget in 1..input.len() {
            let output = truncate_json(&input, budget);
            assert!(output.len() <= budget);
            let value: serde_json::Value = serde_json::from_str(&output).unwrap();
            if budget >= 32 {
                assert_eq!(value["_truncated"], true);
                assert!(input.starts_with(value["preview"].as_str().unwrap()));
            }
        }
        assert_eq!(truncate_json(&input, input.len()), input);
        assert_eq!(truncate_json(&input, 0), "");
        assert!(truncate_json("not json 🦀", 8).len() <= 8);
    }

    #[test]
    fn prefixes_and_caps_respect_utf8_boundaries() {
        let text = "aé🦀終";
        for budget in 0..=text.len() + 1 {
            let prefix = byte_safe_prefix(text, budget);
            assert!(prefix.len() <= budget);
            assert!(text.starts_with(prefix));
            assert!(cap_bytes(text, budget).len() <= budget);
        }
        assert_eq!(byte_safe_prefix(text, 2), "a");
        assert_eq!(char_safe_prefix(text, 3), "aé🦀");
        assert_eq!(char_safe_prefix(text, usize::MAX), text);
        assert_eq!(cap_bytes(text, text.len()), text);
    }
}
