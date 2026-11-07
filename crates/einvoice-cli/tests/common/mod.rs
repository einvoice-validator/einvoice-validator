//! Running the binary, and the CEN EN 16931 UBL 1.3.17 corpus.

#![allow(
    dead_code,
    reason = "shared by several test targets, each using a part"
)]

pub(crate) mod json;

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub(crate) const CORPUS: &str = "corpus/cen-en16931-ubl-1.3.17";
pub(crate) const XSLT: &str =
    "artefacts/cen-en16931-ubl-1.3.17/ubl/xslt/EN16931-UBL-validation.xslt";
pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");

pub(crate) fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .map(Path::to_path_buf)
        .unwrap_or_default()
}

/// Joins a `/`-separated relative path onto `base`.
pub(crate) fn join(base: &Path, relative: &str) -> PathBuf {
    let mut path = base.to_path_buf();
    path.extend(relative.split('/').filter(|p| !p.is_empty()));
    path
}

/// What a run of the binary gave.
#[derive(Debug)]
pub(crate) struct Run {
    pub(crate) code: i32,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

/// Runs `einvoice-validate` in the workspace root with `args`, `stdin` as its standard
/// input, and `NO_COLOR` unset.
pub(crate) fn run(args: &[&str], stdin: Option<&[u8]>) -> Run {
    run_env(args, stdin, &[])
}

pub(crate) fn run_env(args: &[&str], stdin: Option<&[u8]>, env: &[(&str, &str)]) -> Run {
    let mut command = Command::new(env!("CARGO_BIN_EXE_einvoice-validate"));
    command
        .args(args)
        .current_dir(workspace_root())
        .env_remove("NO_COLOR")
        .envs(env.iter().copied())
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().unwrap();
    if let Some(bytes) = stdin {
        let mut input = child.stdin.take().unwrap();
        input.write_all(bytes).unwrap();
    }
    let output = child.wait_with_output().unwrap();
    Run {
        code: output.status.code().unwrap(),
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    }
}

/// One case of `cases.tsv`: its id, its input (relative to the workspace root) and the
/// ids its VEFA test expects to fail.
#[derive(Debug, Clone)]
pub(crate) struct Case {
    pub(crate) id: String,
    pub(crate) input: String,
    pub(crate) error: Vec<String>,
}

impl Case {
    pub(crate) fn golden(&self) -> String {
        std::fs::read_to_string(join(
            &workspace_root(),
            &format!("{CORPUS}/golden/{}.svrl.txt", self.id),
        ))
        .unwrap()
    }
}

pub(crate) fn cases() -> Vec<Case> {
    let text =
        std::fs::read_to_string(join(&workspace_root(), &format!("{CORPUS}/cases.tsv"))).unwrap();
    text.lines()
        .skip(1)
        .map(|line| {
            let columns: Vec<&str> = line.split('\t').collect();
            Case {
                id: columns[0].to_owned(),
                input: columns[1].to_owned(),
                error: columns
                    .get(5)
                    .map(|e| {
                        e.split(' ')
                            .filter(|s| !s.is_empty())
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default(),
            }
        })
        .collect()
}

pub(crate) fn case(id: &str) -> Case {
    cases().into_iter().find(|c| c.id == id).unwrap()
}

/// A finding: kind, id, flag, location.
pub(crate) type Finding = (String, Option<String>, Option<String>, String);

/// The failed asserts and successful reports of a canonical golden, in order.
pub(crate) fn golden_findings(canonical: &str) -> Vec<Finding> {
    einvoice_svrl::parse(canonical)
        .unwrap()
        .into_iter()
        .filter(|l| l.depth == 0 && (l.kind == "failed-assert" || l.kind == "successful-report"))
        .map(|l| {
            (
                l.kind.clone(),
                l.attribute("id").map(str::to_owned),
                l.attribute("flag").map(str::to_owned),
                l.attribute("location").unwrap_or_default().to_owned(),
            )
        })
        .collect()
}

/// The findings of one document of the JSON report.
pub(crate) fn json_findings(document: &json::Value) -> Vec<Finding> {
    document
        .get("findings")
        .array()
        .iter()
        .map(|f| {
            (
                f.get("kind").str().unwrap().to_owned(),
                f.get("id").str().map(str::to_owned),
                f.get("flag").str().map(str::to_owned),
                f.get("location").str().unwrap().to_owned(),
            )
        })
        .collect()
}

/// A fresh empty directory under the system's temporary directory.
pub(crate) fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "einvoice-validate-test-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
