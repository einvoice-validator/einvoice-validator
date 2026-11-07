//! Loader for the XSLT 2.0 subset that the official e-invoicing validation stylesheets use
//! (Schematron compiled to XSLT by the ISO skeleton or by `SchXslt`).
//!
//! [`load`] parses a stylesheet into a [`Stylesheet`]: templates with their match patterns,
//! modes and priorities, named templates, functions, global and local variables and
//! parameters, and the instructions of each sequence constructor, every one with its source
//! line. XPath expressions, patterns, sequence types and attribute value templates are
//! parsed with [`einvoice_xpath`]. Nothing is evaluated here.
//!
//! The subset is closed. Supported XSLT elements: `stylesheet`/`transform` (version 2.0),
//! `output`, `param`, `variable`, `template`, `function`, `apply-templates`, `with-param`,
//! `next-match`, `call-template`, `choose`/`when`/`otherwise`, `if`, `for-each`,
//! `sequence`, `value-of`, `attribute`, `comment`, `text`, `number` (`level` single or
//! multiple, `count`), literal result elements and top-level data elements. Each supports
//! only the attributes the official stylesheets use. Any other XSLT element, any other
//! attribute (including `xsl:` attributes on literal result elements and attributes in
//! other namespaces on XSLT elements, which XSLT would otherwise ignore) and any
//! unsupported attribute value is a [`LoadError`] naming it and its line.
//!
//! The stylesheet's whitespace-only text nodes are stripped as XSLT 2.0 section 4.2
//! prescribes, honouring `xml:space`.
//!
//! Each alternative of a template's pattern is a separate rule ([`Template::rules`]) with
//! its own priority: the explicit `priority`, or the default priority of the alternative
//! ([`default_priority`]). [`Stylesheet::rules`] lists the rules of a mode, including
//! `#all` templates.
//!
//! ```
//! let xslt = "<xsl:stylesheet version='2.0' xmlns:xsl='http://www.w3.org/1999/XSL/Transform'>
//!   <xsl:template match='a | b/c' mode='m'><xsl:apply-templates select='*' mode='#current'/></xsl:template>
//! </xsl:stylesheet>";
//! let stylesheet = einvoice_xslt::load(xslt)?;
//! let priorities: Vec<String> = stylesheet.templates[0].rules.iter().map(|r| r.priority.to_string()).collect();
//! assert_eq!(priorities, ["0", "0.5"]);
//! # Ok::<(), einvoice_xslt::LoadErrors>(())
//! ```

mod error;
mod ir;
mod load;
mod priority;

pub use error::{LoadError, LoadErrorKind, LoadErrors};
pub use ir::{
    ApplyMode, ApplyTemplates, Attribute, CallTemplate, Choose, Comment, DataElement, ForEach,
    Function, GlobalKind, GlobalVariable, If, Instruction, LiteralAttribute, LiteralElement, Mode,
    NamespaceBinding, NextMatch, Number, NumberLevel, Output, OutputMethod, Rule, Sequence,
    Standalone, Stylesheet, Template, TemplateModes, TemplateRule, Text, TextOrigin, ValueOf,
    Variable, VariableValue, When,
};
pub use priority::{Priority, default_priority};

/// The XSLT namespace.
pub const XSLT_NAMESPACE: &str = load::XSL;

/// Loads a stylesheet from its XML text.
///
/// # Errors
///
/// Every problem found: XML that is not well-formed, XSLT outside the supported subset,
/// static errors, and XPath that does not parse. All are reported, not only the first.
pub fn load(text: &str) -> Result<Stylesheet, LoadErrors> {
    load::load(text)
}
