//! Parser and syntax tree for the XPath 2.0 subset that the official e-invoicing validation
//! stylesheets use, and for the XSLT pieces built on it: match patterns, sequence types
//! (`as` attributes) and attribute value templates.
//!
//! Only syntax is handled here: function names are resolved to namespaces but not to
//! functions, and nothing is evaluated. The subset is closed: a construct outside it is
//! refused with an [`ErrorKind::Unsupported`] error naming it, never parsed as something
//! else. Supported:
//!
//! - path expressions on the axes `child`, `attribute`, `self`, `parent`, `ancestor`,
//!   `ancestor-or-self`, `descendant-or-self` (and `//`), `preceding-sibling`, `preceding`,
//!   `following-sibling`; name tests, wildcards and the kind tests `node()`, `text()`,
//!   `comment()`, `processing-instruction()`, `element(name?)`, `attribute(name?)`;
//!   predicates; filter expressions as steps;
//! - `for`, `some`, `every` (one or more bindings), `if then else`, `to`, `instance of`,
//!   `cast as`, `castable as`, general and value comparisons, `or`, `and`, `+ - * div idiv mod`,
//!   unary `-`, `|`, sequences, literals (numeric literals keep their kind and text),
//!   variable references, `.`, function calls;
//! - patterns of XSLT 2.0 without `id()`/`key()`, plus the XSLT 3.0 forms `root()` and
//!   `(A | B)[p]`.
//!
//! Refused: `intersect`, `except`, the `union` keyword, `treat as`, node
//! comparisons, unary `+`, XPath comments, the `descendant`, `following` and `namespace`
//! axes, `document-node()`, `schema-element()`, `schema-attribute()`, typed `element()` and
//! `attribute()` tests, `item()`, `empty-sequence()`, atomic types other than `xs:string`,
//! `xs:anyURI`, `xs:boolean`, `xs:integer`, `xs:decimal` and `xs:date`, and every XPath 3.0
//! addition except the zero-argument `document-uri()`, which is an ordinary function call
//! here.
//!
//! ```
//! use einvoice_xpath::{parse_expr, NamespaceMap, XS_NAMESPACE};
//!
//! let ns = NamespaceMap::new().bind("xs", XS_NAMESPACE);
//! let expr = parse_expr("xs:decimal(Amount) * 2 = 10", &ns)?;
//! assert_eq!(expr.to_string(), "xs:decimal(Amount) * 2 = 10");
//! assert!(parse_expr("a intersect b", &ns).is_err());
//! # Ok::<(), einvoice_xpath::Error>(())
//! ```

mod ast;
mod error;
mod lexer;
mod names;
mod parser;
mod print;

pub use ast::{
    ArithmeticOp, AtomicKind, AtomicType, Avt, AvtPart, Axis, AxisStep, Binding, CompareKind,
    ComparisonOp, Expr, ItemType, KindTest, Literal, NameOrWildcard, NameTest, NodeTest,
    Occurrence, PathExpr, PathPattern, PathStart, Pattern, PatternAlternative, PatternAnchor,
    PatternAxis, PatternLink, PatternStep, Quantifier, SequenceType, SingleType, Step,
};
pub use error::{Error, ErrorKind, Result};
pub use names::{
    FN_NAMESPACE, NamespaceMap, Namespaces, QName, XML_NAMESPACE, XS_NAMESPACE, is_name_char,
    is_name_start_char, is_ncname, is_xml_whitespace,
};

use parser::Parser;

/// Parses an XPath expression (XPath 2.0 production \[1\] `XPath ::= Expr`).
///
/// # Errors
///
/// A syntax error, a construct outside the supported subset, or an undeclared prefix.
pub fn parse_expr(src: &str, namespaces: &dyn Namespaces) -> Result<Expr> {
    let mut parser = Parser::new(src, 0, namespaces);
    let expr = parser.expr()?;
    parser.expect_end()?;
    Ok(expr)
}

/// Parses an XSLT match pattern.
///
/// # Errors
///
/// A syntax error, a construct outside the supported subset, or an undeclared prefix.
pub fn parse_pattern(src: &str, namespaces: &dyn Namespaces) -> Result<Pattern> {
    let mut parser = Parser::new(src, 0, namespaces);
    let pattern = parser.pattern()?;
    parser.expect_end()?;
    Ok(pattern)
}

/// Parses a sequence type, as in an XSLT `as` attribute.
///
/// # Errors
///
/// A syntax error, an unsupported or unknown type, or an undeclared prefix.
pub fn parse_sequence_type(src: &str, namespaces: &dyn Namespaces) -> Result<SequenceType> {
    let mut parser = Parser::new(src, 0, namespaces);
    let sequence_type = parser.sequence_type()?;
    parser.expect_end()?;
    Ok(sequence_type)
}

/// Parses an attribute value template.
///
/// # Errors
///
/// A misplaced `}`, an empty or unterminated `{...}`, or an error in an expression.
pub fn parse_avt(src: &str, namespaces: &dyn Namespaces) -> Result<Avt> {
    parser::parse_avt(src, namespaces)
}
