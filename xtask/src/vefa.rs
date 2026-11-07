//! VEFA unit test sets: each `<test>` of a `<testSet>` holds an expectation
//! (the rule ids that must succeed, raise an error or raise a warning) and one
//! document to validate. Extraction turns each test into a standalone XML
//! document, byte for byte the document element of the test, plus the XML
//! declaration and, if the element relied on namespaces declared by its
//! ancestors in the test set, those declarations. The result is checked by
//! parsing it back and comparing it node by node with the original.

use std::collections::BTreeMap;

use roxmltree::{Document, Node, NodeType, ParsingOptions};

use crate::util::{Error, Result, normalize_space};

/// The VEFA test-set namespace.
const VEFA_NS: &str = "http://difi.no/xsd/vefa/validator/1.0";

/// What a VEFA test expects from validation, for the rule ids it names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Expectation {
    /// Rule ids the test set is about (`<testSet><assert><scope>`).
    pub(crate) scope: Vec<String>,
    /// Rule ids that must not fire (`<success>`).
    pub(crate) success: Vec<String>,
    /// Rule ids that must fire with flag `fatal` (`<error>`).
    pub(crate) error: Vec<String>,
    /// Rule ids that must fire with flag `warning` (`<warning>`).
    pub(crate) warning: Vec<String>,
    /// The test's `<description>`, whitespace normalised (may be empty).
    pub(crate) description: String,
}

/// One extracted test.
#[derive(Debug)]
pub(crate) struct TestCase {
    /// 1-based position of the `<test>` in its test set.
    pub(crate) ordinal: usize,
    pub(crate) expectation: Expectation,
    /// The standalone document.
    pub(crate) document: String,
}

/// Extracts every test of a test set, in document order.
pub(crate) fn extract(test_set: &str) -> Result<Vec<TestCase>> {
    let doc = parse(test_set)?;
    let root = doc.root_element();
    expect_vefa(root, "testSet")?;

    let mut scope = Vec::new();
    let mut tests = Vec::new();
    for child in element_children(root)? {
        match vefa_name(child)? {
            "assert" => {
                for item in element_children(child)? {
                    match vefa_name(item)? {
                        "scope" => scope.push(rule_id(item)?),
                        "description" => {}
                        other => return Err(unexpected(item, other)),
                    }
                }
            }
            "test" => tests.push(child),
            other => return Err(unexpected(child, other)),
        }
    }
    if scope.is_empty() {
        return Err(Error::new("test set without <scope>"));
    }

    let mut cases = Vec::with_capacity(tests.len());
    for (index, test) in tests.into_iter().enumerate() {
        let ordinal = index + 1;
        let case = extract_test(test_set, test, ordinal, &scope)
            .map_err(|e| Error::new(format!("test #{ordinal}: {e}")))?;
        cases.push(case);
    }
    Ok(cases)
}

fn extract_test(
    source: &str,
    test: Node<'_, '_>,
    ordinal: usize,
    scope: &[String],
) -> Result<TestCase> {
    let mut expectation = Expectation {
        scope: scope.to_vec(),
        success: Vec::new(),
        error: Vec::new(),
        warning: Vec::new(),
        description: String::new(),
    };
    let mut asserts = 0;
    let mut documents = Vec::new();
    for child in element_children(test)? {
        if child.tag_name().namespace() != Some(VEFA_NS) {
            documents.push(child);
            continue;
        }
        match vefa_name(child)? {
            "assert" => {
                asserts += 1;
                for item in element_children(child)? {
                    match vefa_name(item)? {
                        "success" => expectation.success.push(rule_id(item)?),
                        "error" => expectation.error.push(rule_id(item)?),
                        "warning" => expectation.warning.push(rule_id(item)?),
                        "description" => {
                            if !expectation.description.is_empty() {
                                return Err(Error::new("more than one <description>"));
                            }
                            expectation.description = normalize_space(&text_of(item)?);
                        }
                        other => return Err(unexpected(item, other)),
                    }
                }
            }
            other => return Err(unexpected(child, other)),
        }
    }
    if asserts != 1 {
        return Err(Error::new(format!(
            "{asserts} <assert> elements, expected 1"
        )));
    }
    let [document] = documents[..] else {
        return Err(Error::new(format!(
            "{} documents, expected exactly 1",
            documents.len()
        )));
    };
    let standalone = standalone_document(source, document)?;
    verify_same_tree(document, &standalone)?;
    Ok(TestCase {
        ordinal,
        expectation,
        document: standalone,
    })
}

/// The document element's source text as a document of its own.
fn standalone_document(source: &str, element: Node<'_, '_>) -> Result<String> {
    let range = element.range();
    let text = source
        .get(range.clone())
        .ok_or_else(|| Error::new("element range outside the source"))?;
    let start_tag = start_tag(text)?;
    let name_end = start_tag
        .find(|c: char| c.is_ascii_whitespace() || c == '>' || c == '/')
        .ok_or_else(|| Error::new("cannot find the end of the element name"))?;
    let declared = declared_namespace_attributes(start_tag)?;

    // In-scope namespaces the element does not declare itself: inherited from
    // the test set. Re-declare them so the standalone document has the same
    // namespaces in scope.
    let mut inherited = BTreeMap::new();
    for ns in element.namespaces() {
        let attribute = ns
            .name()
            .map_or_else(|| "xmlns".to_owned(), |p| format!("xmlns:{p}"));
        if ns.name() == Some("xml") || declared.contains(&attribute) {
            continue;
        }
        inherited.insert(attribute, ns.uri().to_owned());
    }

    let mut out = String::with_capacity(text.len() + 64);
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(&text[..name_end]);
    for (attribute, uri) in &inherited {
        out.push(' ');
        out.push_str(attribute);
        out.push_str("=\"");
        out.push_str(&escape_attribute(uri));
        out.push('"');
    }
    out.push_str(&text[name_end..]);
    out.push('\n');
    Ok(out)
}

/// The start tag at the beginning of `text`, up to and including its `>`.
fn start_tag(text: &str) -> Result<&str> {
    let mut quote = None;
    for (index, c) in text.char_indices() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), _) if c == q => quote = None,
            (None, '>') => return Ok(&text[..=index]),
            _ => {}
        }
    }
    Err(Error::new("unterminated start tag"))
}

/// Names of the `xmlns` / `xmlns:*` attributes written in a start tag.
fn declared_namespace_attributes(start_tag: &str) -> Result<Vec<String>> {
    let body = start_tag
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim_end_matches('/');
    let mut rest = body
        .find(|c: char| c.is_ascii_whitespace())
        .map_or("", |i| &body[i..]);
    let mut names = Vec::new();
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            return Ok(names);
        }
        let eq = rest
            .find('=')
            .ok_or_else(|| Error::new("attribute without a value in a start tag"))?;
        let name = rest[..eq].trim_end();
        rest = rest[eq + 1..].trim_start();
        let quote = rest
            .chars()
            .next()
            .filter(|c| *c == '"' || *c == '\'')
            .ok_or_else(|| Error::new("unquoted attribute value in a start tag"))?;
        let close = rest[1..]
            .find(quote)
            .ok_or_else(|| Error::new("unterminated attribute value in a start tag"))?;
        rest = &rest[close + 2..];
        if name == "xmlns" || name.starts_with("xmlns:") {
            names.push(name.to_owned());
        }
    }
}

fn escape_attribute(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('"', "&quot;")
}

/// Parses the standalone document and checks it is the same tree as the
/// original element: names, namespaces, attributes, text, comments and
/// processing instructions, and the same namespaces in scope at the root.
fn verify_same_tree(original: Node<'_, '_>, standalone: &str) -> Result<()> {
    let doc = parse(standalone).map_err(|e| Error::new(format!("extracted document: {e}")))?;
    let copy = doc.root_element();
    let scope = |node: Node<'_, '_>| {
        let mut v: Vec<(Option<String>, String)> = node
            .namespaces()
            .filter(|ns| ns.name() != Some("xml"))
            .map(|ns| (ns.name().map(str::to_owned), ns.uri().to_owned()))
            .collect();
        v.sort();
        v
    };
    if scope(original) != scope(copy) {
        return Err(Error::new("extracted document: namespaces in scope differ"));
    }
    if same_node(original, copy) {
        Ok(())
    } else {
        Err(Error::new(
            "extracted document differs from the original element",
        ))
    }
}

fn same_node(a: Node<'_, '_>, b: Node<'_, '_>) -> bool {
    if a.node_type() != b.node_type() {
        return false;
    }
    match a.node_type() {
        NodeType::Element => {
            let attrs = |n: Node<'_, '_>| -> Vec<(Option<String>, String, String)> {
                n.attributes()
                    .map(|at| {
                        (
                            at.namespace().map(str::to_owned),
                            at.name().to_owned(),
                            at.value().to_owned(),
                        )
                    })
                    .collect()
            };
            a.tag_name() == b.tag_name()
                && attrs(a) == attrs(b)
                && a.children().count() == b.children().count()
                && a.children().zip(b.children()).all(|(x, y)| same_node(x, y))
        }
        NodeType::Text | NodeType::Comment => a.text() == b.text(),
        NodeType::PI => a.pi() == b.pi(),
        NodeType::Root => false,
    }
}

fn parse(text: &str) -> Result<Document<'_>> {
    let options = ParsingOptions {
        // No DTDs: entity expansion would make the extracted text differ from
        // the parsed tree.
        allow_dtd: false,
        nodes_limit: u32::MAX,
        ..ParsingOptions::default()
    };
    Document::parse_with_options(text, options).map_err(|e| Error::new(format!("XML: {e}")))
}

fn expect_vefa(node: Node<'_, '_>, name: &str) -> Result<()> {
    if vefa_name(node)? == name {
        Ok(())
    } else {
        Err(unexpected(node, node.tag_name().name()))
    }
}

/// The local name of an element in the VEFA namespace.
fn vefa_name<'a>(node: Node<'a, '_>) -> Result<&'a str> {
    if node.tag_name().namespace() == Some(VEFA_NS) {
        Ok(node.tag_name().name())
    } else {
        Err(Error::new(format!(
            "unexpected element {{{}}}{} at byte {}",
            node.tag_name().namespace().unwrap_or(""),
            node.tag_name().name(),
            node.range().start
        )))
    }
}

fn unexpected(node: Node<'_, '_>, name: &str) -> Error {
    Error::new(format!(
        "unexpected element <{name}> at byte {}",
        node.range().start
    ))
}

/// Element children; text other than whitespace between them is refused.
fn element_children<'a, 'input>(node: Node<'a, 'input>) -> Result<Vec<Node<'a, 'input>>> {
    let mut out = Vec::new();
    for child in node.children() {
        match child.node_type() {
            NodeType::Element => out.push(child),
            NodeType::Text if child.text().is_some_and(|t| t.trim().is_empty()) => {}
            NodeType::Comment => {}
            _ => {
                return Err(Error::new(format!(
                    "unexpected content in <{}> at byte {}",
                    node.tag_name().name(),
                    child.range().start
                )));
            }
        }
    }
    Ok(out)
}

/// Text content of an element that holds text only.
fn text_of(node: Node<'_, '_>) -> Result<String> {
    let mut text = String::new();
    for child in node.children() {
        match child.node_type() {
            NodeType::Text => text.push_str(child.text().unwrap_or("")),
            NodeType::Comment => {}
            _ => {
                return Err(Error::new(format!(
                    "unexpected content in <{}> at byte {}",
                    node.tag_name().name(),
                    child.range().start
                )));
            }
        }
    }
    Ok(text)
}

/// A rule id element: non-empty text without inner whitespace.
fn rule_id(node: Node<'_, '_>) -> Result<String> {
    let id = text_of(node)?.trim().to_owned();
    if id.is_empty() || id.contains(char::is_whitespace) {
        return Err(Error::new(format!(
            "invalid rule id {id:?} in <{}> at byte {}",
            node.tag_name().name(),
            node.range().start
        )));
    }
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::extract;

    const SET: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<testSet xmlns="http://difi.no/xsd/vefa/validator/1.0" xmlns:cbc="urn:cbc" configuration="x">
	<assert>
		<scope>BR-01</scope>
	</assert>
	<test>
		<assert>
			<description>  Two
			lines </description>
			<success>BR-01</success>
		</assert>
		<Invoice xmlns="urn:inv" a='1 > 0'>
			<cbc:ID>1</cbc:ID>
		</Invoice>
	</test>
	<test id="2">
		<assert><error>BR-01</error><warning>BR-02</warning></assert>
		<!-- a comment -->
		<i:Invoice xmlns:i="urn:inv"><x/></i:Invoice>
	</test>
</testSet>
"#;

    #[test]
    fn extracts_each_test_with_inherited_namespaces() {
        let cases = extract(SET).unwrap();
        assert_eq!(cases.len(), 2);

        let first = &cases[0];
        assert_eq!(first.ordinal, 1);
        assert_eq!(first.expectation.scope, ["BR-01"]);
        assert_eq!(first.expectation.success, ["BR-01"]);
        assert_eq!(first.expectation.error, Vec::<String>::new());
        assert_eq!(first.expectation.description, "Two lines");
        assert_eq!(
            first.document,
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Invoice xmlns:cbc=\"urn:cbc\" xmlns=\"urn:inv\" a='1 > 0'>\n\t\t\t<cbc:ID>1</cbc:ID>\n\t\t</Invoice>\n"
        );

        // No default namespace of its own: it inherits the VEFA one, faithfully
        // (as in the CEN test set UBL-SR-43, test 2).
        let second = &cases[1];
        assert_eq!(second.ordinal, 2);
        assert_eq!(second.expectation.error, ["BR-01"]);
        assert_eq!(second.expectation.warning, ["BR-02"]);
        assert_eq!(
            second.document,
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<i:Invoice xmlns=\"http://difi.no/xsd/vefa/validator/1.0\" xmlns:cbc=\"urn:cbc\" xmlns:i=\"urn:inv\"><x/></i:Invoice>\n"
        );
    }

    #[test]
    fn refuses_unknown_structure() {
        let bad = SET
            .replace("<success>", "<sucess>")
            .replace("</success>", "</sucess>");
        assert!(extract(&bad).is_err());
        let two_docs = SET.replace("<x/></i:Invoice>", "<x/></i:Invoice><i:Invoice/>");
        assert!(extract(&two_docs).is_err());
        // A document element in the VEFA namespace cannot be told apart from
        // a misspelt VEFA element: refused.
        let vefa_document = SET.replace(
            "<i:Invoice xmlns:i=\"urn:inv\"><x/></i:Invoice>",
            "<Invoice/>",
        );
        assert!(extract(&vefa_document).is_err());
        let no_scope = SET.replace("<scope>BR-01</scope>", "");
        assert!(extract(&no_scope).is_err());
    }
}
