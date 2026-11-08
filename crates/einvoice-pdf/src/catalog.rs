//! The document catalog (ISO 32000-2, 7.7.2): its `/Names /EmbeddedFiles` name tree
//! (7.9.6), its associated files `/AF` (14.13), the file specifications they hold
//! (7.11.3) and their embedded file streams (7.11.4).

use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use crate::document::Document;
use crate::error::{Error, Location, Result};
use crate::md5;
use crate::object::{Dict, ObjRef, Object, display_name};
use crate::text;
use crate::{AfRelationship, Attachment};

/// A file specification with an embedded file, before its stream is decoded.
#[derive(Debug, Clone)]
pub(crate) struct FileSpec {
    pub(crate) file_name: String,
    pub(crate) key: Option<String>,
    pub(crate) in_associated_files: bool,
    pub(crate) relationship: Option<AfRelationship>,
    pub(crate) description: Option<String>,
    pub(crate) stream: ObjRef,
}

fn malformed(location: Option<Location>, message: impl Into<String>) -> Error {
    Error::malformed(location, message)
}

fn location_of(object: &Object) -> Option<Location> {
    match object {
        Object::Ref(reference) => Some(Location::Object(reference.num)),
        _ => None,
    }
}

/// `object`, resolved, as a dictionary.
fn dict(doc: &mut Document<'_>, object: &Object, what: &str) -> Result<Rc<Object>> {
    let resolved = doc.resolve(object)?;
    match &*resolved {
        Object::Dict(_) => Ok(resolved),
        other => Err(malformed(
            location_of(object),
            format!("{what} is {}, not a dictionary", other.kind()),
        )),
    }
}

const EMPTY: &Dict = &Dict(BTreeMap::new());

fn as_dict(object: &Object) -> &Dict {
    match object {
        Object::Dict(dict) => dict,
        // Only called on objects checked by `dict`.
        _ => EMPTY,
    }
}

/// The document catalog, from the trailer's `/Root`.
pub(crate) fn catalog(doc: &mut Document<'_>) -> Result<Rc<Object>> {
    let Some(root) = doc.trailer.get(b"Root").cloned() else {
        return Err(malformed(None, "the trailer has no /Root"));
    };
    let catalog = dict(doc, &root, "the document catalog (/Root)")?;
    if !as_dict(&catalog).has_name(b"Type", b"Catalog") {
        return Err(malformed(
            location_of(&root),
            "the document catalog (/Root) does not have /Type /Catalog",
        ));
    }
    Ok(catalog)
}

/// A text string value, decoded.
fn text_value(object: &Object, what: &str, location: Option<Location>) -> Result<String> {
    match object {
        Object::String(bytes) => {
            text::decode(bytes).map_err(|message| malformed(location, format!("{what}: {message}")))
        }
        other => Err(malformed(
            location,
            format!("{what} is {}, not a string", other.kind()),
        )),
    }
}

/// Every file specification with an embedded file: those of the `/EmbeddedFiles` name
/// tree in key order, then those of `/AF` not already listed (one embedded stream under
/// one file name is one entry, flagged as in both).
pub(crate) fn file_specs(doc: &mut Document<'_>, catalog: &Dict) -> Result<Vec<FileSpec>> {
    let mut specs = Specs::default();
    if let Some(names) = catalog.get(b"Names").cloned() {
        let names = dict(doc, &names, "the catalog's /Names")?;
        if let Some(tree) = as_dict(&names).get(b"EmbeddedFiles").cloned() {
            for (key, value) in name_tree(doc, &tree)? {
                let spec = file_spec(doc, &value, Some(key))?.ok_or_else(|| {
                    malformed(
                        location_of(&value),
                        "a file specification of /EmbeddedFiles has no /EF (no embedded file)",
                    )
                })?;
                specs.push(doc, spec)?;
            }
        }
    }
    if let Some(af) = catalog.get(b"AF").cloned() {
        let array = doc.resolve(&af)?;
        let Object::Array(items) = &*array else {
            return Err(malformed(
                location_of(&af),
                format!("the catalog's /AF is {}, not an array", array.kind()),
            ));
        };
        for item in items {
            // An associated file without /EF is an external file: not embedded, skipped.
            if let Some(mut spec) = file_spec(doc, item, None)? {
                spec.in_associated_files = true;
                specs.push(doc, spec)?;
            }
        }
    }
    Ok(specs.list)
}

/// The file specifications found so far, indexed by embedded stream and file name.
#[derive(Default)]
struct Specs {
    list: Vec<FileSpec>,
    index: BTreeMap<(ObjRef, String), usize>,
}

impl Specs {
    fn push(&mut self, doc: &Document<'_>, spec: FileSpec) -> Result<()> {
        let key = (spec.stream, spec.file_name.clone());
        if let Some(&i) = self.index.get(&key) {
            let known = &mut self.list[i];
            known.in_associated_files |= spec.in_associated_files;
            if known.key.is_none() {
                known.key = spec.key;
            }
            if known.relationship.is_none() {
                known.relationship = spec.relationship;
            }
            if known.description.is_none() {
                known.description = spec.description;
            }
            return Ok(());
        }
        if self.list.len() >= doc.limits().max_attachments {
            return Err(Error::LimitExceeded {
                limit: "max_attachments",
                value: doc.limits().max_attachments,
            });
        }
        self.index.insert(key, self.list.len());
        self.list.push(spec);
        Ok(())
    }
}

/// The (key, value) pairs of a name tree, in tree order. Iterative, with a depth bound
/// and each node visited once, so neither deep nor cyclic trees exhaust anything.
fn name_tree(doc: &mut Document<'_>, root: &Object) -> Result<Vec<(String, Object)>> {
    let mut out = Vec::new();
    let mut visited: BTreeSet<u32> = BTreeSet::new();
    // (node, depth), popped from the end: children are pushed in reverse.
    let mut stack: Vec<(Object, usize)> = vec![(root.clone(), 0)];
    while let Some((node, depth)) = stack.pop() {
        if depth > doc.limits().max_name_tree_depth {
            return Err(Error::LimitExceeded {
                limit: "max_name_tree_depth",
                value: doc.limits().max_name_tree_depth,
            });
        }
        let location = location_of(&node);
        if let Object::Ref(reference) = node
            && !visited.insert(reference.num)
        {
            return Err(malformed(
                location,
                "the /EmbeddedFiles name tree has a cycle or a shared node",
            ));
        }
        let resolved = dict(doc, &node, "a node of the /EmbeddedFiles name tree")?;
        let node_dict = as_dict(&resolved);
        let limits = match node_dict.get(b"Limits") {
            None => None,
            Some(limits) => Some(limits_of(doc, limits, location)?),
        };
        match (node_dict.get(b"Kids"), node_dict.get(b"Names")) {
            (Some(_), Some(_)) => {
                return Err(malformed(
                    location,
                    "a name tree node has both /Kids and /Names",
                ));
            }
            (Some(kids), None) => {
                let kids = doc.resolve(kids)?;
                let Object::Array(kids) = &*kids else {
                    return Err(malformed(location, "name tree /Kids is not an array"));
                };
                for kid in kids.iter().rev() {
                    stack.push((kid.clone(), depth + 1));
                }
            }
            (None, Some(names)) => {
                let names = doc.resolve(names)?;
                let Object::Array(names) = &*names else {
                    return Err(malformed(location, "name tree /Names is not an array"));
                };
                let (pairs, odd) = names.as_chunks::<2>();
                if !odd.is_empty() {
                    return Err(malformed(
                        location,
                        "name tree /Names has an odd number of elements",
                    ));
                }
                for pair in pairs {
                    let key = match &*doc.resolve(&pair[0])? {
                        Object::String(key) => key.clone(),
                        other => {
                            return Err(malformed(
                                location,
                                format!("a name tree key is {}, not a string", other.kind()),
                            ));
                        }
                    };
                    if let Some((low, high)) = &limits
                        && (key < *low || key > *high)
                    {
                        return Err(malformed(
                            location,
                            "a name tree key lies outside the node's /Limits",
                        ));
                    }
                    let key = text::decode(&key).map_err(|message| {
                        malformed(location, format!("a name tree key: {message}"))
                    })?;
                    if out.len() >= doc.limits().max_attachments {
                        return Err(Error::LimitExceeded {
                            limit: "max_attachments",
                            value: doc.limits().max_attachments,
                        });
                    }
                    out.push((key, pair[1].clone()));
                }
            }
            (None, None) => {}
        }
    }
    Ok(out)
}

/// The two strings of a name tree node's `/Limits`.
fn limits_of(
    doc: &mut Document<'_>,
    limits: &Object,
    location: Option<Location>,
) -> Result<(Vec<u8>, Vec<u8>)> {
    let limits = doc.resolve(limits)?;
    if let Object::Array(items) = &*limits
        && let [Object::String(low), Object::String(high)] = items.as_slice()
    {
        if low > high {
            return Err(malformed(
                location,
                "name tree /Limits are in decreasing order",
            ));
        }
        return Ok((low.clone(), high.clone()));
    }
    Err(malformed(
        location,
        "name tree /Limits is not an array of two strings",
    ))
}

/// A file specification dictionary with its embedded file stream; `None` when it has
/// no `/EF`.
fn file_spec(
    doc: &mut Document<'_>,
    object: &Object,
    key: Option<String>,
) -> Result<Option<FileSpec>> {
    let location = location_of(object);
    let resolved = doc.resolve(object)?;
    let spec = match &*resolved {
        Object::Dict(spec) => spec,
        Object::String(_) => {
            return Err(Error::unsupported(
                location,
                "a file specification string instead of a dictionary (no embedded file)",
            ));
        }
        other => {
            return Err(malformed(
                location,
                format!("a file specification is {}, not a dictionary", other.kind()),
            ));
        }
    };
    if let Some(kind) = spec.get(b"Type")
        && !matches!(kind, Object::Name(name) if name == b"Filespec")
    {
        return Err(malformed(
            location,
            "a file specification whose /Type is not /Filespec",
        ));
    }
    let Some(ef) = spec.get(b"EF") else {
        return Ok(None);
    };
    let ef = dict(doc, ef, "a file specification's /EF")?;
    let ef = as_dict(&ef);
    let stream = match (ef.get(b"UF"), ef.get(b"F")) {
        (Some(Object::Ref(uf)), Some(Object::Ref(f))) if uf != f => {
            return Err(Error::unsupported(
                location,
                "/EF with different embedded files under /F and /UF",
            ));
        }
        (Some(Object::Ref(stream)), _) | (None, Some(Object::Ref(stream))) => *stream,
        (None, None) => return Err(malformed(location, "/EF has neither /F nor /UF")),
        _ => {
            return Err(malformed(
                location,
                "/EF /F or /UF is not an indirect reference to a stream",
            ));
        }
    };
    let text_entry = |doc: &mut Document<'_>, name: &[u8], what: &str| -> Result<Option<String>> {
        match spec.get(name) {
            None => Ok(None),
            Some(value) => {
                let value = doc.resolve(value)?;
                text_value(&value, what, location).map(Some)
            }
        }
    };
    let uf = text_entry(doc, b"UF", "file specification /UF")?;
    let f = text_entry(doc, b"F", "file specification /F")?;
    let description = text_entry(doc, b"Desc", "file specification /Desc")?;
    let relationship = match spec.get(b"AFRelationship") {
        None => None,
        Some(value) => match &*doc.resolve(value)? {
            Object::Name(name) => Some(AfRelationship::from_name(name)),
            other => {
                return Err(malformed(
                    location,
                    format!("/AFRelationship is {}, not a name", other.kind()),
                ));
            }
        },
    };
    let Some(file_name) = uf.or(f).or_else(|| key.clone()) else {
        return Err(malformed(
            location,
            "an embedded file has no name (no /UF, /F or name tree key)",
        ));
    };
    Ok(Some(FileSpec {
        file_name,
        key,
        in_associated_files: false,
        relationship,
        description,
        stream,
    }))
}

/// The 16 bytes of an MD5 `/CheckSum`: the 16-byte string ISO 32000-2 specifies, or
/// the 32 hexadecimal digits of the digest as text, which some producers write instead.
fn checksum_bytes(bytes: &[u8]) -> Option<[u8; 16]> {
    if let Ok(raw) = <[u8; 16]>::try_from(bytes) {
        return Some(raw);
    }
    let (pairs, rest) = bytes.as_chunks::<2>();
    if pairs.len() != 16 || !rest.is_empty() {
        return None;
    }
    let mut out = [0u8; 16];
    for (slot, pair) in out.iter_mut().zip(pairs) {
        let text = std::str::from_utf8(pair).ok()?;
        if !text.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        *slot = u8::from_str_radix(text, 16).ok()?;
    }
    Some(out)
}

/// The `/Params` of an embedded file stream (7.11.4.2, Table 45).
#[derive(Default)]
struct Params {
    size: Option<u64>,
    checksum: Option<[u8; 16]>,
    creation_date: Option<String>,
    modification_date: Option<String>,
}

fn params(doc: &mut Document<'_>, stream: &Dict, location: Option<Location>) -> Result<Params> {
    let Some(params) = stream.get(b"Params") else {
        return Ok(Params::default());
    };
    let params = dict(doc, params, "embedded file /Params")?;
    let params = as_dict(&params);
    let mut out = Params::default();
    if let Some(value) = params.get(b"Size") {
        out.size = match &*doc.resolve(value)? {
            Object::Integer(value) => Some(
                u64::try_from(*value).map_err(|_| malformed(location, "negative /Params /Size"))?,
            ),
            other => {
                return Err(malformed(
                    location,
                    format!("/Params /Size is {}, not an integer", other.kind()),
                ));
            }
        };
    }
    if let Some(value) = params.get(b"CheckSum") {
        out.checksum = match &*doc.resolve(value)? {
            Object::String(bytes) => Some(checksum_bytes(bytes).ok_or_else(|| {
                malformed(
                    location,
                    format!(
                        "/Params /CheckSum has {} bytes, neither 16 nor 32 hexadecimal digits",
                        bytes.len()
                    ),
                )
            })?),
            other => {
                return Err(malformed(
                    location,
                    format!("/Params /CheckSum is {}, not a string", other.kind()),
                ));
            }
        };
    }
    for (key, slot, what) in [
        (
            b"CreationDate".as_slice(),
            &mut out.creation_date,
            "/Params /CreationDate",
        ),
        (b"ModDate", &mut out.modification_date, "/Params /ModDate"),
    ] {
        if let Some(value) = params.get(key) {
            let value = doc.resolve(value)?;
            *slot = Some(text_value(&value, what, location)?);
        }
    }
    Ok(out)
}

/// The embedded file of `spec`, decoded, its `/Params /Size` and `/CheckSum` verified.
pub(crate) fn attachment(doc: &mut Document<'_>, spec: &FileSpec) -> Result<Attachment> {
    let location = Some(Location::Object(spec.stream.num));
    let object = doc.get(spec.stream)?;
    let Object::Stream(stream) = &*object else {
        return Err(malformed(
            location,
            format!(
                "the embedded file {} is {}, not a stream",
                spec.file_name,
                object.kind()
            ),
        ));
    };
    if let Some(kind) = stream.dict.get(b"Type")
        && !matches!(kind, Object::Name(name) if name == b"EmbeddedFile")
    {
        return Err(malformed(
            location,
            "an embedded file stream whose /Type is not /EmbeddedFile",
        ));
    }
    let mime_type = match stream.dict.get(b"Subtype") {
        None => None,
        Some(value) => match &*doc.resolve(value)? {
            Object::Name(name) => Some(String::from_utf8(name.clone()).map_err(|_| {
                malformed(
                    location,
                    format!("/Subtype {} is not UTF-8", display_name(name)),
                )
            })?),
            other => {
                return Err(malformed(
                    location,
                    format!("/Subtype is {}, not a name", other.kind()),
                ));
            }
        },
    };
    let Params {
        size,
        checksum,
        creation_date,
        modification_date,
    } = params(doc, &stream.dict, location)?;
    let data = doc.stream_data(stream, spec.stream.num)?;
    if let Some(size) = size
        && u64::try_from(data.len()).ok() != Some(size)
    {
        return Err(malformed(
            location,
            format!(
                "the embedded file {} has {} bytes, its /Params /Size says {size}",
                spec.file_name,
                data.len()
            ),
        ));
    }
    if let Some(checksum) = checksum
        && md5::digest(&data) != checksum
    {
        return Err(malformed(
            location,
            format!(
                "the MD5 of the embedded file {} differs from its /Params /CheckSum",
                spec.file_name
            ),
        ));
    }
    Ok(Attachment {
        file_name: spec.file_name.clone(),
        key: spec.key.clone(),
        in_associated_files: spec.in_associated_files,
        relationship: spec.relationship.clone(),
        description: spec.description.clone(),
        mime_type,
        size,
        checksum,
        creation_date,
        modification_date,
        data,
    })
}

/// The XMP metadata stream of the catalog (`/Metadata`, 14.3.2), decoded.
pub(crate) fn metadata_stream(doc: &mut Document<'_>, catalog: &Dict) -> Result<Option<Vec<u8>>> {
    let Some(reference) = catalog.get(b"Metadata").cloned() else {
        return Ok(None);
    };
    let location = location_of(&reference);
    let object = doc.resolve(&reference)?;
    let Object::Stream(stream) = &*object else {
        return Err(malformed(
            location,
            format!("the catalog's /Metadata is {}, not a stream", object.kind()),
        ));
    };
    if let Some(subtype) = stream.dict.get(b"Subtype")
        && !matches!(subtype, Object::Name(name) if name == b"XML")
    {
        return Err(malformed(
            location,
            "the catalog's /Metadata stream has a /Subtype other than /XML",
        ));
    }
    let num = match reference {
        Object::Ref(reference) => reference.num,
        _ => 0,
    };
    doc.stream_data(stream, num).map(Some)
}
