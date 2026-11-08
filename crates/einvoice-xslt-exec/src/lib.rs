//! Executor of the XSLT 2.0 subset that the official e-invoicing validation stylesheets
//! use (Schematron compiled to XSLT), with the behaviour of the reference engine,
//! Saxon-HE 12.10: same template rules chosen, same result.
//!
//! A stylesheet loaded by [`einvoice_xslt::load`] is prepared once into an
//! [`Executable`], which then runs on any number of documents with
//! [`Executable::transform`]. Preparing compiles every XPath expression with
//! [`einvoice_xpath_eval`], resolves variables, functions, named templates and modes, and
//! indexes the template rules of each mode by the kind and name of the nodes they can
//! match. A transformation parses the document, applies templates to its document node in
//! the default mode and writes the result straight to XML as it is produced (the result
//! tree is never held in memory): for a compiled Schematron, the SVRL report.
//!
//! Supported, as XSLT 2.0 and Saxon define them:
//!
//! - template rules: match patterns (`/`, `root()`, path patterns with predicates, `//`,
//!   the XSLT 3.0 parenthesized patterns as Saxon-HE 12.10 reads them: `(A | B)` alone is
//!   `A | B`, a path ending with `(A | B)` is a union, `(A | B)[p]/c` and
//!   `(a | b)/(c | d)/e` match as written, see [`einvoice_xpath::parse_pattern`]),
//!   modes (named, `#default`, `#all`, `#current`),
//!   explicit and default priorities (each alternative of a pattern without explicit
//!   priority is a rule of its own, a pattern with one is a single rule), the rule
//!   chosen, and the next one for
//!   `xsl:next-match`, exactly as Saxon chooses them (its rule chains, ties going to
//!   the last declared rule with an XTDE0540 warning), the built-in rules for every node
//!   kind, and for atomic values, which Saxon, an XSLT 3.0 processor, lets
//!   `xsl:apply-templates` select and writes as text;
//! - dynamic errors raised while matching a pattern (a template rule's, or the `count`
//!   pattern of `xsl:number`), in its predicates or in a global variable or function
//!   they use: as in Saxon, the node does not match and the error is a warning, except
//!   a circular global variable (`XTDE0640`), which ends the transformation; a global
//!   variable whose evaluation failed is evaluated again when next referred to;
//! - `xsl:apply-templates`, `xsl:call-template`, `xsl:next-match` with `xsl:with-param`;
//!   template, function and global parameters and variables, with `as` conversion;
//!   `xsl:function`; when each value is computed, as Saxon-HE 12.10 computes it
//!   (lazily, inlined, or as its `LearningEvaluator` learns, see the `run` module), which
//!   decides which dynamic errors are raised;
//! - `xsl:choose`, `xsl:if`, `xsl:for-each`, `xsl:sequence`, `xsl:value-of`
//!   (`separator`), `xsl:attribute`, `xsl:comment`, `xsl:text`, literal text,
//!   `xsl:number` (`level` single or multiple, `count`, format `1`), literal result
//!   elements with their namespaces;
//! - temporary trees (variables whose value is a sequence constructor), simple content
//!   and complex content construction as XSLT 2.0 section 5.7 defines them, and
//!   sequences (an `xsl:function` body, a variable or parameter with `as`): an element,
//!   attribute, text node (even of zero length) or comment constructed there is a new
//!   parentless node, as the stylesheets compiled by SchXslt construct their reports;
//!   copied elements keep their namespaces;
//! - `document()` through a [`DocumentResolver`] (the engine reads no file), relative
//!   URIs resolved against the base URI given by [`Executable::with_base_uri`], and the
//!   source document by its own URI, parsed again as Saxon does; a loaded document has
//!   that URI as document URI, the source document none (`document-uri(/)` is empty, as
//!   in Saxon-HE 12.10).
//!
//! Known differences: Saxon simplifies constant predicates before computing the
//! default priority of a pattern (`x[true()]` gets the priority of `x`, 0, where XSLT
//! gives 0.5); this crate uses the pattern as written. The official stylesheets have
//! no constant predicate in a pattern. Error messages are worded differently from
//! Saxon's (the codes are the same), and some of Saxon's quote the values involved
//! where this crate's do not: the warnings for errors in patterns are then fewer
//! distinct messages, for the same patterns and codes. Saxon keeps what its
//! `LearningEvaluator` learns in the compiled stylesheet, over every document it
//! transforms with it; this crate starts afresh with each transformation, as Saxon does
//! with a stylesheet compiled for one document. The elements of a parsed document have
//! no namespace nodes here: copied to the result, they get only the namespaces their
//! names use.
//!
//! Limits ([`TransformOptions`]): the source document and the documents `document()`
//! loads are refused when an element is nested deeper than `max_depth`
//! ([`einvoice_xpath_eval::DEFAULT_MAX_DEPTH`], 256), and a transformation ends when its
//! template rules, built-in rules, named templates and functions are invoked one inside
//! another more than `max_nesting` deep ([`DEFAULT_MAX_NESTING`]) or use more than
//! `max_stack` bytes of stack ([`DEFAULT_MAX_STACK`]), lazy values computed one inside
//! another included: a `LIMIT` error
//! ([`TransformError::limit`]) instead of a stack overflow, which would abort the process.
//! Walking a tree without template rules (copying a node, the path of a node in a
//! warning) uses no recursion. With the defaults, the official stylesheets run on a
//! document 256 deep on a thread with the default 2 MiB stack, debug builds included.
//!
//! Refused when the stylesheet is prepared, rather than run differently from the
//! reference engine: a parenthesized pattern whose predicate may depend on the position.
//! Everything outside the subset is already refused by [`einvoice_xslt::load`].
//!
//! ```
//! let xslt = r#"<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
//!   <xsl:output omit-xml-declaration="yes"/>
//!   <xsl:template match="/"><total><xsl:value-of select="sum(//l/@a)"/></total></xsl:template>
//! </xsl:stylesheet>"#;
//! let executable = einvoice_xslt_exec::Executable::new(&einvoice_xslt::load(xslt)?)?;
//! let result = executable.transform("<i><l a='1.5'/><l a='2'/></i>", None)?;
//! assert_eq!(result.output, "<total>3.5</total>");
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod error;
mod pattern;
mod prepare;
mod run;
mod sink;

pub use error::{PrepareError, PrepareErrors, TransformError};
pub use prepare::Executable;
pub use run::{
    DEFAULT_MAX_NESTING, DEFAULT_MAX_STACK, DocumentResolver, TransformOptions, Transformation,
    max_nesting_for_depth,
};
