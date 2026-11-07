//! Milestone of issue #5: on every case of the Peppol BIS Billing 3.0.22 corpus, and for
//! both stylesheets Peppol validation runs (its own rules and the CEN EN 16931 rules it
//! embeds), the canonical SVRL this crate produces is identical to the Saxon-HE golden,
//! line for line.
//!
//! The Peppol files may not be redistributed, so the corpus and its goldens live in a
//! cache outside the tracked tree, built by `scripts/check.sh peppol`
//! (`scripts/fetch-peppol.sh`, then `cargo xtask peppol`; see `corpus/README.md`). Without
//! the cache this test says SKIPPED and passes, unless `EINVOICE_PEPPOL_REQUIRED` is set,
//! as `scripts/check.sh peppol` and the `peppol` CI workflow do: then it fails.

// Integration tests are only ever built with `cfg(test)`; saying so lets clippy apply its
// test exemptions (unwrap, expect, panic) to the helpers outside `#[test]` functions too.
#![cfg(test)]

mod common;

use common::peppol;

#[test]
fn every_case_matches_the_saxon_goldens_of_both_stylesheets() {
    let root = common::workspace_root();
    let Some(corpus) = peppol::corpus(&root).unwrap_or_else(|e| panic!("{e}")) else {
        let message = format!(
            "no Peppol corpus in {}: build it with scripts/check.sh peppol (needs Java and network access)",
            peppol::BASE
        );
        assert!(
            !peppol::required(),
            "{message}; {} is set",
            peppol::REQUIRED_VAR
        );
        eprintln!("SKIPPED: {message}");
        return;
    };
    assert!(!corpus.cases.is_empty(), "empty Peppol corpus");
    let mut problems = 0;
    for stylesheet in &corpus.stylesheets {
        let executable = common::prepare(&stylesheet.xslt).unwrap_or_else(|e| panic!("{e}"));
        let mut different = Vec::new();
        let mut failed = Vec::new();
        for case in &corpus.cases {
            let text = common::read(std::path::Path::new(&case.input)).unwrap();
            let golden = common::read(&stylesheet.golden(case)).unwrap();
            let ours = common::svrl(&executable, &text, &case.input)
                .and_then(|svrl| common::canonical(&svrl, &case.input));
            match ours {
                Ok(canonical) if canonical == golden => {}
                Ok(canonical) => different.push(format!(
                    "{}\n{}",
                    case.id,
                    common::first_difference(&golden, &canonical)
                )),
                Err(e) => failed.push(format!("{}: {e}", case.id)),
            }
        }
        let equal = corpus.cases.len() - different.len() - failed.len();
        eprintln!(
            "{}: {} cases: {equal} equal, {} different, {} failed",
            stylesheet.name,
            corpus.cases.len(),
            different.len(),
            failed.len()
        );
        for d in different.iter().take(20) {
            eprintln!("DIFFERENT {} {d}", stylesheet.name);
        }
        for f in failed.iter().take(20) {
            eprintln!("FAILED {} {f}", stylesheet.name);
        }
        problems += different.len() + failed.len();
    }
    assert_eq!(problems, 0, "cases differing from the Saxon goldens");
}
