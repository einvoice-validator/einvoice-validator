//! The cross-reference information (ISO 32000-2, 7.5.4 to 7.5.8): the last `startxref`,
//! then every section of the `/Prev` chain, newest first, as classic `xref` tables,
//! cross-reference streams, or both (hybrid files, `/XRefStm`).

use std::collections::{BTreeMap, BTreeSet};

use crate::Limits;
use crate::error::{Error, Location, Result};
use crate::filter::{Budget, filters};
use crate::object::{Dict, Object};
use crate::parser::{Body, Parser, is_whitespace};

/// Where an object is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum XrefEntry {
    /// Free: a reference to it is a reference to `null`.
    Free,
    /// At a byte offset in the file, with its generation number.
    InUse { offset: usize, generation: u16 },
    /// In an object stream, at an index (generation 0).
    Compressed { stream: u32, index: u32 },
}

#[derive(Debug)]
pub(crate) struct Xref {
    pub(crate) entries: BTreeMap<u32, XrefEntry>,
    /// The trailer dictionary of the newest section.
    pub(crate) trailer: Dict,
}

/// One section: its entries in the order read, its trailer dictionary, and the bytes
/// it occupies in the file.
struct Section {
    entries: Vec<(u32, XrefEntry)>,
    trailer: Dict,
    span: (usize, usize),
}

/// The byte ranges of the sections read: sections never overlap, so a chain that makes
/// them overlap (to have the same bytes parsed once per section) is refused.
#[derive(Default)]
struct Spans(BTreeMap<usize, usize>);

impl Spans {
    fn claim(&mut self, (start, end): (usize, usize)) -> Result<()> {
        let before = self.0.range(..=start).next_back();
        let after = self.0.range(start..).next();
        if before.is_some_and(|(_, &e)| e > start) || after.is_some_and(|(&s, _)| s < end) {
            return Err(Error::malformed(
                Some(Location::File(start)),
                "the cross-reference section overlaps another one",
            ));
        }
        self.0.insert(start, end);
        Ok(())
    }
}

const STARTXREF: &[u8] = b"startxref";
const EOF_MARKER: &[u8] = b"%%EOF";
/// The window at the end of the file in which `startxref` is looked for.
const TAIL: usize = 1024;

/// The offset given by the last `startxref` of the file, which must end with `%%EOF`
/// (optionally followed by white space).
pub(crate) fn startxref(data: &[u8]) -> Result<usize> {
    let end = data.len() - data.iter().rev().take_while(|&&b| is_whitespace(b)).count();
    if !data[..end].ends_with(EOF_MARKER) {
        return Err(Error::malformed(
            Some(Location::File(end)),
            "the file does not end with %%EOF",
        ));
    }
    let eof = end - EOF_MARKER.len();
    let window = eof.saturating_sub(TAIL);
    let Some(found) = data[window..eof]
        .windows(STARTXREF.len())
        .rposition(|w| w == STARTXREF)
    else {
        return Err(Error::malformed(
            Some(Location::File(eof)),
            "no `startxref` in the last 1024 bytes before %%EOF",
        ));
    };
    let mut p = Parser::new(data, window + found + STARTXREF.len(), 1);
    let offset = p.unsigned()?;
    p.skip_blanks();
    if p.pos() != eof {
        return Err(p.error("expected %%EOF after the `startxref` offset"));
    }
    usize::try_from(offset).map_err(|_| p.error("`startxref` offset out of range"))
}

/// Every section of the chain that starts at `start`, merged: for each object, the entry
/// of the newest section that lists it.
pub(crate) fn load(
    data: &[u8],
    start: usize,
    limits: &Limits,
    budget: &mut Budget,
) -> Result<Xref> {
    let mut entries = BTreeMap::new();
    let mut newest: Option<Dict> = None;
    let mut visited = BTreeSet::new();
    let mut spans = Spans::default();
    let mut next = Some(start);
    while let Some(offset) = next {
        if visited.len() >= limits.max_xref_sections {
            return Err(Error::LimitExceeded {
                limit: "max_xref_sections",
                value: limits.max_xref_sections,
            });
        }
        if !visited.insert(offset) {
            return Err(Error::malformed(
                Some(Location::File(offset)),
                "the /Prev chain of cross-reference sections loops",
            ));
        }
        let section = section(data, offset, limits, budget)?;
        spans.claim(section.span)?;
        if section.trailer.get(b"Encrypt").is_some() {
            return Err(Error::Encrypted);
        }
        let hybrid = match section.trailer.get(b"XRefStm") {
            None => None,
            Some(Object::Integer(stream_offset)) => {
                let stream_offset = file_offset(*stream_offset, offset, "/XRefStm")?;
                if !visited.insert(stream_offset) {
                    return Err(Error::malformed(
                        Some(Location::File(stream_offset)),
                        "/XRefStm points to a cross-reference section already read",
                    ));
                }
                let stream = section_at(data, stream_offset, limits, budget, Kind::Stream)?;
                spans.claim(stream.span)?;
                if stream.trailer.get(b"Encrypt").is_some() {
                    return Err(Error::Encrypted);
                }
                Some(stream.entries)
            }
            Some(other) => {
                return Err(Error::malformed(
                    Some(Location::File(offset)),
                    format!("/XRefStm is {}, not an integer", other.kind()),
                ));
            }
        };
        // In a hybrid section (7.5.8.4), the table comes first and the stream only adds
        // the objects the table does not list: an entry the table lists as free stays
        // free, as poppler and MuPDF read it. Producers mark the objects of object
        // streams free in an older table and list them in a newer section's stream,
        // which the newest-first order below handles.
        for (num, entry) in section
            .entries
            .into_iter()
            .chain(hybrid.into_iter().flatten())
        {
            entries.entry(num).or_insert(entry);
        }
        next = match section.trailer.get(b"Prev") {
            None => None,
            Some(Object::Integer(prev)) => Some(file_offset(*prev, offset, "/Prev")?),
            Some(other) => {
                return Err(Error::malformed(
                    Some(Location::File(offset)),
                    format!("/Prev is {}, not an integer", other.kind()),
                ));
            }
        };
        if newest.is_none() {
            newest = Some(section.trailer);
        }
    }
    let trailer = newest.unwrap_or_default();
    Ok(Xref { entries, trailer })
}

fn file_offset(value: i64, section: usize, key: &str) -> Result<usize> {
    usize::try_from(value).map_err(|_| {
        Error::malformed(
            Some(Location::File(section)),
            format!("{key} {value} is not a byte offset"),
        )
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Any,
    Stream,
}

fn section(data: &[u8], offset: usize, limits: &Limits, budget: &mut Budget) -> Result<Section> {
    section_at(data, offset, limits, budget, Kind::Any)
}

fn section_at(
    data: &[u8],
    offset: usize,
    limits: &Limits,
    budget: &mut Budget,
    kind: Kind,
) -> Result<Section> {
    if offset >= data.len() {
        return Err(Error::malformed(
            Some(Location::File(offset)),
            "cross-reference offset beyond the end of the file",
        ));
    }
    let mut p = Parser::new(data, offset, limits.max_depth);
    p.skip_blanks();
    let start = p.pos();
    let (entries, trailer) = if kind == Kind::Any && data[start..].starts_with(b"xref") {
        table(&mut p)?
    } else if p.peek().is_some_and(|b| b.is_ascii_digit()) {
        stream(data, &mut p, budget)?
    } else {
        return Err(p.error(
            "cross-reference offset points to neither `xref` nor a cross-reference stream",
        ));
    };
    let section = Section {
        entries,
        trailer,
        span: (start, p.pos()),
    };
    let mut seen = BTreeSet::new();
    for (num, _) in &section.entries {
        if !seen.insert(*num) {
            return Err(Error::malformed(
                Some(Location::File(offset)),
                format!("object {num} listed twice in one cross-reference section"),
            ));
        }
    }
    Ok(section)
}

/// A classic cross-reference table and its trailer (7.5.4, 7.5.5).
fn table(p: &mut Parser<'_>) -> Result<(Vec<(u32, XrefEntry)>, Dict)> {
    p.expect_keyword("xref")?;
    let mut entries = Vec::new();
    loop {
        if p.eat_keyword("trailer") {
            break;
        }
        let at = p.pos();
        let first = p.unsigned()?;
        let count = p.unsigned()?;
        let out_of_range = || {
            Error::malformed(
                Some(Location::File(at)),
                format!("cross-reference subsection {first} {count} out of range"),
            )
        };
        let first = u32::try_from(first).map_err(|_| out_of_range())?;
        let count = u32::try_from(count).map_err(|_| out_of_range())?;
        first.checked_add(count).ok_or_else(out_of_range)?;
        for i in 0..count {
            let entry_at = p.pos();
            let offset = p.unsigned()?;
            let generation = p.unsigned()?;
            p.skip_whitespace();
            let kind = p.token();
            let bad = |what: &str| {
                Error::malformed(
                    Some(Location::File(entry_at)),
                    format!("cross-reference entry: {what}"),
                )
            };
            let entry = match kind {
                b"n" => XrefEntry::InUse {
                    offset: usize::try_from(offset).map_err(|_| bad("offset out of range"))?,
                    generation: u16::try_from(generation)
                        .map_err(|_| bad("generation number out of range"))?,
                },
                // The generation of a free entry only matters to a writer reusing the
                // number; some writers put 65536 on the head of the free list.
                b"f" => XrefEntry::Free,
                _ => return Err(bad("type is neither `n` nor `f`")),
            };
            entries.push((first + i, entry));
        }
    }
    let at = p.pos();
    match p.object()? {
        Object::Dict(trailer) => Ok((entries, trailer)),
        other => Err(Error::malformed(
            Some(Location::File(at)),
            format!("the trailer is {}, not a dictionary", other.kind()),
        )),
    }
}

/// A cross-reference stream (7.5.8). Its dictionary may hold no indirect reference: it
/// is read before any object can be located.
fn stream(
    data: &[u8],
    p: &mut Parser<'_>,
    budget: &mut Budget,
) -> Result<(Vec<(u32, XrefEntry)>, Dict)> {
    let at = p.pos();
    let header = p.object_header()?;
    let location = Some(Location::Object(header.num));
    let Body::Stream(dict, start) = p.object_body()? else {
        return Err(Error::malformed(
            Some(Location::File(at)),
            "cross-reference offset points to an object that is not a stream",
        ));
    };
    if !dict.has_name(b"Type", b"XRef") {
        return Err(Error::malformed(
            Some(Location::File(at)),
            "cross-reference offset points to a stream whose /Type is not /XRef",
        ));
    }
    let length = match dict.get(b"Length") {
        Some(Object::Integer(length)) => {
            usize::try_from(*length).map_err(|_| Error::malformed(location, "negative /Length"))?
        }
        Some(Object::Ref(_)) => {
            return Err(Error::malformed(
                location,
                "the /Length of a cross-reference stream must be a direct integer",
            ));
        }
        _ => {
            return Err(Error::malformed(
                location,
                "stream without an integer /Length",
            ));
        }
    };
    let end = start
        .checked_add(length)
        .filter(|&end| end <= data.len())
        .ok_or_else(|| Error::malformed(location, "stream /Length beyond the end of the file"))?;
    p.stream_end(end)?;
    let filters = filters(&dict, &mut |object| match object {
        Object::Ref(_) => Err(Error::malformed(
            location,
            "indirect reference in the filters of a cross-reference stream",
        )),
        other => Ok(other.clone()),
    })
    .map_err(|e| e.at(Location::Object(header.num)))?;
    let decoded = budget
        .decode(&data[start..end], &filters)
        .map_err(|e| e.at(Location::Object(header.num)))?;
    let entries =
        stream_entries(&dict, &decoded).map_err(|e| e.at(Location::Object(header.num)))?;
    Ok((entries, dict))
}

fn integer(object: Option<&Object>, what: &str) -> Result<i64> {
    match object {
        Some(Object::Integer(value)) => Ok(*value),
        Some(other) => Err(Error::malformed(
            None,
            format!("{what} is {}, not an integer", other.kind()),
        )),
        None => Err(Error::malformed(None, format!("no {what}"))),
    }
}

fn integers(object: Option<&Object>, what: &str) -> Result<Vec<i64>> {
    match object {
        Some(Object::Array(items)) => items
            .iter()
            .map(|item| integer(Some(item), &format!("an element of {what}")))
            .collect(),
        Some(other) => Err(Error::malformed(
            None,
            format!("{what} is {}, not an array", other.kind()),
        )),
        None => Err(Error::malformed(None, format!("no {what}"))),
    }
}

/// The entries of a decoded cross-reference stream, from its `/W`, `/Size` and `/Index`.
fn stream_entries(dict: &Dict, data: &[u8]) -> Result<Vec<(u32, XrefEntry)>> {
    let widths = integers(dict.get(b"W"), "/W")?;
    let [w0, w1, w2] = widths.as_slice() else {
        return Err(Error::malformed(None, "/W does not have three elements"));
    };
    let mut w = [0usize; 3];
    for (slot, value) in w.iter_mut().zip([w0, w1, w2]) {
        *slot = usize::try_from(*value)
            .ok()
            .filter(|&v| v <= 8)
            .ok_or_else(|| {
                Error::malformed(None, format!("/W field width {value} is not 0 to 8"))
            })?;
    }
    let row = w[0] + w[1] + w[2];
    if row == 0 {
        return Err(Error::malformed(None, "/W fields are all zero"));
    }
    let size = integer(dict.get(b"Size"), "/Size")?;
    let index = match dict.get(b"Index") {
        None => vec![0, size],
        index => integers(index, "/Index")?,
    };
    let (pairs, odd) = index.as_chunks::<2>();
    if !odd.is_empty() {
        return Err(Error::malformed(
            None,
            "/Index has an odd number of elements",
        ));
    }
    let mut ranges = Vec::new();
    let mut rows = 0usize;
    for pair in pairs {
        let bad = || {
            Error::malformed(
                None,
                format!("/Index range {} {} out of range", pair[0], pair[1]),
            )
        };
        let first = u32::try_from(pair[0]).map_err(|_| bad())?;
        let count = u32::try_from(pair[1]).map_err(|_| bad())?;
        first.checked_add(count).ok_or_else(bad)?;
        rows = rows
            .checked_add(usize::try_from(count).map_err(|_| bad())?)
            .ok_or_else(bad)?;
        ranges.push((first, count));
    }
    if rows.checked_mul(row) != Some(data.len()) {
        return Err(Error::malformed(
            None,
            format!(
                "cross-reference stream data has {} bytes, /W and /Index give {rows} rows of {row}",
                data.len()
            ),
        ));
    }
    let mut entries = Vec::with_capacity(rows);
    let mut chunks = data.chunks_exact(row);
    for (first, count) in ranges {
        for i in 0..count {
            let Some(chunk) = chunks.next() else {
                return Err(Error::malformed(
                    None,
                    "cross-reference stream data too short",
                ));
            };
            let (f0, rest) = chunk.split_at(w[0]);
            let (f1, f2) = rest.split_at(w[1]);
            let kind = if w[0] == 0 { 1 } else { be(f0) };
            let a = be(f1);
            let b = be(f2);
            let num = first + i;
            let bad = |what: &str| {
                Error::malformed(
                    None,
                    format!("cross-reference stream entry of object {num}: {what}"),
                )
            };
            let entry = match kind {
                0 => XrefEntry::Free,
                1 => XrefEntry::InUse {
                    offset: usize::try_from(a).map_err(|_| bad("offset out of range"))?,
                    generation: u16::try_from(b).map_err(|_| bad("generation out of range"))?,
                },
                2 => XrefEntry::Compressed {
                    stream: u32::try_from(a)
                        .map_err(|_| bad("object stream number out of range"))?,
                    index: u32::try_from(b).map_err(|_| bad("index out of range"))?,
                },
                other => {
                    return Err(Error::unsupported(
                        None,
                        format!("cross-reference stream entry type {other} (object {num})"),
                    ));
                }
            };
            entries.push((num, entry));
        }
    }
    Ok(entries)
}

/// A big-endian unsigned integer of at most 8 bytes.
fn be(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0, |acc, &b| acc << 8 | u64::from(b))
}
