//! Time to evaluate every XPath expression of the CEN EN 16931 UBL 1.3.17 stylesheet on
//! documents: each expression group (a template, or the predicates of a pattern step) at
//! every node its pattern matches, tests as effective boolean values and `select`s as
//! sequences. This is an upper bound of the XPath work of a validation, where each node
//! is processed by one template per mode only.
//!
//! ```text
//! cargo run --release -p einvoice-xpath-eval --example bench-cen                 the largest corpus document
//! cargo run --release -p einvoice-xpath-eval --example bench-cen -- --corpus     every corpus document
//! cargo run --release -p einvoice-xpath-eval --example bench-cen -- FILE...      the given documents
//! cargo run --release -p einvoice-xpath-eval --example bench-cen -- --profile ... also the slowest groups
//! ```

#[path = "../tests/oracle/common.rs"]
mod common;

use std::process::ExitCode;
use std::rc::Rc;
use std::time::{Duration, Instant};

use einvoice_xdm::AtomicValue;
use einvoice_xpath::{
    NamespaceMap, PatternAlternative, PatternAnchor, PatternLink, QName, parse_expr, parse_pattern,
};
use einvoice_xpath_eval::{
    Cardinality, CompiledExpr, Context, Document, DynamicContext, Error, FunctionId, Host, Item,
    Node, SimpleStaticContext, VariableId, compile,
};

const LARGEST: &str = "artefacts/cen-en16931-ubl-1.3.17/ubl/examples/FT G2G_TD01 con Allegato, Bonifico e Split Payment.xml";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// The global parameters (the empty string) followed by the group's local variables.
struct BenchHost {
    values: std::cell::RefCell<Vec<Rc<[Item]>>>,
}

impl Host for BenchHost {
    fn variable(&self, id: VariableId) -> Result<Rc<[Item]>, Error> {
        Ok(Rc::clone(&self.values.borrow()[id.0 as usize]))
    }

    fn call_function(&self, _: FunctionId, _: Vec<Vec<Item>>) -> Result<Vec<Item>, Error> {
        Ok(Vec::new())
    }

    fn document(&self, uri: &str, _: Option<&str>) -> Result<Node, Error> {
        Err(Error::new(
            einvoice_xpath_eval::ErrorCode::Host,
            uri.to_owned(),
        ))
    }

    fn system_property(&self, _: &QName) -> String {
        String::new()
    }
}

struct Group {
    /// Selects the context nodes from the document node.
    contexts: CompiledExpr,
    for_each: Vec<CompiledExpr>,
    variables: Vec<CompiledExpr>,
    /// The expressions and whether each is a test.
    expressions: Vec<(CompiledExpr, bool)>,
}

/// The expression selecting, from the document node, the nodes a pattern matches.
fn pattern_selector(pattern: &str, namespaces: &NamespaceMap) -> Result<String, String> {
    let parsed = parse_pattern(pattern, namespaces).map_err(|e| e.to_string())?;
    let mut alternatives = Vec::new();
    for alternative in &parsed.alternatives {
        alternatives.push(match alternative {
            PatternAlternative::DocumentRoot => "/".to_owned(),
            PatternAlternative::Path(path) => {
                let mut text = match path.anchor {
                    PatternAnchor::Root => "/".to_owned(),
                    PatternAnchor::Relative | PatternAnchor::RootDescendant => "//".to_owned(),
                };
                text.push_str(&path.first.to_string());
                for (link, step) in &path.rest {
                    text.push_str(match link {
                        PatternLink::Child => "/",
                        PatternLink::Descendant => "//",
                    });
                    text.push_str(&step.to_string());
                }
                text
            }
            other => return Err(format!("pattern form not in CEN: {other}")),
        });
    }
    Ok(alternatives.join(" | "))
}

fn compile_groups(root: &std::path::Path) -> Result<(Vec<Group>, usize), String> {
    let inventory = common::cen_inventory(root)?;
    let mut namespaces = NamespaceMap::new();
    for (prefix, uri) in &inventory.namespaces {
        namespaces = namespaces.bind(prefix, uri);
    }
    let compile_in = |text: &str, context: &SimpleStaticContext| -> Result<CompiledExpr, String> {
        let parsed = parse_expr(text, &namespaces).map_err(|e| format!("{text}: {e}"))?;
        compile(&parsed, context).map_err(|e| format!("{text}: {e}"))
    };
    let mut groups = Vec::new();
    let mut count = 0;
    for group in &inventory.groups {
        let mut context = SimpleStaticContext::new()
            .with_namespaces(namespaces.clone())
            .with_xslt_functions();
        for param in &inventory.params {
            context.declare_variable(QName::local(param), Cardinality::Any);
        }
        let contexts = compile_in(&pattern_selector(&group.pattern, &namespaces)?, &context)?;
        let for_each = group
            .for_each
            .iter()
            .map(|s| compile_in(s, &context))
            .collect::<Result<_, _>>()?;
        let mut variables = Vec::new();
        for (name, select) in &group.variables {
            variables.push(compile_in(select, &context)?);
            context.declare_variable(QName::local(name), Cardinality::Any);
        }
        let mut expressions = Vec::new();
        for key in &group.keys {
            let expression = &inventory.expressions[key];
            expressions.push((compile_in(&expression.text, &context)?, expression.test));
            count += 1;
        }
        groups.push(Group {
            contexts,
            for_each,
            variables,
            expressions,
        });
    }
    Ok((groups, count))
}

#[derive(Default)]
struct Totals {
    evaluations: usize,
    errors: usize,
    /// Time per group, for `--profile`.
    per_group: Vec<Duration>,
}

fn context_nodes(group: &Group, dc: &DynamicContext<'_>, root: &Node) -> Result<Vec<Item>, Error> {
    let mut nodes = group
        .contexts
        .evaluate(dc, &Context::for_item(Item::Node(root.clone())))?;
    for select in &group.for_each {
        let mut next = Vec::new();
        for node in &nodes {
            next.extend(select.evaluate(dc, &Context::for_item(node.clone()))?);
        }
        nodes = next;
    }
    Ok(nodes)
}

fn evaluate_document(
    groups: &[Group],
    params: usize,
    doc: &Rc<Document>,
    totals: &mut Totals,
) -> Result<(), String> {
    let host = BenchHost {
        values: std::cell::RefCell::new(Vec::new()),
    };
    let dc = DynamicContext::new(&host).map_err(|e| e.to_string())?;
    let empty: Rc<[Item]> = Rc::from(vec![Item::Atomic(AtomicValue::String(String::new()))]);
    let root = doc.root();
    totals.per_group.resize(groups.len(), Duration::ZERO);
    for (g, group) in groups.iter().enumerate() {
        let group_started = Instant::now();
        *host.values.borrow_mut() = vec![Rc::clone(&empty); params];
        let nodes = context_nodes(group, &dc, &root).map_err(|e| e.to_string())?;
        let size = nodes.len();
        for (i, node) in nodes.into_iter().enumerate() {
            let context = Context::at(node, i + 1, size);
            let mut values = vec![Rc::clone(&empty); params];
            for variable in &group.variables {
                let value = variable.evaluate(&dc, &context).unwrap_or_default();
                values.push(Rc::from(value));
            }
            *host.values.borrow_mut() = values;
            for (expr, test) in &group.expressions {
                totals.evaluations += 1;
                let failed = if *test {
                    expr.effective_boolean_value(&dc, &context).is_err()
                } else {
                    expr.evaluate(&dc, &context).is_err()
                };
                totals.errors += usize::from(failed);
            }
        }
        totals.per_group[g] += group_started.elapsed();
    }
    Ok(())
}

fn run() -> Result<(), String> {
    let root = common::workspace_root();
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let profile = args.first().is_some_and(|a| a == "--profile");
    if profile {
        args.remove(0);
    }
    let files: Vec<std::path::PathBuf> = match args.first().map(String::as_str) {
        None => vec![root.join(LARGEST)],
        Some("--corpus") => {
            let index = common::read(&root.join(common::CEN_CASES))?;
            index
                .lines()
                .skip(1)
                .filter_map(|l| l.split('\t').nth(1))
                .map(|p| root.join(p))
                .collect()
        }
        Some(_) => args.iter().map(std::path::PathBuf::from).collect(),
    };
    let started = Instant::now();
    let (groups, count) = compile_groups(&root)?;
    let patterns: Vec<String> = common::cen_inventory(&root)?
        .groups
        .iter()
        .map(|g| g.pattern.clone())
        .collect();
    let compile_time = started.elapsed();
    let params = common::cen_inventory(&root)?.params.len();
    println!(
        "compiled {count} expression occurrences in {} groups in {compile_time:.2?}",
        groups.len()
    );
    let mut parse_total = Duration::ZERO;
    let mut eval_total = Duration::ZERO;
    let mut slowest = (Duration::ZERO, String::new());
    let mut totals = Totals::default();
    let mut bytes = 0;
    for file in &files {
        let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
        bytes += text.len();
        let t0 = Instant::now();
        let doc = Document::parse(&text, None).map_err(|e| format!("{}: {e}", file.display()))?;
        let t1 = Instant::now();
        evaluate_document(&groups, params, &doc, &mut totals)?;
        let t2 = Instant::now();
        parse_total += t1 - t0;
        eval_total += t2 - t1;
        if t2 - t0 > slowest.0 {
            slowest = (t2 - t0, file.display().to_string());
        }
        if files.len() == 1 {
            println!(
                "{}: {} bytes, {} nodes; parse {:.2?}, evaluate {:.2?}",
                file.display(),
                text.len(),
                doc.node_count(),
                t1 - t0,
                t2 - t1
            );
        }
    }
    println!(
        "{} documents, {bytes} bytes: parse {parse_total:.2?}, evaluate {eval_total:.2?}, total {:.2?}; {} evaluations ({} raised an error); slowest document {:.2?} ({})",
        files.len(),
        parse_total + eval_total,
        totals.evaluations,
        totals.errors,
        slowest.0,
        slowest.1
    );
    if profile {
        let mut ranked: Vec<(Duration, usize)> =
            totals.per_group.iter().copied().zip(0..).collect();
        ranked.sort_unstable_by(|a, b| b.cmp(a));
        for (time, g) in ranked.iter().take(12) {
            println!("{time:>10.2?}  group {g:3}  {}", patterns[*g]);
        }
    }
    Ok(())
}
