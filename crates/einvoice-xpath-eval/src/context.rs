//! The static context an expression is compiled in and the dynamic context it is
//! evaluated in, with the hooks the XSLT layer provides: variables, user functions,
//! `current()`, `document()` and `system-property()`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use einvoice_xdm::{Date, DateTime, XPathRegex};
use einvoice_xpath::{NamespaceMap, Namespaces, QName};

use crate::error::{ErrorCode, Result, err};
use crate::item::Item;
use crate::tree::Node;

/// The identity of a variable declared by the host, given back to it at evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VariableId(pub u32);

/// The identity of a function declared by the host (an `xsl:function`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FunctionId(pub u32);

/// What is statically known of the number of items of a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Cardinality {
    /// At most one item.
    AtMostOne,
    /// Any number of items.
    Any,
}

/// A variable in scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VariableInfo {
    /// Its identity at evaluation.
    pub id: VariableId,
    /// What is known of its value. It only affects the evaluation strategy (which side
    /// of a general comparison is read first), never the result.
    pub cardinality: Cardinality,
}

/// The static context of an expression (XPath 2.0 section 2.1.1), as far as the compiler
/// needs it: names are already resolved by the parser.
pub trait StaticContext {
    /// The variable of that expanded name in scope, if any.
    fn variable(&self, name: &QName) -> Option<VariableInfo>;

    /// The host function of that expanded name and arity, if any.
    fn function(&self, name: &QName, arity: usize) -> Option<FunctionId>;

    /// Whether the XSLT functions `current()`, `document()` and `system-property()` are
    /// available.
    fn xslt_functions(&self) -> bool;

    /// The namespace bound to a prefix, for the `QName` argument of `system-property()`.
    fn namespace_uri(&self, prefix: &str) -> Option<String>;
}

/// A static context built by hand: namespaces, variables and functions declared one by
/// one.
#[derive(Debug, Clone, Default)]
pub struct SimpleStaticContext {
    namespaces: NamespaceMap,
    variables: Vec<(QName, VariableInfo)>,
    functions: Vec<(QName, usize, FunctionId)>,
    xslt: bool,
}

impl SimpleStaticContext {
    /// An empty context: no namespaces, variables or functions, no XSLT functions.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Uses these prefix bindings for `system-property()`.
    #[must_use]
    pub fn with_namespaces(mut self, namespaces: NamespaceMap) -> Self {
        self.namespaces = namespaces;
        self
    }

    /// Makes the XSLT functions available.
    #[must_use]
    pub const fn with_xslt_functions(mut self) -> Self {
        self.xslt = true;
        self
    }

    /// Declares a variable (a later declaration of the same name hides the earlier one)
    /// and returns its identity: the declaration order, from 0.
    pub fn declare_variable(&mut self, name: QName, cardinality: Cardinality) -> VariableId {
        let id = VariableId(u32::try_from(self.variables.len()).unwrap_or(u32::MAX));
        self.variables
            .push((name, VariableInfo { id, cardinality }));
        id
    }

    /// Declares a host function and returns its identity: the declaration order, from 0.
    pub fn declare_function(&mut self, name: QName, arity: usize) -> FunctionId {
        let id = FunctionId(u32::try_from(self.functions.len()).unwrap_or(u32::MAX));
        self.functions.push((name, arity, id));
        id
    }
}

impl StaticContext for SimpleStaticContext {
    fn variable(&self, name: &QName) -> Option<VariableInfo> {
        self.variables
            .iter()
            .rev()
            .find(|(n, _)| n.same_expanded_name(name))
            .map(|(_, info)| *info)
    }

    fn function(&self, name: &QName, arity: usize) -> Option<FunctionId> {
        self.functions
            .iter()
            .find(|(n, a, _)| *a == arity && n.same_expanded_name(name))
            .map(|(_, _, id)| *id)
    }

    fn xslt_functions(&self) -> bool {
        self.xslt
    }

    fn namespace_uri(&self, prefix: &str) -> Option<String> {
        self.namespaces.namespace_uri(prefix)
    }
}

/// What the host (the XSLT layer) provides at evaluation. The compiler only lets
/// through references to variables and functions the static context declared, so a
/// host is only asked for those.
pub trait Host {
    /// The value of a variable.
    ///
    /// # Errors
    ///
    /// Whatever evaluating the variable raises.
    fn variable(&self, id: VariableId) -> Result<Rc<[Item]>>;

    /// Calls a host function.
    ///
    /// # Errors
    ///
    /// Whatever the function raises.
    fn call_function(&self, id: FunctionId, args: Vec<Vec<Item>>) -> Result<Vec<Item>>;

    /// `document()`: the document node of the resource at `uri`, resolved against `base`
    /// (the base URI of the node the URI came from, `None` for the static base URI). The
    /// same URI must give the same document node, parsed with
    /// [`Document::parse_loaded`](crate::Document::parse_loaded)
    /// so that `fn:document-uri` gives its absolute URI, as in Saxon.
    ///
    /// # Errors
    ///
    /// The resource cannot be retrieved or parsed.
    fn document(&self, uri: &str, base: Option<&str>) -> Result<Node>;

    /// `system-property()`: the value of the property (the empty string for an unknown
    /// one).
    fn system_property(&self, name: &QName) -> String;
}

/// A host for plain XPath: no variables, functions or documents.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoHost;

impl Host for NoHost {
    fn variable(&self, id: VariableId) -> Result<Rc<[Item]>> {
        err(ErrorCode::Host, format!("no variable {}", id.0))
    }

    fn call_function(&self, id: FunctionId, _args: Vec<Vec<Item>>) -> Result<Vec<Item>> {
        err(ErrorCode::Host, format!("no function {}", id.0))
    }

    fn document(&self, uri: &str, _base: Option<&str>) -> Result<Node> {
        err(ErrorCode::Host, format!("no document {uri}"))
    }

    fn system_property(&self, _name: &QName) -> String {
        String::new()
    }
}

/// The clock of an evaluation: the implicit timezone and the value of
/// `fn:current-dateTime()`. Both are inputs: the evaluator never reads a clock itself, and
/// the current date and time stays the same for every evaluation made with the clock, as
/// Saxon reads its clock once per transformation.
///
/// [`Clock::system`] reads the system clock on the targets that have one; elsewhere, such
/// as `wasm32-unknown-unknown`, the host gives the time it reads, for instance with
/// [`Clock::from_unix_millis`] and JavaScript's `Date.now()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Clock {
    implicit_timezone: i16,
    current_date_time: DateTime,
}

impl Clock {
    /// The implicit timezone, in minutes east of UTC (Saxon takes the JVM's), and the value
    /// of `fn:current-dateTime()`, returned as given.
    ///
    /// # Errors
    ///
    /// An offset beyond ±14:00, or a date and time without a timezone (the current date
    /// and time always has one).
    pub fn new(implicit_timezone: i16, current_date_time: DateTime) -> Result<Self> {
        check_timezone(implicit_timezone)?;
        if current_date_time.timezone().is_none() {
            return err(
                ErrorCode::Unsupported,
                "the current date and time has no timezone",
            );
        }
        Ok(Self {
            implicit_timezone,
            current_date_time,
        })
    }

    /// The implicit timezone, in minutes east of UTC, and the instant `millis`
    /// milliseconds after the Unix epoch (1970-01-01T00:00:00Z) as the current date and
    /// time, expressed in that timezone.
    ///
    /// # Errors
    ///
    /// An offset beyond ±14:00, or an instant beyond the supported dates.
    pub fn from_unix_millis(implicit_timezone: i16, millis: i64) -> Result<Self> {
        check_timezone(implicit_timezone)?;
        Ok(Self {
            implicit_timezone,
            current_date_time: date_time_from_millis(millis, implicit_timezone)?,
        })
    }

    /// The implicit timezone, in minutes east of UTC, and the current reading of the
    /// system clock, expressed in that timezone. Not available on
    /// `wasm32-unknown-unknown`, which has no clock.
    ///
    /// # Errors
    ///
    /// An offset beyond ±14:00, or a system clock beyond the supported dates.
    #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
    pub fn system(implicit_timezone: i16) -> Result<Self> {
        use std::time::{SystemTime, UNIX_EPOCH};
        let millis = match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(after) => i64::try_from(after.as_millis()),
            Err(before) => i64::try_from(before.duration().as_millis()).map(|m| -m),
        }
        .map_err(|_| crate::error::Error::new(ErrorCode::Fodt0001, "system clock out of range"))?;
        Self::from_unix_millis(implicit_timezone, millis)
    }

    /// The implicit timezone in minutes.
    #[must_use]
    pub const fn implicit_timezone(&self) -> i16 {
        self.implicit_timezone
    }

    /// The value of `fn:current-dateTime()`.
    #[must_use]
    pub const fn current_date_time(&self) -> DateTime {
        self.current_date_time
    }
}

fn check_timezone(minutes: i16) -> Result<()> {
    if (-840..=840).contains(&minutes) {
        Ok(())
    } else {
        err(ErrorCode::Unsupported, "timezone offset out of range")
    }
}

/// The dynamic context shared by evaluations: the host, the [`Clock`] (implicit timezone
/// and current date and time) and a cache of compiled regular expressions.
pub struct DynamicContext<'h> {
    pub(crate) host: &'h dyn Host,
    pub(crate) implicit_timezone: i16,
    pub(crate) current_date_time: DateTime,
    pub(crate) regex_cache: RefCell<HashMap<(String, String), Rc<XPathRegex>>>,
}

impl std::fmt::Debug for DynamicContext<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DynamicContext")
            .field("implicit_timezone", &self.implicit_timezone)
            .field("current_date_time", &self.current_date_time)
            .finish_non_exhaustive()
    }
}

impl<'h> DynamicContext<'h> {
    /// A context with this host and clock.
    #[must_use]
    pub fn new(host: &'h dyn Host, clock: Clock) -> Self {
        Self {
            host,
            implicit_timezone: clock.implicit_timezone,
            current_date_time: clock.current_date_time,
            regex_cache: RefCell::new(HashMap::new()),
        }
    }

    /// The clock: implicit timezone and current date and time.
    #[must_use]
    pub const fn clock(&self) -> Clock {
        Clock {
            implicit_timezone: self.implicit_timezone,
            current_date_time: self.current_date_time,
        }
    }

    /// The implicit timezone in minutes.
    #[must_use]
    pub const fn implicit_timezone(&self) -> i16 {
        self.implicit_timezone
    }

    /// The value of `fn:current-dateTime()`.
    #[must_use]
    pub const fn current_date_time(&self) -> DateTime {
        self.current_date_time
    }

    pub(crate) fn regex(&self, pattern: &str, flags: &str) -> Result<Rc<XPathRegex>> {
        let key = (pattern.to_owned(), flags.to_owned());
        if let Some(regex) = self.regex_cache.borrow().get(&key) {
            return Ok(Rc::clone(regex));
        }
        let regex = Rc::new(XPathRegex::new(pattern, flags)?);
        self.regex_cache.borrow_mut().insert(key, Rc::clone(&regex));
        Ok(regex)
    }
}

/// The date-time `millis` after the epoch, in the timezone `offset` (minutes).
fn date_time_from_millis(millis: i64, offset: i16) -> Result<DateTime> {
    let local = millis
        .checked_add(i64::from(offset) * 60_000)
        .ok_or_else(|| crate::error::Error::new(ErrorCode::Fodt0001, "instant out of range"))?;
    let days = local.div_euclid(86_400_000);
    let in_day = local.rem_euclid(86_400_000);
    // Civil date from days since 1970-01-01 (proleptic Gregorian).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let narrow = |v: i64| u8::try_from(v).unwrap_or(u8::MAX);
    let year = i32::try_from(year)
        .map_err(|_| crate::error::Error::new(ErrorCode::Fodt0001, "year out of range"))?;
    let date = Date::new(year, narrow(month), narrow(day), Some(offset))?;
    let seconds = in_day / 1000;
    let nanos = u32::try_from((in_day % 1000) * 1_000_000).unwrap_or(0);
    Ok(DateTime::new(
        date,
        narrow(seconds / 3600),
        narrow(seconds / 60 % 60),
        narrow(seconds % 60),
        nanos,
    )?)
}

/// The focus of an evaluation: the context item, position and size.
#[derive(Debug, Clone)]
pub struct Focus {
    /// The context item.
    pub item: Item,
    /// The context position, from 1.
    pub position: usize,
    /// The context size.
    pub size: usize,
}

/// Where an expression is evaluated: its focus, and the item `current()` returns.
#[derive(Debug, Clone, Default)]
pub struct Context {
    /// The focus; `None` when the context item is absent.
    pub focus: Option<Focus>,
    /// The current item of XSLT (`current()`); `None` where it is absent.
    pub current: Option<Item>,
}

impl Context {
    /// No focus and no current item.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The item as context item (position and size 1) and as current item, as for an
    /// expression of an XSLT instruction evaluated with that item as context.
    #[must_use]
    pub fn for_item(item: Item) -> Self {
        Self {
            focus: Some(Focus {
                item: item.clone(),
                position: 1,
                size: 1,
            }),
            current: Some(item),
        }
    }

    /// The item as context and current item, at a given position in a sequence of `size`
    /// items.
    #[must_use]
    pub fn at(item: Item, position: usize, size: usize) -> Self {
        Self {
            focus: Some(Focus {
                item: item.clone(),
                position,
                size,
            }),
            current: Some(item),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_conversions() {
        let dt = date_time_from_millis(0, 0).unwrap();
        assert_eq!(dt.to_string(), "1970-01-01T00:00:00Z");
        let dt = date_time_from_millis(1_789_000_123_456, 60).unwrap();
        assert_eq!(dt.to_string(), "2026-09-10T01:28:43.456+01:00");
        let dt = date_time_from_millis(-1, -300).unwrap();
        assert_eq!(dt.to_string(), "1969-12-31T18:59:59.999-05:00");
        // An offset that takes the instant beyond i64: an error, never an overflow.
        for (millis, offset) in [(i64::MAX, 840), (i64::MIN, -840)] {
            assert_eq!(
                date_time_from_millis(millis, offset).unwrap_err().code(),
                ErrorCode::Fodt0001
            );
        }
        let dt = date_time_from_millis(i64::MAX, 0).unwrap();
        assert_eq!(dt.to_string(), "292278994-08-17T07:12:55.807Z");
    }
}
