//! On every case and mutation case of the corpora of the French rules, FNFE-MPE package 1.4.0.04 (CII, UBL and CDAR), and for every stylesheet,
//! the canonical SVRL this crate produces is identical to the Saxon-HE golden, line for
//! line, or the engine stops with the error code Saxon-HE stopped with.
//!
//! None of these files may be redistributed, so the corpora and their goldens live in a
//! cache outside the tracked tree, built by `scripts/check.sh fr-ctc` (`scripts/fetch-fr-ctc.sh`,
//! then `cargo xtask fr-ctc`; see `corpus/README.md`). Without the cache this test says
//! SKIPPED and passes, unless the variable named in `common::fetched::FR_CTC` is set, as
//! `scripts/check.sh fr-ctc` and CI set it: then it fails.

// Integration tests are only ever built with `cfg(test)`; saying so lets clippy apply its
// test exemptions (unwrap, expect, panic) to the helpers outside `#[test]` functions too.
#![cfg(test)]

mod common;

#[test]
fn every_case_matches_the_saxon_goldens_of_every_stylesheet() {
    let problems = common::fetched::check_family(&common::fetched::FR_CTC);
    assert_eq!(problems, 0, "cases differing from the Saxon goldens");
}
