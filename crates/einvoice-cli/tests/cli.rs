//! The `einvoice-validate` binary: options, output formats, exit codes, and its verdicts
//! and findings on the whole CEN EN 16931 UBL 1.3.17 corpus against the Saxon-HE
//! goldens.

#![cfg(test)]

mod common;

use common::json::{self, Value};
use common::run;

const VALID: &str = "artefacts/cen-en16931-ubl-1.3.17/ubl/examples/ubl-tc434-example1.xml";
const BR_02: &str = "corpus/cen-en16931-ubl-1.3.17/vefa/Invoice-unit-UBL/BR-02.002.xml";
/// A credit note with fatal findings and a warning.
const CN_BR_10: &str = "corpus/cen-en16931-ubl-1.3.17/vefa/CreditNote-unit-UBL/BR-10.001.xml";

fn read(relative: &str) -> Vec<u8> {
    std::fs::read(common::join(&common::workspace_root(), relative)).unwrap()
}

#[test]
fn help() {
    for flag in ["--help", "-h"] {
        let r = run(&[flag], None);
        assert_eq!(r.code, 0);
        assert_eq!(r.stderr, "");
        assert!(
            r.stdout
                .starts_with(&format!("einvoice-validate {}\n", common::VERSION))
        );
        for text in [
            "Usage: einvoice-validate [OPTIONS] FILE...",
            "-r, --rules NAME",
            "-b, --bundle DIR",
            "-f, --format FORMAT",
            "-j, --jobs N",
            "cen-ubl  CEN EN 16931 UBL 1.3.17, with the ubl-2.1 schema",
            "NO_COLOR",
            "format_version 1",
            "0  every document is valid",
            "1  a document is invalid",
            "2  usage error",
            "3  an unsupported construct",
        ] {
            assert!(r.stdout.contains(text), "{text:?} not in\n{}", r.stdout);
        }
    }
}

#[test]
fn version() {
    let r = run(&["--version"], None);
    assert_eq!(r.code, 0);
    assert_eq!(
        r.stdout,
        format!(
            "einvoice-validate {}\nBuilt-in rule sets:\n  cen-ubl  CEN EN 16931 UBL 1.3.17, with the ubl-2.1 schema\n",
            common::VERSION
        )
    );
}

#[test]
fn usage_errors_exit_with_2() {
    for args in [
        &[][..],
        &["--bogus", VALID],
        &["-f", "xml", VALID],
        &["-j", "0", VALID],
        &["-r", "cen-ubl", "-b", "dir", VALID],
    ] {
        let r = run(args, None);
        assert_eq!(r.code, 2, "{args:?}");
        assert_eq!(r.stdout, "", "{args:?}");
        assert!(
            r.stderr.starts_with("einvoice-validate: ")
                && r.stderr
                    .ends_with("Try 'einvoice-validate --help' for more information.\n"),
            "{args:?}: {}",
            r.stderr
        );
    }
}

#[test]
fn unknown_rule_set_exits_with_2() {
    let r = run(&["--rules", "peppol", VALID], None);
    assert_eq!(r.code, 2);
    assert_eq!(r.stdout, "");
    assert!(
        r.stderr.contains("no built-in rule set named \"peppol\""),
        "{}",
        r.stderr
    );
}

#[test]
fn every_whole_document_is_valid() {
    let documents: Vec<String> = common::cases()
        .into_iter()
        .filter(|c| !c.id.starts_with("vefa/"))
        .map(|c| c.input)
        .collect();
    assert_eq!(documents.len(), 48);
    let args: Vec<&str> = documents.iter().map(String::as_str).collect();
    let r = run(&args, None);
    assert_eq!(r.code, 0, "{}", r.stdout);
    assert_eq!(r.stderr, "");
    let lines: Vec<&str> = r.stdout.lines().filter(|l| !l.starts_with(' ')).collect();
    assert_eq!(lines.len(), 49);
    for (line, document) in lines.iter().zip(&documents) {
        assert!(
            *line == format!("{document}: valid")
                || line.starts_with(&format!("{document}: valid: ")),
            "{line}"
        );
    }
    assert_eq!(
        lines[48],
        "48 documents: 48 valid, 0 invalid, 0 not validated"
    );
}

#[test]
fn a_failing_vefa_case_exits_with_1() {
    let case = common::case("vefa/Invoice-unit-UBL/BR-02.002.xml");
    assert_eq!(case.input, BR_02);
    let expected = common::golden_findings(&case.golden());
    let r = run(&[BR_02], None);
    assert_eq!(r.code, 1);
    assert_eq!(r.stderr, "");
    let mut lines = r.stdout.lines();
    assert_eq!(
        lines.next(),
        Some(format!("{BR_02}: invalid: 1 schema error, 11 fatal").as_str())
    );
    assert_eq!(
        lines.next(),
        Some(
            "  schema   2:1: missing-element: element ubl:Invoice: content incomplete; expected: ext:UBLExtensions, cbc:UBLVersionID, cbc:CustomizationID, cbc:ProfileID, cbc:ProfileExecutionID, cbc:ID"
        )
    );
    let ids: Vec<&str> = lines
        .filter(|l| !l.starts_with("           at "))
        .map(|l| {
            let rest = l.strip_prefix("  fatal    ").unwrap();
            rest.split_once(':').unwrap().0
        })
        .collect();
    let golden_ids: Vec<&str> = expected.iter().map(|f| f.1.as_deref().unwrap()).collect();
    assert_eq!(ids, golden_ids);
    assert!(ids.contains(&"BR-02"));
    assert!(r.stdout.contains(
        "  fatal    BR-02: [BR-02]-An Invoice shall have an Invoice number (BT-1).\n           at /*:Invoice[namespace-uri()='urn:oasis:names:specification:ubl:schema:xsd:Invoice-2'][1]\n"
    ));
}

#[test]
fn no_colour_without_a_terminal() {
    let r = common::run_env(&[BR_02], None, &[("NO_COLOR", "")]);
    assert!(!r.stdout.contains('\x1b'));
    let r = run(&[BR_02], None);
    assert!(!r.stdout.contains('\x1b'));
}

/// The JSON report of a valid document and two invalid ones (one with a warning), compared
/// with `tests/golden/report.json` (the tool version written `@VERSION@`). Regenerate
/// with `EINVOICE_BLESS=1`.
#[test]
fn json_golden() {
    let r = run(&["--format", "json", VALID, BR_02, CN_BR_10], None);
    assert_eq!(r.code, 1);
    assert_eq!(r.stderr, "");
    let report = json::parse(&r.stdout).unwrap();
    let ours = r.stdout.replace(
        &format!("\"version\": \"{}\"", common::VERSION),
        "\"version\": \"@VERSION@\"",
    );
    let golden = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/report.json");
    if std::env::var_os("EINVOICE_BLESS").is_some() {
        std::fs::write(&golden, &ours).unwrap();
    }
    assert_eq!(ours, std::fs::read_to_string(&golden).unwrap());

    assert_eq!(
        report.keys(),
        [
            "format",
            "format_version",
            "tool",
            "rule_set",
            "documents",
            "summary",
            "exit_code"
        ]
    );
    assert_eq!(report.get("format").str(), Some("einvoice-validate"));
    assert_eq!(report.get("format_version").number(), 1);
    assert_eq!(report.get("exit_code").number(), 1);
    let documents = report.get("documents").array();
    assert_eq!(documents[0].get("status").str(), Some("valid"));
    assert_eq!(documents[1].get("status").str(), Some("invalid"));
    assert_eq!(
        common::json_findings(&documents[1]),
        common::golden_findings(&common::case("vefa/Invoice-unit-UBL/BR-02.002.xml").golden())
    );
    let counts = documents[2].get("counts");
    assert_eq!(counts.get("warning").number(), 1);
    assert_eq!(counts.get("fatal").number(), 10);
}

#[test]
fn standard_input() {
    let r = run(&["-f", "json", "-"], Some(&read(BR_02)));
    assert_eq!(r.code, 1);
    let report = json::parse(&r.stdout).unwrap();
    let document = &report.get("documents").array()[0];
    assert_eq!(document.get("path").str(), Some("-"));
    assert_eq!(document.get("status").str(), Some("invalid"));
    assert!(
        common::json_findings(document)
            .iter()
            .any(|f| f.1.as_deref() == Some("BR-02"))
    );

    let r = run(&["-", VALID], Some(&read(VALID)));
    assert_eq!(r.code, 0, "{}", r.stdout);
    assert!(r.stdout.starts_with("-: valid"), "{}", r.stdout);

    let r = run(&["-"], Some(b"not xml"));
    assert_eq!(r.code, 1);
    assert!(
        r.stdout.starts_with("-: invalid: 1:1: not well-formed XML"),
        "{}",
        r.stdout
    );
}

#[test]
fn a_file_that_cannot_be_read_exits_with_2() {
    let r = run(&["no-such-file.xml"], None);
    assert_eq!(r.code, 2);
    assert!(
        r.stdout.starts_with("no-such-file.xml: error (io): "),
        "{}",
        r.stdout
    );

    // The other documents are still validated; the highest code wins.
    let r = run(&["-f", "json", BR_02, "no-such-file.xml", VALID], None);
    assert_eq!(r.code, 2);
    let report = json::parse(&r.stdout).unwrap();
    let statuses: Vec<_> = report
        .get("documents")
        .array()
        .iter()
        .map(|d| d.get("status").str().unwrap().to_owned())
        .collect();
    assert_eq!(statuses, ["invalid", "error", "valid"]);
    let error = report.get("documents").array()[1].get("error");
    assert_eq!(error.get("kind").str(), Some("io"));
    assert_eq!(error.get("line"), &Value::Null);
    assert_eq!(report.get("summary").get("errors").number(), 1);
    assert_eq!(report.get("exit_code").number(), 2);
}

#[test]
fn a_document_that_is_not_xml_is_invalid() {
    let dir = common::temp_dir("not-xml");
    let path = dir.join("broken.xml");
    std::fs::write(&path, "<Invoice>\n<a></b>\n</Invoice>\n").unwrap();
    let path = path.to_str().unwrap();
    let r = run(&["-f", "json", path], None);
    assert_eq!(r.code, 1);
    let report = json::parse(&r.stdout).unwrap();
    let document = &report.get("documents").array()[0];
    assert_eq!(document.get("status").str(), Some("invalid"));
    let error = document.get("error");
    assert_eq!(error.get("kind").str(), Some("not-xml"));
    assert_eq!(error.get("line").number(), 2);
    std::fs::remove_dir_all(dir).unwrap();
}

/// The official stylesheet stops on a non-numeric amount; the schema has already found
/// the document invalid, so the verdict stands. The line is that of the `xsl:when` whose
/// test raised the error, as Saxon-HE 12.10 reports it (its `xsl:choose` is on the line
/// before).
#[test]
fn a_stylesheet_error_on_a_schema_invalid_document_exits_with_1() {
    let text = String::from_utf8(read(VALID)).unwrap();
    let amount = r#"<cbc:TaxableAmount currencyID="EUR">183.23</cbc:TaxableAmount>"#;
    assert!(text.contains(amount));
    let text = text.replacen(
        amount,
        r#"<cbc:TaxableAmount currencyID="EUR">abc</cbc:TaxableAmount>"#,
        1,
    );
    let r = run(&["-"], Some(text.as_bytes()));
    assert_eq!(r.code, 1);
    assert_eq!(
        r.stdout,
        "-: invalid: 1 schema error, 1 stylesheet error\n  schema   81:13: invalid-value: element cbc:TaxableAmount: \"abc\" is not a valid value of xs:decimal\n  error    stylesheet EN16931-UBL stopped, line 2529: FORG0001: Cannot convert string \"abc\" to xs:decimal: invalid character 'a'\n"
    );
    let r = run(&["-f", "json", "-"], Some(text.as_bytes()));
    assert_eq!(r.code, 1);
    let report = json::parse(&r.stdout).unwrap();
    let document = &report.get("documents").array()[0];
    assert_eq!(document.get("status").str(), Some("invalid"));
    assert_eq!(document.get("counts").get("stylesheet_errors").number(), 1);
    let error = &document.get("stylesheet_errors").array()[0];
    assert_eq!(error.get("stylesheet").str(), Some("EN16931-UBL"));
    assert_eq!(error.get("code").str(), Some("FORG0001"));
    assert_eq!(error.get("line").number(), 2529);
    let r = run(&["-f", "svrl", "-"], Some(text.as_bytes()));
    assert_eq!(r.code, 1);
    assert_eq!(r.stdout, "");
    assert!(
        r.stderr
            .starts_with("-: stylesheet EN16931-UBL stopped: FORG0001: "),
        "{}",
        r.stderr
    );
}

#[test]
fn an_unsupported_document_exits_with_3() {
    let r = run(
        &["-"],
        Some(b"<!DOCTYPE Invoice [<!ENTITY e 'x'>]><Invoice>&e;</Invoice>"),
    );
    assert_eq!(r.code, 3);
    assert!(
        r.stdout
            .starts_with("-: error (unsupported): unsupported: a document type declaration"),
        "{}",
        r.stdout
    );
    let r = run(
        &["-"],
        Some(b"<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><Invoice/>"),
    );
    assert_eq!(r.code, 3);
}

#[test]
fn svrl_output() {
    let r = run(&["--format", "svrl", BR_02], None);
    assert_eq!(r.code, 1);
    assert_eq!(r.stderr, "");
    assert!(r.stdout.starts_with("<?xml"), "{}", &r.stdout[..100]);
    // The document URI is the file's, as Saxon-HE gives it: the canonical form is the
    // golden.
    let path = std::path::absolute(common::join(&common::workspace_root(), BR_02)).unwrap();
    let path = path.to_str().unwrap();
    let canonical =
        einvoice_svrl::canonicalize(&r.stdout, &|u| einvoice_svrl::is_document_uri(u, path))
            .unwrap();
    assert_eq!(
        canonical,
        common::case("vefa/Invoice-unit-UBL/BR-02.002.xml").golden()
    );

    // Problems go to standard error.
    let r = run(&["--format", "svrl", "no-such-file.xml"], None);
    assert_eq!(r.code, 2);
    assert_eq!(r.stdout, "");
    assert!(
        r.stderr.starts_with("no-such-file.xml: error (io): "),
        "{}",
        r.stderr
    );
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    use std::fmt::Write as _;
    sha2::Sha256::digest(bytes)
        .iter()
        .fold(String::new(), |mut hex, b| {
            let _ = write!(hex, "{b:02x}");
            hex
        })
}

#[test]
fn external_bundles() {
    let root = common::workspace_root();
    let xslt = std::fs::read(common::join(&root, common::XSLT)).unwrap();
    let manifest = std::fs::read_to_string(common::join(
        &root,
        "crates/einvoice-validator/bundles/cen-ubl/bundle.toml",
    ))
    .unwrap();
    let dir = common::temp_dir("bundle");
    let xslt_path = common::join(&dir, "ubl/xslt/EN16931-UBL-validation.xslt");
    std::fs::create_dir_all(xslt_path.parent().unwrap()).unwrap();
    std::fs::write(&xslt_path, &xslt).unwrap();
    std::fs::write(dir.join("bundle.toml"), &manifest).unwrap();
    let bundle = dir.to_str().unwrap();

    let builtin = run(&["-f", "json", BR_02], None);
    let external = run(&["-f", "json", "--bundle", bundle, BR_02], None);
    assert_eq!(external.code, 1, "{}", external.stderr);
    assert_eq!(external.stdout, builtin.stdout);

    // A stylesheet that is not the one the manifest records.
    let mut tampered = xslt.clone();
    tampered.extend_from_slice(b"\n");
    std::fs::write(&xslt_path, &tampered).unwrap();
    let r = run(&["--bundle", bundle, BR_02], None);
    assert_eq!(r.code, 2);
    assert_eq!(r.stdout, "");
    assert!(r.stderr.contains("SHA-256"), "{}", r.stderr);

    // A stylesheet outside the supported subset.
    let import = br#"<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform"><xsl:import href="x.xsl"/></xsl:stylesheet>"#;
    std::fs::write(&xslt_path, import).unwrap();
    std::fs::write(
        dir.join("bundle.toml"),
        manifest.replace(&sha256(&xslt), &sha256(import)),
    )
    .unwrap();
    let r = run(&["--bundle", bundle, BR_02], None);
    assert_eq!(r.code, 3, "{}", r.stderr);
    assert!(r.stderr.contains("xsl:import"), "{}", r.stderr);

    // No bundle there.
    let r = run(&["--bundle", &format!("{bundle}/nothing"), BR_02], None);
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("bundle.toml"), "{}", r.stderr);

    // A stylesheet that stops on a document without schema errors: no verdict.
    let failing = br#"<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:xs="http://www.w3.org/2001/XMLSchema">
<xsl:template match="/">
<xsl:value-of select="xs:integer(/a)"/>
</xsl:template>
</xsl:stylesheet>"#;
    std::fs::write(dir.join("f.xsl"), failing).unwrap();
    std::fs::write(
        dir.join("bundle.toml"),
        format!(
            "format = 1\nname = \"t\"\ntitle = \"T\"\nversion = \"1\"\n[[stylesheet]]\nname = \"f\"\npath = \"f.xsl\"\nsha256 = \"{}\"\n",
            sha256(failing)
        ),
    )
    .unwrap();
    let r = run(&["--bundle", bundle, "-"], Some(b"<a>x</a>"));
    assert_eq!(r.code, 3, "{}", r.stderr);
    assert_eq!(
        r.stdout,
        "-: error (transform): stylesheet f, line 3: FORG0001: Cannot convert string \"x\" to an integer\n"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

/// Every case of the corpus in one run, on several threads: the status and the findings
/// (kind, id, flag, location, in order) of each document agree with the Saxon-HE
/// golden, and the output is in the order of the command line.
#[test]
fn the_whole_corpus_agrees_with_the_saxon_goldens() {
    let cases = common::cases();
    assert_eq!(cases.len(), 1179);
    let inputs: Vec<&str> = cases.iter().map(|c| c.input.as_str()).collect();
    let runs = common::run_json_batches(&["--format", "json", "--jobs", "4"], &inputs);
    assert_eq!(runs.stderr(), "");
    assert_eq!(runs.code(), 1);
    let documents = runs.documents();
    assert_eq!(documents.len(), cases.len());
    let mut problems = Vec::new();
    let (mut valid, mut invalid) = (0, 0);
    for (case, document) in cases.iter().zip(documents) {
        assert_eq!(document.get("path").str(), Some(case.input.as_str()));
        let expected = common::golden_findings(&case.golden());
        let fatal = expected.iter().any(|f| f.2.as_deref() == Some("fatal"));
        let schema_errors = document.get("counts").get("schema_errors").number();
        let expected_status = if fatal || schema_errors > 0 {
            "invalid"
        } else {
            "valid"
        };
        let status = document.get("status").str().unwrap();
        if status != expected_status {
            problems.push(format!("{}: {status}, expected {expected_status}", case.id));
        }
        if common::json_findings(document) != expected {
            problems.push(format!("{}: findings differ", case.id));
        }
        valid += usize::from(status == "valid");
        invalid += usize::from(status == "invalid");
    }
    eprintln!(
        "{} documents in {} runs: {valid} valid, {invalid} invalid, {} disagreements",
        cases.len(),
        runs.runs.len(),
        problems.len()
    );
    assert_eq!(problems, Vec::<String>::new());
    assert_eq!(valid + invalid, 1179);
    assert_eq!(runs.summary("documents"), 1179);
    assert_eq!(runs.summary("valid"), u64::try_from(valid).unwrap());
    assert_eq!(runs.summary("errors"), 0);
}

#[test]
fn the_built_in_rule_set_agrees_with_the_saxon_goldens_on_the_mutation_cases() {
    let cases = common::mutation_cases(common::CORPUS, "");
    assert_eq!(cases.len(), 504);
    let inputs: Vec<&str> = cases.iter().map(|c| c.input.as_str()).collect();
    let runs = common::run_json_batches(&["--format", "json", "--jobs", "4"], &inputs);
    let documents = runs.documents();
    assert_eq!(documents.len(), cases.len());
    let golden = format!("{}/golden", common::CORPUS);
    let mut problems = Vec::new();
    for (case, document) in cases.iter().zip(documents) {
        problems.extend(common::check_mutation_document(
            &case.id,
            document,
            &[("EN16931-UBL", golden.as_str())],
        ));
    }
    eprintln!(
        "{} mutation documents in {} runs, {} disagreements",
        cases.len(),
        runs.runs.len(),
        problems.len()
    );
    for p in &problems {
        eprintln!("DISAGREE {p}");
    }
    assert_eq!(problems, Vec::<String>::new());
}
