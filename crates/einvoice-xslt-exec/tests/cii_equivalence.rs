//! On every case of the CEN EN 16931 CII 1.3.17 corpus, and on every one of its mutation
//! cases (`corpus/mutations-cii.toml`), the canonical SVRL this crate produces is identical
//! to the Saxon-HE golden, line for line, or, where Saxon-HE stops with a dynamic error,
//! the engine stops with the same code. No Java is needed: the goldens are committed
//! (`corpus/README.md`).

// Integration tests are only ever built with `cfg(test)`; saying so lets clippy apply its
// test exemptions (unwrap, expect, panic) to the helpers outside `#[test]` functions too.
#![cfg(test)]

mod common;

/// The stack of the thread that runs the cases: as `einvoice-validate`'s workers. The
/// engine recurses once per level of the document tree; the mutation case
/// `long/deep-nesting.xml` (300 levels) needs more than the 2 MiB of a test thread in a
/// debug build.
const STACK_SIZE: usize = 8 << 20;

fn on_a_large_stack(f: fn()) {
    std::thread::Builder::new()
        .stack_size(STACK_SIZE)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn every_cii_case_matches_the_saxon_golden() {
    on_a_large_stack(cases);
}

fn cases() {
    let root = common::workspace_root();
    let executable = common::prepare(&common::join(&root, common::CII_XSLT)).unwrap();
    let cases = common::cases_of(&root, common::CII_RULE_SET).unwrap();
    assert_eq!(cases.len(), 24, "corpus size");
    let mut problems = Vec::new();
    for case in &cases {
        let text = common::read(std::path::Path::new(&case.input)).unwrap();
        let golden = common::read(&case.golden).unwrap();
        let ours = common::svrl(&executable, &text, &case.input)
            .and_then(|svrl| common::canonical(&svrl, &case.input));
        match ours {
            Ok(canonical) if canonical == golden => {}
            Ok(canonical) => problems.push(format!(
                "DIFFERENT {}\n{}",
                case.id,
                common::first_difference(&golden, &canonical)
            )),
            Err(e) => problems.push(format!("FAILED {}: {e}", case.id)),
        }
    }
    eprintln!(
        "{} CII cases: {} equal, {} not",
        cases.len(),
        cases.len() - problems.len(),
        problems.len()
    );
    for p in problems.iter().take(20) {
        eprintln!("{p}");
    }
    assert!(problems.is_empty(), "{} not equal", problems.len());
}

#[test]
fn every_cii_mutation_case_matches_saxon() {
    on_a_large_stack(mutations);
}

fn mutations() {
    let root = common::workspace_root();
    let executable = common::prepare(&common::join(&root, common::CII_XSLT)).unwrap();
    let corpus = common::join(&root, &format!("corpus/{}", common::CII_RULE_SET));
    let cases = common::mutation_cases(&corpus, &root).unwrap();
    assert_eq!(cases.len(), 808, "mutation cases");
    let (problems, messages) = common::check_mutations(&executable, &cases, &corpus.join("golden"));
    eprintln!(
        "CII: {} mutation cases: {} equal, {} different; {} errors with another message",
        cases.len(),
        cases.len() - problems.len(),
        problems.len(),
        messages.len()
    );
    for p in problems.iter().take(30) {
        eprintln!("DIFFERENT {p}");
    }
    for m in &messages {
        eprintln!("MESSAGE {m}");
    }
    assert!(problems.is_empty(), "{} different", problems.len());
}
