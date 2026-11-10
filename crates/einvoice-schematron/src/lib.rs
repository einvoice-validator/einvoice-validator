//! Schematron compiled to XSLT exactly as ph-schematron 10.3.1 writes it, with the ISO
//! Schematron skeleton (`ph-schematron-maven-plugin:convert`, engine `iso-schematron`),
//! in pure Rust: the three XSLT steps of that compiler run by this project's engine, and
//! the last two stages, which are not XSLT.
//!
//! # The pipeline of ph-schematron
//!
//! `Schematron2XSLTMojo` (ph-schematron-maven-plugin 10.3.1) compiles each Schematron
//! file with `SchematronProviderXSLTFromSCH.createSchematronXSLT` (ph-schematron-isosch),
//! with no phase, no language code, no parameters and no URI resolver. There is no Java
//! preprocessing of the schema on that path (`PSPreprocessor` belongs to the pure-Java
//! engines): includes, abstract patterns, abstract rules, phases and `let` are all left
//! to the three XSLT stylesheets of the ISO Schematron skeleton of 2010 as packaged in
//! ph-schematron-isosch (`external/schematron/20100710-xslt2/`), run by Saxon-HE 12.10:
//!
//! 1. `iso_dsdl_include.xsl` on the Schematron file (its path as system ID): includes
//!    and `extends href`;
//! 2. `iso_abstract_expand.xsl` on the result of step 1: abstract patterns;
//! 3. `iso_svrl_for_xslt2.xsl`, which imports `iso_schematron_skeleton_for_saxon.xsl`, on
//!    the result of step 2: the compiled stylesheet. No parameter is set (`phase` and
//!    `langCode` are not set either), so each stylesheet parameter has its default.
//!
//! Each step writes into a new DOM document of the JDK (`DOMResult`), read by the next
//! step as a `DOMSource` with the system ID of the Schematron file. The DOM document of
//! step 3 is then written by ph-commons' `XMLWriter` with a namespace context taken from
//! its document element.
//!
//! One input makes the output depend on the run: an instance of an abstract pattern
//! without `id` gets one from `generate-id()` in step 2, which Saxon makes from a number
//! its configuration gives each document, so that it changes with what the same JVM
//! compiled before (`w231ab1b5Range` or `w291ab1b5Range` for the same file). The
//! official rule sets have no such instance.
//!
//! # What this crate does
//!
//! [`Skeleton`] runs the three steps, on this project's XSLT engine (`einvoice-xslt`,
//! `einvoice-xslt-exec`), from the skeleton's files as ph-schematron-isosch 10.3.1
//! packages them (the caller gives them, checked by SHA-256: [`SKELETON_FILES`]), and
//! passes the result of each step to the next as ph-schematron does (see [`pipeline`]):
//! [`Skeleton::compile`] gives the compiled stylesheet, byte for byte the one
//! ph-schematron-maven-plugin writes, without Java.
//!
//! [`Document::from_result`] builds the DOM document of step 3 from its result tree
//! written as XML, as Saxon's `DOMWriter` builds it (see [`build`](mod@crate::build) and
//! [`dom`]); [`write_stylesheet`] writes it as `Schematron2XSLTMojo` does, byte for byte
//! (see [`write`](mod@crate::write)). [`compile_result`] does both, from a result tree
//! that ran elsewhere.
//!
//! ```
//! let result = r#"<xsl:stylesheet xmlns:xsl="http://www.w3.org/1999/XSL/Transform" version="2.0"><!--A-->
//! <xsl:template match="/"><svrl:schematron-output xmlns:svrl="http://purl.oclc.org/dsdl/svrl" title=""/></xsl:template></xsl:stylesheet>"#;
//! assert_eq!(
//!     einvoice_schematron::compile_result(result)?,
//!     "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"no\"?>\n\
//!      <xsl:stylesheet xmlns:svrl=\"http://purl.oclc.org/dsdl/svrl\" \
//!      xmlns:xsl=\"http://www.w3.org/1999/XSL/Transform\" version=\"2.0\">\n  \
//!      <!--A-->\n<xsl:template match=\"/\">\n    \
//!      <svrl:schematron-output title=\"\" />\n  </xsl:template>\n</xsl:stylesheet>\n"
//! );
//! # Ok::<(), einvoice_schematron::Error>(())
//! ```
//!
//! The writer uses a line feed as line separator, as ph-commons does on Linux and macOS
//! (it uses the platform's: on Windows, the lines it adds itself end with a carriage
//! return and a line feed).

pub mod build;
pub mod dom;
pub mod pipeline;
pub mod write;

mod tables;

pub use dom::{Attribute, Document, Element, Node};
pub use pipeline::{Compilation, SKELETON_DIR, SKELETON_FILES, Skeleton, Sources};

/// The deepest element nesting accepted, the document element at depth 1: deeper, the
/// result tree is refused ([`Error::Xml`] with a depth limit) and a document is not
/// written ([`Error::TooDeep`]), so that the call stack stays bounded.
pub const MAX_DEPTH: usize = 256;

/// Why a document cannot be built or written.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// The result tree is not well-formed XML, or its elements are nested deeper than
    /// [`MAX_DEPTH`].
    Xml(einvoice_xml::Error),
    /// Xerces, the JDK's DOM, refuses a name: `code` is the code of its `DOMException`
    /// (`INVALID_CHARACTER_ERR`, `NAMESPACE_ERR`), `name` the qualified name or the
    /// target of a processing instruction.
    Dom {
        /// The code of Xerces' `DOMException`.
        code: &'static str,
        /// The name refused.
        name: String,
    },
    /// A comment contains `<!--` or `-->`, which ph-commons refuses to write.
    NestedComment(String),
    /// A processing instruction (its target) contains `?>`, which ph-commons refuses to
    /// write.
    PiEndMarker(String),
    /// The document has elements nested deeper than [`MAX_DEPTH`].
    TooDeep,
    /// The Schematron file is not well-formed XML, or has a document type declaration.
    Schematron(String),
    /// A step of the skeleton (1, 2 or 3) raised a dynamic error, `xsl:message` with
    /// `terminate="yes"` included (`XTMM9000`).
    Step {
        /// The step.
        step: u8,
        /// The error.
        error: einvoice_xslt_exec::TransformError,
    },
    /// A file of the ISO Schematron skeleton is missing or is not ph-schematron's, or the
    /// skeleton does not load or prepare (a bug of this project).
    Skeleton(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Xml(e) => write!(f, "result tree: {e}"),
            Self::Dom { code, name } => write!(f, "{code}: the DOM refuses the name {name:?}"),
            Self::NestedComment(c) => write!(f, "comment containing <!-- or -->: {c:?}"),
            Self::PiEndMarker(t) => write!(f, "processing instruction {t:?} containing ?>"),
            Self::TooDeep => write!(f, "elements nested deeper than {MAX_DEPTH}"),
            Self::Schematron(message) => write!(f, "the Schematron file: {message}"),
            Self::Step { step, error } => {
                write!(f, "step {step} of the ISO Schematron skeleton: {error}")
            }
            Self::Skeleton(message) => write!(f, "the ISO Schematron skeleton: {message}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Xml(e) => Some(e),
            Self::Step { error, .. } => Some(error),
            _ => None,
        }
    }
}

impl From<einvoice_xml::Error> for Error {
    fn from(e: einvoice_xml::Error) -> Self {
        Self::Xml(e)
    }
}

impl Document {
    /// The DOM document that Saxon's `DOMWriter` builds from a result tree (the rules are
    /// in [`build`](mod@crate::build)), the tree read from its XML: the XML that Saxon or
    /// any serializer writes of it (method `xml`, no indentation), the XML declaration and
    /// the whitespace around the document element ignored.
    ///
    /// # Errors
    ///
    /// The XML is not well-formed, or elements are nested deeper than [`MAX_DEPTH`]
    /// ([`Error::Xml`]); Xerces refuses a name ([`Error::Dom`]).
    pub fn from_result(xml: &str) -> Result<Self, Error> {
        build::from_result(xml)
    }
}

/// Writes a DOM document as `Schematron2XSLTMojo` writes a compiled stylesheet (the
/// rules are in [`write`](mod@crate::write)).
///
/// # Errors
///
/// A comment containing `<!--` or `-->`, a processing instruction containing `?>`
/// ([`Error::NestedComment`], [`Error::PiEndMarker`]), or elements nested deeper than
/// [`MAX_DEPTH`] ([`Error::TooDeep`]).
pub fn write_stylesheet(document: &Document) -> Result<String, Error> {
    write::write(document)
}

/// The compiled stylesheet ph-schematron writes from the result tree of its last step:
/// [`Document::from_result`], then [`write_stylesheet`].
///
/// # Errors
///
/// Those of both.
pub fn compile_result(xml: &str) -> Result<String, Error> {
    write_stylesheet(&Document::from_result(xml)?)
}
