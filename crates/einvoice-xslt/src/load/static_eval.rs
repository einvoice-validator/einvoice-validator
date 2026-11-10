//! Static evaluation of `use-when` (XSLT 2.0 section 3.12), before the stylesheet is
//! read: an element whose `use-when` is false is left out with its descendants.
//!
//! The expressions evaluated are those of the static context of `use-when` that this
//! crate can evaluate exactly as Saxon-HE 12.10 does: string literals, `true()`,
//! `false()`, `not()`, `boolean()`, `and`, `or`, `=` and `!=` (general and value
//! comparisons) of strings, `element-available()` and `system-property()` with Saxon's
//! answers ([`crate::element_available`], [`crate::system_property`]). Anything else is
//! refused: the stylesheet is not loaded rather than read differently from Saxon.

use roxmltree::Node;

use super::InScope;
use einvoice_xpath::{CompareKind, ComparisonOp, Expr, FN_NAMESPACE, Literal, QName, parse_expr};

/// A static value: a boolean or a string.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Value {
    Boolean(bool),
    String(String),
}

impl Value {
    /// The effective boolean value.
    fn truth(&self) -> bool {
        match self {
            Self::Boolean(b) => *b,
            Self::String(s) => !s.is_empty(),
        }
    }
}

/// Whether the element with `use-when="expr"` is kept: the effective boolean value of
/// the expression, evaluated with the in-scope namespaces of `node`.
pub(crate) fn use_when(expr: &str, node: Node<'_, '_>) -> Result<bool, String> {
    let parsed = parse_expr(expr, &InScope(node)).map_err(|e| format!("`{expr}`: {e}"))?;
    evaluate(&parsed, node).map(|v| v.truth())
}

fn unsupported(expr: &Expr) -> String {
    format!("`{expr}` is outside the expressions evaluated statically")
}

fn evaluate(expr: &Expr, node: Node<'_, '_>) -> Result<Value, String> {
    match expr {
        Expr::Literal(Literal::String(s)) => Ok(Value::String(s.clone())),
        Expr::And(lhs, rhs) => Ok(Value::Boolean(
            evaluate(lhs, node)?.truth() && evaluate(rhs, node)?.truth(),
        )),
        Expr::Or(lhs, rhs) => Ok(Value::Boolean(
            evaluate(lhs, node)?.truth() || evaluate(rhs, node)?.truth(),
        )),
        Expr::Comparison { op, lhs, rhs } => {
            let kind = match op {
                ComparisonOp::General(k) | ComparisonOp::Value(k) => *k,
            };
            let (Value::String(a), Value::String(b)) = (evaluate(lhs, node)?, evaluate(rhs, node)?)
            else {
                return Err(unsupported(expr));
            };
            match kind {
                CompareKind::Eq => Ok(Value::Boolean(a == b)),
                CompareKind::Ne => Ok(Value::Boolean(a != b)),
                _ => Err(unsupported(expr)),
            }
        }
        Expr::FunctionCall { name, args } if name.namespace.as_deref() == Some(FN_NAMESPACE) => {
            function(expr, &name.local, args, node)
        }
        _ => Err(unsupported(expr)),
    }
}

fn function(expr: &Expr, local: &str, args: &[Expr], node: Node<'_, '_>) -> Result<Value, String> {
    match (local, args) {
        ("true", []) => Ok(Value::Boolean(true)),
        ("false", []) => Ok(Value::Boolean(false)),
        ("not", [arg]) => Ok(Value::Boolean(!evaluate(arg, node)?.truth())),
        ("boolean", [arg]) => Ok(Value::Boolean(evaluate(arg, node)?.truth())),
        ("element-available", [arg]) => {
            let name = lexical_qname(&string_argument(expr, arg, node)?, node, "XTDE1440")?;
            if name.prefix.is_none() {
                // Whether the default namespace applies is not settled here.
                return Err(format!("element-available('{name}') of an unprefixed name"));
            }
            crate::element_available(name.namespace.as_deref(), &name.local)
                .map(Value::Boolean)
                .ok_or_else(|| {
                    format!("element-available('{name}'): Saxon-HE's answer is not known here")
                })
        }
        ("system-property", [arg]) => {
            let name = lexical_qname(&string_argument(expr, arg, node)?, node, "XTDE1390")?;
            Ok(Value::String(
                crate::system_property(name.namespace.as_deref(), &name.local).to_owned(),
            ))
        }
        _ => Err(unsupported(expr)),
    }
}

fn string_argument(expr: &Expr, arg: &Expr, node: Node<'_, '_>) -> Result<String, String> {
    match evaluate(arg, node)? {
        Value::String(s) => Ok(s),
        Value::Boolean(_) => Err(unsupported(expr)),
    }
}

/// A lexical `QName` resolved with the in-scope namespaces of `node`: an unprefixed name
/// is in no namespace (as for `system-property()`, XSLT 2.0 section 16.6), an undeclared
/// prefix is the error `code`. The parser gives the argument of `system-property()`
/// resolved already, as `Q{uri}local`.
fn lexical_qname(text: &str, node: Node<'_, '_>, code: &str) -> Result<QName, String> {
    let text = text.trim_matches(einvoice_xpath::is_xml_whitespace);
    if let Some((uri, local)) = text
        .strip_prefix("Q{")
        .and_then(|rest| rest.split_once('}'))
        && einvoice_xpath::is_ncname(local)
    {
        return Ok(QName {
            prefix: Some(String::new()),
            namespace: (!uri.is_empty()).then(|| uri.to_owned()),
            local: local.to_owned(),
        });
    }
    let (prefix, local) = match text.split_once(':') {
        Some((p, l)) => (Some(p), l),
        None => (None, text),
    };
    if !einvoice_xpath::is_ncname(local) || prefix.is_some_and(|p| !einvoice_xpath::is_ncname(p)) {
        return Err(format!("`{text}` is not a lexical QName ({code})"));
    }
    let namespace = match prefix {
        None => None,
        Some(p) => Some(
            node.lookup_namespace_uri(Some(p))
                .ok_or_else(|| format!("the prefix of `{text}` is not declared ({code})"))?
                .to_owned(),
        ),
    };
    Ok(QName {
        prefix: prefix.map(str::to_owned),
        namespace,
        local: local.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(expr: &str) -> Result<bool, String> {
        let doc = roxmltree::Document::parse(
            "<xsl:stylesheet xmlns:xsl='http://www.w3.org/1999/XSL/Transform' xmlns:exsl='http://exslt.org/common'/>",
        )
        .unwrap();
        use_when(expr, doc.root_element())
    }

    #[test]
    fn evaluates_the_static_subset() {
        assert_eq!(eval("element-available('xsl:namespace')"), Ok(true));
        assert_eq!(eval("not(element-available('xsl:namespace'))"), Ok(false));
        assert_eq!(eval("element-available('exsl:document')"), Ok(false));
        assert_eq!(eval("system-property('xsl:vendor') = 'Saxonica'"), Ok(true));
        assert_eq!(
            eval("system-property('xsl:version') != '3.0' or true()"),
            Ok(true)
        );
        assert_eq!(eval("'' and true()"), Ok(false));
        assert!(eval("element-available('xsl:bogus')").is_err());
        assert!(eval("element-available('nope:x')").is_err());
        assert!(eval("1 = 1").is_err());
        assert!(eval("function-available('exsl:node-set')").is_err());
        assert!(eval("$v").is_err());
    }
}
