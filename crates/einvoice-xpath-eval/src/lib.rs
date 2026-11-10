//! Node model and evaluator of the XPath 2.0 subset that the official e-invoicing
//! validation stylesheets use, with the behaviour of the reference engine, Saxon-HE
//! 12.10.
//!
//! * [`tree`]: immutable XML trees ([`Document`], [`Node`]), parsed from text with every
//!   whitespace text node kept, or built with a [`TreeBuilder`].
//! * [`compile`]: an expression parsed by [`einvoice_xpath`] is resolved against a
//!   [`StaticContext`] (variables, host functions, whether the XSLT functions exist) into
//!   a [`CompiledExpr`]. Unknown functions and variables are errors here, not at
//!   evaluation. The static types Saxon infers are inferred too, where they change a
//!   result: `xs:decimal` of an argument statically typed `xs:integer` strips its
//!   trailing zeros, as Saxon-HE 12.10's up-casting converter does, and returns one
//!   statically typed `xs:decimal` unchanged (Saxon removes the cast); `div` of operands
//!   statically typed `xs:decimal` (or one `xs:integer`) is Saxon's decimal division. The
//!   host gives the static types of its variables and functions as [`ValueType`]s, which
//!   infer them as Saxon does in a stylesheet from a `select` expression and an `as`
//!   type.
//! * [`CompiledExpr::evaluate`] and [`CompiledExpr::effective_boolean_value`] evaluate it
//!   in a [`DynamicContext`] (the [`Host`] and the [`Clock`]: the implicit timezone and
//!   the current date and time, given by the caller, so that the evaluator reads no clock,
//!   which `wasm32-unknown-unknown` does not have) with a [`Context`] (focus and current
//!   item). Atomic values and their operations are those of [`einvoice_xdm`].
//!
//! Sequences are evaluated lazily, so that consumers that stop early (effective boolean
//! values, `exists`, general comparisons, `some`/`every`, `[1]`) raise only the errors
//! of the items they read, as Saxon does.
//!
//! Functions: `not`, `true`, `false`, `boolean`, `count`, `sum`, `normalize-space`,
//! `string-length`, `string`, `number`, `contains`, `starts-with`, `ends-with`,
//! `substring`, `substring-before`, `substring-after`, `concat`, `translate`,
//! `upper-case`, `round`, `abs`, `name`, `local-name`, `namespace-uri`, `last`,
//! `position`, `exists`, `document-uri` (also with no argument, as in XPath 3.0), `root`,
//! `base-uri`, `matches`, `replace`, `tokenize`, `string-to-codepoints`,
//! `codepoints-to-string`, `string-join`, `reverse`, `current-dateTime`,
//! `distinct-values`, the constructors `xs:decimal`, `xs:date`, `xs:integer`,
//! `xs:string`, and in XSLT `current`, `document` and `system-property` (the last two
//! through the [`Host`]).
//!
//! ```
//! use einvoice_xpath::{NamespaceMap, XS_NAMESPACE, parse_expr};
//! use einvoice_xpath_eval::{Clock, Context, Document, DynamicContext, Item, NoHost, SimpleStaticContext, compile};
//!
//! let doc = Document::parse("<i><l a='19.99'/><l a='0.01'/></i>", None)?;
//! let ns = NamespaceMap::new().bind("xs", XS_NAMESPACE);
//! let expr = compile(&parse_expr("sum(//l/xs:decimal(@a))", &ns)?, &SimpleStaticContext::new())?;
//! // UTC, and the current date and time read by the host: 2026-09-10T00:26:40Z.
//! let dc = DynamicContext::new(&NoHost, Clock::from_unix_millis(0, 1_789_000_000_000)?);
//! let result = expr.evaluate(&dc, &Context::for_item(Item::Node(doc.root())))?;
//! assert_eq!(result[0].string_value(), "20");
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod compile;
mod context;
mod error;
mod eval;
mod functions;
mod item;
mod library;
mod limit;
pub mod tree;

pub use compile::{BindingValue, CompiledExpr, ValueType, compile};
pub use context::{
    Cardinality, Clock, Context, DynamicContext, Focus, FunctionId, Host, NoHost, RegexCache,
    SimpleStaticContext, StaticContext, VariableId, VariableInfo,
};
pub use error::{Error, ErrorCode, Result};
pub use eval::{Argument, NodeSelection, SelectionStorage};
pub use item::Item;
pub use limit::{DEFAULT_MAX_DEPTH, LimitExceeded, StackLimit};
pub use tree::{
    Document, DocumentCache, DocumentError, Node, NodeAxis, NodeKind, TreeBuilder, TreeView,
    file_uri,
};
