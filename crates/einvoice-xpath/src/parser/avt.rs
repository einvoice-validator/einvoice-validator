//! Attribute value templates (XSLT 2.0 section 5.6.1).

use super::Parser;
use crate::ast::{Avt, AvtPart};
use crate::error::{Error, ErrorKind, Result};
use crate::lexer::Tok;
use crate::names::Namespaces;

/// Splits `src` into literal text and `{expression}` parts. `{{` and `}}` stand for `{`
/// and `}`; an expression ends at the first `}` outside a string literal; a lone `}` in the
/// text is an error (`err:XTSE0370`), and so is an empty expression.
pub(crate) fn parse_avt(src: &str, namespaces: &dyn Namespaces) -> Result<Avt> {
    let mut parts = Vec::new();
    let mut text = String::new();
    let mut pos = 0;
    while let Some(c) = src.get(pos..).and_then(|rest| rest.chars().next()) {
        let next = src
            .get(pos + c.len_utf8()..)
            .and_then(|rest| rest.chars().next());
        match (c, next) {
            ('{', Some('{')) | ('}', Some('}')) => {
                text.push(c);
                pos += 2;
            }
            ('{', _) => {
                if !text.is_empty() {
                    parts.push(AvtPart::Text(std::mem::take(&mut text)));
                }
                let mut parser = Parser::new(src, pos + 1, namespaces);
                if *parser.peek()? == Tok::RBrace {
                    return Err(Error::new(
                        ErrorKind::Syntax,
                        src,
                        pos,
                        "empty expression `{}` in an attribute value template".into(),
                    ));
                }
                let expr = parser.expr()?;
                let close = parser.expect(&Tok::RBrace, "`}`")?;
                parts.push(AvtPart::Expr(*expr.expr));
                pos = close.end;
            }
            ('}', _) => {
                return Err(Error::new(
                    ErrorKind::Syntax,
                    src,
                    pos,
                    "`}` in an attribute value template must be written `}}`".into(),
                ));
            }
            _ => {
                text.push(c);
                pos += c.len_utf8();
            }
        }
    }
    if !text.is_empty() {
        parts.push(AvtPart::Text(text));
    }
    Ok(Avt { parts })
}
