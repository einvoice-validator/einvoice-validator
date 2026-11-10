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
    BUILTIN_RULE_SETS, Clock, Error, ErrorKind, FindingKind, MANIFEST_FILE, NoScenario, RuleSet,
    SchemaId, Severity, ValidateOptions, Validator, Verdict, file_uri,
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
    assert_eq!(rule_set.schema(), Some(&SchemaId::CiiD16b));
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
    // A minimal CII document: the schema wants the mandatory elements; the CEN rules
    // report what is missing.
    let report = validator
        .validate(
            br#"<rsm:CrossIndustryInvoice xmlns:rsm="urn:un:unece:uncefact:data:standard:CrossIndustryInvoice:100"/>"#,
        )
        .unwrap();
    assert_eq!(report.verdict(), Verdict::Invalid);
    let kinds: Vec<&str> = report
        .schema_errors
        .iter()
        .map(|e| e.kind.as_str())
        .collect();
    assert_eq!(kinds, ["missing-element"]);
    assert_eq!(report.rule_set.name, "cen-cii");
    assert_eq!(report.rule_set.schema, Some(SchemaId::CiiD16b));
    assert!(report.findings().any(|f| f.id.as_deref() == Some("BR-01")));
    // An official example is valid for the schema and the rules.
    let example = std::fs::read(common::join(
        &common::workspace_root(),
        "artefacts/cen-en16931-cii-1.3.17/cii/examples/CII_example3.xml",
    ))
    .unwrap();
    let report = validator.validate(&example).unwrap();
    assert_eq!(report.schema_errors, []);
    assert_eq!(report.verdict(), Verdict::Valid);
    // A value only the schema checks: the document is invalid for the schema alone.
    let text = String::from_utf8(example).unwrap();
    assert_eq!(
        text.matches("<udt:Indicator>true</udt:Indicator>").count(),
        1
    );
    let edited = text.replace(
        "<udt:Indicator>true</udt:Indicator>",
        "<udt:Indicator>yes</udt:Indicator>",
    );
    let report = validator.validate(edited.as_bytes()).unwrap();
    assert_eq!(report.verdict(), Verdict::Invalid);
    let errors: Vec<(&str, u32)> = report
        .schema_errors
        .iter()
        .map(|e| (e.kind.as_str(), e.line))
        .collect();
    assert_eq!(errors, [("invalid-value", 121)]);
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
    assert_eq!(rule_set.schema(), Some(&SchemaId::Ubl21));
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
    assert!(
        stylesheet
            .svrl()
            .to_text()
            .contains("svrl:schematron-output")
    );
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
    let cases: [(&[u8], ErrorKind, &str); 9] = [
        (b"<Invoice", ErrorKind::NotXml, "1:1: not well-formed XML"),
        (b"<a>\n<b></a>", ErrorKind::NotXml, "2:"),
        (b"<p:a/>", ErrorKind::NotXml, "not well-formed XML"),
        (
            b"<a>\xFF</a>",
            ErrorKind::NotXml,
            "1:4: not well-formed XML: bytes that are not UTF-8 at 1:4",
        ),
        (
            b"\xFF\xFE<\0a\0>\0\0\xD8<\0/\0a\0>\0",
            ErrorKind::NotXml,
            "1:4: not well-formed XML: bytes that are not UTF-16LE at 1:4",
        ),
        (
            b"\xFF\xFE<\0?\0x\0m\0l\0 \0v\0e\0r\0s\0i\0o\0n\0=\0'\x001\0.\x000\0'\0 \0e\0n\0c\0o\0d\0i\0n\0g\0=\0'\0U\0T\0F\0-\08\0'\0?\0>\0<\0a\0/\0>\0",
            ErrorKind::NotXml,
            "1:1: not well-formed XML: a UTF-16LE byte order mark, but the XML declaration names the encoding 'UTF-8'",
        ),
        (
            b"<?xml version='1.0' encoding='Shift_JIS'?><a/>",
            ErrorKind::Unsupported,
            "the Shift_JIS encoding",
        ),
        (
            b"\xEF\xBB\xBF\xEF\xBB\xBF<a/>",
            ErrorKind::NotXml,
            "1:1: not well-formed XML: unknown token at 1:1",
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
    // XML 1.0 [80], [81]: an encoding name starts with a letter. Not an encoding the
    // validator refuses as unsupported: a declaration that is not well-formed, as for
    // Saxon-HE and xmllint.
    let declaration = "encoding=\"UTF-8\"";
    assert_eq!(text.matches(declaration).count(), 1);
    let e = cen()
        .validate(
            text.replacen(declaration, "encoding=\"-UTF-8\"", 1)
                .as_bytes(),
        )
        .unwrap_err();
    assert_eq!(e.kind(), ErrorKind::NotXml, "{e}");
    assert!(
        e.to_string()
            .ends_with("the encoding is not an encoding name"),
        "{e}"
    );
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

/// The schema of a bundle (`[[schema-document]]`): an entry point in a directory of the
/// bundle, which imports a second document by a location relative to its own.
const BUNDLE_SCHEMA_MAIN: &str = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:t="urn:t" xmlns:u="urn:u" targetNamespace="urn:t" elementFormDefault="qualified">
  <xs:import namespace="urn:u" schemaLocation="types.xsd"/>
  <xs:element name="doc" type="t:DocType"/>
  <xs:complexType name="DocType">
    <xs:sequence><xs:element ref="u:n"/></xs:sequence>
    <xs:attribute name="ok" type="xs:string"/>
  </xs:complexType>
</xs:schema>"#;
const BUNDLE_SCHEMA_TYPES: &str = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" targetNamespace="urn:u" elementFormDefault="qualified">
  <xs:element name="n" type="xs:decimal"/>
</xs:schema>"#;

/// A format 2 manifest with one stylesheet and the schema of the bundle,
/// `xsd/main.xsd` and `xsd/types.xsd` (whose content is `types`).
fn manifest_with_schema(sheet: &str, types: &str) -> String {
    format!(
        "format = 2\nname = \"test\"\ntitle = \"Test rules\"\nversion = \"0.1\"\nschema = \"xsd/main.xsd\"\n\n[[stylesheet]]\nname = \"s\"\npath = \"s.xsl\"\nsha256 = \"{}\"\n\n[[schema-document]]\npath = \"xsd/main.xsd\"\nsha256 = \"{}\"\n\n[[schema-document]]\npath = \"xsd/types.xsd\"\nsha256 = \"{}\"\n",
        sha256(sheet.as_bytes()),
        sha256(BUNDLE_SCHEMA_MAIN.as_bytes()),
        sha256(types.as_bytes())
    )
}

#[test]
fn a_bundle_validates_against_its_own_schema() {
    let sheet = svrl_stylesheet("T-1", "fatal");
    let manifest = manifest_with_schema(&sheet, BUNDLE_SCHEMA_TYPES);
    let files: &[(&str, &[u8])] = &[
        ("s.xsl", sheet.as_bytes()),
        ("xsd/main.xsd", BUNDLE_SCHEMA_MAIN.as_bytes()),
        ("xsd/types.xsd", BUNDLE_SCHEMA_TYPES.as_bytes()),
    ];
    let in_memory = Validator::from_bundle(&manifest, files).unwrap();
    let from_dir =
        Validator::from_bundle_dir(bundle_dir("bundle-schema", &manifest, files)).unwrap();
    let bundle = SchemaId::Bundle("xsd/main.xsd".to_owned());
    for validator in [&in_memory, &from_dir] {
        assert_eq!(validator.rule_set().schema(), Some(&bundle));
        let paths: Vec<&str> = validator
            .rule_set()
            .schema_documents()
            .iter()
            .map(einvoice_validator::SchemaDocumentEntry::path)
            .collect();
        assert_eq!(paths, ["xsd/main.xsd", "xsd/types.xsd"]);
        // Valid for the schema and the stylesheet.
        let report = validator
            .validate(b"<t:doc xmlns:t='urn:t' xmlns:u='urn:u' ok='1'><u:n> 1.50 </u:n></t:doc>")
            .unwrap();
        assert_eq!(report.schema_errors, []);
        assert_eq!(report.verdict(), Verdict::Valid);
        assert_eq!(report.rule_set.schema, Some(bundle.clone()));
        // A value of the imported type that is not a decimal: a schema error, the
        // document invalid, and the stylesheet runs all the same.
        let report = validator
            .validate(b"<t:doc xmlns:t='urn:t' xmlns:u='urn:u'><u:n>x</u:n></t:doc>")
            .unwrap();
        let kinds: Vec<&str> = report
            .schema_errors
            .iter()
            .map(|e| e.kind.as_str())
            .collect();
        assert_eq!(kinds, ["invalid-value"]);
        assert_eq!(report.count(Severity::Fatal), 1);
        assert_eq!(report.verdict(), Verdict::Invalid);
        // A root element the schema does not declare.
        let report = validator.validate(b"<doc ok='1'/>").unwrap();
        assert_eq!(report.schema_errors[0].kind.as_str(), "unknown-root");
        assert_eq!(report.verdict(), Verdict::Invalid);
    }

    // A schema document missing from the bundle.
    let e = Validator::from_bundle(&manifest, &files[..2]).unwrap_err();
    assert_eq!(e.kind(), ErrorKind::Bundle);
    assert!(
        e.to_string()
            .contains("schema document xsd/types.xsd: file missing from the bundle"),
        "{e}"
    );
    let e = Validator::from_bundle_dir(bundle_dir("bundle-schema-missing", &manifest, &files[..2]))
        .unwrap_err();
    assert_eq!(e.kind(), ErrorKind::Io, "{e}");
    // One that is not the file the manifest names.
    let tampered = BUNDLE_SCHEMA_TYPES.replace("xs:decimal", "xs:string");
    let mut changed = files.to_vec();
    changed[2] = ("xsd/types.xsd", tampered.as_bytes());
    match Validator::from_bundle(&manifest, &changed).unwrap_err() {
        Error::Checksum {
            path,
            expected,
            actual,
        } => {
            assert_eq!(path, "xsd/types.xsd");
            assert_eq!(expected, sha256(BUNDLE_SCHEMA_TYPES.as_bytes()));
            assert_eq!(actual, sha256(tampered.as_bytes()));
        }
        other => panic!("{other:?}"),
    }
    // A schema outside the subset of einvoice-xsd is refused when the bundle loads
    // (an anonymous simple type).
    let anonymous = BUNDLE_SCHEMA_TYPES.replace(
        r#"<xs:element name="n" type="xs:decimal"/>"#,
        r#"<xs:element name="n"><xs:simpleType><xs:restriction base="xs:decimal"/></xs:simpleType></xs:element>"#,
    );
    let e = Validator::from_bundle(
        &manifest_with_schema(&sheet, &anonymous),
        &[files[0], files[1], ("xsd/types.xsd", anonymous.as_bytes())],
    )
    .unwrap_err();
    assert_eq!(e.kind(), ErrorKind::Unsupported, "{e}");
    assert!(e.to_string().contains("schema xsd/main.xsd"), "{e}");
    // A schema that does not load (an import of a document the bundle does not have)
    // is a broken bundle.
    let elsewhere = BUNDLE_SCHEMA_MAIN.replace("types.xsd", "other.xsd");
    let manifest_elsewhere = manifest_with_schema(&sheet, BUNDLE_SCHEMA_TYPES).replace(
        &sha256(BUNDLE_SCHEMA_MAIN.as_bytes()),
        &sha256(elsewhere.as_bytes()),
    );
    let e = Validator::from_bundle(
        &manifest_elsewhere,
        &[files[0], ("xsd/main.xsd", elsewhere.as_bytes()), files[2]],
    )
    .unwrap_err();
    assert_eq!(e.kind(), ErrorKind::Bundle, "{e}");
    assert!(e.to_string().contains("schema xsd/main.xsd"), "{e}");
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
    assert_eq!(report.stylesheets[0].svrl(), "");
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

/// A stylesheet reporting, whatever the document, a failed assert without a flag (`U-1`),
/// one flagged `warning` (`W-1`) and a successful report without a flag (`R-1`).
const UNFLAGGED_STYLESHEET: &str = r#"<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:svrl="http://purl.oclc.org/dsdl/svrl">
  <xsl:template match="/">
    <svrl:schematron-output>
      <svrl:fired-rule context="/"/>
      <svrl:failed-assert id="U-1" location="/" test="u"><svrl:text>u</svrl:text></svrl:failed-assert>
      <svrl:failed-assert id="W-1" flag="warning" location="/" test="w"><svrl:text>w</svrl:text></svrl:failed-assert>
      <svrl:successful-report id="R-1" location="/" test="r"><svrl:text>r</svrl:text></svrl:successful-report>
    </svrl:schematron-output>
  </xsl:template>
</xsl:stylesheet>"#;

/// The severities of a manifest: an unflagged failed assert, and the levels of the
/// scenario the document matches, by its root element and its specification identifier.
#[test]
fn severities_of_the_manifest() {
    let manifest = |extra: &str| {
        format!(
            "format = 2\nname = \"u\"\ntitle = \"U\"\nversion = \"1\"\n\n[[stylesheet]]\nname = \"u\"\npath = \"u.xsl\"\nsha256 = \"{}\"\nunflagged-assert = \"fatal\"\n{extra}",
            sha256(UNFLAGGED_STYLESHEET.as_bytes())
        )
    };
    let scenario = "\n[[scenario]]\nname = \"CII X\"\nnamespace = \"urn:un:unece:uncefact:data:standard:CrossIndustryInvoice:100\"\nroot = \"CrossIndustryInvoice\"\nidentifier = \"urn:x\"\nother = \"U-1\"\nfatal = \"W-1 R-1\"\n";
    let files = [("u.xsl", UNFLAGGED_STYLESHEET.as_bytes())];
    let cii = |id: &str| {
        format!(
            "<rsm:CrossIndustryInvoice xmlns:rsm='urn:un:unece:uncefact:data:standard:CrossIndustryInvoice:100' xmlns:ram='urn:un:unece:uncefact:data:standard:ReusableAggregateBusinessInformationEntity:100'><rsm:ExchangedDocumentContext><ram:GuidelineSpecifiedDocumentContextParameter><ram:ID>{id}</ram:ID></ram:GuidelineSpecifiedDocumentContextParameter></rsm:ExchangedDocumentContext></rsm:CrossIndustryInvoice>"
        )
    };
    let severities = |report: &einvoice_validator::Report| -> Vec<Severity> {
        report.findings().map(|f| f.severity).collect()
    };
    // Unflagged failed asserts are fatal; an unflagged successful report is not.
    let validator = Validator::from_bundle(&manifest(""), &files).unwrap();
    assert_eq!(
        validator.rule_set().stylesheets()[0].unflagged_assert(),
        Severity::Fatal
    );
    let report = validator.validate(cii("urn:x").as_bytes()).unwrap();
    assert_eq!(
        severities(&report),
        [Severity::Fatal, Severity::Warning, Severity::Other]
    );
    assert_eq!(report.verdict(), Verdict::Invalid);
    assert_eq!(report.scenario, None);
    // The scenario of the document: its levels win over the flags.
    let validator = Validator::from_bundle(&manifest(scenario), &files).unwrap();
    let report = validator.validate(cii("urn:x").as_bytes()).unwrap();
    assert_eq!(report.scenario.as_deref(), Some("CII X"));
    assert_eq!(
        severities(&report),
        [Severity::Other, Severity::Fatal, Severity::Fatal]
    );
    // Another identifier, the identifier with a space, another root: no scenario.
    for document in [
        cii("urn:y"),
        cii("urn:x "),
        "<Invoice xmlns='urn:oasis:names:specification:ubl:schema:xsd:Invoice-2'/>".to_owned(),
    ] {
        let report = validator.validate(document.as_bytes()).unwrap();
        assert_eq!(report.scenario, None, "{document}");
        assert_eq!(
            severities(&report),
            [Severity::Fatal, Severity::Warning, Severity::Other],
            "{document}"
        );
    }
    // Failing closed leaves the warning a warning.
    let closed = validator
        .validate_with(
            cii("urn:y").as_bytes(),
            &ValidateOptions::new().fail_closed(true),
        )
        .unwrap();
    assert_eq!(
        severities(&closed),
        [Severity::Fatal, Severity::Warning, Severity::Fatal]
    );
}

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
            .svrl()
            .to_text()
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
    let shift_jis = b"<?xml version='1.0' encoding='Shift_JIS'?><codes/>";
    let files: &[(&str, &[u8])] = &[
        ("xslt/codes.xsl", CODES_STYLESHEET.as_bytes()),
        ("xslt/codes.xml", shift_jis),
    ];
    match Validator::from_bundle(&codes_manifest(shift_jis), files) {
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

/// A format 2 manifest of two stylesheets, `a` (failed assert `A-1` flagged `fatal`
/// unless the root has `ok`) and `b` (`B-1`, flagged `warning`), with `schema` and the
/// scenarios `scenarios`.
fn selecting_manifest(schema: &str, scenarios: &str) -> (String, [(&'static str, Vec<u8>); 2]) {
    let a = svrl_stylesheet("A-1", "fatal").into_bytes();
    let b = svrl_stylesheet("B-1", "warning").into_bytes();
    let manifest = format!(
        "format = 2\nname = \"sel\"\ntitle = \"Selection\"\nversion = \"1\"\n{schema}\n[[stylesheet]]\nname = \"a\"\npath = \"a.xsl\"\nsha256 = \"{}\"\n\n[[stylesheet]]\nname = \"b\"\npath = \"b.xsl\"\nsha256 = \"{}\"\n{scenarios}",
        sha256(&a),
        sha256(&b)
    );
    (manifest, [("a.xsl", a), ("b.xsl", b)])
}

/// The scenarios of [`selecting_manifest`], as a KoSIT configuration writes them: the
/// identifier of a UBL invoice, read as the string value of `cbc:CustomizationID` or as
/// its text node children, or an attribute that may not be an integer.
const SELECTING_SCENARIOS: &str = "
[[scenario]]
name = \"Both\"
namespaces = \"cbc=urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2 i=urn:oasis:names:specification:ubl:schema:xsd:Invoice-2\"
match = \"exists(/i:Invoice/cbc:CustomizationID[ . = 'urn:both'])\"
stylesheets = \"a b\"
other = \"A-1\"

[[scenario]]
name = \"B then A\"
namespaces = \"cbc=urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2 i=urn:oasis:names:specification:ubl:schema:xsd:Invoice-2\"
match = \"exists(/i:Invoice[cbc:CustomizationID/text() = 'urn:ba'])\"
stylesheets = \"b a\"

[[scenario]]
name = \"B alone\"
namespaces = \"xs=http://www.w3.org/2001/XMLSchema\"
match = \"xs:integer(/*/@n) = 1\"
stylesheets = \"b\"
";

fn ubl(ids: &[&str], ok: bool) -> String {
    let ids = ids.iter().fold(String::new(), |mut out, id| {
        let _ = write!(out, "<cbc:CustomizationID>{id}</cbc:CustomizationID>");
        out
    });
    format!(
        "<Invoice xmlns='urn:oasis:names:specification:ubl:schema:xsd:Invoice-2' xmlns:cbc='urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2'{}>{ids}</Invoice>",
        if ok { " ok='1'" } else { "" }
    )
}

/// A rule set that selects by scenario: the one scenario whose `match` expression is
/// true runs its stylesheets, in its order, with its levels; a document that matches
/// none, or several, is invalid and nothing runs on it, as the KoSIT validator rejects
/// it with its fallback scenario.
#[test]
fn scenarios_select_the_stylesheets() {
    let (manifest, files) = selecting_manifest("", SELECTING_SCENARIOS);
    let files: Vec<(&str, &[u8])> = files.iter().map(|(p, b)| (*p, b.as_slice())).collect();
    let validator = Validator::from_bundle(&manifest, &files).unwrap();
    assert!(validator.rule_set().selects_by_scenario());
    let run = |document: &str| {
        let report = validator.validate(document.as_bytes()).unwrap();
        let names: Vec<String> = report.stylesheets.iter().map(|s| s.name.clone()).collect();
        (report, names)
    };
    // Both stylesheets, A-1 made other by the scenario: valid.
    let (report, names) = run(&ubl(&["urn:both"], false));
    assert_eq!(report.scenario.as_deref(), Some("Both"));
    assert_eq!(report.no_scenario, None);
    assert_eq!(names, ["a", "b"]);
    let severities: Vec<Severity> = report.findings().map(|f| f.severity).collect();
    assert_eq!(severities, [Severity::Other, Severity::Warning]);
    assert_eq!(report.verdict(), Verdict::Valid);
    // Its own order, its flags: A-1 fatal.
    let (report, names) = run(&ubl(&["urn:ba"], false));
    assert_eq!(report.scenario.as_deref(), Some("B then A"));
    assert_eq!(names, ["b", "a"]);
    assert_eq!(report.verdict(), Verdict::Invalid);
    // A text node split by a comment is not the identifier; the string value is.
    let (report, _) = run(&ubl(&["urn:<!-- -->ba"], true));
    assert_eq!(report.no_scenario, Some(NoScenario::NoneMatches));
    let (report, _) = run(&ubl(&["urn:<!-- -->both"], false));
    assert_eq!(report.scenario.as_deref(), Some("Both"));
    // B alone, though A would reject the document.
    let (report, names) = run("<d n=' 1 '/>");
    assert_eq!(report.scenario.as_deref(), Some("B alone"));
    assert_eq!(names, ["b"]);
    assert_eq!(report.verdict(), Verdict::Valid);
    // A `match` that raises a dynamic error (FORG0001) is false, as for the KoSIT
    // validator: no scenario.
    for document in [
        "<d n='one'/>",
        "<d/>",
        &ubl(&["urn:other"], true),
        &ubl(&[], true),
    ] {
        let (report, names) = run(document);
        assert_eq!(report.scenario, None, "{document}");
        assert_eq!(
            report.no_scenario,
            Some(NoScenario::NoneMatches),
            "{document}"
        );
        assert_eq!(names, Vec::<String>::new());
        assert_eq!(report.findings().count(), 0);
        assert_eq!(report.verdict(), Verdict::Invalid, "{document}");
        assert!(!report.is_valid());
    }
    assert_eq!(
        NoScenario::NoneMatches.to_string(),
        "no scenario of the rule set matches the document"
    );
    // Two identifiers, two scenarios: none applies.
    let (report, names) = run(&ubl(&["urn:both", "urn:ba"], true));
    let several = NoScenario::SeveralMatch(vec!["Both".to_owned(), "B then A".to_owned()]);
    assert_eq!(report.no_scenario.as_ref(), Some(&several));
    assert_eq!(names, Vec::<String>::new());
    assert_eq!(report.verdict(), Verdict::Invalid);
    assert_eq!(
        several.to_string(),
        "2 scenarios of the rule set match the document, one is required: Both, B then A"
    );
    // Not XML: an error, before any scenario.
    assert!(matches!(
        validator.validate(b"<d"),
        Err(Error::NotXml { .. })
    ));
}

/// With a schema: a document that has a scenario keeps its schema errors; one that has
/// none gets none (the KoSIT validator validates against the schema of the scenario).
#[test]
fn a_document_without_a_scenario_has_no_schema_errors() {
    let (manifest, files) = selecting_manifest("schema = \"ubl-2.1\"\n", SELECTING_SCENARIOS);
    let files: Vec<(&str, &[u8])> = files.iter().map(|(p, b)| (*p, b.as_slice())).collect();
    let validator = Validator::from_bundle(&manifest, &files).unwrap();
    let matched = validator
        .validate(ubl(&["urn:both"], true).as_bytes())
        .unwrap();
    assert_eq!(matched.scenario.as_deref(), Some("Both"));
    assert_ne!(matched.schema_errors.len(), 0);
    assert_eq!(matched.verdict(), Verdict::Invalid);
    let unmatched = validator
        .validate(ubl(&["urn:x"], true).as_bytes())
        .unwrap();
    assert_eq!(unmatched.no_scenario, Some(NoScenario::NoneMatches));
    assert_eq!(unmatched.schema_errors, []);
    assert_eq!(unmatched.rule_set.schema, Some(SchemaId::Ubl21));
}

/// The schema a manifest names unchecked is in the report; a `match` expression the
/// engine cannot compile is refused when the bundle is loaded.
#[test]
fn unchecked_schemas_and_unsupported_matches() {
    let (manifest, files) = selecting_manifest(
        "unchecked-schema = \"Another schema (another.xsd)\"\n",
        SELECTING_SCENARIOS,
    );
    let files: Vec<(&str, &[u8])> = files.iter().map(|(p, b)| (*p, b.as_slice())).collect();
    let validator = Validator::from_bundle(&manifest, &files).unwrap();
    assert_eq!(
        validator.rule_set().unchecked_schema(),
        Some("Another schema (another.xsd)")
    );
    let report = validator.validate(b"<d n='1'/>").unwrap();
    assert_eq!(
        report.rule_set.unchecked_schema.as_deref(),
        Some("Another schema (another.xsd)")
    );
    assert_eq!(report.rule_set.schema, None);
    // A function the engine does not have.
    let unknown = manifest.replace("xs:integer(/*/@n) = 1", "lower-case(name(/*)) = 'd'");
    let e = Validator::from_bundle(&unknown, &files).unwrap_err();
    assert_eq!(e.kind(), ErrorKind::Unsupported, "{e}");
    assert!(e.to_string().contains("scenario \"B alone\": match"), "{e}");
}
