//! The function library: exactly the functions the official rule sets call, with the
//! arities the specifications define for them (collation arguments excepted), resolved
//! when an expression is compiled.

use einvoice_xpath::{FN_NAMESPACE, QName, XS_NAMESPACE};

use crate::error::{ErrorCode, Result, err};

/// A built-in function.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Builtin {
    // XPath 1.0 core library.
    Not,
    True,
    False,
    Boolean,
    Count,
    Sum,
    NormalizeSpace,
    StringLength,
    String,
    Number,
    Contains,
    StartsWith,
    SubstringBefore,
    SubstringAfter,
    Substring,
    Concat,
    Translate,
    Round,
    Name,
    LocalName,
    NamespaceUri,
    Last,
    Position,
    // New in XPath 2.0.
    Exists,
    UpperCase,
    EndsWith,
    Abs,
    DocumentUri,
    Root,
    BaseUri,
    Matches,
    Replace,
    Tokenize,
    StringToCodepoints,
    CodepointsToString,
    StringJoin,
    Reverse,
    CurrentDateTime,
    DistinctValues,
    // XSLT 2.0.
    Current,
    Document,
    SystemProperty,
    // Constructor functions.
    XsDecimal,
    XsDate,
    XsInteger,
    XsString,
}

/// What a function returns, for the static analysis of the compiler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Returns {
    /// One `xs:boolean`.
    Boolean,
    /// Atomic values that are never numeric (strings, URIs, dates).
    NonNumeric,
    /// Atomic values that may be numeric.
    Atomic,
    /// Nodes.
    Nodes,
    /// Anything.
    Items,
}

/// Name, function, minimum and maximum arity (`usize::MAX`: no maximum), whether it is an
/// XSLT function.
const FN_TABLE: &[(&str, Builtin, usize, usize, bool)] = &[
    ("not", Builtin::Not, 1, 1, false),
    ("true", Builtin::True, 0, 0, false),
    ("false", Builtin::False, 0, 0, false),
    ("boolean", Builtin::Boolean, 1, 1, false),
    ("count", Builtin::Count, 1, 1, false),
    ("sum", Builtin::Sum, 1, 1, false),
    ("normalize-space", Builtin::NormalizeSpace, 0, 1, false),
    ("string-length", Builtin::StringLength, 0, 1, false),
    ("string", Builtin::String, 0, 1, false),
    ("number", Builtin::Number, 0, 1, false),
    ("contains", Builtin::Contains, 2, 2, false),
    ("starts-with", Builtin::StartsWith, 2, 2, false),
    ("substring-before", Builtin::SubstringBefore, 2, 2, false),
    ("substring-after", Builtin::SubstringAfter, 2, 2, false),
    ("substring", Builtin::Substring, 2, 3, false),
    ("concat", Builtin::Concat, 2, usize::MAX, false),
    ("translate", Builtin::Translate, 3, 3, false),
    ("round", Builtin::Round, 1, 1, false),
    ("name", Builtin::Name, 0, 1, false),
    ("local-name", Builtin::LocalName, 0, 1, false),
    ("namespace-uri", Builtin::NamespaceUri, 0, 1, false),
    ("last", Builtin::Last, 0, 0, false),
    ("position", Builtin::Position, 0, 0, false),
    ("exists", Builtin::Exists, 1, 1, false),
    ("upper-case", Builtin::UpperCase, 1, 1, false),
    ("ends-with", Builtin::EndsWith, 2, 2, false),
    ("abs", Builtin::Abs, 1, 1, false),
    ("document-uri", Builtin::DocumentUri, 0, 1, false),
    ("root", Builtin::Root, 0, 1, false),
    ("base-uri", Builtin::BaseUri, 0, 1, false),
    ("matches", Builtin::Matches, 2, 3, false),
    ("replace", Builtin::Replace, 3, 4, false),
    ("tokenize", Builtin::Tokenize, 2, 3, false),
    (
        "string-to-codepoints",
        Builtin::StringToCodepoints,
        1,
        1,
        false,
    ),
    (
        "codepoints-to-string",
        Builtin::CodepointsToString,
        1,
        1,
        false,
    ),
    ("string-join", Builtin::StringJoin, 2, 2, false),
    ("reverse", Builtin::Reverse, 1, 1, false),
    ("current-dateTime", Builtin::CurrentDateTime, 0, 0, false),
    ("distinct-values", Builtin::DistinctValues, 1, 1, false),
    ("current", Builtin::Current, 0, 0, true),
    ("document", Builtin::Document, 1, 1, true),
    ("system-property", Builtin::SystemProperty, 1, 1, true),
];

const XS_TABLE: &[(&str, Builtin)] = &[
    ("decimal", Builtin::XsDecimal),
    ("date", Builtin::XsDate),
    ("integer", Builtin::XsInteger),
    ("string", Builtin::XsString),
];

/// Functions of the XPath 2.0 and XSLT 2.0 libraries that are not implemented: a call
/// is refused as unsupported rather than reported as unknown.
const KNOWN_UNSUPPORTED: &[&str] = &[
    "adjust-date-to-timezone",
    "adjust-dateTime-to-timezone",
    "adjust-time-to-timezone",
    "avg",
    "ceiling",
    "codepoint-equal",
    "collection",
    "compare",
    "current-date",
    "current-group",
    "current-grouping-key",
    "current-time",
    "data",
    "day-from-date",
    "day-from-dateTime",
    "days-from-duration",
    "deep-equal",
    "default-collation",
    "doc",
    "doc-available",
    "element-available",
    "element-with-id",
    "empty",
    "encode-for-uri",
    "error",
    "escape-html-uri",
    "exactly-one",
    "floor",
    "format-date",
    "format-dateTime",
    "format-number",
    "format-time",
    "function-available",
    "generate-id",
    "hours-from-dateTime",
    "hours-from-duration",
    "hours-from-time",
    "id",
    "idref",
    "implicit-timezone",
    "in-scope-prefixes",
    "index-of",
    "insert-before",
    "iri-to-uri",
    "key",
    "lang",
    "local-name-from-QName",
    "lower-case",
    "max",
    "min",
    "minutes-from-dateTime",
    "minutes-from-duration",
    "minutes-from-time",
    "month-from-date",
    "month-from-dateTime",
    "months-from-duration",
    "namespace-uri-for-prefix",
    "namespace-uri-from-QName",
    "nilled",
    "node-name",
    "normalize-unicode",
    "one-or-more",
    "prefix-from-QName",
    "QName",
    "regex-group",
    "remove",
    "resolve-QName",
    "resolve-uri",
    "round-half-to-even",
    "seconds-from-dateTime",
    "seconds-from-duration",
    "seconds-from-time",
    "static-base-uri",
    "subsequence",
    "timezone-from-date",
    "timezone-from-dateTime",
    "timezone-from-time",
    "trace",
    "type-available",
    "unordered",
    "unparsed-entity-public-id",
    "unparsed-entity-uri",
    "unparsed-text",
    "unparsed-text-available",
    "year-from-date",
    "year-from-dateTime",
    "years-from-duration",
    "zero-or-one",
];

/// Resolves a call to a built-in function.
///
/// # Errors
///
/// `XPST0017` for a name and arity that no function has (an XSLT function counts as
/// unknown outside XSLT), [`ErrorCode::Unsupported`] for a function of the
/// specifications that is not implemented, or an arity of one that is.
pub(crate) fn resolve(name: &QName, arity: usize, xslt: bool) -> Result<Option<Builtin>> {
    match name.namespace.as_deref() {
        Some(FN_NAMESPACE) => {
            if let Some(&(_, builtin, min, max, xslt_only)) =
                FN_TABLE.iter().find(|(local, ..)| *local == name.local)
            {
                if xslt_only && !xslt {
                    return err(
                        ErrorCode::Xpst0017,
                        format!("{name}() is an XSLT function, not available here"),
                    );
                }
                if (min..=max).contains(&arity) {
                    return Ok(Some(builtin));
                }
                return err(
                    ErrorCode::Unsupported,
                    format!("{name}() with {arity} arguments is not supported"),
                );
            }
            if KNOWN_UNSUPPORTED.contains(&name.local.as_str()) {
                return err(
                    ErrorCode::Unsupported,
                    format!("the function {name}() is not supported"),
                );
            }
            Ok(None)
        }
        Some(XS_NAMESPACE) => {
            if let Some(&(_, builtin)) = XS_TABLE.iter().find(|(local, _)| *local == name.local) {
                if arity == 1 {
                    return Ok(Some(builtin));
                }
                return err(
                    ErrorCode::Xpst0017,
                    format!("the constructor function {name}() takes one argument"),
                );
            }
            err(
                ErrorCode::Unsupported,
                format!("the constructor function {name}() is not supported"),
            )
        }
        _ => Ok(None),
    }
}

impl Builtin {
    /// What the function returns.
    pub(crate) const fn returns(self) -> Returns {
        match self {
            Self::Not
            | Self::True
            | Self::False
            | Self::Boolean
            | Self::Exists
            | Self::Contains
            | Self::StartsWith
            | Self::EndsWith
            | Self::Matches => Returns::Boolean,
            Self::NormalizeSpace
            | Self::String
            | Self::SubstringBefore
            | Self::SubstringAfter
            | Self::Substring
            | Self::Concat
            | Self::Translate
            | Self::UpperCase
            | Self::Name
            | Self::LocalName
            | Self::NamespaceUri
            | Self::DocumentUri
            | Self::BaseUri
            | Self::Replace
            | Self::Tokenize
            | Self::CodepointsToString
            | Self::StringJoin
            | Self::CurrentDateTime
            | Self::SystemProperty
            | Self::XsDate
            | Self::XsString => Returns::NonNumeric,
            Self::Count
            | Self::Sum
            | Self::StringLength
            | Self::Number
            | Self::Round
            | Self::Abs
            | Self::Last
            | Self::Position
            | Self::StringToCodepoints
            | Self::DistinctValues
            | Self::XsDecimal
            | Self::XsInteger => Returns::Atomic,
            Self::Root | Self::Document => Returns::Nodes,
            Self::Reverse | Self::Current => Returns::Items,
        }
    }

    /// Whether the function may return more than one item.
    pub(crate) const fn returns_many(self) -> bool {
        matches!(
            self,
            Self::Tokenize
                | Self::StringToCodepoints
                | Self::DistinctValues
                | Self::Reverse
                | Self::Document
        )
    }

    /// The name, for messages.
    pub(crate) fn name(self) -> &'static str {
        FN_TABLE
            .iter()
            .find(|(_, b, ..)| *b == self)
            .map(|(name, ..)| *name)
            .or_else(|| {
                XS_TABLE
                    .iter()
                    .find(|(_, b)| *b == self)
                    .map(|(name, _)| match *name {
                        "decimal" => "xs:decimal",
                        "date" => "xs:date",
                        "integer" => "xs:integer",
                        _ => "xs:string",
                    })
            })
            .unwrap_or("?")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fn_name(local: &str) -> QName {
        QName {
            prefix: None,
            namespace: Some(FN_NAMESPACE.to_owned()),
            local: local.to_owned(),
        }
    }

    #[test]
    fn resolution() {
        assert_eq!(
            resolve(&fn_name("concat"), 7, false).unwrap(),
            Some(Builtin::Concat)
        );
        assert_eq!(resolve(&fn_name("frobnicate"), 1, false).unwrap(), None);
        let unsupported = resolve(&fn_name("contains"), 3, false).unwrap_err();
        assert_eq!(unsupported.code(), ErrorCode::Unsupported);
        let xslt = resolve(&fn_name("current"), 0, false).unwrap_err();
        assert_eq!(xslt.code(), ErrorCode::Xpst0017);
        assert_eq!(
            resolve(&fn_name("current"), 0, true).unwrap(),
            Some(Builtin::Current)
        );
        assert_eq!(
            resolve(&fn_name("floor"), 1, false).unwrap_err().code(),
            ErrorCode::Unsupported
        );
        assert_eq!(Builtin::XsDecimal.name(), "xs:decimal");
        assert_eq!(Builtin::StringJoin.name(), "string-join");
    }
}
