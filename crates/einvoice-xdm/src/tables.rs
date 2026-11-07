//! Character tables whose content is fixed by the reference engine, not by the
//! Unicode version of the Rust toolchain. Both are checked against Saxon by the
//! oracle tests (`tests/oracle/`).

/// Code points matched by the regular expression escape `\d` in Saxon-HE 12.10: the
/// general category Nd of Unicode 6.2, the version of Saxon's category tables
/// (`net/sf/saxon/data/categories.xml`, the same data as in Saxon 9.9).
/// Inclusive ranges.
pub(crate) const SAXON_ND: &[(u32, u32)] = &[
    (0x0030, 0x0039),
    (0x0660, 0x0669),
    (0x06F0, 0x06F9),
    (0x07C0, 0x07C9),
    (0x0966, 0x096F),
    (0x09E6, 0x09EF),
    (0x0A66, 0x0A6F),
    (0x0AE6, 0x0AEF),
    (0x0B66, 0x0B6F),
    (0x0BE6, 0x0BEF),
    (0x0C66, 0x0C6F),
    (0x0CE6, 0x0CEF),
    (0x0D66, 0x0D6F),
    (0x0E50, 0x0E59),
    (0x0ED0, 0x0ED9),
    (0x0F20, 0x0F29),
    (0x1040, 0x1049),
    (0x1090, 0x1099),
    (0x17E0, 0x17E9),
    (0x1810, 0x1819),
    (0x1946, 0x194F),
    (0x19D0, 0x19D9),
    (0x1A80, 0x1A89),
    (0x1A90, 0x1A99),
    (0x1B50, 0x1B59),
    (0x1BB0, 0x1BB9),
    (0x1C40, 0x1C49),
    (0x1C50, 0x1C59),
    (0xA620, 0xA629),
    (0xA8D0, 0xA8D9),
    (0xA900, 0xA909),
    (0xA9D0, 0xA9D9),
    (0xAA50, 0xAA59),
    (0xABF0, 0xABF9),
    (0xFF10, 0xFF19),
    (0x104A0, 0x104A9),
    (0x11066, 0x1106F),
    (0x110F0, 0x110F9),
    (0x11136, 0x1113F),
    (0x111D0, 0x111D9),
    (0x116C0, 0x116C9),
    (0x1D7CE, 0x1D7FF),
];

/// The first code point (digit zero) of each run of ten decimal digits that Java 21's
/// `Character.digit(char, 10)` accepts in the Basic Multilingual Plane (Unicode 15.0).
/// Saxon parses integers of 17 characters or more with `new BigInteger(String)`,
/// which reads its digits this way, one UTF-16 unit at a time.
const JAVA_BMP_DIGIT_ZEROS: &[u32] = &[
    0x0030, 0x0660, 0x06F0, 0x07C0, 0x0966, 0x09E6, 0x0A66, 0x0AE6, 0x0B66, 0x0BE6, 0x0C66, 0x0CE6,
    0x0D66, 0x0DE6, 0x0E50, 0x0ED0, 0x0F20, 0x1040, 0x1090, 0x17E0, 0x1810, 0x1946, 0x19D0, 0x1A80,
    0x1A90, 0x1B50, 0x1BB0, 0x1C40, 0x1C50, 0xA620, 0xA8D0, 0xA900, 0xA9D0, 0xA9F0, 0xAA50, 0xABF0,
    0xFF10,
];

/// The value of a decimal digit as Java's `Character.digit(c, 10)` reads it, for
/// characters of the Basic Multilingual Plane.
pub(crate) fn java_decimal_digit(c: char) -> Option<u8> {
    let code = u32::from(c);
    JAVA_BMP_DIGIT_ZEROS.iter().find_map(|&zero| {
        let offset = code.checked_sub(zero)?;
        u8::try_from(offset).ok().filter(|&d| d < 10)
    })
}

/// Whether `\d` matches the character in Saxon.
#[cfg(test)]
pub(crate) fn saxon_is_digit(c: char) -> bool {
    let code = u32::from(c);
    SAXON_ND
        .iter()
        .any(|&(low, high)| (low..=high).contains(&code))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_are_sorted_and_sized() {
        assert!(SAXON_ND.windows(2).all(|w| w[0].1 < w[1].0));
        let count: u32 = SAXON_ND.iter().map(|(l, h)| h - l + 1).sum();
        assert_eq!(count, 460);
        assert!(JAVA_BMP_DIGIT_ZEROS.windows(2).all(|w| w[0] + 10 <= w[1]));
        assert_eq!(java_decimal_digit('7'), Some(7));
        assert_eq!(java_decimal_digit('\u{0663}'), Some(3));
        assert_eq!(java_decimal_digit('a'), None);
        assert!(saxon_is_digit('\u{1D7CE}'));
        assert!(!saxon_is_digit('\u{A9F0}'));
    }
}
