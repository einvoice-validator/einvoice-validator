//! The syntax of a document, from the name of its root element: what picks the built-in
//! rule set when none is named.

use std::fmt;

use einvoice_xml::Encoding;

use crate::manifest::{CII, UBL_CREDIT_NOTE, UBL_INVOICE};

/// The syntax of an e-invoice, as its root element names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Syntax {
    /// OASIS UBL 2: a root element `Invoice` or `CreditNote` in the UBL 2 namespace of
    /// that name.
    Ubl,
    /// UN/CEFACT Cross Industry Invoice: a root element `CrossIndustryInvoice` in the
    /// namespace `urn:un:unece:uncefact:data:standard:CrossIndustryInvoice:100`.
    Cii,
}

impl Syntax {
    /// The syntax of `document`, from its root element alone: its local name and
    /// namespace. `None` when the document does not start like XML with one of the root
    /// elements above (another root element, a document that is not XML, a document type
    /// declaration before the root, UTF-16 bytes that are not). Only the start of the
    /// document is read, in the encoding the validation reads it in (UTF-16 or an
    /// encoding whose bytes below 0x80 are ASCII); whether it is well-formed, and in an
    /// encoding the validation reads, is for the validation to say.
    #[must_use]
    pub fn detect(document: &[u8]) -> Option<Self> {
        let (local, namespace) = match einvoice_xml::encoding_of(document) {
            Ok(encoding @ (Encoding::Utf16Le | Encoding::Utf16Be)) => {
                root_name(utf16_lossy(document, encoding).as_bytes())?
            }
            _ => root_name(document)?,
        };
        match (local.as_str(), namespace.as_str()) {
            ("Invoice", UBL_INVOICE) | ("CreditNote", UBL_CREDIT_NOTE) => Some(Self::Ubl),
            ("CrossIndustryInvoice", CII) => Some(Self::Cii),
            _ => None,
        }
    }

    /// The name of the built-in rule set of this syntax: `cen-ubl` or `cen-cii`, the CEN
    /// EN 16931 rules (whether this build has it is in
    /// [`BUILTIN_RULE_SETS`](crate::BUILTIN_RULE_SETS)).
    #[must_use]
    pub const fn builtin_rule_set(self) -> &'static str {
        match self {
            Self::Ubl => "cen-ubl",
            Self::Cii => "cen-cii",
        }
    }

    /// `ubl` or `cii`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ubl => "ubl",
            Self::Cii => "cii",
        }
    }
}

impl fmt::Display for Syntax {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A UTF-16 document (after its byte order mark, if any) in UTF-8, each unpaired
/// surrogate replaced with U+FFFD.
fn utf16_lossy(document: &[u8], encoding: Encoding) -> String {
    let bytes = document
        .strip_prefix(&[0xFF, 0xFE])
        .or_else(|| document.strip_prefix(&[0xFE, 0xFF]))
        .unwrap_or(document);
    let units = bytes.as_chunks::<2>().0.iter().map(|&pair| {
        if encoding == Encoding::Utf16Le {
            u16::from_le_bytes(pair)
        } else {
            u16::from_be_bytes(pair)
        }
    });
    char::decode_utf16(units)
        .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect()
}

/// Whitespace in the sense of XML (`S`).
const fn is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | b'\n')
}

/// The local name and namespace of the root element: after an optional byte order mark,
/// XML declaration, comments, processing instructions and whitespace, the first start
/// tag, its name resolved with the namespace declarations of that tag (the root element
/// has no ancestor to inherit others from).
fn root_name(document: &[u8]) -> Option<(String, String)> {
    let text = document.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(document);
    let mut rest = text;
    loop {
        let skip = rest.iter().position(|&b| !is_space(b))?;
        rest = &rest[skip..];
        if let Some(after) = rest.strip_prefix(b"<?") {
            rest = &after[find(after, b"?>")? + 2..];
        } else if let Some(after) = rest.strip_prefix(b"<!--") {
            rest = &after[find(after, b"-->")? + 3..];
        } else if rest.starts_with(b"<!") {
            // A document type declaration: refused by the validation.
            return None;
        } else {
            rest = rest.strip_prefix(b"<")?;
            break;
        }
    }
    let (qname, mut rest) = name(rest)?;
    let mut namespaces: Vec<(String, String)> = Vec::new();
    loop {
        let skip = rest.iter().position(|&b| !is_space(b))?;
        let spaced = skip > 0;
        rest = &rest[skip..];
        if rest.starts_with(b">") || rest.starts_with(b"/>") {
            break;
        }
        if !spaced {
            return None;
        }
        let (attribute, after) = name(rest)?;
        let after = trim_start(after).strip_prefix(b"=")?;
        let after = trim_start(after);
        let quote = *after.first().filter(|q| matches!(q, b'"' | b'\''))?;
        let after = &after[1..];
        let end = after.iter().position(|&b| b == quote)?;
        rest = &after[end + 1..];
        // A value that is not ASCII-compatible text, or with a reference, which would need
        // decoding: not one of the namespaces above.
        let Ok(value) = std::str::from_utf8(&after[..end]) else {
            continue;
        };
        if value.contains(['&', '<']) {
            continue;
        }
        if attribute == "xmlns" {
            namespaces.push((String::new(), value.to_owned()));
        } else if let Some(prefix) = attribute.strip_prefix("xmlns:") {
            namespaces.push((prefix.to_owned(), value.to_owned()));
        }
    }
    let (prefix, local) = qname.split_once(':').unwrap_or(("", &qname));
    let namespace = namespaces
        .iter()
        .rev()
        .find(|(p, _)| p == prefix)
        .map(|(_, uri)| uri.clone())
        .unwrap_or_default();
    Some((local.to_owned(), namespace))
}

fn trim_start(bytes: &[u8]) -> &[u8] {
    let skip = bytes
        .iter()
        .position(|&b| !is_space(b))
        .unwrap_or(bytes.len());
    &bytes[skip..]
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// A name at the start of `bytes` (up to whitespace, `=`, `>` or `/`), and what follows.
fn name(bytes: &[u8]) -> Option<(String, &[u8])> {
    let end = bytes
        .iter()
        .position(|&b| is_space(b) || matches!(b, b'=' | b'>' | b'/'))
        .unwrap_or(bytes.len());
    if end == 0 {
        return None;
    }
    let text = std::str::from_utf8(&bytes[..end]).ok()?;
    Some((text.to_owned(), &bytes[end..]))
}

#[cfg(test)]
mod tests {
    use super::Syntax;

    fn detect(text: &str) -> Option<Syntax> {
        Syntax::detect(text.as_bytes())
    }

    #[test]
    fn root_elements() {
        assert_eq!(
            detect(r#"<Invoice xmlns="urn:oasis:names:specification:ubl:schema:xsd:Invoice-2"/>"#),
            Some(Syntax::Ubl)
        );
        assert_eq!(
            detect(
                "\u{feff}<?xml version='1.0'?>\n<!-- c --><?pi x?>\n<ubl:CreditNote a='1' xmlns:ubl='urn:oasis:names:specification:ubl:schema:xsd:CreditNote-2'>"
            ),
            Some(Syntax::Ubl)
        );
        assert_eq!(
            detect(
                r#"<rsm:CrossIndustryInvoice xmlns:a="x" xmlns:rsm="urn:un:unece:uncefact:data:standard:CrossIndustryInvoice:100"><x/></rsm:CrossIndustryInvoice>"#
            ),
            Some(Syntax::Cii)
        );
        assert_eq!(
            detect(
                "<CrossIndustryInvoice\n xmlns = \"urn:un:unece:uncefact:data:standard:CrossIndustryInvoice:100\" >"
            ),
            Some(Syntax::Cii)
        );
    }

    #[test]
    fn other_encodings() {
        let ubl = "<?xml version='1.0' encoding='UTF-16'?><Invoice note='\u{e9}' xmlns='urn:oasis:names:specification:ubl:schema:xsd:Invoice-2'/>";
        let le: Vec<u8> = [0xFF, 0xFE]
            .into_iter()
            .chain(ubl.encode_utf16().flat_map(u16::to_le_bytes))
            .collect();
        assert_eq!(Syntax::detect(&le), Some(Syntax::Ubl));
        let be: Vec<u8> = ubl.encode_utf16().flat_map(u16::to_be_bytes).collect();
        assert_eq!(Syntax::detect(&be), Some(Syntax::Ubl));
        // An unpaired surrogate after the root's start tag.
        let mut lone = le.clone();
        lone.extend([0x00, 0xD8]);
        assert_eq!(Syntax::detect(&lone), Some(Syntax::Ubl));
        // ISO-8859-1, a non-ASCII byte in another attribute of the root.
        let latin1 = b"<?xml version='1.0' encoding='ISO-8859-1'?><rsm:CrossIndustryInvoice a='\xE9' xmlns:rsm='urn:un:unece:uncefact:data:standard:CrossIndustryInvoice:100'/>";
        assert_eq!(Syntax::detect(latin1), Some(Syntax::Cii));
    }

    #[test]
    fn anything_else() {
        for text in [
            "",
            "   ",
            "not xml",
            "<Invoice/>",
            r#"<Invoice xmlns="urn:oasis:names:specification:ubl:schema:xsd:CreditNote-2"/>"#,
            r#"<CreditNote xmlns="urn:oasis:names:specification:ubl:schema:xsd:Invoice-2"/>"#,
            r#"<x:Invoice xmlns="urn:oasis:names:specification:ubl:schema:xsd:Invoice-2"/>"#,
            r#"<rsm:CrossIndustryInvoice xmlns:rsm="urn:un:unece:uncefact:data:standard:CrossIndustryInvoice:13"/>"#,
            r#"<!DOCTYPE Invoice><Invoice xmlns="urn:oasis:names:specification:ubl:schema:xsd:Invoice-2"/>"#,
            r#"<Invoice xmlns="urn:oasis:names:specification:ubl:schema:xsd:Invoice-2&#x31;"/>"#,
            r#"<Invoice xmlns="urn:oasis:names:specification:ubl:schema:xsd:Invoice-2"x="1"/>"#,
            "<!-- unterminated",
            "<Invoice xmlns='urn:oasis:names:specification:ubl:schema:xsd:Invoice-2",
        ] {
            assert_eq!(detect(text), None, "{text}");
        }
    }

    #[test]
    fn rule_set_names() {
        assert_eq!(Syntax::Ubl.builtin_rule_set(), "cen-ubl");
        assert_eq!(Syntax::Cii.builtin_rule_set(), "cen-cii");
        assert_eq!(Syntax::Cii.to_string(), "cii");
    }
}
