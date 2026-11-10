//! The three XSLT steps of ph-schematron's pipeline, run by this project's engine
//! (`einvoice-xslt`, `einvoice-xslt-exec`) on the ISO Schematron skeleton as packaged in
//! ph-schematron-isosch 10.3.1 ([`SKELETON_FILES`], given to [`Skeleton::new`]), as
//! ph-schematron runs them on Saxon-HE 12.10:
//!
//! 1. `iso_dsdl_include.xsl` on the Schematron file, parsed as Saxon parses a
//!    `StreamSource` (its namespace declarations kept, its `xml:id` attributes IDs);
//! 2. `iso_abstract_expand.xsl` on the result of step 1;
//! 3. `iso_svrl_for_xslt2.xsl`, importing `iso_schematron_skeleton_for_saxon.xsl`, on the
//!    result of step 2.
//!
//! ph-schematron writes the result of steps 1 and 2 into a DOM document of the JDK and
//! gives it to the next step as a `DOMSource`, which Saxon wraps: what the next step reads
//! is the DOM ([`Document::from_result`], attributes in Xerces' order, namespace
//! declarations as Saxon's `DOMWriter` makes them) seen through Saxon's DOM wrapper
//! ([`einvoice_xpath_eval::TreeView::Dom`]: no attribute is an ID, `generate-id()` in
//! the wrapper's form), with the system ID of the Schematron file as base URI. The same
//! is done here: each result tree is written as XML, built into that DOM, and read back
//! as that view. The result of step 3 is the compiled stylesheet's result tree, written
//! by [`compile_result`](crate::compile_result).
//!
//! `document()` reads only the files given ([`Sources`]): the Schematron file, the files
//! it includes, and the skeleton's own modules (`document('')` in its messages).
//! Anything else is `FODC0002`, as for a file that does not exist.
//!
//! The skeleton is not part of this crate: it is the compiler ph-schematron runs, which
//! ph-schematron distributes in `ph-schematron-isosch` (under the MIT licence of the
//! skeleton and the Apache License 2.0 of ph-schematron's changes). The caller gives its
//! files, read from that jar (its directory [`SKELETON_DIR`]), and they must be those of
//! version 10.3.1, byte for byte.

use std::rc::Rc;

use einvoice_xpath::QName;
use einvoice_xpath_eval::{Document as Tree, TreeBuilder};
use einvoice_xslt_exec::{
    DocumentResolver, Executable, Serialization, TransformError, TransformOptions,
};
use sha2::{Digest, Sha256};

use crate::dom::{Document, Element, Node, XMLNS_NAMESPACE};
use crate::{Error, write_stylesheet};

/// The directory of the skeleton's stylesheets in the jar of ph-schematron-isosch.
pub const SKELETON_DIR: &str = "external/schematron/20100710-xslt2/";

/// The stylesheets of the ISO Schematron skeleton of 2010 that the pipeline runs, as
/// packaged in ph-schematron-isosch 10.3.1: the name of each file and its SHA-256.
pub const SKELETON_FILES: [(&str, &str); 4] = [
    (
        "iso_dsdl_include.xsl",
        "e59976a934bcffba9998607f4fce5009130d69f9530ebee9a858aa289ac03412",
    ),
    (
        "iso_abstract_expand.xsl",
        "16e8e7da796100de38446e64c4b2845c318e316a35c66098381b6846f6c89aac",
    ),
    (
        "iso_svrl_for_xslt2.xsl",
        "182fd021dd85948e2671a050d8f2da326597276907afa555e2613f9dfe814113",
    ),
    (
        "iso_schematron_skeleton_for_saxon.xsl",
        "d4fa0fc67d0689f7b957e2b3f486e7c295dfd7a83becd397085bc718d1527694",
    ),
];

/// The URI under which the skeleton's modules are known: `skeleton:/` and the file name.
const SKELETON_BASE: &str = "skeleton:/";

/// The modules of the skeleton, by URI.
#[derive(Debug, Clone)]
struct Modules(Vec<(String, String)>);

impl Modules {
    fn get(&self, uri: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(u, _)| u == uri)
            .map(|(_, text)| text.as_str())
    }
}

impl einvoice_xslt::ModuleResolver for Modules {
    fn resolve(&self, uri: &str) -> Option<String> {
        self.get(uri).map(str::to_owned)
    }
}

/// The stack of the thread that runs the steps (reserved, used only as deep as needed):
/// 64 MiB. The skeleton recurses over each character of every XPath expression of the
/// schema: as tail calls, which need no stack, but a parameter that a recursion passes
/// down and reads only at its end is a chain of values computed one inside another
/// (Saxon computes them so too), as long as the expression (8519 characters in the CEN
/// rules, which need less than 8 MiB of it in a debug build).
pub const STACK: usize = 64 << 20;

/// The stack the steps may use, measured from the start of each
/// ([`TransformOptions::max_stack`]): [`STACK`] less 8 MiB for what runs above them.
const MAX_STACK: u32 = 56 << 20;

const _: () = assert!(STACK - MAX_STACK as usize == 8 << 20);

/// The files `document()` may read while a Schematron file is compiled, besides the
/// skeleton's modules: by absolute URI, the Schematron file and the files it includes.
#[derive(Debug, Clone, Default)]
pub struct Sources {
    files: Vec<(String, String)>,
}

impl Sources {
    /// No file.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a file: its absolute URI and its text.
    #[must_use]
    pub fn with(mut self, uri: impl Into<String>, text: impl Into<String>) -> Self {
        self.files.push((uri.into(), text.into()));
        self
    }
}

/// What `document()` reads while a Schematron file is compiled: the files given and the
/// modules of the skeleton.
struct Resolver<'a> {
    sources: &'a Sources,
    modules: &'a Modules,
}

impl DocumentResolver for Resolver<'_> {
    fn resolve(&self, uri: &str) -> Option<String> {
        self.sources
            .files
            .iter()
            .find(|(u, _)| u == uri)
            .map(|(_, text)| text.clone())
            .or_else(|| self.modules.get(uri).map(str::to_owned))
    }
}

/// The three steps of the skeleton, prepared once to compile any number of Schematron
/// files.
#[derive(Debug)]
pub struct Skeleton {
    steps: [Executable; 3],
    modules: Modules,
}

/// What a compilation produced: the compiled stylesheet, the result of each step and
/// the messages of the skeleton.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compilation {
    /// The compiled stylesheet, as `Schematron2XSLTMojo` writes it.
    pub stylesheet: String,
    /// The result of steps 1 and 2 as the next step reads it (the DOM document of each),
    /// and the result tree of step 3, each written as XML as Saxon writes it (the
    /// `xsl:output` of step 3, without indentation): what the `ph-schematron-oracle`
    /// example writes as `NAME.step1.xml`, `NAME.step2.xml` and `NAME.step3.xml`.
    pub steps: [String; 3],
    /// The messages of `xsl:message` of each step, in order (warnings of the skeleton on
    /// the schema), as [`einvoice_xslt_exec::Transformation::messages`].
    pub messages: Vec<String>,
}

impl Skeleton {
    /// Loads and prepares the skeleton from its files, by name: each of
    /// [`SKELETON_FILES`] (others are ignored), with its SHA-256.
    ///
    /// # Errors
    ///
    /// A file is missing or is not that of ph-schematron-isosch 10.3.1
    /// ([`Error::Skeleton`]); the skeleton does not load or prepare (a bug of this
    /// project).
    pub fn new(files: &[(String, String)]) -> Result<Self, Error> {
        let mut modules = Vec::with_capacity(SKELETON_FILES.len());
        for (name, sha256) in SKELETON_FILES {
            let text = files
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, text)| text)
                .ok_or_else(|| Error::Skeleton(format!("no {name}")))?;
            let actual =
                Sha256::digest(text.as_bytes())
                    .iter()
                    .fold(String::new(), |mut hex, b| {
                        use std::fmt::Write as _;
                        let _ = write!(hex, "{b:02x}");
                        hex
                    });
            if actual != sha256 {
                return Err(Error::Skeleton(format!(
                    "{name} has the SHA-256 {actual}, not {sha256}: not the stylesheet of ph-schematron-isosch 10.3.1"
                )));
            }
            modules.push((format!("{SKELETON_BASE}{name}"), text.clone()));
        }
        let modules = Modules(modules);
        let prepare = |name: &str| -> Result<Executable, Error> {
            let uri = format!("{SKELETON_BASE}{name}");
            let text = modules
                .get(&uri)
                .ok_or_else(|| Error::Skeleton(format!("no {name}")))?;
            let sheet = einvoice_xslt::load_with_imports(text, &uri, &modules)
                .map_err(|e| Error::Skeleton(e.to_string()))?;
            Executable::new(&sheet).map_err(|e| Error::Skeleton(e.to_string()))
        };
        let steps = [
            prepare("iso_dsdl_include.xsl")?,
            prepare("iso_abstract_expand.xsl")?,
            prepare("iso_svrl_for_xslt2.xsl")?,
        ];
        Ok(Self { steps, modules })
    }

    /// Compiles the Schematron file `schematron`, whose absolute URI is `uri` (the system
    /// ID ph-schematron gives Saxon), reading the files it includes from `sources`.
    ///
    /// # Errors
    ///
    /// The Schematron file is not well-formed XML or has a document type declaration
    /// ([`Error::Schematron`]); a step raises a dynamic error, `xsl:message` with
    /// `terminate="yes"` included ([`Error::Step`]); the result of a step cannot be a DOM
    /// document ([`Error::Dom`], [`Error::Xml`]); the stylesheet cannot be written
    /// ([`write_stylesheet`]).
    pub fn compile(
        &self,
        schematron: &str,
        uri: &str,
        sources: &Sources,
    ) -> Result<Compilation, Error> {
        std::thread::scope(|scope| {
            std::thread::Builder::new()
                .name("schematron".to_owned())
                .stack_size(STACK)
                .spawn_scoped(scope, || self.run(schematron, uri, sources))
                .map_err(|e| Error::Skeleton(format!("cannot start a thread: {e}")))?
                .join()
                .map_err(|_| Error::Skeleton("the compilation panicked".to_owned()))?
        })
    }

    /// [`Skeleton::compile`] on the thread it runs on.
    fn run(&self, schematron: &str, uri: &str, sources: &Sources) -> Result<Compilation, Error> {
        let source = Tree::parse(schematron, Some(uri))
            .map_err(|e| Error::Schematron(e.message().to_owned()))?;
        let resolver = Resolver {
            sources,
            modules: &self.modules,
        };
        let options = TransformOptions {
            resolver: Some(&resolver),
            max_stack: MAX_STACK,
            ..TransformOptions::default()
        };
        let mut messages = Vec::new();
        let mut input = source.root();
        let mut steps: [String; 3] = Default::default();
        let mut result_tree = None;
        for (step, executable) in self.steps.iter().enumerate() {
            let number = u8::try_from(step + 1).unwrap_or(u8::MAX);
            let step_error = |error: TransformError| Error::Step {
                step: number,
                error,
            };
            let result = executable
                .transform_tree(&input, &options)
                .map_err(step_error)?;
            messages.extend(result.messages);
            let root = result.document.root();
            if step == 2 {
                let settings = Serialization {
                    indent: false,
                    ..executable.serialization()
                };
                steps[2] = einvoice_xslt_exec::serialize(&root, settings).map_err(step_error)?;
                result_tree = Some(Document::from_result(&steps[2])?);
                break;
            }
            // The DOM document of the result, as the next step sees it.
            let xml = einvoice_xslt_exec::serialize(&root, Serialization::default())
                .map_err(step_error)?;
            let dom = Document::from_result(&xml)?;
            let view = dom_view(&dom, uri)?;
            steps[step] = einvoice_xslt_exec::serialize(&view.root(), Serialization::default())
                .map_err(step_error)?;
            input = view.root();
        }
        let document = result_tree.ok_or_else(|| Error::Skeleton("no step 3".to_owned()))?;
        Ok(Compilation {
            stylesheet: write_stylesheet(&document)?,
            steps,
            messages,
        })
    }
}

/// The tree Saxon-HE 12.10 reads through its DOM wrapper from a DOM document built by its
/// `DOMWriter`: the elements with their attributes in the DOM's order, the namespace
/// declarations (`xmlns` attributes) as namespace nodes, adjacent text merged (the DOM is
/// normalized), every node with the base URI `base_uri`.
fn dom_view(dom: &Document, base_uri: &str) -> Result<Rc<Tree>, Error> {
    let mut builder = TreeBuilder::new().dom_view();
    // The children not added yet of the document node and the open elements, innermost
    // last.
    let mut open: Vec<std::slice::Iter<'_, Node>> = vec![dom.children.iter()];
    let tree_error =
        |e: einvoice_xpath_eval::DocumentError| Error::Skeleton(e.message().to_owned());
    while let Some(children) = open.last_mut() {
        let Some(node) = children.next() else {
            open.pop();
            if !open.is_empty() {
                builder.end_element().map_err(tree_error)?;
            }
            continue;
        };
        match node {
            Node::Element(element) => {
                start(&mut builder, element).map_err(tree_error)?;
                open.push(element.children.iter());
            }
            Node::Text(data) | Node::CData(data) => builder.text(data).map_err(tree_error)?,
            Node::Comment(data) => builder.comment(data).map_err(tree_error)?,
            Node::ProcessingInstruction { target, data } => builder
                .processing_instruction(target, data)
                .map_err(tree_error)?,
        }
    }
    builder
        .finish(None, Some(base_uri.to_owned()))
        .map_err(tree_error)
}

/// Opens an element of the DOM in the view: its namespace declarations as namespace
/// nodes, its other attributes in order.
fn start(
    builder: &mut TreeBuilder,
    element: &Element,
) -> Result<(), einvoice_xpath_eval::DocumentError> {
    let qname = |namespace: Option<&str>, name: &str| {
        let (prefix, local) = match name.split_once(':') {
            Some((p, l)) => (Some(p.to_owned()), l.to_owned()),
            None => (None, name.to_owned()),
        };
        QName {
            prefix,
            namespace: namespace.map(str::to_owned),
            local,
        }
    };
    let namespaces: Vec<einvoice_xpath::NamespaceBinding> = element
        .attributes()
        .iter()
        .filter(|a| a.namespace.as_deref() == Some(XMLNS_NAMESPACE))
        .map(|a| einvoice_xpath::NamespaceBinding {
            prefix: a.prefix().map(|_| a.local_name().to_owned()),
            uri: a.value.clone(),
        })
        .collect();
    builder.start_element_with_namespaces(
        qname(element.namespace(), element.name()),
        namespaces.into(),
    )?;
    for attribute in element
        .attributes()
        .iter()
        .filter(|a| a.namespace.as_deref() != Some(XMLNS_NAMESPACE))
    {
        builder.attribute(
            qname(attribute.namespace.as_deref(), &attribute.name),
            &attribute.value,
        )?;
    }
    Ok(())
}
