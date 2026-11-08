//! The reader's interface: names and namespaces, attributes, ranges, borrowed text,
//! the scope of prefixes, errors.

use std::borrow::Cow;

use einvoice_xml::{Error, Event, Reader, TextPos, XML_NAMESPACE, XMLNS_NAMESPACE, check};

const DOC: &str = "\u{FEFF}<?xml version='1.0'?>\n<i:Invoice xmlns:i='urn:i' xmlns='urn:d' id='a&amp;b' xml:lang='en'>\
<Line n=' 1 '>text &lt; more<![CDATA[<raw>]]></Line><e xmlns='' i:x='1'/><!-- c --><?pi data?></i:Invoice>";

#[test]
fn names_attributes_and_namespaces() {
    let mut reader = Reader::new(DOC);
    let mut log = Vec::new();
    while let Some(event) = reader.next_event().unwrap() {
        match event {
            Event::Start(start) => {
                let name = start.name();
                let attributes: Vec<String> = start
                    .attributes()
                    .map(|a| format!("{:?}:{}={}", a.name.namespace, a.name.local, a.value))
                    .collect();
                let declarations: Vec<(&str, &str)> = start.namespace_declarations().collect();
                log.push(format!(
                    "start {:?} {}:{} {attributes:?} {declarations:?} empty={}",
                    name.namespace,
                    name.prefix,
                    name.local,
                    start.is_empty()
                ));
                if name.local == "Invoice" {
                    assert_eq!(start.attribute(None, "id").map(Cow::as_ref), Some("a&b"));
                    assert_eq!(
                        start
                            .attribute(Some(XML_NAMESPACE), "lang")
                            .map(Cow::as_ref),
                        Some("en")
                    );
                    assert_eq!(start.attribute(Some("urn:d"), "id"), None);
                    assert_eq!(start.lookup_namespace("i"), Some("urn:i"));
                    assert_eq!(start.lookup_namespace(""), Some("urn:d"));
                    assert_eq!(start.lookup_namespace("xml"), Some(XML_NAMESPACE));
                    assert_eq!(start.lookup_namespace("xmlns"), Some(XMLNS_NAMESPACE));
                    assert_eq!(start.lookup_namespace("j"), None);
                    assert_eq!(&DOC[start.range()][..10], "<i:Invoice");
                    assert!(DOC[start.range()].ends_with("'en'>"));
                }
                if name.local == "e" {
                    // The default namespace is undeclared.
                    assert_eq!(start.lookup_namespace(""), None);
                    assert_eq!(start.lookup_namespace("i"), Some("urn:i"));
                }
            }
            Event::End(end) => {
                log.push(format!(
                    "end {:?} {} empty={}",
                    end.name().namespace,
                    end.name().local,
                    end.is_empty()
                ));
            }
            Event::Text(text) => log.push(format!("text {:?} cdata={}", text.value, text.cdata)),
            Event::Comment(comment) => log.push(format!("comment {:?}", comment.value)),
            Event::ProcessingInstruction(pi) => {
                log.push(format!("pi {} {:?}", pi.target, pi.value));
            }
        }
    }
    assert_eq!(
        log,
        [
            "start Some(\"urn:i\") i:Invoice [\"None:id=a&b\", \"Some(\\\"http://www.w3.org/XML/1998/namespace\\\"):lang=en\"] [(\"i\", \"urn:i\"), (\"\", \"urn:d\")] empty=false",
            "start Some(\"urn:d\") :Line [\"None:n= 1 \"] [] empty=false",
            "text \"text < more\" cdata=false",
            "text \"<raw>\" cdata=true",
            "end Some(\"urn:d\") Line empty=false",
            "start None :e [\"Some(\\\"urn:i\\\"):x=1\"] [(\"\", \"\")] empty=true",
            "end None e empty=true",
            "comment \" c \"",
            "pi pi Some(\"data\")",
            "end Some(\"urn:i\") Invoice empty=false",
        ]
    );
    // The end of the document, again and again.
    assert!(reader.next_event().unwrap().is_none());
    assert!(reader.next_event().unwrap().is_none());
}

#[test]
fn text_is_borrowed_unless_it_changes() {
    let doc = "<a b='plain' c='a&amp;b' d='x\ty'>plain<![CDATA[raw]]>&amp;x\r\n</a>";
    let mut reader = Reader::new(doc);
    let mut borrowed = Vec::new();
    while let Some(event) = reader.next_event().unwrap() {
        match event {
            Event::Start(start) => {
                for a in start.attributes() {
                    borrowed.push((a.name.local.to_owned(), matches!(a.value, Cow::Borrowed(_))));
                    // The raw value is always a slice of the text.
                    assert!(std::ptr::eq(
                        a.raw.as_ptr(),
                        doc[a.range.clone()]
                            .as_ptr()
                            .wrapping_add(a.name.local.len() + 2)
                    ));
                }
            }
            Event::Text(text) => borrowed.push((
                text.value.to_string(),
                matches!(text.value, Cow::Borrowed(_)),
            )),
            _ => {}
        }
    }
    assert_eq!(
        borrowed,
        [
            ("b".to_owned(), true),
            ("c".to_owned(), false),
            ("d".to_owned(), false),
            ("plain".to_owned(), true),
            ("raw".to_owned(), true),
            ("&x\n".to_owned(), false),
        ]
    );
}

#[test]
fn the_names_are_slices_of_the_text() {
    let doc = "<p:a xmlns:p='u' q='1'/>";
    let mut reader = Reader::new(doc);
    let Some(Event::Start(start)) = reader.next_event().unwrap() else {
        panic!("no start");
    };
    let name = start.name();
    assert!(std::ptr::eq(name.prefix.as_ptr(), doc[1..].as_ptr()));
    assert!(std::ptr::eq(name.local.as_ptr(), doc[3..].as_ptr()));
}

#[test]
fn depth_and_offsets() {
    let mut reader = Reader::new("<a><b/>x</a>");
    let mut depths = Vec::new();
    while let Some(event) = reader.next_event().unwrap() {
        let kind = match event {
            Event::Start(_) => "start",
            Event::End(_) => "end",
            _ => "other",
        };
        depths.push((kind, reader.depth(), reader.offset()));
    }
    assert_eq!(
        depths,
        [
            ("start", 1, 3),
            ("start", 2, 7),
            ("end", 2, 7),
            ("other", 1, 8),
            ("end", 1, 12)
        ]
    );
}

#[test]
fn an_error_is_returned_again() {
    let mut reader = Reader::new("<a>\n  <b></a>");
    assert!(matches!(reader.next_event(), Ok(Some(Event::Start(_)))));
    assert!(matches!(reader.next_event(), Ok(Some(Event::Text(_)))));
    assert!(matches!(reader.next_event(), Ok(Some(Event::Start(_)))));
    let error = reader.next_event().unwrap_err();
    assert_eq!(
        error,
        Error::UnexpectedCloseTag {
            expected: "b".to_owned(),
            actual: "a".to_owned(),
            pos: TextPos::new(2, 6)
        }
    );
    assert_eq!(error.to_string(), "expected 'b' tag, not 'a' at 2:6");
    assert_eq!(error.pos(), TextPos::new(2, 6));
    assert_eq!(reader.next_event().unwrap_err(), error);
    assert_eq!(reader.read_to_end(), Err(error));
}

#[test]
fn check_and_positions() {
    assert_eq!(check("<a/>"), Ok(()));
    assert_eq!(check("<!DOCTYPE a><a/>"), Err(Error::DtdDetected));
    assert_eq!(
        check("<a>&#xD800;</a>"),
        Err(Error::InvalidCharacterReference(TextPos::new(1, 4)))
    );
    let reader = Reader::new("ab\ncd");
    assert_eq!(reader.text_pos(4), TextPos::new(2, 2));
    assert_eq!(reader.text(), "ab\ncd");
}
