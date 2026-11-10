//! The character encoding of a document given as bytes, and its decoding to UTF-8 before
//! it is read (XML 1.0, 4.3.3 and Appendix F).
//!
//! The encodings read are those that the two reference parsers of e-invoice validation,
//! libxml2 (`xmllint`) and the XML parser of Saxon-HE (the Xerces of the JDK), both read
//! the same way, among those found in e-invoices: UTF-8, UTF-16 (either byte order),
//! US-ASCII, ISO-8859-1, ISO-8859-15 and windows-1252, each under the names both accept.
//! Any other encoding is refused as unsupported, rather than read differently from them.
//! The names both accept are those of their reference environment (libxml2 2.9.14 built
//! with iconv and ICU, on Ubuntu 24.04, and Java 21), where the encoding oracle of the
//! crate records them: libxml2 looks most names up in iconv and ICU, and the names it
//! accepts depend on the tables of the system.

use std::borrow::Cow;

use crate::{Error, TextPos};

/// A character encoding that [`decode`] reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Encoding {
    /// UTF-8, the default.
    Utf8,
    /// UTF-16, little-endian.
    Utf16Le,
    /// UTF-16, big-endian.
    Utf16Be,
    /// US-ASCII: the bytes below 0x80.
    UsAscii,
    /// ISO-8859-1 (Latin-1): each byte is the character of that code point.
    Iso8859_1,
    /// ISO-8859-15 (Latin-9): ISO-8859-1 with the euro sign and seven other letters.
    Iso8859_15,
    /// windows-1252: ISO-8859-1 with printable characters from 0x80 to 0x9F, five of
    /// which (0x81, 0x8D, 0x8F, 0x90, 0x9D) are no character.
    Windows1252,
}

impl Encoding {
    /// The preferred name of the encoding (IANA).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Utf8 => "UTF-8",
            Self::Utf16Le => "UTF-16LE",
            Self::Utf16Be => "UTF-16BE",
            Self::UsAscii => "US-ASCII",
            Self::Iso8859_1 => "ISO-8859-1",
            Self::Iso8859_15 => "ISO-8859-15",
            Self::Windows1252 => "windows-1252",
        }
    }
}

/// What the `encoding` of an XML declaration names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Named {
    /// One of the encodings read.
    Is(Encoding),
    /// `UTF-16`, without a byte order.
    Utf16,
    /// A name that is not an encoding name (`EncName`): the reader reports it.
    Invalid,
    /// Any other encoding.
    Other,
}

/// The names of each encoding read that both reference parsers accept (compared without
/// regard to case, as both do).
const NAMES: &[(&str, Named)] = &[
    ("UTF-8", Named::Is(Encoding::Utf8)),
    ("UTF8", Named::Is(Encoding::Utf8)),
    ("unicode-1-1-utf-8", Named::Is(Encoding::Utf8)),
    ("UTF-16", Named::Utf16),
    ("UTF-16LE", Named::Is(Encoding::Utf16Le)),
    ("UTF-16BE", Named::Is(Encoding::Utf16Be)),
    ("US-ASCII", Named::Is(Encoding::UsAscii)),
    ("ASCII", Named::Is(Encoding::UsAscii)),
    ("ANSI_X3.4-1968", Named::Is(Encoding::UsAscii)),
    ("ANSI_X3.4-1986", Named::Is(Encoding::UsAscii)),
    ("ISO646-US", Named::Is(Encoding::UsAscii)),
    ("ISO-IR-6", Named::Is(Encoding::UsAscii)),
    ("CP367", Named::Is(Encoding::UsAscii)),
    ("IBM367", Named::Is(Encoding::UsAscii)),
    ("US", Named::Is(Encoding::UsAscii)),
    ("csASCII", Named::Is(Encoding::UsAscii)),
    ("ISO-8859-1", Named::Is(Encoding::Iso8859_1)),
    ("ISO_8859-1", Named::Is(Encoding::Iso8859_1)),
    ("ISO8859-1", Named::Is(Encoding::Iso8859_1)),
    ("ISO-IR-100", Named::Is(Encoding::Iso8859_1)),
    ("latin1", Named::Is(Encoding::Iso8859_1)),
    ("l1", Named::Is(Encoding::Iso8859_1)),
    ("CP819", Named::Is(Encoding::Iso8859_1)),
    ("IBM819", Named::Is(Encoding::Iso8859_1)),
    ("csISOLatin1", Named::Is(Encoding::Iso8859_1)),
    ("ISO8859_1", Named::Is(Encoding::Iso8859_1)),
    ("ISO_8859_1", Named::Is(Encoding::Iso8859_1)),
    ("ISO-8859-15", Named::Is(Encoding::Iso8859_15)),
    ("ISO_8859-15", Named::Is(Encoding::Iso8859_15)),
    ("ISO8859-15", Named::Is(Encoding::Iso8859_15)),
    ("LATIN-9", Named::Is(Encoding::Iso8859_15)),
    ("latin9", Named::Is(Encoding::Iso8859_15)),
    ("csISOLatin9", Named::Is(Encoding::Iso8859_15)),
    ("L9", Named::Is(Encoding::Iso8859_15)),
    ("LATIN0", Named::Is(Encoding::Iso8859_15)),
    ("windows-1252", Named::Is(Encoding::Windows1252)),
    ("cp1252", Named::Is(Encoding::Windows1252)),
];

/// Every name of an encoding read that [`decode`] accepts in an XML declaration, with
/// the encoding (`None` for `UTF-16`, whose byte order is that of the byte order mark or
/// of the first bytes).
pub fn encoding_names() -> impl Iterator<Item = (&'static str, Option<Encoding>)> {
    NAMES.iter().map(|&(name, named)| {
        (
            name,
            match named {
                Named::Is(encoding) => Some(encoding),
                _ => None,
            },
        )
    })
}

/// What an encoding name names.
fn named(name: &str) -> Named {
    let valid = name.bytes().next().is_some_and(|b| b.is_ascii_alphabetic())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
    if !valid {
        return Named::Invalid;
    }
    NAMES
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map_or(Named::Other, |&(_, named)| named)
}

/// What the first bytes of a document say of its encoding (XML 1.0, Appendix F).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Signature {
    /// A UTF-8 byte order mark (3 bytes).
    Utf8Bom,
    /// A UTF-16 byte order mark (2 bytes).
    Utf16Bom(Encoding),
    /// `<?` in UTF-16, without a byte order mark.
    Utf16(Encoding),
    /// Nothing: UTF-8 or an encoding the declaration names.
    None,
}

/// The signature of `bytes`, or the name of an encoding that is not read: UCS-4 (four
/// bytes per character, with `<` or a byte order mark in one of its byte orders) or
/// EBCDIC (`<?xm`).
fn signature(bytes: &[u8]) -> Result<Signature, Error> {
    let unsupported = |name: &str| Err(Error::UnsupportedEncoding(name.to_owned()));
    match bytes {
        [0xEF, 0xBB, 0xBF, ..] => Ok(Signature::Utf8Bom),
        [0xFE, 0xFF, ..] => Ok(Signature::Utf16Bom(Encoding::Utf16Be)),
        [0xFF, 0xFE, ..] => Ok(Signature::Utf16Bom(Encoding::Utf16Le)),
        [0x00, 0x00, 0xFE, 0xFF, ..]
        | [0x00, 0x00, 0x00, 0x3C, ..]
        | [0x3C, 0x00, 0x00, 0x00, ..]
        | [0x00, 0x00, 0x3C, 0x00, ..]
        | [0x00, 0x3C, 0x00, 0x00, ..] => unsupported("UCS-4"),
        [0x4C, 0x6F, 0xA7, 0x94, ..] => unsupported("EBCDIC"),
        [0x3C, 0x00, 0x3F, 0x00, ..] => Ok(Signature::Utf16(Encoding::Utf16Le)),
        [0x00, 0x3C, 0x00, 0x3F, ..] => Ok(Signature::Utf16(Encoding::Utf16Be)),
        _ => Ok(Signature::None),
    }
}

/// XML whitespace (`S`).
const fn is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | b'\n')
}

/// The value of the `encoding` pseudo-attribute of the XML declaration at the start of
/// `text` (ASCII-compatible bytes, after the byte order mark), as XML 1.0 writes it:
/// `<?xml`, whitespace, `version`, `=` and a quoted value, whitespace, `encoding`, `=`
/// and a quoted value. `None` when the text does not start so: without an encoding
/// declaration, or with a malformed declaration, which the reader reports.
fn declared_encoding(text: &[u8]) -> Option<&[u8]> {
    let rest = text.strip_prefix(b"<?xml")?;
    let rest = spaces(rest)?;
    let rest = pseudo_attribute(rest, b"version")?.1;
    let rest = spaces(rest)?;
    pseudo_attribute(rest, b"encoding").map(|(value, _)| value)
}

/// What follows one or more whitespace characters, if `bytes` starts with one.
fn spaces(bytes: &[u8]) -> Option<&[u8]> {
    let n = bytes.iter().take_while(|&&b| is_space(b)).count();
    (n > 0).then(|| &bytes[n..])
}

/// `name`, `=` (with optional whitespace around it) and a quoted value at the start of
/// `bytes`: the value and what follows.
fn pseudo_attribute<'b>(bytes: &'b [u8], name: &[u8]) -> Option<(&'b [u8], &'b [u8])> {
    let rest = bytes.strip_prefix(name)?;
    let rest = &rest[rest.iter().take_while(|&&b| is_space(b)).count()..];
    let rest = rest.strip_prefix(b"=")?;
    let rest = &rest[rest.iter().take_while(|&&b| is_space(b)).count()..];
    let (&quote, rest) = rest.split_first()?;
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    let end = rest.iter().position(|&b| b == quote)?;
    Some((&rest[..end], &rest[end + 1..]))
}

/// The encoding of a document: the bytes after its byte order mark (if any) and the
/// encoding they are in.
fn resolve(bytes: &[u8]) -> Result<(Encoding, usize), Error> {
    match signature(bytes)? {
        Signature::Utf16Bom(encoding) => Ok((encoding, 2)),
        Signature::Utf16(encoding) => Ok((encoding, 0)),
        Signature::Utf8Bom => Ok((declared(&bytes[3..], "a UTF-8 byte order mark")?, 3)),
        Signature::None => Ok((declared(bytes, "UTF-8 (no byte order mark)")?, 0)),
    }
}

/// The encoding of ASCII-compatible `bytes` (after a UTF-8 byte order mark, if any): the
/// one its XML declaration names, UTF-8 without one. `found` describes what the first
/// bytes say, for an XML declaration that names UTF-16.
fn declared(bytes: &[u8], found: &'static str) -> Result<Encoding, Error> {
    let Some(name) = declared_encoding(bytes) else {
        return Ok(Encoding::Utf8);
    };
    let name = String::from_utf8_lossy(name);
    match named(&name) {
        Named::Is(Encoding::Utf16Le | Encoding::Utf16Be) | Named::Utf16 => {
            Err(Error::EncodingMismatch {
                found,
                declared: name.into_owned(),
            })
        }
        Named::Is(encoding) => Ok(encoding),
        // The reader reports the invalid name, in the text read as UTF-8.
        Named::Invalid => Ok(Encoding::Utf8),
        Named::Other => Err(Error::UnsupportedEncoding(name.into_owned())),
    }
}

/// Checks the XML declaration of a document decoded from UTF-16 (`encoding`, given by
/// the byte order mark or the first bytes): it may name UTF-16, or the same byte order.
fn check_utf16_declaration(text: &str, encoding: Encoding, bom: bool) -> Result<(), Error> {
    let Some(name) = declared_encoding(text.as_bytes()) else {
        return Ok(());
    };
    // The declaration is ASCII: decoded from UTF-16, it is valid UTF-8 on its own.
    let name = String::from_utf8_lossy(name);
    match named(&name) {
        Named::Utf16 | Named::Invalid => Ok(()),
        Named::Is(declared) if declared == encoding => Ok(()),
        Named::Other => Err(Error::UnsupportedEncoding(name.into_owned())),
        Named::Is(_) => Err(Error::EncodingMismatch {
            found: match (bom, encoding) {
                (true, Encoding::Utf16Le) => "a UTF-16LE byte order mark",
                (true, _) => "a UTF-16BE byte order mark",
                (false, Encoding::Utf16Le) => "UTF-16LE (from the first bytes)",
                (false, _) => "UTF-16BE (from the first bytes)",
            },
            declared: name.into_owned(),
        }),
    }
}

/// The text of a document given as bytes, in UTF-8, without its byte order mark: the
/// bytes themselves when they are UTF-8 (no copy), decoded otherwise.
///
/// The encoding is found as XML 1.0 (4.3.3, Appendix F) says and as the reference
/// parsers find it: a UTF-16 byte order mark, or `<?` in UTF-16 at the start, gives
/// UTF-16 in that byte order, and the XML declaration may then name `UTF-16` or that byte
/// order; otherwise the encoding is the one the XML declaration names, UTF-8 without one
/// (after a UTF-8 byte order mark too, the declaration giving the encoding of what
/// follows, as both reference parsers read it). See [`Encoding`] for the encodings read,
/// and [`encoding_names`] for their names.
///
/// # Errors
///
/// [`Error::UnsupportedEncoding`] for an encoding not read (named by the XML
/// declaration, or UCS-4 and EBCDIC by the first bytes);
/// [`Error::EncodingMismatch`] when the XML declaration names UTF-16 in a document that
/// is not (no UTF-16 byte order mark nor `<?` in UTF-16), or another encoding in a
/// document that is; [`Error::InvalidEncodedBytes`] for bytes that are not a character
/// of the encoding (in UTF-16, an unpaired surrogate or an odd number of bytes; in
/// windows-1252, one of the five bytes that are no character), at the position of the
/// character they would be in the text; [`Error::UnknownToken`] at 1:1 for a text that
/// starts with U+FEFF after the byte order mark (a second byte order mark), which the
/// [`Reader`](crate::Reader) would otherwise skip.
pub fn decode(bytes: &[u8]) -> Result<Cow<'_, str>, Error> {
    let (encoding, start) = resolve(bytes)?;
    let rest = &bytes[start..];
    let text = match encoding {
        Encoding::Utf8 => Cow::Borrowed(utf8(rest)?),
        _ => Cow::Owned(decode_other(rest, encoding, start > 0)?),
    };
    no_second_bom(&text)?;
    Ok(text)
}

/// [`decode`] on a buffer, which becomes the text without a copy when it is UTF-8 (the
/// byte order mark, if any, is removed in place).
///
/// # Errors
///
/// As [`decode`].
pub fn decode_owned(mut bytes: Vec<u8>) -> Result<String, Error> {
    let (encoding, start) = resolve(&bytes)?;
    let text = if encoding == Encoding::Utf8 {
        bytes.drain(..start);
        String::from_utf8(bytes).map_err(|e| {
            let valid = &e.as_bytes()[..e.utf8_error().valid_up_to()];
            // The prefix is UTF-8.
            invalid(Encoding::Utf8, &String::from_utf8_lossy(valid))
        })?
    } else {
        decode_other(&bytes[start..], encoding, start > 0)?
    };
    no_second_bom(&text)?;
    Ok(text)
}

/// The encoding [`decode`] reads `bytes` in, without decoding them: what the first
/// bytes say, and, unless they say UTF-16, what the XML declaration names.
///
/// # Errors
///
/// As [`decode`], for an encoding not read or an XML declaration naming UTF-16 in a
/// document that is not; the declaration of a UTF-16 document is only checked when it
/// is decoded.
pub fn encoding_of(bytes: &[u8]) -> Result<Encoding, Error> {
    resolve(bytes).map(|(encoding, _)| encoding)
}

/// `bytes` as UTF-8, or the error at the first byte that is not.
fn utf8(bytes: &[u8]) -> Result<&str, Error> {
    std::str::from_utf8(bytes).map_err(|e| {
        let valid = &bytes[..e.valid_up_to()];
        // The prefix is UTF-8.
        invalid(Encoding::Utf8, &String::from_utf8_lossy(valid))
    })
}

/// [`Error::InvalidEncodedBytes`] after the decoded text `before`.
fn invalid(encoding: Encoding, before: &str) -> Error {
    Error::InvalidEncodedBytes {
        encoding: encoding.name(),
        pos: TextPos::at(before, before.len()),
    }
}

/// Refuses a text that starts with U+FEFF once its byte order mark is removed.
fn no_second_bom(text: &str) -> Result<(), Error> {
    if text.starts_with('\u{FEFF}') {
        return Err(Error::UnknownToken(TextPos::new(1, 1)));
    }
    Ok(())
}

/// `bytes` (after the byte order mark) decoded from `encoding`, other than UTF-8.
fn decode_other(bytes: &[u8], encoding: Encoding, bom: bool) -> Result<String, Error> {
    let single: fn(u8) -> Option<char> = match encoding {
        Encoding::Utf16Le | Encoding::Utf16Be => {
            let text = utf16(bytes, encoding)?;
            check_utf16_declaration(&text, encoding, bom)?;
            return Ok(text);
        }
        Encoding::Utf8 => return utf8(bytes).map(str::to_owned),
        Encoding::UsAscii => |b| b.is_ascii().then_some(char::from(b)),
        Encoding::Iso8859_1 => |b| Some(char::from(b)),
        Encoding::Iso8859_15 => |b| Some(iso_8859_15(b)),
        Encoding::Windows1252 => windows_1252,
    };
    // Every byte is at most one character, of at most 3 bytes in UTF-8.
    let mut text = String::with_capacity(bytes.len() + bytes.len() / 2);
    for &b in bytes {
        match single(b) {
            Some(c) => text.push(c),
            None => return Err(invalid(encoding, &text)),
        }
    }
    Ok(text)
}

/// UTF-16 in the byte order of `encoding`: an odd number of bytes or an unpaired
/// surrogate is an error.
fn utf16(bytes: &[u8], encoding: Encoding) -> Result<String, Error> {
    let (pairs, odd) = bytes.as_chunks::<2>();
    let unit = |pair: &[u8; 2]| {
        if encoding == Encoding::Utf16Le {
            u16::from_le_bytes(*pair)
        } else {
            u16::from_be_bytes(*pair)
        }
    };
    let mut text = String::with_capacity(bytes.len() + bytes.len() / 2);
    for c in char::decode_utf16(pairs.iter().map(unit)) {
        match c {
            Ok(c) => text.push(c),
            Err(_) => return Err(invalid(encoding, &text)),
        }
    }
    if !odd.is_empty() {
        return Err(invalid(encoding, &text));
    }
    Ok(text)
}

/// ISO-8859-15: ISO-8859-1 but for eight bytes.
fn iso_8859_15(b: u8) -> char {
    match b {
        0xA4 => '\u{20AC}',
        0xA6 => '\u{0160}',
        0xA8 => '\u{0161}',
        0xB4 => '\u{017D}',
        0xB8 => '\u{017E}',
        0xBC => '\u{0152}',
        0xBD => '\u{0153}',
        0xBE => '\u{0178}',
        _ => char::from(b),
    }
}

/// windows-1252 from 0x80 to 0x9F (`None`: no character), as Microsoft's table and the
/// Unicode mapping (`bestfit1252.txt`, `CP1252.TXT`) give it.
const WINDOWS_1252_80: [Option<char>; 32] = [
    Some('\u{20AC}'),
    None,
    Some('\u{201A}'),
    Some('\u{0192}'),
    Some('\u{201E}'),
    Some('\u{2026}'),
    Some('\u{2020}'),
    Some('\u{2021}'),
    Some('\u{02C6}'),
    Some('\u{2030}'),
    Some('\u{0160}'),
    Some('\u{2039}'),
    Some('\u{0152}'),
    None,
    Some('\u{017D}'),
    None,
    None,
    Some('\u{2018}'),
    Some('\u{2019}'),
    Some('\u{201C}'),
    Some('\u{201D}'),
    Some('\u{2022}'),
    Some('\u{2013}'),
    Some('\u{2014}'),
    Some('\u{02DC}'),
    Some('\u{2122}'),
    Some('\u{0161}'),
    Some('\u{203A}'),
    Some('\u{0153}'),
    None,
    Some('\u{017E}'),
    Some('\u{0178}'),
];

/// windows-1252.
fn windows_1252(b: u8) -> Option<char> {
    match b {
        0x80..=0x9F => WINDOWS_1252_80[usize::from(b - 0x80)],
        _ => Some(char::from(b)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16le(text: &str) -> Vec<u8> {
        text.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }

    fn utf16be(text: &str) -> Vec<u8> {
        text.encode_utf16().flat_map(u16::to_be_bytes).collect()
    }

    #[test]
    fn utf8_is_borrowed() {
        assert!(matches!(decode(b"<a/>").unwrap(), Cow::Borrowed("<a/>")));
        assert!(matches!(
            decode(b"\xEF\xBB\xBF<a>\xC3\xA9</a>").unwrap(),
            Cow::Borrowed("<a>\u{e9}</a>")
        ));
        let declared = "<?xml version='1.0' encoding='utf8'?><a/>";
        assert!(matches!(decode(declared.as_bytes()).unwrap(), Cow::Borrowed(t) if t == declared));
        assert_eq!(decode_owned(b"\xEF\xBB\xBF<a/>".to_vec()).unwrap(), "<a/>");
    }

    #[test]
    fn utf16_with_and_without_a_byte_order_mark() {
        let text = "<?xml version=\"1.0\" encoding=\"UTF-16\"?><a>\u{e9}\u{1F600}</a>";
        for (bytes, bom) in [
            ([&[0xFF, 0xFE][..], &utf16le(text)].concat(), true),
            ([&[0xFE, 0xFF][..], &utf16be(text)].concat(), true),
            (utf16le(text), false),
            (utf16be(text), false),
        ] {
            assert_eq!(decode(&bytes).unwrap(), text, "{bom}");
            assert_eq!(decode_owned(bytes).unwrap(), text);
        }
        // Without a declaration, after a byte order mark.
        let bytes = [&[0xFF, 0xFE][..], &utf16le("<a/>")].concat();
        assert_eq!(decode(&bytes).unwrap(), "<a/>");
        assert_eq!(encoding_of(&bytes).unwrap(), Encoding::Utf16Le);
        // The byte order named.
        let le = "<?xml version='1.0' encoding='utf-16le'?><a/>";
        assert_eq!(decode(&utf16le(le)).unwrap(), le);
        // The other byte order, or another encoding: a mismatch.
        let be = "<?xml version='1.0' encoding='UTF-16BE'?><a/>";
        assert!(matches!(
            decode(&utf16le(be)),
            Err(Error::EncodingMismatch { declared, .. }) if declared == "UTF-16BE"
        ));
        let latin = "<?xml version='1.0' encoding='ISO-8859-1'?><a/>";
        assert!(matches!(
            decode(&[&[0xFF, 0xFE][..], &utf16le(latin)].concat()),
            Err(Error::EncodingMismatch { .. })
        ));
        for name in ["ISO-10646-UCS-2", "UCS-2"] {
            let ucs2 = format!("<?xml version='1.0' encoding='{name}'?><a/>");
            assert_eq!(
                decode(&[&[0xFE, 0xFF][..], &utf16be(&ucs2)].concat()),
                Err(Error::UnsupportedEncoding(name.to_owned()))
            );
        }
    }

    #[test]
    fn malformed_utf16() {
        let bytes = [&[0xFF, 0xFE][..], &utf16le("<a>\n\u{e9}x")].concat();
        let mut lone = bytes.clone();
        lone.extend([0x00, 0xD8, b'<', 0]);
        assert_eq!(
            decode(&lone),
            Err(Error::InvalidEncodedBytes {
                encoding: "UTF-16LE",
                pos: TextPos::new(2, 3)
            })
        );
        let mut odd = bytes;
        odd.push(b'>');
        assert_eq!(
            decode(&odd),
            Err(Error::InvalidEncodedBytes {
                encoding: "UTF-16LE",
                pos: TextPos::new(2, 3)
            })
        );
        // A low surrogate first.
        let low = [&[0xFE, 0xFF][..], &[0xDC, 0x00]].concat();
        assert!(matches!(
            decode(&low),
            Err(Error::InvalidEncodedBytes { pos, .. }) if pos == TextPos::new(1, 1)
        ));
    }

    #[test]
    fn single_byte_encodings() {
        let decl = |name: &str| format!("<?xml version=\"1.0\" encoding=\"{name}\"?>");
        let latin1 = [decl("ISO-8859-1").as_bytes(), b"<a>\xE9\x80\xA4</a>"].concat();
        assert_eq!(
            decode(&latin1).unwrap(),
            format!("{}<a>\u{e9}\u{80}\u{a4}</a>", decl("ISO-8859-1"))
        );
        for name in ["latin9", "L9", "LATIN0", "csISOLatin9"] {
            let latin9 = [decl(name).as_bytes(), b"<a>\xA4\xBE\xE9</a>"].concat();
            assert_eq!(
                decode(&latin9).unwrap(),
                format!("{}<a>\u{20AC}\u{178}\u{e9}</a>", decl(name))
            );
        }
        let cp1252 = [decl("Windows-1252").as_bytes(), b"<a>\x80\x9F\xFF</a>"].concat();
        assert_eq!(
            decode(&cp1252).unwrap(),
            format!("{}<a>\u{20AC}\u{178}\u{ff}</a>", decl("Windows-1252"))
        );
        for b in [0x81, 0x8D, 0x8F, 0x90, 0x9D] {
            let bytes = [decl("cp1252").as_bytes(), b"\n<a>x", &[b], b"</a>"].concat();
            assert_eq!(
                decode(&bytes),
                Err(Error::InvalidEncodedBytes {
                    encoding: "windows-1252",
                    pos: TextPos::new(2, 5)
                })
            );
        }
        let ascii = [decl("US-ASCII").as_bytes(), b"<a>\xE9</a>"].concat();
        assert!(matches!(
            decode(&ascii),
            Err(Error::InvalidEncodedBytes {
                encoding: "US-ASCII",
                ..
            })
        ));
        // After a UTF-8 byte order mark, the declaration still gives the encoding.
        let after_bom = [
            b"\xEF\xBB\xBF",
            decl("ISO-8859-1").as_bytes(),
            b"<a>\xC3\xA9</a>",
        ]
        .concat();
        assert_eq!(
            decode_owned(after_bom).unwrap(),
            format!("{}<a>\u{c3}\u{a9}</a>", decl("ISO-8859-1"))
        );
    }

    #[test]
    fn declarations() {
        assert_eq!(
            declared_encoding(b"<?xml version='1.0' encoding = \"latin1\" ?><a/>"),
            Some(&b"latin1"[..])
        );
        assert_eq!(
            declared_encoding(b"<?xml\nversion=\"1.0\"\r\n\tencoding='x'?>"),
            Some(&b"x"[..])
        );
        for text in [
            &b"<?xml version='1.0'?><a/>"[..],
            b"<?xml version='1.0' standalone='yes'?>",
            b"<?xml encoding='latin1'?>",
            b"<?xml version='1.0'encoding='latin1'?>",
            b"<?xmlversion='1.0' encoding='latin1'?>",
            b" <?xml version='1.0' encoding='latin1'?>",
            b"<?xml version='1.0' encoding='latin1",
            b"<?xml version='1.0' encoding=latin1?>",
            b"<a/>",
            b"",
        ] {
            assert_eq!(declared_encoding(text), None, "{text:?}");
        }
    }

    #[test]
    fn names() {
        assert_eq!(named("utf-8"), Named::Is(Encoding::Utf8));
        assert_eq!(named("Latin1"), Named::Is(Encoding::Iso8859_1));
        assert_eq!(named("WINDOWS-1252"), Named::Is(Encoding::Windows1252));
        assert_eq!(named("utf-16"), Named::Utf16);
        assert_eq!(named("ISO-10646-UCS-2"), Named::Other);
        assert_eq!(named("UCS-2"), Named::Other);
        assert_eq!(named("l9"), Named::Is(Encoding::Iso8859_15));
        assert_eq!(named("iso8859_1"), Named::Is(Encoding::Iso8859_1));
        assert_eq!(named("UNICODE-1-1-UTF-8"), Named::Is(Encoding::Utf8));
        assert_eq!(named("UTF_8"), Named::Other);
        assert_eq!(named("x-cp1252"), Named::Other);
        assert_eq!(named("Shift_JIS"), Named::Other);
        assert_eq!(named("ISO-8859-2"), Named::Other);
        assert_eq!(named(""), Named::Invalid);
        assert_eq!(named("1SO"), Named::Invalid);
        assert_eq!(named(" UTF-8"), Named::Invalid);
        assert_eq!(encoding_names().count(), NAMES.len());
        for (name, encoding) in encoding_names() {
            assert_eq!(encoding.is_none(), name == "UTF-16", "{name}");
        }
    }

    #[test]
    fn unsupported_and_mismatched() {
        let decl = |name: &str| format!("<?xml version=\"1.0\" encoding=\"{name}\"?><a/>");
        assert_eq!(
            decode(decl("Shift_JIS").as_bytes()),
            Err(Error::UnsupportedEncoding("Shift_JIS".to_owned()))
        );
        assert!(matches!(
            decode(decl("UTF-16").as_bytes()),
            Err(Error::EncodingMismatch { declared, .. }) if declared == "UTF-16"
        ));
        // An invalid name: read as UTF-8, the reader reports it.
        assert!(decode(decl("1SO-8859-1").as_bytes()).is_ok());
        assert_eq!(
            decode(&[0x4C, 0x6F, 0xA7, 0x94, 0x40]),
            Err(Error::UnsupportedEncoding("EBCDIC".to_owned()))
        );
        assert_eq!(
            decode(b"<\0\0\0a\0\0\0"),
            Err(Error::UnsupportedEncoding("UCS-4".to_owned()))
        );
        // A second byte order mark.
        assert_eq!(
            decode(b"\xEF\xBB\xBF\xEF\xBB\xBF<a/>"),
            Err(Error::UnknownToken(TextPos::new(1, 1)))
        );
        assert_eq!(
            decode_owned(b"\xFF\xFE\xFF\xFE<\0a\0/\0>\0".to_vec()),
            Err(Error::UnknownToken(TextPos::new(1, 1)))
        );
        assert_eq!(
            decode(b"<a>\n  \xC3\xA9\xFF</a>"),
            Err(Error::InvalidEncodedBytes {
                encoding: "UTF-8",
                pos: TextPos::new(2, 4)
            })
        );
        assert_eq!(
            decode_owned(b"\xEF\xBB\xBF<a>\n\xFF</a>".to_vec()),
            Err(Error::InvalidEncodedBytes {
                encoding: "UTF-8",
                pos: TextPos::new(2, 1)
            })
        );
    }
}
