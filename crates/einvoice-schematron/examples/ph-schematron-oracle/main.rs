//! The oracle of this crate: ph-schematron 10.3.1 itself, run on the JVM. It writes the
//! goldens of the tests and checks them.
//!
//! ```text
//! cargo run -p einvoice-schematron --example ph-schematron-oracle             # check the goldens
//! cargo run -p einvoice-schematron --example ph-schematron-oracle -- --write  # rewrite them
//! cargo run -p einvoice-schematron --example ph-schematron-oracle -- --peppol # the Peppol inputs
//! ```
//!
//! What it runs:
//!
//! 1. `ph-schematron-maven-plugin:convert` (engine iso-schematron, the version of the pin
//!    `plugin` of `scripts/fetch-peppol.sh`) on the Schematron files, with Maven in debug
//!    mode: the compiled stylesheets, as the plugin writes them, and the class path of
//!    the plugin (its class realm, read from Maven's log). Maven and its local repository
//!    are those `scripts/fetch-peppol.sh` installs in `target/peppol/` (`tools/` and
//!    `m2/`; `$MAVEN` for another `mvn`); the user's Maven settings are not used.
//! 2. `PhSchematronOracle.java` (next to this file), in Java's source-file mode on that
//!    class path (Java from `$JAVA`, else `java`): the result of each step, the DOM
//!    document of the last one and the stylesheet written from it, which must be the
//!    plugin's, byte for byte; the character tables of ph-commons and of Xerces; the
//!    outcome of the DOM operations of `tests/data/xerces-cases.txt`; and a copy of the
//!    stylesheets of the ISO Schematron skeleton ph-schematron runs, from its class
//!    path, into `target/schematron-oracle/skeleton/` (for the `skeleton-probe`
//!    example).
//!
//! Without option, on the synthetic Schematron files of `tests/data/sch/` and the result
//! trees of `tests/data/write/` (`NAME.xml`, or the result of `NAME.xsl` applied to
//! itself): everything written is compared with `tests/data/expected/`,
//! `tests/data/written/`, `tests/data/xml10-chars.txt` and `tests/data/xerces-expected.txt`;
//! `--write` writes them instead. Then 400 random documents (a fixed seed), written into
//! `target/schematron-oracle/random/`: the DOM document and the stylesheet this crate
//! writes from each must be ph-schematron's. With `--peppol`, on
//! `rules/sch/PEPPOL-EN16931-UBL.sch` and `rules/sch/CEN-EN16931-UBL.sch` of the Peppol
//! release that `scripts/fetch-peppol.sh` extracted: the stylesheets must have the SHA-256
//! pinned in that script, and the results go into
//! `target/peppol/peppol-bis-billing-ubl-<tag>/schematron/`, outside the tracked tree (the
//! Peppol files may not be redistributed), where the test `peppol` reads them.

use std::ffi::{OsStr, OsString};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use sha2::{Digest, Sha256};

/// The plugin whose class realm is the class path of the Java side.
const PLUGIN: &str = "com.helger.maven:ph-schematron-maven-plugin";

/// The two stylesheets of Peppol validation, with the name of the pin of their SHA-256.
const PEPPOL: [(&str, &str); 2] = [
    ("PEPPOL-EN16931-UBL", "peppol_xslt_sha256"),
    ("CEN-EN16931-UBL", "cen_xslt_sha256"),
];

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("ph-schematron-oracle: {e}");
            ExitCode::FAILURE
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Check,
    Write,
    Peppol,
}

/// Where everything is.
struct Setup {
    crate_dir: PathBuf,
    script: String,
    m2: PathBuf,
    maven_home: PathBuf,
    mvn: PathBuf,
    release: PathBuf,
    work: PathBuf,
}

impl Setup {
    fn new() -> Result<Self, String> {
        let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let root = crate_dir
            .ancestors()
            .nth(2)
            .ok_or("no workspace root")?
            .to_path_buf();
        let script = read(&root.join("scripts").join("fetch-peppol.sh"))?;
        let cache = root.join("target").join("peppol");
        let maven_home = cache
            .join("tools")
            .join(format!("apache-maven-{}", pin(&script, "maven")?));
        let mvn = std::env::var_os("MAVEN")
            .map_or_else(|| maven_home.join("bin").join("mvn"), PathBuf::from);
        if !mvn.is_file() {
            return Err(format!(
                "no Maven at {}: run scripts/fetch-peppol.sh first (it installs Maven and \
                 the plugin), or set MAVEN",
                mvn.display()
            ));
        }
        let release = cache.join(format!("peppol-bis-billing-ubl-{}", pin(&script, "tag")?));
        Ok(Self {
            crate_dir,
            m2: cache.join("m2"),
            maven_home,
            mvn,
            release,
            work: root.join("target").join("schematron-oracle"),
            script,
        })
    }

    fn pin(&self, name: &str) -> Result<String, String> {
        pin(&self.script, name)
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        [] => Mode::Check,
        ["--write"] => Mode::Write,
        ["--peppol"] => Mode::Peppol,
        _ => return Err("usage: ph-schematron-oracle [--write | --peppol]".to_owned()),
    };
    let setup = Setup::new()?;
    if setup.work.exists() {
        std::fs::remove_dir_all(&setup.work)
            .map_err(|e| format!("{}: {e}", setup.work.display()))?;
    }
    let input = inputs(&setup, mode)?;
    let mut names: Vec<String> = std::fs::read_dir(&input)
        .map_err(|e| format!("{}: {e}", input.display()))?
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_suffix(".sch").map(str::to_owned))
        .collect();
    names.sort();
    let plugin_out = setup.work.join("plugin");
    let oracle_out = setup.work.join("oracle");
    create_dir(&plugin_out)?;
    create_dir(&oracle_out)?;

    let java = Java {
        classpath: run_plugin(&setup, &input, &plugin_out, names.len())?,
        program: std::env::var_os("JAVA").unwrap_or_else(|| "java".into()),
        source: setup
            .crate_dir
            .join("examples")
            .join("ph-schematron-oracle")
            .join("PhSchematronOracle.java"),
    };
    let mut compile: Vec<OsString> = vec!["compile".into(), oracle_out.clone().into()];
    compile.extend(
        names
            .iter()
            .map(|n| input.join(format!("{n}.sch")).into_os_string()),
    );
    java.run(&compile)?;
    for name in &names {
        let ours = read_bytes(&oracle_out.join(format!("{name}.xslt")))?;
        let plugin = read_bytes(&plugin_out.join(format!("{name}.xslt")))?;
        if ours != plugin {
            return Err(format!(
                "{name}: the stylesheet PhSchematronOracle writes is not the plugin's"
            ));
        }
    }
    eprintln!(
        "the {} stylesheets are the plugin's, byte for byte",
        names.len()
    );
    let skeleton = setup.work.join("skeleton");
    create_dir(&skeleton)?;
    java.run(&["skeleton".into(), skeleton.clone().into()])?;
    eprintln!(
        "the ISO Schematron skeleton of ph-schematron copied into {}",
        skeleton.display()
    );
    if mode == Mode::Peppol {
        peppol(&setup, &plugin_out, &oracle_out)
    } else {
        fixtures(&setup, mode, &java, &oracle_out, &names)
    }
}

/// The directory of the Schematron files: the fixtures, or a copy of the two Peppol files.
fn inputs(setup: &Setup, mode: Mode) -> Result<PathBuf, String> {
    if mode != Mode::Peppol {
        return Ok(setup.crate_dir.join("tests").join("data").join("sch"));
    }
    let input = setup.work.join("in");
    create_dir(&input)?;
    for (name, _) in PEPPOL {
        let sch = setup
            .release
            .join("source")
            .join("rules")
            .join("sch")
            .join(format!("{name}.sch"));
        if !sch.is_file() {
            return Err(format!(
                "no {}: run scripts/fetch-peppol.sh first",
                sch.display()
            ));
        }
        std::fs::copy(&sch, input.join(format!("{name}.sch")))
            .map_err(|e| format!("{}: {e}", sch.display()))?;
    }
    Ok(input)
}

/// Runs the plugin on `input` into `output`; returns the class path of the plugin.
fn run_plugin(
    setup: &Setup,
    input: &Path,
    output: &Path,
    count: usize,
) -> Result<OsString, String> {
    let version = setup.pin("plugin")?;
    let settings = setup.work.join("settings.xml");
    write(
        &settings,
        &format!(
            "<settings><localRepository>{}</localRepository></settings>\n",
            xml_text(&setup.m2)
        ),
    )?;
    let pom = setup.work.join("pom.xml");
    write(&pom, &pom_xml(&version, input, output))?;
    eprintln!("running ph-schematron-maven-plugin {version} on {count} files");
    let run = Command::new(&setup.mvn)
        .env("MAVEN_SKIP_RC", "1")
        .args(["-X", "--batch-mode", "--strict-checksums", "--settings"])
        .arg(&settings)
        .arg("--file")
        .arg(&pom)
        .arg("validate")
        .output()
        .map_err(|e| format!("{}: {e}", setup.mvn.display()))?;
    let log = String::from_utf8_lossy(&run.stdout).into_owned();
    if !run.status.success() {
        let lines: Vec<&str> = log.lines().collect();
        let tail = lines[lines.len().saturating_sub(40)..].join("\n");
        return Err(format!("Maven failed ({}):\n{tail}", run.status));
    }
    let mut classpath = realm(&log, &version, &setup.m2)?;
    let lib = setup.maven_home.join("lib");
    let slf4j = std::fs::read_dir(&lib)
        .map_err(|e| format!("{}: {e}", lib.display()))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| {
            p.extension().is_some_and(|x| x.eq_ignore_ascii_case("jar"))
                && p.file_name()
                    .and_then(OsStr::to_str)
                    .is_some_and(|n| n.starts_with("slf4j-api-"))
        })
        .ok_or_else(|| format!("no slf4j-api jar in {}", lib.display()))?;
    classpath.push(slf4j);
    std::env::join_paths(&classpath).map_err(|e| e.to_string())
}

/// The Java side.
struct Java {
    classpath: OsString,
    program: OsString,
    source: PathBuf,
}

impl Java {
    fn run(&self, args: &[OsString]) -> Result<(), String> {
        let output = Command::new(&self.program)
            .arg("-cp")
            .arg(&self.classpath)
            .arg(&self.source)
            .args(args)
            .output()
            .map_err(|e| format!("{}: {e}", Path::new(&self.program).display()))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(format!(
                "PhSchematronOracle failed ({}):\n{}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            ))
        }
    }
}

/// `--peppol`: the pins checked, the results copied into the cache.
fn peppol(setup: &Setup, plugin_out: &Path, oracle_out: &Path) -> Result<(), String> {
    for (name, pin_name) in PEPPOL {
        let hash = sha256(&read_bytes(&plugin_out.join(format!("{name}.xslt")))?);
        let pinned = setup.pin(pin_name)?;
        if hash != pinned {
            return Err(format!("{name}.xslt: SHA-256 {hash}, pinned {pinned}"));
        }
    }
    let target = setup.release.join("schematron");
    create_dir(&target)?;
    for entry in std::fs::read_dir(oracle_out).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        let to = target.join(path.file_name().ok_or("no file name")?);
        std::fs::copy(&path, &to).map_err(|e| format!("{}: {e}", to.display()))?;
    }
    eprintln!("SHA-256 as pinned; written into {}", target.display());
    Ok(())
}

/// The fixtures: the goldens checked or written, then the random documents compared.
fn fixtures(
    setup: &Setup,
    mode: Mode,
    java: &Java,
    oracle_out: &Path,
    names: &[String],
) -> Result<(), String> {
    let data = setup.crate_dir.join("tests").join("data");
    let chars = oracle_out.join("xml10-chars.txt");
    java.run(&["chars".into(), chars.clone().into()])?;
    let xerces = oracle_out.join("xerces-expected.txt");
    java.run(&[
        "xerces".into(),
        data.join("xerces-cases.txt").into(),
        xerces.clone().into(),
    ])?;
    // The writer's fixtures: every file the oracle writes for them has a golden.
    let written_out = oracle_out.join("written");
    create_dir(&written_out)?;
    let mut write: Vec<OsString> = vec!["write".into(), written_out.clone().into()];
    write.extend(
        sorted_files(&data.join("write"))?
            .into_iter()
            .map(PathBuf::into_os_string),
    );
    java.run(&write)?;

    // Every file the oracle wrote, and where its golden is.
    let expected = data.join("expected");
    let written = data.join("written");
    let mut files: Vec<(PathBuf, PathBuf)> = vec![
        (chars, data.join("xml10-chars.txt")),
        (xerces, data.join("xerces-expected.txt")),
    ];
    for name in names {
        for extension in ["step1.xml", "step2.xml", "step3.xml", "dom", "xslt"] {
            let file = format!("{name}.{extension}");
            files.push((oracle_out.join(&file), expected.join(&file)));
        }
    }
    for from in sorted_files(&written_out)? {
        let to = written.join(from.file_name().ok_or("no file name")?);
        files.push((from, to));
    }
    if mode == Mode::Write {
        for dir in [&expected, &written] {
            if dir.exists() {
                std::fs::remove_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            }
            create_dir(dir)?;
        }
        for (from, to) in &files {
            std::fs::copy(from, to).map_err(|e| format!("{}: {e}", to.display()))?;
        }
        eprintln!("{} goldens written", files.len());
    } else {
        let mut different = Vec::new();
        for (from, to) in &files {
            if read_bytes(from)? != std::fs::read(to).unwrap_or_default() {
                different.push(to.display().to_string());
            }
        }
        let mut stale = Vec::new();
        for dir in [&expected, &written] {
            for path in sorted_files(dir)? {
                if !files.iter().any(|(_, to)| *to == path) {
                    stale.push(path.display().to_string());
                }
            }
        }
        if !different.is_empty() || !stale.is_empty() {
            return Err(format!(
                "goldens that differ from ph-schematron's: {}; goldens of no fixture: {}; \
                 rewrite them with --write",
                different.join(", "),
                stale.join(", ")
            ));
        }
        eprintln!("{} goldens are ph-schematron's", files.len());
    }
    random(setup, java)
}

/// The number of random documents compared with ph-commons' writer.
const RANDOM: usize = 400;

/// Random documents (a fixed seed): the DOM and the stylesheet this crate writes from
/// each are those of Saxon's `DOMWriter` and ph-commons' writer.
fn random(setup: &Setup, java: &Java) -> Result<(), String> {
    let input = setup.work.join("random");
    let output = setup.work.join("random-out");
    create_dir(&input)?;
    create_dir(&output)?;
    let mut rng = Rng(0x5EED_0F5C_4E3A_7001);
    let mut args: Vec<OsString> = vec!["write".into(), output.clone().into()];
    let mut documents = Vec::new();
    for n in 0..RANDOM {
        let xml = random_document(&mut rng);
        let path = input.join(format!("r{n:03}.xml"));
        write(&path, &xml)?;
        args.push(path.into_os_string());
        documents.push((format!("r{n:03}"), xml));
    }
    java.run(&args)?;
    let mut failures = Vec::new();
    for (name, xml) in &documents {
        let dom = read(&output.join(format!("{name}.dom")))?;
        let xslt = read(&output.join(format!("{name}.xslt")))?;
        match einvoice_schematron::Document::from_result(xml) {
            Ok(document) => {
                if dom_text(&document) != dom {
                    failures.push(format!("{name}: DOM"));
                } else if einvoice_schematron::write_stylesheet(&document).as_deref() != Ok(&xslt) {
                    failures.push(format!("{name}: stylesheet"));
                }
            }
            Err(e) => failures.push(format!("{name}: {e}")),
        }
    }
    if !failures.is_empty() {
        return Err(format!(
            "random documents (in {}) where this crate differs from ph-schematron: {}",
            input.display(),
            failures.join(", ")
        ));
    }
    eprintln!("{RANDOM} random documents: DOM and stylesheet are ph-schematron's");
    Ok(())
}

/// xorshift64*.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A number below `n`.
    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % u64::try_from(n).unwrap_or(1)).unwrap_or(0)
    }

    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len())]
    }
}

/// Prefixes and their namespaces: each prefix bound to one namespace in a document,
/// `q` and `ns0` to namespaces that other prefixes have too.
const BINDINGS: [(&str, &str); 6] = [
    ("p0", "urn:0"),
    ("p1", "urn:1"),
    ("p2", "urn:2"),
    ("q", "urn:1"),
    ("ns0", "urn:0"),
    ("xsl", "http://www.w3.org/1999/XSL/Transform"),
];

/// A random document: comments and processing instructions around a document element
/// declaring some of the bindings, elements under prefixes, in the default namespace and
/// in none, attributes, text with the characters the writer escapes.
fn random_document(rng: &mut Rng) -> String {
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>");
    for _ in 0..rng.below(3) {
        random_misc(rng, &mut out);
    }
    let mut declared = String::new();
    for (prefix, uri) in BINDINGS {
        if rng.below(2) == 0 {
            let _ = write!(declared, " xmlns:{prefix}=\"{uri}\"");
        }
    }
    random_element(rng, &mut out, &declared, false, 0);
    for _ in 0..rng.below(3) {
        random_misc(rng, &mut out);
    }
    out
}

fn random_misc(rng: &mut Rng, out: &mut String) {
    if rng.below(2) == 0 {
        let _ = write!(out, "<!--{}-->", rng.pick(&["", "c", " a < b & c ", "é😀"]));
    } else {
        let _ = write!(out, "<?{}?>", rng.pick(&["pi", "pi data", "t d d", "x"]));
    }
}

fn random_text(rng: &mut Rng) -> String {
    let pieces = [
        "a", "b", " ", "  ", "\t", "\n", "&#13;", "&amp;", "&lt;", "&gt;", "\"", "'", "é", "😀",
        "\u{85}", "\u{7F}", "\u{A0}", "]]&gt;",
    ];
    let mut text = String::new();
    for _ in 0..rng.below(5) {
        text.push_str(rng.pick(&pieces));
    }
    text
}

fn random_attribute_value(rng: &mut Rng) -> String {
    let pieces = [
        "a", " ", "&#9;", "&#10;", "&#13;", "&amp;", "&lt;", ">", "&quot;", "'", "é", "😀",
    ];
    let mut text = String::new();
    for _ in 0..rng.below(4) {
        text.push_str(rng.pick(&pieces));
    }
    text
}

/// An element: `declared` are declarations it must carry (the document element's),
/// `default` whether a default namespace is in scope.
fn random_element(rng: &mut Rng, out: &mut String, declared: &str, default: bool, depth: usize) {
    let local = rng.pick(&["a", "b", "item", "x-y", "_z"]);
    let mut declarations = declared.to_owned();
    let mut inner_default = default;
    let name = match rng.below(5) {
        0..=2 => {
            let (prefix, uri) = BINDINGS[rng.below(BINDINGS.len())];
            if !declarations.contains(&format!(" xmlns:{prefix}=")) {
                let _ = write!(declarations, " xmlns:{prefix}=\"{uri}\"");
            }
            format!("{prefix}:{local}")
        }
        3 => {
            declarations.push_str(" xmlns=\"urn:d\"");
            inner_default = true;
            local.to_owned()
        }
        _ => {
            if default {
                declarations.push_str(" xmlns=\"\"");
            }
            inner_default = false;
            local.to_owned()
        }
    };
    let mut attributes = String::new();
    let mut used: Vec<String> = Vec::new();
    for _ in 0..rng.below(4) {
        let local = rng.pick(&["k", "v", "w"]);
        let qname = if rng.below(2) == 0 {
            let (prefix, uri) = BINDINGS[rng.below(3)];
            if !declarations.contains(&format!(" xmlns:{prefix}=")) {
                let _ = write!(declarations, " xmlns:{prefix}=\"{uri}\"");
            }
            format!("{prefix}:{local}")
        } else {
            local.to_owned()
        };
        // An attribute once; p1:k and q:k would be the same attribute (urn:1).
        let key = qname.replace("q:", "p1:").replace("ns0:", "p0:");
        if !used.contains(&key) {
            used.push(key);
            let _ = write!(attributes, " {qname}=\"{}\"", random_attribute_value(rng));
        }
    }
    let _ = write!(out, "<{name}{declarations}{attributes}>");
    if depth < 4 {
        for _ in 0..rng.below(5) {
            match rng.below(6) {
                0 | 1 => random_element(rng, out, "", inner_default, depth + 1),
                2 | 3 => out.push_str(&random_text(rng)),
                _ => random_misc(rng, out),
            }
        }
    }
    let _ = write!(out, "</{name}>");
}

/// `document` in the `.dom` format of `PhSchematronOracle.java`.
fn dom_text(document: &einvoice_schematron::Document) -> String {
    let mut out = String::from("dom 1\n");
    for node in &document.children {
        dump(node, 0, &mut out);
    }
    out
}

fn dump(node: &einvoice_schematron::Node, depth: usize, out: &mut String) {
    use einvoice_schematron::Node;
    match node {
        Node::Element(element) => {
            let _ = writeln!(
                out,
                "E\t{depth}\t{}\t{}",
                escape(element.namespace().unwrap_or("")),
                escape(element.name())
            );
            for a in element.attributes() {
                let _ = writeln!(
                    out,
                    "A\t{depth}\t{}\t{}\t{}",
                    escape(a.namespace.as_deref().unwrap_or("")),
                    escape(&a.name),
                    escape(&a.value)
                );
            }
            for child in &element.children {
                dump(child, depth + 1, out);
            }
        }
        Node::Text(data) => {
            let _ = writeln!(out, "T\t{depth}\t{}", escape(data));
        }
        Node::CData(data) => {
            let _ = writeln!(out, "D\t{depth}\t{}", escape(data));
        }
        Node::Comment(data) => {
            let _ = writeln!(out, "C\t{depth}\t{}", escape(data));
        }
        Node::ProcessingInstruction { target, data } => {
            let _ = writeln!(out, "P\t{depth}\t{}\t{}", escape(target), escape(data));
        }
    }
}

/// The escapes of the `.dom` format.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c if u32::from(c) < 0x20 => {
                let _ = write!(out, "\\x{{{:x}}}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out
}

/// The files of a directory, sorted (none if it does not exist).
fn sorted_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    files.sort();
    Ok(files)
}

/// The class path of the plugin: the jars Maven's debug log lists under
/// "Populating class realm plugin>…", in the local repository.
fn realm(log: &str, version: &str, m2: &Path) -> Result<Vec<PathBuf>, String> {
    let header = format!("Populating class realm plugin>{PLUGIN}:{version}");
    let mut lines = log.lines().skip_while(|l| !l.contains(&header));
    if lines.next().is_none() {
        return Err(format!(
            "no class realm of {PLUGIN}:{version} in Maven's log"
        ));
    }
    let mut jars = Vec::new();
    for line in lines {
        let Some(coordinates) = line
            .trim_start_matches("[DEBUG]")
            .trim()
            .strip_prefix("Included: ")
        else {
            break;
        };
        let parts: Vec<&str> = coordinates.split(':').collect();
        let (group, artifact, classifier, version) = match parts.as_slice() {
            [g, a, "jar", v] => (*g, *a, None, *v),
            [g, a, "jar", c, v] => (*g, *a, Some(*c), *v),
            _ => return Err(format!("unexpected realm entry {coordinates}")),
        };
        let file = match classifier {
            Some(c) => format!("{artifact}-{version}-{c}.jar"),
            None => format!("{artifact}-{version}.jar"),
        };
        let mut path = m2.to_path_buf();
        path.extend(group.split('.'));
        path.push(artifact);
        path.push(version);
        path.push(file);
        if !path.is_file() {
            return Err(format!("no {} in the local repository", path.display()));
        }
        jars.push(path);
    }
    if jars.is_empty() {
        return Err(format!("empty class realm of {PLUGIN}:{version}"));
    }
    Ok(jars)
}

fn pom_xml(version: &str, input: &Path, output: &Path) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<project xmlns="http://maven.apache.org/POM/4.0.0">
  <modelVersion>4.0.0</modelVersion>
  <groupId>local.build</groupId>
  <artifactId>schematron-oracle</artifactId>
  <version>1</version>
  <packaging>pom</packaging>
  <build>
    <plugins>
      <plugin>
        <groupId>com.helger.maven</groupId>
        <artifactId>ph-schematron-maven-plugin</artifactId>
        <version>{version}</version>
        <executions>
          <execution>
            <id>convert</id>
            <phase>validate</phase>
            <goals><goal>convert</goal></goals>
            <configuration>
              <schematronDirectory>{}</schematronDirectory>
              <schematronPattern>*.sch</schematronPattern>
              <xsltDirectory>{}</xsltDirectory>
              <schematronEngine>iso-schematron</schematronEngine>
            </configuration>
          </execution>
        </executions>
      </plugin>
    </plugins>
  </build>
</project>
"#,
        xml_text(input),
        xml_text(output)
    )
}

/// The value of a `name=value` pin of `scripts/fetch-peppol.sh`.
fn pin(script: &str, name: &str) -> Result<String, String> {
    script
        .lines()
        .find_map(|line| line.strip_prefix(name)?.strip_prefix('='))
        .map(|v| v.trim().to_owned())
        .ok_or_else(|| format!("no pin {name} in scripts/fetch-peppol.sh"))
}

fn xml_text(path: &Path) -> String {
    path.display()
        .to_string()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .fold(String::new(), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        })
}

fn create_dir(path: &Path) -> Result<(), String> {
    std::fs::create_dir_all(path).map_err(|e| format!("{}: {e}", path.display()))
}

fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

fn read_bytes(path: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))
}

fn write(path: &Path, text: &str) -> Result<(), String> {
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}
