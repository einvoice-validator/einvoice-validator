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
//!
//! Which encoding names each reference accepts depends on its environment: libxml2 reads
//! a few encodings itself and looks any other name up in the converters it is built with
//! (iconv, then ICU), whose tables are the system's; the XML parser of Saxon-HE takes them
//! from the JDK. The table is recorded in one reference environment, the runner of the
//! `oracle` workflow, which the header of `expected.tsv` describes (`REFERENCE_XMLLINT` and
//! `REFERENCE_JAVA` of `tests/encoding/probes.rs`): libxml2 2.9.14 built with iconv and
//! ICU, on Ubuntu 24.04, and Java 21. The example describes the environment it runs in the
//! same way: the version of libxml2 and its converters (`xmllint --version`), the system
//! (`ID` and `VERSION_ID` of `/etc/os-release`) and the version of the Java specification
//! (`java.specification.version`). In the reference environment, every result must be
//! the recorded one. Elsewhere, the results of a reference whose environment differs are
//! not checked: those that differ from the table are listed as differences of the
//! environment, not as failures; the results of the other reference must still be the
//! recorded ones. In CI (`$CI` set), the environment must be the reference one, and
//! `--write` refuses to rewrite the table in any other.
//!
//! `scripts/encoding-oracle-refenv.sh` runs this example in the reference environment
//! on any machine with podman or docker: in a container built from pinned inputs
//! (Ubuntu 24.04 by digest, its libxml2, ICU and `OpenJDK` 21 packages by version), the
//! example built there; its arguments are passed on (`--write` to rewrite the table).

#[path = "../tests/encoding/probes.rs"]
mod probes;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use probes::{
    Outcome, REFERENCE_JAVA, REFERENCE_XMLLINT, compare, format_expected, parse_disagreements,
    parse_expected,
};
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
/// The script that runs this example in the reference environment, in a container.
const REFENV: &str = "scripts/encoding-oracle-refenv.sh";

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
    let java = std::env::var_os("JAVA").unwrap_or_else(|| "java".into());
    let xmllint = std::env::var_os("XMLLINT").unwrap_or_else(|| "xmllint".into());
    let environment = Environment::detect(&java, &xmllint)?;
    let xmllint_is_reference = environment.xmllint == REFERENCE_XMLLINT;
    let java_is_reference = environment.java == REFERENCE_JAVA;
    environment.print(xmllint_is_reference, java_is_reference);
    if !(xmllint_is_reference && java_is_reference) {
        let ci = std::env::var_os("CI").is_some_and(|v| !v.is_empty());
        if ci || write {
            return Err(format!(
                "{} is recorded in the reference environment (xmllint: {REFERENCE_XMLLINT}; \
                 Saxon-HE 12.10: {REFERENCE_JAVA}), {}; {REFENV} runs this example there, \
                 in a container, on any machine with podman or docker",
                probes::EXPECTED,
                if write {
                    "and only rewritten in it"
                } else {
                    "and checked in it in CI"
                }
            ));
        }
    }

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
    let saxon = saxon(&java, &work, &index, probes.len())?;
    let mut rows = Vec::with_capacity(probes.len());
    for (((id, _), path), saxon) in probes.iter().zip(&files).zip(saxon) {
        rows.push((id.clone(), saxon, xmllint_outcome(&xmllint, path)?));
    }

    let expected_path = crate_dir().join(probes::EXPECTED);
    // The reference results: those just obtained when the table is rewritten, else the
    // recorded ones, which those obtained must equal where the environment is the
    // reference one.
    let reference = if write {
        std::fs::write(&expected_path, format_expected(&rows)).map_err(|e| e.to_string())?;
        println!("wrote {}", expected_path.display());
        rows
    } else {
        let recorded = std::fs::read_to_string(&expected_path).map_err(|e| e.to_string())?;
        let recorded = parse_expected(&recorded)?;
        check(&rows, &recorded, xmllint_is_reference, java_is_reference)
            .map_err(|e| format!("{}: {e}", expected_path.display()))?;
        recorded
    };
    let agree = reference.iter().filter(|(_, s, x)| s == x).count();
    println!(
        "{} probes: Saxon-HE and xmllint agree on {agree} ({} read by both, {} refused by both)",
        reference.len(),
        reference
            .iter()
            .filter(|(_, s, x)| s == x && matches!(s, Outcome::Ok(_)))
            .count(),
        reference
            .iter()
            .filter(|(_, s, x)| s == x && *s == Outcome::Refused)
            .count(),
    );
    let disagreements = std::fs::read_to_string(crate_dir().join(probes::DISAGREEMENTS))
        .map_err(|e| e.to_string())?;
    let comparison = compare(&probes, &reference, &parse_disagreements(&disagreements)?);
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

/// Compares the results obtained (`rows`) with the recorded ones, probe by probe: those
/// of a reference whose environment is the reference one must be equal; those of the
/// other are listed where they differ, as differences of the environment.
fn check(
    rows: &[(String, Outcome, Outcome)],
    recorded: &[(String, Outcome, Outcome)],
    xmllint_is_reference: bool,
    java_is_reference: bool,
) -> Result<(), String> {
    let ids = |rows: &[(String, Outcome, Outcome)]| -> Vec<String> {
        rows.iter().map(|(id, _, _)| id.clone()).collect()
    };
    if ids(rows) != ids(recorded) {
        return Err(format!(
            "{} probes, {} recorded results, not for the same probes (run {REFENV} \
             --write: this example with --write in the reference environment)",
            rows.len(),
            recorded.len()
        ));
    }
    let mut failures = Vec::new();
    let mut environment = Vec::new();
    for ((id, saxon, xmllint), (_, recorded_saxon, recorded_xmllint)) in rows.iter().zip(recorded) {
        for (reference, name, here, recorded) in [
            (java_is_reference, "Saxon-HE", saxon, recorded_saxon),
            (xmllint_is_reference, "xmllint", xmllint, recorded_xmllint),
        ] {
            if here != recorded {
                let line = format!(
                    "{id}: {name} {} here, {} recorded",
                    here.to_field(),
                    recorded.to_field()
                );
                if reference {
                    failures.push(line);
                } else {
                    environment.push(line);
                }
            }
        }
    }
    if !environment.is_empty() {
        println!(
            "{} results differ from the recorded ones in this environment (differences of \
             the environment, not checked):",
            environment.len()
        );
        for line in &environment {
            println!("  {line}");
        }
    }
    if !failures.is_empty() {
        return Err(format!(
            "differs from the reference parsers (run {REFENV} --write: this example with \
             --write in the reference environment):\n{}",
            failures.join("\n")
        ));
    }
    let checked: Vec<&str> = [
        (java_is_reference, "Saxon-HE 12.10"),
        (xmllint_is_reference, "xmllint"),
    ]
    .iter()
    .filter_map(|&(reference, name)| reference.then_some(name))
    .collect();
    if checked.is_empty() {
        println!("results of neither reference checked (not the reference environment)");
    } else {
        println!("results of {} as recorded", checked.join(" and "));
    }
    Ok(())
}

/// The environment of each reference, described as `REFERENCE_XMLLINT` and
/// `REFERENCE_JAVA` describe the reference one, with details for the log.
struct Environment {
    /// The version of libxml2, its converters and the system.
    xmllint: String,
    /// The version of the Java specification.
    java: String,
    /// The vendor and version of the Java runtime.
    java_runtime: String,
}

impl Environment {
    fn detect(java: &std::ffi::OsStr, xmllint: &std::ffi::OsStr) -> Result<Self, String> {
        let output = Command::new(xmllint)
            .arg("--version")
            .output()
            .map_err(|e| format!("cannot run {}: {e}", xmllint.display()))?;
        let text = String::from_utf8_lossy(&output.stderr).into_owned()
            + &String::from_utf8_lossy(&output.stdout);
        let version = text
            .lines()
            .find_map(|l| l.split_once("using libxml version ").map(|(_, v)| v.trim()))
            .ok_or_else(|| format!("{} --version: no libxml version", xmllint.display()))?;
        let features: Vec<&str> = text
            .lines()
            .find_map(|l| l.split_once("compiled with:").map(|(_, f)| f))
            .ok_or_else(|| format!("{} --version: no features", xmllint.display()))?
            .split_whitespace()
            .collect();
        let converters: Vec<&str> = ["Iconv", "ICU", "ISO8859X"]
            .into_iter()
            .filter(|c| features.contains(c))
            .collect();
        let converters = if converters.is_empty() {
            "no converter".to_owned()
        } else {
            converters.join(" ")
        };
        let xmllint = format!("libxml {version} with {converters}, on {}", system());

        let output = Command::new(java)
            .args(["-XshowSettings:properties", "-version"])
            .output()
            .map_err(|e| format!("cannot run {}: {e}", java.display()))?;
        let text = String::from_utf8_lossy(&output.stderr).into_owned()
            + &String::from_utf8_lossy(&output.stdout);
        let property = |name: &str| {
            text.lines().find_map(|l| {
                l.trim()
                    .strip_prefix(name)
                    .and_then(|rest| rest.trim_start().strip_prefix('='))
                    .map(|v| v.trim().to_owned())
            })
        };
        let specification = property("java.specification.version")
            .ok_or_else(|| format!("{}: no java.specification.version", java.display()))?;
        let java_runtime = format!(
            "{} {}",
            property("java.vendor").unwrap_or_default(),
            property("java.runtime.version").unwrap_or_default()
        );
        Ok(Self {
            xmllint,
            java: format!("Java {specification}"),
            java_runtime,
        })
    }

    fn print(&self, xmllint_is_reference: bool, java_is_reference: bool) {
        let reference = |is: bool, which: &str| {
            if is {
                " (the reference)".to_owned()
            } else {
                format!(" (not the reference: {which})")
            }
        };
        println!(
            "xmllint: {}{}",
            self.xmllint,
            reference(xmllint_is_reference, REFERENCE_XMLLINT)
        );
        println!(
            "Saxon-HE 12.10: {} ({}){}",
            self.java,
            self.java_runtime.trim(),
            reference(java_is_reference, REFERENCE_JAVA)
        );
    }
}

/// The system: `ID` and `VERSION_ID` of `/etc/os-release`, else the name of the OS.
fn system() -> String {
    let release = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    let field = |name: &str| {
        release.lines().find_map(|l| {
            l.strip_prefix(name)
                .and_then(|v| v.strip_prefix('='))
                .map(|v| v.trim().trim_matches('"').to_owned())
        })
    };
    match (field("ID"), field("VERSION_ID")) {
        (Some(id), Some(version)) => format!("{id} {version}"),
        (Some(id), None) => id,
        _ => std::env::consts::OS.to_owned(),
    }
}

/// Saxon-HE's outcome for each document of the index, in order.
fn saxon(
    java: &std::ffi::OsStr,
    work: &Path,
    index: &str,
    count: usize,
) -> Result<Vec<Outcome>, String> {
    let class_path = saxon_class_path()?;
    let index_path = work.join("index.txt");
    std::fs::write(&index_path, index).map_err(|e| e.to_string())?;
    let stylesheet = work.join("read.xsl");
    std::fs::write(&stylesheet, STYLESHEET).map_err(|e| e.to_string())?;
    let output = work.join("saxon.xml");
    let status = Command::new(java)
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
