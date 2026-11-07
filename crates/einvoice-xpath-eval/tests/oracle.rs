//! Equivalence with Saxon-HE 12.10, expression by expression: every golden line of
//! `corpus/cen-en16931-ubl-1.3.17/xpath/expected.tsv` (the expressions of the CEN
//! stylesheet at nodes their templates match) and of `tests/oracle/expected.tsv` (edge
//! cases) is evaluated with this crate at the same context and compared. Written by the
//! `saxon-oracle` example; see `tests/oracle/common.rs` for the format.

// Integration tests are only ever built with `cfg(test)`; saying so lets clippy apply its
// test exemptions (unwrap, expect, panic) to the helpers outside `#[test]` functions too.
#![cfg(test)]

#[path = "oracle/common.rs"]
mod common;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;

use einvoice_xpath::{NamespaceMap, QName, parse_expr};
use einvoice_xpath_eval::{
    Cardinality, Clock, CompiledExpr, Context, Document, DynamicContext, Error, FunctionId, Host,
    Item, Node, SimpleStaticContext, VariableId, compile, file_uri,
};

/// Edge-case lines whose disagreement with Saxon is understood and accepted, with the
/// reason. Each one must still disagree (a fixed one must be removed from the list). The
/// CEN expressions have none.
const KNOWN_DIFFERENCES: &[(&str, &str, &str)] = &[(
    "uri-07",
    "b",
    "Saxon proves statically that `count(X | /)` is not zero (`/` is never empty) and never \
     evaluates `root(//d:c[1])`, whose argument has two nodes; this crate evaluates it and \
     reports the cardinality error (XPTY0004), as Saxon does for the value",
)];

/// Where the documents are when Saxon reads them (only their URIs are used here).
fn stage_dir() -> PathBuf {
    std::path::absolute(common::workspace_root().join("target/oracle/xpath/run/in")).unwrap()
}

struct TestHost {
    /// Values of the variables, by identity.
    values: RefCell<Vec<Rc<[Item]>>>,
    /// The documents `document()` can load: URI and text. Like Saxon, `document()`
    /// parses the resource again rather than returning the source document.
    texts: BTreeMap<String, String>,
    loaded: RefCell<BTreeMap<String, Node>>,
    any: FunctionId,
}

impl Host for TestHost {
    fn variable(&self, id: VariableId) -> Result<Rc<[Item]>, Error> {
        Ok(Rc::clone(&self.values.borrow()[id.0 as usize]))
    }

    fn call_function(&self, id: FunctionId, mut args: Vec<Vec<Item>>) -> Result<Vec<Item>, Error> {
        assert_eq!(id, self.any);
        Ok(args.remove(0))
    }

    fn document(&self, uri: &str, base: Option<&str>) -> Result<Node, Error> {
        assert_eq!(base, None, "relative document() URIs are not probed");
        if let Some(node) = self.loaded.borrow().get(uri) {
            return Ok(node.clone());
        }
        let text = self
            .texts
            .get(uri)
            .unwrap_or_else(|| panic!("document({uri})"));
        let node = Document::parse_loaded(text, uri).unwrap().root();
        self.loaded
            .borrow_mut()
            .insert(uri.to_owned(), node.clone());
        Ok(node)
    }

    fn system_property(&self, name: &QName) -> String {
        // Saxon-HE 12.10's values (see the edge cases uri-10 to uri-14).
        if name.namespace.as_deref() != Some("http://www.w3.org/1999/XSL/Transform") {
            return String::new();
        }
        match name.local.as_str() {
            "version" => "3.0",
            "vendor" => "Saxonica",
            "product-name" => "SAXON",
            "product-version" => "HE 12.10",
            _ => "",
        }
        .to_owned()
    }
}

/// One set of goldens to replay.
struct Suite {
    name: &'static str,
    namespaces: NamespaceMap,
    /// Global parameters (bound to the empty string, their default value).
    params: Vec<String>,
    /// Key -> (expression text, local variables: name and select).
    expressions: BTreeMap<String, (String, Vec<(String, String)>)>,
    /// Document id -> (file, name when read).
    documents: BTreeMap<String, (PathBuf, String)>,
    golden: PathBuf,
}

fn cen_suite() -> Suite {
    let root = common::workspace_root();
    let inventory = common::cen_inventory(&root).unwrap();
    let mut namespaces = NamespaceMap::new();
    for (prefix, uri) in &inventory.namespaces {
        namespaces = namespaces.bind(prefix, uri);
    }
    let documents = common::cen_documents(&root)
        .unwrap()
        .into_iter()
        .map(|(id, path)| {
            let name = common::staged_name(&path);
            (id, (root.join(path), name))
        })
        .collect();
    Suite {
        name: "CEN",
        namespaces,
        params: inventory.params.clone(),
        expressions: inventory
            .expressions
            .into_iter()
            .map(|(key, e)| (key, (e.text, e.variables)))
            .collect(),
        documents,
        golden: root.join(common::CEN_GOLDEN),
    }
}

fn edge_suite() -> Suite {
    let root = common::workspace_root();
    let inventory = common::cen_inventory(&root).unwrap();
    let mut namespaces = NamespaceMap::new();
    for (prefix, uri) in &inventory.namespaces {
        namespaces = namespaces.bind(prefix, uri);
    }
    for (prefix, uri) in common::EDGE_NAMESPACES {
        namespaces = namespaces.bind(prefix, uri);
    }
    let documents = common::EDGE_DOCS
        .iter()
        .map(|(file, name)| {
            (
                (*file).to_owned(),
                (
                    common::crate_dir().join(common::EDGE_DIR).join(file),
                    (*name).to_owned(),
                ),
            )
        })
        .collect();
    Suite {
        name: "edge",
        namespaces,
        params: Vec::new(),
        expressions: common::edge_cases()
            .unwrap()
            .into_iter()
            .map(|c| (c.id, (c.expr, Vec::new())))
            .collect(),
        documents,
        golden: common::crate_dir().join(common::EDGE_GOLDEN),
    }
}

/// An expression compiled with its local variables, which are compiled too.
struct Compiled {
    expr: CompiledExpr,
    variables: Vec<CompiledExpr>,
}

fn qname(name: &str) -> QName {
    QName::local(name)
}

fn compile_key(suite: &Suite, key: &str) -> Result<Compiled, String> {
    let (text, variables) = suite
        .expressions
        .get(key)
        .ok_or_else(|| format!("unknown key {key}"))?;
    let mut context = SimpleStaticContext::new()
        .with_namespaces(suite.namespaces.clone())
        .with_xslt_functions();
    let any = QName {
        prefix: Some("p".to_owned()),
        namespace: Some(common::PROBE_NS.to_owned()),
        local: "any".to_owned(),
    };
    context.declare_function(any, 1);
    for param in &suite.params {
        context.declare_variable(qname(param), Cardinality::Any);
    }
    let mut compiled_variables = Vec::new();
    for (name, select) in variables {
        let parsed = parse_expr(select, &suite.namespaces).map_err(|e| format!("${name}: {e}"))?;
        compiled_variables.push(compile(&parsed, &context).map_err(|e| format!("${name}: {e}"))?);
        context.declare_variable(qname(name), Cardinality::Any);
    }
    let parsed = parse_expr(text, &suite.namespaces).map_err(|e| e.to_string())?;
    let expr = compile(&parsed, &context).map_err(|e| e.to_string())?;
    Ok(Compiled {
        expr,
        variables: compiled_variables,
    })
}

fn render_item(item: &Item, dir: &str) -> String {
    match item {
        Item::Node(node) => common::render(
            common::node_kind_name(node.kind()),
            &common::node_path(node),
        ),
        Item::Atomic(value) => common::render(
            value.atomic_type().name(),
            &value.to_string().replace(dir, "{dir}"),
        ),
    }
}

fn render(result: Result<Vec<Item>, Error>, dir: &str) -> String {
    match result {
        Err(e) => format!("error:{}", e.code()),
        Ok(items) if items.is_empty() => "empty".to_owned(),
        Ok(items) => items
            .iter()
            .map(|i| render_item(i, dir))
            .collect::<Vec<_>>()
            .join("\t"),
    }
}

/// The result of replaying a suite.
struct Outcome {
    lines: usize,
    /// Lines that disagree with Saxon and are listed in [`KNOWN_DIFFERENCES`].
    known: usize,
    problems: Vec<String>,
}

/// Replays a suite.
fn replay(suite: &Suite, known_differences: &[(&str, &str, &str)]) -> Outcome {
    let stage = stage_dir();
    let dir = file_uri(&stage).unwrap();
    let mut docs = BTreeMap::new();
    let mut texts = BTreeMap::new();
    for (id, (path, name)) in &suite.documents {
        let text = std::fs::read_to_string(path).unwrap();
        let uri = file_uri(&stage.join(name)).unwrap();
        docs.insert(id.clone(), Document::parse(&text, Some(&uri)).unwrap());
        texts.insert(uri, text);
    }
    let golden = common::read(&suite.golden).unwrap();
    let records = common::read_records(&golden).unwrap();
    let any = FunctionId(0);
    let host = TestHost {
        values: RefCell::new(Vec::new()),
        texts,
        loaded: RefCell::new(BTreeMap::new()),
        any,
    };
    // UTC, as the oracle runs Saxon; no case depends on the value of the current date and
    // time, only on its being stable.
    let dc = DynamicContext::new(
        &host,
        Clock::from_unix_millis(0, 1_789_000_123_456).unwrap(),
    );
    let mut compiled: BTreeMap<String, Result<Compiled, String>> = BTreeMap::new();
    let mut problems = Vec::new();
    let mut known_count = 0;
    for record in &records {
        let compiled = compiled
            .entry(record.key.clone())
            .or_insert_with(|| compile_key(suite, &record.key));
        let expected = &record.result;
        let actual = match compiled {
            Err(e) => format!("compile error: {e}"),
            Ok(c) => {
                let doc = &docs[&record.doc];
                let node = common::resolve_path(doc, &record.context)
                    .unwrap_or_else(|| panic!("{}: no node at {}", record.doc, record.context));
                let context = Context::at(Item::Node(node), record.position, record.size);
                let mut values: Vec<Rc<[Item]>> = suite
                    .params
                    .iter()
                    .map(|_| {
                        Rc::from(vec![Item::Atomic(einvoice_xdm::AtomicValue::String(
                            String::new(),
                        ))])
                    })
                    .collect();
                let mut failed = None;
                for variable in &c.variables {
                    *host.values.borrow_mut() = values.clone();
                    match variable.evaluate(&dc, &context) {
                        Ok(items) => values.push(Rc::from(items)),
                        Err(e) => failed = Some(e),
                    }
                }
                *host.values.borrow_mut() = values;
                if let Some(e) = failed {
                    format!("variable error: {e}")
                } else if record.kind == "b" {
                    match c.expr.effective_boolean_value(&dc, &context) {
                        Ok(b) => format!("xs:boolean:{b}"),
                        Err(e) => format!("error:{}", e.code()),
                    }
                } else {
                    render(c.expr.evaluate(&dc, &context), &dir)
                }
            }
        };
        let known = known_differences
            .iter()
            .any(|(key, kind, _)| *key == record.key && *kind == record.kind);
        if known && actual != *expected {
            known_count += 1;
        }
        if (actual == *expected) == known {
            let text = suite
                .expressions
                .get(&record.key)
                .map(|(t, _)| t.as_str())
                .unwrap_or_default();
            problems.push(format!(
                "{} {} {} [{}] {}\n    expression: {text}\n    Saxon: {expected}\n    here:  {actual}{}",
                record.doc,
                record.context,
                record.key,
                record.kind,
                if known { "(listed as a known difference, but agrees)" } else { "" },
                ""
            ));
        }
    }
    Outcome {
        lines: records.len(),
        known: known_count,
        problems,
    }
}

fn check(suite: &Suite, known_differences: &[(&str, &str, &str)]) {
    let outcome = replay(suite, known_differences);
    let keys = common::read_records(&common::read(&suite.golden).unwrap())
        .unwrap()
        .iter()
        .map(|r| r.key.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    for problem in outcome.problems.iter().take(60) {
        eprintln!("{problem}");
    }
    eprintln!(
        "{}: {} of {} golden lines agree with Saxon, {} known differences, {} disagreements ({keys} expressions)",
        suite.name,
        outcome.lines - outcome.problems.len() - outcome.known,
        outcome.lines,
        outcome.known,
        outcome.problems.len(),
    );
    assert!(
        outcome.problems.is_empty(),
        "{}: {} disagreements",
        suite.name,
        outcome.problems.len()
    );
}

#[test]
fn cen_expressions_agree_with_saxon() {
    check(&cen_suite(), &[]);
}

#[test]
fn edge_cases_agree_with_saxon() {
    check(&edge_suite(), KNOWN_DIFFERENCES);
}

#[test]
fn every_cen_expression_is_probed() {
    // Every distinct expression of the stylesheet has goldens, either at nodes of the CEN
    // documents or, for the one no corpus document reaches (a template for processing
    // instructions in an unused mode), as an edge case.
    let root = common::workspace_root();
    let inventory = common::cen_inventory(&root).unwrap();
    let records =
        common::read_records(&common::read(&root.join(common::CEN_GOLDEN)).unwrap()).unwrap();
    let probed: std::collections::BTreeSet<&str> = records.iter().map(|r| r.key.as_str()).collect();
    let edge: Vec<String> = common::edge_cases()
        .unwrap()
        .into_iter()
        .map(|c| c.expr)
        .collect();
    let missing: Vec<&str> = inventory
        .expressions
        .values()
        .filter(|e| !probed.contains(e.key.as_str()) && !edge.contains(&e.text))
        .map(|e| e.text.as_str())
        .collect();
    assert_eq!(missing, Vec::<&str>::new());
    eprintln!(
        "{} distinct expressions ({} occurrences) in {} groups",
        inventory.expressions.len(),
        inventory.occurrences,
        inventory.groups.len()
    );
}
