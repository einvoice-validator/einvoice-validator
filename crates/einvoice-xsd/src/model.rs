//! The compiled schema components.

use std::collections::HashMap;
use std::sync::Arc;

use crate::automaton::Automaton;
use crate::datatypes::Builtin;

/// The XML Schema namespace.
pub(crate) const XS: &str = "http://www.w3.org/2001/XMLSchema";
/// The XML Schema instance namespace.
pub(crate) const XSI: &str = "http://www.w3.org/2001/XMLSchema-instance";

pub(crate) type TypeId = usize;
pub(crate) type ElementId = usize;

/// The type `xs:anyType` is always the first type.
pub(crate) const ANY_TYPE: TypeId = 0;

/// An expanded name: namespace (absent for no namespace) and local name.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct QName {
    pub(crate) namespace: Option<String>,
    pub(crate) local: String,
}

impl QName {
    pub(crate) fn new(namespace: Option<&str>, local: &str) -> Self {
        Self {
            namespace: namespace.map(str::to_owned),
            local: local.to_owned(),
        }
    }

    pub(crate) fn matches(&self, namespace: Option<&str>, local: &str) -> bool {
        self.local == local && self.namespace.as_deref() == namespace
    }
}

/// A namespace name as the XML parser reports it, with the empty name (from
/// `xmlns=""`, which undeclares the default namespace) mapped to no namespace.
pub(crate) fn namespace_name(namespace: Option<&str>) -> Option<&str> {
    namespace.filter(|ns| !ns.is_empty())
}

/// Clark notation: `{namespace}local`, or `local` without a namespace.
pub(crate) fn clark(namespace: Option<&str>, local: &str) -> String {
    match namespace {
        Some(ns) => format!("{{{ns}}}{local}"),
        None => local.to_owned(),
    }
}

impl std::fmt::Display for QName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&clark(self.namespace.as_deref(), &self.local))
    }
}

/// An element declaration (global or local). Declarations are never nillable and
/// never abstract: the loader refuses those attributes.
#[derive(Debug, Clone)]
pub(crate) struct ElementDecl {
    pub(crate) name: QName,
    pub(crate) type_id: TypeId,
}

/// `processContents` of a wildcard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProcessContents {
    Strict,
    Lax,
    Skip,
}

/// The namespace constraint of a wildcard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NamespaceConstraint {
    /// `##any`.
    Any,
    /// `##other`: any namespace except the given target namespace, and not absent.
    Other(Option<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Wildcard {
    pub(crate) namespaces: NamespaceConstraint,
    pub(crate) process: ProcessContents,
}

impl Wildcard {
    pub(crate) fn allows(&self, namespace: Option<&str>) -> bool {
        match &self.namespaces {
            NamespaceConstraint::Any => true,
            NamespaceConstraint::Other(target) => {
                namespace.is_some() && namespace != target.as_deref()
            }
        }
    }
}

/// An attribute use of a complex type.
#[derive(Debug, Clone)]
pub(crate) struct AttributeUse {
    pub(crate) name: QName,
    /// A simple type.
    pub(crate) type_id: TypeId,
    pub(crate) required: bool,
    /// The `fixed` value constraint: a present attribute must have that value
    /// (cvc-attribute.4). A `default` has no effect on validity (it only supplies
    /// the value of an absent attribute in the post-schema-validation infoset,
    /// which this crate does not build), so it is checked when the schema is loaded
    /// and not kept.
    pub(crate) fixed: Option<FixedValue>,
}

/// A fixed attribute value, as written in the schema, kept in
/// [`Schema::fixed_values`]: an index rather than the text, so that an attribute use
/// is no larger than without it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FixedValue(std::num::NonZeroU32);

impl FixedValue {
    /// The handle of the value at `index` in [`Schema::fixed_values`]; `None` past
    /// `u32::MAX - 1` values.
    pub(crate) fn new(index: usize) -> Option<Self> {
        u32::try_from(index)
            .ok()
            .and_then(|i| i.checked_add(1))
            .and_then(std::num::NonZeroU32::new)
            .map(Self)
    }

    fn index(self) -> usize {
        // u32 to usize does not lose bits on the supported targets (32 and 64
        // bits); the fallback only avoids a panic path.
        usize::try_from(self.0.get() - 1).unwrap_or(usize::MAX)
    }
}

/// Whether the value constraint of an element declaration is a default or a fixed
/// value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConstraintKind {
    Default,
    Fixed,
}

impl ConstraintKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Fixed => "fixed",
        }
    }
}

/// The value constraint (`default` or `fixed`) of an element declaration, whose
/// type has simple content.
#[derive(Debug, Clone)]
pub(crate) struct ValueConstraint {
    pub(crate) kind: ConstraintKind,
    /// The value as written in the schema.
    pub(crate) value: Box<str>,
}

/// The content type of a complex type.
#[derive(Debug, Clone)]
pub(crate) enum Content {
    /// No children at all, not even whitespace.
    Empty,
    /// Text only, valid for the given simple type.
    Simple(TypeId),
    /// Child elements, whitespace between them.
    ElementOnly(Box<Automaton>),
    /// Child elements and text.
    Mixed(Box<Automaton>),
}

#[derive(Debug, Clone)]
pub(crate) struct ComplexType {
    pub(crate) name: Option<QName>,
    /// The base type (`anyType` is its own base). Whether by extension or
    /// restriction does not matter here: no derivation is blocked in the subset.
    pub(crate) base: TypeId,
    pub(crate) is_abstract: bool,
    pub(crate) content: Content,
    /// Shared with the base type when the derivation adds or changes none.
    pub(crate) attributes: Arc<[AttributeUse]>,
    pub(crate) attribute_wildcard: Option<Wildcard>,
}

#[derive(Debug, Clone)]
pub(crate) struct SimpleType {
    pub(crate) name: Option<QName>,
    /// The base type: a simple type, or `anyType` for `anySimpleType`.
    pub(crate) base: TypeId,
    /// The built-in type at the root of the derivation chain below
    /// `anySimpleType` that decides whitespace and lexical space (for built-ins,
    /// the type itself).
    pub(crate) builtin: Builtin,
    /// The `enumeration` facet of this restriction step, if any.
    pub(crate) enumeration: Option<Vec<String>>,
    /// The `minLength` facet of this restriction step, if any (types derived from
    /// `xs:string` only: a length in characters).
    pub(crate) min_length: Option<usize>,
    /// The `maxLength` facet of this restriction step, if any.
    pub(crate) max_length: Option<usize>,
}

impl SimpleType {
    /// Whether this restriction step has a facet to check besides the lexical
    /// space of its built-in type.
    pub(crate) fn has_facets(&self) -> bool {
        self.enumeration.is_some() || self.min_length.is_some() || self.max_length.is_some()
    }
}

#[derive(Debug, Clone)]
pub(crate) enum TypeDef {
    Simple(SimpleType),
    Complex(ComplexType),
}

impl TypeDef {
    pub(crate) fn name(&self) -> Option<&QName> {
        match self {
            Self::Simple(s) => s.name.as_ref(),
            Self::Complex(c) => c.name.as_ref(),
        }
    }

    pub(crate) fn base(&self) -> TypeId {
        match self {
            Self::Simple(s) => s.base,
            Self::Complex(c) => c.base,
        }
    }
}

/// Whether a simple type or one of the simple types it is derived from has a facet
/// ([`SimpleType::has_facets`]).
pub(crate) fn has_facets(types: &[TypeDef], type_id: TypeId) -> bool {
    let mut current = type_id;
    while let TypeDef::Simple(step) = &types[current] {
        if step.has_facets() {
            return true;
        }
        current = step.base;
    }
    false
}

/// A facet that a value does not satisfy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FacetProblem<'s> {
    /// Not one of these values.
    Enumeration(&'s [String]),
    /// Shorter than `min` characters.
    MinLength { length: usize, min: usize },
    /// Longer than `max` characters.
    MaxLength { length: usize, max: usize },
}

impl FacetProblem<'_> {
    /// A description, completing "`value` is ..." or "the value is ...";
    /// `type_name` names the type whose facet it is.
    pub(crate) fn describe(self, type_name: &str) -> String {
        match self {
            Self::Enumeration(values) => format!(
                "not one of the values allowed by {type_name}: {}",
                values
                    .iter()
                    .map(|v| quote(v))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::MinLength { length, min } => format!(
                "{length} characters long, less than the minimum length {min} of {type_name}"
            ),
            Self::MaxLength { length, max } => format!(
                "{length} characters long, more than the maximum length {max} of {type_name}"
            ),
        }
    }
}

/// The first facet of the derivation chain of a simple type (from the type itself
/// towards its built-in type) that a value, already whitespace-processed and in the
/// lexical space of the built-in type, does not satisfy. `None` when it satisfies
/// every facet.
pub(crate) fn facet_problem<'s>(
    types: &'s [TypeDef],
    type_id: TypeId,
    value: &str,
) -> Option<FacetProblem<'s>> {
    let mut length = None;
    let mut current = type_id;
    while let TypeDef::Simple(step) = &types[current] {
        if let Some(values) = &step.enumeration
            && !values.iter().any(|v| v == value)
        {
            return Some(FacetProblem::Enumeration(values));
        }
        if step.min_length.is_some() || step.max_length.is_some() {
            let length = *length.get_or_insert_with(|| value.chars().count());
            if let Some(min) = step.min_length
                && length < min
            {
                return Some(FacetProblem::MinLength { length, min });
            }
            if let Some(max) = step.max_length
                && length > max
            {
                return Some(FacetProblem::MaxLength { length, max });
            }
        }
        current = step.base;
    }
    None
}

/// Values quoted in messages are cut to this many characters.
const MAX_QUOTED: usize = 80;

/// A value quoted for a message, cut to [`MAX_QUOTED`] characters.
pub(crate) fn quote(value: &str) -> String {
    let mut out: String = value.chars().take(MAX_QUOTED).collect();
    if value.chars().nth(MAX_QUOTED).is_some() {
        out.push_str("...");
    }
    format!("{out:?}")
}

/// A loaded schema set: every component reachable from the entry points.
#[derive(Debug, Clone)]
pub struct Schema {
    pub(crate) types: Vec<TypeDef>,
    pub(crate) elements: Vec<ElementDecl>,
    /// Global element declarations by namespace ([`namespace_key`]), then local
    /// name.
    pub(crate) global_elements: HashMap<String, HashMap<String, ElementId>>,
    /// Global type definitions by namespace ([`namespace_key`]), then local name
    /// (built-ins included).
    pub(crate) global_types: HashMap<String, HashMap<String, TypeId>>,
    /// The schema documents that were loaded, in load order.
    pub(crate) documents: Vec<String>,
    /// The imports that were not loaded, in the order they were found.
    pub(crate) omitted_imports: Vec<OmittedImport>,
    /// The value constraints of element declarations, sorted by element: few
    /// declarations have one, so they are kept out of [`ElementDecl`].
    pub(crate) element_constraints: Vec<(ElementId, ValueConstraint)>,
    /// The fixed values of attribute uses ([`FixedValue`]).
    pub(crate) fixed_values: Vec<Box<str>>,
}

/// An `xs:import` that was not loaded: it imports one of the namespaces of
/// [`XADES_NAMESPACES`](crate::XADES_NAMESPACES) and the schema document it names
/// is not in the schema set. Elements of that namespace then have no declaration;
/// where a lax wildcard allows them (the UBL extension content, `ds:Object`) they
/// are validated laxly, as `xs:anyType`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OmittedImport {
    /// The importing schema document, as its path relative to the root of the
    /// schema set.
    pub file: String,
    /// 1-based line of the `xs:import` element in that document.
    pub line: u32,
    /// The imported namespace.
    pub namespace: String,
    /// The schema document the import names, resolved against the importing
    /// document: a path that is not in the schema set.
    pub location: String,
}

/// The key of a namespace in the global component maps of [`Schema`]: the
/// namespace name, or the empty string for no namespace (a target namespace is never
/// empty: the loader refuses `targetNamespace=""`), so that a lookup borrows the name
/// instead of copying it. `None` for `Some("")`, which names no namespace of the set.
pub(crate) fn namespace_key(namespace: Option<&str>) -> Option<&str> {
    match namespace {
        None => Some(""),
        Some("") => None,
        Some(ns) => Some(ns),
    }
}

impl Schema {
    pub(crate) fn global_element(&self, namespace: Option<&str>, local: &str) -> Option<ElementId> {
        let by_name = self.global_elements.get(namespace_key(namespace)?)?;
        by_name.get(local).copied()
    }

    pub(crate) fn global_type(&self, namespace: Option<&str>, local: &str) -> Option<TypeId> {
        let by_name = self.global_types.get(namespace_key(namespace)?)?;
        by_name.get(local).copied()
    }

    /// Releases the spare capacity that loading leaves in the tables: the schema
    /// is kept for many validations.
    pub(crate) fn shrink_to_fit(&mut self) {
        self.types.shrink_to_fit();
        for type_def in &mut self.types {
            if let TypeDef::Simple(SimpleType {
                enumeration: Some(values),
                ..
            }) = type_def
            {
                values.shrink_to_fit();
            }
        }
        self.elements.shrink_to_fit();
        for by_name in self.global_elements.values_mut() {
            by_name.shrink_to_fit();
        }
        self.global_elements.shrink_to_fit();
        for by_name in self.global_types.values_mut() {
            by_name.shrink_to_fit();
        }
        self.global_types.shrink_to_fit();
        self.element_constraints.shrink_to_fit();
        self.fixed_values.shrink_to_fit();
    }

    /// Whether a simple type or one of the simple types it is derived from has a
    /// facet ([`SimpleType::has_facets`]).
    pub(crate) fn has_facets(&self, type_id: TypeId) -> bool {
        has_facets(&self.types, type_id)
    }

    /// The text of a fixed attribute value.
    pub(crate) fn fixed_value(&self, fixed: FixedValue) -> &str {
        self.fixed_values.get(fixed.index()).map_or("", |v| v)
    }

    /// The value constraint of an element declaration, if it has one.
    pub(crate) fn element_constraint(&self, element: ElementId) -> Option<&ValueConstraint> {
        if self.element_constraints.is_empty() {
            return None;
        }
        self.element_constraints
            .binary_search_by_key(&element, |(id, _)| *id)
            .ok()
            .map(|index| &self.element_constraints[index].1)
    }

    /// Whether `derived` is `base` or derived from it through any number of
    /// derivation steps (no derivation is blocked in the supported subset).
    pub(crate) fn is_derived_from(&self, derived: TypeId, base: TypeId) -> bool {
        let mut current = derived;
        loop {
            if current == base {
                return true;
            }
            if current == ANY_TYPE {
                return false;
            }
            current = self.types[current].base();
        }
    }

    /// The paths of the schema documents the set was loaded from.
    #[must_use]
    pub fn documents(&self) -> &[String] {
        &self.documents
    }

    /// The imports that were not loaded because their namespace is one of
    /// [`XADES_NAMESPACES`](crate::XADES_NAMESPACES) and the schema document they
    /// name is not in the set, in the order they were found. Empty when every
    /// import was loaded.
    #[must_use]
    pub fn omitted_imports(&self) -> &[OmittedImport] {
        &self.omitted_imports
    }

    /// The number of global element declarations.
    #[must_use]
    pub fn global_element_count(&self) -> usize {
        self.global_elements.values().map(HashMap::len).sum()
    }

    /// The number of type definitions, built-in and anonymous included.
    #[must_use]
    pub fn type_count(&self) -> usize {
        self.types.len()
    }
}
