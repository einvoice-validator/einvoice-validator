//! The Saxon-HE oracle of the probes in `tests/probes/` (format in
//! `tests/common/probes.rs`): runs Saxon on every probe stylesheet with `input.xml` and
//! compares its result, error code, warnings and messages with `expected/`, or rewrites
//! them. Saxon may print two kinds of warnings, ambiguous rule matches (XTDE0540) and
//! dynamic errors recovered while matching a pattern; any other is an error. What else it
//! prints before the error that ends the transformation, if any, is what `xsl:message`
//! printed.
//!
//! ```text
//! cargo run -p einvoice-xslt-exec --example saxon-probes              compare with expected/
//! cargo run -p einvoice-xslt-exec --example saxon-probes -- --write   rewrite expected/
//! ```
//!
//! Saxon-HE runs from the jars `scripts/fetch-saxon.sh` pins (file names and SHA-256 are
//! read from it): Saxon-HE's from `$SAXON_JAR`, else where that script puts it, and the
//! XML Resolver jars it needs from the same directory, each checksum verified. Java from
//! `$JAVA`, else `java`; the JVM runs in UTC.

#[path = "../tests/common/probes.rs"]
mod probes;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use probes::Expected;
use sha2::{Digest, Sha256};

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
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .map(Path::to_path_buf)
        .unwrap_or_default()
}

/// The jars `scripts/fetch-saxon.sh` pins, in class path order, Saxon-HE first: the
/// file name and SHA-256 of each, read from its `jars` array (file name, SHA-256 and URL
/// per jar), `$name` standing for the value of a `name=` line of the script.
fn pinned_jars(root: &Path) -> Result<Vec<(String, String)>, String> {
    let script = root.join("scripts").join("fetch-saxon.sh");
    let text =
        std::fs::read_to_string(&script).map_err(|e| format!("{}: {e}", script.display()))?;
    let bad = |what: &str| format!("{}: {what}", script.display());
    let mut values: Vec<(&str, &str)> = Vec::new();
    let mut words: Vec<String> = Vec::new();
    let mut in_array = false;
    for line in text.lines() {
        if in_array {
            let word = line.trim();
            if word == ")" {
                in_array = false;
            } else if !word.is_empty() && !word.starts_with('#') {
                words.push(substitute(word.trim_matches('"'), &values).map_err(|e| bad(&e))?);
            }
        } else if line == "jars=(" {
            in_array = true;
        } else if let Some((name, value)) = line.split_once('=')
            && !name.is_empty()
            && name.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
        {
            values.push((name, value.trim_matches('"')));
        }
    }
    if in_array || words.is_empty() || !words.len().is_multiple_of(3) {
        return Err(bad(
            "no `jars=(` array of file name, SHA-256 and URL triples",
        ));
    }
    let jars: Vec<(String, String)> = words
        .as_chunks::<3>()
        .0
        .iter()
        .map(|[name, sha256, _url]| (name.clone(), sha256.clone()))
        .collect();
    if !jars[0].0.starts_with("Saxon-HE-") {
        return Err(bad("the first jar of `jars` is not Saxon-HE"));
    }
    if let Some((name, _)) = jars
        .iter()
        .find(|(_, sha256)| sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err(bad(&format!("{name}: the SHA-256 is not 64 hex digits")));
    }
    Ok(jars)
}

/// `word` with each `$name` replaced by the value of the `name=` line in `values`.
fn substitute(word: &str, values: &[(&str, &str)]) -> Result<String, String> {
    let mut out = String::with_capacity(word.len());
    let mut rest = word;
    while let Some(dollar) = rest.find('$') {
        out.push_str(&rest[..dollar]);
        let after = &rest[dollar + 1..];
        let end = after
            .find(|c: char| !(c.is_ascii_lowercase() || c == '_'))
            .unwrap_or(after.len());
        let name = &after[..end];
        let value = values
            .iter()
            .find_map(|(n, v)| (*n == name).then_some(*v))
            .ok_or_else(|| format!("`${name}` in {word:?} has no `{name}=` line"))?;
        out.push_str(value);
        rest = &after[end..];
    }
    out.push_str(rest);
    Ok(out)
}

/// The class path of `net.sf.saxon.Transform`: the jars of [`pinned_jars`], Saxon-HE's
/// from `$SAXON_JAR`, else where `scripts/fetch-saxon.sh` puts it, and the others from
/// the same directory, the SHA-256 of each verified.
fn saxon_class_path(root: &Path) -> Result<std::ffi::OsString, String> {
    let pinned = pinned_jars(root)?;
    let saxon = std::env::var_os("SAXON_JAR").map_or_else(
        || root.join("target/oracle/saxon").join(&pinned[0].0),
        PathBuf::from,
    );
    let dir = saxon
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .to_path_buf();
    let mut jars = Vec::with_capacity(pinned.len());
    for (index, (name, sha256)) in pinned.iter().enumerate() {
        let jar = if index == 0 {
            saxon.clone()
        } else {
            dir.join(name)
        };
        let bytes = std::fs::read(&jar)
            .map_err(|e| format!("{}: {e} (run scripts/fetch-saxon.sh)", jar.display()))?;
        let mut actual = String::with_capacity(64);
        for byte in Sha256::digest(&bytes) {
            let _ = write!(actual, "{byte:02x}");
        }
        if actual != *sha256 {
            return Err(format!(
                "{}: SHA-256 {actual}, expected {sha256} ({name})",
                jar.display()
            ));
        }
        jars.push(jar);
    }
    std::env::join_paths(&jars).map_err(|e| format!("class path: {e}"))
}

/// The first XPath/XSLT error code in Saxon's messages.
fn error_code(stderr: &str) -> Option<String> {
    stderr
        .split(|c: char| !c.is_ascii_alphanumeric())
        .find_map(|word| {
            let bytes = word.as_bytes();
            (bytes.len() == 8
                && bytes[..4].iter().all(u8::is_ascii_uppercase)
                && bytes[4..].iter().all(u8::is_ascii_digit))
            .then(|| word.to_owned())
        })
}

/// Saxon's warnings, read from its standard error: each is a line starting with
/// `Warning`, which holds the location (`on line <L>`), then the lines up to the next
/// warning or error (the first of which, indented, starts with the code; the message of
/// an XTDE0540 warning goes on over unindented lines).
#[derive(Debug, Default)]
struct Warnings {
    /// The number of XTDE0540 warnings.
    ambiguities: usize,
    /// `(line, code)` of the errors recovered while matching a pattern.
    pattern_errors: std::collections::BTreeSet<(u32, String)>,
    /// The other lines of the standard error, the error that stopped Saxon if any.
    rest: String,
}

/// Whether a line of Saxon's standard error starts a warning or an error
/// (`StandardErrorReporter`: `Warning`, `Error `, `Type error `, `Static error `,
/// `Syntax error `).
fn starts_report(line: &str) -> bool {
    [
        "Warning",
        "Error ",
        "Type error ",
        "Static error ",
        "Syntax error ",
    ]
    .iter()
    .any(|p| line.starts_with(p))
}

fn warnings(stderr: &str) -> Result<Warnings, String> {
    let mut found = Warnings::default();
    let mut lines = stderr.lines().peekable();
    while let Some(line) = lines.next() {
        if !line.starts_with("Warning") {
            found.rest.push_str(line);
            found.rest.push('\n');
            continue;
        }
        let mut body = Vec::new();
        while let Some(more) = lines.next_if(|l| !starts_report(l)) {
            body.push(more.trim());
        }
        let body = body.join(" ");
        let code = error_code(&body).ok_or_else(|| format!("a warning without code: {line}"))?;
        if code == "XTDE0540" {
            found.ambiguities += 1;
        } else if body.contains("An error occurred matching pattern") {
            let at = line
                .split_once(" on line ")
                .and_then(|(_, after)| after.split(' ').next())
                .and_then(|n| n.parse().ok())
                .ok_or_else(|| format!("a warning without line: {line}"))?;
            found.pattern_errors.insert((at, code));
        } else {
            return Err(format!("an unexpected warning: {line}\n  {body}"));
        }
    }
    Ok(found)
}

fn run_saxon(
    java: &str,
    class_path: &std::ffi::OsStr,
    probe: &Path,
    input: &Path,
) -> Result<Expected, String> {
    let mut source = std::ffi::OsString::from("-s:");
    source.push(input);
    let mut xsl = std::ffi::OsString::from("-xsl:");
    xsl.push(probe);
    let output = Command::new(java)
        .arg("-Duser.timezone=UTC")
        .arg("-cp")
        .arg(class_path)
        .arg("net.sf.saxon.Transform")
        .arg(source)
        .arg(xsl)
        .output()
        .map_err(|e| format!("cannot run {java}: {e}"))?;
    let stdout = String::from_utf8(output.stdout).map_err(|e| e.to_string())?;
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    // Saxon's error reporter prints each distinct warning once, and stops after 25.
    if stderr.contains("No more warnings will be displayed") {
        return Err(format!(
            "{}: Saxon stopped printing warnings; the count would be wrong:\n{stderr}",
            probe.display()
        ));
    }
    let found = warnings(&stderr).map_err(|e| format!("{}: {e}", probe.display()))?;
    let pattern_errors = found.pattern_errors.into_iter().collect();
    if output.status.success() {
        if found.rest.lines().any(starts_report) {
            return Err(format!(
                "{}: Saxon printed more than warnings and messages:\n{stderr}",
                probe.display()
            ));
        }
        Ok(Expected {
            error: None,
            warnings: found.ambiguities,
            pattern_errors,
            messages: found.rest,
            output: stdout,
        })
    } else {
        // The messages, then the error.
        let at = found
            .rest
            .match_indices('\n')
            .map(|(i, _)| i + 1)
            .chain(std::iter::once(0))
            .filter(|&i| found.rest[i..].lines().next().is_some_and(starts_report))
            .min()
            .unwrap_or(found.rest.len());
        let (messages, report) = found.rest.split_at(at);
        // The code of the error, after the warnings.
        let code = error_code(report)
            .ok_or_else(|| format!("{}: no error code in\n{stderr}", probe.display()))?;
        Ok(Expected {
            error: Some(code),
            warnings: found.ambiguities,
            pattern_errors,
            messages: messages.to_owned(),
            output: String::new(),
        })
    }
}

fn run() -> Result<(), String> {
    let write = match std::env::args().nth(1).as_deref() {
        None => false,
        Some("--write") => true,
        Some(other) => return Err(format!("unknown argument {other}")),
    };
    let root = workspace_root();
    let class_path = saxon_class_path(&root)?;
    let java = std::env::var("JAVA").unwrap_or_else(|_| "java".to_owned());
    let dir = probes::probes_dir();
    let input = dir.join("input.xml");
    let mut problems = Vec::new();
    let names = probes::probes()?;
    for name in &names {
        let expected = run_saxon(&java, &class_path, &dir.join(format!("{name}.xsl")), &input)?;
        let path = probes::expected_path(name);
        if write {
            std::fs::create_dir_all(dir.join("expected")).map_err(|e| e.to_string())?;
            std::fs::write(&path, expected.render()).map_err(|e| e.to_string())?;
        } else {
            match std::fs::read_to_string(&path) {
                Ok(text) if text == expected.render() => {}
                Ok(_) => problems.push(format!("differs: {}", path.display())),
                Err(_) => problems.push(format!("missing: {}", path.display())),
            }
        }
    }
    if !write {
        for entry in std::fs::read_dir(dir.join("expected")).map_err(|e| e.to_string())? {
            let path = entry.map_err(|e| e.to_string())?.path();
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            if !names.iter().any(|n| n == stem) {
                problems.push(format!("stale: {}", path.display()));
            }
        }
    }
    for p in &problems {
        eprintln!("{p}");
    }
    eprintln!(
        "saxon-probes: {} probes, {}",
        names.len(),
        if write {
            "written".to_owned()
        } else {
            format!("{} problems", problems.len())
        }
    );
    if problems.is_empty() {
        Ok(())
    } else {
        Err("the expected results differ from Saxon's".to_owned())
    }
}
