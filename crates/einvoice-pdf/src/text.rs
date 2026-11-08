//! Text strings (ISO 32000-2, 7.9.2.2): UTF-16BE with a byte order mark, UTF-8 with a
//! byte order mark (PDF 2.0), else `PDFDocEncoding` (Annex D, Table D.2). Bytes that
//! `PDFDocEncoding` leaves undefined, and language escape sequences, are refused.

/// `PDFDocEncoding` bytes 0x18 to 0x1F.
const LOW: [char; 8] = [
    '\u{02D8}', '\u{02C7}', '\u{02C6}', '\u{02D9}', '\u{02DD}', '\u{02DB}', '\u{02DA}', '\u{02DC}',
];

/// `PDFDocEncoding` bytes 0x80 to 0xA0 (`None`: undefined, byte 0x9F).
const HIGH: [Option<char>; 33] = [
    Some('\u{2022}'),
    Some('\u{2020}'),
    Some('\u{2021}'),
    Some('\u{2026}'),
    Some('\u{2014}'),
    Some('\u{2013}'),
    Some('\u{0192}'),
    Some('\u{2044}'),
    Some('\u{2039}'),
    Some('\u{203A}'),
    Some('\u{2212}'),
    Some('\u{2030}'),
    Some('\u{201E}'),
    Some('\u{201C}'),
    Some('\u{201D}'),
    Some('\u{2018}'),
    Some('\u{2019}'),
    Some('\u{201A}'),
    Some('\u{2122}'),
    Some('\u{FB01}'),
    Some('\u{FB02}'),
    Some('\u{0141}'),
    Some('\u{0152}'),
    Some('\u{0160}'),
    Some('\u{0178}'),
    Some('\u{017D}'),
    Some('\u{0131}'),
    Some('\u{0142}'),
    Some('\u{0153}'),
    Some('\u{0161}'),
    Some('\u{017E}'),
    None,
    Some('\u{20AC}'),
];

fn pdf_doc_char(byte: u8) -> Option<char> {
    match byte {
        0x18..=0x1F => Some(LOW[usize::from(byte - 0x18)]),
        0x80..=0xA0 => HIGH[usize::from(byte - 0x80)],
        b'\t' | b'\n' | b'\r' | 0x20..=0x7E | 0xA1..=0xAC | 0xAE..=0xFF => Some(char::from(byte)),
        _ => None,
    }
}

/// The text of a text string, or why it cannot be read.
pub(crate) fn decode(bytes: &[u8]) -> Result<String, String> {
    if let Some(utf16) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        let (pairs, odd) = utf16.as_chunks::<2>();
        if !odd.is_empty() {
            return Err("UTF-16BE text string with an odd number of bytes".to_owned());
        }
        let units = pairs.iter().map(|pair| u16::from_be_bytes(*pair));
        let text: String = char::decode_utf16(units)
            .collect::<Result<_, _>>()
            .map_err(|_| "UTF-16BE text string with an unpaired surrogate".to_owned())?;
        refuse_escape(text)
    } else if let Some(utf8) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        let text = std::str::from_utf8(utf8)
            .map_err(|_| "UTF-8 text string that is not valid UTF-8".to_owned())?;
        refuse_escape(text.to_owned())
    } else {
        bytes
            .iter()
            .map(|&byte| {
                pdf_doc_char(byte).ok_or_else(|| {
                    format!("byte 0x{byte:02X}, undefined in PDFDocEncoding, in a text string")
                })
            })
            .collect()
    }
}

fn refuse_escape(text: String) -> Result<String, String> {
    if text.contains('\u{1B}') {
        Err("language escape sequence in a text string".to_owned())
    } else {
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::decode;

    #[test]
    fn pdf_doc_encoding() {
        assert_eq!(decode(b"factur-x.xml").unwrap(), "factur-x.xml");
        assert_eq!(
            decode(b"\x80\x92\xA0\xE9\x18").unwrap(),
            "\u{2022}\u{2122}\u{20AC}é\u{02D8}"
        );
        assert!(decode(b"a\x9Fb").is_err());
        assert!(decode(b"a\xADb").is_err());
        assert!(decode(b"a\x7Fb").is_err());
        assert!(decode(b"a\x01b").is_err());
        assert_eq!(decode(b"").unwrap(), "");
    }

    #[test]
    fn unicode() {
        assert_eq!(
            decode(b"\xFE\xFF\x00f\x00x\x00.\x00x\x00m\x00l").unwrap(),
            "fx.xml"
        );
        assert_eq!(decode(b"\xFE\xFF\xD8\x3D\xDE\x00").unwrap(), "\u{1F600}");
        assert!(decode(b"\xFE\xFF\x00").is_err());
        assert!(decode(b"\xFE\xFF\xD8\x3D").is_err());
        assert!(decode(b"\xFE\xFF\x00\x1B\x00f\x00r\x00\x1B").is_err());
        assert_eq!(decode("\u{FEFF}é".as_bytes()).unwrap(), "é");
        assert!(decode(b"\xEF\xBB\xBF\xFF").is_err());
    }
}
