//! The vendored CEN EN 16931 UBL 1.3.17 stylesheet loads without error, and the loaded
//! stylesheet matches what an independent scan of the XML finds: templates per mode, rule
//! alternatives, priorities, every kind of instruction, literal result elements, literal
//! text, variables and parameters, data elements and source lines.

// Integration tests are only ever built with `cfg(test)`; saying so lets clippy apply its
// test exemptions (unwrap, expect, panic) to the helpers outside `#[test]` functions too.
#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use einvoice_xslt::{
    GlobalKind, Instruction, Mode, Priority, Stylesheet, TemplateModes, TextOrigin, Variable,
    VariableValue, XSLT_NAMESPACE, load,
};

const STYLESHEET: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../artefacts/cen-en16931-ubl-1.3.17/ubl/xslt/EN16931-UBL-validation.xslt"
);

fn text() -> String {
    std::fs::read_to_string(STYLESHEET).unwrap()
}

// ---- Counting the IR ----

#[derive(Default, Debug)]
struct IrCounts {
    /// Instruction kind (XSLT local name, or `#lre` / `#text`) -> count.
    instructions: BTreeMap<String, usize>,
    local_variables: usize,
    with_params: usize,
}

impl IrCounts {
    fn add(&mut self, kind: &str) {
        *self.instructions.entry(kind.to_owned()).or_default() += 1;
    }

    fn variable(&mut self, v: &Variable) {
        if let VariableValue::Content(body) = &v.value {
            self.body(body);
        }
    }

    fn params(&mut self, params: &[Variable]) {
        self.with_params += params.len();
        for p in params {
            self.variable(p);
        }
    }

    fn body(&mut self, body: &[Instruction]) {
        for instruction in body {
            match instruction {
                Instruction::ApplyTemplates(i) => {
                    self.add("apply-templates");
                    self.params(&i.params);
                }
                Instruction::NextMatch(i) => {
                    self.add("next-match");
                    self.params(&i.params);
                }
                Instruction::CallTemplate(i) => {
                    self.add("call-template");
                    self.params(&i.params);
                }
                Instruction::Choose(i) => {
                    self.add("choose");
                    for when in &i.whens {
                        self.add("when");
                        self.body(&when.body);
                    }
                    if let Some(otherwise) = &i.otherwise {
                        self.add("otherwise");
                        self.body(otherwise);
                    }
                }
                Instruction::If(i) => {
                    self.add("if");
                    self.body(&i.body);
                }
                Instruction::ForEach(i) => {
                    self.add("for-each");
                    self.body(&i.body);
                }
                Instruction::Variable(v) => {
                    self.add("variable");
                    self.local_variables += 1;
                    self.variable(v);
                }
                Instruction::Sequence(_) => self.add("sequence"),
                Instruction::ValueOf(i) => {
                    self.add("value-of");
                    self.body(&i.body);
                }
                Instruction::Attribute(i) => {
                    self.add("attribute");
                    self.body(&i.body);
                }
                Instruction::Comment(i) => {
                    self.add("comment");
                    self.body(&i.body);
                }
                Instruction::Text(t) => self.add(match t.origin {
                    TextOrigin::XslText => "text",
                    TextOrigin::Literal => "#text",
                }),
                Instruction::Number(_) => self.add("number"),
                Instruction::LiteralElement(e) => {
                    self.add("#lre");
                    self.body(&e.body);
                }
                Instruction::Copy(i) => {
                    self.add("copy");
                    self.body(&i.body);
                }
                Instruction::CopyOf(_) => self.add("copy-of"),
                Instruction::Element(i) => {
                    self.add("element");
                    self.body(&i.body);
                }
                Instruction::Namespace(i) => {
                    self.add("namespace");
                    self.body(&i.body);
                }
                Instruction::ProcessingInstruction(i) => {
                    self.add("processing-instruction");
                    self.body(&i.body);
                }
                Instruction::Message(i) => {
                    self.add("message");
                    self.body(&i.body);
                }
            }
        }
    }
}

fn ir_counts(stylesheet: &Stylesheet) -> IrCounts {
    let mut counts = IrCounts::default();
    for global in &stylesheet.globals {
        counts.add(match global.kind {
            GlobalKind::Param => "param",
            GlobalKind::Variable => "variable",
        });
        counts.variable(&global.variable);
    }
    for template in &stylesheet.templates {
        counts.add("template");
        for p in &template.params {
            counts.add("param");
            counts.variable(p);
        }
        counts.body(&template.body);
    }
    for function in &stylesheet.functions {
        counts.add("function");
        counts.body(&function.body);
    }
    counts
}

// ---- Scanning the XML independently ----

fn is_xsl(node: roxmltree::Node<'_, '_>) -> bool {
    node.tag_name().namespace() == Some(XSLT_NAMESPACE)
}

/// Splits a pattern on `|` outside brackets, parentheses and string literals.
fn split_alternatives(pattern: &str) -> Vec<String> {
    let mut parts = vec![String::new()];
    let (mut depth, mut quote) = (0_i32, None::<char>);
    for c in pattern.chars() {
        match (quote, c) {
            (Some(q), _) if c == q => quote = None,
            (None, '\'' | '"') => quote = Some(c),
            (None, '[' | '(') => depth += 1,
            (None, ']' | ')') => depth -= 1,
            (None, '|') if depth == 0 => {
                parts.push(String::new());
                continue;
            }
            _ => {}
        }
        parts.last_mut().unwrap().push(c);
    }
    parts.into_iter().map(|p| p.trim().to_owned()).collect()
}

fn line_of(text: &str, node: roxmltree::Node<'_, '_>) -> u32 {
    u32::try_from(text[..node.range().start].matches('\n').count() + 1).unwrap()
}

fn loaded(text: &str) -> Stylesheet {
    load(text).unwrap_or_else(|e| panic!("{e}"))
}

/// The top-level `xsl:template` elements.
fn template_elements<'a, 'input>(
    doc: &'a roxmltree::Document<'input>,
) -> Vec<roxmltree::Node<'a, 'input>> {
    doc.root_element()
        .children()
        .filter(|n| n.has_tag_name((XSLT_NAMESPACE, "template")))
        .collect()
}

/// Counts of XSLT elements by local name, of literal result elements (`#lre`) and of
/// literal text nodes that are not whitespace-only (`#text`), outside top-level data.
fn scan_instructions(doc: &roxmltree::Document<'_>) -> BTreeMap<String, usize> {
    let root = doc.root_element();
    let data: Vec<roxmltree::Node<'_, '_>> = root
        .children()
        .filter(|n| n.is_element() && !is_xsl(*n))
        .collect();
    let in_data = |n: roxmltree::Node<'_, '_>| n.ancestors().any(|a| data.contains(&a));
    let mut scanned: BTreeMap<String, usize> = BTreeMap::new();
    for node in doc
        .descendants()
        .filter(|n| n.is_element() && *n != root && !in_data(*n))
    {
        let key = if is_xsl(node) {
            node.tag_name().name().to_owned()
        } else {
            "#lre".to_owned()
        };
        *scanned.entry(key).or_default() += 1;
    }
    let literal_text = doc
        .descendants()
        .filter(|n| n.is_text() && !in_data(*n))
        // XML whitespace only: U+00A0 and other Unicode spaces are text (`str::trim` would
        // drop them).
        .filter(|n| {
            !n.text()
                .unwrap_or("")
                .chars()
                .all(|c| matches!(c, ' ' | '\t' | '\r' | '\n'))
        })
        .filter(|n| {
            !n.parent()
                .is_some_and(|p| p.has_tag_name((XSLT_NAMESPACE, "text")))
        })
        .count();
    scanned.insert("#text".into(), literal_text);
    scanned
}

#[test]
fn data_elements_match_the_scan() {
    let text = text();
    let (stylesheet, doc) = (loaded(&text), roxmltree::Document::parse(&text).unwrap());
    let data: Vec<u32> = doc
        .root_element()
        .children()
        .filter(|n| n.is_element() && !is_xsl(*n))
        .map(|n| line_of(&text, n))
        .collect();
    assert_eq!(
        stylesheet
            .data_elements
            .iter()
            .map(|d| d.line)
            .collect::<Vec<_>>(),
        data
    );
    assert_eq!(data.len(), 1, "the top-level svrl:text");
}

#[test]
fn every_instruction_matches_the_scan() {
    let text = text();
    let (stylesheet, doc) = (loaded(&text), roxmltree::Document::parse(&text).unwrap());
    let mut scanned = scan_instructions(&doc);
    assert_eq!(scanned.remove("output"), Some(1));
    assert!(stylesheet.output.is_some());
    let counts = ir_counts(&stylesheet);
    assert_eq!(counts.instructions, scanned);
    // The published counts of the stylesheet.
    for (kind, n) in [
        ("apply-templates", 1100),
        ("attribute", 2955),
        ("choose", 984),
        ("when", 984),
        ("otherwise", 984),
        ("text", 25),
        ("value-of", 28),
        ("comment", 1),
        ("for-each", 2),
        ("if", 4),
        ("number", 4),
        ("template", 127),
        ("param", 4),
        ("variable", 2),
    ] {
        assert_eq!(counts.instructions.get(kind), Some(&n), "{kind}");
    }
    // Globals: four parameters and one variable; one local variable; no function.
    let kinds: Vec<GlobalKind> = stylesheet.globals.iter().map(|g| g.kind).collect();
    assert_eq!(
        kinds,
        [
            GlobalKind::Param,
            GlobalKind::Param,
            GlobalKind::Param,
            GlobalKind::Param,
            GlobalKind::Variable
        ]
    );
    assert!(matches!(
        stylesheet.globals[4].variable.value,
        VariableValue::Content(_)
    ));
    assert_eq!(counts.local_variables, 1);
    assert_eq!(counts.with_params, 0);
    assert_eq!(stylesheet.functions.len(), 0);
}

#[test]
fn templates_match_the_scan() {
    let text = text();
    let (stylesheet, doc) = (loaded(&text), roxmltree::Document::parse(&text).unwrap());
    let templates = template_elements(&doc);
    let lines: Vec<u32> = templates.iter().map(|n| line_of(&text, *n)).collect();
    assert_eq!(
        stylesheet
            .templates
            .iter()
            .map(|t| t.line)
            .collect::<Vec<_>>(),
        lines
    );
    for (template, node) in stylesheet.templates.iter().zip(&templates) {
        let alternatives = split_alternatives(node.attribute("match").unwrap());
        assert_eq!(
            template.rules.len(),
            alternatives.len(),
            "line {}",
            template.line
        );
        assert_eq!(
            template.pattern.as_ref().unwrap().alternatives.len(),
            alternatives.len()
        );
        match node.attribute("priority") {
            Some(p) => {
                let p = Priority::parse(p).unwrap();
                let explicit = template
                    .rules
                    .iter()
                    .all(|r| r.priority == p && !r.is_default);
                assert!(explicit, "line {}", template.line);
            }
            None => assert!(
                template.rules.iter().all(|r| r.is_default),
                "line {}",
                template.line
            ),
        }
    }
    assert_eq!(stylesheet.templates.len(), 127);
    assert_eq!(
        stylesheet
            .templates
            .iter()
            .map(|t| t.rules.len())
            .sum::<usize>(),
        171
    );
}

#[test]
fn templates_per_mode_match_the_scan() {
    let text = text();
    let (stylesheet, doc) = (loaded(&text), roxmltree::Document::parse(&text).unwrap());
    let mut scanned: BTreeMap<String, usize> = BTreeMap::new();
    for node in template_elements(&doc) {
        *scanned
            .entry(node.attribute("mode").unwrap_or("#default").to_owned())
            .or_default() += 1;
    }
    let mut ir: BTreeMap<String, usize> = BTreeMap::new();
    for template in &stylesheet.templates {
        let TemplateModes::Listed(modes) = &template.modes else {
            panic!("#all in CEN")
        };
        for mode in modes {
            let label = match mode {
                Mode::Default => "#default".to_owned(),
                Mode::Named(q) => q.to_string(),
            };
            *ir.entry(label).or_default() += 1;
        }
    }
    assert_eq!(ir, scanned);
    let expected: BTreeMap<String, usize> = [
        ("#default", 2),
        ("M11", 68),
        ("M12", 18),
        ("M13", 24),
        ("generate-id-2", 4),
        ("generate-id-from-path", 6),
        ("schematron-get-full-path", 2),
        ("schematron-get-full-path-2", 1),
        ("schematron-get-full-path-3", 1),
        ("schematron-select-full-path", 1),
    ]
    .into_iter()
    .map(|(m, n)| (m.to_owned(), n))
    .collect();
    assert_eq!(ir, expected);
    assert_eq!(stylesheet.modes().len(), 10);

    // Rules per mode, as the evaluator will see them: one per alternative.
    let alternatives_in = |mode: &str| -> usize {
        template_elements(&doc)
            .iter()
            .filter(|n| n.attribute("mode") == Some(mode))
            .map(|n| split_alternatives(n.attribute("match").unwrap()).len())
            .sum()
    };
    let m11 = stylesheet.rules(&Mode::Named(einvoice_xpath::QName::local("M11")));
    assert_eq!(m11.len(), alternatives_in("M11"));
    let priorities: BTreeSet<String> = m11.iter().map(|r| r.priority.to_string()).collect();
    assert!(priorities.contains("1065"), "{priorities:?}");
    assert_eq!(stylesheet.rules(&Mode::Default).len(), 2);
}

#[test]
fn templates_without_priority_get_the_default_priority() {
    // Every template without `priority` here has a pattern whose alternatives are `/`, `*`,
    // `@*`, `node()`, `text()`, `comment()` or `processing-instruction()`: -0.5 each.
    let stylesheet = loaded(&text());
    let defaulted: Vec<(u32, Vec<String>)> = stylesheet
        .templates
        .iter()
        .filter(|t| t.priority.is_none())
        .map(|t| {
            (
                t.line,
                t.rules.iter().map(|r| r.priority.to_string()).collect(),
            )
        })
        .collect();
    let half = || vec!["-0.5".to_owned()];
    let two_halves = || vec!["-0.5".to_owned(), "-0.5".to_owned()];
    assert_eq!(
        defaulted,
        [
            (37, half()),
            (43, half()),
            (63, half()),
            (81, two_halves()),
            (98, two_halves()),
            (114, half()),
            (115, half()),
            (119, half()),
            (123, half()),
            (127, half()),
            (138, half()),
            (143, half()),
            (149, half()),
            (161, half()),
        ]
    );
}

#[test]
fn whitespace_and_literal_text_follow_the_stripping_rules() {
    let stylesheet = loaded(&text());
    // Line 65: `<xsl:when test="namespace-uri()=''">@<xsl:value-of select="name()" />` then a
    // line break: the `@` is kept, the whitespace-only text after the value-of is stripped.
    let template = stylesheet.templates.iter().find(|t| t.line == 63).unwrap();
    let Instruction::Choose(choose) = &template.body[2] else {
        panic!("{:?}", template.body[2])
    };
    let when = &choose.whens[0].body;
    assert_eq!(when.len(), 2, "{when:?}");
    assert!(
        matches!(&when[0], Instruction::Text(t) if t.value == "@" && t.origin == TextOrigin::Literal)
    );
    assert!(matches!(&when[1], Instruction::ValueOf(_)));
    // The comment of the root template: four value-of separated by text that is not
    // whitespace-only for XML, since it holds a no-break space (U+00A0), so it is kept.
    let root = stylesheet.templates.iter().find(|t| t.line == 161).unwrap();
    let Instruction::LiteralElement(output) = &root.body[0] else {
        panic!()
    };
    assert_eq!(output.name.to_string(), "svrl:schematron-output");
    assert!(output.namespaces.iter().all(|ns| ns.uri != XSLT_NAMESPACE));
    assert!(
        output
            .namespaces
            .iter()
            .any(|ns| ns.prefix.as_deref() == Some("svrl"))
    );
    let Instruction::Comment(comment) = &output.body[0] else {
        panic!()
    };
    assert_eq!(comment.body.len(), 7, "{:?}", comment.body);
    for (i, instruction) in comment.body.iter().enumerate() {
        if i % 2 == 0 {
            assert!(
                matches!(instruction, Instruction::ValueOf(_)),
                "{instruction:?}"
            );
        } else {
            let kept = matches!(instruction, Instruction::Text(t)
                if t.value.contains('\u{a0}') && t.value.chars().all(|c| matches!(c, '\u{a0}' | ' ' | '\t' | '\n')));
            assert!(kept, "{instruction:?}");
        }
    }
}
