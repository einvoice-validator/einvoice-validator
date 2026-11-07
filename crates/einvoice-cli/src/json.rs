//! A JSON value and its pretty-printed form (two-space indentation, keys in the order
//! given), enough for the reports of this tool.

use std::fmt::Write as _;

/// A JSON value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Json {
    Null,
    Number(u64),
    String(String),
    Array(Vec<Self>),
    Object(Vec<(&'static str, Self)>),
}

impl Json {
    pub(crate) fn string(s: impl Into<String>) -> Self {
        Self::String(s.into())
    }

    pub(crate) fn optional(s: Option<&str>) -> Self {
        s.map_or(Self::Null, Self::string)
    }

    pub(crate) fn count(n: usize) -> Self {
        Self::Number(u64::try_from(n).unwrap_or(u64::MAX))
    }

    /// Writes the value at nesting level `level` (its first line is not indented; the
    /// lines of its members are indented by `level + 1` steps).
    pub(crate) fn write(&self, level: usize, out: &mut String) {
        match self {
            Self::Null => out.push_str("null"),
            Self::Number(n) => {
                let _ = write!(out, "{n}");
            }
            Self::String(s) => write_string(s, out),
            Self::Array(items) if items.is_empty() => out.push_str("[]"),
            Self::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push('\n');
                    indent(level + 1, out);
                    item.write(level + 1, out);
                }
                out.push('\n');
                indent(level, out);
                out.push(']');
            }
            Self::Object(members) if members.is_empty() => out.push_str("{}"),
            Self::Object(members) => {
                out.push('{');
                for (i, (key, value)) in members.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push('\n');
                    indent(level + 1, out);
                    write_string(key, out);
                    out.push_str(": ");
                    value.write(level + 1, out);
                }
                out.push('\n');
                indent(level, out);
                out.push('}');
            }
        }
    }
}

pub(crate) fn indent(level: usize, out: &mut String) {
    for _ in 0..level {
        out.push_str("  ");
    }
}

/// A JSON string (RFC 8259): `"` and `\` escaped, control characters as `\n`, `\r`,
/// `\t`, `\b`, `\f` or `\u00XX`; everything else as is (UTF-8).
pub(crate) fn write_string(s: &str, out: &mut String) {
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
            c if u32::from(c) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(value: &Json) -> String {
        let mut out = String::new();
        value.write(0, &mut out);
        out
    }

    #[test]
    fn strings_are_escaped() {
        assert_eq!(
            text(&Json::string("a\"b\\c\n\r\t\u{8}\u{c}\u{1}\u{1f} é\u{a0}")),
            r#""a\"b\\c\n\r\t\b\f\u0001\u001f é"#.to_owned() + "\u{a0}\""
        );
    }

    /// Reads back a string written by [`write_string`] (the escapes it writes only).
    fn unescape(json: &str) -> String {
        let inner = &json[1..json.len() - 1];
        let mut out = String::new();
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            if c != '\\' {
                assert!(c != '"' && u32::from(c) >= 0x20, "{json:?}");
                out.push(c);
                continue;
            }
            match chars.next().unwrap() {
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                'b' => out.push('\u{8}'),
                'f' => out.push('\u{c}'),
                'u' => {
                    let hex: String = chars.by_ref().take(4).collect();
                    out.push(char::from_u32(u32::from_str_radix(&hex, 16).unwrap()).unwrap());
                }
                other => panic!("escape {other:?} in {json:?}"),
            }
        }
        out
    }

    proptest::proptest! {
        /// Any string is written as one JSON string that reads back to it.
        #[test]
        fn strings_round_trip(value in "\\PC*|[\\x00-\\x7f]*|[\\x00-\\x20\"\\\\]*") {
            let mut out = String::new();
            write_string(&value, &mut out);
            proptest::prop_assert!(out.starts_with('"') && out.ends_with('"') && out.len() >= 2);
            proptest::prop_assert_eq!(unescape(&out), value);
        }
    }

    #[test]
    fn pretty_printing() {
        let value = Json::Object(vec![
            ("a", Json::Number(1)),
            ("b", Json::Array(vec![Json::Null, Json::Number(2)])),
            ("c", Json::Array(Vec::new())),
            ("d", Json::Object(Vec::new())),
            ("e", Json::Object(vec![("f", Json::optional(Some("g")))])),
        ]);
        assert_eq!(
            text(&value),
            "{\n  \"a\": 1,\n  \"b\": [\n    null,\n    2\n  ],\n  \"c\": [],\n  \"d\": {},\n  \"e\": {\n    \"f\": \"g\"\n  }\n}"
        );
    }
}
