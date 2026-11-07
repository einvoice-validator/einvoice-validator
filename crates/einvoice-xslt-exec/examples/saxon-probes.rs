//! The Saxon-HE oracle of the probes in `tests/probes/` (format in
//! `tests/common/probes.rs`): runs Saxon on every probe stylesheet with `input.xml` and
//! compares its result, error code and warnings with `expected/`, or rewrites them.
//!
//! ```text
//! cargo run -p einvoice-xslt-exec --example saxon-probes              compare with expected/
//! cargo run -p einvoice-xslt-exec --example saxon-probes -- --write   rewrite expected/
//! ```
//!
//! Saxon-HE is the release `scripts/fetch-saxon.sh` pins (version and SHA-256 are read
//! from it), taken from `$SAXON_JAR`, else where that script puts it; its checksum is
//! verified. Java from `$JAVA`, else `java`; the JVM runs in UTC.

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

/// The pinned Saxon release: `version` and `sha256` of `scripts/fetch-saxon.sh`.
fn pinned_saxon(root: &Path) -> Result<(String, String), String> {
    let script = root.join("scripts").join("fetch-saxon.sh");
    let text =
        std::fs::read_to_string(&script).map_err(|e| format!("{}: {e}", script.display()))?;
    let value = |key: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(&format!("{key}=")))
            .map(|v| v.trim().trim_matches('"').to_owned())
            .ok_or_else(|| format!("{}: no `{key}=` line", script.display()))
    };
    Ok((value("version")?, value("sha256")?))
}

fn saxon_jar(root: &Path) -> Result<PathBuf, String> {
    let (version, sha256) = pinned_saxon(root)?;
    let jar = std::env::var_os("SAXON_JAR").map_or_else(
        || root.join(format!("target/oracle/saxon/Saxon-HE-{version}.jar")),
        PathBuf::from,
    );
    let bytes = std::fs::read(&jar)
        .map_err(|e| format!("{}: {e} (run scripts/fetch-saxon.sh)", jar.display()))?;
    let mut actual = String::with_capacity(64);
    for byte in Sha256::digest(&bytes) {
        let _ = write!(actual, "{byte:02x}");
    }
    if actual != sha256 {
        return Err(format!(
            "{}: SHA-256 {actual}, expected {sha256} (Saxon-HE {version})",
            jar.display()
        ));
    }
    Ok(jar)
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

fn run_saxon(java: &str, jar: &Path, probe: &Path, input: &Path) -> Result<Expected, String> {
    let mut source = std::ffi::OsString::from("-s:");
    source.push(input);
    let mut xsl = std::ffi::OsString::from("-xsl:");
    xsl.push(probe);
    let output = Command::new(java)
        .arg("-Duser.timezone=UTC")
        .arg("-jar")
        .arg(jar)
        .arg(source)
        .arg(xsl)
        .output()
        .map_err(|e| format!("cannot run {java}: {e}"))?;
    let stdout = String::from_utf8(output.stdout).map_err(|e| e.to_string())?;
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let warnings = stderr.matches("XTDE0540").count();
    if output.status.success() {
        let unexpected: Vec<&str> = stderr
            .lines()
            .filter(|l| l.contains("Warning") || l.contains("Error"))
            .collect();
        if unexpected.len() != warnings {
            return Err(format!(
                "{}: Saxon printed warnings other than XTDE0540:\n{stderr}",
                probe.display()
            ));
        }
        Ok(Expected {
            error: None,
            warnings,
            output: stdout,
        })
    } else {
        let code = error_code(&stderr)
            .ok_or_else(|| format!("{}: no error code in\n{stderr}", probe.display()))?;
        Ok(Expected {
            error: Some(code),
            warnings,
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
    let jar = saxon_jar(&root)?;
    let java = std::env::var("JAVA").unwrap_or_else(|_| "java".to_owned());
    let dir = probes::probes_dir();
    let input = dir.join("input.xml");
    let mut problems = Vec::new();
    let names = probes::probes()?;
    for name in &names {
        let expected = run_saxon(&java, &jar, &dir.join(format!("{name}.xsl")), &input)?;
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
