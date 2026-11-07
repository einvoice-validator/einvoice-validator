//! The official rule sets the corpus covers, and where their files live.

/// One vendored rule set and the corpus built from it.
#[derive(Debug)]
pub(crate) struct RuleSet {
    /// Directory name under `artefacts/` (vendored, unmodified upstream files)
    /// and under `corpus/` (derived cases and golden results).
    pub(crate) name: &'static str,
    /// The upstream licence text, relative to the vendored directory. The
    /// derived corpus carries a copy of it.
    pub(crate) licence: &'static str,
    /// The compiled XSLT the oracle runs, relative to the vendored directory.
    pub(crate) xslt: &'static str,
    /// Whole documents used as cases: (case id prefix, directory relative to
    /// the vendored directory).
    pub(crate) document_dirs: &'static [(&'static str, &'static str)],
    /// VEFA unit test sets, relative to the vendored directory. Each `<test>`
    /// becomes one case under `vefa/<directory name>/`.
    pub(crate) vefa_dirs: &'static [&'static str],
}

impl RuleSet {
    /// `artefacts/<name>`, relative to the repository root.
    pub(crate) fn vendored_dir(&self) -> String {
        format!("artefacts/{}", self.name)
    }

    /// `corpus/<name>`, relative to the repository root.
    pub(crate) fn corpus_dir(&self) -> String {
        format!("corpus/{}", self.name)
    }
}

/// CEN EN 16931 validation artefacts for UBL, release 1.3.17.
pub(crate) const CEN_UBL: RuleSet = RuleSet {
    name: "cen-en16931-ubl-1.3.17",
    licence: "LICENSE.txt",
    xslt: "ubl/xslt/EN16931-UBL-validation.xslt",
    document_dirs: &[
        ("examples", "ubl/examples"),
        ("testfiles", "test/testfiles"),
    ],
    vefa_dirs: &["test/Invoice-unit-UBL", "test/CreditNote-unit-UBL"],
};

/// Every rule set in the corpus.
pub(crate) const ALL: &[&RuleSet] = &[&CEN_UBL];
