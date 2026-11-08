// Nearly's own rules for events, escaped lengths and token names, copied from factory lib.rs at e82bfa0.
use near_sdk::{log, serde_json};

pub fn emit(event: &str, data: serde_json::Value) {
    log!(
        r#"EVENT_JSON:{{"standard":"nearpad","version":"2.0.0","event":"{}","data":[{}]}}"#,
        event,
        data
    );
}

// Nearly's json_len (lib.rs:5313): the bytes a string takes once JSON-escaped.
pub fn json_len(s: &str) -> usize {
    s.len()
        + s.chars()
            .map(|c| match c {
                '"' | '\\' | '\n' | '\r' | '\t' | '\u{08}' | '\u{0c}' => 1,
                c if (c as u32) < 0x20 => 5,
                _ => 0,
            })
            .sum::<usize>()
}

// Nearly's sanitize_slug and short_name (factory lib.rs:4773-4779).
pub fn sanitize_slug(symbol: &str) -> String {
    let s: String = symbol
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .take(12)
        .collect();
    if s.is_empty() {
        "t".into()
    } else {
        s
    }
}

pub fn short_name(slug: &str, taken: u32) -> String {
    if taken == 0 {
        slug.into()
    } else {
        format!("{}-{}", slug, taken + 1)
    }
}
