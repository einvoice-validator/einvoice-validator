//! What Saxon-HE 12.10 answers to `element-available()` and `system-property()`, the
//! values its `use-when` and its stylesheets read: taken from Saxon itself (the
//! `availability` probe of `einvoice-xslt-exec`).

use crate::load::XSL;

/// The elements of the XSLT namespace for which Saxon-HE 12.10's `element-available()`
/// is true: its instructions, and its declarations too (XSLT 3.0 limits the function to
/// instructions; Saxon does not).
const XSLT_ELEMENTS: &[&str] = &[
    "accept",
    "accumulator",
    "analyze-string",
    "apply-imports",
    "apply-templates",
    "assert",
    "attribute",
    "attribute-set",
    "break",
    "call-template",
    "catch",
    "character-map",
    "choose",
    "comment",
    "context-item",
    "copy",
    "copy-of",
    "decimal-format",
    "document",
    "element",
    "evaluate",
    "expose",
    "fallback",
    "for-each",
    "for-each-group",
    "fork",
    "function",
    "global-context-item",
    "if",
    "import",
    "include",
    "iterate",
    "key",
    "map",
    "map-entry",
    "matching-substring",
    "merge",
    "merge-action",
    "merge-key",
    "merge-source",
    "message",
    "mode",
    "namespace",
    "namespace-alias",
    "next-iteration",
    "next-match",
    "number",
    "on-empty",
    "on-non-empty",
    "otherwise",
    "output",
    "output-character",
    "override",
    "package",
    "param",
    "perform-sort",
    "preserve-space",
    "processing-instruction",
    "result-document",
    "sequence",
    "sort",
    "source-document",
    "strip-space",
    "stylesheet",
    "switch",
    "template",
    "text",
    "try",
    "use-package",
    "value-of",
    "variable",
    "when",
    "where-populated",
    "with-param",
];

/// Names in the XSLT namespace for which Saxon-HE 12.10's `element-available()` is
/// false.
const XSLT_NOT_ELEMENTS: &[&str] = &["array", "array-member", "import-schema", "record", "select"];

/// Saxon-HE 12.10's `element-available()` for the element `{namespace}local`: `None`
/// when this crate does not know Saxon's answer (a name of the XSLT namespace that is
/// in neither of its lists, or a name in Saxon's namespace, where Saxon has extension
/// instructions), so that the caller refuses rather than guesses. An element in any
/// other namespace is not available: Saxon-HE has no extension instructions there.
#[must_use]
pub fn element_available(namespace: Option<&str>, local: &str) -> Option<bool> {
    match namespace {
        Some(XSL) => {
            if XSLT_ELEMENTS.contains(&local) {
                Some(true)
            } else if XSLT_NOT_ELEMENTS.contains(&local) {
                Some(false)
            } else {
                None
            }
        }
        Some("http://saxon.sf.net/") => None,
        _ => Some(false),
    }
}

/// Saxon-HE 12.10's `system-property()` for the property `{namespace}local`: the
/// properties of the XSLT namespace it defines, and the empty string for any other.
#[must_use]
pub fn system_property(namespace: Option<&str>, local: &str) -> &'static str {
    if namespace != Some(XSL) {
        return "";
    }
    match local {
        "version" => "3.0",
        "vendor" => "Saxonica",
        "vendor-url" => "http://www.saxonica.com/",
        "product-name" => "SAXON",
        "product-version" => "HE 12.10",
        "is-schema-aware" | "supports-streaming" => "no",
        "supports-serialization"
        | "supports-backwards-compatibility"
        | "supports-namespace-axis"
        | "supports-dynamic-evaluation"
        | "supports-higher-order-functions" => "yes",
        "xpath-version" => "3.1",
        "xsd-version" => "1.1",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lists_are_sorted_and_disjoint() {
        assert!(XSLT_ELEMENTS.is_sorted());
        assert!(XSLT_NOT_ELEMENTS.is_sorted());
        assert!(XSLT_NOT_ELEMENTS.iter().all(|n| !XSLT_ELEMENTS.contains(n)));
    }

    #[test]
    fn answers() {
        assert_eq!(element_available(Some(XSL), "namespace"), Some(true));
        assert_eq!(element_available(Some(XSL), "import-schema"), Some(false));
        assert_eq!(element_available(Some(XSL), "bogus"), None);
        assert_eq!(
            element_available(Some("http://exslt.org/common"), "document"),
            Some(false)
        );
        assert_eq!(
            element_available(Some("http://saxon.sf.net/"), "doctype"),
            None
        );
        assert_eq!(system_property(Some(XSL), "vendor"), "Saxonica");
        assert_eq!(system_property(Some(XSL), "other"), "");
        assert_eq!(system_property(None, "vendor"), "");
    }
}
