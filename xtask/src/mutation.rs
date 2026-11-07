//! Mutation cases: documents derived from a few base cases of a rule set by
//! the edits of the mutation catalogue, `corpus/mutations.toml`.
//!
//! The catalogue is shared by every rule set. Each `[[mutation]]` has:
//!
//! - `id`: lower-case words separated by `-`, in `/`-separated groups
//!   (`amounts/payable-plus-one-cent`); the case id is
//!   `mutations/<id>.xml`.
//! - `base`: a base name; each rule set maps it to one of its whole-document
//!   cases ([`crate::ruleset::RuleSet::mutation_bases`]).
//! - `reason`: one line, what the mutation exercises.
//! - `edits`: the edits, applied in order; each is an array of strings, an
//!   operation and its arguments (see [`Edit`]).
//! - `only` (optional): the rule sets the mutation applies to, when it does
//!   not apply to all.
//!
//! Edits work on the text of the document, so everything they do not touch
//! stays byte for byte as in the base (comments, whitespace, namespace
//! declarations, quotes). A path selects elements or attributes with a subset
//! of XPath's abbreviated syntax, see [`Path`]. Every path must select
//! something (except for `remove-all`), the edits together must change the
//! document, and the result must be well-formed XML. An edit may leave the
//! document unchanged in one base (a value that is already there), so that
//! one mutation applies to the bases of every rule set.

use std::fmt::Write as _;
use std::path::Path as FsPath;

use roxmltree::{Document, Node};

use crate::toml::{self, Value};
use crate::util::{self, Error, Result};

/// The catalogue, relative to the repository root.
pub(crate) const CATALOGUE: &str = "corpus/mutations.toml";
/// Directory of the mutated documents, relative to the corpus directory; also
/// the prefix of their case ids.
pub(crate) const DIR: &str = "mutations";

/// The prefixes a path may use.
const NAMESPACES: [(&str, &str); 5] = [
    (
        "cbc",
        "urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2",
    ),
    (
        "cac",
        "urn:oasis:names:specification:ubl:schema:xsd:CommonAggregateComponents-2",
    ),
    (
        "ext",
        "urn:oasis:names:specification:ubl:schema:xsd:CommonExtensionComponents-2",
    ),
    (
        "inv",
        "urn:oasis:names:specification:ubl:schema:xsd:Invoice-2",
    ),
    (
        "cn",
        "urn:oasis:names:specification:ubl:schema:xsd:CreditNote-2",
    ),
];

/// One mutation of the catalogue.
#[derive(Debug, Clone)]
pub(crate) struct Mutation {
    pub(crate) id: String,
    pub(crate) base: String,
    pub(crate) reason: String,
    pub(crate) edits: Vec<Edit>,
    pub(crate) only: Option<Vec<String>>,
}

impl Mutation {
    /// The case id of the mutated document.
    pub(crate) fn case_id(&self) -> String {
        format!("{DIR}/{}.xml", self.id)
    }

    /// Whether the mutation applies to the rule set named `name`.
    pub(crate) fn applies_to(&self, name: &str) -> bool {
        self.only
            .as_ref()
            .is_none_or(|only| only.iter().any(|o| o == name))
    }
}

/// One edit. `path` arguments are [`Path`]s; `xml` arguments are inserted as
/// they are (markup), `value` arguments are escaped as character data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Edit {
    /// `["set", path, value]`: the content of each element becomes the text
    /// `value`; or the value of each attribute becomes `value` (the attribute
    /// is added to an element that has none). An optional third argument, a
    /// count, repeats `value` (as for `prefix` and `suffix`).
    Set(Path, String),
    /// `["prefix", path, value]`: inserts the text `value` at the start of the
    /// content of each element, or of the value of each attribute.
    Prefix(Path, String),
    /// `["suffix", path, value]`: the same, at the end.
    Suffix(Path, String),
    /// `["prefix-xml", path, xml]`: inserts `xml` at the start of the content
    /// of each element.
    PrefixXml(Path, String),
    /// `["suffix-xml", path, xml]`: the same, at the end.
    SuffixXml(Path, String),
    /// `["set-xml", path, xml]`: the content of each element becomes `xml`.
    SetXml(Path, String),
    /// `["add", path, decimal]`: the decimal number in each element or
    /// attribute (surrounding whitespace kept) gets `decimal` added, written
    /// with the larger of the two scales.
    Add(Path, Decimal),
    /// `["multiply", path, decimal]`: the same, multiplied; the scale of the
    /// result is the sum of the scales.
    Multiply(Path, Decimal),
    /// `["remove", path]`: removes each element or attribute, with the
    /// whitespace before it.
    Remove(Path),
    /// `["remove-all", path]`: the same, but the path may select nothing (for
    /// an element some bases have and others do not).
    RemoveAll(Path),
    /// `["insert-before", path, xml]`: inserts `xml` before each element,
    /// followed by the whitespace that precedes the element.
    InsertBefore(Path, String),
    /// `["insert-after", path, xml]`: inserts the whitespace that precedes
    /// each element, then `xml`, after the element.
    InsertAfter(Path, String),
    /// `["append", path, xml]`: inserts `xml` as the last content of each
    /// element (after its last child element, with the same indentation).
    Append(Path, String),
    /// `["prepend", path, xml]`: inserts `xml` as the first content of each
    /// element.
    Prepend(Path, String),
    /// `["duplicate", path]`: inserts a copy of each element after it.
    Duplicate(Path),
    /// `["rename", path, qname]`: renames each element (start and end tags).
    Rename(Path, String),
    /// `["replace", find, with]`: replaces every occurrence of the text `find`
    /// in the whole document (there must be one at least).
    Replace(String, String),
    /// `["replace-in", path, find, with]`: the same, inside each element.
    ReplaceIn(Path, String, String),
}

/// Reads the catalogue.
pub(crate) fn catalogue(root: &FsPath) -> Result<Vec<Mutation>> {
    let text = util::read_string(&util::join(root, CATALOGUE))?;
    parse_catalogue(&text, CATALOGUE)
}

fn parse_catalogue(text: &str, file: &str) -> Result<Vec<Mutation>> {
    let mut mutations: Vec<Mutation> = Vec::new();
    for table in toml::parse(text, file)? {
        let at = format!("{file}:{}", table.line);
        if table.name != "mutation" {
            return Err(Error::new(format!(
                "{at}: unknown table [[{}]], expected [[mutation]]",
                table.name
            )));
        }
        let mut id = None;
        let mut base = None;
        let mut reason = None;
        let mut edits = None;
        let mut only = None;
        for (key, value) in table.entries {
            match (key.as_str(), value) {
                ("id", Value::String(s)) => id = Some(s),
                ("base", Value::String(s)) => base = Some(s),
                ("reason", Value::String(s)) => reason = Some(s),
                ("edits", Value::Array(items)) => edits = Some(items),
                ("only", Value::Array(items)) => {
                    let names = items
                        .into_iter()
                        .map(|item| match item {
                            Value::String(s) => Ok(s),
                            Value::Array(_) => Err(Error::new(format!(
                                "{at}: `only` must be an array of strings"
                            ))),
                        })
                        .collect::<Result<Vec<_>>>()?;
                    only = Some(names);
                }
                (key, _) => {
                    return Err(Error::new(format!(
                        "{at}: unexpected key or value type: {key}"
                    )));
                }
            }
        }
        let missing = |what: &str| Error::new(format!("{at}: no `{what}`"));
        let id = id.ok_or_else(|| missing("id"))?;
        let at = format!("{at} ({id})");
        if !valid_id(&id) {
            return Err(Error::new(format!(
                "{at}: an id is lower-case words separated by - in /-separated groups"
            )));
        }
        if mutations.iter().any(|m| m.id == id) {
            return Err(Error::new(format!("{at}: duplicate id")));
        }
        let base = base.ok_or_else(|| missing("base"))?;
        let reason = reason.ok_or_else(|| missing("reason"))?;
        if reason.trim().is_empty()
            || reason != reason.trim()
            || reason.contains(['\t', '\n', '\r'])
        {
            return Err(Error::new(format!(
                "{at}: the reason must be one non-empty trimmed line"
            )));
        }
        let edits = edits.ok_or_else(|| missing("edits"))?;
        if edits.is_empty() {
            return Err(Error::new(format!("{at}: no edits")));
        }
        let edits = edits
            .into_iter()
            .map(|item| parse_edit(item).map_err(|e| Error::new(format!("{at}: {e}"))))
            .collect::<Result<Vec<_>>>()?;
        if only.as_ref().is_some_and(Vec::is_empty) {
            return Err(Error::new(format!("{at}: `only` is empty")));
        }
        mutations.push(Mutation {
            id,
            base,
            reason,
            edits,
            only,
        });
    }
    if mutations.is_empty() {
        return Err(Error::new(format!("{file}: no mutation")));
    }
    Ok(mutations)
}

fn valid_id(id: &str) -> bool {
    id.split('/').all(|group| {
        !group.is_empty()
            && group.split('-').all(|word| {
                !word.is_empty()
                    && word
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
            })
    })
}

fn parse_edit(item: Value) -> Result<Edit> {
    let Value::Array(parts) = item else {
        return Err(Error::new("an edit is an array of strings"));
    };
    let parts = parts
        .into_iter()
        .map(|p| match p {
            Value::String(s) => Ok(s),
            Value::Array(_) => Err(Error::new("an edit is an array of strings")),
        })
        .collect::<Result<Vec<_>>>()?;
    let Some((op, args)) = parts.split_first() else {
        return Err(Error::new("empty edit"));
    };
    build_edit(op, args).map_err(|e| Error::new(format!("{op}: {e}")))
}

/// The edit `op` with its arguments.
fn build_edit(op: &str, args: &[String]) -> std::result::Result<Edit, String> {
    let arity = |n: usize| -> std::result::Result<(), String> {
        if args.len() == n {
            Ok(())
        } else {
            Err(format!("{n} arguments expected, {} given", args.len()))
        }
    };
    let path = || -> std::result::Result<Path, String> {
        let text = args.first().ok_or("no path")?;
        Path::parse(text).map_err(|e| format!("path {text:?}: {e}"))
    };
    let element = || -> std::result::Result<Path, String> {
        let p = path()?;
        if p.attribute.is_some() {
            Err("the path must select elements".to_owned())
        } else {
            Ok(p)
        }
    };
    // A text argument, then an optional repeat count.
    let repeated = || -> std::result::Result<String, String> {
        match args {
            [_, text] => Ok(text.clone()),
            [_, text, count] => count
                .parse::<usize>()
                .ok()
                .filter(|n| (1..=100_000).contains(n))
                .map(|n| text.repeat(n))
                .ok_or_else(|| format!("bad repeat count {count:?}")),
            _ => Err(format!("2 or 3 arguments expected, {} given", args.len())),
        }
    };
    let decimal = || -> std::result::Result<Decimal, String> {
        arity(2)?;
        Decimal::parse(&args[1]).ok_or_else(|| format!("{:?} is not a decimal number", args[1]))
    };
    let second = || -> std::result::Result<String, String> {
        arity(2)?;
        Ok(args[1].clone())
    };
    Ok(match op {
        "set" => Edit::Set(path()?, repeated()?),
        "prefix" => Edit::Prefix(path()?, repeated()?),
        "suffix" => Edit::Suffix(path()?, repeated()?),
        "add" => Edit::Add(path()?, decimal()?),
        "multiply" => Edit::Multiply(path()?, decimal()?),
        "set-xml" => Edit::SetXml(element()?, second()?),
        "prefix-xml" => Edit::PrefixXml(element()?, second()?),
        "suffix-xml" => Edit::SuffixXml(element()?, second()?),
        "insert-before" => Edit::InsertBefore(element()?, second()?),
        "insert-after" => Edit::InsertAfter(element()?, second()?),
        "append" => Edit::Append(element()?, second()?),
        "prepend" => Edit::Prepend(element()?, second()?),
        "remove" | "remove-all" | "duplicate" => {
            arity(1)?;
            match op {
                "remove" => Edit::Remove(path()?),
                "remove-all" => Edit::RemoveAll(path()?),
                _ => Edit::Duplicate(element()?),
            }
        }
        "rename" => {
            let name = second()?;
            if !is_qname(&name) {
                return Err(format!("{name:?} is not a QName"));
            }
            Edit::Rename(element()?, name)
        }
        "replace" => {
            arity(2)?;
            if args[0].is_empty() {
                return Err("empty text to find".to_owned());
            }
            Edit::Replace(args[0].clone(), args[1].clone())
        }
        "replace-in" => {
            arity(3)?;
            if args[1].is_empty() {
                return Err("empty text to find".to_owned());
            }
            Edit::ReplaceIn(element()?, args[1].clone(), args[2].clone())
        }
        other => return Err(format!("unknown edit {other:?}")),
    })
}

fn is_name_char(c: char, first: bool) -> bool {
    c.is_alphabetic() || c == '_' || (!first && (c.is_ascii_digit() || c == '-' || c == '.'))
}

fn is_ncname(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| is_name_char(c, true)) && chars.all(|c| is_name_char(c, false))
}

fn is_qname(s: &str) -> bool {
    match s.split_once(':') {
        Some((prefix, local)) => is_ncname(prefix) && is_ncname(local),
        None => is_ncname(s),
    }
}

// ---------------------------------------------------------------------------
// Paths

/// A location path: `/`-separated steps from the document node, each a child
/// step (`/name`) or a descendant step (`//name`), optionally ending with an
/// attribute (`/@name`, or `//@name` for the attribute of every element that
/// has it). A name is `*`, `prefix:local` with a prefix of
/// [`NAMESPACES`], or an unprefixed name (no namespace). Predicates, after a
/// name: `[N]` (1-based position among the matching children of each parent),
/// `[last()]`, `[child='text']` (a child element, named like a step, whose
/// string value is `text`; also a relative path of child steps,
/// `[cac:TaxCategory/cbc:ID='E']`), `[child!='text']` (one whose string value
/// is not `text`) and `[@attribute='text']`. The final attribute may
/// also be a namespace declaration (`@xmlns`, `@xmlns:prefix`) or a prefixed
/// name (`@xsi:type`, the prefix declared in the document), which only `set`
/// can add.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Path {
    steps: Vec<Step>,
    attribute: Option<String>,
    /// `//@name`: the attribute wherever it is, never added.
    existing_only: bool,
    text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Step {
    descendant: bool,
    test: NameTest,
    predicates: Vec<Predicate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum NameTest {
    Any,
    Name {
        namespace: &'static str,
        local: String,
    },
}

impl NameTest {
    fn parse(text: &str) -> std::result::Result<Self, String> {
        if text == "*" {
            return Ok(Self::Any);
        }
        let (namespace, local) = match text.split_once(':') {
            Some((prefix, local)) => {
                let namespace = NAMESPACES
                    .iter()
                    .find(|(p, _)| *p == prefix)
                    .map(|(_, uri)| *uri)
                    .ok_or_else(|| format!("unknown prefix {prefix}"))?;
                (namespace, local)
            }
            None => ("", text),
        };
        if !is_ncname(local) {
            return Err(format!("bad name {text:?}"));
        }
        Ok(Self::Name {
            namespace,
            local: local.to_owned(),
        })
    }

    fn matches(&self, node: Node<'_, '_>) -> bool {
        node.is_element()
            && match self {
                Self::Any => true,
                Self::Name { namespace, local } => {
                    node.tag_name().name() == local
                        && node.tag_name().namespace().unwrap_or("") == *namespace
                }
            }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Predicate {
    Position(usize),
    Last,
    Child(Vec<NameTest>, String),
    ChildNot(Vec<NameTest>, String),
    Attribute(String, String),
}

impl Path {
    pub(crate) fn parse(text: &str) -> std::result::Result<Self, String> {
        let mut rest = text
            .strip_prefix('/')
            .ok_or("a path starts with / (from the document node)")?;
        let mut steps = Vec::new();
        let mut attribute = None;
        let mut existing_only = false;
        loop {
            let descendant = if let Some(r) = rest.strip_prefix('/') {
                rest = r;
                true
            } else {
                false
            };
            if let Some(name) = rest.strip_prefix('@') {
                if descendant {
                    if !is_ncname(name) {
                        return Err(format!("bad attribute step //@{name}"));
                    }
                    steps.push(Step {
                        descendant: true,
                        test: NameTest::Any,
                        predicates: Vec::new(),
                    });
                    existing_only = true;
                } else if !(name == "xmlns" || is_qname(name)) {
                    return Err(format!("bad attribute step @{name}"));
                }
                attribute = Some(name.to_owned());
                break;
            }
            let end = step_end(rest)?;
            let (step, after) = rest.split_at(end);
            steps.push(parse_step(step, descendant)?);
            match after.strip_prefix('/') {
                Some(r) => rest = r,
                None if after.is_empty() => break,
                None => return Err(format!("unexpected {after:?}")),
            }
        }
        if steps.is_empty() {
            return Err("no element step".to_owned());
        }
        Ok(Self {
            steps,
            attribute,
            existing_only,
            text: text.to_owned(),
        })
    }
}

/// The end of the step at the start of `text`: the first `/` outside brackets
/// and quotes.
fn step_end(text: &str) -> std::result::Result<usize, String> {
    let mut depth = 0usize;
    let mut quote = None;
    for (i, c) in text.char_indices() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '\'' | '"' => quote = Some(c),
            '[' => depth += 1,
            ']' => depth = depth.checked_sub(1).ok_or("unbalanced ]")?,
            '/' if depth == 0 => return Ok(i),
            _ => {}
        }
    }
    if depth != 0 || quote.is_some() {
        return Err("unbalanced [ or quote".to_owned());
    }
    Ok(text.len())
}

fn parse_step(text: &str, descendant: bool) -> std::result::Result<Step, String> {
    let name_end = text.find('[').unwrap_or(text.len());
    let name_test = NameTest::parse(&text[..name_end])?;
    let mut predicates = Vec::new();
    let mut rest = &text[name_end..];
    while let Some(r) = rest.strip_prefix('[') {
        let close = predicate_end(r)?;
        predicates.push(parse_predicate(&r[..close])?);
        rest = &r[close + 1..];
    }
    if !rest.is_empty() {
        return Err(format!("unexpected {rest:?} after the predicates"));
    }
    Ok(Step {
        descendant,
        test: name_test,
        predicates,
    })
}

fn predicate_end(text: &str) -> std::result::Result<usize, String> {
    let mut quote = None;
    for (i, c) in text.char_indices() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '\'' | '"' => quote = Some(c),
            ']' => return Ok(i),
            '[' => return Err("nested predicates are not supported".to_owned()),
            _ => {}
        }
    }
    Err("unterminated predicate".to_owned())
}

fn parse_predicate(text: &str) -> std::result::Result<Predicate, String> {
    if text == "last()" {
        return Ok(Predicate::Last);
    }
    if let Ok(n) = text.parse::<usize>() {
        return if n == 0 {
            Err("positions start at 1".to_owned())
        } else {
            Ok(Predicate::Position(n))
        };
    }
    let (left, right, negated) = if let Some((left, right)) = text.split_once("!=") {
        (left, right, true)
    } else {
        let (left, right) = text
            .split_once('=')
            .ok_or_else(|| format!("unsupported predicate [{text}]"))?;
        (left, right, false)
    };
    let value = right
        .strip_prefix('\'')
        .and_then(|r| r.strip_suffix('\''))
        .or_else(|| right.strip_prefix('"').and_then(|r| r.strip_suffix('"')))
        .ok_or_else(|| format!("predicate [{text}]: the value must be quoted"))?;
    if let Some(name) = left.strip_prefix('@') {
        if !is_ncname(name) || negated {
            return Err(format!("unsupported attribute predicate [{text}]"));
        }
        return Ok(Predicate::Attribute(name.to_owned(), value.to_owned()));
    }
    let steps = left
        .split('/')
        .map(NameTest::parse)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(if negated {
        Predicate::ChildNot(steps, value.to_owned())
    } else {
        Predicate::Child(steps, value.to_owned())
    })
}

/// What a path selects in a document.
enum Target<'a, 'input> {
    Element(Node<'a, 'input>),
    /// An attribute of an element: the element, the attribute name, and the
    /// attribute when the element has it.
    Attribute(Node<'a, 'input>, String, bool),
}

/// Selects the elements of `path`, in document order.
fn select_elements<'a, 'input>(doc: &'a Document<'input>, path: &Path) -> Vec<Node<'a, 'input>> {
    let mut context = vec![doc.root()];
    for step in &path.steps {
        let mut parents = Vec::new();
        for node in &context {
            if step.descendant {
                parents.extend(node.descendants().filter(Node::is_element));
                if node.is_root() {
                    parents.push(*node);
                }
            } else {
                parents.push(*node);
            }
        }
        let mut next = Vec::new();
        for parent in parents {
            let mut children: Vec<Node> = parent
                .children()
                .filter(|c| step.test.matches(*c))
                .collect();
            for predicate in &step.predicates {
                children = match predicate {
                    Predicate::Position(n) => children.get(n - 1).copied().into_iter().collect(),
                    Predicate::Last => children.last().copied().into_iter().collect(),
                    Predicate::Child(steps, value) => children
                        .into_iter()
                        .filter(|c| relative(*c, steps).any(|g| string_value(g) == *value))
                        .collect(),
                    Predicate::ChildNot(steps, value) => children
                        .into_iter()
                        .filter(|c| relative(*c, steps).any(|g| string_value(g) != *value))
                        .collect(),
                    Predicate::Attribute(name, value) => children
                        .into_iter()
                        .filter(|c| c.attribute(name.as_str()) == Some(value.as_str()))
                        .collect(),
                };
            }
            next.extend(children);
        }
        // Document order: elements start at increasing offsets.
        next.sort_by_key(|n| n.range().start);
        next.dedup_by_key(|n| n.range().start);
        context = next;
    }
    context
}

fn select<'a, 'input>(doc: &'a Document<'input>, path: &Path) -> Vec<Target<'a, 'input>> {
    let elements = select_elements(doc, path);
    match &path.attribute {
        None => elements.into_iter().map(Target::Element).collect(),
        Some(name) => elements
            .into_iter()
            .map(|e| {
                // A prefixed name or a namespace declaration is never seen
                // as present: `set` adds it.
                let present = !name.contains(':')
                    && name != "xmlns"
                    && e.attributes()
                        .any(|a| a.namespace().is_none() && a.name() == name);
                Target::Attribute(e, name.clone(), present)
            })
            .filter(|t| !path.existing_only || matches!(t, Target::Attribute(_, _, true)))
            .collect(),
    }
}

/// The elements reached from `node` by the child steps `steps`.
fn relative<'a, 'input>(
    node: Node<'a, 'input>,
    steps: &[NameTest],
) -> impl Iterator<Item = Node<'a, 'input>> {
    let mut nodes = vec![node];
    for test in steps {
        nodes = nodes
            .iter()
            .flat_map(|n| n.children().filter(|c| test.matches(*c)))
            .collect();
    }
    nodes.into_iter()
}

fn string_value(node: Node<'_, '_>) -> String {
    node.descendants()
        .filter(Node::is_text)
        .filter_map(|t| t.text())
        .collect()
}

// ---------------------------------------------------------------------------
// Applying edits

/// One replacement of the bytes `start..end` of the text.
#[derive(Debug)]
struct Splice {
    start: usize,
    end: usize,
    text: String,
}

/// Applies `edits` to `base`. `name` names the result in errors. Path edits
/// need a well-formed document; the result must be well-formed.
pub(crate) fn apply(base: &str, edits: &[Edit], name: &str) -> Result<String> {
    let mut text = base.to_owned();
    for (index, edit) in edits.iter().enumerate() {
        let at = || format!("{name}: edit {}", index + 1);
        text = apply_one(&text, edit).map_err(|e| Error::new(format!("{}: {e}", at())))?;
    }
    if text == base {
        return Err(Error::new(format!("{name}: the edits change nothing")));
    }
    Document::parse(&text)
        .map_err(|e| Error::new(format!("{name}: the result is not well-formed XML: {e}")))?;
    Ok(text)
}

fn apply_one(text: &str, edit: &Edit) -> std::result::Result<String, String> {
    if let Edit::Replace(find, with) = edit {
        if !text.contains(find.as_str()) {
            return Err(format!("{find:?} does not occur"));
        }
        return Ok(text.replace(find.as_str(), with));
    }
    let doc = Document::parse(text).map_err(|e| e.to_string())?;
    let path = match edit {
        Edit::Set(p, _)
        | Edit::Prefix(p, _)
        | Edit::Suffix(p, _)
        | Edit::PrefixXml(p, _)
        | Edit::SuffixXml(p, _)
        | Edit::RemoveAll(p)
        | Edit::SetXml(p, _)
        | Edit::Add(p, _)
        | Edit::Multiply(p, _)
        | Edit::Remove(p)
        | Edit::InsertBefore(p, _)
        | Edit::InsertAfter(p, _)
        | Edit::Append(p, _)
        | Edit::Prepend(p, _)
        | Edit::Duplicate(p)
        | Edit::Rename(p, _)
        | Edit::ReplaceIn(p, _, _) => p,
        Edit::Replace(..) => return Err("unreachable".to_owned()),
    };
    let targets = select(&doc, path);
    if targets.is_empty() && !matches!(edit, Edit::RemoveAll(_)) {
        return Err(format!("{} selects nothing", path.text));
    }
    let mut splices = Vec::new();
    for target in targets {
        splices.extend(splices_for(text, edit, &target)?);
    }
    splice(text, splices)
}

fn splices_for(
    text: &str,
    edit: &Edit,
    target: &Target<'_, '_>,
) -> std::result::Result<Vec<Splice>, String> {
    match target {
        Target::Element(node) => element_splices(text, edit, *node),
        Target::Attribute(element, name, present) => {
            attribute_splices(text, edit, *element, name, *present)
        }
    }
}

/// The parts of an element in the text.
struct Tags {
    /// The whole element.
    start: usize,
    end: usize,
    /// The qualified name in the start tag.
    name: (usize, usize),
    /// End of the start tag (after `>` or `/>`).
    start_tag_end: usize,
    /// For an element with an end tag: the start of the end tag, and its name.
    end_tag: Option<(usize, (usize, usize))>,
}

impl Tags {
    fn of(text: &str, node: Node<'_, '_>) -> std::result::Result<Self, String> {
        let range = node.range();
        let bytes = text.as_bytes();
        let mut i = range.start + 1;
        let name_start = i;
        while i < range.end && !matches!(bytes[i], b' ' | b'\t' | b'\r' | b'\n' | b'/' | b'>') {
            i += 1;
        }
        let name = (name_start, i);
        loop {
            while i < range.end && matches!(bytes[i], b' ' | b'\t' | b'\r' | b'\n') {
                i += 1;
            }
            if text[i..].starts_with("/>") {
                return Ok(Self {
                    start: range.start,
                    end: range.end,
                    name,
                    start_tag_end: i + 2,
                    end_tag: None,
                });
            }
            if bytes.get(i) == Some(&b'>') {
                i += 1;
                break;
            }
            // An attribute: name, =, quoted value.
            while i < range.end && !matches!(bytes[i], b'=' | b' ' | b'\t' | b'\r' | b'\n') {
                i += 1;
            }
            while i < range.end && matches!(bytes[i], b' ' | b'\t' | b'\r' | b'\n' | b'=') {
                i += 1;
            }
            let quote = *bytes.get(i).ok_or("unterminated start tag")?;
            if quote != b'"' && quote != b'\'' {
                return Err("cannot scan the start tag".to_owned());
            }
            let close = text[i + 1..range.end]
                .find(char::from(quote))
                .ok_or("unterminated attribute value")?;
            i += close + 2;
        }
        let close = text[range.start..range.end]
            .rfind("</")
            .map(|c| range.start + c)
            .filter(|c| *c >= i)
            .ok_or("no end tag")?;
        let mut j = close + 2;
        while j < range.end && !matches!(bytes[j], b' ' | b'\t' | b'\r' | b'\n' | b'>') {
            j += 1;
        }
        Ok(Self {
            start: range.start,
            end: range.end,
            name,
            start_tag_end: i,
            end_tag: Some((close, (close + 2, j))),
        })
    }

    fn qname<'t>(&self, text: &'t str) -> &'t str {
        &text[self.name.0..self.name.1]
    }
}

/// The run of whitespace right before `pos`.
fn whitespace_before(text: &str, pos: usize) -> usize {
    let bytes = text.as_bytes();
    let mut i = pos;
    while i > 0 && matches!(bytes[i - 1], b' ' | b'\t' | b'\r' | b'\n') {
        i -= 1;
    }
    i
}

fn element_splices(
    text: &str,
    edit: &Edit,
    node: Node<'_, '_>,
) -> std::result::Result<Vec<Splice>, String> {
    let tags = Tags::of(text, node)?;
    let indent_start = whitespace_before(text, tags.start);
    let indent = &text[indent_start..tags.start];
    // Replaces the content of the element (expanding an empty-element tag).
    let set_content = |content: String| -> Splice {
        match tags.end_tag {
            Some((close, _)) => Splice {
                start: tags.start_tag_end,
                end: close,
                text: content,
            },
            None => Splice {
                start: tags.start_tag_end - 2,
                end: tags.start_tag_end,
                text: format!(">{content}</{}>", tags.qname(text)),
            },
        }
    };
    // Inserts text at the start or the end of the content.
    let insert_content = |content: String, at_end: bool| -> Splice {
        match tags.end_tag {
            Some((close, _)) => {
                let at = if at_end { close } else { tags.start_tag_end };
                Splice {
                    start: at,
                    end: at,
                    text: content,
                }
            }
            None => set_content(content),
        }
    };
    Ok(match edit {
        Edit::Set(_, value) => vec![set_content(escape(value, false))],
        Edit::SetXml(_, xml) => vec![set_content(xml.clone())],
        Edit::Prefix(_, value) => vec![insert_content(escape(value, false), false)],
        Edit::Suffix(_, value) => vec![insert_content(escape(value, false), true)],
        Edit::PrefixXml(_, xml) => vec![insert_content(xml.clone(), false)],
        Edit::SuffixXml(_, xml) => vec![insert_content(xml.clone(), true)],
        Edit::Add(_, delta) | Edit::Multiply(_, delta) => {
            let (close, _) = tags.end_tag.ok_or("an empty element holds no number")?;
            let content = &text[tags.start_tag_end..close];
            if content.contains(['<', '&']) {
                return Err("the element holds markup, not a number".to_owned());
            }
            vec![number_splice(tags.start_tag_end, content, edit, *delta)?]
        }
        Edit::Remove(_) | Edit::RemoveAll(_) => vec![Splice {
            start: indent_start,
            end: tags.end,
            text: String::new(),
        }],
        Edit::InsertBefore(_, xml) => vec![Splice {
            start: tags.start,
            end: tags.start,
            text: format!("{xml}{indent}"),
        }],
        Edit::InsertAfter(_, xml) => vec![Splice {
            start: tags.end,
            end: tags.end,
            text: format!("{indent}{xml}"),
        }],
        Edit::Duplicate(_) => vec![Splice {
            start: tags.end,
            end: tags.end,
            text: format!("{indent}{}", &text[tags.start..tags.end]),
        }],
        Edit::Append(_, xml) | Edit::Prepend(_, xml) => {
            insert_child(text, node, &tags, xml, matches!(edit, Edit::Append(..)))?
        }
        Edit::Rename(_, name) => {
            let mut splices = vec![Splice {
                start: tags.name.0,
                end: tags.name.1,
                text: name.clone(),
            }];
            if let Some((_, (start, end))) = tags.end_tag {
                splices.push(Splice {
                    start,
                    end,
                    text: name.clone(),
                });
            }
            splices
        }
        Edit::ReplaceIn(_, find, with) => {
            let inside = &text[tags.start..tags.end];
            if !inside.contains(find.as_str()) {
                return Err(format!("{find:?} does not occur in the element"));
            }
            vec![Splice {
                start: tags.start,
                end: tags.end,
                text: inside.replace(find.as_str(), with),
            }]
        }
        Edit::Replace(..) => return Err("unreachable".to_owned()),
    })
}

/// Inserts `xml` as the last (`last`) or first content of an element, with
/// the indentation of its child elements.
fn insert_child(
    text: &str,
    node: Node<'_, '_>,
    tags: &Tags,
    xml: &str,
    last: bool,
) -> std::result::Result<Vec<Splice>, String> {
    let mut children = node.children().filter(Node::is_element);
    let child = if last {
        children.next_back()
    } else {
        children.next()
    };
    let Some(child) = child else {
        return Ok(vec![match tags.end_tag {
            Some((close, _)) => {
                let at = if last { close } else { tags.start_tag_end };
                Splice {
                    start: at,
                    end: at,
                    text: xml.to_owned(),
                }
            }
            None => Splice {
                start: tags.start_tag_end - 2,
                end: tags.start_tag_end,
                text: format!(">{xml}</{}>", tags.qname(text)),
            },
        }]);
    };
    let child_tags = Tags::of(text, child)?;
    let indent = &text[whitespace_before(text, child_tags.start)..child_tags.start];
    Ok(vec![if last {
        Splice {
            start: child_tags.end,
            end: child_tags.end,
            text: format!("{indent}{xml}"),
        }
    } else {
        Splice {
            start: child_tags.start,
            end: child_tags.start,
            text: format!("{xml}{indent}"),
        }
    }])
}

fn attribute_splices(
    text: &str,
    edit: &Edit,
    element: Node<'_, '_>,
    name: &str,
    present: bool,
) -> std::result::Result<Vec<Splice>, String> {
    let attribute = element
        .attributes()
        .find(|a| a.namespace().is_none() && a.name() == name);
    let Some(attribute) = attribute.filter(|_| present) else {
        return match edit {
            Edit::Set(_, value) => {
                let tags = Tags::of(text, element)?;
                let at = if tags.end_tag.is_some() {
                    tags.start_tag_end - 1
                } else {
                    tags.start_tag_end - 2
                };
                let at = whitespace_before(text, at);
                Ok(vec![Splice {
                    start: at,
                    end: at,
                    text: format!(" {name}=\"{}\"", escape(value, true)),
                }])
            }
            // Elements without the attribute are left alone; the path as a
            // whole must still select an attribute (checked by the caller
            // through `changes nothing`).
            _ => Ok(Vec::new()),
        };
    };
    let value = attribute.range_value();
    Ok(match edit {
        Edit::Set(_, new) => vec![Splice {
            start: value.start,
            end: value.end,
            text: escape(new, true),
        }],
        Edit::Add(_, delta) | Edit::Multiply(_, delta) => {
            let raw = &text[value.start..value.end];
            if raw.contains('&') {
                return Err("the attribute holds a reference, not a number".to_owned());
            }
            vec![number_splice(value.start, raw, edit, *delta)?]
        }
        Edit::Prefix(_, text) => vec![Splice {
            start: value.start,
            end: value.start,
            text: escape(text, true),
        }],
        Edit::Suffix(_, text) => vec![Splice {
            start: value.end,
            end: value.end,
            text: escape(text, true),
        }],
        Edit::Remove(_) | Edit::RemoveAll(_) => {
            let range = attribute.range();
            vec![Splice {
                start: whitespace_before(text, range.start),
                end: range.end,
                text: String::new(),
            }]
        }
        _ => return Err("this edit does not apply to attributes".to_owned()),
    })
}

/// Replaces the number in `raw` (at `offset` in the text), keeping the
/// whitespace around it.
fn number_splice(
    offset: usize,
    raw: &str,
    edit: &Edit,
    operand: Decimal,
) -> std::result::Result<Splice, String> {
    let trimmed = raw.trim_matches([' ', '\t', '\r', '\n']);
    let lead = raw.len() - raw.trim_start_matches([' ', '\t', '\r', '\n']).len();
    let number =
        Decimal::parse(trimmed).ok_or_else(|| format!("{trimmed:?} is not a decimal number"))?;
    let result = match edit {
        Edit::Add(..) => number.add(operand),
        _ => number.multiply(operand),
    }
    .ok_or("overflow")?;
    Ok(Splice {
        start: offset + lead,
        end: offset + lead + trimmed.len(),
        text: result.to_string(),
    })
}

/// Applies non-overlapping splices.
fn splice(text: &str, mut splices: Vec<Splice>) -> std::result::Result<String, String> {
    splices.sort_by_key(|s| (s.start, s.end));
    for pair in splices.windows(2) {
        if pair[0].end > pair[1].start {
            return Err("the selected nodes overlap".to_owned());
        }
    }
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for s in splices {
        out.push_str(&text[at..s.start]);
        out.push_str(&s.text);
        at = s.end;
    }
    out.push_str(&text[at..]);
    Ok(out)
}

/// Character data (or an attribute value) for `value`: `&`, `<`, `>` and CR
/// escaped, and in an attribute also `"`, tab and LF, so that the parsed
/// value is `value`.
fn escape(value: &str, attribute: bool) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\r' => out.push_str("&#13;"),
            '"' if attribute => out.push_str("&quot;"),
            '\t' if attribute => out.push_str("&#9;"),
            '\n' if attribute => out.push_str("&#10;"),
            c => out.push(c),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Decimal numbers

/// An exact decimal number: `mantissa / 10^scale`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Decimal {
    mantissa: i128,
    scale: u32,
}

impl Decimal {
    /// `[+-]?digits(.digits)?` or `[+-]?.digits`.
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let (negative, digits) = match text.as_bytes().first()? {
            b'-' => (true, &text[1..]),
            b'+' => (false, &text[1..]),
            _ => (false, text),
        };
        let (int, frac) = digits.split_once('.').unwrap_or((digits, ""));
        if int.is_empty() && frac.is_empty() {
            return None;
        }
        if !int.bytes().chain(frac.bytes()).all(|b| b.is_ascii_digit()) {
            return None;
        }
        let mut mantissa: i128 = 0;
        for b in int.bytes().chain(frac.bytes()) {
            mantissa = mantissa
                .checked_mul(10)?
                .checked_add(i128::from(b - b'0'))?;
        }
        Some(Self {
            mantissa: if negative { -mantissa } else { mantissa },
            scale: u32::try_from(frac.len()).ok()?,
        })
    }

    fn rescale(self, scale: u32) -> Option<i128> {
        self.mantissa
            .checked_mul(10i128.checked_pow(scale.checked_sub(self.scale)?)?)
    }

    fn add(self, other: Self) -> Option<Self> {
        let scale = self.scale.max(other.scale);
        Some(Self {
            mantissa: self.rescale(scale)?.checked_add(other.rescale(scale)?)?,
            scale,
        })
    }

    fn multiply(self, other: Self) -> Option<Self> {
        Some(Self {
            mantissa: self.mantissa.checked_mul(other.mantissa)?,
            scale: self.scale.checked_add(other.scale)?,
        })
    }
}

impl std::fmt::Display for Decimal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let digits = self.mantissa.unsigned_abs().to_string();
        let scale = usize::try_from(self.scale).map_err(|_| std::fmt::Error)?;
        let digits = format!("{digits:0>width$}", width = scale + 1);
        let (int, frac) = digits.split_at(digits.len() - scale);
        let mut out = String::new();
        if self.mantissa < 0 {
            out.push('-');
        }
        out.push_str(int);
        if scale > 0 {
            let _ = write!(out, ".{frac}");
        }
        f.write_str(&out)
    }
}

#[cfg(test)]
mod tests {
    use super::{Decimal, Edit, Path, apply, parse_catalogue, valid_id};

    const DOC: &str = "<?xml version=\"1.0\"?>\n<Invoice xmlns=\"urn:oasis:names:specification:ubl:schema:xsd:Invoice-2\"\n  xmlns:cbc=\"urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2\"\n  xmlns:cac=\"urn:oasis:names:specification:ubl:schema:xsd:CommonAggregateComponents-2\">\n  <cbc:ID>A1</cbc:ID>\n  <!-- c -->\n  <cac:Line>\n    <cbc:ID>1</cbc:ID>\n    <cbc:Amount currencyID = 'EUR'> 10.50 </cbc:Amount>\n  </cac:Line>\n  <cac:Line>\n    <cbc:ID>2</cbc:ID>\n    <cbc:Amount currencyID=\"EUR\">-3</cbc:Amount>\n    <cbc:Note/>\n  </cac:Line>\n</Invoice>\n";

    fn edit(op: &[&str]) -> Edit {
        let items = op
            .iter()
            .map(|s| crate::toml::Value::String((*s).to_owned()))
            .collect();
        super::parse_edit(crate::toml::Value::Array(items)).unwrap()
    }

    fn run(ops: &[&[&str]]) -> String {
        let edits: Vec<Edit> = ops.iter().map(|op| edit(op)).collect();
        apply(DOC, &edits, "t").unwrap_or_else(|e| panic!("{e}"))
    }

    fn diff(after: &str) -> (String, String) {
        // The changed region, for compact assertions.
        let start = DOC
            .bytes()
            .zip(after.bytes())
            .take_while(|(a, b)| a == b)
            .count();
        let end = DOC
            .bytes()
            .rev()
            .zip(after.bytes().rev())
            .take_while(|(a, b)| a == b)
            .count()
            .min(DOC.len() - start)
            .min(after.len() - start);
        (
            DOC[start..DOC.len() - end].to_owned(),
            after[start..after.len() - end].to_owned(),
        )
    }

    #[test]
    fn decimals() {
        let d = |s: &str| Decimal::parse(s).unwrap();
        assert_eq!(d("10.50").add(d("0.01")).unwrap().to_string(), "10.51");
        assert_eq!(d("0").add(d("-0.01")).unwrap().to_string(), "-0.01");
        assert_eq!(d("-3").add(d("0.004")).unwrap().to_string(), "-2.996");
        assert_eq!(d("12500").multiply(d("-1")).unwrap().to_string(), "-12500");
        assert_eq!(d("1.5").multiply(d("0.10")).unwrap().to_string(), "0.150");
        assert_eq!(d("+.5").to_string(), "0.5");
        for bad in ["", "-", ".", "1e3", "1,5", " 1", "1..2"] {
            assert!(Decimal::parse(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn paths() {
        for bad in [
            "x",
            "/",
            "/foo:x",
            "/*/cbc:ID[0]",
            "/*/cbc:ID[",
            "/*/cbc:ID[a]",
            "/*/@",
            "//@x:y",
            "/*/cbc:ID[cbc:X=1]",
            "/*/cac:Line[cbc:ID='1'][cbc:X[1]='2']",
        ] {
            assert!(Path::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn set_text_and_attribute() {
        let out = run(&[&["set", "/*/cac:Line[2]/cbc:Amount", "1 < 2 & 4"]]);
        assert_eq!(diff(&out), ("-3".to_owned(), "1 &lt; 2 &amp; 4".to_owned()));
        let out = run(&[&["set", "/*/cac:Line[1]/cbc:Amount/@currencyID", "a\"b"]]);
        assert_eq!(diff(&out), ("EUR".to_owned(), "a&quot;b".to_owned()));
        let out = run(&[&["set", "/*/cac:Line[cbc:ID!='1']/cbc:Note", "x"]]);
        assert_eq!(diff(&out), ("/".to_owned(), ">x</cbc:Note".to_owned()));
        let out = run(&[&["set", "/*[cac:Line/cbc:ID='2']/cbc:ID", "B2"]]);
        assert_eq!(diff(&out), ("A1".to_owned(), "B2".to_owned()));
        let out = run(&[&["set", "/*/cac:Line[cbc:ID!='1']/cbc:Note", "x"]]);
        assert_eq!(diff(&out), ("/".to_owned(), ">x</cbc:Note".to_owned()));
        let out = run(&[&["set", "/*/cbc:ID/@schemeID", "0088"]]);
        assert_eq!(diff(&out), (String::new(), " schemeID=\"0088\"".to_owned()));
        let out = run(&[&["set", "/*/cac:Line[last()]/cbc:Note/@x", "1"]]);
        assert_eq!(diff(&out), (String::new(), " x=\"1\"".to_owned()));
    }

    #[test]
    fn numbers() {
        let out = run(&[&["set", "//@currencyID", "USD"]]);
        assert_eq!(out.matches("USD").count(), 2, "{out}");
        assert!(!out.contains("<cbc:ID currencyID"), "{out}");
        let out = run(&[&["add", "//cbc:Amount", "0.001"]]);
        assert!(out.contains("'EUR'> 10.501 </cbc:Amount>"), "{out}");
        assert!(out.contains(">-2.999</cbc:Amount>"), "{out}");
        let out = run(&[&["multiply", "/*/cac:Line[1]/cbc:Amount", "-1"]]);
        assert!(out.contains("> -10.50 <"), "{out}");
        let out = run(&[
            &["prefix", "/*/cac:Line[2]/cbc:Amount", "+"],
            &["suffix", "/*/cac:Line[2]/cbc:Amount", "0", "3"],
            &["suffix-xml", "/*/cac:Line[2]/cbc:Amount", "<!--c-->"],
            &["prefix-xml", "/*/cac:Line[2]/cbc:Note", "<![CDATA[x]]>"],
            &["suffix", "/*/cac:Line[2]/cbc:Amount/@currencyID", " "],
        ]);
        assert!(
            out.contains("<cbc:Amount currencyID=\"EUR \">+-3000<!--c--></cbc:Amount>"),
            "{out}"
        );
        assert!(out.contains("<cbc:Note><![CDATA[x]]></cbc:Note>"), "{out}");
        let out = run(&[&["set", "/*/cbc:ID", "ab", "3"]]);
        assert!(out.contains("<cbc:ID>ababab</cbc:ID>"), "{out}");
    }

    #[test]
    fn structure() {
        let out = run(&[&["remove", "/*/cac:Line[1]/cbc:ID"]]);
        assert_eq!(diff(&out).1, "");
        assert!(!out.contains("<cbc:ID>1</cbc:ID>"));
        let out = run(&[&["duplicate", "/*/cbc:ID"]]);
        assert!(
            out.contains("  <cbc:ID>A1</cbc:ID>\n  <cbc:ID>A1</cbc:ID>\n"),
            "{out}"
        );
        let out = run(&[&["insert-before", "/*/cbc:ID", "<cbc:X/>"]]);
        assert!(out.contains("\n  <cbc:X/>\n  <cbc:ID>A1"), "{out}");
        let out = run(&[&["insert-after", "/*/cac:Line[2]", "<!--z-->"]]);
        assert!(out.contains("</cac:Line>\n  <!--z-->\n</Invoice>"), "{out}");
        let out = run(&[&["append", "/*/cac:Line[1]", "<cbc:Y/>"]]);
        assert!(
            out.contains("</cbc:Amount>\n    <cbc:Y/>\n  </cac:Line>"),
            "{out}"
        );
        let out = run(&[&["prepend", "/*/cac:Line[1]", "<cbc:Y/>"]]);
        assert!(
            out.contains("<cac:Line>\n    <cbc:Y/>\n    <cbc:ID>1"),
            "{out}"
        );
        let out = run(&[&["rename", "/*/cac:Line[1]/cbc:ID", "cbc:LineID"]]);
        assert!(out.contains("<cbc:LineID>1</cbc:LineID>"), "{out}");
        let out = run(&[
            &["remove-all", "/*/cbc:Missing"],
            &["remove-all", "/*/cac:Line/cbc:Note"],
        ]);
        assert!(!out.contains("<cbc:Note/>"), "{out}");
        let out = run(&[
            &[
                "set",
                "/*/@xmlns:xsi",
                "http://www.w3.org/2001/XMLSchema-instance",
            ],
            &["set", "/*/@xsi:type", "x"],
        ]);
        assert!(out.contains(" xsi:type=\"x\">"), "{out}");
        let out = run(&[&["remove", "/*/cac:Line[1]/cbc:Amount/@currencyID"]]);
        assert!(out.contains("<cbc:Amount> 10.50 </cbc:Amount>"), "{out}");
        let out = run(&[
            &["replace", "cbc:", "b:"],
            &["replace", "xmlns:cbc=", "xmlns:b="],
        ]);
        assert!(out.contains("<b:ID>A1</b:ID>"), "{out}");
        assert!(out.contains(" xmlns:b=\""), "{out}");
        let out = run(&[
            &["set", "/*/cac:Line[1]/@xmlns:x", "u"],
            &["replace-in", "/*/cac:Line[1]", "cbc:", "x:"],
        ]);
        assert!(
            out.contains("<cac:Line xmlns:x=\"u\">\n    <x:ID>1</x:ID>"),
            "{out}"
        );
    }

    #[test]
    fn edits_must_select_and_change() {
        let fail = |ops: &[&str]| apply(DOC, &[edit(ops)], "t").unwrap_err().to_string();
        assert!(fail(&["remove", "/*/cbc:Missing"]).contains("selects nothing"));
        assert!(fail(&["set", "/*/cbc:ID", "A1"]).contains("the edits change nothing"));
        assert!(fail(&["replace", "nowhere", "x"]).contains("does not occur"));
        assert!(fail(&["set-xml", "/*/cbc:ID", "<a>"]).contains("not well-formed"));
        assert!(fail(&["add", "/*/cbc:ID", "1"]).contains("not a decimal"));
        assert!(fail(&["remove", "//cac:Line/cbc:Amount/@x"]).contains("change nothing"));
        assert!(fail(&["remove-all", "/*/cbc:Missing"]).contains("the edits change nothing"));
        let items = ["set", "/*/cbc:ID", "x", "0"]
            .iter()
            .map(|s| crate::toml::Value::String((*s).to_owned()))
            .collect();
        let error = super::parse_edit(crate::toml::Value::Array(items)).unwrap_err();
        assert!(error.to_string().contains("bad repeat count"), "{error}");
    }

    #[test]
    fn catalogue_is_strict() {
        let ok = "[[mutation]]\nid = \"a/b-c\"\nbase = \"invoice\"\nreason = \"why\"\nedits = [[\"remove\", \"/*/cbc:ID\"]]\n";
        assert_eq!(parse_catalogue(ok, "m").unwrap().len(), 1);
        for (text, message) in [
            (ok.replace("a/b-c", "A"), "an id is"),
            (ok.replace("why", " why"), "one non-empty"),
            (ok.replace("remove", "drop"), "unknown edit"),
            (ok.replace("[[mutation]]", "[[m]]"), "unknown table"),
            (format!("{ok}{ok}"), "duplicate id"),
            (ok.replace("edits", "edit"), "unexpected key"),
        ] {
            let error = parse_catalogue(&text, "m").unwrap_err().to_string();
            assert!(error.contains(message), "{error}");
        }
        assert!(valid_id("amounts/payable-plus-0-01"));
        assert!(!valid_id("amounts//x"));
    }
}
