//! The CEN EN 16931 UBL 1.3.17 corpus (`corpus/README.md`): its cases and their Saxon-HE
//! goldens.

#![allow(
    dead_code,
    reason = "shared by several test targets, each using a part"
)]

use std::path::{Path, PathBuf};

pub(crate) const CORPUS: &str = "corpus/cen-en16931-ubl-1.3.17";
pub(crate) const ARTEFACTS: &str = "artefacts/cen-en16931-ubl-1.3.17";
pub(crate) const XSLT: &str = "ubl/xslt/EN16931-UBL-validation.xslt";

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

/// One case of `cases.tsv`.
#[derive(Debug, Clone)]
pub(crate) struct Case {
    pub(crate) id: String,
    /// The input document, relative to the workspace root.
    pub(crate) input: String,
    /// VEFA only: the ids expected with flag `fatal`.
    pub(crate) error: Vec<String>,
}

impl Case {
    pub(crate) fn path(&self) -> PathBuf {
        join(&workspace_root(), &self.input)
    }

    pub(crate) fn golden(&self) -> PathBuf {
        join(
            &workspace_root(),
            &format!("{CORPUS}/golden/{}.svrl.txt", self.id),
        )
    }
}

pub(crate) fn cases() -> Vec<Case> {
    let text =
        std::fs::read_to_string(join(&workspace_root(), &format!("{CORPUS}/cases.tsv"))).unwrap();
    text.lines()
        .skip(1)
        .map(|line| {
            let columns: Vec<&str> = line.split('\t').collect();
            let error = columns
                .get(5)
                .map(|e| {
                    e.split(' ')
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            Case {
                id: columns[0].to_owned(),
                input: columns[1].to_owned(),
                error,
            }
        })
        .collect()
}

pub(crate) fn case(id: &str) -> Case {
    cases().into_iter().find(|c| c.id == id).unwrap()
}

/// A finding as the canonical golden states it: kind, id, flag, location.
pub(crate) type Expected = (String, Option<String>, Option<String>, String);

/// The failed asserts and successful reports of a canonical golden, in order.
pub(crate) fn expected_findings(canonical: &str) -> Vec<Expected> {
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

/// A fresh empty directory under the system's temporary directory.
pub(crate) fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "einvoice-validator-test-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
