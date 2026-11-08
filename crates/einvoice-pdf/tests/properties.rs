//! Property tests: the reader never panics and always finishes quickly, on arbitrary
//! bytes and on random edits of valid files; random contents round-trip through every
//! layout; random valid object syntax is accepted.

// Integration tests are only ever built with `cfg(test)`; saying so lets clippy apply its
// test exemptions (unwrap, expect, panic) to the helpers outside `#[test]` functions too.
#![cfg(test)]

use std::fmt::Write as _;
use std::time::{Duration, Instant};

use einvoice_pdf::{INVOICE_FILE_NAMES, attachments, extract};
use einvoice_testkit::pdf::{Fixture, LAYOUTS, Layout, literal};
use proptest::prelude::*;

/// Far above what any of these inputs takes (well under a millisecond in release
/// builds), to catch runaway work rather than to benchmark.
const BOUND: Duration = Duration::from_secs(2);

fn bounded<T>(f: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    let out = f();
    assert!(start.elapsed() < BOUND, "took {:?}", start.elapsed());
    out
}

fn layout() -> impl Strategy<Value = Layout> {
    (0..LAYOUTS.len()).prop_map(|i| LAYOUTS[i])
}

#[derive(Debug, Clone)]
enum Edit {
    Delete(usize),
    Insert(usize, u8),
    Overwrite(usize, u8),
    /// Copies `len` bytes from the first position to the second.
    Copy(usize, usize, usize),
    Truncate(usize),
}

fn edit() -> impl Strategy<Value = Edit> {
    prop_oneof![
        any::<usize>().prop_map(Edit::Delete),
        (any::<usize>(), any::<u8>()).prop_map(|(i, b)| Edit::Insert(i, b)),
        (any::<usize>(), any::<u8>()).prop_map(|(i, b)| Edit::Overwrite(i, b)),
        (any::<usize>(), any::<usize>(), 1usize..64).prop_map(|(a, b, n)| Edit::Copy(a, b, n)),
        any::<usize>().prop_map(Edit::Truncate),
    ]
}

fn apply(pdf: &mut Vec<u8>, edit: &Edit) {
    if pdf.is_empty() {
        return;
    }
    let n = pdf.len();
    match *edit {
        Edit::Delete(i) => {
            pdf.remove(i % n);
        }
        Edit::Insert(i, b) => pdf.insert(i % (n + 1), b),
        Edit::Overwrite(i, b) => pdf[i % n] = b,
        Edit::Copy(from, to, len) => {
            let from = from % n;
            let to = to % n;
            let len = len.min(n - from).min(n - to);
            pdf.copy_within(from..from + len, to);
        }
        Edit::Truncate(i) => pdf.truncate(i % n),
    }
}

/// A random valid PDF object, as text, nested at most `depth` deep.
fn object(depth: u32) -> BoxedStrategy<String> {
    let leaf = prop_oneof![
        Just("null".to_owned()),
        Just("true".to_owned()),
        Just("false".to_owned()),
        any::<i32>().prop_map(|i| i.to_string()),
        (any::<i16>(), 0u16..1000).prop_map(|(a, b)| format!("{a}.{b:03}")),
        Just("-.5".to_owned()),
        "[a-zA-Z0-9_.]{0,8}".prop_map(|n| format!("/{n}")),
        any::<u8>().prop_map(|b| format!("/a#{b:02X}b")),
        "[ -~]{0,12}".prop_map(|t| literal(&t)),
        Just("(nested (paren) \\(escaped\\) \\101\\n\\\r\nend)".to_owned()),
        proptest::collection::vec(any::<u8>(), 0..8).prop_map(|bytes| {
            let hex = bytes.iter().fold(String::new(), |mut out, b| {
                write!(out, "{b:02x} ").unwrap();
                out
            });
            format!("<{hex}>")
        }),
        (50u32..60, 0u16..3).prop_map(|(n, g)| format!("{n} {g} R")),
    ];
    leaf.prop_recursive(depth, 64, 6, |inner| {
        prop_oneof![
            proptest::collection::vec(inner.clone(), 0..6)
                .prop_map(|items| format!("[{}]", items.join(" "))),
            proptest::collection::vec(("[A-Za-z]{1,6}", inner), 0..6).prop_map(|entries| {
                // Keys are unique in a dictionary: number them.
                let body =
                    entries
                        .iter()
                        .enumerate()
                        .fold(String::new(), |mut out, (i, (k, v))| {
                            write!(out, "/{k}{i} {v}\n% comment\n").unwrap();
                            out
                        });
                format!("<<{body}>>")
            }),
        ]
    })
    .boxed()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn arbitrary_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..2048)) {
        bounded(|| {
            let _ = extract(&bytes);
            let _ = attachments(&bytes);
        });
    }

    #[test]
    fn arbitrary_bodies_never_panic(
        body in proptest::collection::vec(any::<u8>(), 0..2048),
        offset in any::<usize>(),
    ) {
        let mut pdf = b"%PDF-1.7\n".to_vec();
        pdf.extend_from_slice(&body);
        let offset = offset % pdf.len();
        pdf.extend_from_slice(format!("\nstartxref\n{offset}\n%%EOF\n").as_bytes());
        bounded(|| {
            let _ = extract(&pdf);
        });
    }

    #[test]
    fn edited_files_never_panic(
        layout in layout(),
        compress in any::<bool>(),
        edits in proptest::collection::vec(edit(), 1..6),
    ) {
        let mut pdf = Fixture { layout, compress, ..Fixture::default() }.build();
        for edit in &edits {
            apply(&mut pdf, edit);
        }
        bounded(|| {
            if let Ok(embedded) = extract(&pdf) {
                assert!(INVOICE_FILE_NAMES.contains(&embedded.xml.file_name.as_str()));
            }
            let _ = attachments(&pdf);
        });
    }

    #[test]
    fn contents_round_trip(
        layout in layout(),
        compress in any::<bool>(),
        name in 0..INVOICE_FILE_NAMES.len(),
        data in proptest::collection::vec(any::<u8>(), 0..4096),
    ) {
        let name = INVOICE_FILE_NAMES[name];
        let pdf = Fixture {
            layout,
            compress,
            name,
            data: &data,
            checksum: None,
            ..Fixture::default()
        }
        .build();
        let embedded = extract(&pdf).unwrap();
        prop_assert_eq!(&embedded.xml.data, &data);
        prop_assert_eq!(embedded.xml.file_name.as_str(), name);
        prop_assert_eq!(embedded.xml.size, Some(u64::try_from(data.len()).unwrap()));
    }

    #[test]
    fn valid_object_syntax_is_read(value in object(4), layout in layout()) {
        // A random value in the catalog (in an object stream for some layouts): the
        // reader parses it, then finds the invoice.
        let pdf = Fixture {
            layout,
            catalog_extra: format!("/Extra {value}"),
            ..Fixture::default()
        }
        .build();
        let embedded = bounded(|| extract(&pdf)).unwrap();
        prop_assert_eq!(embedded.xml.data, einvoice_testkit::pdf::INVOICE);
    }
}
