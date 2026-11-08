//! A loaded PDF: its cross-reference information, and the objects it resolves on demand
//! (ISO 32000-2, 7.3.10 and 7.5.7). Every object and object stream is parsed at most
//! once (cached); a reference cycle is an error.

use std::collections::BTreeMap;
use std::rc::Rc;

use crate::Limits;
use crate::error::{Error, Location, Result};
use crate::filter::{Budget, filters};
use crate::object::{Dict, ObjRef, Object, Stream};
use crate::parser::{Body, Parser, parse_unsigned};
use crate::xref::{self, XrefEntry};

/// The decoded data of an object stream and where each of its objects starts.
#[derive(Debug)]
struct ObjectStream {
    data: Vec<u8>,
    /// For each index: the object number and the byte range of the object in `data`,
    /// up to the next object (objects never overlap).
    objects: Vec<(u32, std::ops::Range<usize>)>,
}

#[derive(Debug)]
pub(crate) struct Document<'a> {
    data: &'a [u8],
    limits: &'a Limits,
    /// The version in the header, such as `1.7`.
    pub(crate) version: String,
    entries: BTreeMap<u32, XrefEntry>,
    pub(crate) trailer: Dict,
    objects: BTreeMap<u32, Rc<Object>>,
    /// The byte ranges of the objects parsed in the file, start to end: objects never
    /// overlap, so a file that makes them overlap (to have the same bytes parsed once per
    /// object) is refused at the first overlap.
    parsed: BTreeMap<usize, usize>,
    object_streams: BTreeMap<u32, Rc<ObjectStream>>,
    /// The objects being resolved, innermost last (cycle detection).
    resolving: Vec<u32>,
    budget: Budget,
}

/// The version of a `%PDF-1.x` or `%PDF-2.0` header at byte 0.
fn header(data: &[u8]) -> Result<String> {
    let rest = data.strip_prefix(b"%PDF-").ok_or(Error::NotPdf)?;
    let version = match rest {
        [b'1', b'.', minor @ b'0'..=b'7', ..] => format!("1.{}", char::from(*minor)),
        [b'2', b'.', b'0', ..] => "2.0".to_owned(),
        _ => return Err(Error::NotPdf),
    };
    // The version is followed by an end-of-line marker (7.5.2).
    match rest.get(3) {
        Some(b'\r' | b'\n') => Ok(version),
        _ => Err(Error::NotPdf),
    }
}

impl<'a> Document<'a> {
    pub(crate) fn load(data: &'a [u8], limits: &'a Limits) -> Result<Self> {
        let version = header(data)?;
        let mut budget = Budget::new(limits);
        let start = xref::startxref(data)?;
        let xref = xref::load(data, start, limits, &mut budget)?;
        Ok(Self {
            data,
            limits,
            version,
            entries: xref.entries,
            trailer: xref.trailer,
            objects: BTreeMap::new(),
            parsed: BTreeMap::new(),
            object_streams: BTreeMap::new(),
            resolving: Vec::new(),
            budget,
        })
    }

    pub(crate) fn limits(&self) -> &Limits {
        self.limits
    }

    /// The object `reference` points to; `null` when the object is free, absent, or has
    /// another generation number (7.3.10).
    pub(crate) fn get(&mut self, reference: ObjRef) -> Result<Rc<Object>> {
        let entry = self.entries.get(&reference.num).copied();
        let matches = match entry {
            None | Some(XrefEntry::Free) => false,
            Some(XrefEntry::InUse { generation, .. }) => generation == reference.generation,
            Some(XrefEntry::Compressed { .. }) => reference.generation == 0,
        };
        if !matches {
            return Ok(Rc::new(Object::Null));
        }
        if let Some(object) = self.objects.get(&reference.num) {
            return Ok(Rc::clone(object));
        }
        if self.resolving.contains(&reference.num) {
            return Err(Error::malformed(
                Some(Location::Object(reference.num)),
                "the object depends on itself (reference cycle)",
            ));
        }
        if self.resolving.len() >= self.limits.max_resolution_depth {
            return Err(Error::LimitExceeded {
                limit: "max_resolution_depth",
                value: self.limits.max_resolution_depth,
            });
        }
        self.resolving.push(reference.num);
        let result = match entry {
            Some(XrefEntry::InUse { offset, .. }) => self.parse_at(reference, offset),
            Some(XrefEntry::Compressed { stream, index }) => {
                self.parse_compressed(reference.num, stream, index)
            }
            None | Some(XrefEntry::Free) => Ok(Object::Null),
        };
        self.resolving.pop();
        let object = Rc::new(result?);
        self.objects.insert(reference.num, Rc::clone(&object));
        Ok(object)
    }

    /// `object` itself, or the object it refers to (following chains of references).
    pub(crate) fn resolve(&mut self, object: &Object) -> Result<Rc<Object>> {
        let mut current = match object {
            Object::Ref(reference) => self.get(*reference)?,
            direct => return Ok(Rc::new(direct.clone())),
        };
        let mut chain: Vec<u32> = Vec::new();
        if let Object::Ref(first) = object {
            chain.push(first.num);
        }
        while let Object::Ref(reference) = *current {
            if chain.contains(&reference.num) {
                return Err(Error::malformed(
                    Some(Location::Object(reference.num)),
                    "a chain of indirect references loops",
                ));
            }
            if chain.len() >= self.limits.max_resolution_depth {
                return Err(Error::LimitExceeded {
                    limit: "max_resolution_depth",
                    value: self.limits.max_resolution_depth,
                });
            }
            chain.push(reference.num);
            current = self.get(reference)?;
        }
        Ok(current)
    }

    /// An uncompressed object at a byte offset.
    fn parse_at(&mut self, reference: ObjRef, offset: usize) -> Result<Object> {
        if offset >= self.data.len() {
            return Err(Error::malformed(
                Some(Location::Object(reference.num)),
                format!("cross-reference offset {offset} beyond the end of the file"),
            ));
        }
        let mut p = Parser::new(self.data, offset, self.limits.max_depth);
        p.skip_blanks();
        let at = p.pos();
        let header = p.object_header()?;
        if header != reference {
            return Err(Error::malformed(
                Some(Location::File(at)),
                format!(
                    "the cross-reference entry of object {} {} points to object {} {}",
                    reference.num, reference.generation, header.num, header.generation
                ),
            ));
        }
        let object = match p.object_body()? {
            Body::Object(object) => object,
            Body::Stream(dict, start) => {
                let length = match dict.get(b"Length") {
                    Some(length) => match *self.resolve(length)? {
                        Object::Integer(length) => usize::try_from(length).map_err(|_| {
                            Error::malformed(
                                Some(Location::Object(reference.num)),
                                "negative /Length",
                            )
                        })?,
                        ref other => {
                            return Err(Error::malformed(
                                Some(Location::Object(reference.num)),
                                format!("stream /Length is {}, not an integer", other.kind()),
                            ));
                        }
                    },
                    None => {
                        return Err(Error::malformed(
                            Some(Location::Object(reference.num)),
                            "stream without /Length",
                        ));
                    }
                };
                let end = start
                    .checked_add(length)
                    .filter(|&end| end <= self.data.len())
                    .ok_or_else(|| {
                        Error::malformed(
                            Some(Location::Object(reference.num)),
                            "stream /Length beyond the end of the file",
                        )
                    })?;
                p.stream_end(end)?;
                Object::Stream(Stream {
                    dict,
                    data: start..end,
                })
            }
        };
        self.claim(at, p.pos(), reference.num)?;
        Ok(object)
    }

    /// Records that object `num` occupies `start..end` of the file; an error if another
    /// object parsed before overlaps it.
    fn claim(&mut self, start: usize, end: usize, num: u32) -> Result<()> {
        let before = self.parsed.range(..=start).next_back();
        let after = self.parsed.range(start..).next();
        if before.is_some_and(|(_, &e)| e > start) || after.is_some_and(|(&s, _)| s < end) {
            return Err(Error::malformed(
                Some(Location::Object(num)),
                "the object overlaps another object in the file",
            ));
        }
        self.parsed.insert(start, end);
        Ok(())
    }

    /// Object `num`, at `index` in object stream `stream` (7.5.7).
    fn parse_compressed(&mut self, num: u32, stream: u32, index: u32) -> Result<Object> {
        let objects = self.object_stream(stream)?;
        let found = usize::try_from(index)
            .ok()
            .and_then(|i| objects.objects.get(i).cloned());
        let Some((listed, range)) = found else {
            return Err(Error::malformed(
                Some(Location::Object(num)),
                format!("index {index} beyond the objects of object stream {stream}"),
            ));
        };
        if listed != num {
            return Err(Error::malformed(
                Some(Location::Object(num)),
                format!("index {index} of object stream {stream} holds object {listed}"),
            ));
        }
        // The object is parsed within its own range: the next object's bytes are not its.
        let mut p =
            Parser::in_object_stream(&objects.data[..range.end], stream, self.limits.max_depth);
        p.set_pos(range.start);
        let object = p.object()?;
        if let Object::Stream(_) = object {
            // Unreachable by construction (the parser makes no stream from a lone value),
            // kept as a guard: streams are never in object streams.
            return Err(p.error("stream in an object stream"));
        }
        Ok(object)
    }

    fn object_stream(&mut self, num: u32) -> Result<Rc<ObjectStream>> {
        if let Some(stream) = self.object_streams.get(&num) {
            return Ok(Rc::clone(stream));
        }
        let location = Some(Location::Object(num));
        match self.entries.get(&num) {
            Some(XrefEntry::InUse { .. }) => {}
            Some(XrefEntry::Compressed { .. }) => {
                return Err(Error::malformed(
                    location,
                    "an object stream is itself in an object stream",
                ));
            }
            None | Some(XrefEntry::Free) => {
                return Err(Error::malformed(location, "object stream not found"));
            }
        }
        let object = self.get(ObjRef { num, generation: 0 })?;
        let Object::Stream(stream) = &*object else {
            return Err(Error::malformed(location, "object stream is not a stream"));
        };
        if !stream.dict.has_name(b"Type", b"ObjStm") {
            return Err(Error::malformed(
                location,
                "object stream without /Type /ObjStm",
            ));
        }
        let count = self.count(&stream.dict, b"N", num)?;
        let first = self.count(&stream.dict, b"First", num)?;
        let data = self.stream_data(stream, num)?;
        if first > data.len() {
            return Err(Error::malformed(location, "/First beyond the decoded data"));
        }
        let mut p = Parser::in_object_stream(&data, num, self.limits.max_depth);
        // Each pair takes at least four bytes ("1 2 "), so a /N beyond that is malformed
        // and is refused before any allocation.
        if count > first / 4 + 1 {
            return Err(Error::malformed(
                location,
                format!("/N {count} too large for /First {first}"),
            ));
        }
        let mut objects = Vec::with_capacity(count);
        for _ in 0..count {
            p.skip_whitespace();
            let at = p.pos();
            let object = parse_unsigned(p.token()).and_then(|n| u32::try_from(n).ok());
            p.skip_whitespace();
            let offset = parse_unsigned(p.token()).and_then(|n| usize::try_from(n).ok());
            let (Some(object), Some(offset)) = (object, offset) else {
                return Err(Error::malformed(
                    Some(Location::ObjectStream {
                        stream: num,
                        offset: at,
                    }),
                    "expected an object number and an offset",
                ));
            };
            let start = first
                .checked_add(offset)
                .filter(|&start| start < data.len())
                .ok_or_else(|| {
                    Error::malformed(
                        Some(Location::ObjectStream {
                            stream: num,
                            offset: at,
                        }),
                        format!("object {object} offset beyond the decoded data"),
                    )
                })?;
            if p.pos() > first {
                return Err(Error::malformed(
                    Some(Location::ObjectStream {
                        stream: num,
                        offset: at,
                    }),
                    "object numbers and offsets run past /First",
                ));
            }
            objects.push((object, start));
        }
        // Each object ends where the next one (by offset) starts; two objects at one
        // offset would be parsed twice over the same bytes, so they are refused.
        let mut starts: Vec<usize> = objects.iter().map(|(_, start)| *start).collect();
        starts.sort_unstable();
        if let Some(pair) = starts.windows(2).find(|pair| pair[0] == pair[1]) {
            return Err(Error::malformed(
                location,
                format!(
                    "two objects at offset {} of the object stream",
                    pair[0] - first
                ),
            ));
        }
        let objects = objects
            .into_iter()
            .map(|(object, start)| {
                let next = starts.partition_point(|&s| s <= start);
                let end = starts.get(next).copied().unwrap_or(data.len());
                (object, start..end)
            })
            .collect();
        let stream = Rc::new(ObjectStream { data, objects });
        self.object_streams.insert(num, Rc::clone(&stream));
        Ok(stream)
    }

    fn count(&mut self, dict: &Dict, key: &[u8], num: u32) -> Result<usize> {
        let location = Some(Location::Object(num));
        let name = String::from_utf8_lossy(key);
        let Some(value) = dict.get(key) else {
            return Err(Error::malformed(location, format!("no /{name}")));
        };
        match *self.resolve(value)? {
            Object::Integer(value) => usize::try_from(value)
                .map_err(|_| Error::malformed(location, format!("negative /{name}"))),
            ref other => Err(Error::malformed(
                location,
                format!("/{name} is {}, not an integer", other.kind()),
            )),
        }
    }

    /// The decoded data of `stream` (object `num`).
    pub(crate) fn stream_data(&mut self, stream: &Stream, num: u32) -> Result<Vec<u8>> {
        let filters = filters(&stream.dict, &mut |object| {
            Ok((*self.resolve(object)?).clone())
        })
        .map_err(|e| e.at(Location::Object(num)))?;
        self.budget
            .decode(&self.data[stream.data.clone()], &filters)
            .map_err(|e| e.at(Location::Object(num)))
    }
}

#[cfg(test)]
mod tests {
    use super::header;

    #[test]
    fn headers() {
        assert_eq!(header(b"%PDF-1.7\n").unwrap(), "1.7");
        assert_eq!(header(b"%PDF-1.0\r\n").unwrap(), "1.0");
        assert_eq!(header(b"%PDF-2.0\r").unwrap(), "2.0");
        assert!(header(b"%PDF-1.8\n").is_err());
        assert!(header(b"%PDF-1.7").is_err());
        assert!(header(b"%PDF-1.7 \n").is_err());
        assert!(header(b" %PDF-1.7\n").is_err());
        assert!(header(b"%PDF-2.1\n").is_err());
        assert!(header(b"").is_err());
    }
}
