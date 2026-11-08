//! Character data as the application sees it: references replaced, line ends
//! normalised (XML 1.0 §2.11), attribute values normalised (§3.3.3, CDATA type).

use std::borrow::Cow;

use crate::chars::{is_name, is_name_start, is_xml_char};

/// What an `&` starts.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Reference<'s> {
    /// A character reference or one of the five predefined entities.
    Char(char),
    /// A reference to another entity (its name).
    Entity(&'s str),
    /// Not a well-formed reference to an XML character.
    Malformed,
    /// A well-formed character reference to a number that is not a Unicode scalar
    /// value (a surrogate, or above `0x10FFFF`).
    NotScalar,
}

/// The reference that starts at the `&` at byte `at` of `s`, and the offset after it.
/// The reference must end within `s`.
pub(crate) fn reference(s: &str, at: usize) -> (Reference<'_>, usize) {
    let bytes = s.as_bytes();
    let mut i = at + 1;
    if bytes.get(i) == Some(&b'#') {
        i += 1;
        let hex = bytes.get(i) == Some(&b'x');
        if hex {
            i += 1;
        }
        let digits_start = i;
        while bytes.get(i).is_some_and(|b| {
            if hex {
                b.is_ascii_hexdigit()
            } else {
                b.is_ascii_digit()
            }
        }) {
            i += 1;
        }
        let Ok(n) = u32::from_str_radix(&s[digits_start..i], if hex { 16 } else { 10 }) else {
            return (Reference::Malformed, i);
        };
        let reference = match char::from_u32(n) {
            Some(c) if is_xml_char(c) => Reference::Char(c),
            Some(_) => return (Reference::Malformed, i),
            None => Reference::NotScalar,
        };
        if bytes.get(i) != Some(&b';') {
            return (Reference::Malformed, i);
        }
        return (reference, i + 1);
    }
    let name_start = i;
    match s[i..].chars().next() {
        Some(c) if is_name_start(c) => i += c.len_utf8(),
        _ => return (Reference::Malformed, i),
    }
    for c in s[i..].chars() {
        if !is_name(c) {
            break;
        }
        i += c.len_utf8();
    }
    let reference = match &s[name_start..i] {
        "quot" => Reference::Char('"'),
        "amp" => Reference::Char('&'),
        "apos" => Reference::Char('\''),
        "lt" => Reference::Char('<'),
        "gt" => Reference::Char('>'),
        name => Reference::Entity(name),
    };
    if bytes.get(i) != Some(&b';') {
        return (Reference::Malformed, i);
    }
    (reference, i + 1)
}

/// Why a text or attribute value cannot be decoded: an unknown entity (its name) or a
/// malformed reference, at a byte offset of the raw text.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Problem<'s> {
    UnknownEntity(&'s str, usize),
    Malformed(usize),
}

/// A decoded value and the offset of its first reference to a number that is not a
/// Unicode scalar value (replaced by U+FFFD in the value), if any.
pub(crate) type Decoded<'s> = (Cow<'s, str>, Option<usize>);

/// Character data between markup: references replaced, `\r\n` and lone `\r` written
/// `\n`; a character reference is never normalised.
pub(crate) fn text(raw: &str) -> Result<Decoded<'_>, Problem<'_>> {
    if memchr::memchr2(b'&', b'\r', raw.as_bytes()).is_none() {
        return Ok((Cow::Borrowed(raw), None));
    }
    decode(raw, false)
}

/// An attribute value of type CDATA: references replaced, each literal tab, line feed
/// and carriage return (a `\r\n` pair counted once) written as a space; a character
/// reference is never normalised.
pub(crate) fn attribute(raw: &str) -> Result<Decoded<'_>, Problem<'_>> {
    let bytes = raw.as_bytes();
    if memchr::memchr3(b'&', b'\t', b'\n', bytes).is_none()
        && memchr::memchr(b'\r', bytes).is_none()
    {
        return Ok((Cow::Borrowed(raw), None));
    }
    decode(raw, true)
}

fn decode(raw: &str, attribute: bool) -> Result<Decoded<'_>, Problem<'_>> {
    let bytes = raw.as_bytes();
    let mut out = String::with_capacity(raw.len());
    let mut not_scalar = None;
    let mut i = 0;
    while i < bytes.len() {
        let special = bytes[i..]
            .iter()
            .position(|&b| matches!(b, b'&' | b'\r') || (attribute && matches!(b, b'\t' | b'\n')))
            .map_or(bytes.len(), |n| i + n);
        out.push_str(&raw[i..special]);
        i = special;
        let Some(&b) = bytes.get(i) else { break };
        match b {
            b'&' => match reference(raw, i) {
                (Reference::Char(c), end) => {
                    out.push(c);
                    i = end;
                }
                (Reference::NotScalar, end) => {
                    not_scalar.get_or_insert(i);
                    out.push('\u{FFFD}');
                    i = end;
                }
                (Reference::Entity(name), _) => return Err(Problem::UnknownEntity(name, i)),
                (Reference::Malformed, _) => return Err(Problem::Malformed(i)),
            },
            b'\r' => {
                out.push(if attribute { ' ' } else { '\n' });
                i += if bytes.get(i + 1) == Some(&b'\n') {
                    2
                } else {
                    1
                };
            }
            _ => {
                out.push(' ');
                i += 1;
            }
        }
    }
    Ok((Cow::Owned(out), not_scalar))
}

/// Line ends normalised: `\r\n` and a lone `\r` written `\n` (comments, processing
/// instructions and CDATA sections, which have no references).
pub(crate) fn line_ends(raw: &str) -> Cow<'_, str> {
    let Some(first) = memchr::memchr(b'\r', raw.as_bytes()) else {
        return Cow::Borrowed(raw);
    };
    let bytes = raw.as_bytes();
    let mut out = String::with_capacity(raw.len());
    out.push_str(&raw[..first]);
    let mut i = first;
    while i < bytes.len() {
        let next = memchr::memchr(b'\r', &bytes[i..]).map_or(bytes.len(), |n| i + n);
        out.push_str(&raw[i..next]);
        if next == bytes.len() {
            break;
        }
        out.push('\n');
        i = if bytes.get(next + 1) == Some(&b'\n') {
            next + 2
        } else {
            next + 1
        };
    }
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references() {
        let r = |s: &'static str| reference(s, 0);
        assert_eq!(r("&#65;x"), (Reference::Char('A'), 5));
        assert_eq!(r("&#x41;"), (Reference::Char('A'), 6));
        assert_eq!(r("&#x00041;"), (Reference::Char('A'), 9));
        assert_eq!(r("&#X41;").0, Reference::Malformed);
        assert_eq!(r("&#x;").0, Reference::Malformed);
        assert_eq!(r("&#;").0, Reference::Malformed);
        assert_eq!(r("&#0;").0, Reference::Malformed);
        assert_eq!(r("&#xFFFE;").0, Reference::Malformed);
        assert_eq!(r("&#99999999999;").0, Reference::Malformed);
        assert_eq!(r("&#xD800;"), (Reference::NotScalar, 8));
        assert_eq!(r("&#x110000;"), (Reference::NotScalar, 10));
        assert_eq!(r("&#xD800").0, Reference::Malformed);
        assert_eq!(r("&#x10FFFF;").0, Reference::Char('\u{10FFFF}'));
        assert_eq!(r("&#9;").0, Reference::Char('\t'));
        assert_eq!(r("&amp;"), (Reference::Char('&'), 5));
        assert_eq!(r("&lt;&gt;").0, Reference::Char('<'));
        assert_eq!(r("&quot;").0, Reference::Char('"'));
        assert_eq!(r("&apos;").0, Reference::Char('\''));
        assert_eq!(r("&a:b;"), (Reference::Entity("a:b"), 5));
        assert_eq!(r("&amp").0, Reference::Malformed);
        assert_eq!(r("& ;").0, Reference::Malformed);
        assert_eq!(r("&1;").0, Reference::Malformed);
        assert_eq!(r("&").0, Reference::Malformed);
    }

    #[test]
    fn text_line_ends_and_references() {
        let t = |s: &'static str| text(s).unwrap().0.into_owned();
        assert!(matches!(text("plain").unwrap().0, Cow::Borrowed("plain")));
        assert_eq!(t("a\r\nb\rc\r"), "a\nb\nc\n");
        assert_eq!(t("\r\r\n"), "\n\n");
        assert_eq!(t("&#13;\n"), "\r\n");
        assert_eq!(t("&#65;\r"), "A\n");
        assert_eq!(t("\r&#65;"), "\nA");
        assert_eq!(t("&amp;\r&amp;"), "&\n&");
        assert_eq!(t("x&lt;y&#x20AC;"), "x<y\u{20AC}");
        assert_eq!(text("a&b;c"), Err(Problem::UnknownEntity("b", 1)));
        assert_eq!(text("a&b c"), Err(Problem::Malformed(1)));
        assert_eq!(
            text("&#xD800;&#xDFFF;"),
            Ok((Cow::Owned("\u{FFFD}\u{FFFD}".to_owned()), Some(0)))
        );
    }

    #[test]
    fn attribute_normalisation() {
        let a = |s: &'static str| attribute(s).unwrap().0.into_owned();
        assert!(matches!(attribute("a b").unwrap().0, Cow::Borrowed("a b")));
        assert_eq!(a("x\ty\nz\r\nw\rv"), "x y z w v");
        assert_eq!(a("x&#9;y&#10;z&#13;w"), "x\ty\nz\rw");
        assert_eq!(a("\r"), " ");
        assert_eq!(a("&lt;\r\n"), "< ");
    }

    #[test]
    fn line_end_normalisation() {
        assert!(matches!(line_ends("a\nb"), Cow::Borrowed("a\nb")));
        assert_eq!(line_ends("a\r\nb\rc\r\r\n"), "a\nb\nc\n\n");
        assert_eq!(line_ends("\r"), "\n");
    }
}
