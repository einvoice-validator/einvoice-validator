//! The parts of the evaluator that the Saxon oracle cannot reach: static errors, an
//! absent focus, the host hooks (variables, functions, `document()`), trees built with
//! the builder, and the refusal of what is not implemented.

// Integration tests are only ever built with `cfg(test)`; saying so lets clippy apply its
// test exemptions (unwrap, expect, panic) to the helpers outside `#[test]` functions too.
#![cfg(test)]

use std::cell::Cell;
use std::rc::Rc;

use einvoice_xdm::AtomicValue;
use einvoice_xpath::{NamespaceMap, QName, XS_NAMESPACE, parse_expr};
use einvoice_xpath_eval::{
    Cardinality, CompiledExpr, Context, Document, DynamicContext, Error, ErrorCode, FunctionId,
    Host, Item, NoHost, Node, SimpleStaticContext, TreeBuilder, VariableId, compile,
};

fn namespaces() -> NamespaceMap {
    NamespaceMap::new()
        .bind("xs", XS_NAMESPACE)
        .bind("xsl", "http://www.w3.org/1999/XSL/Transform")
        .bind("f", "urn:f")
}

fn compiled(text: &str, context: &SimpleStaticContext) -> Result<CompiledExpr, Error> {
    compile(&parse_expr(text, &namespaces()).unwrap(), context)
}

fn plain(text: &str) -> CompiledExpr {
    compiled(text, &SimpleStaticContext::new()).unwrap()
}

fn strings(items: &[Item]) -> Vec<String> {
    items
        .iter()
        .map(|i| i.string_value().into_owned())
        .collect()
}

fn eval_at(text: &str, node: &Node) -> Result<Vec<String>, ErrorCode> {
    let dc = DynamicContext::new(&NoHost).unwrap();
    plain(text)
        .evaluate(&dc, &Context::for_item(Item::Node(node.clone())))
        .map(|items| strings(&items))
        .map_err(|e| e.code())
}

#[test]
fn static_errors() {
    let context = SimpleStaticContext::new();
    let code = |text: &str| compiled(text, &context).unwrap_err().code();
    assert_eq!(code("frobnicate(1)"), ErrorCode::Xpst0017);
    assert_eq!(code("f:g(1)"), ErrorCode::Xpst0017);
    assert_eq!(code("concat('a')"), ErrorCode::Unsupported);
    assert_eq!(code("contains('a', 'b', 'c')"), ErrorCode::Unsupported);
    assert_eq!(code("floor(1.5)"), ErrorCode::Unsupported);
    assert_eq!(code("xs:double('1')"), ErrorCode::Unsupported);
    assert_eq!(code("xs:decimal(1, 2)"), ErrorCode::Xpst0017);
    assert_eq!(code("$undeclared"), ErrorCode::Xpst0008);
    assert_eq!(code("for $a in 1 return $b"), ErrorCode::Xpst0008);
    // The XSLT functions exist only where the static context says so.
    assert_eq!(code("current()"), ErrorCode::Xpst0017);
    assert_eq!(code("document('a.xml')"), ErrorCode::Xpst0017);
    let xslt = SimpleStaticContext::new()
        .with_namespaces(namespaces())
        .with_xslt_functions();
    assert!(compiled("current()", &xslt).is_ok());
    assert!(compiled("system-property('xsl:version')", &xslt).is_ok());
    let code = |text: &str| compiled(text, &xslt).unwrap_err().code();
    assert_eq!(code("system-property('nope:version')"), ErrorCode::Xtde1390);
    assert_eq!(code("system-property('1x')"), ErrorCode::Xtde1390);
    assert_eq!(
        code("system-property(concat('xsl:', 'version'))"),
        ErrorCode::Unsupported
    );
    assert!(ErrorCode::Xpst0017.is_static() && !ErrorCode::Xpty0004.is_static());
}

#[test]
fn absent_focus() {
    let dc = DynamicContext::new(&NoHost).unwrap();
    for text in [
        ".",
        "/",
        "a",
        "name()",
        "position()",
        "last()",
        "string()",
        "number()",
        "root()",
    ] {
        let error = plain(text).evaluate(&dc, &Context::new()).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Xpdy0002, "{text}");
    }
    // Expressions that do not read the focus need none.
    let result = plain("1 + 1").evaluate(&dc, &Context::new()).unwrap();
    assert_eq!(strings(&result), ["2"]);
    // current() needs a current item.
    let xslt = SimpleStaticContext::new().with_xslt_functions();
    let current = compiled("current()", &xslt).unwrap();
    let error = current.evaluate(&dc, &Context::new()).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Xtde1360);
}

#[test]
fn focus_from_the_caller() {
    let doc = Document::parse("<r><a/><a/><a/></r>", None).unwrap();
    let second = doc
        .root()
        .children()
        .next()
        .unwrap()
        .children()
        .nth(1)
        .unwrap();
    let dc = DynamicContext::new(&NoHost).unwrap();
    let context = Context::at(Item::Node(second), 2, 3);
    let result = plain("(position(), last(), count(preceding-sibling::*))")
        .evaluate(&dc, &context)
        .unwrap();
    assert_eq!(strings(&result), ["2", "3", "1"]);
}

/// A host with two variables, a function and a document.
struct TestHost {
    calls: Cell<usize>,
    other: Rc<Document>,
}

impl Host for TestHost {
    fn variable(&self, id: VariableId) -> Result<Rc<[Item]>, Error> {
        Ok(match id.0 {
            0 => Rc::from(vec![Item::Atomic(AtomicValue::String("EUR".to_owned()))]),
            _ => Rc::from(vec![
                Item::Atomic(AtomicValue::from(1.5)),
                Item::Atomic(AtomicValue::from(2.5)),
            ]),
        })
    }

    fn call_function(&self, id: FunctionId, args: Vec<Vec<Item>>) -> Result<Vec<Item>, Error> {
        assert_eq!(id, FunctionId(0));
        self.calls.set(self.calls.get() + 1);
        Ok(vec![Item::Atomic(AtomicValue::String(format!(
            "{} item(s)",
            args[0].len()
        )))])
    }

    fn document(&self, uri: &str, base: Option<&str>) -> Result<Node, Error> {
        if uri == "other.xml" && base.is_none() {
            Ok(self.other.root())
        } else {
            Err(Error::new(ErrorCode::Host, format!("FODC0002: {uri}")))
        }
    }

    fn system_property(&self, name: &QName) -> String {
        format!(
            "{{{}}}{}",
            name.namespace.as_deref().unwrap_or(""),
            name.local
        )
    }
}

#[test]
fn host_variables_functions_and_documents() {
    let mut context = SimpleStaticContext::new()
        .with_namespaces(namespaces())
        .with_xslt_functions();
    let currency = QName::local("currency");
    context.declare_variable(currency.clone(), Cardinality::AtMostOne);
    context.declare_variable(QName::local("rates"), Cardinality::Any);
    let function = QName {
        prefix: Some("f".to_owned()),
        namespace: Some("urn:f".to_owned()),
        local: "count".to_owned(),
    };
    context.declare_function(function, 1);
    let host = TestHost {
        calls: Cell::new(0),
        other: Document::parse_loaded("<o><x>1</x><x>2</x></o>", "urn:other").unwrap(),
    };
    let dc = DynamicContext::new(&host).unwrap();
    let eval = |text: &str| {
        compiled(text, &context)
            .unwrap()
            .evaluate(&dc, &Context::new())
            .map(|items| strings(&items))
    };
    assert_eq!(eval("$currency = 'EUR'").unwrap(), ["true"]);
    assert_eq!(eval("sum($rates)").unwrap(), ["4"]);
    assert_eq!(eval("$rates[2]").unwrap(), ["2.5"]);
    assert_eq!(
        eval("for $currency in 1 to 2 return $currency").unwrap(),
        ["1", "2"]
    );
    assert_eq!(eval("f:count((1, 2, 3))").unwrap(), ["3 item(s)"]);
    assert_eq!(host.calls.get(), 1);
    assert_eq!(eval("document('other.xml')/o/x").unwrap(), ["1", "2"]);
    assert_eq!(
        eval("document('missing.xml')").unwrap_err().code(),
        ErrorCode::Host
    );
    assert_eq!(
        eval("document-uri(document('other.xml'))").unwrap(),
        ["urn:other"]
    );
    assert_eq!(
        eval("system-property('xsl:vendor')").unwrap(),
        ["{http://www.w3.org/1999/XSL/Transform}vendor"]
    );
}

#[test]
fn temporary_trees_built_with_the_builder() {
    let mut builder = TreeBuilder::new();
    let svrl = |local: &str| QName {
        prefix: Some("svrl".to_owned()),
        namespace: Some("http://purl.oclc.org/dsdl/svrl".to_owned()),
        local: local.to_owned(),
    };
    builder.start_element(svrl("schematron-output")).unwrap();
    for id in ["R1", "R2"] {
        builder.start_element(svrl("failed-assert")).unwrap();
        builder.attribute(QName::local("id"), id).unwrap();
        builder.text("text of ").unwrap();
        builder.text(id).unwrap();
        builder.end_element().unwrap();
    }
    builder.end_element().unwrap();
    let doc = builder.finish(None, Some("urn:base".to_owned())).unwrap();
    let root = doc.root();
    let ns = namespaces().bind("svrl", "http://purl.oclc.org/dsdl/svrl");
    let dc = DynamicContext::new(&NoHost).unwrap();
    let eval = |text: &str| {
        compile(&parse_expr(text, &ns).unwrap(), &SimpleStaticContext::new())
            .unwrap()
            .evaluate(&dc, &Context::for_item(Item::Node(root.clone())))
            .map(|items| strings(&items))
            .unwrap()
    };
    assert_eq!(eval("//svrl:failed-assert/@id"), ["R1", "R2"]);
    assert_eq!(eval("string(/*/*[2])"), ["text of R2"]);
    assert_eq!(eval("count(//text())"), ["2"]);
    assert_eq!(eval("name(/*)"), ["svrl:schematron-output"]);
    assert_eq!(eval("document-uri(/)"), Vec::<String>::new());
    assert_eq!(eval("base-uri((//@id)[1])"), ["urn:base"]);
}

#[test]
fn xml_base_is_refused_not_ignored() {
    let doc = Document::parse("<r xml:base='http://x/'><a/></r>", Some("file:/d.xml")).unwrap();
    assert_eq!(
        eval_at("base-uri(//a)", &doc.root()),
        Err(ErrorCode::Unsupported)
    );
    // Saxon-HE 12.10: a document that was not loaded by document() has no document URI.
    assert_eq!(
        eval_at("document-uri(/)", &doc.root()).unwrap(),
        Vec::<String>::new()
    );
}

#[test]
fn casting_a_string_to_any_uri_is_refused_not_false() {
    // einvoice-xdm refuses this cast; `castable as` must say so rather than answer false.
    let doc = Document::parse("<r/>", None).unwrap();
    assert_eq!(
        eval_at("'a' castable as xs:anyURI", &doc.root()),
        Err(ErrorCode::Unsupported)
    );
}

#[test]
fn lazy_reading_stops_at_the_answer() {
    // The second item raises an error when read: consumers that stop at the first one
    // never see it.
    let doc = Document::parse("<r><n>1</n><n>x</n></r>", None).unwrap();
    let root = doc.root();
    assert_eq!(eval_at("exists(//n[. > 0])", &root).unwrap(), ["true"]);
    assert_eq!(eval_at("(//n[. > 0])[1]", &root).unwrap(), ["1"]);
    assert_eq!(
        eval_at("some $n in //n satisfies $n = 1", &root).unwrap(),
        ["true"]
    );
    assert_eq!(eval_at("//n = 1", &root).unwrap(), ["true"]);
    assert_eq!(
        eval_at("count(//n[. > 0])", &root),
        Err(ErrorCode::Forg0001)
    );
    assert_eq!(
        eval_at("//n[. > 0][last()]", &root),
        Err(ErrorCode::Forg0001)
    );
    assert_eq!(
        eval_at("every $n in //n satisfies $n = 1", &root),
        Err(ErrorCode::Forg0001)
    );
}

#[test]
fn static_cardinality() {
    assert!(plain("@id").is_at_most_one());
    assert!(plain("xs:decimal(a)").is_at_most_one());
    assert!(plain("a[1]").is_at_most_one());
    assert!(!plain("a").is_at_most_one());
    assert!(!plain("tokenize('a b', ' ')").is_at_most_one());
}

#[test]
fn current_date_time_is_stable_and_settable() {
    let dc = DynamicContext::new(&NoHost)
        .unwrap()
        .with_implicit_timezone(120)
        .unwrap();
    let text = plain("string(current-dateTime())")
        .evaluate(&dc, &Context::new())
        .unwrap();
    assert!(strings(&text)[0].ends_with("+02:00"), "{text:?}");
    let fixed = einvoice_xdm::DateTime::parse("2024-01-31T12:00:00Z").unwrap();
    let dc = dc.with_current_date_time(fixed);
    let result = plain("current-dateTime()")
        .evaluate(&dc, &Context::new())
        .unwrap();
    assert_eq!(strings(&result), ["2024-01-31T12:00:00Z"]);
    assert!(
        DynamicContext::new(&NoHost)
            .unwrap()
            .with_implicit_timezone(900)
            .is_err()
    );
}

#[test]
fn compiled_expressions_can_be_shared_between_threads() {
    fn shareable<T: Send + Sync>() {}
    shareable::<CompiledExpr>();
}
