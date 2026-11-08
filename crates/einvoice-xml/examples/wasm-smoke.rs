//! A smoke test that runs on any target, `wasm32-unknown-unknown` included (no clock, no
//! file system, no output): reads a document with every construct the reader supports
//! and malformed ones, and returns 0 when every result is the expected one, else the
//! number of the first wrong check.
//!
//! ```text
//! cargo run -p einvoice-xml --example wasm-smoke; echo $?
//! cargo build --release -p einvoice-xml --example wasm-smoke --target wasm32-unknown-unknown
//! wasmtime run --invoke main target/wasm32-unknown-unknown/release/examples/wasm-smoke.wasm 0 0
//! ```

use std::process::ExitCode;

use einvoice_xml::{Error, Event, Reader, TextPos};

const XML: &str = "\u{FEFF}<?xml version='1.0' encoding='UTF-8'?>\r\n<!-- c\r\n -->\
<i:Invoice xmlns:i='urn:i' xmlns='urn:d' a='x&#9;y\tz' xml:lang='en'>\
<ID>1 &amp; 2</ID><![CDATA[<raw>]]><?pi data?><e xmlns=''/></i:Invoice>";

fn main() -> ExitCode {
    ExitCode::from(run())
}

/// The events of `text` as one string, or the error.
fn events(text: &str) -> Result<String, Error> {
    let mut reader = Reader::new(text);
    let mut out = String::new();
    while let Some(event) = reader.next_event()? {
        let line = match event {
            Event::Start(s) => {
                let name = s.name();
                let attributes: Vec<String> = s
                    .attributes()
                    .map(|a| format!("{}={}", a.name.local, a.value))
                    .collect();
                format!(
                    "<{}{}{}>",
                    name.namespace.unwrap_or("-"),
                    name.local,
                    attributes.concat()
                )
            }
            Event::End(e) => format!("</{}>", e.name().local),
            Event::Text(t) => format!("[{}]", t.value),
            Event::Comment(c) => format!("!{}", c.value),
            Event::ProcessingInstruction(p) => format!("?{}", p.target),
        };
        out.push_str(&line);
    }
    Ok(out)
}

/// 0 when every check passes, else the number of the first failing one (from 1).
fn run() -> u8 {
    let checks: [(&str, Result<String, Error>); 5] = [
        (
            XML,
            Ok("! c\n <urn:iInvoicea=x\ty zlang=en><urn:dID>[1 & 2]</ID>[<raw>]?pi<-e></e></Invoice>"
                .to_owned()),
        ),
        (
            "<a>\n<b></a>",
            Err(Error::UnexpectedCloseTag {
                expected: "b".to_owned(),
                actual: "a".to_owned(),
                pos: TextPos::new(2, 4),
            }),
        ),
        ("<!DOCTYPE a><a/>", Err(Error::DtdDetected)),
        (
            "<p:a/>",
            Err(Error::UnknownNamespace("p".to_owned(), TextPos::new(1, 2))),
        ),
        (
            "<a>&#xD800;</a>",
            Err(Error::InvalidCharacterReference(TextPos::new(1, 4))),
        ),
    ];
    for (number, (text, expected)) in (1..).zip(checks) {
        if events(text) != expected {
            return number;
        }
    }
    0
}
