//! From XML text to [`Stylesheet`].
//!
//! The loader walks the stylesheet once. Every element and attribute is checked against
//! the supported subset; whatever is outside it is a [`LoadError`]. Errors do not stop the
//! walk: the siblings of a faulty element are still checked, so one run lists them all.

mod declarations;
mod instructions;
mod strip;

use roxmltree::{Attribute, Document, Node};

use crate::error::{LoadError, LoadErrorKind, LoadErrors};
use crate::ir::Stylesheet;
use einvoice_xpath::{Namespaces, QName, XML_NAMESPACE, is_ncname, is_xml_whitespace};

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

/// The loader state: the document, a line index and the errors found so far.
pub(crate) struct Loader<'a, 'input> {
    doc: &'a Document<'input>,
    line_starts: Vec<usize>,
    errors: Vec<LoadError>,
}

/// Loads a stylesheet from its text.
pub(crate) fn load(text: &str) -> Result<Stylesheet, LoadErrors> {
    let doc = Document::parse(text).map_err(|e| {
        LoadErrors(vec![LoadError {
            line: e.pos().row,
            kind: LoadErrorKind::Xml(e.to_string()),
        }])
    })?;
    let mut loader = Loader {
        doc: &doc,
        line_starts: std::iter::once(0)
            .chain(text.match_indices('\n').map(|(i, _)| i + 1))
            .collect(),
        errors: Vec::new(),
    };
    let result = loader.stylesheet(doc.root_element());
    match result {
        Ok(stylesheet) if loader.errors.is_empty() => Ok(stylesheet),
        _ => Err(LoadErrors(loader.errors)),
    }
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

impl Loader<'_, '_> {
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
        self.errors.push(LoadError { line, kind });
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
    /// namespace) or be `xml:space` with a valid value.
    pub(crate) fn check_attributes(&mut self, node: Node<'_, '_>, allowed: &[&str]) -> Load<()> {
        let mut failed = false;
        for attribute in node.attributes() {
            let known = match attribute.namespace() {
                None => allowed.contains(&attribute.name()),
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
