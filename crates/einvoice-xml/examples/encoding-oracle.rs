//! What Saxon-HE 12.10 and xmllint (libxml2) make of every document of
//! `tests/encoding/probes.rs`, documents in several encodings: each reads it to the text
//! of its string value (`string(/)`), or refuses it. The results must be those of
//! `tests/encoding/expected.tsv` (rewritten with `--write`), and the decoder of this
//! crate must compare with them as `tests/encoding/disagreements.tsv` lists (the test
//! `tests/encoding.rs` checks the same against the file, without Java or xmllint).
//!
//! ```text
//! cargo run -p einvoice-xml --example encoding-oracle             # check
//! cargo run -p einvoice-xml --example encoding-oracle -- --write  # rewrite expected.tsv
//! ```
//!
//! Saxon-HE 12.10 is taken from `$SAXON_JAR`, else where `scripts/fetch-saxon.sh` puts it
//! (`target/oracle/saxon/`), with the two XML Resolver jars it needs from the same
//! directory; the SHA-256 of each jar is checked. Java from `$JAVA`, else `java`;
//! xmllint from `$XMLLINT`, else `xmllint`. Saxon-HE reads each document with `doc()`,
//! that is with the XML parser it reads source documents with (the JDK's).

#[path = "../tests/encoding/probes.rs"]
mod probes;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use probes::{Outcome, compare, format_expected, parse_disagreements, parse_expected};
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

/// For each URI of the index, `<case ok="true" cp="code points"/>` or `<case ok="false"/>`.
const STYLESHEET: &str = r#"<xsl:stylesheet version="3.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform"
    xmlns:xs="http://www.w3.org/2001/XMLSchema" exclude-result-prefixes="xs">
  <xsl:output method="xml" indent="no"/>
  <xsl:param name="index" as="xs:string"/>
  <xsl:template name="xsl:initial-template">
    <cases>
      <xsl:for-each select="unparsed-text-lines($index)">
        <case>
          <xsl:choose>
            <xsl:when test="doc-available(.)">
              <xsl:attribute name="ok">true</xsl:attribute>
              <xsl:attribute name="cp" select="string-to-codepoints(string(doc(.)))"/>
            </xsl:when>
            <xsl:otherwise>
              <xsl:attribute name="ok">false</xsl:attribute>
            </xsl:otherwise>
          </xsl:choose>
        </case>
      </xsl:for-each>
    </cases>
  </xsl:template>
</xsl:stylesheet>
"#;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn crate_dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn run() -> Result<(), String> {
    let write = std::env::args().skip(1).any(|a| a == "--write");
    let probes = probes::probes();
    let work = crate_dir().join("../../target/oracle/einvoice-xml-encoding");
    if work.exists() {
        std::fs::remove_dir_all(&work).map_err(|e| format!("{}: {e}", work.display()))?;
    }
    std::fs::create_dir_all(&work).map_err(|e| format!("{}: {e}", work.display()))?;
    let work = std::path::absolute(&work).map_err(|e| e.to_string())?;
    let mut files = Vec::with_capacity(probes.len());
    let mut index = String::new();
    for (i, (_, bytes)) in probes.iter().enumerate() {
        let path = work.join(format!("probe-{i:04}.xml"));
        std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        let _ = writeln!(index, "file://{}", path.display());
        files.push(path);
    }
    let saxon = saxon(&work, &index, probes.len())?;
    let xmllint = std::env::var_os("XMLLINT").unwrap_or_else(|| "xmllint".into());
    let mut rows = Vec::with_capacity(probes.len());
    for (((id, _), path), saxon) in probes.iter().zip(&files).zip(saxon) {
        rows.push((id.clone(), saxon, xmllint_outcome(&xmllint, path)?));
    }

    let expected_path = crate_dir().join(probes::EXPECTED);
    let written = format_expected(&rows);
    if write {
        std::fs::write(&expected_path, &written).map_err(|e| e.to_string())?;
        println!("wrote {}", expected_path.display());
    } else {
        let current = std::fs::read_to_string(&expected_path).map_err(|e| e.to_string())?;
        let current = parse_expected(&current)?;
        let mut stale = Vec::new();
        for row in &rows {
            if !current.contains(row) {
                stale.push(format!(
                    "{}: Saxon-HE {}, xmllint {}",
                    row.0,
                    row.1.to_field(),
                    row.2.to_field()
                ));
            }
        }
        if !stale.is_empty() || current.len() != rows.len() {
            return Err(format!(
                "{} differs from the reference parsers (run with --write):\n{}",
                expected_path.display(),
                stale.join("\n")
            ));
        }
        println!("{}: as Saxon-HE 12.10 and xmllint", expected_path.display());
    }
    let agree = rows.iter().filter(|(_, s, x)| s == x).count();
    println!(
        "{} probes: Saxon-HE and xmllint agree on {agree} ({} read by both, {} refused by both)",
        rows.len(),
        rows.iter()
            .filter(|(_, s, x)| s == x && matches!(s, Outcome::Ok(_)))
            .count(),
        rows.iter()
            .filter(|(_, s, x)| s == x && *s == Outcome::Refused)
            .count(),
    );
    let disagreements = std::fs::read_to_string(crate_dir().join(probes::DISAGREEMENTS))
        .map_err(|e| e.to_string())?;
    let comparison = compare(&probes, &rows, &parse_disagreements(&disagreements)?);
    println!(
        "this crate: {} read and {} refused as both, {} listed:",
        comparison.read,
        comparison.refused,
        comparison.listed.len()
    );
    for line in &comparison.listed {
        println!("  {line}");
    }
    if !comparison.failures.is_empty() {
        return Err(comparison.failures.join("\n"));
    }
    Ok(())
}

/// Saxon-HE's outcome for each document of the index, in order.
fn saxon(work: &Path, index: &str, count: usize) -> Result<Vec<Outcome>, String> {
    let class_path = saxon_class_path()?;
    let index_path = work.join("index.txt");
    std::fs::write(&index_path, index).map_err(|e| e.to_string())?;
    let stylesheet = work.join("read.xsl");
    std::fs::write(&stylesheet, STYLESHEET).map_err(|e| e.to_string())?;
    let output = work.join("saxon.xml");
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
    let outcomes: Vec<Outcome> = doc
        .root_element()
        .children()
        .filter(roxmltree::Node::is_element)
        .map(|case| {
            if case.attribute("ok") != Some("true") {
                return Ok(Outcome::Refused);
            }
            case.attribute("cp")
                .unwrap_or("")
                .split_whitespace()
                .map(|n| {
                    n.parse::<u32>()
                        .ok()
                        .and_then(char::from_u32)
                        .ok_or_else(|| format!("Saxon output: code point {n}"))
                })
                .collect::<Result<String, _>>()
                .map(Outcome::Ok)
        })
        .collect::<Result<_, _>>()?;
    if outcomes.len() != count {
        return Err(format!(
            "{} Saxon results for {count} documents",
            outcomes.len()
        ));
    }
    Ok(outcomes)
}

/// xmllint's outcome for the document at `path`: `string(/)`, which it prints followed by
/// a line feed, or a refusal (an error message and a failure status).
fn xmllint_outcome(xmllint: &std::ffi::OsStr, path: &Path) -> Result<Outcome, String> {
    let output = Command::new(xmllint)
        .arg("--xpath")
        .arg("string(/)")
        .arg(path)
        .output()
        .map_err(|e| format!("cannot run {}: {e}", xmllint.display()))?;
    if !output.status.success() {
        if output.stderr.is_empty() {
            return Err(format!(
                "{}: xmllint failed ({}) without a message",
                path.display(),
                output.status
            ));
        }
        return Ok(Outcome::Refused);
    }
    let text = String::from_utf8(output.stdout)
        .map_err(|e| format!("{}: xmllint output: {e}", path.display()))?;
    let text = text
        .strip_suffix('\n')
        .ok_or_else(|| format!("{}: xmllint output without a line feed", path.display()))?;
    Ok(Outcome::Ok(text.to_owned()))
}

fn saxon_class_path() -> Result<std::ffi::OsString, String> {
    let root = crate_dir().join("../..");
    let saxon =
        std::env::var_os("SAXON_JAR").map_or_else(|| root.join(SAXON_DEFAULT), PathBuf::from);
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
