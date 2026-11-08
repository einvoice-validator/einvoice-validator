//! Qualified names, namespace resolution and the XML name character classes.

use std::collections::BTreeMap;
use std::fmt;

/// The namespace of the XPath functions (`fn:`), the default function namespace.
pub const FN_NAMESPACE: &str = "http://www.w3.org/2005/xpath-functions";
/// The XML Schema namespace (`xs:`), home of the atomic types.
pub const XS_NAMESPACE: &str = "http://www.w3.org/2001/XMLSchema";
/// The namespace bound to the `xml` prefix in every XML document.
pub const XML_NAMESPACE: &str = "http://www.w3.org/XML/1998/namespace";

/// A namespace binding: a namespace node of an element, or a namespace declaration.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NamespaceBinding {
    /// The prefix; `None` for the default namespace.
    pub prefix: Option<String>,
    /// The namespace URI.
    pub uri: String,
}

/// A qualified name as written (`prefix:local` or `local`) together with the namespace
/// it was resolved to when it was parsed.
///
/// Two names are equal when prefix, namespace and local part are all equal; use
/// [`QName::same_expanded_name`] to compare expanded names only.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct QName {
    /// The prefix as written, if any.
    pub prefix: Option<String>,
    /// The namespace URI the name belongs to; `None` for a name in no namespace.
    pub namespace: Option<String>,
    /// The local part.
    pub local: String,
}

impl QName {
    /// A name in no namespace, without prefix.
    #[must_use]
    pub fn local(local: &str) -> Self {
        Self {
            prefix: None,
            namespace: None,
            local: local.to_owned(),
        }
    }

    /// Whether both names have the same namespace and local part (the prefix is ignored).
    #[must_use]
    pub fn same_expanded_name(&self, other: &Self) -> bool {
        self.namespace == other.namespace && self.local == other.local
    }
}

impl fmt::Display for QName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.prefix {
            Some(prefix) => write!(f, "{prefix}:{}", self.local),
            None => f.write_str(&self.local),
        }
    }
}

/// The statically known namespaces of an expression: what its prefixes are bound to.
///
/// The `xml` prefix is always bound to [`XML_NAMESPACE`] by the parser and is never
/// looked up here.
pub trait Namespaces {
    /// The namespace URI bound to a (non-empty) prefix, or `None` when it is not declared.
    fn namespace_uri(&self, prefix: &str) -> Option<String>;

    /// The default namespace for unprefixed element and type names (XSLT's
    /// `xpath-default-namespace`); `None`, the default, puts them in no namespace.
    fn default_element_namespace(&self) -> Option<String> {
        None
    }
}

/// A fixed set of prefix bindings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NamespaceMap {
    bindings: BTreeMap<String, String>,
    default_element: Option<String>,
}

impl NamespaceMap {
    /// An empty map: no prefix is bound (except `xml`, implicitly).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Binds `prefix` to `uri`, replacing any earlier binding of the prefix.
    #[must_use]
    pub fn bind(mut self, prefix: &str, uri: &str) -> Self {
        self.bindings.insert(prefix.to_owned(), uri.to_owned());
        self
    }

    /// Sets the default namespace for unprefixed element and type names.
    #[must_use]
    pub fn with_default_element_namespace(mut self, uri: &str) -> Self {
        self.default_element = Some(uri.to_owned());
        self
    }
}

impl Namespaces for NamespaceMap {
    fn namespace_uri(&self, prefix: &str) -> Option<String> {
        self.bindings.get(prefix).cloned()
    }

    fn default_element_namespace(&self) -> Option<String> {
        self.default_element.clone()
    }
}

/// `NameStartChar` of XML 1.0 (fifth edition) production \[4\], without `:`.
#[must_use]
pub fn is_name_start_char(c: char) -> bool {
    matches!(c,
        'A'..='Z' | '_' | 'a'..='z'
        | '\u{C0}'..='\u{D6}' | '\u{D8}'..='\u{F6}' | '\u{F8}'..='\u{2FF}'
        | '\u{370}'..='\u{37D}' | '\u{37F}'..='\u{1FFF}' | '\u{200C}'..='\u{200D}'
        | '\u{2070}'..='\u{218F}' | '\u{2C00}'..='\u{2FEF}' | '\u{3001}'..='\u{D7FF}'
        | '\u{F900}'..='\u{FDCF}' | '\u{FDF0}'..='\u{FFFD}' | '\u{10000}'..='\u{EFFFF}')
}

/// `NameChar` of XML 1.0 (fifth edition) production \[4a\], without `:`.
#[must_use]
pub fn is_name_char(c: char) -> bool {
    is_name_start_char(c)
        || matches!(c,
            '-' | '.' | '0'..='9' | '\u{B7}' | '\u{300}'..='\u{36F}' | '\u{203F}'..='\u{2040}')
}

/// Whether `s` is an `NCName` (a name without a colon).
#[must_use]
pub fn is_ncname(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(is_name_start_char) && chars.all(is_name_char)
}

/// The whitespace of XML and XPath: space, tab, carriage return and line feed.
#[must_use]
pub fn is_xml_whitespace(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\r' | '\n')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ncnames() {
        for ok in [
            "a",
            "_x",
            "a-b",
            "a.b",
            "schematron-get-full-path",
            "é1",
            "M11",
        ] {
            assert!(is_ncname(ok), "{ok}");
        }
        for bad in ["", "1a", "-a", ".a", "a:b", "a b", "*"] {
            assert!(!is_ncname(bad), "{bad}");
        }
    }

    #[test]
    fn qname_display_and_expanded_equality() {
        let a = QName {
            prefix: Some("p".into()),
            namespace: Some("urn:x".into()),
            local: "n".into(),
        };
        let b = QName {
            prefix: Some("q".into()),
            ..a.clone()
        };
        assert_eq!(a.to_string(), "p:n");
        assert_eq!(QName::local("n").to_string(), "n");
        assert!(a.same_expanded_name(&b));
        assert_ne!(a, b);
    }
}
