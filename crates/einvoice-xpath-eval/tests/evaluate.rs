//! The parts of the evaluator that the Saxon oracle cannot reach: static errors, an
//! absent focus, the host hooks (variables, functions, `document()`), trees built with
//! the builder, and the refusal of what is not implemented.

// Integration tests are only ever built with `cfg(test)`; saying so lets clippy apply its
// test exemptions (unwrap, expect, panic) to the helpers outside `#[test]` functions too.
#![cfg(test)]

use std::cell::Cell;
use std::fmt::Write as _;
use std::rc::Rc;

use einvoice_xdm::{AtomicValue, Integer};
use einvoice_xpath::{NamespaceMap, QName, XS_NAMESPACE, parse_expr, parse_sequence_type};
use einvoice_xpath_eval::{
    Argument, BindingValue, Cardinality, Clock, CompiledExpr, Context, Document, DynamicContext,
    Error, ErrorCode, FunctionId, Host, Item, NoHost, Node, SelectionStorage, SimpleStaticContext,
    TreeBuilder, ValueType, VariableId, compile,
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

/// A fixed clock: UTC, 2026-09-10T00:28:43.456Z.
fn clock() -> Clock {
    Clock::from_unix_millis(0, 1_789_000_123_456).unwrap()
}

fn eval_at(text: &str, node: &Node) -> Result<Vec<String>, ErrorCode> {
    let dc = DynamicContext::new(&NoHost, clock());
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
    let dc = DynamicContext::new(&NoHost, clock());
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
    let dc = DynamicContext::new(&NoHost, clock());
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

    fn call_function(
        &self,
        id: FunctionId,
        args: Vec<Argument>,
        dc: &DynamicContext<'_>,
    ) -> Result<Vec<Item>, Error> {
        assert_eq!(id, FunctionId(0));
        self.calls.set(self.calls.get() + 1);
        Ok(vec![Item::Atomic(AtomicValue::String(format!(
            "{} item(s)",
            args[0].evaluate(dc)?.len()
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
    let dc = DynamicContext::new(&host, clock());
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
    let dc = DynamicContext::new(&NoHost, clock());
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
fn the_clock_is_an_input() {
    // An instant the host read (1_789_000_123_456 ms after the epoch), seen in the
    // implicit timezone +02:00: no clock is read by the evaluator.
    let clock = Clock::from_unix_millis(120, 1_789_000_123_456).unwrap();
    assert_eq!(clock.implicit_timezone(), 120);
    let dc = DynamicContext::new(&NoHost, clock);
    assert_eq!(dc.clock(), clock);
    assert_eq!(dc.implicit_timezone(), 120);
    let eval = |dc: &DynamicContext<'_>, text: &str| {
        strings(&plain(text).evaluate(dc, &Context::new()).unwrap())
    };
    // The same value at every call and in every evaluation.
    let now = "2026-09-10T02:28:43.456+02:00";
    assert_eq!(
        eval(&dc, "(current-dateTime(), current-dateTime())"),
        [now, now]
    );
    assert_eq!(eval(&dc, "string(current-dateTime())"), [now]);
    // The implicit timezone is that of values without one.
    let same_day = "xs:date('2026-09-10') = xs:date('2026-09-10+02:00')";
    assert_eq!(eval(&dc, same_day), ["true"]);
    let utc = DynamicContext::new(&NoHost, Clock::from_unix_millis(0, 0).unwrap());
    assert_eq!(eval(&utc, same_day), ["false"]);
    assert_eq!(eval(&utc, "current-dateTime()"), ["1970-01-01T00:00:00Z"]);
    // A date and time given as such is returned as given.
    let fixed = einvoice_xdm::DateTime::parse("2024-01-31T12:00:00Z").unwrap();
    let dc = DynamicContext::new(&NoHost, Clock::new(60, fixed).unwrap());
    assert_eq!(dc.current_date_time(), fixed);
    assert_eq!(eval(&dc, "current-dateTime()"), ["2024-01-31T12:00:00Z"]);
    // Out of range or without timezone: refused.
    assert_eq!(
        Clock::from_unix_millis(900, 0).unwrap_err().code(),
        ErrorCode::Unsupported
    );
    assert!(Clock::new(-841, fixed).is_err());
    assert!(Clock::new(840, fixed).is_ok());
    let floating = einvoice_xdm::DateTime::parse("2024-01-31T12:00:00").unwrap();
    assert_eq!(
        Clock::new(0, floating).unwrap_err().code(),
        ErrorCode::Unsupported
    );
}

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
#[test]
fn system_clock() {
    let clock = Clock::system(-300).unwrap();
    assert_eq!(clock.implicit_timezone(), -300);
    assert_eq!(clock.current_date_time().timezone(), Some(-300));
    let dc = DynamicContext::new(&NoHost, clock);
    let text = plain("string(current-dateTime())")
        .evaluate(&dc, &Context::new())
        .unwrap();
    assert!(strings(&text)[0].ends_with("-05:00"), "{text:?}");
    assert!(Clock::system(900).is_err());
}

#[test]
fn compiled_expressions_can_be_shared_between_threads() {
    fn shareable<T: Send + Sync>() {}
    shareable::<CompiledExpr>();
}

/// A host whose variable and function give the integer 300, whatever their static type
/// says.
struct ThreeHundred;

impl Host for ThreeHundred {
    fn variable(&self, _id: VariableId) -> Result<Rc<[Item]>, Error> {
        Ok(Rc::from(vec![Item::Atomic(AtomicValue::from(
            Integer::from(300),
        ))]))
    }

    fn call_function(
        &self,
        _id: FunctionId,
        _args: Vec<Argument>,
        _dc: &DynamicContext<'_>,
    ) -> Result<Vec<Item>, Error> {
        Ok(vec![Item::Atomic(AtomicValue::from(Integer::from(300)))])
    }

    fn document(&self, uri: &str, base: Option<&str>) -> Result<Node, Error> {
        NoHost.document(uri, base)
    }

    fn system_property(&self, _name: &QName) -> String {
        String::new()
    }
}

#[test]
fn variable_and_function_types_from_the_host() {
    // The value the host gives is the same xs:integer 300 each time; only its static type
    // differs, and with it what Saxon does with `xs:decimal` and `div`.
    let ns = namespaces();
    let decimal = parse_sequence_type("xs:decimal", &ns).unwrap();
    let integer = parse_sequence_type("xs:integer", &ns).unwrap();
    let run = |value_type: ValueType, text: &str| {
        let mut context = SimpleStaticContext::new();
        context.declare_typed_variable(QName::local("v"), Cardinality::AtMostOne, value_type);
        let dc = DynamicContext::new(&ThreeHundred, clock());
        strings(
            &compiled(text, &context)
                .unwrap()
                .evaluate(&dc, &Context::new())
                .unwrap(),
        )
        .join(" ")
    };
    let probe = "xs:decimal($v) instance of xs:integer, 1.1 div xs:decimal($v), ($v - 301) div 300";
    // Unknown: cast at run time (scale 0 kept), division chosen from the values (Saxon
    // strips a negative integer).
    assert_eq!(
        run(ValueType::unknown(), probe),
        "false 0.0036666666666666667 -0.00333333333333333333"
    );
    // A local variable is of the type of its type-checked value, xs:decimal here: no
    // cast, decimal division.
    let quotient = plain("count(/a) * 1000 div 10");
    let local = ValueType::local_variable(BindingValue::Select(&quotient), None);
    assert_eq!(
        run(local, probe),
        "true 0.0036666666666666667 -0.003333333333333333"
    );
    // A global variable is of the type inferred before type checking: arithmetic has no
    // type yet.
    let global = ValueType::global_variable(BindingValue::Select(&quotient), None);
    assert_eq!(
        run(global, probe),
        "false 0.0036666666666666667 -0.00333333333333333333"
    );
    // Declared xs:integer: the up-casting converter strips the trailing zeros.
    assert_eq!(
        run(
            ValueType::declared(Some(&integer)),
            "1.1 div xs:decimal($v)"
        ),
        "0.003666666666666666667"
    );
    // A literal select is a constant: the reference is the literal, whatever the host
    // gives.
    let literal = plain("1.5");
    assert_eq!(
        run(
            ValueType::local_variable(BindingValue::Select(&literal), None),
            "$v * 2"
        ),
        "3"
    );
    // A local variable declared xs:decimal with an xs:integer value: the cast is removed
    // while the instruction is type-checked alone (the variable is then of its declared
    // type), the division chosen once the body is (it is then an xs:integer).
    let product = plain("count(/a) * 100");
    let declared = ValueType::local_variable(BindingValue::Select(&product), Some(&decimal));
    assert_eq!(
        run(declared, probe),
        "true 0.0036666666666666667 -0.00333333333333333333"
    );
    // The declared result type of a function.
    let mut context = SimpleStaticContext::new();
    let name = QName {
        prefix: Some("f".to_owned()),
        namespace: Some("urn:f".to_owned()),
        local: "g".to_owned(),
    };
    context.declare_typed_function(name, 0, ValueType::declared(Some(&decimal)));
    let dc = DynamicContext::new(&ThreeHundred, clock());
    let result = compiled("xs:decimal(f:g()) instance of xs:integer", &context)
        .unwrap()
        .evaluate(&dc, &Context::new())
        .unwrap();
    assert_eq!(strings(&result), ["true"]);
}

#[test]
fn nodes_selected_in_place_are_those_of_the_evaluation() {
    let mut xml = String::from("<r a='1' b='2'>");
    for i in 0..30 {
        write!(xml, "<e n='{i}'>t<f/></e> ").unwrap();
    }
    xml.push_str("<!--c--></r>");
    let doc = Document::parse(xml.as_str(), None).unwrap();
    let r = doc.root().children().next().unwrap();
    let e = r.children().next().unwrap();
    let dc = DynamicContext::new(&NoHost, clock());
    for node in [&doc.root(), &r, &e] {
        let context = Context::for_item(Item::Node(node.clone()));
        for text in [
            "@*|*",
            "*",
            "@*",
            "node()",
            "*|@*",
            "e|node()",
            "..",
            ".//f|@n",
            "f[1]",
            "@*|..",
            ".",
            "*|comment()|processing-instruction()",
            "@*|*|text()|..",
            "@*|*|text()|..|comment()",
        ] {
            let expr = plain(text);
            let evaluated = expr.evaluate(&dc, &context).unwrap();
            match expr
                .select_in_place(&dc, &context, SelectionStorage::default())
                .unwrap()
            {
                Some(selection) => {
                    let selected: Vec<Item> = selection.iter().map(Item::Node).collect();
                    assert_eq!(selection.len(), evaluated.len(), "{text}");
                    assert_eq!(selection.is_empty(), evaluated.is_empty(), "{text}");
                    assert!(
                        selected
                            .iter()
                            .zip(&evaluated)
                            .all(|(a, b)| a.as_node() == b.as_node()),
                        "{text}"
                    );
                }
                // Predicates, a path: not read in place.
                None => assert!(
                    matches!(text, ".//f|@n" | "f[1]" | "@*|*|text()|..|comment()"),
                    "{text}"
                ),
            }
        }
    }
    // The context item must be a node, as for the evaluation.
    let atomic = Context::for_item(Item::Atomic(AtomicValue::Integer(Integer::from(1))));
    assert_eq!(
        plain("@*|*")
            .select_in_place(&dc, &atomic, SelectionStorage::default())
            .unwrap_err()
            .code(),
        plain("@*|*").evaluate(&dc, &atomic).unwrap_err().code()
    );
    assert_eq!(
        plain("*")
            .select_in_place(&dc, &Context::new(), SelectionStorage::default())
            .unwrap_err()
            .code(),
        ErrorCode::Xpdy0002
    );
}
