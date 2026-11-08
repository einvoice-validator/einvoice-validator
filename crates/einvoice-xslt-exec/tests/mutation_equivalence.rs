//! The mutation cases of the corpus (`corpus/mutations.toml`, see `corpus/README.md`): on
//! every one, the engine gives what Saxon-HE gave: the same canonical SVRL, or a dynamic
//! error with the same code when Saxon-HE stops with one. The CEN mutation cases are
//! committed; the Peppol ones are built in the cache with the Peppol corpus (without the
//! cache that test says SKIPPED, unless `EINVOICE_PEPPOL_REQUIRED` is set).

#![cfg(test)]

mod common;

use common::peppol;

fn report(name: &str, total: usize, problems: &[String], messages: &[String]) {
    eprintln!(
        "{name}: {total} mutation cases: {} equal, {} different; {} errors with another message",
        total - problems.len(),
        problems.len(),
        messages.len()
    );
    for p in problems.iter().take(30) {
        eprintln!("DIFFERENT {name} {p}");
    }
    for m in messages {
        eprintln!("MESSAGE {name} {m}");
    }
}

#[test]
fn every_cen_mutation_case_matches_saxon() {
    let root = common::workspace_root();
    let executable = common::executable(&root).unwrap();
    let corpus = common::join(&root, &format!("corpus/{}", common::RULE_SET));
    let cases = common::mutation_cases(&corpus, &root).unwrap();
    assert_eq!(cases.len(), 508, "mutation cases");
    let (problems, messages) = common::check_mutations(&executable, &cases, &corpus.join("golden"));
    report("CEN", cases.len(), &problems, &messages);
    assert!(problems.is_empty(), "{} different", problems.len());
}

#[test]
fn every_peppol_mutation_case_matches_saxon() {
    let root = common::workspace_root();
    let Some(corpus) = peppol::corpus(&root).unwrap_or_else(|e| panic!("{e}")) else {
        assert!(
            !peppol::required(),
            "no Peppol corpus; {} is set",
            peppol::REQUIRED_VAR
        );
        eprintln!("SKIPPED: no Peppol corpus in {}", peppol::BASE);
        return;
    };
    let base = common::join(&root, peppol::BASE);
    let cases = common::mutation_cases(&base.join("corpus"), &base).unwrap();
    assert!(!cases.is_empty(), "no Peppol mutation case");
    let stamp = common::read(&base.join("corpus").join("complete.txt")).unwrap();
    assert!(
        stamp
            .lines()
            .any(|l| l == format!("mutations {}", cases.len())),
        "the stamp does not record {} mutation cases",
        cases.len()
    );
    let mut different = 0;
    for stylesheet in &corpus.stylesheets {
        let executable = common::prepare(&stylesheet.xslt).unwrap();
        let golden_dir = stylesheet.golden(&peppol::Case {
            id: String::new(),
            input: String::new(),
        });
        let golden_dir = golden_dir.parent().unwrap().to_path_buf();
        let (problems, messages) = common::check_mutations(&executable, &cases, &golden_dir);
        report(stylesheet.name, cases.len(), &problems, &messages);
        different += problems.len();
    }
    assert_eq!(different, 0, "different");
}
