//! Character classes of XML 1.0 (fifth edition).

/// `S`: space, tab, line feed, carriage return.
pub(crate) const fn is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r')
}

/// The ASCII bytes of `NameStartChar`, `:` included.
const fn is_ascii_name_start(b: u8) -> bool {
    matches!(b, b'A'..=b'Z' | b'a'..=b'z' | b':' | b'_')
}

/// The ASCII bytes of `NameChar`, `:` included.
pub(crate) const fn is_ascii_name(b: u8) -> bool {
    matches!(b, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b':' | b'_' | b'-' | b'.')
}

/// `NameStartChar`.
pub(crate) const fn is_name_start(c: char) -> bool {
    if c.is_ascii() {
        return is_ascii_name_start(c as u8);
    }
    matches!(c as u32,
        0xC0..=0xD6
        | 0xD8..=0xF6
        | 0xF8..=0x2FF
        | 0x370..=0x37D
        | 0x37F..=0x1FFF
        | 0x200C..=0x200D
        | 0x2070..=0x218F
        | 0x2C00..=0x2FEF
        | 0x3001..=0xD7FF
        | 0xF900..=0xFDCF
        | 0xFDF0..=0xFFFD
        | 0x1_0000..=0xE_FFFF)
}

/// `NameChar`.
pub(crate) const fn is_name(c: char) -> bool {
    if c.is_ascii() {
        return is_ascii_name(c as u8);
    }
    matches!(c as u32,
        0xB7
        | 0xC0..=0xD6
        | 0xD8..=0xF6
        | 0xF8..=0x2FF
        | 0x300..=0x36F
        | 0x370..=0x37D
        | 0x37F..=0x1FFF
        | 0x200C..=0x200D
        | 0x203F..=0x2040
        | 0x2070..=0x218F
        | 0x2C00..=0x2FEF
        | 0x3001..=0xD7FF
        | 0xF900..=0xFDCF
        | 0xFDF0..=0xFFFD
        | 0x1_0000..=0xE_FFFF)
}

/// `Char`, for a `char` (which is never a surrogate).
pub(crate) const fn is_xml_char(c: char) -> bool {
    match c as u32 {
        0x9 | 0xA | 0xD => true,
        n if n < 0x20 => false,
        n => n != 0xFFFE && n != 0xFFFF,
    }
}

/// Whether the name (non-empty or not) starts with a `NameStartChar`.
pub(crate) fn starts_with_name_start(name: &str) -> bool {
    name.chars().next().is_some_and(is_name_start)
}

/// The first character of `s` that is not a `Char`, with its byte offset. In UTF-8,
/// those are the C0 controls other than tab, line feed and carriage return (one byte
/// each) and U+FFFE and U+FFFF (`EF BF BE`, `EF BF BF`).
pub(crate) fn find_non_xml_char(s: &str) -> Option<(usize, char)> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while let Some(found) = bytes[i..]
        .iter()
        .position(|&b| (b < 0x20 && !matches!(b, b'\t' | b'\n' | b'\r')) || b == 0xEF)
    {
        let at = i + found;
        let b = bytes[at];
        if b != 0xEF {
            return Some((at, char::from(b)));
        }
        if bytes.get(at + 1) == Some(&0xBF) && matches!(bytes.get(at + 2), Some(0xBE | 0xBF)) {
            let c = if bytes[at + 2] == 0xBE {
                '\u{FFFE}'
            } else {
                '\u{FFFF}'
            };
            return Some((at, c));
        }
        i = at + 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_xml_characters_are_found_by_bytes() {
        assert_eq!(find_non_xml_char("abc\t\n\r"), None);
        assert_eq!(find_non_xml_char("ab\u{1}c"), Some((2, '\u{1}')));
        assert_eq!(
            find_non_xml_char("\u{e9}\u{ef}\u{FFFD}x\u{FFFE}"),
            Some((8, '\u{FFFE}'))
        );
        assert_eq!(find_non_xml_char("\u{FFFF}"), Some((0, '\u{FFFF}')));
        assert_eq!(find_non_xml_char("\u{FEFF}\u{FFEF}"), None);
        assert_eq!(find_non_xml_char("\0"), Some((0, '\0')));
        // Agrees with the definition on every character.
        for n in 0..=0x10_FFFFu32 {
            let Some(c) = char::from_u32(n) else { continue };
            let mut buffer = [0; 4];
            let s = c.encode_utf8(&mut buffer);
            assert_eq!(find_non_xml_char(s).is_none(), is_xml_char(c), "{n:X}");
        }
    }
}
