//! Loading a schema set: parsing the schema documents reachable from the entry points
//! through `xs:import` and `xs:include`, and compiling their components.
//!
//! Everything outside the supported subset is refused with
//! [`LoadErrorKind::Unsupported`] naming the construct and its line; nothing is
//! silently ignored except the content of `xs:annotation`. The one import that may
//! be left unloaded is that of a namespace of [`XADES_NAMESPACES`] whose schema
//! document is not in the set: it is recorded in [`Schema::omitted_imports`].

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use roxmltree::{Node, NodeId};

use crate::automaton::{Automaton, MAX_FINITE_OCCURS, Particle, ParticleTerm};
use crate::datatypes::{self, Builtin, UNIMPLEMENTED_BUILTINS};
use crate::error::{LoadError, LoadErrorKind};
use crate::model::{
    ANY_TYPE, AttributeUse, ComplexType, ConstraintKind, Content, ElementDecl, ElementId,
    FixedValue, NamespaceConstraint, OmittedImport, ProcessContents, QName, Schema, SimpleType,
    TypeDef, TypeId, ValueConstraint, Wildcard, XS, facet_problem, quote,
};
use crate::ubl::XADES_NAMESPACES;

/// A parsed schema document.
struct Doc<'a> {
    path: String,
    xml: roxmltree::Document<'a>,
    target: Option<String>,
    qualified_elements: bool,
    qualified_attributes: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BuildState {
    Pending,
    Building,
    Built,
}

type Result<T> = std::result::Result<T, LoadError>;

fn error(
    doc: &Doc<'_>,
    node: Node<'_, '_>,
    kind: LoadErrorKind,
    message: impl Into<String>,
) -> LoadError {
    LoadError {
        kind,
        file: doc.path.clone(),
        line: doc.xml.text_pos_at(node.range().start).row,
        message: message.into(),
    }
}

fn is_xs(node: Node<'_, '_>, local: &str) -> bool {
    node.is_element() && node.tag_name().namespace() == Some(XS) && node.tag_name().name() == local
}

/// Joins a relative schema location to the directory of the including document and
/// normalises `.` and `..` segments.
fn resolve_location(base: &str, location: &str) -> Option<String> {
    if location.contains(':') || location.starts_with('/') || location.contains('\\') {
        return None;
    }
    let mut segments: Vec<&str> = base.split('/').collect();
    segments.pop();
    for segment in location.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop()?;
            }
            other => segments.push(other),
        }
    }
    Some(segments.join("/"))
}

/// The `default` or `fixed` value constraint of an element or attribute
/// declaration, if it has one (both at once is an error: src-element.1,
/// src-attribute.1).
fn value_constraint(doc: &Doc<'_>, node: Node<'_, '_>) -> Result<Option<ValueConstraint>> {
    let constraint = |kind, value: &str| {
        Some(ValueConstraint {
            kind,
            value: value.into(),
        })
    };
    match (node.attribute("default"), node.attribute("fixed")) {
        (Some(_), Some(_)) => Err(error(
            doc,
            node,
            LoadErrorKind::Invalid,
            "the attributes default and fixed are mutually exclusive",
        )),
        (Some(value), None) => Ok(constraint(ConstraintKind::Default, value)),
        (None, Some(value)) => Ok(constraint(ConstraintKind::Fixed, value)),
        (None, None) => Ok(None),
    }
}

fn parse_boolean(value: &str) -> Option<bool> {
    match datatypes::normalize(value, datatypes::WhiteSpace::Collapse).as_ref() {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}

/// Checks that a schema element has only the given attributes (all unqualified).
fn check_attributes(doc: &Doc<'_>, node: Node<'_, '_>, allowed: &[&str]) -> Result<()> {
    for attribute in node.attributes() {
        if attribute.namespace().is_some() || !allowed.contains(&attribute.name()) {
            return Err(error(
                doc,
                node,
                LoadErrorKind::Unsupported,
                format!(
                    "attribute {} on xs:{}",
                    crate::model::clark(attribute.namespace(), attribute.name()),
                    node.tag_name().name()
                ),
            ));
        }
    }
    Ok(())
}

/// The element children of a schema element, without the leading `xs:annotation`.
/// Fails on foreign elements and on non-whitespace text.
fn children<'a, 'i>(doc: &Doc<'_>, node: Node<'a, 'i>) -> Result<Vec<Node<'a, 'i>>> {
    let mut out = Vec::new();
    for child in node.children() {
        if child.is_text() {
            if !child
                .text()
                .unwrap_or("")
                .chars()
                .all(datatypes::is_xml_space)
            {
                return Err(error(
                    doc,
                    node,
                    LoadErrorKind::Invalid,
                    "text in a schema element",
                ));
            }
        } else if child.is_element() {
            if child.tag_name().namespace() != Some(XS) {
                return Err(error(
                    doc,
                    child,
                    LoadErrorKind::Unsupported,
                    format!(
                        "element {} in a schema",
                        crate::model::clark(child.tag_name().namespace(), child.tag_name().name())
                    ),
                ));
            }
            if is_xs(child, "annotation") {
                if !out.is_empty() {
                    return Err(error(
                        doc,
                        child,
                        LoadErrorKind::Invalid,
                        "xs:annotation after other content",
                    ));
                }
                continue;
            }
            out.push(child);
        }
    }
    Ok(out)
}

struct Loader<'d, 'a> {
    docs: &'d [Doc<'a>],
    /// The imports that were not loaded.
    omitted: &'d [OmittedImport],
    types: Vec<TypeDef>,
    elements: Vec<ElementDecl>,
    global_types: HashMap<Option<String>, HashMap<String, TypeId>>,
    global_elements: HashMap<Option<String>, HashMap<String, ElementId>>,
    /// Source of each user-defined global type, by type id.
    type_sources: BTreeMap<TypeId, (usize, NodeId)>,
    /// Source of each global element declaration.
    element_sources: Vec<(ElementId, usize, NodeId)>,
    state: Vec<BuildState>,
    /// Attribute uses whose type must turn out to be simple: (type, doc, node).
    simple_type_checks: Vec<(TypeId, usize, NodeId)>,
    /// The value constraints of attribute declarations, checked once every type
    /// is built.
    attribute_constraints: Vec<PendingConstraint>,
    /// The value constraints of element declarations, checked once every type is
    /// built.
    element_constraints: Vec<(ElementId, PendingConstraint)>,
    /// The fixed values of attribute uses ([`FixedValue`]).
    fixed_values: Vec<Box<str>>,
}

/// A value constraint whose validity can only be checked once the type it
/// constrains is built: the declared type, the constraint, and where it is.
struct PendingConstraint {
    type_id: TypeId,
    constraint: ValueConstraint,
    doc: usize,
    node: NodeId,
}

/// Loads a schema set. `files` maps paths (relative, `/`-separated) to schema
/// document texts; `entry_points` are paths among them.
pub(crate) fn load(files: &[(&str, &str)], entry_points: &[&str]) -> Result<Schema> {
    let texts: HashMap<&str, &str> = files.iter().copied().collect();
    let (docs, omitted) = discover(&texts, entry_points)?;
    let mut loader = Loader {
        docs: &docs,
        omitted: &omitted,
        types: Vec::new(),
        elements: Vec::new(),
        global_types: HashMap::new(),
        global_elements: HashMap::new(),
        type_sources: BTreeMap::new(),
        element_sources: Vec::new(),
        state: Vec::new(),
        simple_type_checks: Vec::new(),
        attribute_constraints: Vec::new(),
        element_constraints: Vec::new(),
        fixed_values: Vec::new(),
    };
    loader.add_builtins();
    loader.collect_globals()?;
    let ids: Vec<TypeId> = loader.type_sources.keys().copied().collect();
    for id in ids {
        loader.build_type(id)?;
    }
    loader.resolve_global_elements()?;
    loader.check_simple_types()?;
    let element_constraints = loader.check_value_constraints()?;
    let mut schema = Schema {
        types: loader.types,
        elements: loader.elements,
        global_elements: by_namespace_key(loader.global_elements),
        global_types: by_namespace_key(loader.global_types),
        documents: docs.iter().map(|d| d.path.clone()).collect(),
        element_constraints,
        fixed_values: loader.fixed_values,
        omitted_imports: omitted,
    };
    schema.shrink_to_fit();
    Ok(schema)
}

/// A global component map keyed as [`Schema`] keys it
/// ([`namespace_key`](crate::model::namespace_key)).
fn by_namespace_key<T>(
    map: HashMap<Option<String>, HashMap<String, T>>,
) -> HashMap<String, HashMap<String, T>> {
    map.into_iter()
        .map(|(namespace, by_name)| (namespace.unwrap_or_default(), by_name))
        .collect()
}

/// Parses the entry points and every document they import or include, except for
/// the imports of a namespace of [`XADES_NAMESPACES`] whose document is not in
/// `texts`, which are returned.
fn discover<'a>(
    texts: &HashMap<&str, &'a str>,
    entry_points: &[&str],
) -> Result<(Vec<Doc<'a>>, Vec<OmittedImport>)> {
    let mut docs: Vec<Doc<'a>> = Vec::new();
    let mut omitted: Vec<OmittedImport> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    // (path, expected target namespace or None for an entry point, includer target
    // for include)
    let mut queue: Vec<(String, Expect)> = entry_points
        .iter()
        .map(|p| ((*p).to_owned(), Expect::Any))
        .collect();
    queue.reverse();
    while let Some((path, expect)) = queue.pop() {
        let doc_index = if let Some(&i) = index.get(&path) {
            i
        } else {
            let text = texts.get(path.as_str()).ok_or_else(|| LoadError {
                kind: LoadErrorKind::MissingFile,
                file: path.clone(),
                line: 0,
                message: "not in the schema set".to_owned(),
            })?;
            let options = roxmltree::ParsingOptions {
                allow_dtd: true,
                ..roxmltree::ParsingOptions::default()
            };
            let xml =
                roxmltree::Document::parse_with_options(text, options).map_err(|e| LoadError {
                    kind: LoadErrorKind::Malformed,
                    file: path.clone(),
                    line: e.pos().row,
                    message: e.to_string(),
                })?;
            let doc = schema_doc(path.clone(), xml)?;
            let root = doc.xml.root_element();
            let mut found = Vec::new();
            for child in children(&doc, root)? {
                if is_xs(child, "import") {
                    check_attributes(&doc, child, &["namespace", "schemaLocation"])?;
                    let namespace = child.attribute("namespace").map(str::to_owned);
                    if namespace == doc.target {
                        return Err(error(
                            &doc,
                            child,
                            LoadErrorKind::Invalid,
                            "xs:import of the target namespace",
                        ));
                    }
                    let location = location(&doc, child)?;
                    if let Some(namespace) = namespace.as_deref()
                        && XADES_NAMESPACES.contains(&namespace)
                        && !texts.contains_key(location.as_str())
                    {
                        omitted.push(OmittedImport {
                            file: doc.path.clone(),
                            line: doc.xml.text_pos_at(child.range().start).row,
                            namespace: namespace.to_owned(),
                            location,
                        });
                        continue;
                    }
                    found.push((location, Expect::Namespace(namespace)));
                } else if is_xs(child, "include") {
                    check_attributes(&doc, child, &["schemaLocation"])?;
                    let location = location(&doc, child)?;
                    found.push((location, Expect::Namespace(doc.target.clone())));
                }
            }
            docs.push(doc);
            index.insert(path.clone(), docs.len() - 1);
            for item in found.into_iter().rev() {
                queue.push(item);
            }
            docs.len() - 1
        };
        if let Expect::Namespace(namespace) = expect {
            let doc = &docs[doc_index];
            if doc.target != namespace {
                return Err(LoadError {
                    kind: LoadErrorKind::Invalid,
                    file: doc.path.clone(),
                    line: 0,
                    message: format!(
                        "target namespace {:?} differs from the namespace {:?} it is imported or included for",
                        doc.target, namespace
                    ),
                });
            }
        }
    }
    Ok((docs, omitted))
}

enum Expect {
    Any,
    Namespace(Option<String>),
}

fn location(doc: &Doc<'_>, node: Node<'_, '_>) -> Result<String> {
    let value = node.attribute("schemaLocation").ok_or_else(|| {
        error(
            doc,
            node,
            LoadErrorKind::Unsupported,
            "xs:import or xs:include without schemaLocation",
        )
    })?;
    resolve_location(&doc.path, value.trim()).ok_or_else(|| {
        error(
            doc,
            node,
            LoadErrorKind::Unsupported,
            format!(
                "schemaLocation {value:?}: only relative paths within the schema set are supported"
            ),
        )
    })
}

fn schema_doc(path: String, xml: roxmltree::Document<'_>) -> Result<Doc<'_>> {
    let mut doc = Doc {
        path,
        xml,
        target: None,
        qualified_elements: false,
        qualified_attributes: false,
    };
    let root = doc.xml.root_element();
    if !is_xs(root, "schema") {
        return Err(error(
            &doc,
            root,
            LoadErrorKind::Invalid,
            "the document element is not xs:schema",
        ));
    }
    check_attributes(
        &doc,
        root,
        &[
            "targetNamespace",
            "elementFormDefault",
            "attributeFormDefault",
            "version",
        ],
    )?;
    let form = |name: &str| -> Result<bool> {
        match root.attribute(name) {
            None | Some("unqualified") => Ok(false),
            Some("qualified") => Ok(true),
            Some(other) => Err(error(
                &doc,
                root,
                LoadErrorKind::Invalid,
                format!("{name}={other:?}"),
            )),
        }
    };
    let qualified_elements = form("elementFormDefault")?;
    let qualified_attributes = form("attributeFormDefault")?;
    let target = match root.attribute("targetNamespace") {
        Some("") => {
            return Err(error(
                &doc,
                root,
                LoadErrorKind::Invalid,
                "empty targetNamespace",
            ));
        }
        other => other.map(str::to_owned),
    };
    doc.target = target;
    doc.qualified_elements = qualified_elements;
    doc.qualified_attributes = qualified_attributes;
    Ok(doc)
}

/// The facets of one restriction step of a simple type.
struct Facets {
    /// Whitespace-processed as the base type processes its values.
    enumeration: Vec<String>,
    min_length: Option<usize>,
    max_length: Option<usize>,
}

/// The facets of an `xs:restriction` of a simple type whose built-in type is
/// `builtin`: `enumeration`, `minLength` and `maxLength` only.
fn facets(doc: &Doc<'_>, restriction: Node<'_, '_>, builtin: Builtin) -> Result<Facets> {
    let mut out = Facets {
        enumeration: Vec::new(),
        min_length: None,
        max_length: None,
    };
    for facet in children(doc, restriction)? {
        let kind = facet.tag_name().name();
        if !matches!(kind, "enumeration" | "minLength" | "maxLength") {
            return Err(error(
                doc,
                facet,
                LoadErrorKind::Unsupported,
                format!("facet xs:{kind}"),
            ));
        }
        check_attributes(doc, facet, &["value"])?;
        if !children(doc, facet)?.is_empty() {
            return Err(error(
                doc,
                facet,
                LoadErrorKind::Invalid,
                format!("content in xs:{kind}"),
            ));
        }
        let value = facet.attribute("value").ok_or_else(|| {
            error(
                doc,
                facet,
                LoadErrorKind::Invalid,
                format!("xs:{kind} without value"),
            )
        })?;
        if kind == "enumeration" {
            out.enumeration
                .push(datatypes::normalize(value, builtin.white_space()).into_owned());
            continue;
        }
        let slot = if kind == "minLength" {
            &mut out.min_length
        } else {
            &mut out.max_length
        };
        if slot.is_some() {
            // Part 2, 4.1.3, Single Facet Value: only the pattern and enumeration
            // facets may be repeated in one derivation step.
            return Err(error(
                doc,
                facet,
                LoadErrorKind::Invalid,
                format!("xs:{kind} twice in one restriction"),
            ));
        }
        *slot = Some(length_facet(doc, facet, kind, value, builtin)?);
    }
    Ok(out)
}

/// The value of a `minLength` or `maxLength` facet of a restriction of a type
/// whose built-in type is `builtin`: an `xs:nonNegativeInteger` (Part 2, 4.3.2 and
/// 4.3.3), for a type the facet applies to.
fn length_facet(
    doc: &Doc<'_>,
    facet: Node<'_, '_>,
    kind: &str,
    value: &str,
    builtin: Builtin,
) -> Result<usize> {
    if !builtin.has_length() {
        // The length facets apply to anyURI and base64Binary too (Part 2, 4.1.5,
        // Applicable Facets), which this crate does not restrict that way; not to
        // the other types.
        let applies = matches!(builtin, Builtin::AnyUri | Builtin::Base64Binary);
        return Err(error(
            doc,
            facet,
            if applies {
                LoadErrorKind::Unsupported
            } else {
                LoadErrorKind::Invalid
            },
            format!(
                "xs:{kind} on a type derived from xs:{}{}",
                builtin.name(),
                if applies {
                    ""
                } else {
                    ", to which it does not apply"
                }
            ),
        ));
    }
    let collapsed = datatypes::normalize(value, datatypes::WhiteSpace::Collapse);
    // nonNegativeInteger: an optional sign, `-` only for zero, then digits.
    let digits = match collapsed.strip_prefix('-') {
        Some(rest) if !rest.is_empty() && rest.bytes().all(|b| b == b'0') => rest,
        Some(_) => "",
        None => collapsed.strip_prefix('+').unwrap_or(&collapsed),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(error(
            doc,
            facet,
            LoadErrorKind::Invalid,
            format!("xs:{kind} value {value:?} is not a nonNegativeInteger"),
        ));
    }
    digits.parse().map_err(|_| {
        error(
            doc,
            facet,
            LoadErrorKind::Unsupported,
            format!("xs:{kind} value {value:?}: lengths above {}", usize::MAX),
        )
    })
}

fn occurs(doc: &Doc<'_>, node: Node<'_, '_>) -> Result<(u32, Option<u32>)> {
    let parse = |name: &str, value: &str| -> Result<u32> {
        let value = value.trim();
        let number: u32 = value.parse().map_err(|_| {
            error(
                doc,
                node,
                LoadErrorKind::Invalid,
                format!("{name}={value:?}"),
            )
        })?;
        if number > MAX_FINITE_OCCURS {
            return Err(error(
                doc,
                node,
                LoadErrorKind::Unsupported,
                format!("{name}={number}: finite bounds above {MAX_FINITE_OCCURS}"),
            ));
        }
        Ok(number)
    };
    let min = node
        .attribute("minOccurs")
        .map_or(Ok(1), |v| parse("minOccurs", v))?;
    let max = match node.attribute("maxOccurs").map(str::trim) {
        None => Some(1),
        Some("unbounded") => None,
        Some(v) => Some(parse("maxOccurs", v)?),
    };
    if max.is_some_and(|max| max < min) {
        return Err(error(
            doc,
            node,
            LoadErrorKind::Invalid,
            "maxOccurs is less than minOccurs",
        ));
    }
    Ok((min, max))
}

impl<'d, 'a> Loader<'d, 'a> {
    fn push_type(&mut self, def: TypeDef, state: BuildState) -> TypeId {
        self.types.push(def);
        self.state.push(state);
        self.types.len() - 1
    }

    fn register_type(&mut self, name: &QName, id: TypeId) -> bool {
        self.global_types
            .entry(name.namespace.clone())
            .or_default()
            .insert(name.local.clone(), id)
            .is_none()
    }

    /// `xs:anyType` (id 0) and the implemented built-in simple types.
    fn add_builtins(&mut self) {
        let any = Particle {
            term: ParticleTerm::Wildcard(Wildcard {
                namespaces: NamespaceConstraint::Any,
                process: ProcessContents::Lax,
            }),
            min: 0,
            max: None,
        };
        // A lone ##any wildcard is always deterministic.
        let automaton = Automaton::compile(&any, &[]).ok().map(Box::new);
        let name = QName::new(Some(XS), "anyType");
        let id = self.push_type(
            TypeDef::Complex(ComplexType {
                name: Some(name.clone()),
                base: ANY_TYPE,
                is_abstract: false,
                content: automaton.map_or(Content::Empty, Content::Mixed),
                attributes: Arc::from([]),
                attribute_wildcard: Some(Wildcard {
                    namespaces: NamespaceConstraint::Any,
                    process: ProcessContents::Lax,
                }),
            }),
            BuildState::Built,
        );
        self.register_type(&name, id);
        let mut ids: HashMap<Builtin, TypeId> = HashMap::new();
        for builtin in Builtin::ALL {
            let base = builtin
                .base()
                .and_then(|b| ids.get(&b).copied())
                .unwrap_or(ANY_TYPE);
            let name = QName::new(Some(XS), builtin.name());
            let id = self.push_type(
                TypeDef::Simple(SimpleType {
                    name: Some(name.clone()),
                    base,
                    builtin,
                    enumeration: None,
                    min_length: None,
                    max_length: None,
                }),
                BuildState::Built,
            );
            ids.insert(builtin, id);
            self.register_type(&name, id);
        }
    }

    /// Registers the global components of every document, with placeholders.
    fn collect_globals(&mut self) -> Result<()> {
        for (doc_index, doc) in self.docs.iter().enumerate() {
            for child in children(doc, doc.xml.root_element())? {
                let kind = child.tag_name().name();
                match kind {
                    "import" | "include" => {}
                    "element" | "complexType" | "simpleType" => {
                        let local = child.attribute("name").ok_or_else(|| {
                            error(
                                doc,
                                child,
                                LoadErrorKind::Invalid,
                                format!("global xs:{kind} without a name"),
                            )
                        })?;
                        let name = QName::new(doc.target.as_deref(), local);
                        if kind == "element" {
                            self.elements.push(ElementDecl {
                                name: name.clone(),
                                type_id: ANY_TYPE,
                            });
                            let id = self.elements.len() - 1;
                            let fresh = self
                                .global_elements
                                .entry(name.namespace.clone())
                                .or_default()
                                .insert(name.local.clone(), id)
                                .is_none();
                            if !fresh {
                                return Err(error(
                                    doc,
                                    child,
                                    LoadErrorKind::Invalid,
                                    format!("duplicate global element {name}"),
                                ));
                            }
                            self.element_sources.push((id, doc_index, child.id()));
                        } else {
                            let placeholder = TypeDef::Simple(SimpleType {
                                name: Some(name.clone()),
                                base: ANY_TYPE,
                                builtin: Builtin::AnySimpleType,
                                enumeration: None,
                                min_length: None,
                                max_length: None,
                            });
                            let id = self.push_type(placeholder, BuildState::Pending);
                            if !self.register_type(&name, id) {
                                return Err(error(
                                    doc,
                                    child,
                                    LoadErrorKind::Invalid,
                                    format!("duplicate global type {name}"),
                                ));
                            }
                            self.type_sources.insert(id, (doc_index, child.id()));
                        }
                    }
                    other => {
                        return Err(error(
                            doc,
                            child,
                            LoadErrorKind::Unsupported,
                            format!("top-level xs:{other}"),
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    fn doc(&self, index: usize) -> &'d Doc<'a> {
        &self.docs[index]
    }

    fn node(&self, doc: usize, id: NodeId) -> Node<'d, 'a> {
        let xml = &self.doc(doc).xml;
        // Ids are taken from the same documents, so the lookup cannot fail; the
        // fallback only avoids a panic path.
        xml.get_node(id).unwrap_or_else(|| xml.root())
    }

    /// Resolves a `QName` attribute value of a schema element to a global type.
    fn resolve_type(&mut self, doc: usize, node: Node<'_, '_>, value: &str) -> Result<TypeId> {
        let d = self.doc(doc);
        let name = self.resolve_qname(doc, node, value)?;
        if let Some(id) = self
            .global_types
            .get(&name.namespace)
            .and_then(|m| m.get(&name.local))
        {
            return Ok(*id);
        }
        if name.namespace.as_deref() == Some(XS)
            && UNIMPLEMENTED_BUILTINS.contains(&name.local.as_str())
        {
            return Err(error(
                d,
                node,
                LoadErrorKind::Unsupported,
                format!("built-in type {name}"),
            ));
        }
        Err(self.unresolved(doc, node, "type", &name))
    }

    /// The error for a reference to a global component that does not exist: a
    /// [`LoadErrorKind::MissingFile`] when the component is in a namespace whose
    /// import was omitted (and that no loaded document has as target namespace),
    /// else [`LoadErrorKind::Invalid`].
    fn unresolved(&self, doc: usize, node: Node<'_, '_>, what: &str, name: &QName) -> LoadError {
        let d = self.doc(doc);
        let omitted = self
            .omitted
            .iter()
            .find(|o| name.namespace.as_deref() == Some(o.namespace.as_str()))
            .filter(|_| !self.docs.iter().any(|x| x.target == name.namespace));
        match omitted {
            Some(import) => error(
                d,
                node,
                LoadErrorKind::MissingFile,
                format!(
                    "{what} {name}: its namespace is not loaded, because {} (imported at {}:{}) is not in the schema set",
                    import.location, import.file, import.line
                ),
            ),
            None => error(
                d,
                node,
                LoadErrorKind::Invalid,
                format!("unresolved {what} {name}"),
            ),
        }
    }

    fn resolve_qname(&self, doc: usize, node: Node<'_, '_>, value: &str) -> Result<QName> {
        let d = self.doc(doc);
        let collapsed = datatypes::normalize(value, datatypes::WhiteSpace::Collapse);
        let (prefix, local) = datatypes::split_qname(&collapsed).ok_or_else(|| {
            error(
                d,
                node,
                LoadErrorKind::Invalid,
                format!("{value:?} is not a QName"),
            )
        })?;
        let namespace = match prefix {
            Some(p) => Some(node.lookup_namespace_uri(Some(p)).ok_or_else(|| {
                error(
                    d,
                    node,
                    LoadErrorKind::Invalid,
                    format!("undeclared prefix {p:?}"),
                )
            })?),
            None => crate::model::namespace_name(node.lookup_namespace_uri(None)),
        };
        Ok(QName::new(namespace, local))
    }

    fn build_type(&mut self, id: TypeId) -> Result<()> {
        match self.state[id] {
            BuildState::Built => return Ok(()),
            BuildState::Building => {
                let (doc, node) = self.type_sources[&id];
                let d = self.doc(doc);
                return Err(error(
                    d,
                    self.node(doc, node),
                    LoadErrorKind::Invalid,
                    "circular type derivation",
                ));
            }
            BuildState::Pending => {}
        }
        self.state[id] = BuildState::Building;
        let (doc, node_id) = self.type_sources[&id];
        let docs = self.docs;
        let node = docs[doc]
            .xml
            .get_node(node_id)
            .unwrap_or_else(|| docs[doc].xml.root());
        let name = self.types[id].name().cloned();
        let def = if is_xs(node, "simpleType") {
            TypeDef::Simple(self.simple_type(doc, node, name)?)
        } else {
            TypeDef::Complex(self.complex_type(doc, node, name)?)
        };
        self.types[id] = def;
        self.state[id] = BuildState::Built;
        Ok(())
    }

    fn simple_type(
        &mut self,
        doc: usize,
        node: Node<'_, '_>,
        name: Option<QName>,
    ) -> Result<SimpleType> {
        let d = self.doc(doc);
        check_attributes(d, node, &["name"])?;
        let [restriction] = children(d, node)?[..] else {
            return Err(error(
                d,
                node,
                LoadErrorKind::Invalid,
                "xs:simpleType needs exactly one derivation",
            ));
        };
        if !is_xs(restriction, "restriction") {
            return Err(error(
                d,
                restriction,
                LoadErrorKind::Unsupported,
                format!("xs:simpleType by xs:{}", restriction.tag_name().name()),
            ));
        }
        check_attributes(d, restriction, &["base"])?;
        let base_name = restriction.attribute("base").ok_or_else(|| {
            error(
                d,
                restriction,
                LoadErrorKind::Unsupported,
                "xs:restriction without base",
            )
        })?;
        let base = self.resolve_type(doc, restriction, base_name)?;
        self.build_type(base)?;
        let d = self.doc(doc);
        let TypeDef::Simple(base_def) = &self.types[base] else {
            return Err(error(
                d,
                restriction,
                LoadErrorKind::Invalid,
                "simple type restricting a complex type",
            ));
        };
        let builtin = base_def.builtin;
        let Facets {
            enumeration,
            min_length,
            max_length,
        } = facets(d, restriction, builtin)?;
        let string_based = matches!(
            builtin,
            Builtin::String | Builtin::NormalizedString | Builtin::Token
        );
        if !enumeration.is_empty() && !string_based {
            return Err(error(
                d,
                restriction,
                LoadErrorKind::Unsupported,
                format!("enumeration on a type derived from xs:{}", builtin.name()),
            ));
        }
        self.check_length_restriction(doc, restriction, base, min_length, max_length)?;
        // Part 2, 4.3.5.4: every enumerated value is a value of the base type.
        for value in &enumeration {
            if let Some(problem) = facet_problem(&self.types, base, value) {
                let base_name = self.type_name(base);
                return Err(error(
                    self.doc(doc),
                    restriction,
                    LoadErrorKind::Invalid,
                    format!(
                        "the enumerated value {} is {}",
                        quote(value),
                        problem.describe(&base_name)
                    ),
                ));
            }
        }
        Ok(SimpleType {
            name,
            base,
            builtin,
            enumeration: (!enumeration.is_empty()).then_some(enumeration),
            min_length,
            max_length,
        })
    }

    /// The name of a type for a message.
    fn type_name(&self, type_id: TypeId) -> String {
        self.types[type_id]
            .name()
            .map_or_else(|| "an anonymous type".to_owned(), ToString::to_string)
    }

    /// Checks the length facets of a restriction step against each other and
    /// against those it inherits from its base (Part 2, 4.3.2.4 and 4.3.3.4:
    /// `minLength` at most `maxLength`, `minLength` not below the base's,
    /// `maxLength` not above it).
    fn check_length_restriction(
        &self,
        doc: usize,
        restriction: Node<'_, '_>,
        base: TypeId,
        min_length: Option<usize>,
        max_length: Option<usize>,
    ) -> Result<()> {
        let mut base_min = None;
        let mut base_max = None;
        let mut current = base;
        while let TypeDef::Simple(step) = &self.types[current] {
            base_min = base_min.or(step.min_length);
            base_max = base_max.or(step.max_length);
            current = step.base;
        }
        let invalid = |message: String| {
            Err(error(
                self.doc(doc),
                restriction,
                LoadErrorKind::Invalid,
                message,
            ))
        };
        if let (Some(min), Some(base_min)) = (min_length, base_min)
            && min < base_min
        {
            return invalid(format!(
                "minLength {min} is less than the minLength {base_min} of the base type"
            ));
        }
        if let (Some(max), Some(base_max)) = (max_length, base_max)
            && max > base_max
        {
            return invalid(format!(
                "maxLength {max} is greater than the maxLength {base_max} of the base type"
            ));
        }
        if let (Some(min), Some(max)) = (min_length.or(base_min), max_length.or(base_max))
            && min > max
        {
            return invalid(format!("minLength {min} is greater than maxLength {max}"));
        }
        Ok(())
    }

    fn complex_type(
        &mut self,
        doc: usize,
        node: Node<'_, '_>,
        name: Option<QName>,
    ) -> Result<ComplexType> {
        let d = self.doc(doc);
        check_attributes(d, node, &["name", "mixed", "abstract"])?;
        let flag = |attribute: &str| -> Result<bool> {
            node.attribute(attribute).map_or(Ok(false), |v| {
                parse_boolean(v).ok_or_else(|| {
                    error(
                        d,
                        node,
                        LoadErrorKind::Invalid,
                        format!("{attribute}={v:?}"),
                    )
                })
            })
        };
        let mixed = flag("mixed")?;
        let is_abstract = flag("abstract")?;
        let items = children(d, node)?;
        match items.first() {
            Some(&first) if is_xs(first, "simpleContent") => {
                if items.len() > 1 {
                    return Err(error(
                        d,
                        node,
                        LoadErrorKind::Invalid,
                        "content after xs:simpleContent",
                    ));
                }
                if mixed {
                    return Err(error(
                        d,
                        node,
                        LoadErrorKind::Invalid,
                        "mixed type with simple content",
                    ));
                }
                self.simple_content(doc, first, name, is_abstract)
            }
            Some(&first) if is_xs(first, "complexContent") => {
                if items.len() > 1 {
                    return Err(error(
                        d,
                        node,
                        LoadErrorKind::Invalid,
                        "content after xs:complexContent",
                    ));
                }
                self.complex_content(doc, first, name, is_abstract, mixed)
            }
            _ => {
                let (particle, attributes, wildcard) =
                    self.particle_and_attributes(doc, node, &items)?;
                let content = self.content(doc, node, particle, mixed)?;
                Ok(ComplexType {
                    name,
                    base: ANY_TYPE,
                    is_abstract,
                    content,
                    attributes: attributes.into(),
                    attribute_wildcard: wildcard,
                })
            }
        }
    }

    /// An optional particle (`xs:sequence` or `xs:choice`) followed by attribute
    /// declarations and an optional `xs:anyAttribute`.
    fn particle_and_attributes(
        &mut self,
        doc: usize,
        parent: Node<'_, '_>,
        items: &[Node<'_, '_>],
    ) -> Result<(Option<Particle>, Vec<AttributeUse>, Option<Wildcard>)> {
        let mut rest = items;
        let mut particle = None;
        if let Some((&first, tail)) = rest.split_first()
            && (is_xs(first, "sequence") || is_xs(first, "choice"))
        {
            particle = Some(self.particle(doc, first)?);
            rest = tail;
        }
        let (attributes, wildcard) = self.attributes(doc, parent, rest)?;
        Ok((particle, attributes, wildcard))
    }

    fn attributes(
        &mut self,
        doc: usize,
        parent: Node<'_, '_>,
        items: &[Node<'_, '_>],
    ) -> Result<(Vec<AttributeUse>, Option<Wildcard>)> {
        let mut uses: Vec<AttributeUse> = Vec::new();
        let mut wildcard = None;
        for &item in items {
            let d = self.doc(doc);
            if wildcard.is_some() {
                return Err(error(
                    d,
                    item,
                    LoadErrorKind::Invalid,
                    "content after xs:anyAttribute",
                ));
            }
            if is_xs(item, "attribute") {
                let attribute = self.attribute(doc, item)?;
                if uses.iter().any(|u| u.name == attribute.name) {
                    return Err(error(
                        d,
                        item,
                        LoadErrorKind::Invalid,
                        format!("duplicate attribute {}", attribute.name),
                    ));
                }
                uses.push(attribute);
            } else if is_xs(item, "anyAttribute") {
                check_attributes(d, item, &["namespace", "processContents"])?;
                if !children(d, item)?.is_empty() {
                    return Err(error(
                        d,
                        item,
                        LoadErrorKind::Invalid,
                        "content in xs:anyAttribute",
                    ));
                }
                wildcard = Some(self.wildcard(doc, item)?);
            } else {
                return Err(error(
                    d,
                    item,
                    LoadErrorKind::Unsupported,
                    format!(
                        "xs:{} in xs:{}",
                        item.tag_name().name(),
                        parent.tag_name().name()
                    ),
                ));
            }
        }
        Ok((uses, wildcard))
    }

    fn attribute(&mut self, doc: usize, node: Node<'_, '_>) -> Result<AttributeUse> {
        let d = self.doc(doc);
        check_attributes(d, node, &["name", "type", "use", "default", "fixed"])?;
        if !children(d, node)?.is_empty() {
            return Err(error(
                d,
                node,
                LoadErrorKind::Unsupported,
                "anonymous attribute type",
            ));
        }
        let local = node.attribute("name").ok_or_else(|| {
            error(
                d,
                node,
                LoadErrorKind::Unsupported,
                "xs:attribute without a name",
            )
        })?;
        let required = match node.attribute("use").map(str::trim) {
            None | Some("optional") => false,
            Some("required") => true,
            Some(other) => {
                return Err(error(
                    d,
                    node,
                    LoadErrorKind::Unsupported,
                    format!("use={other:?}"),
                ));
            }
        };
        let namespace = if d.qualified_attributes {
            d.target.clone()
        } else {
            None
        };
        let type_name = node.attribute("type").ok_or_else(|| {
            error(
                d,
                node,
                LoadErrorKind::Unsupported,
                "xs:attribute without a type",
            )
        })?;
        let constraint = value_constraint(d, node)?;
        if required
            && let Some(ValueConstraint {
                kind: ConstraintKind::Default,
                ..
            }) = constraint
        {
            // src-attribute.2.
            return Err(error(
                d,
                node,
                LoadErrorKind::Invalid,
                "an attribute with a default value must have use=\"optional\"",
            ));
        }
        let type_id = self.resolve_type(doc, node, type_name)?;
        self.simple_type_checks.push((type_id, doc, node.id()));
        let fixed = match &constraint {
            Some(ValueConstraint {
                kind: ConstraintKind::Fixed,
                value,
            }) => {
                let fixed = FixedValue::new(self.fixed_values.len()).ok_or_else(|| {
                    error(
                        self.doc(doc),
                        node,
                        LoadErrorKind::Unsupported,
                        "more than 2^32 - 1 fixed attribute values",
                    )
                })?;
                self.fixed_values.push(value.clone());
                Some(fixed)
            }
            _ => None,
        };
        if let Some(constraint) = constraint {
            self.attribute_constraints.push(PendingConstraint {
                type_id,
                constraint,
                doc,
                node: node.id(),
            });
        }
        Ok(AttributeUse {
            name: QName {
                namespace,
                local: local.to_owned(),
            },
            type_id,
            required,
            fixed,
        })
    }

    fn wildcard(&self, doc: usize, node: Node<'_, '_>) -> Result<Wildcard> {
        let d = self.doc(doc);
        let namespaces = match node.attribute("namespace").map(str::trim) {
            None | Some("##any") => NamespaceConstraint::Any,
            Some("##other") => NamespaceConstraint::Other(d.target.clone()),
            Some(other) => {
                return Err(error(
                    d,
                    node,
                    LoadErrorKind::Unsupported,
                    format!("wildcard namespace={other:?}"),
                ));
            }
        };
        let process = match node.attribute("processContents").map(str::trim) {
            None | Some("strict") => ProcessContents::Strict,
            Some("lax") => ProcessContents::Lax,
            Some("skip") => ProcessContents::Skip,
            Some(other) => {
                return Err(error(
                    d,
                    node,
                    LoadErrorKind::Invalid,
                    format!("processContents={other:?}"),
                ));
            }
        };
        Ok(Wildcard {
            namespaces,
            process,
        })
    }

    fn particle(&mut self, doc: usize, node: Node<'_, '_>) -> Result<Particle> {
        let d = self.doc(doc);
        let kind = node.tag_name().name();
        match kind {
            "sequence" | "choice" => {
                check_attributes(d, node, &["minOccurs", "maxOccurs"])?;
                let (min, max) = occurs(d, node)?;
                let mut items = Vec::new();
                for child in children(d, node)? {
                    items.push(self.particle(doc, child)?);
                }
                let term = if kind == "sequence" {
                    ParticleTerm::Sequence(items)
                } else {
                    ParticleTerm::Choice(items)
                };
                Ok(Particle { term, min, max })
            }
            "any" => {
                check_attributes(
                    d,
                    node,
                    &["namespace", "processContents", "minOccurs", "maxOccurs"],
                )?;
                if !children(d, node)?.is_empty() {
                    return Err(error(d, node, LoadErrorKind::Invalid, "content in xs:any"));
                }
                let (min, max) = occurs(d, node)?;
                Ok(Particle {
                    term: ParticleTerm::Wildcard(self.wildcard(doc, node)?),
                    min,
                    max,
                })
            }
            "element" => self.local_element(doc, node),
            other => Err(error(
                d,
                node,
                LoadErrorKind::Unsupported,
                format!("particle xs:{other}"),
            )),
        }
    }

    /// An `xs:element` particle: a reference to a global declaration, or a local
    /// declaration (a new element declaration).
    fn local_element(&mut self, doc: usize, node: Node<'_, '_>) -> Result<Particle> {
        let d = self.doc(doc);
        check_attributes(
            d,
            node,
            &[
                "name",
                "type",
                "ref",
                "minOccurs",
                "maxOccurs",
                "default",
                "fixed",
            ],
        )?;
        // The only content supported: one anonymous xs:complexType.
        let anonymous = match children(d, node)?[..] {
            [] => None,
            [child] if is_xs(child, "complexType") => Some(child),
            [first, ref rest @ ..] => {
                let child = match rest.first() {
                    Some(&second) if is_xs(first, "complexType") => second,
                    _ => first,
                };
                let is_type = |n: Node<'_, '_>| is_xs(n, "complexType") || is_xs(n, "simpleType");
                if is_type(first) && is_type(child) && first != child {
                    // At most one anonymous type (the schema for schemas).
                    return Err(error(
                        d,
                        child,
                        LoadErrorKind::Invalid,
                        "a local xs:element with two anonymous types",
                    ));
                }
                return Err(error(
                    d,
                    child,
                    LoadErrorKind::Unsupported,
                    format!("xs:{} in a local xs:element", child.tag_name().name()),
                ));
            }
        };
        let (min, max) = occurs(d, node)?;
        let id = match (node.attribute("ref"), node.attribute("name")) {
            (Some(reference), None) => {
                // src-element.2.2: a reference has no type and no value constraint
                // of its own.
                if node.attribute("type").is_some() || anonymous.is_some() {
                    return Err(error(
                        d,
                        node,
                        LoadErrorKind::Invalid,
                        "xs:element with both ref and a type",
                    ));
                }
                if node.attribute("default").is_some() || node.attribute("fixed").is_some() {
                    return Err(error(
                        d,
                        node,
                        LoadErrorKind::Invalid,
                        "xs:element with both ref and a value constraint",
                    ));
                }
                let name = self.resolve_qname(doc, node, reference)?;
                self.global_elements
                    .get(&name.namespace)
                    .and_then(|m| m.get(&name.local))
                    .copied()
                    .ok_or_else(|| self.unresolved(doc, node, "element", &name))?
            }
            (None, Some(local)) => self.local_declaration(doc, node, local, anonymous)?,
            _ => {
                return Err(error(
                    d,
                    node,
                    LoadErrorKind::Invalid,
                    "xs:element needs exactly one of name and ref",
                ));
            }
        };
        Ok(Particle {
            term: ParticleTerm::Element(id),
            min,
            max,
        })
    }

    /// A local element declaration named `local`, typed by its `type` attribute, by
    /// its anonymous complex type `anonymous`, or else `xs:anyType`.
    fn local_declaration(
        &mut self,
        doc: usize,
        node: Node<'_, '_>,
        local: &str,
        anonymous: Option<Node<'_, '_>>,
    ) -> Result<ElementId> {
        let d = self.doc(doc);
        let namespace = if d.qualified_elements {
            d.target.clone()
        } else {
            None
        };
        let constraint = value_constraint(d, node)?;
        let type_id = match (node.attribute("type"), anonymous) {
            (Some(_), Some(_)) => {
                // src-element.3.
                return Err(error(
                    d,
                    node,
                    LoadErrorKind::Invalid,
                    "xs:element with both a type attribute and an anonymous type",
                ));
            }
            (Some(t), None) => self.resolve_type(doc, node, t)?,
            (None, Some(definition)) => self.anonymous_complex_type(doc, definition)?,
            (None, None) => ANY_TYPE,
        };
        self.elements.push(ElementDecl {
            name: QName {
                namespace,
                local: local.to_owned(),
            },
            type_id,
        });
        let id = self.elements.len() - 1;
        if let Some(constraint) = constraint {
            self.element_constraints.push((
                id,
                PendingConstraint {
                    type_id,
                    constraint,
                    doc,
                    node: node.id(),
                },
            ));
        }
        Ok(id)
    }

    /// The anonymous complex type of a local element declaration, built and added
    /// to the type table.
    fn anonymous_complex_type(&mut self, doc: usize, node: Node<'_, '_>) -> Result<TypeId> {
        let d = self.doc(doc);
        // A local complex type has no name, and no abstract, final or block (the
        // schema for schemas, localComplexType).
        for attribute in ["name", "abstract", "final", "block"] {
            if node.attribute(attribute).is_some() {
                return Err(error(
                    d,
                    node,
                    LoadErrorKind::Invalid,
                    format!("attribute {attribute} on a local xs:complexType"),
                ));
            }
        }
        check_attributes(d, node, &["mixed"])?;
        let definition = self.complex_type(doc, node, None)?;
        Ok(self.push_type(TypeDef::Complex(definition), BuildState::Built))
    }

    fn content(
        &self,
        doc: usize,
        node: Node<'_, '_>,
        particle: Option<Particle>,
        mixed: bool,
    ) -> Result<Content> {
        let particle = particle.unwrap_or(Particle {
            term: ParticleTerm::Sequence(Vec::new()),
            min: 1,
            max: Some(1),
        });
        let automaton = Automaton::compile(&particle, &self.elements)
            .map_err(|message| error(self.doc(doc), node, LoadErrorKind::Invalid, message))?;
        Ok(match (automaton.is_empty(), mixed) {
            (_, true) => Content::Mixed(Box::new(automaton)),
            (true, false) => Content::Empty,
            (false, false) => Content::ElementOnly(Box::new(automaton)),
        })
    }

    fn base_complex(&mut self, doc: usize, node: Node<'_, '_>) -> Result<TypeId> {
        let d = self.doc(doc);
        check_attributes(d, node, &["base"])?;
        let base_name = node
            .attribute("base")
            .ok_or_else(|| error(d, node, LoadErrorKind::Invalid, "derivation without base"))?;
        let base = self.resolve_type(doc, node, base_name)?;
        self.build_type(base)?;
        Ok(base)
    }

    /// The content, attribute uses and attribute wildcard of a simple-content
    /// extension of `base` (a simple type, or a complex type with simple content).
    fn simple_extension(
        &self,
        d: &Doc<'_>,
        derivation: Node<'_, '_>,
        base: TypeId,
        own: Vec<AttributeUse>,
        own_wildcard: Option<Wildcard>,
    ) -> Result<(Content, Arc<[AttributeUse]>, Option<Wildcard>)> {
        let (content_type, base_attributes, base_wildcard) = match &self.types[base] {
            TypeDef::Simple(_) => (base, Arc::from([]), None),
            TypeDef::Complex(ComplexType {
                content: Content::Simple(s),
                attributes,
                attribute_wildcard,
                ..
            }) => (*s, Arc::clone(attributes), attribute_wildcard.clone()),
            TypeDef::Complex(_) => {
                return Err(error(
                    d,
                    derivation,
                    LoadErrorKind::Invalid,
                    "simple content extending a type without simple content",
                ));
            }
        };
        // Without attributes of its own, the extension shares the base's list.
        let attributes = if own.is_empty() {
            base_attributes
        } else {
            let mut attributes = base_attributes.to_vec();
            for attribute in own {
                if attributes.iter().any(|a| a.name == attribute.name) {
                    return Err(error(
                        d,
                        derivation,
                        LoadErrorKind::Invalid,
                        format!("attribute {} already in the base type", attribute.name),
                    ));
                }
                attributes.push(attribute);
            }
            attributes.into()
        };
        let attribute_wildcard = match (base_wildcard, own_wildcard) {
            (Some(_), Some(_)) => {
                return Err(error(
                    d,
                    derivation,
                    LoadErrorKind::Unsupported,
                    "union of attribute wildcards",
                ));
            }
            (a, b) => a.or(b),
        };
        Ok((
            Content::Simple(content_type),
            attributes,
            attribute_wildcard,
        ))
    }

    fn simple_content(
        &mut self,
        doc: usize,
        node: Node<'_, '_>,
        name: Option<QName>,
        is_abstract: bool,
    ) -> Result<ComplexType> {
        let d = self.doc(doc);
        check_attributes(d, node, &[])?;
        let [derivation] = children(d, node)?[..] else {
            return Err(error(
                d,
                node,
                LoadErrorKind::Invalid,
                "xs:simpleContent needs exactly one derivation",
            ));
        };
        let base = self.base_complex(doc, derivation)?;
        let items = children(self.doc(doc), derivation)?;
        let (own, own_wildcard) = self.attributes(doc, derivation, &items)?;
        let d = self.doc(doc);
        if is_xs(derivation, "extension") {
            let (content, attributes, attribute_wildcard) =
                self.simple_extension(d, derivation, base, own, own_wildcard)?;
            Ok(ComplexType {
                name,
                base,
                is_abstract,
                content,
                attributes,
                attribute_wildcard,
            })
        } else if is_xs(derivation, "restriction") {
            let TypeDef::Complex(ComplexType {
                content: Content::Simple(content_type),
                attributes: base_attributes,
                ..
            }) = &self.types[base]
            else {
                return Err(error(
                    d,
                    derivation,
                    LoadErrorKind::Invalid,
                    "simple content restricting a type without simple content",
                ));
            };
            if own_wildcard.is_some() {
                return Err(error(
                    d,
                    derivation,
                    LoadErrorKind::Unsupported,
                    "xs:anyAttribute in a restriction",
                ));
            }
            let attributes = restrict_attributes(d, derivation, base_attributes, own)?;
            Ok(ComplexType {
                name,
                base,
                is_abstract,
                content: Content::Simple(*content_type),
                attributes,
                attribute_wildcard: None,
            })
        } else {
            Err(error(
                d,
                derivation,
                LoadErrorKind::Invalid,
                format!("xs:{} in xs:simpleContent", derivation.tag_name().name()),
            ))
        }
    }

    fn complex_content(
        &mut self,
        doc: usize,
        node: Node<'_, '_>,
        name: Option<QName>,
        is_abstract: bool,
        mixed: bool,
    ) -> Result<ComplexType> {
        let d = self.doc(doc);
        check_attributes(d, node, &[])?;
        let [derivation] = children(d, node)?[..] else {
            return Err(error(
                d,
                node,
                LoadErrorKind::Invalid,
                "xs:complexContent needs exactly one derivation",
            ));
        };
        if !is_xs(derivation, "restriction") {
            return Err(error(
                d,
                derivation,
                LoadErrorKind::Unsupported,
                format!("xs:{} in xs:complexContent", derivation.tag_name().name()),
            ));
        }
        let base = self.base_complex(doc, derivation)?;
        let items = children(self.doc(doc), derivation)?;
        let (particle, own, own_wildcard) =
            self.particle_and_attributes(doc, derivation, &items)?;
        let d = self.doc(doc);
        let TypeDef::Complex(base_type) = &self.types[base] else {
            return Err(error(
                d,
                derivation,
                LoadErrorKind::Invalid,
                "complex content restricting a simple type",
            ));
        };
        if matches!(base_type.content, Content::Simple(_)) {
            return Err(error(
                d,
                derivation,
                LoadErrorKind::Invalid,
                "complex content restricting a type with simple content",
            ));
        }
        if own_wildcard.is_some() {
            return Err(error(
                d,
                derivation,
                LoadErrorKind::Unsupported,
                "xs:anyAttribute in a restriction",
            ));
        }
        let attributes = restrict_attributes(d, derivation, &base_type.attributes, own)?;
        let content = self.content(doc, derivation, particle, mixed)?;
        Ok(ComplexType {
            name,
            base,
            is_abstract,
            content,
            attributes,
            attribute_wildcard: None,
        })
    }

    fn resolve_global_elements(&mut self) -> Result<()> {
        let sources = std::mem::take(&mut self.element_sources);
        for &(id, doc, node_id) in &sources {
            let node = self.node(doc, node_id);
            let d = self.doc(doc);
            check_attributes(d, node, &["name", "type", "default", "fixed"])?;
            if let Some(&child) = children(d, node)?.first() {
                return Err(error(
                    d,
                    child,
                    LoadErrorKind::Unsupported,
                    format!("xs:{} in a global xs:element", child.tag_name().name()),
                ));
            }
            let constraint = value_constraint(d, node)?;
            let docs = self.docs;
            let node = docs[doc]
                .xml
                .get_node(node_id)
                .unwrap_or_else(|| docs[doc].xml.root());
            if let Some(type_name) = node.attribute("type") {
                let type_id = self.resolve_type(doc, node, type_name)?;
                self.elements[id].type_id = type_id;
            }
            if let Some(constraint) = constraint {
                self.element_constraints.push((
                    id,
                    PendingConstraint {
                        type_id: self.elements[id].type_id,
                        constraint,
                        doc,
                        node: node_id,
                    },
                ));
            }
        }
        self.element_sources = sources;
        Ok(())
    }

    /// Checks every value constraint, now that the types are built, and returns
    /// those of element declarations, sorted by element.
    fn check_value_constraints(&mut self) -> Result<Vec<(ElementId, ValueConstraint)>> {
        for pending in std::mem::take(&mut self.attribute_constraints) {
            self.check_constraint(&pending, pending.type_id)?;
        }
        let mut constraints = Vec::new();
        for (id, pending) in std::mem::take(&mut self.element_constraints) {
            let d = self.doc(pending.doc);
            let node = self.node(pending.doc, pending.node);
            // e-props-correct.2 and cos-valid-default.2: the type must have
            // simple content, or mixed content that may be empty.
            let simple = match &self.types[pending.type_id] {
                TypeDef::Simple(_) => pending.type_id,
                TypeDef::Complex(ComplexType {
                    content: Content::Simple(simple),
                    ..
                }) => *simple,
                TypeDef::Complex(ComplexType {
                    content: Content::Mixed(automaton),
                    ..
                }) if automaton.accepts(None) => {
                    return Err(error(
                        d,
                        node,
                        LoadErrorKind::Unsupported,
                        format!(
                            "a {} value on an element with mixed content",
                            pending.constraint.kind.as_str()
                        ),
                    ));
                }
                TypeDef::Complex(_) => {
                    return Err(error(
                        d,
                        node,
                        LoadErrorKind::Invalid,
                        format!(
                            "a {} value on an element whose type has neither simple content nor mixed content that may be empty",
                            pending.constraint.kind.as_str()
                        ),
                    ));
                }
            };
            self.check_constraint(&pending, simple)?;
            constraints.push((id, pending.constraint));
        }
        constraints.sort_by_key(|(id, _)| *id);
        Ok(constraints)
    }

    /// Checks a value constraint against the simple type it constrains (the type
    /// of an attribute, the simple content of an element): the value must be valid
    /// for it (a-props-correct.2, e-props-correct.2), and the type must not be
    /// derived from `xs:ID` (a-props-correct.3, e-props-correct.4). A fixed value on
    /// a date or time type is refused: comparing such values is not implemented. An
    /// attribute whose type is not simple is reported by [`Self::check_simple_types`].
    fn check_constraint(&self, pending: &PendingConstraint, simple: TypeId) -> Result<()> {
        let TypeDef::Simple(definition) = &self.types[simple] else {
            return Ok(());
        };
        let d = self.doc(pending.doc);
        let node = self.node(pending.doc, pending.node);
        let kind = pending.constraint.kind.as_str();
        let builtin = definition.builtin;
        if builtin == Builtin::Id {
            return Err(error(
                d,
                node,
                LoadErrorKind::Invalid,
                format!("a {kind} value on a type derived from xs:ID"),
            ));
        }
        if pending.constraint.kind == ConstraintKind::Fixed && !builtin.has_value_key() {
            return Err(error(
                d,
                node,
                LoadErrorKind::Unsupported,
                format!(
                    "a fixed value on a type derived from xs:{}: comparing its values is not implemented",
                    builtin.name()
                ),
            ));
        }
        let value = &pending.constraint.value;
        let type_name = self.type_name(simple);
        if !builtin.is_valid_raw(value) {
            return Err(error(
                d,
                node,
                LoadErrorKind::Invalid,
                format!(
                    "the {kind} value {} is not a valid value of {type_name}",
                    quote(value)
                ),
            ));
        }
        let processed = datatypes::normalize(value, builtin.white_space());
        if let Some(problem) = facet_problem(&self.types, simple, &processed) {
            return Err(error(
                d,
                node,
                LoadErrorKind::Invalid,
                format!(
                    "the {kind} value {} is {}",
                    quote(value),
                    problem.describe(&type_name)
                ),
            ));
        }
        Ok(())
    }

    fn check_simple_types(&self) -> Result<()> {
        for &(type_id, doc, node_id) in &self.simple_type_checks {
            if !matches!(self.types[type_id], TypeDef::Simple(_)) {
                let d = self.doc(doc);
                return Err(error(
                    d,
                    self.node(doc, node_id),
                    LoadErrorKind::Invalid,
                    "attribute of a complex type",
                ));
            }
        }
        Ok(())
    }
}

/// The attribute uses of a restriction: the base's, each replaced by the
/// restriction's own use of the same name (the base's list itself, shared, when the
/// restriction has none). A restriction cannot add attributes (the base types of
/// the supported schemas have no attribute wildcard).
fn restrict_attributes(
    doc: &Doc<'_>,
    node: Node<'_, '_>,
    base: &Arc<[AttributeUse]>,
    own: Vec<AttributeUse>,
) -> Result<Arc<[AttributeUse]>> {
    if own.is_empty() {
        return Ok(Arc::clone(base));
    }
    let mut attributes = base.to_vec();
    for attribute in own {
        match attributes.iter_mut().find(|a| a.name == attribute.name) {
            Some(slot) => {
                if slot.fixed.is_some() {
                    // derivation-ok-restriction 2.1.3 requires the restriction to
                    // keep the base's fixed value; that check is not implemented.
                    return Err(error(
                        doc,
                        node,
                        LoadErrorKind::Unsupported,
                        format!(
                            "restriction of attribute {}, which has a fixed value in the base type",
                            attribute.name
                        ),
                    ));
                }
                *slot = attribute;
            }
            None => {
                return Err(error(
                    doc,
                    node,
                    LoadErrorKind::Invalid,
                    format!("restriction adds attribute {}", attribute.name),
                ));
            }
        }
    }
    Ok(attributes.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locations() {
        assert_eq!(
            resolve_location("xsd/maindoc/A.xsd", "../common/B.xsd").as_deref(),
            Some("xsd/common/B.xsd")
        );
        assert_eq!(
            resolve_location("xsd/common/A.xsd", "B.xsd").as_deref(),
            Some("xsd/common/B.xsd")
        );
        assert_eq!(resolve_location("A.xsd", "../B.xsd"), None);
        assert_eq!(resolve_location("A.xsd", "http://example.org/B.xsd"), None);
        assert_eq!(resolve_location("A.xsd", "/B.xsd"), None);
    }

    fn load_one(schema: &str) -> Result<Schema> {
        load(&[("s.xsd", schema)], &["s.xsd"])
    }

    fn kind_of(schema: &str) -> LoadErrorKind {
        match load_one(schema) {
            Ok(_) => panic!("loaded: {schema}"),
            Err(e) => e.kind,
        }
    }

    const HEAD: &str = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:t="urn:t" targetNamespace="urn:t" elementFormDefault="qualified">"#;

    #[test]
    fn refuses_unsupported_constructs() {
        for body in [
            r#"<xs:attribute name="a" type="xs:string"/>"#,
            r#"<xs:group name="g"><xs:sequence/></xs:group>"#,
            r#"<xs:element name="e" nillable="true"/>"#,
            r#"<xs:element name="e" abstract="true"/>"#,
            r#"<xs:element name="e"><xs:complexType/></xs:element>"#,
            r#"<xs:complexType name="c"><xs:all/></xs:complexType>"#,
            r###"<xs:complexType name="c"><xs:sequence><xs:any namespace="##local"/></xs:sequence></xs:complexType>"###,
            r#"<xs:complexType name="c"><xs:attribute name="a" type="xs:string" use="prohibited"/></xs:complexType>"#,
            r#"<xs:complexType name="c"><xs:attribute ref="t:a"/></xs:complexType>"#,
            r#"<xs:complexType name="c"><xs:complexContent><xs:extension base="xs:anyType"/></xs:complexContent></xs:complexType>"#,
            r#"<xs:simpleType name="s"><xs:restriction base="xs:string"><xs:length value="3"/></xs:restriction></xs:simpleType>"#,
            r#"<xs:simpleType name="s"><xs:restriction base="xs:string"><xs:maxLength value="3" fixed="true"/></xs:restriction></xs:simpleType>"#,
            r#"<xs:simpleType name="s"><xs:restriction base="xs:anyURI"><xs:maxLength value="3"/></xs:restriction></xs:simpleType>"#,
            r#"<xs:simpleType name="s"><xs:restriction base="xs:string"><xs:maxLength value="99999999999999999999999"/></xs:restriction></xs:simpleType>"#,
            // Fixed values of the date and time types are not compared.
            r#"<xs:element name="e" type="xs:date" fixed="2024-01-01"/>"#,
            r#"<xs:complexType name="c"><xs:attribute name="a" type="xs:time" fixed="10:00:00"/></xs:complexType>"#,
            // Value constraints on mixed content.
            r#"<xs:element name="e" fixed="x"/>"#,
            r#"<xs:complexType name="c" mixed="true"/><xs:element name="e" type="t:c" default="x"/>"#,
            // Restricting an attribute that has a fixed value.
            r#"<xs:complexType name="b"><xs:simpleContent><xs:extension base="xs:token"><xs:attribute name="a" type="xs:token" fixed="x"/></xs:extension></xs:simpleContent></xs:complexType><xs:complexType name="c"><xs:simpleContent><xs:restriction base="t:b"><xs:attribute name="a" type="xs:token" fixed="x"/></xs:restriction></xs:simpleContent></xs:complexType>"#,
            // Anonymous simple types, anything but one anonymous complex type.
            r#"<xs:complexType name="c"><xs:sequence><xs:element name="e"><xs:simpleType><xs:restriction base="xs:string"/></xs:simpleType></xs:element></xs:sequence></xs:complexType>"#,
            r#"<xs:complexType name="c"><xs:sequence><xs:element name="e"><xs:complexType/><xs:key name="k"/></xs:element></xs:sequence></xs:complexType>"#,
            r#"<xs:complexType name="c"><xs:sequence><xs:element name="e"><xs:complexType id="x"/></xs:element></xs:sequence></xs:complexType>"#,
            r#"<xs:simpleType name="s"><xs:list itemType="xs:string"/></xs:simpleType>"#,
            r#"<xs:simpleType name="s"><xs:restriction base="xs:float"/></xs:simpleType>"#,
            r#"<xs:simpleType name="s"><xs:restriction base="xs:decimal"><xs:enumeration value="1"/></xs:restriction></xs:simpleType>"#,
            r#"<xs:complexType name="c"><xs:sequence maxOccurs="1000"/></xs:complexType>"#,
            r#"<xs:import namespace="urn:x" schemaLocation="http://example.org/x.xsd"/>"#,
            r#"<xs:complexType name="c" block="extension"/>"#,
        ] {
            let schema = format!("{HEAD}{body}</xs:schema>");
            assert_eq!(kind_of(&schema), LoadErrorKind::Unsupported, "{body}");
        }
    }

    #[test]
    fn refuses_incorrect_schemas() {
        for body in [
            r#"<xs:element name="e" type="t:missing"/>"#,
            r#"<xs:element name="e"/><xs:element name="e"/>"#,
            r#"<xs:complexType name="a"><xs:simpleContent><xs:extension base="t:b"/></xs:simpleContent></xs:complexType><xs:complexType name="b"><xs:simpleContent><xs:extension base="t:a"/></xs:simpleContent></xs:complexType>"#,
            r#"<xs:element name="e"/><xs:complexType name="c"><xs:sequence><xs:element ref="t:e" minOccurs="0"/><xs:element ref="t:e"/></xs:sequence></xs:complexType>"#,
            r#"<xs:complexType name="c"><xs:sequence minOccurs="2" maxOccurs="1"/></xs:complexType>"#,
            r#"<xs:complexType name="c"><xs:attribute name="a" type="t:c"/></xs:complexType>"#,
            // Value constraints: both at once, default on a required attribute, not
            // a valid value of the type or of its facets, on an ID, on a type
            // without simple content, on a reference.
            r#"<xs:element name="e" type="xs:token" default="a" fixed="a"/>"#,
            r#"<xs:complexType name="c"><xs:attribute name="a" type="xs:token" default="a" fixed="a"/></xs:complexType>"#,
            r#"<xs:complexType name="c"><xs:attribute name="a" type="xs:token" default="a" use="required"/></xs:complexType>"#,
            r#"<xs:element name="e" type="xs:decimal" fixed="x"/>"#,
            r#"<xs:complexType name="c"><xs:attribute name="a" type="xs:boolean" default="yes"/></xs:complexType>"#,
            r#"<xs:simpleType name="s"><xs:restriction base="xs:token"><xs:maxLength value="2"/></xs:restriction></xs:simpleType><xs:complexType name="c"><xs:attribute name="a" type="t:s" fixed="abc"/></xs:complexType>"#,
            r#"<xs:simpleType name="s"><xs:restriction base="xs:token"><xs:enumeration value="x"/></xs:restriction></xs:simpleType><xs:element name="e" type="t:s" default="y"/>"#,
            r#"<xs:element name="e" type="xs:ID" fixed="a"/>"#,
            r#"<xs:complexType name="c"><xs:attribute name="a" type="xs:ID" default="a"/></xs:complexType>"#,
            r#"<xs:complexType name="c"><xs:sequence><xs:element name="x"/></xs:sequence></xs:complexType><xs:element name="e" type="t:c" fixed="a"/>"#,
            r#"<xs:complexType name="c"/><xs:element name="e" type="t:c" default="a"/>"#,
            r#"<xs:complexType name="c" mixed="true"><xs:sequence><xs:element name="x"/></xs:sequence></xs:complexType><xs:element name="e" type="t:c" fixed="a"/>"#,
            r#"<xs:element name="e" type="xs:token"/><xs:complexType name="c"><xs:sequence><xs:element ref="t:e" fixed="a"/></xs:sequence></xs:complexType>"#,
            // Length facets: not a nonNegativeInteger, on a type they do not apply
            // to, repeated, minLength above maxLength, loosening the base's.
            r#"<xs:simpleType name="s"><xs:restriction base="xs:token"><xs:maxLength value="-1"/></xs:restriction></xs:simpleType>"#,
            r#"<xs:simpleType name="s"><xs:restriction base="xs:token"><xs:maxLength value="1.0"/></xs:restriction></xs:simpleType>"#,
            r#"<xs:simpleType name="s"><xs:restriction base="xs:decimal"><xs:maxLength value="2"/></xs:restriction></xs:simpleType>"#,
            r#"<xs:simpleType name="s"><xs:restriction base="xs:token"><xs:maxLength value="2"/><xs:maxLength value="3"/></xs:restriction></xs:simpleType>"#,
            r#"<xs:simpleType name="s"><xs:restriction base="xs:token"><xs:minLength value="3"/><xs:maxLength value="2"/></xs:restriction></xs:simpleType>"#,
            r#"<xs:simpleType name="b"><xs:restriction base="xs:token"><xs:maxLength value="2"/></xs:restriction></xs:simpleType><xs:simpleType name="s"><xs:restriction base="t:b"><xs:maxLength value="3"/></xs:restriction></xs:simpleType>"#,
            r#"<xs:simpleType name="b"><xs:restriction base="xs:token"><xs:minLength value="2"/></xs:restriction></xs:simpleType><xs:simpleType name="s"><xs:restriction base="t:b"><xs:minLength value="1"/></xs:restriction></xs:simpleType>"#,
            r#"<xs:simpleType name="b"><xs:restriction base="xs:token"><xs:minLength value="2"/></xs:restriction></xs:simpleType><xs:simpleType name="s"><xs:restriction base="t:b"><xs:maxLength value="1"/></xs:restriction></xs:simpleType>"#,
            r#"<xs:simpleType name="b"><xs:restriction base="xs:token"><xs:maxLength value="2"/></xs:restriction></xs:simpleType><xs:simpleType name="s"><xs:restriction base="t:b"><xs:enumeration value="abc"/></xs:restriction></xs:simpleType>"#,
            // Anonymous complex types: with a type attribute, named, abstract.
            r#"<xs:complexType name="c"><xs:sequence><xs:element name="e" type="xs:token"><xs:complexType/></xs:element></xs:sequence></xs:complexType>"#,
            r#"<xs:complexType name="c"><xs:sequence><xs:element name="e"><xs:complexType name="d"/></xs:element></xs:sequence></xs:complexType>"#,
            r#"<xs:complexType name="c"><xs:sequence><xs:element name="e"><xs:complexType abstract="true"/></xs:element></xs:sequence></xs:complexType>"#,
            r#"<xs:complexType name="c"><xs:sequence><xs:element name="e"><xs:complexType block="extension"/></xs:element></xs:sequence></xs:complexType>"#,
            r#"<xs:element name="e" type="xs:token"/><xs:complexType name="c"><xs:sequence><xs:element ref="t:e"><xs:complexType/></xs:element></xs:sequence></xs:complexType>"#,
        ] {
            let schema = format!("{HEAD}{body}</xs:schema>");
            assert_eq!(kind_of(&schema), LoadErrorKind::Invalid, "{body}");
        }
    }

    #[test]
    fn value_constraints_length_facets_and_anonymous_types() {
        let schema = load_one(&format!(
            r#"{HEAD}
            <xs:simpleType name="code"><xs:restriction base="xs:token"><xs:minLength value=" 1 "/><xs:maxLength value="+3"/></xs:restriction></xs:simpleType>
            <xs:simpleType name="short"><xs:restriction base="t:code"><xs:maxLength value="2"/><xs:enumeration value="ab"/><xs:enumeration value="c"/></xs:restriction></xs:simpleType>
            <xs:complexType name="amount"><xs:simpleContent><xs:extension base="xs:decimal">
                <xs:attribute name="unit" type="t:code" fixed="EUR"/>
                <xs:attribute name="scale" type="xs:integer" default="2"/>
            </xs:extension></xs:simpleContent></xs:complexType>
            <xs:element name="root"/>
            <xs:element name="total" type="t:amount" default="0"/>
            <xs:complexType name="r"><xs:sequence>
                <xs:element name="when" minOccurs="0"><xs:complexType><xs:choice>
                    <xs:element name="text"><xs:complexType><xs:simpleContent><xs:extension base="xs:string"><xs:attribute name="format" type="xs:token" use="required"/></xs:extension></xs:simpleContent></xs:complexType></xs:element>
                    <xs:element name="date" type="xs:date" default="2024-01-01"/>
                </xs:choice></xs:complexType></xs:element>
                <xs:element name="flag" type="xs:boolean" fixed="true" minOccurs="0"/>
                <xs:element name="code" type="t:short" minOccurs="0"/>
            </xs:sequence></xs:complexType>
            <xs:element name="doc" type="t:r"/>
            </xs:schema>"#
        ))
        .unwrap();
        // anyType, the 16 built-ins, three named types and the two anonymous ones.
        assert_eq!(schema.type_count(), 1 + 16 + 4 + 2);
        let total = schema.global_element(Some("urn:t"), "total").unwrap();
        let constraint = schema.element_constraint(total).unwrap();
        assert_eq!(
            (constraint.kind, &*constraint.value),
            (ConstraintKind::Default, "0")
        );
        assert!(
            schema
                .element_constraint(schema.global_element(Some("urn:t"), "root").unwrap())
                .is_none()
        );
        // Three element declarations have a constraint: total, date and flag.
        assert_eq!(schema.element_constraints.len(), 3);
        assert!(
            schema
                .element_constraints
                .windows(2)
                .all(|w| w[0].0 < w[1].0)
        );
        let short = schema.global_type(Some("urn:t"), "short").unwrap();
        let TypeDef::Simple(short) = &schema.types[short] else {
            panic!("short is simple");
        };
        assert_eq!((short.min_length, short.max_length), (None, Some(2)));
        let code = schema.global_type(Some("urn:t"), "code").unwrap();
        let TypeDef::Simple(code) = &schema.types[code] else {
            panic!("code is simple");
        };
        assert_eq!((code.min_length, code.max_length), (Some(1), Some(3)));
    }

    #[test]
    fn missing_and_malformed_files() {
        let missing = load(&[], &["a.xsd"]).unwrap_err();
        assert_eq!(missing.kind, LoadErrorKind::MissingFile);
        let malformed = load(&[("a.xsd", "<xs:schema")], &["a.xsd"]).unwrap_err();
        assert_eq!(malformed.kind, LoadErrorKind::Malformed);
        let import =
            format!(r#"{HEAD}<xs:import namespace="urn:x" schemaLocation="x.xsd"/></xs:schema>"#);
        let wrong_namespace = load(
            &[
                ("s.xsd", import.as_str()),
                ("x.xsd", r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" targetNamespace="urn:y"/>"#),
            ],
            &["s.xsd"],
        )
        .unwrap_err();
        assert_eq!(wrong_namespace.kind, LoadErrorKind::Invalid);
    }

    const XADES: &str = "http://uri.etsi.org/01903/v1.3.2#";

    fn importing(namespace: &str, body: &str) -> String {
        format!(
            r#"{HEAD}<xs:import namespace="{namespace}" schemaLocation="x/X.xsd"/>{body}</xs:schema>"#
        )
    }

    #[test]
    fn xades_imports_may_be_absent() {
        let main = importing(XADES, r#"<xs:element name="e"/>"#);
        let schema = load(&[("s.xsd", main.as_str())], &["s.xsd"]).unwrap();
        assert_eq!(schema.documents, ["s.xsd"]);
        assert_eq!(
            schema.omitted_imports,
            [OmittedImport {
                file: "s.xsd".to_owned(),
                line: 1,
                namespace: XADES.to_owned(),
                location: "x/X.xsd".to_owned(),
            }]
        );
        // Present, it is loaded like any other import.
        let xades = format!(
            r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" targetNamespace="{XADES}"><xs:element name="Q"/></xs:schema>"#
        );
        let schema = load(
            &[("s.xsd", main.as_str()), ("x/X.xsd", xades.as_str())],
            &["s.xsd"],
        )
        .unwrap();
        assert_eq!(schema.documents, ["s.xsd", "x/X.xsd"]);
        assert_eq!(schema.omitted_imports, []);
        assert!(schema.global_element(Some(XADES), "Q").is_some());
        // Its location is still checked.
        let remote = format!(
            r#"{HEAD}<xs:import namespace="{XADES}" schemaLocation="http://example.org/X.xsd"/></xs:schema>"#
        );
        assert_eq!(kind_of(&remote), LoadErrorKind::Unsupported);
    }

    #[test]
    fn other_imports_may_not_be_absent() {
        for namespace in [
            "urn:x",
            "http://uri.etsi.org/01903/v1.3.2",
            "http://uri.etsi.org/01903/v1.3.3#",
        ] {
            let main = importing(namespace, "");
            let error = load(&[("s.xsd", main.as_str())], &["s.xsd"]).unwrap_err();
            assert_eq!(error.kind, LoadErrorKind::MissingFile, "{namespace}");
            assert_eq!(error.file, "x/X.xsd");
        }
    }

    #[test]
    fn references_into_an_omitted_namespace_are_refused() {
        for body in [
            r#"<xs:element name="e" type="x:T"/>"#,
            r#"<xs:complexType name="c"><xs:sequence><xs:element ref="x:Q"/></xs:sequence></xs:complexType>"#,
            r#"<xs:complexType name="c"><xs:simpleContent><xs:extension base="x:T"/></xs:simpleContent></xs:complexType>"#,
        ] {
            let main = importing(XADES, body).replacen(
                "<xs:schema ",
                &format!(r#"<xs:schema xmlns:x="{XADES}" "#),
                1,
            );
            let error = load_one(&main).unwrap_err();
            assert_eq!(error.kind, LoadErrorKind::MissingFile, "{body}");
            assert_eq!(error.file, "s.xsd", "{body}");
            assert!(
                error.message.contains("its namespace is not loaded")
                    && error.message.contains("x/X.xsd"),
                "{error}"
            );
        }
    }
}
