//! Loader for the XSLT 2.0 subset that the official e-invoicing validation stylesheets use
//! (Schematron compiled to XSLT by the ISO skeleton or by `SchXslt`), and that the ISO
//! Schematron skeleton itself uses to compile Schematron.
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
//! `sequence`, `value-of`, `attribute` (with `namespace`), `comment`, `text`, `number`
//! (`level` single or multiple, `count`), `copy`, `copy-of`, `element`, `namespace`,
//! `processing-instruction`, `message` (with `terminate`), `import` (with
//! [`load_with_imports`]) and `namespace-alias`, literal result elements and top-level
//! data elements; `use-when` is evaluated statically, as Saxon-HE 12.10 does
//! ([`element_available`], [`system_property`]), and the namespaces of
//! `extension-element-prefixes` are not copied to the result (an extension instruction
//! is refused). Each supports only the attributes the official stylesheets and the
//! skeleton use. Any other XSLT element, any other
//! attribute (including `xsl:` attributes on literal result elements and attributes in
//! other namespaces on XSLT elements, which XSLT would otherwise ignore) and any
//! unsupported attribute value is a [`LoadError`] naming it and its line. So are
//! instructions nested deeper than [`MAX_NESTING`] ([`LoadErrorKind::TooDeep`]) and
//! expressions nested deeper than [`einvoice_xpath::MAX_DEPTH`]: the stack needed to load,
//! prepare and run a stylesheet stays bounded, whatever its text.
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

mod available;
mod error;
mod ir;
mod load;
mod priority;

pub use available::{element_available, system_property};
pub use error::{LoadError, LoadErrorKind, LoadErrors};
pub use ir::{
    ApplyMode, ApplyTemplates, Attribute, CallTemplate, Choose, Comment, Copy, CopyOf, DataElement,
    Element, ForEach, Function, GlobalKind, GlobalVariable, If, Instruction, LiteralAttribute,
    LiteralElement, Message, Mode, Module, NamespaceBinding, NamespaceNode, NextMatch, Number,
    NumberLevel, Output, OutputMethod, ProcessingInstruction, Rule, Sequence, Standalone,
    Stylesheet, Template, TemplateModes, TemplateRule, Text, TextOrigin, ValueOf, Variable,
    VariableValue, When,
};
pub use load::resolve_uri;
pub use priority::{Priority, default_priority};

/// The XSLT namespace.
pub const XSLT_NAMESPACE: &str = load::XSL;

/// The maximum nesting of sequence constructors: deeper, a stylesheet is refused with a
/// [`LoadErrorKind::TooDeep`] error.
///
/// Loading, preparing and running a stylesheet recurse over the nesting of its
/// instructions, so the call stack they need grows with it: without a bound, a short
/// stylesheet nested deeply enough would overflow the stack of its thread, which aborts
/// the whole process. The body of a template, a function or a global variable is at
/// level 1; the content of an instruction or of a literal result element (`xsl:if`,
/// `xsl:when`, `xsl:otherwise`, `xsl:for-each`, `xsl:variable`, `xsl:with-param`,
/// `xsl:value-of`, `xsl:attribute`, `xsl:comment`, an element) is one level deeper than
/// that instruction. The XPath expressions in the stylesheet are bounded by
/// [`einvoice_xpath::MAX_DEPTH`].
///
/// 64: the official stylesheets nest their instructions 9 deep at most (CEN, Peppol,
/// Factur-X, XRechnung and the French rules).
pub const MAX_NESTING: u32 = 64;

/// Gives the loader the text of the stylesheet modules that `xsl:import` names, by
/// absolute URI ([`load_with_imports`]). The loader reads no file itself.
pub trait ModuleResolver {
    /// The text of the module at `uri`, or `None` when it cannot be read.
    fn resolve(&self, uri: &str) -> Option<String>;
}

/// Loads a stylesheet from its XML text. It must not import other modules: `xsl:import`
/// is refused as unsupported here (see [`load_with_imports`]).
///
/// # Errors
///
/// Every problem found: XML that is not well-formed, XSLT outside the supported subset,
/// static errors, and XPath that does not parse. All are reported, not only the first.
pub fn load(text: &str) -> Result<Stylesheet, LoadErrors> {
    load::load(text)
}

/// Loads a stylesheet whose principal module, at `uri`, has the text `text`, and the
/// modules it imports with `xsl:import`, whose `href` is resolved against the URI of the
/// importing module ([`resolve_uri`]) and read through `resolver`. The declarations of
/// a module have a lower import precedence than those of the module that imports it
/// (XSLT 2.0 section 3.10.3): a global variable, a function or a named template is
/// overridden by one of the same name and a higher precedence, and template rules are
/// ranked by precedence before priority. A module may not import itself, directly or
/// not, nor be imported twice; `xsl:include` is not supported.
///
/// # Errors
///
/// Those of [`load`] in each module, each error naming its module, and a module that
/// cannot be read.
pub fn load_with_imports(
    text: &str,
    uri: &str,
    resolver: &dyn ModuleResolver,
) -> Result<Stylesheet, LoadErrors> {
    load::load_with_imports(text, uri, resolver)
}
