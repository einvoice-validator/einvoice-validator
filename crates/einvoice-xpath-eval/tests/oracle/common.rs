//! The expression oracle, shared by the `saxon-oracle` example (which runs Saxon-HE and
//! writes the goldens) and the `oracle` test (which runs this crate against them).
//!
//! Two sets of probes:
//!
//! * every distinct XPath expression of the vendored CEN EN 16931 UBL 1.3.17 stylesheet
//!   (`xsl:when`/`xsl:if` tests, `select` attributes, predicates of match patterns),
//!   each evaluated at nodes its template's pattern matches in corpus documents
//!   (goldens in `corpus/cen-en16931-ubl-1.3.17/xpath/`, which are derived from the
//!   CEN artefacts and share their licence);
//! * hand-written edge cases on synthetic documents (`cases.tsv`, `docs/`, goldens in
//!   `expected.tsv`).
//!
//! An expression is identified by a key: `L<line>:<attribute>` for an attribute of the
//! stylesheet (the first occurrence of its text), `L<line>:match:<alternative>.<step>.
//! <predicate>` for a predicate of a match pattern, the case id for an edge case. The
//! goldens hold no expression text: it is read again from the stylesheet.
//!
//! Golden lines, tab-separated: document, context node path, context position, context
//! size, key, `v` (the value) or `b` (the effective boolean value, as `xsl:when`
//! computes it), then one column per item (`type:value`), `error:CODE` or `empty`.
//! A node is written `kind:path`; a path is `/` for the document node, otherwise one
//! `/N` per level (position among the parent's child nodes) and `/@name` for an
//! attribute (`@Q{uri}local` in a namespace). Values longer than 200 characters are
//! written `type:#<first 16 hex digits of their SHA-256>:<length>`. Within a value, the
//! URI of the directory the documents were read from is written `{dir}`.

#![expect(
    dead_code,
    reason = "each of the example and the test uses part of this module"
)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use einvoice_xpath::{Namespaces, PatternAlternative, PatternAxis, parse_pattern};
use sha2::{Digest, Sha256};

pub(crate) const XSL: &str = "http://www.w3.org/1999/XSL/Transform";
/// The vendored CEN stylesheet, relative to the workspace root.
pub(crate) const CEN_XSLT: &str =
    "artefacts/cen-en16931-ubl-1.3.17/ubl/xslt/EN16931-UBL-validation.xslt";
/// The CEN corpus index, relative to the workspace root.
pub(crate) const CEN_CASES: &str = "corpus/cen-en16931-ubl-1.3.17/cases.tsv";
/// The CEN expression goldens and the documents they use, relative to the workspace root.
pub(crate) const CEN_GOLDEN: &str = "corpus/cen-en16931-ubl-1.3.17/xpath/expected.tsv";
pub(crate) const CEN_DOCUMENTS: &str = "corpus/cen-en16931-ubl-1.3.17/xpath/documents.tsv";
/// The edge cases, relative to the crate.
pub(crate) const EDGE_CASES: &str = "tests/oracle/cases.tsv";
pub(crate) const EDGE_GOLDEN: &str = "tests/oracle/expected.tsv";
pub(crate) const EDGE_DIR: &str = "tests/oracle/docs";
/// The namespace of the probe's helper functions.
pub(crate) const PROBE_NS: &str = "urn:einvoice-xpath-eval:probe";
/// Namespaces of the edge cases, besides those of the CEN stylesheet.
pub(crate) const EDGE_NAMESPACES: &[(&str, &str)] =
    &[("t", "urn:t"), ("d", "urn:d"), ("p", PROBE_NS)];
/// The edge-case documents: file in `docs/` and the name the document has when read
/// (its URI tests the encoding of special characters).
pub(crate) const EDGE_DOCS: &[(&str, &str)] = &[
    ("edge.xml", "edge doc é#%,;[1].xml"),
    ("amounts.xml", "amounts.xml"),
];
/// Longest value written verbatim.
const MAX_VALUE: usize = 200;

pub(crate) fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

pub(crate) fn workspace_root() -> PathBuf {
    crate_dir().join("../..")
}

pub(crate) fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

/// One distinct expression.
#[derive(Debug, Clone)]
pub(crate) struct Expression {
    pub(crate) key: String,
    pub(crate) text: String,
    /// Whether some occurrence is a test (`xsl:when`, `xsl:if`, a pattern predicate):
    /// its effective boolean value is probed too.
    pub(crate) test: bool,
    /// The local variables in scope at the first occurrence: name and `select`.
    pub(crate) variables: Vec<(String, String)>,
}

/// Expressions sharing a context: those of one template (or of one `xsl:for-each` in
/// it), or the predicates of one pattern step.
#[derive(Debug, Clone)]
pub(crate) struct Group {
    /// The match pattern that selects the context nodes.
    pub(crate) pattern: String,
    /// Nested `xsl:for-each` selects between the template and the expressions.
    pub(crate) for_each: Vec<String>,
    /// Local variables declared before the expressions: name and `select`.
    pub(crate) variables: Vec<(String, String)>,
    /// The keys of the expressions.
    pub(crate) keys: Vec<String>,
}

/// The expressions of the CEN stylesheet.
#[derive(Debug, Clone)]
pub(crate) struct Inventory {
    /// The namespaces declared on the stylesheet element.
    pub(crate) namespaces: Vec<(String, String)>,
    /// The global parameters (none has a default value).
    pub(crate) params: Vec<String>,
    pub(crate) expressions: BTreeMap<String, Expression>,
    pub(crate) groups: Vec<Group>,
    /// Occurrences of expressions (with duplicates).
    pub(crate) occurrences: usize,
}

struct InScope<'a, 'input>(roxmltree::Node<'a, 'input>);

impl Namespaces for InScope<'_, '_> {
    fn namespace_uri(&self, prefix: &str) -> Option<String> {
        self.0.lookup_namespace_uri(Some(prefix)).map(str::to_owned)
    }
}

fn is_xsl(node: roxmltree::Node<'_, '_>, local: &str) -> bool {
    node.is_element() && node.tag_name().namespace() == Some(XSL) && node.tag_name().name() == local
}

struct Walker<'t> {
    line_starts: Vec<usize>,
    by_text: BTreeMap<String, String>,
    inventory: Inventory,
    text: &'t str,
}

impl Walker<'_> {
    fn line(&self, node: roxmltree::Node<'_, '_>) -> usize {
        self.line_starts
            .partition_point(|&s| s <= node.range().start)
    }

    /// Records an expression, returns its key.
    fn record(
        &mut self,
        key: String,
        text: &str,
        is_test: bool,
        variables: &[(String, String)],
    ) -> String {
        self.inventory.occurrences += 1;
        if let Some(existing) = self.by_text.get(text) {
            let existing = existing.clone();
            if is_test && let Some(e) = self.inventory.expressions.get_mut(&existing) {
                e.test = true;
            }
            return existing;
        }
        self.by_text.insert(text.to_owned(), key.clone());
        self.inventory.expressions.insert(
            key.clone(),
            Expression {
                key: key.clone(),
                text: text.to_owned(),
                test: is_test,
                variables: variables.to_vec(),
            },
        );
        key
    }

    /// Walks a sequence constructor, adding its expressions to `group` and creating a
    /// group per `xsl:for-each`.
    fn body(&mut self, parent: roxmltree::Node<'_, '_>, group: usize) {
        for child in parent.children().filter(roxmltree::Node::is_element) {
            let line = self.line(child);
            let xsl_name =
                (child.tag_name().namespace() == Some(XSL)).then(|| child.tag_name().name());
            let attribute = |walker: &mut Self, attr: &str, test: bool| {
                if let Some(text) = child.attribute(attr) {
                    let variables = walker.inventory.groups[group].variables.clone();
                    let key = walker.record(format!("L{line:05}:{attr}"), text, test, &variables);
                    walker.inventory.groups[group].keys.push(key);
                }
            };
            match xsl_name {
                Some("when" | "if") => attribute(self, "test", true),
                Some("apply-templates" | "value-of" | "sequence" | "attribute") => {
                    attribute(self, "select", false);
                }
                Some("variable") => {
                    attribute(self, "select", false);
                    if let (Some(name), Some(select)) =
                        (child.attribute("name"), child.attribute("select"))
                    {
                        self.inventory.groups[group]
                            .variables
                            .push((name.to_owned(), select.to_owned()));
                    }
                }
                Some("for-each") => {
                    attribute(self, "select", false);
                    let mut inner = self.inventory.groups[group].clone();
                    inner
                        .for_each
                        .push(child.attribute("select").unwrap_or_default().to_owned());
                    inner.keys.clear();
                    self.inventory.groups.push(inner);
                    let id = self.inventory.groups.len() - 1;
                    self.body(child, id);
                    continue;
                }
                _ => {}
            }
            self.body(child, group);
        }
    }

    fn pattern_predicates(
        &mut self,
        template: roxmltree::Node<'_, '_>,
        pattern: &str,
    ) -> Result<(), String> {
        let line = self.line(template);
        let ns = InScope(template);
        let parsed = parse_pattern(pattern, &ns).map_err(|e| format!("line {line}: {e}"))?;
        for (a, alternative) in parsed.alternatives.iter().enumerate() {
            let PatternAlternative::Path(path) = alternative else {
                continue;
            };
            let steps = std::iter::once(&path.first).chain(path.rest.iter().map(|(_, s)| s));
            for (s, step) in steps.enumerate() {
                let einvoice_xpath::PathStep::Axis(step) = step else {
                    continue;
                };
                if step.predicates.is_empty() {
                    continue;
                }
                let test = step.test.to_string();
                let pattern = match step.axis {
                    PatternAxis::Attribute if !test.starts_with("attribute(") => format!("@{test}"),
                    _ => test,
                };
                let mut group = Group {
                    pattern,
                    for_each: Vec::new(),
                    variables: Vec::new(),
                    keys: Vec::new(),
                };
                for (p, predicate) in step.predicates.iter().enumerate() {
                    let key = self.record(
                        format!("L{line:05}:match:{a}.{s}.{p}"),
                        &predicate.to_string(),
                        true,
                        &[],
                    );
                    group.keys.push(key);
                }
                self.inventory.groups.push(group);
            }
        }
        Ok(())
    }
}

/// Reads the expressions of the CEN stylesheet and the contexts they are evaluated in.
pub(crate) fn cen_inventory(root: &Path) -> Result<Inventory, String> {
    let text = read(&root.join(CEN_XSLT))?;
    let doc = roxmltree::Document::parse(&text).map_err(|e| format!("{CEN_XSLT}: {e}"))?;
    let stylesheet = doc.root_element();
    let mut walker = Walker {
        line_starts: std::iter::once(0)
            .chain(text.match_indices('\n').map(|(i, _)| i + 1))
            .collect(),
        by_text: BTreeMap::new(),
        inventory: Inventory {
            namespaces: stylesheet
                .namespaces()
                .filter_map(|n| n.name().map(|p| (p.to_owned(), n.uri().to_owned())))
                .collect(),
            params: Vec::new(),
            expressions: BTreeMap::new(),
            groups: Vec::new(),
            occurrences: 0,
        },
        text: &text,
    };
    for top in stylesheet.children().filter(roxmltree::Node::is_element) {
        if is_xsl(top, "param") {
            if top.has_children() || top.attribute("select").is_some() {
                return Err("a global parameter with a default value".to_owned());
            }
            walker
                .inventory
                .params
                .push(top.attribute("name").unwrap_or_default().to_owned());
        }
        if !is_xsl(top, "template") {
            continue;
        }
        let Some(pattern) = top.attribute("match") else {
            continue;
        };
        walker.inventory.groups.push(Group {
            pattern: pattern.to_owned(),
            for_each: Vec::new(),
            variables: Vec::new(),
            keys: Vec::new(),
        });
        let id = walker.inventory.groups.len() - 1;
        walker.body(top, id);
        walker.pattern_predicates(top, pattern)?;
    }
    let _ = walker.text;
    walker.inventory.groups.retain(|g| !g.keys.is_empty());
    Ok(walker.inventory)
}

/// One hand-written case.
#[derive(Debug, Clone)]
pub(crate) struct EdgeCase {
    pub(crate) id: String,
    /// The file in `docs/`.
    pub(crate) doc: String,
    /// The expression selecting the context nodes (evaluated by Saxon only).
    pub(crate) context: String,
    /// Whether the effective boolean value is probed too (Saxon refuses some statically).
    pub(crate) ebv: bool,
    pub(crate) expr: String,
}

/// Reads `cases.tsv`: `id  document  context  v|vb  expression`, `#` comments.
pub(crate) fn edge_cases() -> Result<Vec<EdgeCase>, String> {
    let text = read(&crate_dir().join(EDGE_CASES))?;
    let mut cases = Vec::new();
    for (n, line) in text.lines().enumerate() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let columns: Vec<&str> = line.split('\t').collect();
        let [id, doc, context, probe, expr] = columns.as_slice() else {
            return Err(format!("{EDGE_CASES}:{}: expected 5 columns", n + 1));
        };
        let ebv = match *probe {
            "v" => false,
            "vb" => true,
            other => return Err(format!("{EDGE_CASES}:{}: probe {other:?}", n + 1)),
        };
        cases.push(EdgeCase {
            id: (*id).to_owned(),
            doc: (*doc).to_owned(),
            context: (*context).to_owned(),
            ebv,
            expr: unescape(expr)?,
        });
    }
    Ok(cases)
}

/// One line of a golden file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Record {
    pub(crate) doc: String,
    pub(crate) context: String,
    pub(crate) position: usize,
    pub(crate) size: usize,
    pub(crate) key: String,
    /// `v` or `b`.
    pub(crate) kind: String,
    pub(crate) result: String,
}

impl Record {
    pub(crate) fn line(&self) -> String {
        format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}",
            self.doc, self.context, self.position, self.size, self.key, self.kind, self.result
        )
    }
}

pub(crate) fn read_records(text: &str) -> Result<Vec<Record>, String> {
    let mut records = Vec::new();
    for (n, line) in text.lines().enumerate() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut columns = line.splitn(7, '\t');
        let mut next = || {
            columns
                .next()
                .ok_or_else(|| format!("golden line {}: too short", n + 1))
        };
        let doc = next()?.to_owned();
        let context = next()?.to_owned();
        let position = next()?
            .parse()
            .map_err(|e| format!("golden line {}: {e}", n + 1))?;
        let size = next()?
            .parse()
            .map_err(|e| format!("golden line {}: {e}", n + 1))?;
        let key = next()?.to_owned();
        let kind = next()?.to_owned();
        let result = next()?.to_owned();
        records.push(Record {
            doc,
            context,
            position,
            size,
            key,
            kind,
            result,
        });
    }
    Ok(records)
}

/// Escapes a value for a column: backslash, tab, line ends, other control characters and
/// invisible spaces.
pub(crate) fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c if c.is_control()
                || matches!(u32::from(c), 0xA0 | 0xAD | 0x2000..=0x200F | 0x2028..=0x202F | 0x2060 | 0xFEFF) =>
            {
                let _ = write!(out, "\\u{{{:X}}}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out
}

/// Reverses the `\t`, `\n`, `\r`, `\\` and `\u{HEX}` escapes of `cases.tsv`; any other
/// backslash stands for itself.
pub(crate) fn unescape(text: &str) -> Result<String, String> {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.peek().copied() {
            Some('\\') => {
                chars.next();
                out.push('\\');
            }
            Some('t') => {
                chars.next();
                out.push('\t');
            }
            Some('n') => {
                chars.next();
                out.push('\n');
            }
            Some('r') => {
                chars.next();
                out.push('\r');
            }
            Some('u') => {
                chars.next();
                if chars.next() != Some('{') {
                    return Err(format!("bad \\u escape in {text:?}"));
                }
                let hex: String = chars.by_ref().take_while(|&c| c != '}').collect();
                let code = u32::from_str_radix(&hex, 16).map_err(|e| format!("{text:?}: {e}"))?;
                out.push(
                    char::from_u32(code).ok_or_else(|| format!("bad code point in {text:?}"))?,
                );
            }
            _ => out.push('\\'),
        }
    }
    Ok(out)
}

/// One item of a result column.
pub(crate) fn render(kind: &str, value: &str) -> String {
    let length = value.chars().count();
    if length > MAX_VALUE {
        let digest = Sha256::digest(value.as_bytes());
        let hex: String = digest.iter().take(8).fold(String::new(), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        });
        return format!("{kind}:#{hex}:{length}");
    }
    format!("{kind}:{}", escape(value))
}

/// The path of a node (see the module documentation).
pub(crate) fn node_path(node: &einvoice_xpath_eval::Node) -> String {
    use einvoice_xpath_eval::NodeKind;
    let mut segments = Vec::new();
    let mut current = Some(node.clone());
    while let Some(n) = current {
        let parent = n.parent();
        match (&parent, n.kind()) {
            (None, _) => {}
            (Some(_), NodeKind::Attribute) => {
                let name = n.name().map(|q| match &q.namespace {
                    Some(ns) => format!("@Q{{{ns}}}{}", q.local),
                    None => format!("@{}", q.local),
                });
                segments.push(name.unwrap_or_default());
            }
            (Some(p), _) => {
                let index = p.children().position(|c| c == n).map_or(0, |i| i + 1);
                segments.push(index.to_string());
            }
        }
        current = parent;
    }
    if segments.is_empty() {
        return "/".to_owned();
    }
    segments.reverse();
    format!("/{}", segments.join("/"))
}

/// The node at a path.
pub(crate) fn resolve_path(
    doc: &std::rc::Rc<einvoice_xpath_eval::Document>,
    path: &str,
) -> Option<einvoice_xpath_eval::Node> {
    let mut node = doc.root();
    let mut rest = path.strip_prefix('/')?;
    while !rest.is_empty() {
        // A namespace URI in `@Q{...}` may contain slashes.
        let end = if rest.starts_with("@Q{") {
            let close = rest.find('}')?;
            close + rest[close..].find('/').unwrap_or(rest.len() - close)
        } else {
            rest.find('/').unwrap_or(rest.len())
        };
        let segment = &rest[..end];
        rest = rest[end..].strip_prefix('/').unwrap_or("");
        node = if let Some(name) = segment.strip_prefix('@') {
            let (ns, local) = match name.strip_prefix("Q{") {
                Some(inner) => {
                    let (ns, local) = inner.split_once('}')?;
                    (Some(ns), local)
                }
                None => (None, name),
            };
            node.attributes().find(|a| {
                a.name()
                    .is_some_and(|q| q.local == local && q.namespace.as_deref() == ns)
            })?
        } else {
            let index: usize = segment.parse().ok()?;
            node.children().nth(index.checked_sub(1)?)?
        };
    }
    Some(node)
}

/// The kind name of a node in a result column.
pub(crate) fn node_kind_name(kind: einvoice_xpath_eval::NodeKind) -> &'static str {
    use einvoice_xpath_eval::NodeKind;
    match kind {
        NodeKind::Document => "document",
        NodeKind::Element => "element",
        NodeKind::Attribute => "attribute",
        NodeKind::Text => "text",
        NodeKind::Comment => "comment",
        NodeKind::ProcessingInstruction => "processing-instruction",
    }
}

/// The CEN corpus documents of the expression oracle: case id and file, relative to the
/// workspace root.
pub(crate) fn cen_documents(root: &Path) -> Result<Vec<(String, String)>, String> {
    let index = read(&root.join(CEN_CASES))?;
    let inputs: BTreeMap<&str, &str> = index
        .lines()
        .skip(1)
        .filter_map(|l| {
            let mut c = l.split('\t');
            Some((c.next()?, c.next()?))
        })
        .collect();
    let list = read(&root.join(CEN_DOCUMENTS))?;
    list.lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|id| {
            inputs
                .get(id)
                .map(|input| (id.to_owned(), (*input).to_owned()))
                .ok_or_else(|| format!("{CEN_DOCUMENTS}: unknown case {id}"))
        })
        .collect()
}

/// The name a CEN document has when read: its file name.
pub(crate) fn staged_name(input: &str) -> String {
    input.rsplit('/').next().unwrap_or(input).to_owned()
}
