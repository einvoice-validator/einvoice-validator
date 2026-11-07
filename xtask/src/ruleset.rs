//! The official rule sets the corpus covers, and where their files live.
//!
//! A rule set is either vendored (its upstream files are committed under
//! `artefacts/<name>/`, its corpus and goldens under `corpus/<name>/`) or
//! fetched (its upstream files may not be redistributed: `scripts/fetch-peppol.sh`
//! downloads and compiles them into a cache under `target/`, where the corpus
//! and the goldens are generated too; only the record of VEFA disagreements is
//! committed, under `corpus/<name>/`).
//!
//! Every path of a rule set is relative to its base directory: the repository
//! root for a vendored rule set, the cache directory for a fetched one. The
//! `input` column of `cases.tsv` is relative to the base too.

use std::path::{Path, PathBuf};

use crate::util;

/// Where the files of a rule set come from.
#[derive(Debug)]
pub(crate) enum Origin {
    /// Committed under `artefacts/<name>/`, with a `SHA256SUMS` manifest; the
    /// derived corpus and the goldens are committed under `corpus/<name>/`
    /// with a copy of the licence (relative to the vendored directory).
    Vendored { licence: &'static str },
    /// Built by `scripts/fetch-peppol.sh` into `base` (relative to the
    /// repository root): the upstream tree under `base/source`, the compiled
    /// stylesheets under `base/xslt`; the corpus is generated in `base/corpus`.
    Fetched { base: &'static str },
}

/// One compiled stylesheet of a rule set, run on every case.
#[derive(Debug)]
pub(crate) struct Stylesheet {
    /// Short name, in messages.
    pub(crate) name: &'static str,
    /// The compiled XSLT, relative to the base directory.
    pub(crate) xslt: &'static str,
    /// Directory of its goldens, relative to the corpus directory.
    pub(crate) golden_dir: &'static str,
    /// For a fetched rule set: the name of the pin in `scripts/fetch-peppol.sh`
    /// that holds the SHA-256 of the compiled XSLT.
    pub(crate) sha256_pin: Option<&'static str>,
}

/// One rule set and the corpus built from it.
#[derive(Debug)]
pub(crate) struct RuleSet {
    /// Directory name under `corpus/` (and under `artefacts/` when vendored).
    pub(crate) name: &'static str,
    pub(crate) origin: Origin,
    /// The directory the document and VEFA directories are relative to,
    /// relative to the base directory.
    pub(crate) source: &'static str,
    /// The stylesheets run on every case; each has its own goldens.
    pub(crate) stylesheets: &'static [Stylesheet],
    /// Whole documents used as cases: (case id prefix, directory relative to
    /// the source directory). Every file below the directory is a case.
    pub(crate) document_dirs: &'static [(&'static str, &'static str)],
    /// VEFA unit test sets, relative to the source directory. Each `<test>`
    /// of each `<name>.xml` file becomes one case under `vefa/<directory name>/`.
    pub(crate) vefa_dirs: &'static [&'static str],
    /// Files in those directories that are neither documents nor test sets,
    /// relative to the source directory. Each must exist.
    pub(crate) ignored: &'static [&'static str],
    /// The base documents of the mutation catalogue (`corpus/mutations.toml`):
    /// (base name, id of a whole-document case).
    pub(crate) mutation_bases: &'static [(&'static str, &'static str)],
}

impl RuleSet {
    /// The base directory of the rule set's paths.
    pub(crate) fn base(&self, root: &Path) -> PathBuf {
        match self.origin {
            Origin::Vendored { .. } => root.to_path_buf(),
            Origin::Fetched { base } => util::join(root, base),
        }
    }

    /// `artefacts/<name>`, relative to the repository root, when vendored.
    pub(crate) fn vendored_dir(&self) -> Option<String> {
        match self.origin {
            Origin::Vendored { .. } => Some(format!("artefacts/{}", self.name)),
            Origin::Fetched { .. } => None,
        }
    }

    /// The licence file, relative to the source directory, when vendored.
    pub(crate) fn licence(&self) -> Option<&'static str> {
        match self.origin {
            Origin::Vendored { licence } => Some(licence),
            Origin::Fetched { .. } => None,
        }
    }

    /// The corpus directory, relative to the base directory.
    pub(crate) fn corpus_dir(&self) -> String {
        match self.origin {
            Origin::Vendored { .. } => self.record_dir(),
            Origin::Fetched { .. } => "corpus".to_owned(),
        }
    }

    /// `corpus/<name>`, relative to the repository root: the committed files of
    /// the rule set's corpus (all of it when vendored, the record of VEFA
    /// disagreements and the README when fetched).
    pub(crate) fn record_dir(&self) -> String {
        format!("corpus/{}", self.name)
    }
}

/// CEN EN 16931 validation artefacts for UBL, release 1.3.17.
pub(crate) const CEN_UBL: RuleSet = RuleSet {
    name: "cen-en16931-ubl-1.3.17",
    origin: Origin::Vendored {
        licence: "LICENSE.txt",
    },
    source: "artefacts/cen-en16931-ubl-1.3.17",
    stylesheets: &[Stylesheet {
        name: "EN16931-UBL-validation",
        xslt: "artefacts/cen-en16931-ubl-1.3.17/ubl/xslt/EN16931-UBL-validation.xslt",
        golden_dir: "golden",
        sha256_pin: None,
    }],
    document_dirs: &[
        ("examples", "ubl/examples"),
        ("testfiles", "test/testfiles"),
    ],
    vefa_dirs: &["test/Invoice-unit-UBL", "test/CreditNote-unit-UBL"],
    ignored: &[],
    mutation_bases: &[
        ("invoice", "testfiles/Invoice-Max_content.xml"),
        ("credit-note", "testfiles/CreditNote-Max_content.xml"),
        ("allowances", "examples/ubl-tc434-example2.xml"),
    ],
};

/// Peppol BIS Billing 3.0.22, UBL syntax: its own rules and the
/// CEN EN 16931 rules it embeds, both compiled by `scripts/fetch-peppol.sh`.
pub(crate) const PEPPOL_UBL: RuleSet = RuleSet {
    name: "peppol-bis-billing-ubl-3.0.22",
    origin: Origin::Fetched {
        base: "target/peppol/peppol-bis-billing-ubl-3.0.22",
    },
    source: "source/rules",
    stylesheets: &[
        Stylesheet {
            name: "PEPPOL-EN16931-UBL",
            xslt: "xslt/PEPPOL-EN16931-UBL.xslt",
            golden_dir: "golden-peppol",
            sha256_pin: Some("peppol_xslt_sha256"),
        },
        Stylesheet {
            name: "CEN-EN16931-UBL",
            xslt: "xslt/CEN-EN16931-UBL.xslt",
            golden_dir: "golden-cen",
            sha256_pin: Some("cen_xslt_sha256"),
        },
    ],
    document_dirs: &[
        ("examples", "examples"),
        ("snippets", "snippets"),
        ("national-examples", "national-examples"),
    ],
    vefa_dirs: &[
        "unit-UBL-PEPPOL",
        "unit-UBL-DE",
        "unit-UBL-DK",
        "unit-UBL-GR",
        "unit-UBL-IT",
        "unit-UBL-NL",
        "unit-UBL-NO",
        "unit-UBL-SE",
    ],
    ignored: &[
        "national-examples/README.adoc",
        // Every test is commented out, and the upstream build, which runs the
        // `*.xml` files of a test folder, does not run it.
        "unit-UBL-PEPPOL/PEPPOL-COMMON-R048.xm",
    ],
    mutation_bases: &[
        ("invoice", "examples/base-example.xml"),
        ("credit-note", "examples/base-creditnote-correction.xml"),
        ("allowances", "examples/Allowance-example.xml"),
    ],
};

/// The vendored rule sets: their corpus is committed.
pub(crate) const VENDORED: &[&RuleSet] = &[&CEN_UBL];

/// Every rule set, vendored or fetched.
pub(crate) const ALL: &[&RuleSet] = &[&CEN_UBL, &PEPPOL_UBL];
