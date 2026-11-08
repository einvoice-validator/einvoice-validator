//! PDF objects (ISO 32000-2, 7.3).

use std::collections::BTreeMap;
use std::ops::Range;

/// An indirect reference `num gen R`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ObjRef {
    pub(crate) num: u32,
    pub(crate) generation: u16,
}

/// A PDF object. Real numbers are recognised but their value is not kept: nothing this
/// crate reads is a real number.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Object {
    Null,
    Bool(bool),
    Integer(i64),
    Real,
    Name(Vec<u8>),
    String(Vec<u8>),
    Array(Vec<Object>),
    Dict(Dict),
    Stream(Stream),
    Ref(ObjRef),
}

impl Object {
    /// The kind of object with its article ("an array"), for error messages.
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Bool(_) => "a boolean",
            Self::Integer(_) => "an integer",
            Self::Real => "a real number",
            Self::Name(_) => "a name",
            Self::String(_) => "a string",
            Self::Array(_) => "an array",
            Self::Dict(_) => "a dictionary",
            Self::Stream(_) => "a stream",
            Self::Ref(_) => "an indirect reference",
        }
    }
}

/// A dictionary. Keys are unique (a repeated key is a syntax error); an entry whose value
/// is `null` is equivalent to an absent entry (7.3.7), so [`Dict::get`] does not return it.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Dict(pub(crate) BTreeMap<Vec<u8>, Object>);

impl Dict {
    pub(crate) fn get(&self, key: &[u8]) -> Option<&Object> {
        self.0.get(key).filter(|value| **value != Object::Null)
    }

    /// The value of `key` when it is the name `value`.
    pub(crate) fn has_name(&self, key: &[u8], value: &[u8]) -> bool {
        matches!(self.get(key), Some(Object::Name(name)) if name == value)
    }
}

/// A stream: its dictionary and the byte range of its (encoded) data in the file.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Stream {
    pub(crate) dict: Dict,
    pub(crate) data: Range<usize>,
}

const HEX: &[u8; 16] = b"0123456789ABCDEF";

/// A name as text, for error messages (`#xx` for bytes outside printable ASCII).
pub(crate) fn display_name(name: &[u8]) -> String {
    let mut out = String::from("/");
    for &byte in name {
        if byte.is_ascii_graphic() && byte != b'#' {
            out.push(char::from(byte));
        } else {
            out.push('#');
            out.push(char::from(HEX[usize::from(byte >> 4)]));
            out.push(char::from(HEX[usize::from(byte & 0x0F)]));
        }
    }
    out
}
