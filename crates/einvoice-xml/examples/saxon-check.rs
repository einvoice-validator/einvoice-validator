//! Saxon-HE on every hand-written case of the tests (`tests/common/cases.rs`): its
//! verdict, and for the documents it accepts its data model, must be the reader's,
//! except for the cases listed in [`EXPECTED`] (document type declarations, which the
//! reader refuses by design, and what Saxon's XML parser accepts against Namespaces in
//! XML or that `roxmltree` refuses). In particular, every document where the reader
//! departs from `roxmltree` gets Saxon's verdict and data.
//!
//! ```text
//! cargo run -p einvoice-xml --example saxon-check
//! ```
//!
//! Saxon-HE 12.10 is taken from `$SAXON_JAR`, else where `scripts/fetch-saxon.sh` puts
//! it (`target/oracle/saxon/`), with the two XML Resolver jars it needs from the same
//! directory; the SHA-256 of each jar is checked. Java from `$JAVA`, else `java`.

#[path = "../tests/common/mod.rs"]
mod common;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use common::Outcome;
use common::cases::{DATA, LINE_ENDS, MALFORMED, STRICT, WELL_FORMED};
use sha2::{Digest, Sha256};

const SAXON_JARS: [(&str, &str); 3] = [
    (
        "Saxon-HE-12.10.jar",
        "b571af282f25d7301059f788b9a149aab8b5cdc14ef3d212dc5425d3dcbb9a97",
    ),
    (
        "xmlresolver-5.3.3.jar",
        "1fe4d5b92f708dcdb82dbce12919e0171e6b5ca62c6dca6220483625098feb5f",
    ),
    (
        "xmlresolver-5.3.3-data.jar",
        "b0c487ad2f3e558be8d829c916d2458d10aca6a5bafa7a4d0524b70845e48a5c",
    ),
];
const SAXON_DEFAULT: &str = "target/oracle/saxon/Saxon-HE-12.10.jar";

/// The cases where Saxon-HE and the reader differ, and why.
const EXPECTED: &[(&str, &str)] = &[
    ("doctype", "a document type declaration: refused by design"),
    (
        "doctype after a comment",
        "a document type declaration: refused by design",
    ),
    (
        "doctype with an internal subset",
        "a document type declaration: refused by design",
    ),
    (
        "xml element prefix",
        "the xml prefix on an element: refused as roxmltree refuses it",
    ),
    (
        "xml element prefix declared",
        "the xml prefix on an element: refused as roxmltree refuses it",
    ),
    (
        "only a colon",
        "Saxon's XML parser accepts the name `:`, which is not a qualified name (roxmltree refuses it too)",
    ),
    (
        "element with an empty prefix",
        "Saxon's XML parser accepts `<:a/>`, which is not a qualified name",
    ),
    (
        "attribute with an empty prefix",
        "Saxon's XML parser accepts `:b='1'`, which is not a qualified name",
    ),
    (
        "names with combining marks",
        "U+203F is a name character in XML 1.0 fifth edition, as for roxmltree; Saxon's XML parser uses older tables",
    ),
    (
        "namespace with whitespace",
        "Saxon trims the namespace name; the reader keeps the normalised attribute value, as Namespaces in XML and roxmltree do",
    ),
    (
        "prefixed xmlns attribute with the prefix bound",
        "an attribute for XML, a default namespace declaration for roxmltree: refused by design",
    ),
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

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

const STYLESHEET: &str = r#"<xsl:stylesheet version="3.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform"
    xmlns:xs="http://www.w3.org/2001/XMLSchema" exclude-result-prefixes="xs">
  <xsl:output method="xml" indent="no"/>
  <xsl:param name="index" as="xs:string"/>
  <xsl:template name="xsl:initial-template">
    <cases>
      <xsl:for-each select="unparsed-text-lines($index)">
        <xsl:variable name="uri" select="."/>
        <case>
          <xsl:choose>
            <xsl:when test="doc-available($uri)">
              <xsl:attribute name="ok">true</xsl:attribute>
              <xsl:apply-templates select="doc($uri)/node()" mode="dump"/>
            </xsl:when>
            <xsl:otherwise>
              <xsl:attribute name="ok">false</xsl:attribute>
            </xsl:otherwise>
          </xsl:choose>
        </case>
      </xsl:for-each>
    </cases>
  </xsl:template>
  <xsl:template match="*" mode="dump">
    <xsl:variable name="e" select="."/>
    <s ns="{namespace-uri()}" local="{local-name()}" prefix="{prefix-from-QName(node-name())}">
      <xsl:for-each select="@*">
        <a ns="{namespace-uri()}" local="{local-name()}" prefix="{prefix-from-QName(node-name())}" v="{.}"/>
      </xsl:for-each>
      <xsl:for-each select="in-scope-prefixes($e)[. ne 'xml']">
        <n p="{.}" u="{namespace-uri-for-prefix(., $e)}"/>
      </xsl:for-each>
      <xsl:apply-templates select="node()" mode="dump"/>
    </s>
  </xsl:template>
  <xsl:template match="text()" mode="dump"><t v="{.}"/></xsl:template>
  <xsl:template match="comment()" mode="dump"><c v="{.}"/></xsl:template>
  <xsl:template match="processing-instruction()" mode="dump"><p t="{name()}" v="{.}"/></xsl:template>
</xsl:stylesheet>
"#;

/// The lines of Saxon's data model of a case, in the format of the tests' streams.
fn saxon_lines(case: roxmltree::Node<'_, '_>, lines: &mut Vec<String>) {
    for node in case.children().filter(roxmltree::Node::is_element) {
        let attr = |name: &str| node.attribute(name).unwrap_or("").to_owned();
        match node.tag_name().name() {
            "s" => {
                let mut attributes = Vec::new();
                let mut scope = Vec::new();
                for child in node.children().filter(roxmltree::Node::is_element) {
                    let a = |name: &str| child.attribute(name).unwrap_or("").to_owned();
                    match child.tag_name().name() {
                        "a" => attributes.push((a("ns"), a("prefix"), a("local"), a("v"))),
                        "n" => scope.push((a("p"), a("u"))),
                        _ => {}
                    }
                }
                scope.sort();
                let ns = attr("ns");
                lines.push(common::element_line(
                    Some(ns.as_str()).filter(|n| !n.is_empty()),
                    &attr("prefix"),
                    &attr("local"),
                    &attributes,
                    &scope,
                ));
                saxon_lines(node, lines);
                lines.push(format!("</{{{ns}}}{}>", attr("local")));
            }
            "t" => lines.push(format!("T{:?}", attr("v"))),
            "c" => lines.push(format!("C{:?}", attr("v"))),
            "p" => {
                let v = attr("v");
                lines.push(format!(
                    "P{} {:?}",
                    attr("t"),
                    (!v.is_empty()).then_some(v.as_str())
                ));
            }
            _ => {}
        }
    }
}

/// The reader's lines, as Saxon shows a document: no undeclared default namespace in
/// scope.
fn reader_lines(lines: Vec<String>) -> Vec<String> {
    lines
        .into_iter()
        .map(|line| line.replace(" xmlns[]=\"\"", ""))
        .collect()
}

fn run() -> Result<(), String> {
    let mut cases: Vec<(&str, &str)> = Vec::new();
    cases.extend(MALFORMED.iter().copied());
    cases.extend(WELL_FORMED.iter().copied());
    cases.extend(STRICT.iter().map(|(name, text, _)| (*name, *text)));
    cases.extend(DATA.iter().map(|(name, text, _)| (*name, *text)));
    cases.extend(LINE_ENDS.iter().map(|(name, text, _)| (*name, *text)));

    let class_path = saxon_class_path()?;
    let work = workspace_root().join("target/oracle/einvoice-xml");
    std::fs::create_dir_all(&work).map_err(|e| format!("{}: {e}", work.display()))?;
    let work = std::path::absolute(&work).map_err(|e| e.to_string())?;
    let mut index = String::new();
    for (i, (_, text)) in cases.iter().enumerate() {
        let path = work.join(format!("case-{i:04}.xml"));
        std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
        let _ = writeln!(index, "file://{}", path.display());
    }
    let index_path = work.join("index.txt");
    std::fs::write(&index_path, index).map_err(|e| e.to_string())?;
    let stylesheet = work.join("dump.xsl");
    std::fs::write(&stylesheet, STYLESHEET).map_err(|e| e.to_string())?;
    let output = work.join("results.xml");
    let java = std::env::var_os("JAVA").unwrap_or_else(|| "java".into());
    let status = Command::new(&java)
        .arg("-cp")
        .arg(&class_path)
        .arg("net.sf.saxon.Transform")
        .arg(format!("-xsl:{}", stylesheet.display()))
        .arg("-it")
        .arg(format!("-o:{}", output.display()))
        .arg(format!("index=file://{}", index_path.display()))
        .output()
        .map_err(|e| format!("cannot run {}: {e}", java.display()))?;
    if !status.status.success() {
        return Err(format!(
            "Saxon failed ({}):\n{}",
            status.status,
            String::from_utf8_lossy(&status.stderr)
        ));
    }
    let results = std::fs::read_to_string(&output).map_err(|e| e.to_string())?;
    let doc = roxmltree::Document::parse(&results).map_err(|e| format!("Saxon output: {e}"))?;
    let saxon: Vec<_> = doc
        .root_element()
        .children()
        .filter(roxmltree::Node::is_element)
        .collect();
    if saxon.len() != cases.len() {
        return Err(format!("{} results for {} cases", saxon.len(), cases.len()));
    }

    let (mut agree, mut expected_seen, mut differences) = (0, Vec::new(), Vec::new());
    for ((name, text), result) in cases.iter().zip(saxon) {
        let saxon_ok = result.attribute("ok") == Some("true");
        let mut saxon = Vec::new();
        saxon_lines(result, &mut saxon);
        let agrees = match common::ours(text) {
            Outcome::Stream(lines) => saxon_ok && reader_lines(lines) == saxon,
            Outcome::Error(_) => !saxon_ok,
        };
        let expected = EXPECTED.iter().find(|(n, _)| n == name);
        match (agrees, expected) {
            (true, None) => agree += 1,
            (false, Some((_, why))) => expected_seen.push(format!("{name}: {why}")),
            (true, Some(_)) => {
                differences.push(format!("{name}: listed as a difference, but Saxon agrees"));
            }
            (false, None) => differences.push(format!(
                "{name}: Saxon {} {saxon:?}, reader {}",
                if saxon_ok { "accepts" } else { "refuses" },
                common::describe(&common::ours(text))
            )),
        }
    }
    println!(
        "Saxon-HE 12.10 on {} cases: {agree} agree, {} differ as listed:",
        cases.len(),
        expected_seen.len()
    );
    for line in &expected_seen {
        println!("  {line}");
    }
    if !differences.is_empty() {
        return Err(format!(
            "unexpected differences:\n{}",
            differences.join("\n")
        ));
    }
    Ok(())
}

fn saxon_class_path() -> Result<std::ffi::OsString, String> {
    let saxon = std::env::var_os("SAXON_JAR")
        .map_or_else(|| workspace_root().join(SAXON_DEFAULT), PathBuf::from);
    let dir = saxon.parent().unwrap_or_else(|| Path::new(""));
    let mut jars = Vec::with_capacity(SAXON_JARS.len());
    for (index, (name, sha256)) in SAXON_JARS.iter().enumerate() {
        let jar = if index == 0 {
            saxon.clone()
        } else {
            dir.join(name)
        };
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
        if hash != *sha256 {
            return Err(format!(
                "{}: SHA-256 {hash}, expected {sha256}",
                jar.display()
            ));
        }
        jars.push(jar);
    }
    std::env::join_paths(&jars).map_err(|e| format!("class path: {e}"))
}
