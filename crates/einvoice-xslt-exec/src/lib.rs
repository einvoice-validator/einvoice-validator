//! Executor of the XSLT 2.0 subset that the official e-invoicing validation stylesheets
//! use (Schematron compiled to XSLT), with the behaviour of the reference engine,
//! Saxon-HE 9.9.1-8: same template rules chosen, same result.
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
//!   the XSLT 3.0 `(A | B)[p]` form), modes (named, `#default`, `#all`, `#current`),
//!   explicit and default priorities (each alternative of a pattern without explicit
//!   priority is a rule of its own), the rule chosen, and the next one for
//!   `xsl:next-match`, exactly as Saxon chooses them (its rule chains, ties going to
//!   the last declared rule with an XTDE0540 warning), the built-in rules for every node
//!   kind, and for atomic values, which Saxon, an XSLT 3.0 processor, lets
//!   `xsl:apply-templates` select and writes as text;
//! - `xsl:apply-templates`, `xsl:call-template`, `xsl:next-match` with `xsl:with-param`;
//!   template, function and global parameters and variables, with `as` conversion;
//!   `xsl:function`;
//! - `xsl:choose`, `xsl:if`, `xsl:for-each`, `xsl:sequence`, `xsl:value-of`
//!   (`separator`), `xsl:attribute`, `xsl:comment`, `xsl:text`, literal text,
//!   `xsl:number` (`level` single or multiple, `count`, format `1`), literal result
//!   elements with their namespaces;
//! - temporary trees (variables whose value is a sequence constructor), simple content
//!   and complex content construction as XSLT 2.0 section 5.7 defines them;
//! - `document()` through a [`DocumentResolver`] (the engine reads no file), relative
//!   URIs resolved against the base URI given by [`Executable::with_base_uri`], and the
//!   source document by its own URI, parsed again as Saxon does.
//!
//! One known difference: Saxon simplifies constant predicates before computing the
//! default priority of a pattern (`x[true()]` gets the priority of `x`, 0, where XSLT
//! gives 0.5); this crate uses the pattern as written. The official stylesheets have
//! no constant predicate in a pattern.
//!
//! Refused when the stylesheet is prepared, rather than run differently from the
//! reference engine: instructions that construct nodes (or call templates) inside an
//! `xsl:function` or a variable with `as`, a parenthesized pattern whose predicate
//! may depend on the position, `system-property()` with a prefixed name. Everything
//! outside the subset is already refused by [`einvoice_xslt::load`].
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
pub use run::{DocumentResolver, TransformOptions, Transformation};
