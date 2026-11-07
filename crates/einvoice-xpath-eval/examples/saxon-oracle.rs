//! The Saxon-HE oracle of the expression evaluator (see `tests/oracle/common.rs` for
//! what is probed and the golden format).
//!
//! ```text
//! cargo run -p einvoice-xpath-eval --example saxon-oracle                     run Saxon, compare with the goldens
//! cargo run -p einvoice-xpath-eval --example saxon-oracle -- --write          run Saxon, rewrite the goldens
//! cargo run -p einvoice-xpath-eval --example saxon-oracle -- select-documents choose the CEN documents (whole corpus)
//! ```
//!
//! A generated stylesheet evaluates every probe: for the CEN expressions, one mode per
//! group of expressions, whose template has the pattern of the stylesheet's template (or
//! the node test of a pattern step, for predicates) and is applied to every node of the
//! document; for the edge cases, an `xsl:for-each` over the case's context nodes. Saxon
//! runs it on all documents at once. For each CEN expression the goldens keep up to six
//! contexts over all documents, contexts with different results first.
//!
//! `select-documents` runs the CEN probes on every corpus document, then chooses
//! documents greedily (a fixed starting set first) until every group of expressions that
//! some document reaches is reached, and writes the list to `documents.tsv`.
//!
//! Saxon-HE 9.9.1-8 is taken from `$SAXON_JAR`, else where `scripts/fetch-saxon.sh` puts
//! it; its SHA-256 is checked. Java from `$JAVA`, else `java`. Saxon runs with the UTC
//! timezone, the implicit timezone the test uses.

#[path = "../tests/oracle/common.rs"]
mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use common::{EdgeCase, Inventory, Record};
use sha2::{Digest, Sha256};

const SAXON_SHA256: &str = "504ad5c71b55a3fd04c5609419b5e51d51b419f26628b4b6254c38766487abdf";
const SAXON_DEFAULT: &str = "target/oracle/saxon/Saxon-HE-9.9.1-8.jar";
const WORK_DIR: &str = "target/oracle/xpath";
/// Contexts kept per CEN expression.
const CONTEXTS_PER_EXPRESSION: usize = 6;
/// The documents `select-documents` starts from.
const STARTING_DOCUMENTS: &[&str] = &[
    "examples/ubl-tc434-example1.xml",
    "examples/ubl-tc434-creditnote1.xml",
    "examples/FT G2G_TD01 con Allegato, Bonifico e Split Payment.xml",
    "examples/sample-discount-price.xml",
    "testfiles/BIS_Billing_30-Inkopskort.xml",
    "vefa/Invoice-unit-UBL/UBL-DT-01.002.xml",
    "vefa/Invoice-unit-UBL/BR-IC-08.007.xml",
];

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        [] => oracle(false),
        ["--write"] => oracle(true),
        ["select-documents"] => select_documents(),
        _ => Err("usage: saxon-oracle [--write | select-documents]".to_owned()),
    }
}

fn write(path: &Path, text: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

fn saxon_jar() -> Result<PathBuf, String> {
    let jar = std::env::var_os("SAXON_JAR").map_or_else(
        || common::workspace_root().join(SAXON_DEFAULT),
        PathBuf::from,
    );
    let bytes = std::fs::read(&jar).map_err(|e| {
        format!(
            "{}: {e} (run scripts/fetch-saxon.sh or set SAXON_JAR)",
            jar.display()
        )
    })?;
    let hash = Sha256::digest(&bytes)
        .iter()
        .fold(String::new(), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        });
    if hash != SAXON_SHA256 {
        return Err(format!(
            "{}: SHA-256 {hash}, expected {SAXON_SHA256}",
            jar.display()
        ));
    }
    Ok(jar)
}

/// Runs Saxon on every file of `input` with `stylesheet`, into `output`.
fn run_saxon(stylesheet: &Path, input: &Path, output: &Path) -> Result<(), String> {
    let jar = saxon_jar()?;
    let java = std::env::var_os("JAVA").unwrap_or_else(|| "java".into());
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get().min(8));
    let result = Command::new(&java)
        .env("TZ", "UTC")
        .arg("-Duser.timezone=UTC")
        .arg("-jar")
        .arg(&jar)
        .arg(format!("-xsl:{}", stylesheet.display()))
        .arg(format!("-s:{}", input.display()))
        .arg(format!("-o:{}", output.display()))
        .arg(format!("-threads:{threads}"))
        .output()
        .map_err(|e| format!("cannot run {}: {e}", java.display()))?;
    if !result.status.success() || !result.stderr.is_empty() {
        return Err(format!(
            "Saxon failed ({}); a clean run prints nothing:\n{}",
            result.status,
            String::from_utf8_lossy(&result.stderr)
        ));
    }
    Ok(())
}

fn xml_attr(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\t' | '\n' | '\r' => {
                let _ = write!(out, "&#{};", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out
}

/// The probe of one expression: its value and, for a test, its effective boolean value.
fn probe_blocks(out: &mut String, key: &str, text: &str, with_ebv: bool) {
    let key = xml_attr(key);
    let select = xml_attr(text);
    let _ = writeln!(
        out,
        "<x e=\"{key}\"><xsl:try><xsl:variable name=\"p:r\" select=\"{select}\"/>\
         <xsl:for-each select=\"$p:r\"><v t=\"{{p:type(.)}}\"><xsl:value-of select=\"p:ser(.)\"/></v></xsl:for-each>\
         <xsl:catch><e c=\"{{local-name-from-QName($err:code)}}\"/></xsl:catch></xsl:try></x>"
    );
    if with_ebv {
        let _ = writeln!(
            out,
            "<b e=\"{key}\"><xsl:try><xsl:choose><xsl:when test=\"{select}\">true</xsl:when>\
             <xsl:otherwise>false</xsl:otherwise></xsl:choose>\
             <xsl:catch><e c=\"{{local-name-from-QName($err:code)}}\"/></xsl:catch></xsl:try></b>"
        );
    }
}

const HELPERS: &str = r#"
<xsl:output method="xml" indent="no" encoding="UTF-8"/>
<xsl:function name="p:any" as="item()*">
  <xsl:param name="x" as="item()*"/>
  <xsl:sequence select="if (empty($x) and exists($x)) then p:any($x) else $x"/>
</xsl:function>
<xsl:function name="p:type" as="xs:string">
  <xsl:param name="v" as="item()"/>
  <xsl:choose>
    <xsl:when test="$v instance of document-node()">document</xsl:when>
    <xsl:when test="$v instance of element()">element</xsl:when>
    <xsl:when test="$v instance of attribute()">attribute</xsl:when>
    <xsl:when test="$v instance of text()">text</xsl:when>
    <xsl:when test="$v instance of comment()">comment</xsl:when>
    <xsl:when test="$v instance of processing-instruction()">processing-instruction</xsl:when>
    <xsl:when test="$v instance of xs:integer">xs:integer</xsl:when>
    <xsl:when test="$v instance of xs:decimal">xs:decimal</xsl:when>
    <xsl:when test="$v instance of xs:double">xs:double</xsl:when>
    <xsl:when test="$v instance of xs:boolean">xs:boolean</xsl:when>
    <xsl:when test="$v instance of xs:untypedAtomic">xs:untypedAtomic</xsl:when>
    <xsl:when test="$v instance of xs:anyURI">xs:anyURI</xsl:when>
    <xsl:when test="$v instance of xs:string">xs:string</xsl:when>
    <xsl:when test="$v instance of xs:dateTime">xs:dateTime</xsl:when>
    <xsl:when test="$v instance of xs:date">xs:date</xsl:when>
    <xsl:otherwise>other</xsl:otherwise>
  </xsl:choose>
</xsl:function>
<xsl:function name="p:path" as="xs:string">
  <xsl:param name="n" as="node()"/>
  <xsl:sequence select="if ($n instance of document-node()) then '/' else
    concat('/', string-join(
      for $a in $n/ancestor-or-self::node()[parent::node()] return
        if ($a instance of attribute())
        then concat('@', if (namespace-uri($a) = '') then local-name($a)
                         else concat('Q{', namespace-uri($a), '}', local-name($a)))
        else string(count($a/preceding-sibling::node()) + 1),
      '/'))"/>
</xsl:function>
<xsl:function name="p:ser" as="xs:string">
  <xsl:param name="v" as="item()"/>
  <xsl:sequence select="if ($v instance of node()) then p:path($v) else string($v)"/>
</xsl:function>
"#;

/// The probe stylesheet. With `coverage`, the CEN groups only report that they matched.
fn probe(
    inventory: &Inventory,
    cen_names: &[String],
    edge: &[(String, Vec<&EdgeCase>)],
    coverage: bool,
) -> String {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<xsl:stylesheet version=\"2.0\"\n",
    );
    let mut namespaces: BTreeMap<String, String> = inventory.namespaces.iter().cloned().collect();
    for (prefix, uri) in common::EDGE_NAMESPACES {
        namespaces.insert((*prefix).to_owned(), (*uri).to_owned());
    }
    namespaces.insert("xsl".to_owned(), common::XSL.to_owned());
    namespaces.insert(
        "err".to_owned(),
        "http://www.w3.org/2005/xqt-errors".to_owned(),
    );
    for (prefix, uri) in &namespaces {
        let _ = writeln!(out, "    xmlns:{prefix}=\"{}\"", xml_attr(uri));
    }
    out.push_str("    exclude-result-prefixes=\"#all\">\n");
    out.push_str(HELPERS);
    for param in &inventory.params {
        let _ = writeln!(out, "<xsl:param name=\"{param}\"/>");
    }
    out.push_str("<xsl:template match=\"/\">\n<xsl:variable name=\"p:name\" select=\"replace(string(document-uri(/)), '^.*/', '')\"/>\n<results name=\"{$p:name}\">\n");
    let names: Vec<String> = cen_names
        .iter()
        .map(|n| format!("'{}'", n.replace('\'', "''")))
        .collect();
    if coverage {
        out.push_str("<xsl:if test=\"true()\">\n");
    } else {
        let _ = writeln!(
            out,
            "<xsl:if test=\"$p:name = ({})\">",
            xml_attr(&names.join(", "))
        );
    }
    for i in 0..inventory.groups.len() {
        let _ = writeln!(
            out,
            "<xsl:apply-templates select=\"(/, //node(), //@*)\" mode=\"p:g{i}\"/>"
        );
    }
    out.push_str("</xsl:if>\n");
    for (name, cases) in edge {
        let _ = writeln!(
            out,
            "<xsl:if test=\"$p:name = '{}'\">",
            xml_attr(&name.replace('\'', "''"))
        );
        for case in cases {
            let _ = writeln!(
                out,
                "<xsl:for-each select=\"{}\"><c g=\"edge\" p=\"{{p:path(.)}}\" n=\"{{position()}}\" s=\"{{last()}}\">",
                xml_attr(&case.context)
            );
            probe_blocks(&mut out, &case.id, &case.expr, case.ebv);
            out.push_str("</c></xsl:for-each>\n");
        }
        out.push_str("</xsl:if>\n");
    }
    out.push_str("</results>\n</xsl:template>\n");
    for (i, group) in inventory.groups.iter().enumerate() {
        let _ = writeln!(
            out,
            "<xsl:template match=\"{}\" mode=\"p:g{i}\" priority=\"1\">",
            xml_attr(&group.pattern)
        );
        if coverage {
            let _ = writeln!(out, "<g n=\"{i}\"/>");
        } else {
            for select in &group.for_each {
                let _ = writeln!(out, "<xsl:for-each select=\"{}\">", xml_attr(select));
            }
            let _ = writeln!(
                out,
                "<c g=\"{i}\" p=\"{{p:path(.)}}\" n=\"{{position()}}\" s=\"{{last()}}\">"
            );
            for (name, select) in &group.variables {
                let _ = writeln!(
                    out,
                    "<xsl:variable name=\"{name}\" select=\"{}\"/>",
                    xml_attr(select)
                );
            }
            for key in &group.keys {
                let expression = &inventory.expressions[key];
                probe_blocks(&mut out, key, &expression.text, expression.test);
            }
            out.push_str("</c>\n");
            for _ in &group.for_each {
                out.push_str("</xsl:for-each>\n");
            }
        }
        out.push_str("</xsl:template>\n");
        let _ = writeln!(
            out,
            "<xsl:template match=\"/ | node() | @*\" mode=\"p:g{i}\" priority=\"-1\"/>"
        );
    }
    out.push_str("</xsl:stylesheet>\n");
    out
}

/// What Saxon returned for one context: the group, the context and the probe results
/// (key, `v`/`b`, result columns).
struct Context {
    path: String,
    position: usize,
    size: usize,
    results: Vec<(String, String, String)>,
}

/// Parses one output file: the name Saxon saw, and the contexts.
fn parse_output(xml: &str, dir_uri: &str) -> Result<(String, Vec<Context>), String> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| format!("Saxon output: {e}"))?;
    let root = doc.root_element();
    let name = root.attribute("name").unwrap_or_default().to_owned();
    let mut contexts = Vec::new();
    for c in root.children().filter(|n| n.has_tag_name("c")) {
        let number = |a: &str| -> Result<usize, String> {
            c.attribute(a)
                .unwrap_or_default()
                .parse()
                .map_err(|e| format!("attribute {a}: {e}"))
        };
        let mut results = Vec::new();
        for block in c.children().filter(roxmltree::Node::is_element) {
            let key = block.attribute("e").unwrap_or_default().to_owned();
            let kind = block.tag_name().name().to_owned();
            let mut columns = Vec::new();
            for item in block.children().filter(roxmltree::Node::is_element) {
                if item.has_tag_name("e") {
                    columns.push(format!("error:{}", item.attribute("c").unwrap_or_default()));
                } else if item.has_tag_name("v") {
                    let text: String = item.children().filter_map(|n| n.text()).collect();
                    columns.push(common::render(
                        item.attribute("t").unwrap_or_default(),
                        &text.replace(dir_uri, "{dir}"),
                    ));
                }
            }
            let text: String = block.children().filter_map(|n| n.text()).collect();
            if kind == "b" && columns.is_empty() {
                columns.push(format!("xs:boolean:{}", text.trim()));
            }
            if columns.is_empty() {
                columns.push("empty".to_owned());
            }
            results.push((
                key,
                if kind == "x" {
                    "v".to_owned()
                } else {
                    "b".to_owned()
                },
                columns.join("\t"),
            ));
        }
        contexts.push(Context {
            path: c.attribute("p").unwrap_or_default().to_owned(),
            position: number("n")?,
            size: number("s")?,
            results,
        });
    }
    Ok((name, contexts))
}

fn work_dir() -> PathBuf {
    common::workspace_root().join(WORK_DIR)
}

fn reset(dir: &Path) -> Result<(), String> {
    if dir.exists() {
        std::fs::remove_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))
}

fn copy(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::copy(from, to)
        .map(drop)
        .map_err(|e| format!("{} -> {}: {e}", from.display(), to.display()))
}

/// The URI Saxon gives the staging directory, as the evaluator computes it.
fn dir_uri(input: &Path) -> Result<String, String> {
    let absolute = std::path::absolute(input).map_err(|e| e.to_string())?;
    einvoice_xpath_eval::file_uri(&absolute)
        .ok_or_else(|| format!("{}: no file URI", input.display()))
}

/// The documents copied where Saxon reads them, by the last segment of their URI.
struct Staged {
    input: PathBuf,
    /// The URI of `input`, written `{dir}` in the goldens.
    dir: String,
    /// The file names in `input`.
    files: Vec<String>,
    /// URI name of a CEN document -> its case id.
    cen: BTreeMap<String, String>,
    /// URI name of an edge document -> its file in `docs/`.
    edge: BTreeMap<String, String>,
}

fn stage(documents: &[(String, String)], input: &Path) -> Result<Staged, String> {
    let root = common::workspace_root();
    reset(input)?;
    let mut staged = Staged {
        input: input.to_owned(),
        dir: dir_uri(input)?,
        files: Vec::new(),
        cen: BTreeMap::new(),
        edge: BTreeMap::new(),
    };
    for (id, path) in documents {
        let name = common::staged_name(path);
        if staged.files.contains(&name) {
            return Err(format!("two CEN documents named {name}"));
        }
        copy(&root.join(path), &input.join(&name))?;
        staged.cen.insert(uri_name(input, &name)?, id.clone());
        staged.files.push(name);
    }
    for (file, name) in common::EDGE_DOCS {
        copy(
            &common::crate_dir().join(common::EDGE_DIR).join(file),
            &input.join(name),
        )?;
        staged
            .edge
            .insert(uri_name(input, name)?, (*file).to_owned());
        staged.files.push((*name).to_owned());
    }
    Ok(staged)
}

/// Saxon's results: the edge-case lines, and the candidate contexts of each CEN document.
type Collected = (Vec<Record>, BTreeMap<String, Vec<(Context, usize)>>);

fn collect(staged: &Staged, output: &Path) -> Result<Collected, String> {
    let mut cen_candidates: BTreeMap<String, Vec<(Context, usize)>> = BTreeMap::new();
    let mut edge_records = Vec::new();
    let mut seen_edge = BTreeSet::new();
    for name in &staged.files {
        let xml = common::read(&output.join(name))?;
        let (saxon_name, contexts) = parse_output(&xml, &staged.dir)?;
        if let Some(file) = staged.edge.get(&saxon_name) {
            seen_edge.insert(file.clone());
            for context in contexts {
                for (key, kind, result) in &context.results {
                    edge_records.push(Record {
                        doc: file.clone(),
                        context: context.path.clone(),
                        position: context.position,
                        size: context.size,
                        key: key.clone(),
                        kind: kind.clone(),
                        result: result.clone(),
                    });
                }
            }
        } else if let Some(id) = staged.cen.get(&saxon_name) {
            let entry = cen_candidates.entry(id.clone()).or_default();
            for (order, context) in contexts.into_iter().enumerate() {
                entry.push((context, order));
            }
        } else {
            return Err(format!(
                "Saxon named a document {saxon_name:?}, which is none of the staged names: the document URI differs from file_uri()"
            ));
        }
    }
    for (file, _) in common::EDGE_DOCS {
        if !seen_edge.contains(*file) {
            return Err(format!("no results for {file}"));
        }
    }
    Ok((edge_records, cen_candidates))
}

/// Writes the goldens, or compares them with what Saxon gave.
fn write_or_compare(targets: &[(PathBuf, String)], write_goldens: bool) -> Result<(), String> {
    if write_goldens {
        for (path, text) in targets {
            write(path, text)?;
            println!("written: {}", path.display());
        }
        return Ok(());
    }
    let mut problems = Vec::new();
    for (path, text) in targets {
        let committed = common::read(path)?;
        if committed != *text {
            let old: BTreeSet<&str> = committed.lines().collect();
            let new: BTreeSet<&str> = text.lines().collect();
            for line in new.difference(&old).take(20) {
                problems.push(format!("{}: Saxon gives {line}", path.display()));
            }
            for line in old.difference(&new).take(20) {
                problems.push(format!("{}: golden has {line}", path.display()));
            }
            if problems.is_empty() {
                problems.push(format!("{}: lines in another order", path.display()));
            }
        }
    }
    if problems.is_empty() {
        println!("goldens match Saxon");
        Ok(())
    } else {
        Err(format!(
            "goldens differ from Saxon (review, then rerun with --write):\n  {}",
            problems.join("\n  ")
        ))
    }
}

fn oracle(write_goldens: bool) -> Result<(), String> {
    let root = common::workspace_root();
    let inventory = common::cen_inventory(&root)?;
    let documents = common::cen_documents(&root)?;
    let cases = common::edge_cases()?;
    let work = work_dir().join("run");
    let output = work.join("out");
    reset(&output)?;
    let staged = stage(&documents, &work.join("in"))?;
    let cen_names: Vec<String> = staged.cen.keys().cloned().collect();
    let edge: Vec<(String, Vec<&EdgeCase>)> = staged
        .edge
        .iter()
        .map(|(uri_name, file)| {
            (
                uri_name.clone(),
                cases.iter().filter(|c| c.doc == *file).collect(),
            )
        })
        .collect();
    let stylesheet = work.join("probe.xsl");
    write(&stylesheet, &probe(&inventory, &cen_names, &edge, false))?;
    eprintln!(
        "oracle: Saxon on {} CEN documents ({} expressions in {} groups) and {} edge cases",
        documents.len(),
        inventory.expressions.len(),
        inventory.groups.len(),
        cases.len()
    );
    run_saxon(&stylesheet, &staged.input, &output)?;
    let (edge_records, cen_candidates) = collect(&staged, &output)?;
    let cen_records = select_contexts(&inventory, &documents, &cen_candidates)?;
    let edge_text = golden_text(
        "Saxon-HE 9.9.1-8 results of the edge cases (tests/oracle/cases.tsv).",
        &edge_records,
    );
    let cen_text = golden_text(
        "Saxon-HE 9.9.1-8 results of the expressions of ubl/xslt/EN16931-UBL-validation.xslt\n\
         # at nodes their templates match in the documents of documents.tsv.",
        &cen_records,
    );
    for text in [&edge_text, &cen_text] {
        let lowered = text.to_lowercase();
        if lowered.contains(&staged.dir.to_lowercase()) || lowered.contains("file:/") {
            return Err("a golden still contains a local file URI".to_owned());
        }
    }
    let keys: BTreeSet<&str> = cen_records.iter().map(|r| r.key.as_str()).collect();
    println!(
        "CEN: {} expressions, {} evaluated ({} golden lines); edge cases: {} ({} golden lines)",
        inventory.expressions.len(),
        keys.len(),
        cen_records.len(),
        cases.len(),
        edge_records.len()
    );
    write_or_compare(
        &[
            (common::crate_dir().join(common::EDGE_GOLDEN), edge_text),
            (root.join(common::CEN_GOLDEN), cen_text),
        ],
        write_goldens,
    )
}

/// The last segment of the URI of a staged file.
fn uri_name(input: &Path, name: &str) -> Result<String, String> {
    let uri = dir_uri(&input.join(name))?;
    Ok(uri.rsplit('/').next().unwrap_or_default().to_owned())
}

fn golden_text(title: &str, records: &[Record]) -> String {
    let mut text = format!(
        "# {title}\n# Written by `cargo run -p einvoice-xpath-eval --example saxon-oracle -- --write`; do not edit.\n\
         # document\tcontext\tposition\tsize\tkey\tv|b\tresult...\n"
    );
    for record in records {
        text.push_str(&record.line());
        text.push('\n');
    }
    text
}

/// Keeps, for each CEN expression, up to [`CONTEXTS_PER_EXPRESSION`] contexts over all
/// documents: one of each distinct value first (round robin), in document list order and
/// then Saxon's order. A context reached by two groups keeps its first results; differing
/// results for the same context are an error.
fn select_contexts(
    inventory: &Inventory,
    documents: &[(String, String)],
    candidates: &BTreeMap<String, Vec<(Context, usize)>>,
) -> Result<Vec<Record>, String> {
    // key -> (doc, path) -> (position, size, v result, b result)
    type Seen = BTreeMap<(usize, String), (usize, usize, String, Option<String>, usize)>;
    let mut per_key: BTreeMap<String, Seen> = BTreeMap::new();
    let doc_index: BTreeMap<&str, usize> = documents
        .iter()
        .enumerate()
        .map(|(i, (id, _))| (id.as_str(), i))
        .collect();
    for (id, contexts) in candidates {
        let d = doc_index[id.as_str()];
        for (context, order) in contexts {
            let mut values: BTreeMap<&str, (String, Option<String>)> = BTreeMap::new();
            for (key, kind, result) in &context.results {
                let entry = values.entry(key).or_insert_with(|| (String::new(), None));
                if kind == "v" {
                    entry.0.clone_from(result);
                } else {
                    entry.1 = Some(result.clone());
                }
            }
            for (key, (value, ebv)) in values {
                let seen = per_key.entry(key.to_owned()).or_default();
                match seen.get(&(d, context.path.clone())) {
                    Some((_, _, v, b, _)) => {
                        if *v != value || *b != ebv {
                            return Err(format!(
                                "{key} at {id} {}: different results in two groups",
                                context.path
                            ));
                        }
                    }
                    None => {
                        seen.insert(
                            (d, context.path.clone()),
                            (context.position, context.size, value, ebv, *order),
                        );
                    }
                }
            }
        }
    }
    let mut records = Vec::new();
    for (key, seen) in per_key {
        if !inventory.expressions.contains_key(&key) {
            return Err(format!("Saxon returned an unknown key {key}"));
        }
        let mut ordered: Vec<_> = seen.into_iter().collect();
        ordered.sort_by_key(|a| (a.0.0, a.1.4));
        let mut classes: Vec<(String, Vec<usize>)> = Vec::new();
        for (i, (_, (_, _, value, ebv, _))) in ordered.iter().enumerate() {
            let class = format!("{value}|{ebv:?}");
            match classes.iter_mut().find(|(c, _)| *c == class) {
                Some((_, members)) => members.push(i),
                None => classes.push((class, vec![i])),
            }
        }
        let mut chosen = Vec::new();
        let mut round = 0;
        while chosen.len() < CONTEXTS_PER_EXPRESSION {
            let mut any = false;
            for (_, members) in &classes {
                if let Some(&i) = members.get(round) {
                    any = true;
                    if chosen.len() < CONTEXTS_PER_EXPRESSION {
                        chosen.push(i);
                    }
                }
            }
            if !any {
                break;
            }
            round += 1;
        }
        chosen.sort_unstable();
        for i in chosen {
            let ((d, path), (position, size, value, ebv, _)) = &ordered[i];
            let doc = documents[*d].0.clone();
            records.push(Record {
                doc: doc.clone(),
                context: path.clone(),
                position: *position,
                size: *size,
                key: key.clone(),
                kind: "v".to_owned(),
                result: value.clone(),
            });
            if let Some(ebv) = ebv {
                records.push(Record {
                    doc,
                    context: path.clone(),
                    position: *position,
                    size: *size,
                    key: key.clone(),
                    kind: "b".to_owned(),
                    result: ebv.clone(),
                });
            }
        }
    }
    Ok(records)
}

fn select_documents() -> Result<(), String> {
    let root = common::workspace_root();
    let inventory = common::cen_inventory(&root)?;
    let index = common::read(&root.join(common::CEN_CASES))?;
    let all: Vec<(String, String)> = index
        .lines()
        .skip(1)
        .filter_map(|l| {
            let mut c = l.split('\t');
            Some((c.next()?.to_owned(), c.next()?.to_owned()))
        })
        .collect();
    let work = work_dir().join("select");
    let input = work.join("in");
    let output = work.join("out");
    reset(&input)?;
    reset(&output)?;
    for (i, (_, path)) in all.iter().enumerate() {
        copy(&root.join(path), &input.join(format!("c{:05}.xml", i + 1)))?;
    }
    let stylesheet = work.join("probe.xsl");
    write(&stylesheet, &probe(&inventory, &[], &[], true))?;
    eprintln!("select-documents: Saxon on {} corpus documents", all.len());
    run_saxon(&stylesheet, &input, &output)?;
    let mut reached: Vec<BTreeSet<usize>> = Vec::new();
    for i in 0..all.len() {
        let xml = common::read(&output.join(format!("c{:05}.xml", i + 1)))?;
        let doc = roxmltree::Document::parse(&xml).map_err(|e| e.to_string())?;
        reached.push(
            doc.descendants()
                .filter(|n| n.has_tag_name("g"))
                .filter_map(|n| n.attribute("n")?.parse().ok())
                .collect(),
        );
    }
    let reachable: BTreeSet<usize> = reached.iter().flatten().copied().collect();
    let mut chosen: Vec<usize> = Vec::new();
    let mut covered = BTreeSet::new();
    for id in STARTING_DOCUMENTS {
        let i = all
            .iter()
            .position(|(c, _)| c == id)
            .ok_or_else(|| format!("unknown starting document {id}"))?;
        chosen.push(i);
        covered.extend(reached[i].iter().copied());
    }
    loop {
        let best = (0..all.len())
            .filter(|i| !chosen.contains(i))
            .map(|i| {
                (
                    reached[i].difference(&covered).count(),
                    std::cmp::Reverse(i),
                )
            })
            .max();
        match best {
            Some((gain, std::cmp::Reverse(i))) if gain > 0 => {
                chosen.push(i);
                covered.extend(reached[i].iter().copied());
            }
            _ => break,
        }
    }
    let unreached: Vec<String> = (0..inventory.groups.len())
        .filter(|g| !reachable.contains(g))
        .map(|g| inventory.groups[g].pattern.clone())
        .collect();
    let mut text = format!(
        "# The corpus documents of the expression oracle, chosen by\n\
         # `cargo run -p einvoice-xpath-eval --example saxon-oracle -- select-documents`:\n\
         # {} groups of expressions, {} reached by some corpus document, all of them by these\n\
         # {} documents.\n",
        inventory.groups.len(),
        reachable.len(),
        chosen.len()
    );
    for i in &chosen {
        text.push_str(&all[*i].0);
        text.push('\n');
    }
    write(&root.join(common::CEN_DOCUMENTS), &text)?;
    println!(
        "{} documents chosen; {} of {} groups reached; never reached: {unreached:?}",
        chosen.len(),
        reachable.len(),
        inventory.groups.len()
    );
    Ok(())
}
