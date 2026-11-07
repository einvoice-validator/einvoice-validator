//! Milestone of issue #3: on every case of the CEN EN 16931 UBL 1.3.17 corpus, the
//! canonical SVRL this crate produces is identical to the Saxon-HE golden, line for line.
//! No Java is needed: the goldens are committed (`corpus/README.md`).

// Integration tests are only ever built with `cfg(test)`; saying so lets clippy apply its
// test exemptions (unwrap, expect, panic) to the helpers outside `#[test]` functions too.
#![cfg(test)]

mod common;

#[test]
fn every_case_matches_the_saxon_golden() {
    let root = common::workspace_root();
    let executable = common::executable(&root).unwrap();
    let cases = common::cases(&root).unwrap();
    assert_eq!(cases.len(), 1179, "corpus size");
    let mut different = Vec::new();
    let mut failed = Vec::new();
    for case in &cases {
        let text = common::read(std::path::Path::new(&case.input)).unwrap();
        let golden = common::read(&case.golden).unwrap();
        let ours = common::svrl(&executable, &text, &case.input)
            .and_then(|svrl| common::canonical(&svrl, &case.input));
        match ours {
            Ok(canonical) if canonical == golden => {}
            Ok(canonical) => different.push(format!(
                "{}\n{}",
                case.id,
                first_difference(&golden, &canonical)
            )),
            Err(e) => failed.push(format!("{}: {e}", case.id)),
        }
    }
    let equal = cases.len() - different.len() - failed.len();
    eprintln!(
        "{} cases: {equal} equal, {} different, {} failed",
        cases.len(),
        different.len(),
        failed.len()
    );
    for d in different.iter().take(20) {
        eprintln!("DIFFERENT {d}");
    }
    for f in failed.iter().take(20) {
        eprintln!("FAILED {f}");
    }
    assert!(
        different.is_empty() && failed.is_empty(),
        "{} different, {} failed",
        different.len(),
        failed.len()
    );
}

/// The first line where two canonical files differ, with both versions.
fn first_difference(golden: &str, ours: &str) -> String {
    let (g, o): (Vec<&str>, Vec<&str>) = (golden.lines().collect(), ours.lines().collect());
    let index = g
        .iter()
        .zip(&o)
        .position(|(a, b)| a != b)
        .unwrap_or_else(|| g.len().min(o.len()));
    format!(
        "  line {}\n  saxon: {}\n  ours:  {}\n  (saxon {} lines, ours {} lines)",
        index + 1,
        g.get(index).unwrap_or(&"<end>"),
        o.get(index).unwrap_or(&"<end>"),
        g.len(),
        o.len()
    )
}
