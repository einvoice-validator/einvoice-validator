//! What the Saxon probes cannot reach: refusals when preparing, the options of a
//! transformation, `document()` through a resolver.

// Integration tests are only ever built with `cfg(test)`; saying so lets clippy apply its
// test exemptions (unwrap, expect, panic) to the helpers outside `#[test]` functions too.
#![cfg(test)]

use einvoice_xpath::QName;
use einvoice_xpath_eval::Item;
use einvoice_xslt_exec::{DocumentResolver, Executable, PrepareErrors, TransformOptions};

const HEAD: &str = r#"<xsl:stylesheet version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:f="urn:f">"#;
/// The namespaces the literal result elements copy from [`HEAD`].
const NS: &str = r#" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:f="urn:f""#;

fn prepare(body: &str) -> Result<Executable, PrepareErrors> {
    let xslt = format!("{HEAD}\n{body}\n</xsl:stylesheet>");
    Executable::new(&einvoice_xslt::load(&xslt).unwrap_or_else(|e| panic!("{e}")))
}

/// Prepares `body`, which must fail, and returns the error messages.
fn messages(body: &str) -> Vec<String> {
    match prepare(body) {
        Ok(_) => panic!("prepared: {body}"),
        Err(errors) => errors.0.iter().map(ToString::to_string).collect(),
    }
}

#[test]
fn node_construction_in_a_sequence_is_refused() {
    let errors = messages(
        r#"<xsl:function name="f:a" as="xs:string"><xsl:param name="p"/><xsl:value-of select="$p"/></xsl:function>
<xsl:function name="f:b"><e/></xsl:function>
<xsl:function name="f:c"><xsl:apply-templates select="."/></xsl:function>
<xsl:template match="/"><xsl:variable name="v" as="xs:string*"><xsl:text>t</xsl:text><xsl:comment/></xsl:variable></xsl:template>"#,
    );
    assert_eq!(errors.len(), 5, "{errors:#?}");
    for (error, what) in errors.iter().zip([
        "line 2: xsl:value-of",
        "line 3: a literal result element",
        "line 4: xsl:apply-templates",
        "line 5: text",
        "line 5: xsl:comment",
    ]) {
        assert!(error.starts_with(what), "{error}");
        assert!(error.contains("in a sequence"), "{error}");
    }
}

#[test]
fn static_errors_are_all_reported() {
    let errors = messages(
        r#"<xsl:template match="/"><xsl:value-of select="$nope, f:nope(1)"/><xsl:call-template name="missing"/><xsl:call-template name="t"><xsl:with-param name="q" select="1"/></xsl:call-template></xsl:template>
<xsl:template name="t"><xsl:param name="p"/></xsl:template>
<xsl:template match="(a | b)[1]" mode="m"/>
<xsl:template match="/"><xsl:value-of select="system-property('xsl:version')"/></xsl:template>"#,
    );
    let all = errors.join("\n");
    assert_eq!(errors.len(), 5, "{all}");
    assert!(all.contains("XPST0008"), "{all}");
    assert!(all.contains("XTSE0650"), "{all}");
    assert!(all.contains("XTSE0680"), "{all}");
    assert!(all.contains("may depend on the position"), "{all}");
    assert!(
        all.contains("system-property() with a prefixed name"),
        "{all}"
    );
}

#[test]
fn an_executable_runs_many_documents() {
    let executable = prepare(r#"<xsl:output omit-xml-declaration="yes"/><xsl:template match="/"><n><xsl:value-of select="count(//*)"/></n></xsl:template>"#).unwrap();
    for (input, count) in [
        ("<a/>", 1),
        ("<a><b/><b/></a>", 3),
        ("<a><b><c/></b></a>", 3),
    ] {
        assert_eq!(
            executable.transform(input, None).unwrap().output,
            format!("<n{NS}>{count}</n>")
        );
    }
    let error = executable.transform("<a>", None).unwrap_err();
    assert_eq!(error.code(), "SXXP0003");
    let error = executable
        .transform("<!DOCTYPE a [<!ENTITY e 'x'>]><a>&e;</a>", None)
        .unwrap_err();
    assert_eq!(error.code(), "SXXP0003");
}

#[test]
fn options_set_parameters_timezone_and_clock() {
    let executable = prepare(
        r#"<xsl:output omit-xml-declaration="yes"/><xsl:param name="p" as="xs:integer" select="1"/><xsl:param name="q"/>
<xsl:template match="/"><r p="{$p * 2}" q="{$q}" now="{current-dateTime()}"/></xsl:template>"#,
    )
    .unwrap();
    let now = einvoice_xdm::AtomicValue::String("2026-01-02T03:04:05Z".to_owned())
        .cast(einvoice_xdm::AtomicType::DateTime)
        .unwrap();
    let einvoice_xdm::AtomicValue::DateTime(now) = now else {
        panic!("{now:?}")
    };
    let options = TransformOptions {
        implicit_timezone: Some(60),
        current_date_time: Some(now),
        parameters: vec![
            (
                QName::local("p"),
                vec![Item::Atomic(einvoice_xdm::AtomicValue::UntypedAtomic(
                    "21".to_owned(),
                ))],
            ),
            (
                QName::local("q"),
                vec![Item::Atomic(einvoice_xdm::AtomicValue::String(
                    "v".to_owned(),
                ))],
            ),
        ],
        resolver: None,
    };
    let result = executable.transform_with("<a/>", None, &options).unwrap();
    assert_eq!(
        result.output,
        format!(r#"<r{NS} p="42" q="v" now="2026-01-02T03:04:05Z"/>"#)
    );
    let defaults = executable.transform("<a/>", None).unwrap();
    assert!(
        defaults
            .output
            .starts_with(&format!(r#"<r{NS} p="2" q="" now=""#)),
        "{}",
        defaults.output
    );
    let bad = TransformOptions {
        parameters: vec![(
            QName::local("p"),
            vec![Item::Atomic(einvoice_xdm::AtomicValue::String(
                "x".to_owned(),
            ))],
        )],
        ..TransformOptions::default()
    };
    assert_eq!(
        executable
            .transform_with("<a/>", None, &bad)
            .unwrap_err()
            .code(),
        "XTTE0590"
    );
}

struct Resolver;

impl DocumentResolver for Resolver {
    fn resolve(&self, uri: &str) -> Option<String> {
        (uri == "urn:test:dir/other.xml").then(|| "<o><i/><i/></o>".to_owned())
    }
}

#[test]
fn documents_come_from_the_resolver() {
    let executable = prepare(
        r#"<xsl:output omit-xml-declaration="yes"/><xsl:template match="/"><r><xsl:value-of select="count(document('other.xml')//i), count(document('other.xml') | document('urn:test:dir/other.xml'))"/></r></xsl:template>"#,
    )
    .unwrap()
    .with_base_uri("urn:test:dir/sheet.xsl");
    let options = TransformOptions {
        resolver: Some(&Resolver),
        ..TransformOptions::default()
    };
    let result = executable
        .transform_with("<a/>", Some("urn:test:dir/source.xml"), &options)
        .unwrap();
    assert_eq!(result.output, format!("<r{NS}>2 1</r>"));
    let error = executable
        .transform("<a/>", Some("urn:test:dir/source.xml"))
        .unwrap_err();
    assert_eq!(error.code(), "FODC0002", "{error}");
}

#[test]
fn the_source_can_be_given_as_a_string() {
    // document() asks for the source document again from a template body, a function, an
    // attribute value template, a global variable and a match pattern: the source given
    // as an owned String is kept for it, and the result is the same as from a &str.
    let executable = prepare(
        r#"<xsl:output omit-xml-declaration="yes"/>
<xsl:variable name="g" select="count(document('s.xml')//b)"/>
<xsl:function name="f:n"><xsl:param name="u"/><xsl:sequence select="count(document($u)//b)"/></xsl:function>
<xsl:template match="/"><r n="{count(document(base-uri(/))//b)}"><xsl:value-of select="$g, f:n(base-uri(/)), count(document(base-uri(/)) | /)"/><xsl:apply-templates select="a"/></r></xsl:template>
<xsl:template match="a[count(document(base-uri(/))//b) = 2]"> two</xsl:template>"#,
    )
    .unwrap()
    .with_base_uri("urn:test:dir/sheet.xsl");
    let source = "<a><b/><b/></a>";
    let uri = Some("urn:test:dir/s.xml");
    let expected = format!("<r{NS} n=\"2\">2 2 2 two</r>");
    let borrowed = executable.transform(source, uri).unwrap();
    assert_eq!(borrowed.output, expected);
    assert_eq!(
        executable.transform(source.to_owned(), uri).unwrap(),
        borrowed
    );
    for (i, body) in [
        r#"<xsl:template match="/"><r><xsl:value-of select="count(document(base-uri(/))//b)"/></r></xsl:template>"#,
        r#"<xsl:function name="f:n"><xsl:param name="u"/><xsl:sequence select="count(document($u)//b)"/></xsl:function><xsl:template match="/"><r><xsl:value-of select="f:n(base-uri(/))"/></r></xsl:template>"#,
        r#"<xsl:template match="/"><r><xsl:attribute name="n" select="count(document(base-uri(/))//b)"/></r></xsl:template>"#,
        r#"<xsl:template match="/"><r><xsl:apply-templates select="a"/></r></xsl:template><xsl:template match="a[document(base-uri(/))//b]">2</xsl:template>"#,
        r#"<xsl:template match="/"><r><xsl:apply-templates select="a"/></r></xsl:template><xsl:template match="(a|z)[document(base-uri(/))//b]">2</xsl:template>"#,
    ]
    .into_iter()
    .enumerate()
    {
        let executable = prepare(body).unwrap();
        let borrowed = executable.transform(source, uri).unwrap();
        assert!(borrowed.output.contains('2'), "{i}: {}", borrowed.output);
        assert_eq!(
            executable.transform(source.to_owned(), uri).unwrap(),
            borrowed,
            "{i}"
        );
    }
    // Without document(), or without a URI to load the source by, nothing is kept.
    let executable = prepare(
        r#"<xsl:template match="/"><r><xsl:value-of select="count(//b)"/></r></xsl:template>"#,
    )
    .unwrap();
    assert_eq!(
        executable.transform(source.to_owned(), uri).unwrap(),
        executable.transform(source, uri).unwrap()
    );
}

#[test]
fn ambiguous_matches_are_warnings() {
    let executable = prepare(
        r#"<xsl:output omit-xml-declaration="yes"/><xsl:template match="/"><r><xsl:apply-templates select="a/b"/></r></xsl:template>
<xsl:template match="b">1</xsl:template>
<xsl:template match="a/*">2</xsl:template>
<xsl:template match="*[1]">3</xsl:template>"#,
    )
    .unwrap();
    let result = executable.transform("<a><b/></a>", None).unwrap();
    assert_eq!(result.output, format!("<r{NS}>3</r>"));
    assert_eq!(result.warnings.len(), 1, "{:?}", result.warnings);
    assert!(result.warnings[0].starts_with("XTDE0540: Ambiguous rule match for /a/b[1]"));
}

#[test]
fn errors_in_patterns_are_warnings_reported_once() {
    // Line 4: the rule's pattern fails on both b (xs:integer('x')); they get the next rule.
    let executable = prepare(
        r#"<xsl:output omit-xml-declaration="yes"/>
<xsl:template match="/"><r><xsl:apply-templates select="a/b"/></r></xsl:template>
<xsl:template match="b[xs:integer(@n) gt 0]" priority="2">1</xsl:template>
<xsl:template match="b">2</xsl:template>"#,
    )
    .unwrap();
    let result = executable
        .transform("<a><b n='x'/><b n='x'/></a>", None)
        .unwrap();
    assert_eq!(result.output, format!("<r{NS}>22</r>"));
    assert_eq!(result.warnings.len(), 1, "{:?}", result.warnings);
    assert!(
        result.warnings[0].starts_with(
            "line 4: FORG0001: An error occurred matching pattern {b[xs:integer(@n) gt 0]}: "
        ),
        "{:?}",
        result.warnings
    );
}

#[test]
fn a_test_error_names_the_line_of_its_xsl_when() {
    let executable = prepare(
        r#"<xsl:template match="/">
<xsl:choose>
<xsl:when test="false()"/>
<xsl:when test="xs:integer('x')"/>
</xsl:choose>
</xsl:template>"#,
    )
    .unwrap();
    let error = executable.transform("<a/>", None).unwrap_err();
    assert_eq!(error.code(), "FORG0001");
    assert_eq!(error.line(), Some(5));
}

#[test]
fn errors_name_the_instruction_line() {
    let executable = prepare(
        r#"<xsl:template match="/">
<r>
<xsl:value-of select="xs:integer('x')"/>
</r>
</xsl:template>"#,
    )
    .unwrap();
    let error = executable.transform("<a/>", None).unwrap_err();
    assert_eq!(error.code(), "FORG0001");
    assert_eq!(error.line(), Some(4));
}

#[test]
fn an_executable_can_be_shared_between_threads() {
    fn shared<T: Send + Sync>() {}
    shared::<Executable>();
}
