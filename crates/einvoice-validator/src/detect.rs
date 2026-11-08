//! The syntax of a document, from the name of its root element: what picks the built-in
//! rule set when none is named.

use std::fmt;

/// The namespace of a UBL 2 invoice.
const UBL_INVOICE: &str = "urn:oasis:names:specification:ubl:schema:xsd:Invoice-2";
/// The namespace of a UBL 2 credit note.
const UBL_CREDIT_NOTE: &str = "urn:oasis:names:specification:ubl:schema:xsd:CreditNote-2";
/// The namespace of a UN/CEFACT Cross Industry Invoice (D16B and the other releases of
/// version 100).
const CII: &str = "urn:un:unece:uncefact:data:standard:CrossIndustryInvoice:100";

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
    /// declaration before the root, an encoding other than UTF-8). Only the start of the
    /// document is read; whether it is well-formed is for the validation to say.
    #[must_use]
    pub fn detect(document: &[u8]) -> Option<Self> {
        let (local, namespace) = root_name(document)?;
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
        let value = std::str::from_utf8(&after[..end]).ok()?;
        rest = &after[end + 1..];
        // A value with a reference would need decoding: not one of the namespaces above.
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
