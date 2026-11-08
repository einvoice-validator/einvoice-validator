//! A JSON value and its compact text (keys in the order given), which the JavaScript side
//! reads back with `JSON.parse`.

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

    pub(crate) fn line(n: Option<u32>) -> Self {
        n.map_or(Self::Null, |n| Self::Number(u64::from(n)))
    }

    /// The compact JSON text of the value.
    pub(crate) fn text(&self) -> String {
        let mut out = String::new();
        self.write(&mut out);
        out
    }

    fn write(&self, out: &mut String) {
        match self {
            Self::Null => out.push_str("null"),
            Self::Number(n) => {
                let _ = write!(out, "{n}");
            }
            Self::String(s) => write_string(s, out),
            Self::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    item.write(out);
                }
                out.push(']');
            }
            Self::Object(members) => {
                out.push('{');
                for (i, (key, value)) in members.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_string(key, out);
                    out.push(':');
                    value.write(out);
                }
                out.push('}');
            }
        }
    }
}

/// A JSON string (RFC 8259): `"` and `\` escaped, control characters as `\n`, `\r`,
/// `\t`, `\b`, `\f` or `\u00XX`; everything else as is (UTF-8).
fn write_string(s: &str, out: &mut String) {
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

    #[test]
    fn strings_are_escaped() {
        assert_eq!(
            Json::string("a\"b\\c\n\r\t\u{8}\u{c}\u{1}\u{1f} é\u{a0}\u{2028}").text(),
            "\"a\\\"b\\\\c\\n\\r\\t\\b\\f\\u0001\\u001f é\u{a0}\u{2028}\""
        );
    }

    /// Round trips with proptest, which does not build for WebAssembly.
    #[cfg(not(target_family = "wasm"))]
    mod round_trip {
        use super::*;

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
                let out = Json::string(value.clone()).text();
                proptest::prop_assert!(out.starts_with('"') && out.ends_with('"') && out.len() >= 2);
                proptest::prop_assert_eq!(unescape(&out), value);
            }
        }
    }

    #[test]
    fn compact_text() {
        let value = Json::Object(vec![
            ("a", Json::Number(1)),
            ("b", Json::Array(vec![Json::Null, Json::Number(2)])),
            ("c", Json::Array(Vec::new())),
            ("d", Json::Object(Vec::new())),
            ("e", Json::Object(vec![("f", Json::optional(Some("g")))])),
            ("h", Json::line(None)),
            ("i", Json::line(Some(7))),
            ("j", Json::count(3)),
        ]);
        assert_eq!(
            value.text(),
            r#"{"a":1,"b":[null,2],"c":[],"d":{},"e":{"f":"g"},"h":null,"i":7,"j":3}"#
        );
    }
}
