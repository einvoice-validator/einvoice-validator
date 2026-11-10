//! The documents of the encoding oracle (`tests/encoding.rs`, `examples/encoding-oracle.rs`):
//! synthetic documents in every encoding [`einvoice_xml::decode`] reads and in others,
//! with byte order marks and XML declarations that agree or not, every byte of each
//! single-byte encoding, the names of each encoding, malformed byte sequences, and the
//! files of `expected.tsv`, which records what Saxon-HE and xmllint make of each.

use std::fmt::Write as _;

/// The file of the expected results, relative to the crate.
pub(crate) const EXPECTED: &str = "tests/encoding/expected.tsv";
/// The cases where the decoder differs from the reference parsers, and why.
pub(crate) const DISAGREEMENTS: &str = "tests/encoding/disagreements.tsv";

/// The text of the documents that hold a few characters of each kind: two-byte, three-byte
/// and four-byte in UTF-8.
const BODY: &str = "<a>\u{e9}\u{20ac}\u{1F600} x</a>";

fn declaration(encoding: &str) -> String {
    format!("<?xml version=\"1.0\" encoding=\"{encoding}\"?>")
}

/// `text` in UTF-16, little-endian or big-endian, after a byte order mark or not; code
/// units given as `u16` so that unpaired surrogates can be written.
fn utf16_units(units: &[u16], big_endian: bool, bom: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(units.len() * 2 + 2);
    let mut push = |u: u16| {
        out.extend(if big_endian {
            u.to_be_bytes()
        } else {
            u.to_le_bytes()
        });
    };
    if bom {
        push(0xFEFF);
    }
    for &u in units {
        push(u);
    }
    out
}

fn utf16(text: &str, big_endian: bool, bom: bool) -> Vec<u8> {
    utf16_units(&text.encode_utf16().collect::<Vec<_>>(), big_endian, bom)
}

/// `text` (ASCII) with the code units `inserted` after its first `at` characters.
fn utf16_with(text: &str, at: usize, inserted: &[u16], big_endian: bool) -> Vec<u8> {
    let mut units: Vec<u16> = text.encode_utf16().collect();
    let tail = units.split_off(at);
    units.extend_from_slice(inserted);
    units.extend(tail);
    utf16_units(&units, big_endian, true)
}

/// The names given to each single-byte encoding in an XML declaration, the first one
/// being the name of the documents of every byte: those the decoder accepts, then others
/// that one reference parser or both accept. The last two groups are encodings the
/// decoder does not read.
pub(crate) const SINGLE_BYTE: &[(&str, &[&str])] = &[
    (
        "latin1",
        &[
            "ISO-8859-1",
            "iso-8859-1",
            "ISO_8859-1",
            "ISO8859-1",
            "ISO-IR-100",
            "iso-ir-100",
            "latin1",
            "LATIN1",
            "l1",
            "L1",
            "CP819",
            "IBM819",
            "csISOLatin1",
            "ISO8859_1",
            "8859_1",
            "ISO-LATIN-1",
            "ISO_8859-1:1987",
            "ISO_8859_1",
        ],
    ),
    (
        "latin9",
        &[
            "ISO-8859-15",
            "iso-8859-15",
            "ISO_8859-15",
            "ISO8859-15",
            "LATIN-9",
            "Latin-9",
            "latin9",
            "csISOLatin9",
            "L9",
            "ISO_8859-15:1998",
            "ISO-IR-203",
            "LATIN0",
            "csISO885915",
        ],
    ),
    (
        "cp1252",
        &[
            "windows-1252",
            "WINDOWS-1252",
            "Windows-1252",
            "cp1252",
            "CP1252",
            "Cp1252",
            "x-cp1252",
            "MS-ANSI",
            "cp-1252",
        ],
    ),
    (
        "ascii",
        &[
            "US-ASCII",
            "us-ascii",
            "ASCII",
            "ascii",
            "ANSI_X3.4-1968",
            "ANSI_X3.4-1986",
            "ISO646-US",
            "ISO-IR-6",
            "iso-ir-6",
            "CP367",
            "IBM367",
            "US",
            "csASCII",
            "ISO_646.irv:1991",
            "646",
        ],
    ),
    ("latin2", &["ISO-8859-2"]),
    ("cp1250", &["windows-1250"]),
];

/// Every probe: an identifier and the bytes of the document.
#[must_use]
#[expect(clippy::too_many_lines, reason = "one list of cases")]
pub(crate) fn probes() -> Vec<(String, Vec<u8>)> {
    let mut out: Vec<(String, Vec<u8>)> = Vec::new();
    let mut add = |id: String, bytes: Vec<u8>| out.push((id, bytes));

    // UTF-16, in both byte orders.
    for (big, e) in [(false, "le"), (true, "be")] {
        let other = if big { "LE" } else { "BE" };
        let own = e.to_uppercase();
        let decl = |name: &str| declaration(name) + BODY;
        add(format!("utf16{e}/bom"), utf16(BODY, big, true));
        add(
            format!("utf16{e}/bom-decl-utf-16"),
            utf16(&decl("UTF-16"), big, true),
        );
        add(
            format!("utf16{e}/bom-decl-utf-16-lower"),
            utf16(&decl("utf-16"), big, true),
        );
        add(
            format!("utf16{e}/bom-decl-own-order"),
            utf16(&decl(&format!("UTF-16{own}")), big, true),
        );
        add(
            format!("utf16{e}/bom-decl-own-order-lower"),
            utf16(&decl(&format!("utf-16{e}")), big, true),
        );
        add(
            format!("utf16{e}/bom-decl-other-order"),
            utf16(&decl(&format!("UTF-16{other}")), big, true),
        );
        add(
            format!("utf16{e}/bom-decl-utf-8"),
            utf16(&decl("UTF-8"), big, true),
        );
        add(
            format!("utf16{e}/bom-decl-iso-8859-1"),
            utf16(&decl("ISO-8859-1"), big, true),
        );
        add(
            format!("utf16{e}/bom-decl-us-ascii"),
            utf16(&decl("US-ASCII"), big, true),
        );
        add(
            format!("utf16{e}/bom-decl-ucs-2"),
            utf16(&decl("ISO-10646-UCS-2"), big, true),
        );
        add(
            format!("utf16{e}/bom-decl-shift-jis"),
            utf16(&decl("Shift_JIS"), big, true),
        );
        add(
            format!("utf16{e}/bom-decl-unknown"),
            utf16(&decl("x-unknown"), big, true),
        );
        add(
            format!("utf16{e}/bom-decl-utf16-no-dash"),
            utf16(&decl("UTF16"), big, true),
        );
        add(
            format!("utf16{e}/bom-decl-without-encoding"),
            utf16(&format!("<?xml version=\"1.0\"?>{BODY}"), big, true),
        );
        add(
            format!("utf16{e}/bom-decl-spaces"),
            utf16(
                &format!("<?xml version=\"1.0\"\r\n encoding = 'UTF-16' ?>\n{BODY}"),
                big,
                true,
            ),
        );
        add(
            format!("utf16{e}/no-bom-decl-utf-16"),
            utf16(&decl("UTF-16"), big, false),
        );
        add(
            format!("utf16{e}/no-bom-decl-own-order"),
            utf16(&decl(&format!("UTF-16{own}")), big, false),
        );
        add(
            format!("utf16{e}/no-bom-decl-other-order"),
            utf16(&decl(&format!("UTF-16{other}")), big, false),
        );
        add(
            format!("utf16{e}/no-bom-decl-utf-8"),
            utf16(&decl("UTF-8"), big, false),
        );
        add(
            format!("utf16{e}/no-bom-decl-iso-8859-1"),
            utf16(&decl("ISO-8859-1"), big, false),
        );
        add(
            format!("utf16{e}/no-bom-decl-without-encoding"),
            utf16(&format!("<?xml version=\"1.0\"?>{BODY}"), big, false),
        );
        add(
            format!("utf16{e}/no-bom-pi"),
            utf16(&format!("<?pi x?>{BODY}"), big, false),
        );
        add(format!("utf16{e}/no-bom-element"), utf16(BODY, big, false));
        add(
            format!("utf16{e}/no-bom-comment"),
            utf16(&format!("<!-- c -->{BODY}"), big, false),
        );
        add(
            format!("utf16{e}/no-bom-space-first"),
            utf16(&format!(" {}", decl("UTF-16")), big, false),
        );
        add(format!("utf16{e}/bom-empty"), utf16("", big, true));
        add(
            format!("utf16{e}/bom-twice"),
            utf16("\u{feff}<a/>", big, true),
        );
        add(
            format!("utf16{e}/bom-crlf"),
            utf16("<a>1\r\n2\r3</a>", big, true),
        );
        add(
            format!("utf16{e}/bom-fffe"),
            utf16("<a>x\u{fffe}y</a>", big, true),
        );
        add(
            format!("utf16{e}/bom-nul"),
            utf16("<a>x\u{0}y</a>", big, true),
        );
        add(
            format!("utf16{e}/bom-lone-high"),
            utf16_with("<a>xy</a>", 4, &[0xD800], big),
        );
        add(
            format!("utf16{e}/bom-lone-low"),
            utf16_with("<a>xy</a>", 4, &[0xDC00], big),
        );
        add(
            format!("utf16{e}/bom-reversed-pair"),
            utf16_with("<a>xy</a>", 4, &[0xDC00, 0xD800], big),
        );
        add(
            format!("utf16{e}/bom-high-at-end"),
            utf16_with("<a>x</a>", 8, &[0xD800], big),
        );
        add(
            format!("utf16{e}/bom-high-on-line-2"),
            utf16_with("<a>\n\u{e9}\u{e9}</a>", 6, &[0xD800], big),
        );
        let mut odd = utf16("<a>x</a>", big, true);
        odd.push(0);
        add(format!("utf16{e}/bom-odd-length"), odd);
        let mut odd_inside = utf16("<a>x", big, true);
        odd_inside.push(0);
        add(format!("utf16{e}/bom-odd-length-inside"), odd_inside);
        add(
            format!("utf16{e}/bom-lines"),
            utf16("<a>\r\n\u{e9}\u{e9}<b>\r\n  \u{e9}<c></b>\n</a>", big, true),
        );
        add(
            format!("utf16{e}/bom-names"),
            utf16(
                "<\u{e9}l\u{e9}ment attr\u{e9}\u{3b1}='\u{1F600}'>t</\u{e9}l\u{e9}ment>",
                big,
                true,
            ),
        );
    }

    // UTF-8: its names, byte order marks, a declaration naming another encoding.
    for (i, name) in [
        "UTF-8",
        "utf-8",
        "UTF8",
        "utf8",
        "Utf-8",
        "UTF_8",
        "unicode-1-1-utf-8",
    ]
    .iter()
    .enumerate()
    {
        add(
            format!("utf8/name-{i}-{name}"),
            (declaration(name) + BODY).into_bytes(),
        );
    }
    let bom = |rest: &[u8]| [&b"\xEF\xBB\xBF"[..], rest].concat();
    add("utf8/bom".into(), bom(BODY.as_bytes()));
    add("utf8/bom-twice".into(), bom(&bom(b"<a/>")));
    add(
        "utf8/bom-character-in-text".into(),
        bom("<a>\u{feff}</a>".as_bytes()),
    );
    for name in ["UTF-16", "UTF-16LE", "utf-16be"] {
        add(
            format!("utf8/decl-{name}"),
            (declaration(name) + BODY).into_bytes(),
        );
        add(
            format!("utf8/bom-decl-{name}"),
            bom((declaration(name) + BODY).as_bytes()),
        );
    }
    for name in [
        "ISO-8859-1",
        "ISO-8859-15",
        "windows-1252",
        "US-ASCII",
        "Shift_JIS",
        "ISO-10646-UCS-2",
        "x-unknown",
    ] {
        add(
            format!("utf8/bom-decl-{name}-ascii"),
            bom((declaration(name) + "<a>x</a>").as_bytes()),
        );
        add(
            format!("utf8/bom-decl-{name}-non-ascii"),
            bom((declaration(name) + "<a>\u{e9}\u{20ac}</a>").as_bytes()),
        );
    }
    add(
        "utf8/invalid-byte".into(),
        b"<a>\n\xC3\xA9\xFF</a>".to_vec(),
    );
    add("utf8/overlong".into(), b"<a>\xC0\xAF</a>".to_vec());
    add("utf8/surrogate".into(), b"<a>\xED\xA0\x80</a>".to_vec());
    add("utf8/truncated".into(), b"<a>\xE2\x82</a>".to_vec());
    add("utf8/truncated-at-end".into(), b"<a/>\xE2\x82".to_vec());
    add(
        "utf8/above-10ffff".into(),
        b"<a>\xF4\x90\x80\x80</a>".to_vec(),
    );
    add(
        "utf8/latin1-without-declaration".into(),
        b"<a>\xE9</a>".to_vec(),
    );

    // The single-byte encodings: their names; for those the decoder reads, every byte
    // from 0x80 and a few documents.
    for (group, names) in SINGLE_BYTE {
        for (i, name) in names.iter().enumerate() {
            add(
                format!("{group}/name-{i:02}-{name}"),
                (declaration(name) + "<a>x</a>").into_bytes(),
            );
        }
        if matches!(*group, "latin2" | "cp1250") {
            continue;
        }
        let decl = declaration(names[0]);
        for b in 0x80..=0xFF_u8 {
            add(
                format!("{group}/byte-{b:02x}"),
                [decl.as_bytes(), b"<a>", &[b], b"</a>"].concat(),
            );
        }
        let printable: Vec<u8> = (0x20..0x7F_u8)
            .filter(|b| !matches!(b, b'<' | b'&'))
            .collect();
        add(
            format!("{group}/ascii"),
            [decl.as_bytes(), b"<a>", &printable, b"</a>"].concat(),
        );
        add(
            format!("{group}/names"),
            [decl.as_bytes(), b"<a\xE9 b\xE9='\xE9'>\xE9</a\xE9>"].concat(),
        );
        add(
            format!("{group}/lines"),
            [
                decl.as_bytes(),
                b"\n<a>\r\n\xE9\xE9<b>\n  \xE9<c></b>\n</a>",
            ]
            .concat(),
        );
        add(
            format!("{group}/control"),
            [decl.as_bytes(), b"<a>\x01</a>"].concat(),
        );
        add(
            format!("{group}/single-quotes"),
            format!("<?xml version='1.0' encoding='{}'?><a>x</a>", names[0]).into_bytes(),
        );
        add(
            format!("{group}/standalone"),
            format!(
                "<?xml version='1.0' encoding='{}' standalone='yes'?><a>x</a>",
                names[0]
            )
            .into_bytes(),
        );
    }

    // Other encodings, by name or by their first bytes.
    for name in [
        "foo",
        "x-unknown",
        "IBM037",
        "UTF-32",
        "UCS-4",
        "Shift_JIS",
        "EUC-JP",
        "ISO-2022-JP",
        "UTF-7",
        "KOI8-R",
        "GB2312",
        "UCS-2",
        "ISO-10646-UCS-2",
        "UTF-32LE",
        "ISO-8859-16",
        "x-mac-roman",
        "ISO-8859-5",
        "windows-1251",
        "Big5",
    ] {
        add(
            format!("other/name-{name}"),
            (declaration(name) + "<a>x</a>").into_bytes(),
        );
    }
    let mut little = vec![0xFF, 0xFE, 0, 0];
    let mut big = vec![0, 0, 0xFE, 0xFF];
    let mut declared_little = Vec::new();
    for c in "<a>\u{e9}</a>".chars() {
        little.extend(u32::from(c).to_le_bytes());
        big.extend(u32::from(c).to_be_bytes());
    }
    for c in (declaration("UTF-32LE") + "<a>\u{e9}</a>").chars() {
        declared_little.extend(u32::from(c).to_le_bytes());
    }
    add("other/utf32le-bom".into(), little);
    add("other/utf32be-bom".into(), big);
    add("other/utf32le-no-bom".into(), declared_little);
    // `<?xml version="1.0" encoding="IBM037"?><a>x</a>` in EBCDIC (code page 037).
    let ebcdic: Vec<u8> = (declaration("IBM037") + "<a>x</a>")
        .bytes()
        .map(ebcdic_037)
        .collect();
    add("other/ebcdic".into(), ebcdic);

    // Malformed or unusual XML declarations.
    for (id, bytes) in [
        (
            "decl/newlines",
            &b"<?xml\nversion='1.0'\nencoding='ISO-8859-1'?>\n<a>\xE9</a>"[..],
        ),
        (
            "decl/tab",
            b"<?xml\tversion='1.0' encoding='ISO-8859-1'?><a>\xE9</a>",
        ),
        (
            "decl/spaces",
            b"<?xml version = '1.0'  encoding = 'ISO-8859-1' ?><a>\xE9</a>",
        ),
        (
            "decl/not-first",
            b" <?xml version='1.0' encoding='ISO-8859-1'?><a>\xE9</a>",
        ),
        (
            "decl/no-version",
            b"<?xml encoding='ISO-8859-1'?><a>\xE9</a>",
        ),
        (
            "decl/unterminated",
            b"<?xml version='1.0' encoding='ISO-8859-1'<a>\xE9</a>",
        ),
        (
            "decl/mismatched-quotes",
            b"<?xml version='1.0' encoding='ISO-8859-1\"?><a>\xE9</a>",
        ),
        (
            "decl/invalid-name",
            b"<?xml version='1.0' encoding='1SO-8859-1'?><a>x</a>",
        ),
        (
            "decl/space-in-name",
            b"<?xml version='1.0' encoding=' ISO-8859-1'?><a>x</a>",
        ),
        (
            "decl/empty-name",
            b"<?xml version='1.0' encoding=''?><a>x</a>",
        ),
        (
            "decl/encoding-twice",
            b"<?xml version='1.0' encoding='ISO-8859-1' encoding='UTF-8'?><a>\xE9</a>",
        ),
        (
            "decl/upper-case-xml",
            b"<?XML version='1.0' encoding='ISO-8859-1'?><a>\xE9</a>",
        ),
    ] {
        add(id.to_owned(), bytes.to_vec());
    }
    out
}

/// The EBCDIC code page 037 byte of an ASCII character of an XML declaration.
fn ebcdic_037(b: u8) -> u8 {
    match b {
        b' ' => 0x40,
        b'<' => 0x4C,
        b'>' => 0x6E,
        b'?' => 0x6F,
        b'=' => 0x7E,
        b'"' => 0x7F,
        b'.' => 0x4B,
        b'-' => 0x60,
        b'/' => 0x61,
        b'0'..=b'9' => 0xF0 + (b - b'0'),
        b'a'..=b'i' => 0x81 + (b - b'a'),
        b'j'..=b'r' => 0x91 + (b - b'j'),
        b's'..=b'z' => 0xA2 + (b - b's'),
        b'A'..=b'I' => 0xC1 + (b - b'A'),
        b'J'..=b'R' => 0xD1 + (b - b'J'),
        b'S'..=b'Z' => 0xE2 + (b - b'S'),
        _ => 0x3F,
    }
}

/// What a parser makes of a document: its string value (`string(/)`) or a refusal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// Parsed; the string value of the document.
    Ok(String),
    /// Refused.
    Refused,
}

impl Outcome {
    /// `ok` and the code points of the string value in hexadecimal, or `refused`.
    #[must_use]
    pub(crate) fn to_field(&self) -> String {
        match self {
            Self::Ok(text) => text.chars().fold("ok".to_owned(), |mut s, c| {
                let _ = write!(s, " {:x}", u32::from(c));
                s
            }),
            Self::Refused => "refused".to_owned(),
        }
    }

    /// The outcome of a field written by [`Outcome::to_field`].
    ///
    /// # Errors
    ///
    /// A field in another form.
    pub(crate) fn parse(field: &str) -> Result<Self, String> {
        if field == "refused" {
            return Ok(Self::Refused);
        }
        let Some(rest) = field.strip_prefix("ok") else {
            return Err(format!("not an outcome: {field}"));
        };
        rest.split_whitespace()
            .map(|h| {
                u32::from_str_radix(h, 16)
                    .ok()
                    .and_then(char::from_u32)
                    .ok_or_else(|| format!("not a code point: {h}"))
            })
            .collect::<Result<String, _>>()
            .map(Self::Ok)
    }
}

/// The lines of `expected.tsv`: id, Saxon-HE's outcome, xmllint's outcome.
#[must_use]
pub(crate) fn format_expected(rows: &[(String, Outcome, Outcome)]) -> String {
    let mut out = String::from(
        "# Written by `cargo run -p einvoice-xml --example encoding-oracle -- --write`: what\n\
         # Saxon-HE 12.10 (doc()) and xmllint (libxml2, --xpath 'string(/)') make of each\n\
         # document of tests/encoding/probes.rs: `ok` and the code points of the string value\n\
         # of the document, in hexadecimal, or `refused`. Do not edit by hand.\n\
         case\tsaxon\txmllint\n",
    );
    for (id, saxon, xmllint) in rows {
        let _ = writeln!(out, "{id}\t{}\t{}", saxon.to_field(), xmllint.to_field());
    }
    out
}

/// The rows of `expected.tsv`.
///
/// # Errors
///
/// A malformed line.
pub(crate) fn parse_expected(text: &str) -> Result<Vec<(String, Outcome, Outcome)>, String> {
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .skip(1)
        .map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            let [id, saxon, xmllint] = fields[..] else {
                return Err(format!("not three fields: {line}"));
            };
            Ok((
                id.to_owned(),
                Outcome::parse(saxon)?,
                Outcome::parse(xmllint)?,
            ))
        })
        .collect()
}

/// What this crate makes of a document: [`einvoice_xml::decode`], then the reader to the
/// end; the string value is the concatenation of the text events.
pub(crate) fn ours(bytes: &[u8]) -> Outcome {
    let Ok(text) = einvoice_xml::decode(bytes) else {
        return Outcome::Refused;
    };
    let mut reader = einvoice_xml::Reader::new(&text);
    let mut value = String::new();
    loop {
        match reader.next_event() {
            Ok(Some(einvoice_xml::Event::Text(t))) => value.push_str(&t.value),
            Ok(Some(_)) => {}
            Ok(None) => return Outcome::Ok(value),
            Err(_) => return Outcome::Refused,
        }
    }
}

/// The cases of `disagreements.tsv`: id and reason.
///
/// # Errors
///
/// A malformed line.
pub(crate) fn parse_disagreements(text: &str) -> Result<Vec<(String, String)>, String> {
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .skip(1)
        .map(|line| {
            line.split_once('\t')
                .map(|(id, why)| (id.to_owned(), why.to_owned()))
                .ok_or_else(|| format!("not two fields: {line}"))
        })
        .collect()
}

/// The comparison of this crate with the reference parsers on every probe.
#[derive(Debug, Default)]
pub(crate) struct Comparison {
    /// Cases where Saxon-HE and xmllint read the same text, and so does this crate.
    pub(crate) read: usize,
    /// Cases both refuse, and so does this crate.
    pub(crate) refused: usize,
    /// Listed cases that differ as listed.
    pub(crate) listed: Vec<String>,
    /// What is wrong: an unlisted difference, a listed case that does not differ.
    pub(crate) failures: Vec<String>,
}

/// Compares this crate ([`ours`]) with `expected` (Saxon-HE, xmllint) on `probes`:
/// where both reference parsers read a document to the same text, this crate must read
/// it to that text, or refuse it and have it listed in `disagreements` (an encoding it
/// does not read); where both refuse it, this crate must refuse it; where they differ,
/// this crate must refuse it and have it listed. A listed case that does not differ
/// from both, or that is not a probe, is a failure too.
pub(crate) fn compare(
    probes: &[(String, Vec<u8>)],
    expected: &[(String, Outcome, Outcome)],
    disagreements: &[(String, String)],
) -> Comparison {
    let mut out = Comparison::default();
    if probes.len() != expected.len() {
        out.failures.push(format!(
            "{} probes, {} expected results (run the encoding-oracle example with --write)",
            probes.len(),
            expected.len()
        ));
        return out;
    }
    for ((id, bytes), (expected_id, saxon, xmllint)) in probes.iter().zip(expected) {
        if id != expected_id {
            out.failures
                .push(format!("probe {id}, expected result for {expected_id}"));
            continue;
        }
        let ours = ours(bytes);
        let listed = disagreements.iter().find(|(d, _)| d == id);
        let differs = match (saxon == xmllint, &ours) {
            (true, ours) => ours != saxon,
            (false, Outcome::Refused) => true,
            (false, Outcome::Ok(text)) => {
                out.failures.push(format!(
                    "{id}: Saxon-HE {}, xmllint {}, but this crate reads {text:?}",
                    saxon.to_field(),
                    xmllint.to_field()
                ));
                continue;
            }
        };
        if saxon == xmllint && *saxon == Outcome::Refused && ours != Outcome::Refused {
            out.failures
                .push(format!("{id}: both refuse it, this crate reads it"));
            continue;
        }
        if saxon == xmllint
            && let (Outcome::Ok(theirs), Outcome::Ok(text)) = (saxon, &ours)
            && theirs != text
        {
            out.failures
                .push(format!("{id}: both read {theirs:?}, this crate {text:?}"));
            continue;
        }
        match (differs, listed) {
            (false, None) if ours == Outcome::Refused => out.refused += 1,
            (false, None) => out.read += 1,
            (true, Some((_, why))) => out.listed.push(format!("{id}: {why}")),
            (true, None) => out.failures.push(format!(
                "{id}: Saxon-HE {}, xmllint {}, this crate {}: not listed in {DISAGREEMENTS}",
                saxon.to_field(),
                xmllint.to_field(),
                ours.to_field()
            )),
            (false, Some(_)) => out.failures.push(format!(
                "{id}: listed in {DISAGREEMENTS}, but this crate and both reference parsers agree"
            )),
        }
    }
    for (id, _) in disagreements {
        if !probes.iter().any(|(p, _)| p == id) {
            out.failures
                .push(format!("{id}: listed in {DISAGREEMENTS}, but not a probe"));
        }
    }
    out
}
