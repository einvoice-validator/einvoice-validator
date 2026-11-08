//! The implementations of the built-in functions, with the argument conversions of the
//! function conversion rules (atomization, `xs:untypedAtomic` cast to the expected type,
//! numeric promotion, cardinality checks) and the error codes Saxon raises.

use std::collections::HashMap;
use std::rc::Rc;

use einvoice_xdm::{AtomicType, AtomicValue, Integer, XPathRegex, ops, parse_double};

use crate::compile::{CallData, Expr};
use crate::error::{Error, ErrorCode, Result, err};
use crate::eval::{Ctx, Iter, Run, What, from_vec, once, sort_nodes};
use crate::functions::Builtin;
use crate::item::Item;
use crate::tree::{Node, NodeKind};

pub(crate) const fn ordinal(n: usize) -> &'static str {
    match n {
        0 => "first",
        1 => "second",
        2 => "third",
        _ => "fourth",
    }
}

fn atomic<'s>(value: AtomicValue) -> Iter<'s> {
    once(Item::Atomic(value))
}

fn string<'s>(value: String) -> Iter<'s> {
    atomic(AtomicValue::String(value))
}

fn integer(value: usize) -> AtomicValue {
    AtomicValue::Integer(Integer::from(i64::try_from(value).unwrap_or(i64::MAX)))
}

fn empty<'s>() -> Iter<'s> {
    Box::new(std::iter::empty())
}

/// `fn:normalize-space`: XML whitespace trimmed, inner runs collapsed to one space.
fn normalize_space(text: &str) -> String {
    text.split([' ', '\t', '\r', '\n'])
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// `fn:round` of a double, as `fn:substring` uses it.
fn round(value: f64) -> f64 {
    match ops::round(&AtomicValue::Double(value)) {
        Ok(AtomicValue::Double(d)) => d,
        _ => value,
    }
}

/// `fn:substring` (F&O 2.0 section 7.4.3): the characters at positions `p` with
/// `round(start) <= p < round(start) + round(length)`.
fn substring(text: &str, start: f64, length: Option<f64>) -> String {
    let first = round(start);
    let end = length.map_or(f64::INFINITY, |l| first + round(l));
    let mut position = 0.0_f64;
    text.chars()
        .filter(|_| {
            position += 1.0;
            position >= first && position < end
        })
        .collect()
}

fn translate(text: &str, map: &str, with: &str) -> String {
    let map: Vec<char> = map.chars().collect();
    let with: Vec<char> = with.chars().collect();
    text.chars()
        .filter_map(|c| match map.iter().position(|&m| m == c) {
            Some(i) => with.get(i).copied(),
            None => Some(c),
        })
        .collect()
}

/// Whether a code point is a character of XML 1.0.
fn xml_char(code: i64) -> Option<char> {
    let valid =
        matches!(code, 0x9 | 0xA | 0xD | 0x20..=0xD7FF | 0xE000..=0xFFFD | 0x1_0000..=0x10_FFFF);
    if !valid {
        return None;
    }
    char::from_u32(u32::try_from(code).ok()?)
}

/// The bucket of `fn:distinct-values` (Saxon hashes values, then compares the values of
/// a bucket with `eq`): strings and untyped values by their text, booleans by value,
/// numbers by their value as a double (so that the decimal `0.1` and the double `0.1`
/// meet, as they are equal under `eq`), dates and times all in one bucket.
#[derive(Debug, PartialEq, Eq, Hash)]
enum DistinctKey {
    String(String),
    Boolean(bool),
    Number(u64),
    Temporal,
}

fn distinct_key(value: &AtomicValue) -> DistinctKey {
    match value {
        AtomicValue::String(s) | AtomicValue::UntypedAtomic(s) | AtomicValue::AnyUri(s) => {
            DistinctKey::String(s.clone())
        }
        AtomicValue::Boolean(b) => DistinctKey::Boolean(*b),
        AtomicValue::Integer(_) | AtomicValue::Decimal(_) | AtomicValue::Double(_) => {
            let d = value.numeric_as_f64().unwrap_or(f64::NAN);
            let canonical = if d.is_nan() {
                f64::NAN
            } else if d == 0.0 {
                0.0
            } else {
                d
            };
            DistinctKey::Number(canonical.to_bits())
        }
        AtomicValue::Date(_) | AtomicValue::DateTime(_) => DistinctKey::Temporal,
    }
}

/// Equality in `fn:distinct-values`: `eq`, except that NaN equals NaN and values that
/// cannot be compared are distinct.
fn distinct_equal(a: &AtomicValue, b: &AtomicValue, implicit_timezone: i16) -> bool {
    if a.is_nan() && b.is_nan() {
        return true;
    }
    ops::value_compare(a, ops::ComparisonOp::Equal, b, implicit_timezone).unwrap_or(false)
}

impl<'a> Run<'a> {
    /// A call to a built-in function. The functions are evaluated in groups, each by a
    /// function of its own, so that a call nested in the argument of another holds only
    /// what its group needs on the call stack (that of a debug build keeps every local of
    /// a function).
    pub(crate) fn call<'s>(
        &'s self,
        function: Builtin,
        args: &'s [Expr],
        data: &'s CallData,
        ctx: &Ctx,
    ) -> Result<Iter<'s>>
    where
        'a: 's,
    {
        match function {
            Builtin::Not
            | Builtin::True
            | Builtin::False
            | Builtin::Boolean
            | Builtin::Exists
            | Builtin::Empty => Ok(boolean(self.boolean_function(function, args, ctx)?)),
            Builtin::Count
            | Builtin::Sum
            | Builtin::Reverse
            | Builtin::DistinctValues
            | Builtin::StringJoin => self.sequence_function(function, args, ctx),
            Builtin::NormalizeSpace
            | Builtin::StringLength
            | Builtin::String
            | Builtin::Number
            | Builtin::UpperCase
            | Builtin::Round
            | Builtin::Abs => self.value_function(function, args, ctx),
            Builtin::Contains
            | Builtin::StartsWith
            | Builtin::EndsWith
            | Builtin::SubstringBefore
            | Builtin::SubstringAfter => self.substring_function(function, args, ctx),
            Builtin::Substring
            | Builtin::Concat
            | Builtin::Translate
            | Builtin::StringToCodepoints
            | Builtin::CodepointsToString => self.string_function(function, args, ctx),
            Builtin::Name
            | Builtin::LocalName
            | Builtin::NamespaceUri
            | Builtin::DocumentUri
            | Builtin::Root
            | Builtin::BaseUri => self.node_function(function, args, ctx),
            Builtin::Matches | Builtin::Replace | Builtin::Tokenize => {
                self.regex_function(function, args, data, ctx)
            }
            Builtin::Last
            | Builtin::Position
            | Builtin::CurrentDateTime
            | Builtin::Current
            | Builtin::Document
            | Builtin::SystemProperty => self.context_function(function, args, data, ctx),
            Builtin::XsDecimal | Builtin::XsDate | Builtin::XsInteger | Builtin::XsString => {
                self.constructor(function, &args[0], data, ctx)
            }
        }
    }

    /// `not`, `true`, `false`, `boolean`, `exists`, `empty`.
    #[inline]
    fn boolean_function(&self, function: Builtin, args: &[Expr], ctx: &Ctx) -> Result<bool> {
        Ok(match function {
            Builtin::Not => !self.ebv(&args[0], ctx)?,
            Builtin::True => true,
            Builtin::Boolean => self.ebv(&args[0], ctx)?,
            Builtin::Exists => self.non_empty(&args[0], ctx)?,
            Builtin::Empty => !self.non_empty(&args[0], ctx)?,
            _ => false,
        })
    }

    /// `count`, `sum`, `reverse`, `distinct-values`, `string-join`: functions of a whole
    /// sequence.
    #[inline]
    fn sequence_function<'s>(
        &'s self,
        function: Builtin,
        args: &'s [Expr],
        ctx: &Ctx,
    ) -> Result<Iter<'s>>
    where
        'a: 's,
    {
        Ok(match function {
            Builtin::Count => {
                let mut count = 0;
                for item in self.seq(&args[0], ctx)? {
                    item?;
                    count += 1;
                }
                atomic(integer(count))
            }
            Builtin::Sum => atomic(ops::sum_lazily(self.atomized(&args[0], ctx)?)?),
            Builtin::Reverse => {
                let mut items = self.seq(&args[0], ctx)?.collect::<Result<Vec<_>>>()?;
                items.reverse();
                from_vec(items)
            }
            Builtin::DistinctValues => from_vec(self.distinct_values(&args[0], ctx)?),
            _ => {
                let mut parts = Vec::new();
                for value in self.atomized(&args[0], ctx)? {
                    parts.push(value?.to_string());
                }
                let separator = self
                    .atomic_opt(&args[1], ctx, What::Argument(1, function.name()))?
                    .ok_or_else(|| {
                        Error::new(
                            ErrorCode::Xpty0004,
                            "An empty sequence is not allowed as the second argument of string-join()",
                        )
                    })?;
                let separator = string_value(separator, function.name(), 1)?;
                string(parts.join(&separator))
            }
        })
    }

    /// `normalize-space`, `string-length`, `string`, `number`, `upper-case`, `round`,
    /// `abs`: functions of one value.
    #[inline]
    fn value_function<'s>(&self, function: Builtin, args: &[Expr], ctx: &Ctx) -> Result<Iter<'s>> {
        let f = function.name();
        Ok(match function {
            Builtin::NormalizeSpace => {
                string(normalize_space(&self.string_or_focus(args, ctx, f)?))
            }
            Builtin::StringLength => {
                atomic(integer(self.string_or_focus(args, ctx, f)?.chars().count()))
            }
            Builtin::String => string(match args.first() {
                None => focus_item(ctx, f)?.string_value().into_owned(),
                Some(arg) => self
                    .item_opt(arg, ctx, What::Argument(0, f))?
                    .map(|i| i.string_value().into_owned())
                    .unwrap_or_default(),
            }),
            Builtin::Number => {
                let value = match args.first() {
                    None => Some(focus_item(ctx, f)?.atomize()),
                    Some(arg) => self.atomic_opt(arg, ctx, What::Argument(0, f))?,
                };
                atomic(AtomicValue::Double(
                    value.map_or(f64::NAN, |v| ops::number(&v)),
                ))
            }
            Builtin::UpperCase => string(self.string_arg(&args[0], ctx, f, 0)?.to_uppercase()),
            _ => match self.atomic_opt(&args[0], ctx, What::Argument(0, f))? {
                None => empty(),
                Some(v) if function == Builtin::Round => atomic(ops::round(&v)?),
                Some(v) => atomic(ops::abs(&v)?),
            },
        })
    }

    /// `contains`, `starts-with`, `ends-with`, `substring-before`, `substring-after`.
    #[inline]
    fn substring_function<'s>(
        &self,
        function: Builtin,
        args: &[Expr],
        ctx: &Ctx,
    ) -> Result<Iter<'s>> {
        let f = function.name();
        let text = self.string_arg(&args[0], ctx, f, 0)?;
        let part = self.string_arg(&args[1], ctx, f, 1)?;
        Ok(match function {
            Builtin::Contains => boolean(text.contains(part.as_str())),
            Builtin::StartsWith => boolean(text.starts_with(part.as_str())),
            Builtin::EndsWith => boolean(text.ends_with(part.as_str())),
            Builtin::SubstringBefore => string(
                text.find(part.as_str())
                    .filter(|_| !part.is_empty())
                    .map(|i| text[..i].to_owned())
                    .unwrap_or_default(),
            ),
            _ => string(
                text.find(part.as_str())
                    .map(|i| text[i + part.len()..].to_owned())
                    .unwrap_or_default(),
            ),
        })
    }

    /// `substring`, `concat`, `translate`, `string-to-codepoints`,
    /// `codepoints-to-string`.
    #[inline]
    fn string_function<'s>(&self, function: Builtin, args: &[Expr], ctx: &Ctx) -> Result<Iter<'s>> {
        let f = function.name();
        Ok(match function {
            Builtin::Substring => {
                let text = self.string_arg(&args[0], ctx, f, 0)?;
                let start = self.double_arg(&args[1], ctx, f, 1)?;
                let length = match args.get(2) {
                    Some(arg) => Some(self.double_arg(arg, ctx, f, 2)?),
                    None => None,
                };
                string(substring(&text, start, length))
            }
            Builtin::Concat => {
                let mut out = String::new();
                for (i, arg) in args.iter().enumerate() {
                    if let Some(value) = self.atomic_opt(arg, ctx, What::Argument(i, f))? {
                        out.push_str(&value.to_string());
                    }
                }
                string(out)
            }
            Builtin::Translate => string(translate(
                &self.string_arg(&args[0], ctx, f, 0)?,
                &self.string_arg(&args[1], ctx, f, 1)?,
                &self.string_arg(&args[2], ctx, f, 2)?,
            )),
            Builtin::StringToCodepoints => {
                let text = self.string_arg(&args[0], ctx, f, 0)?;
                from_vec(
                    text.chars()
                        .map(|c| {
                            Item::Atomic(AtomicValue::Integer(Integer::from(i64::from(u32::from(
                                c,
                            )))))
                        })
                        .collect(),
                )
            }
            _ => string(self.codepoints_to_string(&args[0], ctx)?),
        })
    }

    /// `last`, `position`, `current-dateTime`, `current`, `document`,
    /// `system-property`: functions of the context.
    #[inline]
    fn context_function<'s>(
        &self,
        function: Builtin,
        args: &[Expr],
        data: &CallData,
        ctx: &Ctx,
    ) -> Result<Iter<'s>> {
        Ok(match function {
            Builtin::Last => atomic(integer(context_size(ctx)?)),
            Builtin::Position => {
                focus_item(ctx, function.name())?;
                atomic(integer(ctx.position))
            }
            Builtin::CurrentDateTime => atomic(AtomicValue::DateTime(self.dc.current_date_time)),
            Builtin::Current => once(self.current.clone().ok_or_else(|| {
                Error::new(
                    ErrorCode::Xtde1360,
                    "The current() function is not available here: there is no current item",
                )
            })?),
            Builtin::Document => from_vec(self.document(&args[0], ctx)?),
            _ => match data {
                CallData::Property(name) => string(self.dc.host.system_property(name)),
                _ => return err(ErrorCode::Internal, "system-property without its name"),
            },
        })
    }

    /// The constructors `xs:decimal`, `xs:date`, `xs:integer`, `xs:string`.
    #[inline]
    fn constructor<'s>(
        &self,
        function: Builtin,
        arg: &Expr,
        data: &CallData,
        ctx: &Ctx,
    ) -> Result<Iter<'s>> {
        let target = match function {
            Builtin::XsDecimal => AtomicType::Decimal,
            Builtin::XsDate => AtomicType::Date,
            Builtin::XsInteger => AtomicType::Integer,
            _ => AtomicType::String,
        };
        Ok(
            match self.atomic_opt(arg, ctx, What::Argument(0, function.name()))? {
                None => empty(),
                // An argument statically typed xs:integer (Saxon-HE 12.10's
                // up-casting converter: trailing zeros stripped).
                Some(value) if matches!(data, CallData::Upcast) => atomic(value.upcast(target)?),
                // An argument statically typed xs:decimal: no cast (Saxon-HE 12.10
                // removes it), an xs:integer value stays one.
                Some(value) if matches!(data, CallData::Unchanged) => atomic(value),
                Some(value) => atomic(value.cast(target)?),
            },
        )
    }

    /// The string argument of a function with an optional one (`fn:string-length`,
    /// `fn:normalize-space`), or the string value of the context item.
    fn string_or_focus(&self, args: &[Expr], ctx: &Ctx, f: &str) -> Result<String> {
        match args.first() {
            Some(arg) => self.string_arg(arg, ctx, f, 0),
            None => Ok(focus_item(ctx, f)?.string_value().into_owned()),
        }
    }

    /// An `xs:string?` argument; the empty sequence gives the empty string.
    fn string_arg(&self, arg: &Expr, ctx: &Ctx, f: &str, n: usize) -> Result<String> {
        match self.atomic_opt(arg, ctx, What::Argument(n, f))? {
            None => Ok(String::new()),
            Some(value) => string_value(value, f, n),
        }
    }

    /// An `xs:double` argument (exactly one value).
    fn double_arg(&self, arg: &Expr, ctx: &Ctx, f: &str, n: usize) -> Result<f64> {
        let what = What::Argument(n, f);
        match self.atomic_opt(arg, ctx, what)? {
            None => err(
                ErrorCode::Xpty0004,
                format!("An empty sequence is not allowed as the {what}"),
            ),
            Some(AtomicValue::UntypedAtomic(s)) => Ok(parse_double(&s)?),
            Some(value) => value.numeric_as_f64().ok_or_else(|| {
                Error::new(
                    ErrorCode::Xpty0004,
                    format!(
                        "Required item type of the {what} is xs:double; supplied value has item type {}",
                        value.atomic_type()
                    ),
                )
            }),
        }
    }

    /// The node argument of a function with an optional one, or the context node.
    fn node_arg(&self, args: &[Expr], ctx: &Ctx, f: &str) -> Result<Option<Node>> {
        let item = match args.first() {
            None => Some(focus_item(ctx, f)?),
            Some(arg) => self.item_opt(arg, ctx, What::Argument(0, f))?,
        };
        match item {
            None => Ok(None),
            Some(Item::Node(node)) => Ok(Some(node)),
            Some(Item::Atomic(value)) => err(
                ErrorCode::Xpty0004,
                if args.is_empty() {
                    format!("The context item for {f}() is not a node")
                } else {
                    format!(
                        "Required item type of first argument of {f}() is node(); supplied value has item type {}",
                        value.atomic_type()
                    )
                },
            ),
        }
    }

    fn node_function<'s>(
        &'s self,
        function: Builtin,
        args: &'s [Expr],
        ctx: &Ctx,
    ) -> Result<Iter<'s>>
    where
        'a: 's,
    {
        let node = self.node_arg(args, ctx, function.name())?;
        let named = node
            .as_ref()
            .filter(|n| {
                matches!(
                    n.kind(),
                    NodeKind::Element | NodeKind::Attribute | NodeKind::ProcessingInstruction
                )
            })
            .and_then(Node::name);
        Ok(match function {
            Builtin::Name => string(named.map(ToString::to_string).unwrap_or_default()),
            Builtin::LocalName => string(named.map(|n| n.local.clone()).unwrap_or_default()),
            Builtin::NamespaceUri => atomic(AtomicValue::AnyUri(
                node.as_ref()
                    .filter(|n| matches!(n.kind(), NodeKind::Element | NodeKind::Attribute))
                    .and_then(Node::name)
                    .and_then(|n| n.namespace.clone())
                    .unwrap_or_default(),
            )),
            Builtin::DocumentUri => match node
                .filter(|n| n.kind() == NodeKind::Document)
                .and_then(|n| n.document().document_uri().map(str::to_owned))
            {
                Some(uri) => atomic(AtomicValue::AnyUri(uri)),
                None => empty(),
            },
            Builtin::Root => match node {
                Some(n) => once(Item::Node(n.root())),
                None => empty(),
            },
            _ => match node {
                None => empty(),
                Some(n) if n.document().has_xml_base() => {
                    return err(
                        ErrorCode::Unsupported,
                        "base-uri() in a document with xml:base attributes",
                    );
                }
                Some(n) => match n.document().base_uri() {
                    Some(uri) => atomic(AtomicValue::AnyUri(uri.to_owned())),
                    None => empty(),
                },
            },
        })
    }

    fn regex_function<'s>(
        &'s self,
        function: Builtin,
        args: &'s [Expr],
        data: &'s CallData,
        ctx: &Ctx,
    ) -> Result<Iter<'s>>
    where
        'a: 's,
    {
        let f = function.name();
        let input = self.atomic_opt(&args[0], ctx, What::Argument(0, f))?;
        let input = match input {
            Some(value) => Some(string_value(value, f, 0)?),
            None => None,
        };
        let pattern = self.string_arg(&args[1], ctx, f, 1)?;
        let (replacement, flags_at) = if function == Builtin::Replace {
            (Some(self.string_arg(&args[2], ctx, f, 2)?), 3)
        } else {
            (None, 2)
        };
        let flags = match args.get(flags_at) {
            Some(arg) => self.string_arg(arg, ctx, f, flags_at)?,
            None => String::new(),
        };
        let compiled: Rc<XPathRegex>;
        let regex: &XPathRegex = if let CallData::Regex(precompiled) = data {
            precompiled.as_ref().as_ref().map_err(Clone::clone)?
        } else {
            compiled = self.dc.regex(&pattern, &flags)?;
            &compiled
        };
        let text = input.unwrap_or_default();
        Ok(match function {
            Builtin::Matches => boolean(regex.is_match(&text)),
            Builtin::Replace => {
                string(regex.replace(&text, replacement.as_deref().unwrap_or_default())?)
            }
            _ => {
                let tokens: Vec<Item> = regex
                    .tokenize(&text)?
                    .into_iter()
                    .map(|t| Item::Atomic(AtomicValue::String(t.to_owned())))
                    .collect();
                from_vec(tokens)
            }
        })
    }

    fn codepoints_to_string(&self, arg: &Expr, ctx: &Ctx) -> Result<String> {
        let mut out = String::new();
        for value in self.atomized(arg, ctx)? {
            let value = match value? {
                AtomicValue::UntypedAtomic(s) => {
                    AtomicValue::UntypedAtomic(s).cast(AtomicType::Integer)?
                }
                v => v,
            };
            let AtomicValue::Integer(code) = &value else {
                return err(
                    ErrorCode::Xpty0004,
                    format!(
                        "Required item type of first argument of codepoints-to-string() is xs:integer; supplied value has item type {}",
                        value.atomic_type()
                    ),
                );
            };
            let c = code.to_i64().and_then(xml_char).ok_or_else(|| {
                Error::new(
                    ErrorCode::Foch0001,
                    format!("Invalid XML character [x{code}]"),
                )
            })?;
            out.push(c);
        }
        Ok(out)
    }

    fn distinct_values(&self, arg: &Expr, ctx: &Ctx) -> Result<Vec<Item>> {
        let tz = self.dc.implicit_timezone;
        let mut buckets: HashMap<DistinctKey, Vec<AtomicValue>> = HashMap::new();
        let mut out = Vec::new();
        for value in self.atomized(arg, ctx)? {
            let value = value?;
            let bucket = buckets.entry(distinct_key(&value)).or_default();
            if !bucket.iter().any(|kept| distinct_equal(kept, &value, tz)) {
                bucket.push(value.clone());
                out.push(Item::Atomic(value));
            }
        }
        Ok(out)
    }

    fn document(&self, arg: &Expr, ctx: &Ctx) -> Result<Vec<Item>> {
        let mut nodes = Vec::new();
        for item in self.seq(arg, ctx)? {
            let node = match item? {
                Item::Node(n) => {
                    let base = n.document().base_uri().map(str::to_owned);
                    self.dc.host.document(&n.string_value(), base.as_deref())?
                }
                Item::Atomic(value) => self.dc.host.document(&value.to_string(), None)?,
            };
            nodes.push(node);
        }
        Ok(sort_nodes(nodes))
    }
}

fn boolean<'s>(value: bool) -> Iter<'s> {
    atomic(AtomicValue::Boolean(value))
}

fn context_size(ctx: &Ctx) -> Result<usize> {
    focus_item(ctx, "last")?;
    ctx.size
        .ok_or_else(|| Error::new(ErrorCode::Internal, "the context size was not counted"))
}

fn focus_item(ctx: &Ctx, f: &str) -> Result<Item> {
    ctx.item.clone().ok_or_else(|| {
        Error::new(
            ErrorCode::Xpdy0002,
            format!("The context item for {f}() is absent"),
        )
    })
}

/// The value of an `xs:string` parameter: a string, an untyped value (cast) or a URI
/// (promoted); anything else is a type error.
fn string_value(value: AtomicValue, f: &str, n: usize) -> Result<String> {
    match value {
        AtomicValue::String(s) | AtomicValue::UntypedAtomic(s) | AtomicValue::AnyUri(s) => Ok(s),
        other => err(
            ErrorCode::Xpty0004,
            format!(
                "Required item type of {} argument of {f}() is xs:string; supplied value has item type {}",
                ordinal(n),
                other.atomic_type()
            ),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substring_rules() {
        assert_eq!(substring("12345", 1.5, Some(2.6)), "234");
        assert_eq!(substring("12345", 0.0, Some(3.0)), "12");
        assert_eq!(substring("12345", 5.0, Some(-3.0)), "");
        assert_eq!(substring("12345", -3.0, Some(5.0)), "1");
        assert_eq!(substring("12345", f64::NAN, Some(3.0)), "");
        assert_eq!(substring("12345", 1.0, Some(f64::NAN)), "");
        assert_eq!(substring("12345", -42.0, Some(f64::INFINITY)), "12345");
        assert_eq!(
            substring("12345", f64::NEG_INFINITY, Some(f64::INFINITY)),
            ""
        );
        assert_eq!(substring("motor car", 6.0, None), " car");
    }

    #[test]
    fn small_string_functions() {
        assert_eq!(normalize_space("  a \t b\n\nc "), "a b c");
        assert_eq!(normalize_space("\u{a0}a"), "\u{a0}a");
        assert_eq!(translate("bar", "abc", "ABC"), "BAr");
        assert_eq!(translate("--aaa--", "abc-", "ABC"), "AAA");
        assert_eq!(xml_char(0), None);
        assert_eq!(xml_char(0xD800), None);
        assert_eq!(xml_char(65), Some('A'));
    }
}
