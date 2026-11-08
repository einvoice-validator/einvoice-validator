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
    BUILTIN_RULE_SETS, Clock, Error, ErrorKind, FindingKind, MANIFEST_FILE, RuleSet, SchemaId,
    Severity, ValidateOptions, Validator, Verdict, file_uri,
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

#[test]
fn the_builtin_rule_sets() {
    let mut expected = Vec::new();
    if cfg!(feature = "cen-ubl") {
        expected.push("cen-ubl");
    }
    if cfg!(feature = "cen-cii") {
        expected.push("cen-cii");
    }
    assert_eq!(BUILTIN_RULE_SETS, expected);
}

#[cfg(feature = "cen-cii")]
#[test]
fn the_builtin_cii_rule_set() {
    let validator = Validator::cen_cii().unwrap();
    let rule_set = validator.rule_set();
    assert_eq!(rule_set, &RuleSet::builtin("cen-cii").unwrap());
    assert_eq!(
        (rule_set.name(), rule_set.title(), rule_set.version()),
        ("cen-cii", "CEN EN 16931 CII", "1.3.17")
    );
    assert_eq!(rule_set.schema(), None);
    let [stylesheet] = rule_set.stylesheets() else {
        panic!("one stylesheet")
    };
    assert_eq!(stylesheet.path(), "cii/xslt/EN16931-CII-validation.xslt");
    let sums = std::fs::read_to_string(common::join(
        &common::workspace_root(),
        "artefacts/cen-en16931-cii-1.3.17/SHA256SUMS",
    ))
    .unwrap();
    let line = format!("{}  {}", stylesheet.sha256(), stylesheet.path());
    assert!(sums.lines().any(|l| l == line), "{line}");
    // A minimal CII document: no schema, so only the CEN rules report what is missing.
    let report = validator
        .validate(
            br#"<rsm:CrossIndustryInvoice xmlns:rsm="urn:un:unece:uncefact:data:standard:CrossIndustryInvoice:100"/>"#,
        )
        .unwrap();
    assert_eq!(report.verdict(), Verdict::Invalid);
    assert_eq!(report.schema_errors, []);
    assert_eq!(report.rule_set.name, "cen-cii");
    assert_eq!(report.rule_set.schema, None);
    assert!(report.findings().any(|f| f.id.as_deref() == Some("BR-01")));
}

#[cfg(feature = "cen-ubl")]
#[test]
fn the_builtin_rule_set() {
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

/// Constructs that XML 1.0 or Namespaces in XML 1.0 forbid, which the `roxmltree` crate
/// accepted (until 0.2.0 the documents were checked with it): each makes the document not
/// well-formed, with the message of `einvoice-xml`, in a valid invoice.
#[cfg(feature = "cen-ubl")]
#[test]
fn constructs_xml_forbids_are_not_xml() {
    let case = common::case("examples/ubl-tc434-example1.xml");
    let text = std::fs::read_to_string(case.path()).unwrap();
    assert!(cen().validate(text.as_bytes()).unwrap().is_valid());
    let note = "<cbc:Note>";
    assert_eq!(text.matches(note).count(), 1);
    let cases = [
        // XML 1.0 section 4.1 (WFC: Legal Character): a surrogate is not a character.
        ("<cbc:Note>&#xD800;", "the character reference at"),
        ("<cbc:Note>&#x110000;", "the character reference at"),
        // XML 1.0 section 2.6: targets matching [Xx][Mm][Ll] are reserved; Namespaces
        // in XML 1.0 section 7: no colon in a processing instruction target.
        (
            "<?XmL x?><cbc:Note>",
            "the processing instruction target at",
        ),
        (
            "<?a:b x?><cbc:Note>",
            "the processing instruction target at",
        ),
        // XML 1.0 [16]: whitespace between the target and the data.
        (
            "<?abc?def?><cbc:Note>",
            "expected a whitespace between the processing instruction target",
        ),
        // Namespaces in XML 1.0 section 3: a prefix cannot be undeclared (in 1.0), nor
        // the xmlns prefix declared.
        (
            "<cbc:Note xmlns:p=''>",
            "is bound to an empty namespace name",
        ),
        (
            "<cbc:Note xmlns:xmlns='urn:x'>",
            "the 'xmlns' prefix is declared at",
        ),
        // XML 1.0 [5], Namespaces in XML 1.0 [7]: no qualified name starts with a colon.
        ("<cbc:Note :a='1'>", "a name with an empty prefix at"),
        // XML 1.0 section 3.1 (WFC: Unique Att Spec): the default namespace twice.
        (
            "<cbc:Note xmlns='urn:a' xmlns='urn:b'>",
            "namespace declaration 'xmlns' at",
        ),
        // Namespaces in XML 1.0 section 3: p:xmlns is not a namespace declaration, and
        // was taken for one.
        (
            "<cbc:Note xmlns:p='urn:p' p:xmlns='urn:x'>",
            "a prefixed attribute named 'xmlns' is not supported",
        ),
    ];
    for (replacement, message) in cases {
        let document = text.replacen(note, replacement, 1);
        let e = cen().validate(document.as_bytes()).unwrap_err();
        assert_eq!(e.kind(), ErrorKind::NotXml, "{replacement}: {e}");
        assert!(e.to_string().contains(message), "{replacement}: {e}");
        assert_eq!(
            cen()
                .validate_owned(document.into_bytes())
                .unwrap_err()
                .to_string(),
            e.to_string()
        );
    }
    // XML 1.0 [80], [81]: an encoding name starts with a letter. The validator refuses
    // encodings other than UTF-8 before reading the document.
    let declaration = "encoding=\"UTF-8\"";
    assert_eq!(text.matches(declaration).count(), 1);
    let e = cen()
        .validate(
            text.replacen(declaration, "encoding=\"-UTF-8\"", 1)
                .as_bytes(),
        )
        .unwrap_err();
    assert_eq!(e.kind(), ErrorKind::Unsupported, "{e}");
}

#[cfg(feature = "cen-ubl")]
#[test]
fn a_byte_order_mark_is_accepted() {
    let case = common::case("examples/ubl-tc434-example1.xml");
    let mut bytes = b"\xEF\xBB\xBF".to_vec();
    bytes.extend(std::fs::read(case.path()).unwrap());
    let report = cen().validate(&bytes).unwrap();
    assert!(report.is_valid());
    assert_eq!(cen().validate_owned(bytes).unwrap(), report);
}

#[test]
fn a_document_given_as_a_buffer() {
    // Two stylesheets: the first borrows the text, the last is given it; same reports as
    // from borrowed bytes, errors included.
    let first = svrl_stylesheet("T-1", "fatal");
    let second = svrl_stylesheet("T-2", "warning");
    let validator = Validator::from_bundle(
        &manifest(
            None,
            &[
                ("first", "first.xsl", first.as_bytes()),
                ("second", "second.xsl", second.as_bytes()),
            ],
        ),
        &[
            ("first.xsl", first.as_bytes()),
            ("second.xsl", second.as_bytes()),
        ],
    )
    .unwrap();
    let options = ValidateOptions::new().document_uri("file:/x/doc.xml");
    for document in [
        &b"<doc/>"[..],
        b"\xEF\xBB\xBF<doc ok='1'/>",
        b"<doc>",
        b"\xFF\xFE<\0d\0/\0>\0",
        b"<doc>\xFF</doc>",
        b"<!DOCTYPE doc><doc/>",
    ] {
        let borrowed = validator.validate_with(document, &options);
        let owned = validator.validate_owned_with(document.to_vec(), &options);
        match (&borrowed, &owned) {
            (Ok(a), Ok(b)) => assert_eq!(a, b, "{document:?}"),
            (Err(a), Err(b)) => assert_eq!(format!("{a:?}"), format!("{b:?}"), "{document:?}"),
            _ => panic!("{document:?}: {borrowed:?} / {owned:?}"),
        }
        assert_eq!(
            validator.validate(document).map_err(|e| format!("{e:?}")),
            validator
                .validate_owned(document.to_vec())
                .map_err(|e| format!("{e:?}")),
            "{document:?}"
        );
    }
}

#[test]
fn the_clock_is_an_option() {
    // A stylesheet that writes the current date and time, and whether the implicit
    // timezone is +02:00.
    let xslt = r#"<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:svrl="http://purl.oclc.org/dsdl/svrl">
  <xsl:template match="/">
    <svrl:schematron-output title="clock">
      <svrl:active-pattern id="P" documents=""/>
      <svrl:fired-rule context="/*"/>
      <svrl:successful-report id="NOW" location="/*[1]" test="true()">
        <svrl:text><xsl:value-of select="current-dateTime(), xs:date('2026-09-10') = xs:date('2026-09-10+02:00')"/></svrl:text>
      </svrl:successful-report>
    </svrl:schematron-output>
  </xsl:template>
</xsl:stylesheet>"#;
    let validator = Validator::from_bundle(
        &manifest(None, &[("clock", "clock.xsl", xslt.as_bytes())]),
        &[("clock.xsl", xslt.as_bytes())],
    )
    .unwrap();
    let clock = Clock::from_unix_millis(120, 1_789_000_123_456).unwrap();
    let text = |report: einvoice_validator::Report| {
        report.findings().next().map(|f| f.text.clone()).unwrap()
    };
    let options = ValidateOptions::new().clock(clock);
    assert_eq!(
        text(validator.validate_with(b"<a/>", &options).unwrap()),
        "2026-09-10T02:28:43.456+02:00 true"
    );
    assert_eq!(
        text(
            validator
                .validate_owned_with(b"<a/>".to_vec(), &options)
                .unwrap()
        ),
        "2026-09-10T02:28:43.456+02:00 true"
    );
    // Without a clock: the system clock, in UTC.
    let now = text(validator.validate(b"<a/>").unwrap());
    assert!(now.ends_with("Z false"), "{now}");
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
    // The vendored files of the built-in rule set are a bundle directory.
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("bundles/cen-ubl");
    let validator = Validator::from_bundle_dir(&dir).unwrap();
    assert_eq!(validator.rule_set(), cen().rule_set());
    let case = common::case("vefa/Invoice-unit-UBL/BR-02.002.xml");
    let bytes = std::fs::read(case.path()).unwrap();
    assert_eq!(
        validator.validate(&bytes).unwrap(),
        cen().validate(&bytes).unwrap()
    );
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

/// A stylesheet reading the code list `codes.xml` next to it with `document()`, then the
/// document `other` (absent from the bundle) when the document element has `other`: one
/// failed assert, unflagged, when the document element's `code` is not in the list.
const CODES_STYLESHEET: &str = r#"<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:svrl="http://purl.oclc.org/dsdl/svrl">
  <xsl:template match="/">
    <svrl:schematron-output title="codes">
      <svrl:active-pattern id="P" documents="{document-uri(/)}"/>
      <svrl:fired-rule context="/*"/>
      <svrl:fired-rule context="/*"/>
      <xsl:if test="not(/*/@code = document('codes.xml')//code)">
        <svrl:failed-assert id="C-1" location="/*[1]" test="@code = codes">
          <svrl:text>[C-1] <xsl:value-of select="count(document('codes.xml')//code)"/> codes</svrl:text>
        </svrl:failed-assert>
      </xsl:if>
      <xsl:if test="/*/@other"><xsl:value-of select="count(document(/*/@other)/*)"/></xsl:if>
    </svrl:schematron-output>
  </xsl:template>
</xsl:stylesheet>"#;

const CODES: &str = "<codes><code>A</code><code>B</code></codes>";

/// A format 2 manifest with the stylesheet at `xslt/codes.xsl` and its code list.
fn codes_manifest(codes: &[u8]) -> String {
    format!(
        "format = 2\nname = \"codes\"\ntitle = \"Codes\"\nversion = \"1\"\n\n[[stylesheet]]\nname = \"codes\"\npath = \"xslt/codes.xsl\"\nsha256 = \"{}\"\n\n[[resource]]\npath = \"xslt/codes.xml\"\nsha256 = \"{}\"\n",
        sha256(CODES_STYLESHEET.as_bytes()),
        sha256(codes)
    )
}

fn check_codes(validator: &Validator) {
    let report = validator.validate(b"<d code='A'/>").unwrap();
    assert_eq!(report.findings().count(), 0);
    assert_eq!(report.stylesheets[0].fired_rules, 2);
    let report = validator.validate(b"<d code='Z'/>").unwrap();
    let [finding] = report.findings().collect::<Vec<_>>()[..] else {
        panic!("{report:?}")
    };
    assert_eq!(finding.text, "[C-1] 2 codes");
    // Unflagged: the document conforms, unless the validation fails closed.
    assert_eq!(finding.severity, Severity::Other);
    assert_eq!(report.verdict(), Verdict::Valid);
    let closed = validator
        .validate_with(b"<d code='Z'/>", &ValidateOptions::new().fail_closed(true))
        .unwrap();
    assert_eq!(closed.findings().next().unwrap().severity, Severity::Fatal);
    assert_eq!(closed.verdict(), Verdict::Invalid);
    // Nothing but the bundle's resources: another file is not retrieved.
    for other in [
        "other.xml",
        "/etc/hostname",
        "../codes.xml",
        "xslt/codes.xml",
    ] {
        match validator.validate(format!("<d code='A' other='{other}'/>").as_bytes()) {
            Err(Error::Transform { code, .. }) => assert_eq!(code, "FODC0002", "{other}"),
            other => panic!("{other:?}"),
        }
    }
    // A URI that resolves to a resource, relative to the stylesheet's.
    let report = validator
        .validate(b"<d code='A' other='x/../codes.xml'/>")
        .unwrap();
    assert!(
        report.stylesheets[0]
            .svrl
            .ends_with("1</svrl:schematron-output>")
    );
}

#[test]
fn stylesheets_read_the_resources_of_their_bundle() {
    let manifest = codes_manifest(CODES.as_bytes());
    let files: &[(&str, &[u8])] = &[
        ("xslt/codes.xsl", CODES_STYLESHEET.as_bytes()),
        ("xslt/codes.xml", CODES.as_bytes()),
    ];
    let validator = Validator::from_bundle(&manifest, files).unwrap();
    assert_eq!(validator.rule_set().resources()[0].path(), "xslt/codes.xml");
    check_codes(&validator);
    // From a directory.
    let dir = common::temp_dir("resources");
    std::fs::create_dir_all(dir.join("xslt")).unwrap();
    std::fs::write(dir.join(MANIFEST_FILE), &manifest).unwrap();
    std::fs::write(dir.join("xslt/codes.xsl"), CODES_STYLESHEET).unwrap();
    std::fs::write(dir.join("xslt/codes.xml"), CODES).unwrap();
    check_codes(&Validator::from_bundle_dir(&dir).unwrap());
    // A clone shares the rule set, on another thread.
    let clone = validator.clone();
    std::thread::spawn(move || check_codes(&clone))
        .join()
        .unwrap();
    // A resource is checked when the bundle is loaded.
    let changed = "<codes><code>A</code></codes>";
    let files: &[(&str, &[u8])] = &[
        ("xslt/codes.xsl", CODES_STYLESHEET.as_bytes()),
        ("xslt/codes.xml", changed.as_bytes()),
    ];
    match Validator::from_bundle(&manifest, files) {
        Err(Error::Checksum { path, .. }) => assert_eq!(path, "xslt/codes.xml"),
        other => panic!("{other:?}"),
    }
    let files: &[(&str, &[u8])] = &[("xslt/codes.xsl", CODES_STYLESHEET.as_bytes())];
    match Validator::from_bundle(&manifest, files) {
        Err(Error::Bundle { message }) => assert!(message.contains("xslt/codes.xml"), "{message}"),
        other => panic!("{other:?}"),
    }
    let latin1 = b"<?xml version='1.0' encoding='ISO-8859-1'?><codes/>";
    let files: &[(&str, &[u8])] = &[
        ("xslt/codes.xsl", CODES_STYLESHEET.as_bytes()),
        ("xslt/codes.xml", latin1),
    ];
    match Validator::from_bundle(&codes_manifest(latin1), files) {
        Err(Error::Bundle { message }) => {
            assert!(message.contains("resource xslt/codes.xml"), "{message}");
        }
        other => panic!("{other:?}"),
    }
    // A resource deeper than the maximum depth is refused when it is read.
    let deep = format!("{}{}", "<c>".repeat(10), "</c>".repeat(10));
    let files: &[(&str, &[u8])] = &[
        ("xslt/codes.xsl", CODES_STYLESHEET.as_bytes()),
        ("xslt/codes.xml", deep.as_bytes()),
    ];
    let validator = Validator::from_bundle(&codes_manifest(deep.as_bytes()), files).unwrap();
    assert!(validator.validate(b"<d code='A'/>").is_ok());
    match validator.validate_with(b"<d code='A'/>", &ValidateOptions::new().max_depth(5)) {
        Err(Error::Limit { .. }) => {}
        other => panic!("{other:?}"),
    }
}
