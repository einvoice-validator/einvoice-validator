//! The public API: built-in and external bundles, the stages of a validation, the
//! report, and the errors.

#![cfg(test)]
#![cfg_attr(
    not(feature = "cen-ubl"),
    expect(
        unused_imports,
        reason = "some imports serve the tests of the built-in rule set"
    )
)]

mod common;

use std::fmt::Write as _;
use std::path::Path;

use einvoice_validator::{
    BUILTIN_RULE_SETS, Error, ErrorKind, FindingKind, MANIFEST_FILE, RuleSet, SchemaId, Severity,
    ValidateOptions, Validator, Verdict, file_uri,
};
use sha2::{Digest, Sha256};

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .fold(String::new(), |mut hex, b| {
            let _ = write!(hex, "{b:02x}");
            hex
        })
}

/// A stylesheet writing an SVRL report with one failed assert, flagged `flag`, when the
/// document element has no `ok` attribute.
fn svrl_stylesheet(id: &str, flag: &str) -> String {
    format!(
        r#"<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:svrl="http://purl.oclc.org/dsdl/svrl">
  <xsl:template match="/">
    <svrl:schematron-output title="test">
      <svrl:active-pattern id="P-{id}" documents="{{document-uri(/)}}"/>
      <svrl:fired-rule context="/*"/>
      <xsl:if test="not(/*/@ok)">
        <svrl:failed-assert id="{id}" flag="{flag}" location="/*[1]" test="@ok">
          <svrl:text>[{id}] the document element needs ok</svrl:text>
        </svrl:failed-assert>
      </xsl:if>
    </svrl:schematron-output>
  </xsl:template>
</xsl:stylesheet>"#
    )
}

/// A manifest for `stylesheets` (name, path, content), without schema unless given.
fn manifest(schema: Option<&str>, stylesheets: &[(&str, &str, &[u8])]) -> String {
    let mut text =
        String::from("format = 1\nname = \"test\"\ntitle = \"Test rules\"\nversion = \"0.1\"\n");
    if let Some(schema) = schema {
        let _ = writeln!(text, "schema = \"{schema}\"");
    }
    for (name, path, content) in stylesheets {
        let _ = write!(
            text,
            "\n[[stylesheet]]\nname = \"{name}\"\npath = \"{path}\"\nsha256 = \"{}\"\n",
            sha256(content)
        );
    }
    text
}

#[cfg(feature = "cen-ubl")]
fn cen() -> &'static Validator {
    static CEN: std::sync::OnceLock<Validator> = std::sync::OnceLock::new();
    CEN.get_or_init(|| Validator::cen_ubl().unwrap())
}

#[test]
fn a_validator_can_be_shared_between_threads() {
    fn shared<T: Send + Sync>() {}
    shared::<Validator>();
    shared::<einvoice_validator::Report>();
    shared::<Error>();
}

#[cfg(feature = "cen-ubl")]
#[test]
fn the_builtin_rule_set() {
    assert_eq!(BUILTIN_RULE_SETS, ["cen-ubl"]);
    let rule_set = cen().rule_set();
    assert_eq!(rule_set, &RuleSet::builtin("cen-ubl").unwrap());
    assert_eq!(
        (rule_set.name(), rule_set.title(), rule_set.version()),
        ("cen-ubl", "CEN EN 16931 UBL", "1.3.17")
    );
    assert_eq!(rule_set.schema(), Some(SchemaId::Ubl21));
    let [stylesheet] = rule_set.stylesheets() else {
        panic!("one stylesheet")
    };
    assert_eq!(stylesheet.path(), common::XSLT);
    // The checksum of the manifest is the one of the vendored artefacts' SHA256SUMS.
    let sums = std::fs::read_to_string(common::join(
        &common::workspace_root(),
        &format!("{}/SHA256SUMS", common::ARTEFACTS),
    ))
    .unwrap();
    let line = format!("{}  {}", stylesheet.sha256(), common::XSLT);
    assert!(sums.lines().any(|l| l == line), "{line}");
    assert_eq!(Validator::builtin("cen-ubl").unwrap().rule_set(), rule_set);
}

#[test]
fn unknown_builtin_rule_sets_are_refused() {
    let e = Validator::builtin("peppol").unwrap_err();
    assert_eq!(e.kind(), ErrorKind::Bundle);
    assert!(
        e.to_string()
            .contains("no built-in rule set named \"peppol\""),
        "{e}"
    );
    assert_eq!(RuleSet::builtin("x").unwrap_err().kind(), ErrorKind::Bundle);
}

#[cfg(feature = "cen-ubl")]
#[test]
fn a_valid_example() {
    let case = common::case("examples/ubl-tc434-example1.xml");
    let bytes = std::fs::read(case.path()).unwrap();
    let report = cen().validate(&bytes).unwrap();
    assert_eq!(report.verdict(), Verdict::Valid);
    assert!(report.is_valid());
    assert_eq!(report.schema_errors, Vec::new());
    assert_eq!(report.count(Severity::Fatal), 0);
    assert_eq!(report.rule_set.name, "cen-ubl");
    assert_eq!(report.rule_set.version, "1.3.17");
    assert_eq!(report.rule_set.schema, Some(SchemaId::Ubl21));
    assert_eq!(report.document_uri, None);
    let [stylesheet] = report.stylesheets.as_slice() else {
        panic!("one stylesheet report")
    };
    assert_eq!(stylesheet.name, "EN16931-UBL");
    assert_eq!(stylesheet.version, "1.3.17");
    assert_eq!(
        stylesheet.sha256,
        cen().rule_set().stylesheets()[0].sha256()
    );
    assert!(stylesheet.svrl.contains("svrl:schematron-output"));
    assert_eq!(stylesheet.warnings, Vec::<String>::new());
    // Without a document URI, the SVRL names no document.
    let canonical = stylesheet.canonical_svrl().unwrap();
    assert!(canonical.starts_with("svrl-canonical 1\n"));
    assert!(
        canonical.contains("active-pattern documents=\"\""),
        "{canonical}"
    );
}

#[cfg(feature = "cen-ubl")]
#[test]
fn a_failing_vefa_case() {
    let case = common::case("vefa/Invoice-unit-UBL/BR-02.002.xml");
    assert_eq!(case.error, ["BR-02"]);
    let path = std::path::absolute(case.path()).unwrap();
    let uri = file_uri(&path).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let report = cen()
        .validate_with(&bytes, &ValidateOptions::new().document_uri(&uri))
        .unwrap();
    assert_eq!(report.verdict(), Verdict::Invalid);
    assert_eq!(report.document_uri.as_deref(), Some(uri.as_str()));
    let br02 = report
        .findings()
        .find(|f| f.id.as_deref() == Some("BR-02"))
        .unwrap();
    assert_eq!(br02.kind, FindingKind::FailedAssert);
    assert_eq!(br02.flag.as_deref(), Some("fatal"));
    assert_eq!(br02.severity, Severity::Fatal);
    assert_eq!(br02.test, "normalize-space(cbc:ID) != ''");
    assert_eq!(
        br02.context.as_deref(),
        Some("/ubl:Invoice | /cn:CreditNote")
    );
    assert_eq!(br02.pattern.as_deref(), Some("UBL-model"));
    assert_eq!(
        br02.text,
        "[BR-02]-An Invoice shall have an Invoice number (BT-1)."
    );
    assert_eq!(
        br02.location,
        "/*:Invoice[namespace-uri()='urn:oasis:names:specification:ubl:schema:xsd:Invoice-2'][1]"
    );
    // The golden, canonical, is what Saxon-HE reports.
    let golden = std::fs::read_to_string(case.golden()).unwrap();
    assert_eq!(report.stylesheets[0].canonical_svrl().unwrap(), golden);
    // The schema reports the missing mandatory elements, with their position.
    let [schema_error] = report.schema_errors.as_slice() else {
        panic!("{:?}", report.schema_errors)
    };
    assert_eq!(schema_error.kind.as_str(), "missing-element");
    assert_eq!((schema_error.line, schema_error.column), (2, 1));
}

#[cfg(feature = "cen-ubl")]
#[test]
fn documents_that_are_not_xml() {
    let cases: [(&[u8], ErrorKind, &str); 7] = [
        (b"<Invoice", ErrorKind::NotXml, "1:1: not well-formed XML"),
        (b"<a>\n<b></a>", ErrorKind::NotXml, "2:"),
        (b"<p:a/>", ErrorKind::NotXml, "not well-formed XML"),
        (
            b"<a>\xFF</a>",
            ErrorKind::NotXml,
            "1:4: not well-formed XML: not UTF-8",
        ),
        (b"\xFF\xFE<\0a\0/\0>\0", ErrorKind::Unsupported, "UTF-16"),
        (
            b"<?xml version='1.0' encoding='ISO-8859-1'?><a/>",
            ErrorKind::Unsupported,
            "the ISO-8859-1 encoding",
        ),
        (
            b"<!DOCTYPE a [<!ENTITY e 'x'>]><a>&e;</a>",
            ErrorKind::Unsupported,
            "document type declaration",
        ),
    ];
    for (bytes, kind, message) in cases {
        let e = cen().validate(bytes).unwrap_err();
        assert_eq!(e.kind(), kind, "{e}");
        assert!(e.to_string().contains(message), "{message:?} not in {e}");
    }
    match cen().validate(b"<a>\n<b></a>").unwrap_err() {
        Error::NotXml { line, column, .. } => assert_eq!((line, column), (2, 4)),
        other => panic!("{other:?}"),
    }
}

#[cfg(feature = "cen-ubl")]
#[test]
fn a_byte_order_mark_is_accepted() {
    let case = common::case("examples/ubl-tc434-example1.xml");
    let mut bytes = b"\xEF\xBB\xBF".to_vec();
    bytes.extend(std::fs::read(case.path()).unwrap());
    assert!(cen().validate(&bytes).unwrap().is_valid());
}

#[test]
fn a_bundle_from_memory_with_two_stylesheets() {
    let first = svrl_stylesheet("T-1", "fatal");
    let second = svrl_stylesheet("T-2", "warning");
    let both = manifest(
        None,
        &[
            ("first", "a/first.xsl", first.as_bytes()),
            ("second", "second.xsl", second.as_bytes()),
        ],
    );
    let validator = Validator::from_bundle(
        &both,
        &[
            ("second.xsl", second.as_bytes()),
            ("a/first.xsl", first.as_bytes()),
            ("unused.txt", b"ignored"),
        ],
    )
    .unwrap();
    assert_eq!(validator.rule_set().schema(), None);

    let report = validator
        .validate_with(
            b"<doc/>",
            &ValidateOptions::new().document_uri("file:/x/doc.xml"),
        )
        .unwrap();
    let ids: Vec<(&str, Severity)> = report
        .findings()
        .map(|f| (f.id.as_deref().unwrap(), f.severity))
        .collect();
    assert_eq!(ids, [("T-1", Severity::Fatal), ("T-2", Severity::Warning)]);
    assert_eq!(report.verdict(), Verdict::Invalid);
    assert_eq!(report.stylesheets[1].name, "second");
    assert_eq!(report.stylesheets[1].version, "0.1");
    assert_eq!(report.stylesheets[0].sha256, sha256(first.as_bytes()));
    assert_eq!(report.schema_errors, Vec::new(), "no schema in the bundle");
    let canonical = report.stylesheets[0].canonical_svrl().unwrap();
    assert!(
        canonical.contains("active-pattern documents=\"\" id=\"P-T-1\""),
        "{canonical}"
    );

    // Only warnings: valid.
    let report = Validator::from_bundle(
        &manifest(None, &[("second", "second.xsl", second.as_bytes())]),
        &[("second.xsl", second.as_bytes())],
    )
    .unwrap()
    .validate(b"<doc/>")
    .unwrap();
    assert_eq!(report.verdict(), Verdict::Valid);
    assert_eq!(report.count(Severity::Warning), 1);
    assert!(validator.validate(b"<doc ok='1'/>").unwrap().is_valid());
}

#[test]
fn a_bundle_with_the_ubl_schema() {
    let sheet = svrl_stylesheet("T-1", "fatal");
    let validator = Validator::from_bundle(
        &manifest(Some("ubl-2.1"), &[("s", "s.xsl", sheet.as_bytes())]),
        &[("s.xsl", sheet.as_bytes())],
    )
    .unwrap();
    let report = validator.validate(b"<doc ok='1'/>").unwrap();
    assert_eq!(report.count(Severity::Fatal), 0);
    assert_eq!(report.schema_errors[0].kind.as_str(), "unknown-root");
    assert_eq!(report.verdict(), Verdict::Invalid);
}

#[test]
fn bundle_errors() {
    let sheet = svrl_stylesheet("T-1", "fatal");
    let good = manifest(None, &[("s", "s.xsl", sheet.as_bytes())]);
    let files: &[(&str, &[u8])] = &[("s.xsl", sheet.as_bytes())];

    let e = Validator::from_bundle(&good, &[]).unwrap_err();
    assert_eq!(e.kind(), ErrorKind::Bundle);
    assert!(
        e.to_string().contains("file s.xsl missing from the bundle"),
        "{e}"
    );

    let e = Validator::from_bundle(&good.replace("format = 1", "format = 9"), files).unwrap_err();
    assert!(e.to_string().contains("bundle.toml:1: format 9"), "{e}");

    let tampered = sheet.replace("needs ok", "needs OK");
    match Validator::from_bundle(&good, &[("s.xsl", tampered.as_bytes())]).unwrap_err() {
        Error::Checksum {
            path,
            expected,
            actual,
        } => {
            assert_eq!(path, "s.xsl");
            assert_eq!(expected, sha256(sheet.as_bytes()));
            assert_eq!(actual, sha256(tampered.as_bytes()));
        }
        other => panic!("{other:?}"),
    }

    // A stylesheet outside the supported subset is refused when the bundle loads.
    let import = r#"<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform"><xsl:import href="x.xsl"/></xsl:stylesheet>"#;
    let e = Validator::from_bundle(
        &manifest(None, &[("i", "i.xsl", import.as_bytes())]),
        &[("i.xsl", import.as_bytes())],
    )
    .unwrap_err();
    assert_eq!(e.kind(), ErrorKind::Unsupported, "{e}");
    assert!(e.to_string().contains("stylesheet i (i.xsl)"), "{e}");
    assert!(e.to_string().contains("xsl:import"), "{e}");

    // So is a function the engine does not know.
    let function = r#"<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform"><xsl:template match="/"><xsl:value-of select="unparsed-text('x')"/></xsl:template></xsl:stylesheet>"#;
    let e = Validator::from_bundle(
        &manifest(None, &[("f", "f.xsl", function.as_bytes())]),
        &[("f.xsl", function.as_bytes())],
    )
    .unwrap_err();
    assert_eq!(e.kind(), ErrorKind::Unsupported, "{e}");

    // A stylesheet that is not well-formed is a broken bundle.
    let broken = b"<xsl:stylesheet";
    let e = Validator::from_bundle(
        &manifest(None, &[("b", "b.xsl", broken)]),
        &[("b.xsl", broken)],
    )
    .unwrap_err();
    assert_eq!(e.kind(), ErrorKind::Bundle, "{e}");
}

#[test]
fn errors_while_validating() {
    let load = |xslt: &str| {
        Validator::from_bundle(
            &manifest(None, &[("s", "s.xsl", xslt.as_bytes())]),
            &[("s.xsl", xslt.as_bytes())],
        )
        .unwrap()
    };
    // A dynamic error of the stylesheet, with its code and line.
    let failing = load(
        r#"<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:xs="http://www.w3.org/2001/XMLSchema">
<xsl:template match="/">
<xsl:value-of select="xs:integer(/a)"/>
</xsl:template>
</xsl:stylesheet>"#,
    );
    match failing.validate(b"<a>x</a>").unwrap_err() {
        Error::Transform {
            stylesheet,
            code,
            line,
            ..
        } => {
            assert_eq!(stylesheet, "s");
            assert_eq!(code, "FORG0001");
            assert_eq!(line, Some(3));
        }
        other => panic!("{other:?}"),
    }
    // A stylesheet whose result is not SVRL.
    let not_svrl = load(
        r#"<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform"><xsl:template match="/"><r/></xsl:template></xsl:stylesheet>"#,
    );
    let e = not_svrl.validate(b"<a/>").unwrap_err();
    assert_eq!(e.kind(), ErrorKind::Bundle);
    assert!(e.to_string().contains("not an SVRL report"), "{e}");
}

/// When the document has schema errors, a stylesheet that stops on it does not hide
/// them: the report is returned, invalid, and says which stylesheet stopped.
#[test]
fn a_stylesheet_error_on_a_schema_invalid_document() {
    let failing = r#"<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:xs="http://www.w3.org/2001/XMLSchema">
<xsl:template match="/">
<xsl:value-of select="xs:integer(/a)"/>
</xsl:template>
</xsl:stylesheet>"#;
    let fine = svrl_stylesheet("T-1", "warning");
    let validator = Validator::from_bundle(
        &manifest(
            Some("ubl-2.1"),
            &[
                ("failing", "f.xsl", failing.as_bytes()),
                ("fine", "s.xsl", fine.as_bytes()),
            ],
        ),
        &[("f.xsl", failing.as_bytes()), ("s.xsl", fine.as_bytes())],
    )
    .unwrap();
    let report = validator.validate(b"<a>x</a>").unwrap();
    assert_eq!(report.verdict(), Verdict::Invalid);
    assert!(!report.is_complete());
    assert_eq!(report.schema_errors[0].kind.as_str(), "unknown-root");
    let failure = report.stylesheets[0].failure.as_ref().unwrap();
    assert_eq!((failure.code.as_str(), failure.line), ("FORG0001", Some(3)));
    assert_eq!(report.stylesheets[0].svrl, "");
    assert_eq!(
        report.stylesheets[0].canonical_svrl().unwrap_err().kind(),
        ErrorKind::Transform
    );
    // The other stylesheets still run.
    assert_eq!(report.stylesheets[1].failure, None);
    assert_eq!(report.count(Severity::Warning), 1);
}

#[cfg(feature = "cen-ubl")]
#[test]
fn a_non_numeric_amount() {
    let case = common::case("examples/ubl-tc434-example1.xml");
    let text = std::fs::read_to_string(case.path()).unwrap();
    let amount = r#"<cbc:TaxableAmount currencyID="EUR">183.23</cbc:TaxableAmount>"#;
    assert!(text.contains(amount));
    let text = text.replacen(
        amount,
        r#"<cbc:TaxableAmount currencyID="EUR">abc</cbc:TaxableAmount>"#,
        1,
    );
    let report = cen().validate(text.as_bytes()).unwrap();
    assert_eq!(report.verdict(), Verdict::Invalid);
    let [schema_error] = report.schema_errors.as_slice() else {
        panic!("{:?}", report.schema_errors)
    };
    assert_eq!(schema_error.kind.as_str(), "invalid-value");
    let failure = report.stylesheets[0].failure.as_ref().unwrap();
    assert_eq!(failure.code, "FORG0001");
    assert!(failure.message.contains("\"abc\""), "{}", failure.message);
}

/// Copies `files` (path in the bundle, content) and the manifest into a fresh directory.
fn bundle_dir(name: &str, manifest: &str, files: &[(&str, &[u8])]) -> std::path::PathBuf {
    let dir = common::temp_dir(name);
    std::fs::write(dir.join(MANIFEST_FILE), manifest).unwrap();
    for (path, content) in files {
        let target = common::join(&dir, path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, content).unwrap();
    }
    dir
}

#[cfg(feature = "cen-ubl")]
#[test]
fn the_cen_bundle_from_a_directory() {
    let root = common::workspace_root();
    let xslt = std::fs::read(common::join(
        &root,
        &format!("{}/{}", common::ARTEFACTS, common::XSLT),
    ))
    .unwrap();
    let manifest =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("bundles/cen-ubl.toml"))
            .unwrap();
    let dir = bundle_dir("cen", &manifest, &[(common::XSLT, &xslt)]);
    let validator = Validator::from_bundle_dir(&dir).unwrap();
    assert_eq!(validator.rule_set(), cen().rule_set());
    let case = common::case("vefa/Invoice-unit-UBL/BR-02.002.xml");
    let bytes = std::fs::read(case.path()).unwrap();
    assert_eq!(
        validator.validate(&bytes).unwrap(),
        cen().validate(&bytes).unwrap()
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn bundle_directory_errors() {
    let missing = common::temp_dir("missing").join("nothing");
    let e = Validator::from_bundle_dir(&missing).unwrap_err();
    match &e {
        Error::Io { path, source } => {
            assert_eq!(path, &missing.join(MANIFEST_FILE));
            assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(e.kind(), ErrorKind::Io);

    let sheet = svrl_stylesheet("T-1", "fatal");
    let manifest = manifest(None, &[("s", "sub/s.xsl", sheet.as_bytes())]);
    let dir = bundle_dir("no-stylesheet", &manifest, &[]);
    let e = Validator::from_bundle_dir(&dir).unwrap_err();
    assert_eq!(e.kind(), ErrorKind::Io, "{e}");
    assert!(e.to_string().contains("s.xsl"), "{e}");
    std::fs::remove_dir_all(dir).unwrap();

    let dir = bundle_dir("good", &manifest, &[("sub/s.xsl", sheet.as_bytes())]);
    let report = Validator::from_bundle_dir(&dir)
        .unwrap()
        .validate(b"<a/>")
        .unwrap();
    assert_eq!(report.count(Severity::Fatal), 1);
    std::fs::remove_dir_all(dir).unwrap();

    let dir = bundle_dir("escape", &manifest.replace("sub/s.xsl", "../s.xsl"), &[]);
    let e = Validator::from_bundle_dir(&dir).unwrap_err();
    assert_eq!(e.kind(), ErrorKind::Bundle, "{e}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(feature = "cen-ubl")]
#[test]
fn one_validator_on_many_threads() {
    let cases: Vec<common::Case> = common::cases().into_iter().step_by(25).collect();
    let documents: Vec<Vec<u8>> = cases
        .iter()
        .map(|c| std::fs::read(c.path()).unwrap())
        .collect();
    let sequential: Vec<_> = documents
        .iter()
        .map(|d| cen().validate(d).unwrap())
        .collect();
    let parallel: Vec<_> = std::thread::scope(|scope| {
        let handles: Vec<_> = documents
            .iter()
            .map(|d| scope.spawn(move || cen().validate(d).unwrap()))
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert_eq!(sequential, parallel);
}
