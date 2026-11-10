//! The loader on synthetic stylesheets: every supported construct, and the refusal of
//! everything else with a message naming the construct and its line.

// Integration tests are only ever built with `cfg(test)`; saying so lets clippy apply its
// test exemptions (unwrap, expect, panic) to the helpers outside `#[test]` functions too.
#![cfg(test)]

use einvoice_xpath::{Expr, PathPattern, PathStep, PatternAlternative, QName};
use einvoice_xslt::{
    ApplyMode, GlobalKind, Instruction, LoadErrorKind, Mode, NumberLevel, OutputMethod, Standalone,
    TemplateModes, TextOrigin, VariableValue, load,
};

const HEAD: &str = r#"<xsl:transform version="2.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform"
    xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:u="urn:u" xmlns:r="urn:report">"#;

fn stylesheet(body: &str) -> String {
    format!("{HEAD}\n{body}\n</xsl:transform>")
}

/// A stylesheet with every construct the official stylesheets use, in the style of a
/// `SchXslt`-compiled schema (written for this test).
const FULL: &str = r##"
<xsl:output indent="yes" method="xml" omit-xml-declaration="no" standalone="yes"/>
<xsl:param name="initial" as="xs:anyURI?" select="()"/>
<xsl:variable name="flag" as="xs:boolean" select="true()"/>
<xsl:function name="u:slack" as="xs:boolean">
  <xsl:param name="exp" as="xs:decimal"/>
  <xsl:param name="val" as="xs:decimal"/>
  <xsl:variable name="d" as="xs:decimal" select="$exp - $val"/>
  <xsl:choose>
    <xsl:when test="$d lt 0"><xsl:sequence select="-$d le 0.02"/></xsl:when>
    <xsl:otherwise><xsl:sequence select="$d le 0.02"/></xsl:otherwise>
  </xsl:choose>
</xsl:function>
<xsl:function name="u:location" as="xs:string">
  <xsl:param name="node" as="node()"/>
  <xsl:variable name="steps" as="xs:string*">
    <xsl:for-each select="$node/ancestor-or-self::node()">
      <xsl:variable name="pos"><xsl:number level="single"/></xsl:variable>
      <xsl:if test=". instance of element()"><xsl:value-of select="concat(name(), '[', $pos, ']')"/></xsl:if>
    </xsl:for-each>
  </xsl:variable>
  <xsl:value-of select="$steps" separator="/"/>
</xsl:function>
<xsl:template name="validate">
  <xsl:apply-templates select="document($initial)"/>
</xsl:template>
<xsl:template match="root()">
  <xsl:variable name="report" as="element(r:report)">
    <r:report><xsl:apply-templates select="." mode="p1"/></r:report>
  </xsl:variable>
  <svrl:output xmlns:svrl="http://purl.oclc.org/dsdl/svrl" title="T">
    <xsl:comment>generated</xsl:comment>
    <xsl:sequence select="$report/node()"/>
  </svrl:output>
</xsl:template>
<xsl:template match="text() | @*" mode="#all" priority="-10"/>
<xsl:template match="*" mode="#all" priority="-10">
  <xsl:apply-templates select="@* | node()" mode="#current"/>
</xsl:template>
<xsl:template match="(/r:a | /r:b)[$flag]" mode="p1" priority="3">
  <r:rule pattern="{u:location(.)}" fixed="{{literal}}"/>
  <xsl:next-match>
    <xsl:with-param name="fired" as="xs:string*" select="('x', 'y')"/>
  </xsl:next-match>
</xsl:template>
<xsl:template match="r:a/r:c[1]" mode="p1 #default" priority="2.5">
  <xsl:param name="fired" as="xs:string*" select="()"/>
  <xsl:if test="not(u:slack(1, 2))">
    <xsl:attribute name="flag" select="'fatal'"/>
    <xsl:attribute name="id">R1</xsl:attribute>
    <xsl:text>  kept  </xsl:text>
  </xsl:if>
  <xsl:call-template name="validate"/>
</xsl:template>
"##;

fn full() -> einvoice_xslt::Stylesheet {
    load(&stylesheet(FULL)).unwrap_or_else(|e| panic!("{e}"))
}

#[test]
fn output_and_globals() {
    let s = full();
    let output = s.output.unwrap();
    assert_eq!(
        (
            output.method,
            output.indent,
            output.omit_xml_declaration,
            output.standalone
        ),
        (
            Some(OutputMethod::Xml),
            Some(true),
            Some(false),
            Some(Standalone::Yes)
        )
    );
    assert_eq!(
        s.globals.iter().map(|g| g.kind).collect::<Vec<_>>(),
        [GlobalKind::Param, GlobalKind::Variable]
    );
    assert!(s.globals[0].variable.as_type.is_some());
}

#[test]
fn functions() {
    let s = full();
    assert_eq!(s.functions.len(), 2);
    let slack = s
        .function(
            &QName {
                prefix: Some("u".into()),
                namespace: Some("urn:u".into()),
                local: "slack".into(),
            },
            2,
        )
        .unwrap();
    assert_eq!(slack.params.len(), 2);
    assert!(
        slack
            .params
            .iter()
            .all(|p| p.value == VariableValue::Absent)
    );
    assert!(
        matches!(slack.body.as_slice(), [Instruction::Variable(_), Instruction::Choose(c)] if c.otherwise.is_some())
    );
    let location = &s.functions[1];
    let [Instruction::Variable(steps), Instruction::ValueOf(value_of)] = location.body.as_slice()
    else {
        panic!("{:?}", location.body)
    };
    assert!(
        value_of
            .separator
            .as_ref()
            .is_some_and(|sep| sep.as_literal().as_deref() == Some("/"))
    );
    let VariableValue::Content(content) = &steps.value else {
        panic!()
    };
    let [Instruction::ForEach(for_each)] = content.as_slice() else {
        panic!()
    };
    let [Instruction::Variable(pos), Instruction::If(_)] = for_each.body.as_slice() else {
        panic!()
    };
    let VariableValue::Content(number) = &pos.value else {
        panic!()
    };
    assert!(
        matches!(number.as_slice(), [Instruction::Number(n)] if n.level == NumberLevel::Single && n.count.is_none())
    );
}

#[test]
fn named_and_root_templates() {
    let s = full();
    let named = s.named_template(&QName::local("validate")).unwrap();
    assert!(named.pattern.is_none() && named.rules.is_empty());
    let [Instruction::ApplyTemplates(apply)] = named.body.as_slice() else {
        panic!()
    };
    assert!(
        matches!(&apply.select, Some(Expr::FunctionCall { name, .. }) if name.local == "document")
    );
    assert_eq!(apply.mode, ApplyMode::Mode(Mode::Default));

    let root = &s.templates[1];
    assert_eq!(
        root.pattern.as_ref().unwrap().alternatives,
        [PatternAlternative::RootFunction]
    );
    assert_eq!(root.rules[0].priority.to_string(), "0.5");
    let [
        Instruction::Variable(report),
        Instruction::LiteralElement(svrl),
    ] = root.body.as_slice()
    else {
        panic!("{:?}", root.body)
    };
    assert!(report.as_type.is_some());
    assert_eq!(svrl.name.to_string(), "svrl:output");
    assert_eq!(
        svrl.name.namespace.as_deref(),
        Some("http://purl.oclc.org/dsdl/svrl")
    );
    let prefixes: Vec<Option<&str>> = svrl
        .namespaces
        .iter()
        .map(|n| n.prefix.as_deref())
        .collect();
    for p in ["xs", "u", "r", "svrl"] {
        assert!(prefixes.contains(&Some(p)), "{prefixes:?}");
    }
    assert!(
        !prefixes.contains(&Some("xsl")) && !prefixes.contains(&Some("xml")),
        "{prefixes:?}"
    );
    assert!(matches!(
        svrl.body.as_slice(),
        [Instruction::Comment(_), Instruction::Sequence(_)]
    ));
}

#[test]
fn all_modes_and_rules_per_mode() {
    let s = full();
    let all_text = &s.templates[2];
    assert_eq!(all_text.modes, TemplateModes::All);
    assert_eq!(all_text.rules.len(), 2);
    assert!(
        all_text
            .rules
            .iter()
            .all(|r| r.priority.to_string() == "-10" && !r.is_default)
    );
    let all_elements = &s.templates[3];
    assert!(
        matches!(&all_elements.body[0], Instruction::ApplyTemplates(a) if a.mode == ApplyMode::Current)
    );

    let p1 = Mode::Named(QName::local("p1"));
    let rules = s.rules(&p1);
    let summary: Vec<(usize, String)> = rules
        .iter()
        .map(|r| (r.template, r.priority.to_string()))
        .collect();
    assert_eq!(
        summary,
        [
            (2, "-10".into()),
            (2, "-10".into()),
            (3, "-10".into()),
            (4, "3".into()),
            (5, "2.5".into())
        ]
    );
    assert_eq!(s.rules(&Mode::Default).len(), 5);
    assert_eq!(
        s.modes().into_iter().collect::<Vec<_>>(),
        [Mode::Default, p1]
    );
}

#[test]
fn parenthesized_pattern_and_next_match() {
    let s = full();
    let filtered = &s.templates[4];
    assert!(matches!(
        filtered.pattern.as_ref().unwrap().alternatives.as_slice(),
        [PatternAlternative::Path(PathPattern {
            first: PathStep::Parenthesized { .. },
            ..
        })]
    ));
    let [
        Instruction::LiteralElement(rule),
        Instruction::NextMatch(next),
    ] = filtered.body.as_slice()
    else {
        panic!()
    };
    assert!(rule.attributes[0].value.as_literal().is_none());
    assert_eq!(
        rule.attributes[1].value.as_literal().as_deref(),
        Some("{literal}")
    );
    assert_eq!(next.params.len(), 1);
}

#[test]
fn params_attributes_text_and_lines() {
    let s = full();
    let c = &s.templates[5];
    assert_eq!(
        c.modes,
        TemplateModes::Listed(vec![Mode::Named(QName::local("p1")), Mode::Default])
    );
    assert_eq!(c.params.len(), 1);
    let [Instruction::If(if_), Instruction::CallTemplate(call)] = c.body.as_slice() else {
        panic!("{:?}", c.body)
    };
    assert_eq!(call.name, QName::local("validate"));
    assert!(matches!(&if_.body[0], Instruction::Attribute(a) if a.select.is_some()));
    assert!(
        matches!(&if_.body[1], Instruction::Attribute(a) if a.select.is_none() && a.body.len() == 1)
    );
    assert!(
        matches!(&if_.body[2], Instruction::Text(t) if t.value == "  kept  " && t.origin == TextOrigin::XslText)
    );
    // Line numbers are those of the source: the two lines of HEAD, then FULL from line 3.
    assert_eq!(
        c.line,
        3 + u32::try_from(
            FULL[..FULL.find("<xsl:template match=\"r:a/r:c[1]\"").unwrap()]
                .matches('\n')
                .count()
        )
        .unwrap()
    );
}

/// Loads `body` and returns the error messages.
fn errors(body: &str) -> Vec<String> {
    match load(&stylesheet(body)) {
        Ok(_) => panic!("loaded: {body}"),
        Err(errors) => errors.0.iter().map(ToString::to_string).collect(),
    }
}

#[track_caller]
fn refused(body: &str, expected: &str) {
    let messages = errors(body);
    assert_eq!(messages, [expected.to_owned()], "{body}");
}

#[test]
fn unknown_instructions_are_refused_with_their_line() {
    refused(
        "<xsl:template match='/'>\n  <xsl:result-document href='x'/>\n</xsl:template>",
        "line 4: unsupported XSLT element xsl:result-document",
    );
    refused(
        "<xsl:template match='/'><xsl:analyze-string select='.' regex='a'/></xsl:template>",
        "line 3: unsupported XSLT element xsl:analyze-string",
    );
    refused(
        "<xsl:template match='/'><xsl:apply-imports/></xsl:template>",
        "line 3: unsupported XSLT element xsl:apply-imports",
    );
    refused(
        "<xsl:key name='k' match='a' use='b'/>",
        "line 3: unsupported XSLT element xsl:key",
    );
    refused(
        "<xsl:include href='x.xsl'/>",
        "line 3: unsupported XSLT element xsl:include",
    );
    refused(
        "<xsl:import href='x.xsl'/>",
        "line 3: unsupported XSLT element xsl:import",
    );
    refused(
        "<xsl:strip-space elements='*'/>",
        "line 3: unsupported XSLT element xsl:strip-space",
    );
    refused(
        "<xsl:template match='/'><xsl:apply-templates select='*'><xsl:sort select='.'/></xsl:apply-templates></xsl:template>",
        "line 3: unsupported XSLT element xsl:sort",
    );
    refused(
        "<xsl:template match='/'><xsl:for-each select='*'><xsl:sort select='.'/></xsl:for-each></xsl:template>",
        "line 3: unsupported XSLT element xsl:sort",
    );
    refused(
        "<xsl:template match='/'><xsl:next-match><xsl:fallback/></xsl:next-match></xsl:template>",
        "line 3: unsupported XSLT element xsl:fallback",
    );
}

#[test]
fn unknown_attributes_and_values_are_refused() {
    refused(
        "<xsl:template match='/' as='xs:string'/>",
        "line 3: unsupported attribute `as` on xsl:template",
    );
    refused(
        "<xsl:template match='/'><xsl:param name='p' tunnel='yes'/></xsl:template>",
        "line 3: unsupported attribute `tunnel` on xsl:param",
    );
    refused(
        "<xsl:template match='/'><xsl:number level='any'/></xsl:template>",
        "line 3: unsupported value `any` of attribute `level` on xsl:number",
    );
    refused(
        "<xsl:template match='/'><xsl:number format='a'/></xsl:template>",
        "line 3: unsupported attribute `format` on xsl:number",
    );
    refused(
        "<xsl:template match='/'><xsl:text disable-output-escaping='yes'>x</xsl:text></xsl:template>",
        "line 3: unsupported attribute `disable-output-escaping` on xsl:text",
    );
    refused(
        "<xsl:template match='/'><xsl:copy copy-namespaces='no'/></xsl:template>",
        "line 3: unsupported attribute `copy-namespaces` on xsl:copy",
    );
    refused(
        "<xsl:template match='/'><xsl:copy-of select='.' copy-namespaces='no'/></xsl:template>",
        "line 3: unsupported attribute `copy-namespaces` on xsl:copy-of",
    );
    refused(
        "<xsl:template match='/'><xsl:message select='1'/></xsl:template>",
        "line 3: unsupported attribute `select` on xsl:message",
    );
    refused(
        "<xsl:template match='/'><xsl:element name='e' inherit-namespaces='no'/></xsl:template>",
        "line 3: unsupported attribute `inherit-namespaces` on xsl:element",
    );
    refused(
        "<xsl:template match='/'><xsl:comment select='1'/></xsl:template>",
        "line 3: unsupported attribute `select` on xsl:comment",
    );
    refused(
        "<xsl:output method='html'/>",
        "line 3: unsupported value `html` of attribute `method` on xsl:output",
    );
    refused(
        "<xsl:output encoding='UTF-8'/>",
        "line 3: unsupported attribute `encoding` on xsl:output",
    );
    refused(
        "<xsl:template match='/' u:ext='1'/>",
        "line 3: unsupported attribute `u:ext` on xsl:template",
    );
    refused(
        "<xsl:template match='/'><r:e xsl:use-attribute-sets='s'/></xsl:template>",
        "line 3: unsupported attribute `xsl:use-attribute-sets` on r:e",
    );
    refused(
        "<xsl:template match='/'><xsl:apply-templates mode='#all'/></xsl:template>",
        "line 3: unsupported value `#all` of attribute `mode` on xsl:apply-templates",
    );
    refused(
        "<xsl:function name='u:f' override='no'/>",
        "line 3: unsupported attribute `override` on xsl:function",
    );
    refused(
        "<xsl:template match='/'><xsl:apply-templates><xsl:with-param name='p' tunnel='yes'/></xsl:apply-templates></xsl:template>",
        "line 3: unsupported attribute `tunnel` on xsl:with-param",
    );
    assert!(load(&stylesheet("")).is_ok(), "the bare stylesheet loads");
}

#[test]
fn the_stylesheet_element_is_checked() {
    let one = "<xsl:stylesheet version='1.0' xmlns:xsl='http://www.w3.org/1999/XSL/Transform'/>";
    assert_eq!(
        load(one).unwrap_err().to_string(),
        "line 1: unsupported value `1.0` of attribute `version` on xsl:stylesheet"
    );
    let ns = "<xsl:stylesheet version='2.0' xpath-default-namespace='urn:x' xmlns:xsl='http://www.w3.org/1999/XSL/Transform'/>";
    assert_eq!(
        load(ns).unwrap_err().to_string(),
        "line 1: unsupported attribute `xpath-default-namespace` on xsl:stylesheet"
    );
    let simplified = "<out xsl:version='2.0' xmlns:xsl='http://www.w3.org/1999/XSL/Transform'/>";
    assert!(
        load(simplified)
            .unwrap_err()
            .to_string()
            .contains("simplified stylesheets are not supported")
    );
    let broken =
        "<xsl:stylesheet version='2.0' xmlns:xsl='http://www.w3.org/1999/XSL/Transform'>\n<a></b>";
    assert!(matches!(
        load(broken).unwrap_err().0[0].kind,
        LoadErrorKind::Xml(_)
    ));
}

#[test]
fn xpath_errors_name_the_attribute_and_line() {
    refused(
        "<xsl:template match='/'>\n<xsl:if test='a intersect b'/></xsl:template>",
        "line 4: xsl:if/@test: unsupported at column 3: `intersect` operator",
    );
    refused(
        "<xsl:template match='/'><xsl:value-of select='a +'/></xsl:template>",
        "line 3: xsl:value-of/@select: syntax error at column 4: expected an expression, found the end of the expression",
    );
    refused(
        "<xsl:template match='key(\"k\", 1)'/>",
        "line 3: xsl:template/@match: unsupported at column 1: `key()` pattern",
    );
    refused(
        "<xsl:variable name='v' as='item()*'/>",
        "line 3: xsl:variable/@as: unsupported at column 1: item type `item()`",
    );
    refused(
        "<xsl:template match='/'><r:e a='{q:x}'/></xsl:template>",
        "line 3: r:e/@a: undeclared namespace prefix at column 2: prefix `q` is not declared",
    );
    refused(
        "<xsl:template match='/'><r:e a='}'/></xsl:template>",
        "line 3: r:e/@a: syntax error at column 1: `}` in an attribute value template must be written `}}`",
    );
}

#[test]
fn static_errors_in_declarations_are_reported() {
    refused(
        "<xsl:template/>",
        "line 3: xsl:template needs a `match` or a `name` attribute (XTSE0500)",
    );
    refused(
        "<xsl:template name='t' mode='m'/>",
        "line 3: `mode` and `priority` need a `match` attribute (XTSE0500)",
    );
    refused(
        "<xsl:template match='/' priority='high'/>",
        "line 3: priority `high` is not a decimal number (XTSE0530)",
    );
    refused(
        "<xsl:template match='/' mode='#all m'/>",
        "line 3: invalid mode list `#all m` (XTSE0550)",
    );
    refused(
        "<xsl:template match='/' mode='m m'/>",
        "line 3: mode `m` is listed twice (XTSE0550)",
    );
    refused(
        "<xsl:template match='/' mode='#current'/>",
        "line 3: unsupported value `#current` of attribute `mode` on xsl:template",
    );
    refused(
        "<xsl:variable name='v' select='1'>x</xsl:variable>",
        "line 3: xsl:variable has both a `select` attribute and content (XTSE0620)",
    );
    refused(
        "<xsl:function name='f'/>",
        "line 3: function name `f` must have a prefix (XTSE0740)",
    );
    refused(
        "<xsl:function name='u:f'><xsl:param name='p'>1</xsl:param></xsl:function>",
        "line 3: a function parameter cannot have a default value (XTSE0760)",
    );
    refused(
        "<xsl:function name='u:f'><xsl:param name='p' select='1'/></xsl:function>",
        "line 3: unsupported attribute `select` on xsl:param",
    );
}

#[test]
fn static_errors_in_sequence_constructors_are_reported() {
    refused(
        "<xsl:template match='/'><xsl:value-of select='1'>x</xsl:value-of></xsl:template>",
        "line 3: xsl:value-of has both a `select` attribute and content (XTSE0870)",
    );
    refused(
        "<xsl:template match='/'><xsl:attribute name='a' select='1'>x</xsl:attribute></xsl:template>",
        "line 3: xsl:attribute has both a `select` attribute and content (XTSE0840)",
    );
    refused(
        "<xsl:template match='/'><xsl:choose><xsl:otherwise/></xsl:choose></xsl:template>",
        "line 3: xsl:otherwise is not allowed here in xsl:choose (XTSE0010)",
    );
    refused(
        "<xsl:template match='/'><xsl:choose/></xsl:template>",
        "line 3: xsl:choose needs at least one xsl:when (XTSE0010)",
    );
    refused(
        "<xsl:template match='/'><xsl:choose>x<xsl:when test='1'/></xsl:choose></xsl:template>",
        "line 3: text is not allowed in xsl:choose (XTSE0010)",
    );
    refused(
        "<xsl:template match='/'><xsl:when test='1'/></xsl:template>",
        "line 3: xsl:when is not allowed here (XTSE0010)",
    );
    refused(
        "<xsl:template match='/'><xsl:text>a<r:b/></xsl:text></xsl:template>",
        "line 3: r:b is not allowed in xsl:text (XTSE0010)",
    );
    refused(
        "<xsl:template match='/'><xsl:sequence select='1'>x</xsl:sequence></xsl:template>",
        "line 3: xsl:sequence must be empty (XTSE0260)",
    );
    refused(
        "<xsl:template match='/'><xsl:sequence/></xsl:template>",
        "line 3: xsl:sequence needs a `select` attribute",
    );
    refused(
        "<xsl:template match='/'><x/><xsl:param name='late'/></xsl:template>",
        "line 3: xsl:param is not allowed here (XTSE0010)",
    );
    refused(
        "<xsl:template match='/'><xsl:param name='p'/><xsl:param name='p'/></xsl:template>",
        "line 3: parameter `p` is declared twice (XTSE0580)",
    );
    refused(
        "<xsl:template match='/'><xsl:next-match><xsl:with-param name='p'/><xsl:with-param name='p'/></xsl:next-match></xsl:template>",
        "line 3: parameter `p` is passed twice (XTSE0670)",
    );
    refused(
        "<xsl:template match='/'><xsl:call-template name='t'>x</xsl:call-template></xsl:template>",
        "line 3: text is not allowed in xsl:call-template (XTSE0010)",
    );
}

#[test]
fn duplicates_names_and_top_level_errors_are_reported() {
    refused(
        "<xsl:variable name='v'/><xsl:param name='v'/>",
        "line 3: global variable `v` is declared twice (XTSE0630)",
    );
    refused(
        "<xsl:template name='t'/><xsl:template name='t'/>",
        "line 3: template `t` is declared twice (XTSE0660)",
    );
    refused(
        "<xsl:function name='u:f'/><xsl:function name='u:f'/>",
        "line 3: function `u:f` with 0 parameters is declared twice (XTSE0770)",
    );
    refused(
        "<xsl:template name='q:t'/>",
        "line 3: prefix `q` in attribute `name` of xsl:template is not declared (XTSE0280)",
    );
    refused(
        "<xsl:template name='1t'/>",
        "line 3: `1t` in attribute `name` of xsl:template is not a QName (XTSE0020)",
    );
    refused(
        "text",
        "line 2: text is not allowed at the top level of a stylesheet (XTSE0120)",
    );
    refused(
        "<data/>",
        "line 3: top-level element data must be in a namespace (XTSE0130)",
    );
    refused(
        "<xsl:template match='/' xml:space='keep'/>",
        "line 3: xml:space must be `preserve` or `default`, not `keep`",
    );
}

#[test]
fn all_errors_are_collected() {
    let messages = errors(
        "<xsl:template match='/'>\n<xsl:try/>\n<xsl:if test='a except b'/>\n</xsl:template>\n<xsl:key name='k' match='a' use='b'/>\n<xsl:template match='a' bogus='1'/>",
    );
    assert_eq!(
        messages,
        [
            "line 4: unsupported XSLT element xsl:try",
            "line 5: xsl:if/@test: unsupported at column 3: `except` operator",
            "line 7: unsupported XSLT element xsl:key",
            "line 8: unsupported attribute `bogus` on xsl:template",
        ]
    );
}

#[test]
fn data_elements_are_kept_and_not_interpreted() {
    let s = load(&stylesheet(
        "<r:doc><anything xsl:version='9'><xsl:message/></anything></r:doc>",
    ))
    .unwrap();
    assert_eq!(s.data_elements.len(), 1);
    assert_eq!(s.data_elements[0].name.to_string(), "r:doc");
    assert_eq!(s.data_elements[0].line, 3);
}
