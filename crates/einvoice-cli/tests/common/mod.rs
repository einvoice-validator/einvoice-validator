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

/// The longest command line, in characters, a batch of [`run_json_batches`] may build.
/// Windows limits a command line to 32767 UTF-16 code units (`CreateProcess`); this
/// leaves room for the quotes and separators the standard library adds.
const COMMAND_LINE_BUDGET: usize = 30_000;

/// The runs of [`run_json_batches`], each with its JSON report.
#[derive(Debug)]
pub(crate) struct JsonBatches {
    pub(crate) runs: Vec<(Run, json::Value)>,
}

impl JsonBatches {
    /// The reports' documents, in the order of the inputs.
    pub(crate) fn documents(&self) -> Vec<&json::Value> {
        self.runs
            .iter()
            .flat_map(|(_, report)| report.get("documents").array())
            .collect()
    }

    /// The highest exit status of the runs: the one a single run on every input gives.
    pub(crate) fn code(&self) -> i32 {
        self.runs.iter().map(|(r, _)| r.code).max().unwrap()
    }

    /// The standard error of every run, concatenated.
    pub(crate) fn stderr(&self) -> String {
        self.runs.iter().map(|(r, _)| r.stderr.as_str()).collect()
    }

    /// The sum of a count of the reports' summaries.
    pub(crate) fn summary(&self, key: &str) -> u64 {
        self.runs
            .iter()
            .map(|(_, report)| report.get("summary").get(key).number())
            .sum()
    }

    /// The report of the first run (its rule set is the same in every run).
    pub(crate) fn first(&self) -> &json::Value {
        &self.runs[0].1
    }
}

/// Runs `einvoice-validate` with `args` (which must ask for `--format json`), then the
/// `inputs`, in as many runs as it takes for each command line to stay within
/// [`COMMAND_LINE_BUDGET`], and parses each run's report. The inputs keep their order.
pub(crate) fn run_json_batches(args: &[&str], inputs: &[&str]) -> JsonBatches {
    let length = |arg: &str| arg.len() + 3;
    let fixed = length(env!("CARGO_BIN_EXE_einvoice-validate"))
        + args.iter().map(|a| length(a)).sum::<usize>();
    let mut batches: Vec<Vec<&str>> = vec![Vec::new()];
    let mut used = fixed;
    for input in inputs {
        let last = batches.last_mut().unwrap();
        if !last.is_empty() && used + length(input) > COMMAND_LINE_BUDGET {
            batches.push(Vec::new());
            used = fixed;
        }
        assert!(
            fixed + length(input) <= COMMAND_LINE_BUDGET,
            "{input}: path too long"
        );
        batches.last_mut().unwrap().push(input);
        used += length(input);
    }
    let runs = batches
        .into_iter()
        .map(|batch| {
            let mut command: Vec<&str> = args.to_vec();
            command.extend(batch);
            let r = run(&command, None);
            let report = json::parse(&r.stdout).unwrap();
            (r, report)
        })
        .collect();
    JsonBatches { runs }
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

/// One mutation case of `<corpus>/mutations.tsv`: its id and its input, relative to the
/// workspace root (`prefix` is the base directory of the inputs, relative to the root).
#[derive(Debug, Clone)]
pub(crate) struct MutationCase {
    pub(crate) id: String,
    pub(crate) input: String,
}

pub(crate) fn mutation_cases(corpus: &str, prefix: &str) -> Vec<MutationCase> {
    let text = std::fs::read_to_string(join(&workspace_root(), &format!("{corpus}/mutations.tsv")))
        .unwrap();
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some("case\tinput\tbase\treason"));
    lines
        .map(|line| {
            let mut columns = line.split('\t');
            let id = columns.next().unwrap().to_owned();
            let input = columns.next().unwrap();
            let input = if prefix.is_empty() {
                input.to_owned()
            } else {
                format!("{prefix}/{input}")
            };
            MutationCase { id, input }
        })
        .collect()
}

/// What Saxon-HE gave with one stylesheet: findings, or the code of the error that
/// stopped it (`<id>.error.txt`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Golden {
    Findings(Vec<Finding>),
    Error(String),
}

pub(crate) fn golden_of(golden_dir: &str, id: &str) -> Golden {
    let root = workspace_root();
    let error = join(&root, &format!("{golden_dir}/{id}.error.txt"));
    if error.exists() {
        let text = std::fs::read_to_string(error).unwrap();
        let code = text.lines().next().unwrap().strip_prefix("error ").unwrap();
        return Golden::Error(code.to_owned());
    }
    let svrl =
        std::fs::read_to_string(join(&root, &format!("{golden_dir}/{id}.svrl.txt"))).unwrap();
    Golden::Findings(golden_findings(&svrl))
}

/// Compares one document of a JSON report with the goldens of its stylesheets
/// (`(stylesheet name, golden directory)`, in the order they run). Returns the problems.
pub(crate) fn check_mutation_document(
    id: &str,
    document: &json::Value,
    stylesheets: &[(&str, &str)],
) -> Vec<String> {
    let mut problems = Vec::new();
    let schema_errors = document.get("counts").get("schema_errors").number();
    let status = document.get("status").str().unwrap();
    let errors: Vec<(&str, &str)> = document
        .get("stylesheet_errors")
        .array()
        .iter()
        .map(|e| {
            (
                e.get("stylesheet").str().unwrap(),
                e.get("code").str().unwrap(),
            )
        })
        .collect();
    let mut fatal = false;
    let mut stopped = None;
    for (name, dir) in stylesheets {
        match golden_of(dir, id) {
            Golden::Error(code) => {
                stopped.get_or_insert((*name, code.clone()));
                if schema_errors > 0 && !errors.contains(&(*name, code.as_str())) {
                    problems.push(format!(
                        "{id}: {name}: expected stylesheet error {code}, got {errors:?}"
                    ));
                }
            }
            Golden::Findings(expected) => {
                fatal |= expected.iter().any(|f| f.2.as_deref() == Some("fatal"));
                if stopped.is_some() && schema_errors == 0 {
                    continue;
                }
                let ours: Vec<Finding> = document
                    .get("findings")
                    .array()
                    .iter()
                    .filter(|f| f.get("stylesheet").str() == Some(*name))
                    .map(|f| {
                        (
                            f.get("kind").str().unwrap().to_owned(),
                            f.get("id").str().map(str::to_owned),
                            f.get("flag").str().map(str::to_owned),
                            f.get("location").str().unwrap().to_owned(),
                        )
                    })
                    .collect();
                if ours != expected || errors.iter().any(|(s, _)| s == name) {
                    problems.push(format!(
                        "{id}: {name}: findings differ (stylesheet errors {errors:?})"
                    ));
                }
            }
        }
    }
    let expected_status = match &stopped {
        Some((name, code)) if schema_errors == 0 => {
            let message = document.get("error").get("message").str().unwrap_or("");
            if document.get("error").get("kind").str() != Some("transform")
                || !message.contains(code.as_str())
            {
                problems.push(format!(
                    "{id}: {name}: expected a transform error {code}, got {message:?}"
                ));
            }
            "error"
        }
        _ if fatal || schema_errors > 0 => "invalid",
        _ => "valid",
    };
    if status != expected_status {
        problems.push(format!("{id}: {status}, expected {expected_status}"));
    }
    problems
}
