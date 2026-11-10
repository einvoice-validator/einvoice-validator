//! Documents re-encoded from UTF-8 into the other encodings the validator reads, for the
//! test `tests/encodings.rs` (the validator gives the report of the UTF-8 document) and the
//! example `encoding-oracle` (Saxon-HE and xmllint give the results of the UTF-8 document).
//! The encoders here are written from the published tables, independently of the decoder
//! of `einvoice-xml`.

/// How a target encoding writes the text.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Form {
    /// UTF-16, big-endian or not, after a byte order mark or not.
    Utf16 { big_endian: bool, bom: bool },
    /// One byte per character.
    SingleByte(fn(char) -> Option<u8>),
}

/// A target encoding: a label (a directory name), the name the XML declaration gives
/// (`None`: the declaration, if any, names no encoding), and its form.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Target {
    pub(crate) label: &'static str,
    pub(crate) declared: Option<&'static str>,
    pub(crate) form: Form,
}

impl Target {
    /// Whether the target is UTF-16, which writes every character.
    pub(crate) const fn form_is_utf16(&self) -> bool {
        matches!(self.form, Form::Utf16 { .. })
    }
}

/// The target encodings: UTF-16 in each byte order, with a byte order mark and without
/// (the first bytes `<?` then give it), and each single-byte encoding, under one of its
/// names.
pub(crate) const TARGETS: [Target; 8] = [
    Target {
        label: "utf-16le-bom",
        declared: None,
        form: Form::Utf16 {
            big_endian: false,
            bom: true,
        },
    },
    Target {
        label: "utf-16be-bom",
        declared: Some("UTF-16"),
        form: Form::Utf16 {
            big_endian: true,
            bom: true,
        },
    },
    Target {
        label: "utf-16le",
        declared: Some("UTF-16LE"),
        form: Form::Utf16 {
            big_endian: false,
            bom: false,
        },
    },
    Target {
        label: "utf-16be",
        declared: Some("utf-16"),
        form: Form::Utf16 {
            big_endian: true,
            bom: false,
        },
    },
    Target {
        label: "iso-8859-1",
        declared: Some("ISO-8859-1"),
        form: Form::SingleByte(latin1),
    },
    Target {
        label: "iso-8859-15",
        declared: Some("latin9"),
        form: Form::SingleByte(latin9),
    },
    Target {
        label: "windows-1252",
        declared: Some("windows-1252"),
        form: Form::SingleByte(windows_1252),
    },
    Target {
        label: "us-ascii",
        declared: Some("US-ASCII"),
        form: Form::SingleByte(ascii),
    },
];

fn ascii(c: char) -> Option<u8> {
    u8::try_from(c).ok().filter(u8::is_ascii)
}

fn latin1(c: char) -> Option<u8> {
    u8::try_from(c).ok()
}

/// ISO-8859-15 (ISO/IEC 8859-15:1999): ISO-8859-1 with eight positions replaced.
fn latin9(c: char) -> Option<u8> {
    const REPLACED: [(char, u8); 8] = [
        ('\u{20AC}', 0xA4),
        ('\u{0160}', 0xA6),
        ('\u{0161}', 0xA8),
        ('\u{017D}', 0xB4),
        ('\u{017E}', 0xB8),
        ('\u{0152}', 0xBC),
        ('\u{0153}', 0xBD),
        ('\u{0178}', 0xBE),
    ];
    if let Some(&(_, b)) = REPLACED.iter().find(|(r, _)| *r == c) {
        return Some(b);
    }
    latin1(c).filter(|b| !REPLACED.iter().any(|(_, r)| r == b))
}

/// windows-1252 (Unicode's `CP1252.TXT`): ISO-8859-1 with 27 characters from 0x80 to
/// 0x9F, where the other five bytes are no character.
fn windows_1252(c: char) -> Option<u8> {
    const HIGH: [(char, u8); 27] = [
        ('\u{20AC}', 0x80),
        ('\u{201A}', 0x82),
        ('\u{0192}', 0x83),
        ('\u{201E}', 0x84),
        ('\u{2026}', 0x85),
        ('\u{2020}', 0x86),
        ('\u{2021}', 0x87),
        ('\u{02C6}', 0x88),
        ('\u{2030}', 0x89),
        ('\u{0160}', 0x8A),
        ('\u{2039}', 0x8B),
        ('\u{0152}', 0x8C),
        ('\u{017D}', 0x8E),
        ('\u{2018}', 0x91),
        ('\u{2019}', 0x92),
        ('\u{201C}', 0x93),
        ('\u{201D}', 0x94),
        ('\u{2022}', 0x95),
        ('\u{2013}', 0x96),
        ('\u{2014}', 0x97),
        ('\u{02DC}', 0x98),
        ('\u{2122}', 0x99),
        ('\u{0161}', 0x9A),
        ('\u{203A}', 0x9B),
        ('\u{0153}', 0x9C),
        ('\u{017E}', 0x9E),
        ('\u{0178}', 0x9F),
    ];
    if let Some(&(_, b)) = HIGH.iter().find(|(h, _)| *h == c) {
        return Some(b);
    }
    latin1(c).filter(|b| !(0x80..=0x9F).contains(b))
}

/// `text` (the text of a document, without its byte order mark) with its XML
/// declaration naming `declared`, or naming no encoding with `None`: the value of its
/// `encoding` replaced, or the pseudo-attribute added (or removed). A document without
/// an XML declaration is given one when `declared` is set. `None` when the declaration
/// is not in the form written here.
pub(crate) fn with_declaration(text: &str, declared: Option<&str>) -> Option<String> {
    let Some(rest) = text.strip_prefix("<?xml") else {
        return Some(match declared {
            Some(name) => format!("<?xml version=\"1.0\" encoding=\"{name}\"?>{text}"),
            None => text.to_owned(),
        });
    };
    let end = rest.find("?>")? + "<?xml".len();
    let declaration = &text[..end];
    let rest = &text[end..];
    let version_end = {
        let start = declaration.find("version")?;
        let after = &declaration[start..];
        let open = after.find(['"', '\''])?;
        let quote = after[open..].chars().next()?;
        start + open + 1 + after[open + 1..].find(quote)? + 1
    };
    // What follows the version: an encoding, then perhaps standalone.
    let tail = &declaration[version_end..];
    let tail = match tail.find("encoding") {
        Some(start) => {
            let after = &tail[start..];
            let open = after.find(['"', '\''])?;
            let quote = after[open..].chars().next()?;
            let close = open + 1 + after[open + 1..].find(quote)?;
            // Drop the whitespace before `encoding` and the pseudo-attribute.
            format!("{}{}", tail[..start].trim_end(), &after[close + 1..])
        }
        None => tail.to_owned(),
    };
    let encoding = declared.map_or_else(String::new, |name| format!(" encoding=\"{name}\""));
    Some(format!(
        "{}{encoding}{tail}{rest}",
        &declaration[..version_end]
    ))
}

/// The bytes of `text` (without a byte order mark) in `target`, with its XML declaration
/// naming the target (see [`with_declaration`]); `None` when a character of the text has
/// no byte in a single-byte encoding, or the declaration cannot be rewritten.
pub(crate) fn reencode(text: &str, target: &Target) -> Option<Vec<u8>> {
    let text = with_declaration(text, target.declared)?;
    match target.form {
        Form::Utf16 { big_endian, bom } => {
            if !bom && !text.starts_with("<?") {
                return None;
            }
            let mut out = Vec::with_capacity(text.len() * 2 + 2);
            let units = bom.then_some(0xFEFF).into_iter().chain(text.encode_utf16());
            for unit in units {
                out.extend(if big_endian {
                    unit.to_be_bytes()
                } else {
                    unit.to_le_bytes()
                });
            }
            Some(out)
        }
        Form::SingleByte(encode) => text.chars().map(encode).collect(),
    }
}
