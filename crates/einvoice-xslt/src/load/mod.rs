//! From XML text to [`Stylesheet`].
//!
//! The loader reads the stylesheet modules first (the principal module, then the modules
//! it imports, [`modules`]), then walks each module once. Every element and attribute is
//! checked against the supported subset; whatever is outside it is a [`LoadError`]. Errors
//! do not stop the walk: the siblings of a faulty element are still checked, so one run
//! lists them all. Elements whose `use-when` is false are left out before the walk, as
//! XSLT 2.0 section 3.12 prescribes ([`static_eval`]).

mod declarations;
mod instructions;
mod modules;
mod static_eval;
mod strip;

use std::collections::HashSet;
use std::sync::Arc;

use roxmltree::{Attribute, Document, Node, NodeId};

use crate::ModuleResolver;
use crate::error::{LoadError, LoadErrorKind, LoadErrors};
use crate::ir::{NamespaceBinding, Stylesheet};
use einvoice_xpath::{Namespaces, QName, XML_NAMESPACE, is_ncname, is_xml_whitespace};

pub use modules::resolve_uri;

pub(crate) const XSL: &str = "http://www.w3.org/1999/XSL/Transform";

/// Whether `node` is an element in the XSLT namespace.
pub(crate) fn is_xsl(node: Node<'_, '_>) -> bool {
    node.is_element() && node.tag_name().namespace() == Some(XSL)
}

/// Marks a failure whose error has already been recorded.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Failed;

pub(crate) type Load<T> = Result<T, Failed>;

/// The in-scope namespaces of a stylesheet element, for the XPath parser.
pub(crate) struct InScope<'a, 'input>(pub(crate) Node<'a, 'input>);

impl Namespaces for InScope<'_, '_> {
    fn namespace_uri(&self, prefix: &str) -> Option<String> {
        self.0.lookup_namespace_uri(Some(prefix)).map(str::to_owned)
    }
}

/// An `xsl:namespace-alias` declaration (XSLT 2.0 section 11.1.4), in force in every
/// module: names in the literal namespace are written in the target namespace, with the
/// result prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Alias {
    /// The literal namespace URI (`stylesheet-prefix`), empty for no namespace.
    pub(crate) literal: String,
    /// The target namespace URI (`result-prefix`), empty for no namespace.
    pub(crate) target: String,
    /// The result prefix, `None` for `#default`.
    pub(crate) prefix: Option<String>,
}

/// What a module's walk needs to know of the others.
#[derive(Debug, Clone)]
pub(crate) struct ModuleContext {
    /// The index of the module in [`Stylesheet::modules`].
    pub(crate) index: usize,
    /// Its URI.
    pub(crate) uri: Option<String>,
    /// The namespace aliases of the whole stylesheet.
    pub(crate) aliases: Arc<[Alias]>,
}

/// The loader state of one module: the document, a line index and the errors found so
/// far.
pub(crate) struct Loader<'a, 'input> {
    doc: &'a Document<'input>,
    line_starts: Vec<usize>,
    errors: Vec<LoadError>,
    /// The distinct namespace sets seen so far, shared by the elements that have them.
    namespace_sets: Vec<Arc<[NamespaceBinding]>>,
    /// How many sequence constructors contain the one being read (see
    /// [`MAX_NESTING`](crate::MAX_NESTING)).
    nesting: u32,
    /// The module.
    module: ModuleContext,
    /// The namespaces of the extension element prefixes of the module
    /// (`extension-element-prefixes` on its stylesheet element).
    extension_namespaces: Vec<String>,
    /// The elements that `use-when` leaves out.
    excluded: HashSet<NodeId>,
}

/// Loads a stylesheet without imports from its text.
pub(crate) fn load(text: &str) -> Result<Stylesheet, LoadErrors> {
    load_modules(text, None, None)
}

/// Loads a stylesheet and the modules it imports.
pub(crate) fn load_with_imports(
    text: &str,
    uri: &str,
    resolver: &dyn ModuleResolver,
) -> Result<Stylesheet, LoadErrors> {
    load_modules(text, Some(uri), Some(resolver))
}

fn load_modules(
    text: &str,
    uri: Option<&str>,
    resolver: Option<&dyn ModuleResolver>,
) -> Result<Stylesheet, LoadErrors> {
    let sources = modules::discover(text, uri, resolver).map_err(LoadErrors)?;
    let documents: Vec<Document<'_>> = sources
        .iter()
        .map(|s| Document::parse(&s.text))
        .collect::<Result<_, _>>()
        .map_err(|e| {
            LoadErrors(vec![LoadError {
                module: None,
                line: e.pos().row,
                kind: LoadErrorKind::Xml(e.to_string()),
            }])
        })?;
    let (aliases, mut errors) = modules::aliases(&sources, &documents);
    let aliases: Arc<[Alias]> = aliases.into();
    let mut parts = Vec::with_capacity(sources.len());
    for (index, (source, doc)) in sources.iter().zip(&documents).enumerate() {
        let mut loader = Loader {
            doc,
            line_starts: std::iter::once(0)
                .chain(source.text.match_indices('\n').map(|(i, _)| i + 1))
                .collect(),
            errors: Vec::new(),
            namespace_sets: Vec::new(),
            nesting: 0,
            module: ModuleContext {
                index,
                uri: source.uri.clone(),
                aliases: Arc::clone(&aliases),
            },
            extension_namespaces: Vec::new(),
            excluded: HashSet::new(),
        };
        loader.exclude_by_use_when(doc.root_element());
        let result = loader.stylesheet(doc.root_element());
        errors.append(&mut loader.errors);
        if let Ok(part) = result {
            parts.push(part);
        }
    }
    if !errors.is_empty() {
        return Err(LoadErrors(errors));
    }
    modules::merge(&sources, parts).map_err(LoadErrors)
}

/// Runs `f` on each item and keeps going after a failure; fails if any item failed.
pub(crate) fn each<I, T>(items: I, mut f: impl FnMut(I::Item) -> Load<T>) -> Load<Vec<T>>
where
    I: IntoIterator,
{
    let mut out = Vec::new();
    let mut failed = false;
    for item in items {
        match f(item) {
            Ok(value) => out.push(value),
            Err(Failed) => failed = true,
        }
    }
    if failed { Err(Failed) } else { Ok(out) }
}

impl<'a, 'input> Loader<'a, 'input> {
    /// The children of `node` after whitespace stripping, without the elements
    /// `use-when` leaves out (see [`strip::children`]).
    pub(crate) fn children<'n>(&self, node: Node<'n, 'input>) -> Vec<strip::Child<'n, 'input>> {
        strip::children(node, &self.excluded)
    }

    /// Finds the elements whose `use-when` (`xsl:use-when` on an element outside the
    /// XSLT namespace) is false, statically evaluated: they and their descendants are not
    /// part of the stylesheet. An expression that cannot be evaluated statically is an
    /// error, and the element is left out.
    fn exclude_by_use_when(&mut self, root: Node<'a, 'input>) {
        let mut stack = vec![root];
        while let Some(node) = stack.pop() {
            let use_when = if is_xsl(node) {
                node.attribute_node("use-when")
            } else {
                node.attribute_node((XSL, "use-when"))
            };
            if let Some(attribute) = use_when {
                match static_eval::use_when(attribute.value(), node) {
                    Ok(true) => {}
                    Ok(false) => {
                        self.excluded.insert(node.id());
                        continue;
                    }
                    Err(message) => {
                        let line = self.line_at(attribute.range().start);
                        let element = self.element_name(node);
                        self.fail(
                            line,
                            LoadErrorKind::Invalid(format!("{element}/@use-when: {message}")),
                        );
                        self.excluded.insert(node.id());
                        continue;
                    }
                }
            }
            stack.extend(node.children().filter(Node::is_element));
        }
    }

    /// The namespaces in scope on `node`, without the implicit `xml` namespace and, when
    /// `exclude_xslt`, without the XSLT namespace; an equal set seen before is shared.
    pub(crate) fn in_scope_namespaces(
        &mut self,
        node: Node<'_, '_>,
        exclude_xslt: bool,
    ) -> Arc<[NamespaceBinding]> {
        let set: Vec<NamespaceBinding> = node
            .namespaces()
            .filter(|ns| ns.uri() != XML_NAMESPACE && !(exclude_xslt && ns.uri() == XSL))
            .map(|ns| NamespaceBinding {
                prefix: ns.name().map(str::to_owned),
                uri: ns.uri().to_owned(),
            })
            .collect();
        self.share(set)
    }

    /// An equal set seen before, or this one, shared.
    pub(crate) fn share(&mut self, set: Vec<NamespaceBinding>) -> Arc<[NamespaceBinding]> {
        if let Some(shared) = self.namespace_sets.iter().find(|s| ***s == *set) {
            return Arc::clone(shared);
        }
        let shared: Arc<[NamespaceBinding]> = set.into();
        self.namespace_sets.push(Arc::clone(&shared));
        shared
    }

    /// The 1-based line of a byte offset.
    pub(crate) fn line_at(&self, offset: usize) -> u32 {
        let line = self.line_starts.partition_point(|&start| start <= offset);
        u32::try_from(line).unwrap_or(u32::MAX)
    }

    pub(crate) fn line(&self, node: Node<'_, '_>) -> u32 {
        self.line_at(node.range().start)
    }

    /// Records an error at `line` and returns the failure marker.
    pub(crate) fn fail(&mut self, line: u32, kind: LoadErrorKind) -> Failed {
        self.errors.push(LoadError {
            module: self.module.uri.clone(),
            line,
            kind,
        });
        Failed
    }

    pub(crate) fn invalid(&mut self, node: Node<'_, '_>, message: String) -> Failed {
        let line = self.line(node);
        self.fail(line, LoadErrorKind::Invalid(message))
    }

    /// The qualified name of an element or attribute as written in the source.
    fn written_name(&self, range: std::ops::Range<usize>) -> String {
        let text = self.doc.input_text().get(range).unwrap_or("");
        let text = text.strip_prefix('<').unwrap_or(text);
        text.chars()
            .take_while(|&c| c == ':' || einvoice_xpath::is_name_char(c))
            .collect()
    }

    /// How an element is named in messages: `xsl:local` for XSLT elements, as written
    /// otherwise.
    pub(crate) fn element_name(&self, node: Node<'_, '_>) -> String {
        if is_xsl(node) {
            format!("xsl:{}", node.tag_name().name())
        } else {
            self.written_name(node.range())
        }
    }

    /// The name of an element as a [`QName`], with its prefix as written.
    pub(crate) fn element_qname(&self, node: Node<'_, '_>) -> QName {
        let written = self.written_name(node.range());
        QName {
            prefix: written.split_once(':').map(|(p, _)| p.to_owned()),
            namespace: node.tag_name().namespace().map(str::to_owned),
            local: node.tag_name().name().to_owned(),
        }
    }

    /// The name of an attribute as a [`QName`], with its prefix as written.
    pub(crate) fn attribute_qname(&self, attribute: &Attribute<'_, '_>) -> QName {
        let written = self.written_name(attribute.range_qname());
        QName {
            prefix: written.split_once(':').map(|(p, _)| p.to_owned()),
            namespace: attribute.namespace().map(str::to_owned),
            local: attribute.name().to_owned(),
        }
    }

    /// Checks the attributes of an XSLT element: each must be in `allowed` (names in no
    /// namespace), `use-when` (already evaluated) or `xml:space` with a valid value.
    pub(crate) fn check_attributes(&mut self, node: Node<'_, '_>, allowed: &[&str]) -> Load<()> {
        let mut failed = false;
        for attribute in node.attributes() {
            let known = match attribute.namespace() {
                None => allowed.contains(&attribute.name()) || attribute.name() == "use-when",
                Some(XML_NAMESPACE) if attribute.name() == "space" => {
                    if !matches!(attribute.value(), "preserve" | "default") {
                        failed = true;
                        let line = self.line_at(attribute.range().start);
                        self.fail(
                            line,
                            LoadErrorKind::Invalid(format!(
                                "xml:space must be `preserve` or `default`, not `{}`",
                                attribute.value()
                            )),
                        );
                    }
                    true
                }
                Some(_) => false,
            };
            if !known {
                failed = true;
                let element = self.element_name(node);
                let name = self.attribute_qname(&attribute).to_string();
                let line = self.line_at(attribute.range().start);
                self.fail(
                    line,
                    LoadErrorKind::UnsupportedAttribute {
                        element,
                        attribute: name,
                    },
                );
            }
        }
        if failed { Err(Failed) } else { Ok(()) }
    }

    /// The value of a required attribute.
    pub(crate) fn required<'n>(&mut self, node: Node<'n, '_>, name: &str) -> Load<&'n str> {
        if let Some(value) = node.attribute(name) {
            return Ok(value);
        }
        let element = self.element_name(node);
        let line = self.line(node);
        Err(self.fail(
            line,
            LoadErrorKind::MissingAttribute {
                element,
                attribute: name.to_owned(),
            },
        ))
    }

    fn attribute_line(&self, node: Node<'_, '_>, name: &str) -> u32 {
        node.attribute_node(name)
            .map_or_else(|| self.line(node), |a| self.line_at(a.range().start))
    }

    /// Records an unsupported value of attribute `name`.
    pub(crate) fn unsupported_value(
        &mut self,
        node: Node<'_, '_>,
        name: &str,
        value: &str,
    ) -> Failed {
        let element = self.element_name(node);
        let line = self.attribute_line(node, name);
        self.fail(
            line,
            LoadErrorKind::UnsupportedValue {
                element,
                attribute: name.to_owned(),
                value: value.to_owned(),
            },
        )
    }

    /// Parses attribute `name` with one of the `einvoice_xpath` entry points; `None` when
    /// the attribute is absent.
    pub(crate) fn xpath<T>(
        &mut self,
        node: Node<'_, '_>,
        name: &str,
        parse: fn(&str, &dyn Namespaces) -> einvoice_xpath::Result<T>,
    ) -> Load<Option<T>> {
        let Some(value) = node.attribute(name) else {
            return Ok(None);
        };
        parse(value, &InScope(node)).map(Some).map_err(|error| {
            let element = self.element_name(node);
            let line = self.attribute_line(node, name);
            self.fail(
                line,
                LoadErrorKind::XPath {
                    element,
                    attribute: name.to_owned(),
                    error,
                },
            )
        })
    }

    /// Like [`Loader::xpath`] for a required attribute.
    pub(crate) fn required_xpath<T>(
        &mut self,
        node: Node<'_, '_>,
        name: &str,
        parse: fn(&str, &dyn Namespaces) -> einvoice_xpath::Result<T>,
    ) -> Load<T> {
        self.required(node, name)?;
        self.xpath(node, name, parse)?.ok_or(Failed)
    }

    /// Resolves a `QName`-valued attribute (`name`, `mode`, ...): an unprefixed name is in
    /// no namespace (XSLT 2.0 section 5.1).
    pub(crate) fn qname_value(
        &mut self,
        node: Node<'_, '_>,
        attribute: &str,
        value: &str,
    ) -> Load<QName> {
        let value = value.trim_matches(is_xml_whitespace);
        let (prefix, local) = match value.split_once(':') {
            Some((prefix, local)) => (Some(prefix), local),
            None => (None, value),
        };
        if !is_ncname(local) || prefix.is_some_and(|p| !is_ncname(p)) {
            let message = format!(
                "`{value}` in attribute `{attribute}` of {} is not a QName (XTSE0020)",
                self.element_name(node)
            );
            return Err(self.invalid(node, message));
        }
        let namespace = match prefix {
            None => None,
            Some(p) => Some(node.lookup_namespace_uri(Some(p)).map(str::to_owned).ok_or_else(|| {
                let message = format!("prefix `{p}` in attribute `{attribute}` of {} is not declared (XTSE0280)", self.element_name(node));
                self.invalid(node, message)
            })?),
        };
        Ok(QName {
            prefix: prefix.map(str::to_owned),
            namespace,
            local: local.to_owned(),
        })
    }

    /// A required `QName`-valued attribute.
    pub(crate) fn required_qname(&mut self, node: Node<'_, '_>, attribute: &str) -> Load<QName> {
        let value = self.required(node, attribute)?;
        self.qname_value(node, attribute, value)
    }
}
