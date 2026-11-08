//! Recursive-descent parser, one function per grammar level of XPath 2.0 (appendix A.1 of
//! the specification; production numbers are cited as \[n\]).
//!
//! Precedence, loosest first: `,` \[2\]; `for`/`some`/`every`/`if` \[3\]; `or` \[8\]; `and`
//! \[9\]; comparisons \[10\] (not associative); `to` \[11\] (not associative); `+ -` \[12\];
//! `* div idiv mod` \[13\]; `|` \[14\]; `instance of` \[16\]; `castable as` \[18\]; unary `-`
//! \[20\]; paths \[25\]; predicates and primaries \[38\], \[41\].

mod avt;
mod path;
mod pattern;
mod types;

use std::collections::VecDeque;

use crate::ast::{ArithmeticOp, Binding, CompareKind, ComparisonOp, Expr, Quantifier};
use crate::error::{Error, ErrorKind, Result};
use crate::lexer::{Lexer, Tok, Token};
use crate::names::{Namespaces, QName, XML_NAMESPACE};

pub(crate) use avt::parse_avt;

/// What an unprefixed name resolves to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NameRole {
    /// Element and type names: the default element namespace.
    Element,
    /// Attribute and variable names: no namespace.
    NoNamespace,
    /// Function names: the default function namespace.
    Function,
}

/// The parser state: a lexer, a lookahead buffer and the namespace bindings.
pub(crate) struct Parser<'a> {
    src: &'a str,
    lexer: Lexer<'a>,
    lookahead: VecDeque<Token>,
    namespaces: &'a dyn Namespaces,
}

impl<'a> Parser<'a> {
    /// A parser that reads `src` from byte `pos`.
    pub(crate) fn new(src: &'a str, pos: usize, namespaces: &'a dyn Namespaces) -> Self {
        Self {
            src,
            lexer: Lexer::new(src, pos),
            lookahead: VecDeque::new(),
            namespaces,
        }
    }

    fn fill(&mut self, n: usize) -> Result<()> {
        while self.lookahead.len() <= n {
            let token = self.lexer.next_token()?;
            self.lookahead.push_back(token);
        }
        Ok(())
    }

    /// The token at lookahead position `n` (0 is the current token).
    fn peek_at(&mut self, n: usize) -> Result<&Token> {
        self.fill(n)?;
        self.lookahead.get(n).ok_or_else(|| {
            Error::new(
                ErrorKind::Syntax,
                self.src,
                self.src.len(),
                "lookahead".into(),
            )
        })
    }

    fn peek(&mut self) -> Result<&Tok> {
        Ok(&self.peek_at(0)?.tok)
    }

    fn peek2(&mut self) -> Result<&Tok> {
        Ok(&self.peek_at(1)?.tok)
    }

    fn bump(&mut self) -> Result<Token> {
        self.fill(0)?;
        self.lookahead.pop_front().ok_or_else(|| {
            Error::new(
                ErrorKind::Syntax,
                self.src,
                self.src.len(),
                "lookahead".into(),
            )
        })
    }

    fn offset(&mut self) -> Result<usize> {
        Ok(self.peek_at(0)?.start)
    }

    fn error_at(&self, kind: ErrorKind, offset: usize, message: impl Into<String>) -> Error {
        Error::new(kind, self.src, offset, message.into())
    }

    /// An error about the current token.
    fn error_here(&mut self, kind: ErrorKind, message: impl Into<String>) -> Error {
        match self.peek_at(0) {
            Ok(token) => {
                let offset = token.start;
                self.error_at(kind, offset, message)
            }
            Err(e) => e,
        }
    }

    fn unexpected(&mut self, expected: &str) -> Error {
        match self.peek_at(0) {
            Ok(token) => {
                let (offset, found) = (token.start, token.tok.describe());
                self.error_at(
                    ErrorKind::Syntax,
                    offset,
                    format!("expected {expected}, found {found}"),
                )
            }
            Err(e) => e,
        }
    }

    fn expect(&mut self, tok: &Tok, expected: &str) -> Result<Token> {
        if self.peek()? == tok {
            self.bump()
        } else {
            Err(self.unexpected(expected))
        }
    }

    /// Consumes the current token if it is the unprefixed name `word`.
    fn eat_word(&mut self, word: &str) -> Result<bool> {
        if self.peek()?.is_word(word) {
            self.bump()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn expect_word(&mut self, word: &str) -> Result<()> {
        if self.eat_word(word)? {
            Ok(())
        } else {
            Err(self.unexpected(&format!("`{word}`")))
        }
    }

    /// Resolves a written name. `xml` is always bound; other prefixes come from the caller.
    fn resolve(
        &self,
        prefix: Option<String>,
        local: String,
        role: NameRole,
        offset: usize,
    ) -> Result<QName> {
        let namespace = match &prefix {
            Some(p) if p == "xml" => Some(XML_NAMESPACE.to_owned()),
            Some(p) => Some(self.namespaces.namespace_uri(p).ok_or_else(|| {
                self.error_at(
                    ErrorKind::UndeclaredPrefix,
                    offset,
                    format!("prefix `{p}` is not declared"),
                )
            })?),
            None => match role {
                NameRole::Element => self.namespaces.default_element_namespace(),
                NameRole::NoNamespace => None,
                NameRole::Function => Some(crate::names::FN_NAMESPACE.to_owned()),
            },
        };
        Ok(QName {
            prefix,
            namespace,
            local,
        })
    }

    /// Resolves a namespace prefix of a `prefix:*` wildcard.
    fn resolve_prefix(&self, prefix: &str, offset: usize) -> Result<String> {
        if prefix == "xml" {
            return Ok(XML_NAMESPACE.to_owned());
        }
        self.namespaces.namespace_uri(prefix).ok_or_else(|| {
            self.error_at(
                ErrorKind::UndeclaredPrefix,
                offset,
                format!("prefix `{prefix}` is not declared"),
            )
        })
    }

    /// Consumes a `QName` token and resolves it.
    fn qname(&mut self, role: NameRole, expected: &str) -> Result<QName> {
        let token = self.bump()?;
        match token.tok {
            Tok::Name { prefix, local } => self.resolve(prefix, local, role, token.start),
            other => Err(self.error_at(
                ErrorKind::Syntax,
                token.start,
                format!("expected {expected}, found {}", other.describe()),
            )),
        }
    }

    /// Fails unless the whole input has been read.
    pub(crate) fn expect_end(&mut self) -> Result<()> {
        if *self.peek()? == Tok::Eof {
            Ok(())
        } else {
            let found = self.peek()?.describe();
            Err(self.error_here(
                ErrorKind::Syntax,
                format!("unexpected {found} after the end of the expression"),
            ))
        }
    }

    /// \[2\] `Expr ::= ExprSingle ("," ExprSingle)*`
    pub(crate) fn expr(&mut self) -> Result<Expr> {
        let first = self.expr_single()?;
        if *self.peek()? != Tok::Comma {
            return Ok(first);
        }
        let mut items = vec![first];
        while *self.peek()? == Tok::Comma {
            self.bump()?;
            items.push(self.expr_single()?);
        }
        Ok(Expr::Sequence(items))
    }

    /// \[3\] `ExprSingle ::= ForExpr | QuantifiedExpr | IfExpr | OrExpr`. The keywords are
    /// recognised only when followed by `$` (or `(` for `if`): otherwise they are names.
    pub(crate) fn expr_single(&mut self) -> Result<Expr> {
        let next_is_dollar =
            matches!(self.peek()?, Tok::Name { prefix: None, .. }) && *self.peek2()? == Tok::Dollar;
        let next_is_paren =
            matches!(self.peek()?, Tok::Name { prefix: None, .. }) && *self.peek2()? == Tok::LParen;
        let tok = self.peek()?.clone();
        if next_is_dollar {
            if tok.is_word("for") {
                return self.for_expr();
            }
            if tok.is_word("some") || tok.is_word("every") {
                return self.quantified_expr();
            }
            if tok.is_word("let") {
                return Err(self.error_here(ErrorKind::Unsupported, "`let` expression (XPath 3.0)"));
            }
        }
        if next_is_paren {
            if tok.is_word("if") {
                return self.if_expr();
            }
            if tok.is_word("typeswitch") {
                return Err(self.error_here(ErrorKind::Unsupported, "`typeswitch` expression"));
            }
        }
        self.or_expr()
    }

    /// `$name in ExprSingle ("," $name in ExprSingle)*`, shared by \[5\] and \[6\].
    fn bindings(&mut self) -> Result<Vec<Binding>> {
        let mut bindings = Vec::new();
        loop {
            self.expect(&Tok::Dollar, "`$`")?;
            let variable = self.qname(NameRole::NoNamespace, "a variable name")?;
            self.expect_word("in")?;
            let sequence = self.expr_single()?;
            bindings.push(Binding { variable, sequence });
            if *self.peek()? != Tok::Comma {
                return Ok(bindings);
            }
            self.bump()?;
        }
    }

    /// \[4\] `ForExpr ::= SimpleForClause "return" ExprSingle`, \[5\] `SimpleForClause`.
    fn for_expr(&mut self) -> Result<Expr> {
        self.bump()?;
        let bindings = self.bindings()?;
        self.expect_word("return")?;
        let body = Box::new(self.expr_single()?);
        Ok(Expr::For { bindings, body })
    }

    /// \[6\] `QuantifiedExpr ::= ("some" | "every") ... "satisfies" ExprSingle`.
    fn quantified_expr(&mut self) -> Result<Expr> {
        let quantifier = if self.bump()?.tok.is_word("some") {
            Quantifier::Some
        } else {
            Quantifier::Every
        };
        let bindings = self.bindings()?;
        self.expect_word("satisfies")?;
        let satisfies = Box::new(self.expr_single()?);
        Ok(Expr::Quantified {
            quantifier,
            bindings,
            satisfies,
        })
    }

    /// \[7\] `IfExpr ::= "if" "(" Expr ")" "then" ExprSingle "else" ExprSingle`.
    fn if_expr(&mut self) -> Result<Expr> {
        self.bump()?;
        self.expect(&Tok::LParen, "`(`")?;
        let condition = Box::new(self.expr()?);
        self.expect(&Tok::RParen, "`)`")?;
        self.expect_word("then")?;
        let then = Box::new(self.expr_single()?);
        self.expect_word("else")?;
        let otherwise = Box::new(self.expr_single()?);
        Ok(Expr::If {
            condition,
            then,
            otherwise,
        })
    }

    /// \[8\] `OrExpr ::= AndExpr ( "or" AndExpr )*`
    fn or_expr(&mut self) -> Result<Expr> {
        let mut lhs = self.and_expr()?;
        while self.eat_word("or")? {
            let rhs = self.and_expr()?;
            lhs = Expr::Or(Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    /// \[9\] `AndExpr ::= ComparisonExpr ( "and" ComparisonExpr )*`
    fn and_expr(&mut self) -> Result<Expr> {
        let mut lhs = self.comparison_expr()?;
        while self.eat_word("and")? {
            let rhs = self.comparison_expr()?;
            lhs = Expr::And(Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    /// The comparison operator at the current token, if any. Node comparisons (\[24\]) are
    /// refused.
    fn comparison_op(&mut self) -> Result<Option<ComparisonOp>> {
        use CompareKind::{Eq, Ge, Gt, Le, Lt, Ne};
        let op = match self.peek()? {
            Tok::Eq => ComparisonOp::General(Eq),
            Tok::Ne => ComparisonOp::General(Ne),
            Tok::Lt => ComparisonOp::General(Lt),
            Tok::Le => ComparisonOp::General(Le),
            Tok::Gt => ComparisonOp::General(Gt),
            Tok::Ge => ComparisonOp::General(Ge),
            Tok::Precedes | Tok::Follows => {
                let found = self.peek()?.describe();
                return Err(
                    self.error_here(ErrorKind::Unsupported, format!("node comparison {found}"))
                );
            }
            Tok::Name {
                prefix: None,
                local,
            } => match local.as_str() {
                "eq" => ComparisonOp::Value(Eq),
                "ne" => ComparisonOp::Value(Ne),
                "lt" => ComparisonOp::Value(Lt),
                "le" => ComparisonOp::Value(Le),
                "gt" => ComparisonOp::Value(Gt),
                "ge" => ComparisonOp::Value(Ge),
                "is" => {
                    return Err(self.error_here(ErrorKind::Unsupported, "node comparison `is`"));
                }
                _ => return Ok(None),
            },
            _ => return Ok(None),
        };
        Ok(Some(op))
    }

    /// \[10\] `ComparisonExpr ::= RangeExpr ( (ValueComp | GeneralComp | NodeComp) RangeExpr )?`
    fn comparison_expr(&mut self) -> Result<Expr> {
        let lhs = self.range_expr()?;
        let Some(op) = self.comparison_op()? else {
            return Ok(lhs);
        };
        self.bump()?;
        let rhs = self.range_expr()?;
        Ok(Expr::Comparison {
            op,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
        })
    }

    /// \[11\] `RangeExpr ::= AdditiveExpr ( "to" AdditiveExpr )?`
    fn range_expr(&mut self) -> Result<Expr> {
        let lhs = self.additive_expr()?;
        if !self.eat_word("to")? {
            return Ok(lhs);
        }
        let rhs = self.additive_expr()?;
        Ok(Expr::Range(Box::new(lhs), Box::new(rhs)))
    }

    /// \[12\] `AdditiveExpr ::= MultiplicativeExpr ( ("+" | "-") MultiplicativeExpr )*`
    fn additive_expr(&mut self) -> Result<Expr> {
        let mut lhs = self.multiplicative_expr()?;
        loop {
            let op = match self.peek()? {
                Tok::Plus => ArithmeticOp::Add,
                Tok::Minus => ArithmeticOp::Subtract,
                _ => return Ok(lhs),
            };
            self.bump()?;
            let rhs = self.multiplicative_expr()?;
            lhs = Expr::Arithmetic {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
    }

    /// \[13\] `MultiplicativeExpr ::= UnionExpr ( ("*" | "div" | "idiv" | "mod") UnionExpr )*`
    fn multiplicative_expr(&mut self) -> Result<Expr> {
        let mut lhs = self.union_expr()?;
        loop {
            let tok = self.peek()?;
            let op = if *tok == Tok::Star {
                ArithmeticOp::Multiply
            } else if tok.is_word("div") {
                ArithmeticOp::Divide
            } else if tok.is_word("idiv") {
                ArithmeticOp::IntegerDivide
            } else if tok.is_word("mod") {
                ArithmeticOp::Modulo
            } else {
                return Ok(lhs);
            };
            self.bump()?;
            let rhs = self.union_expr()?;
            lhs = Expr::Arithmetic {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
    }

    /// \[14\] `UnionExpr ::= IntersectExceptExpr ( ("union" | "|") IntersectExceptExpr )*`.
    /// Only `|` is supported.
    fn union_expr(&mut self) -> Result<Expr> {
        let mut lhs = self.intersect_except_expr()?;
        loop {
            if self.peek()?.is_word("union") {
                return Err(self.error_here(
                    ErrorKind::Unsupported,
                    "`union` operator (only `|` is supported)",
                ));
            }
            if *self.peek()? != Tok::Pipe {
                return Ok(lhs);
            }
            self.bump()?;
            let rhs = self.intersect_except_expr()?;
            lhs = Expr::Union(Box::new(lhs), Box::new(rhs));
        }
    }

    /// \[15\] `IntersectExceptExpr ::= InstanceofExpr ( ("intersect" | "except") InstanceofExpr )*`.
    /// Neither operator is supported.
    fn intersect_except_expr(&mut self) -> Result<Expr> {
        let lhs = self.instanceof_expr()?;
        for word in ["intersect", "except"] {
            if self.peek()?.is_word(word) {
                return Err(self.error_here(ErrorKind::Unsupported, format!("`{word}` operator")));
            }
        }
        Ok(lhs)
    }

    /// Whether the current and next tokens are the unprefixed names `first` and `second`.
    fn at_words(&mut self, first: &str, second: &str) -> Result<bool> {
        Ok(self.peek()?.is_word(first) && self.peek2()?.is_word(second))
    }

    /// \[16\] `InstanceofExpr ::= TreatExpr ( "instance" "of" SequenceType )?`
    fn instanceof_expr(&mut self) -> Result<Expr> {
        let operand = self.treat_expr()?;
        if !self.at_words("instance", "of")? {
            return Ok(operand);
        }
        self.bump()?;
        self.bump()?;
        let target = self.sequence_type()?;
        Ok(Expr::InstanceOf {
            operand: Box::new(operand),
            target,
        })
    }

    /// \[17\] `TreatExpr ::= CastableExpr ( "treat" "as" SequenceType )?`. Not supported.
    fn treat_expr(&mut self) -> Result<Expr> {
        let operand = self.castable_expr()?;
        if self.at_words("treat", "as")? {
            return Err(self.error_here(ErrorKind::Unsupported, "`treat as` expression"));
        }
        Ok(operand)
    }

    /// \[18\] `CastableExpr ::= CastExpr ( "castable" "as" SingleType )?`
    fn castable_expr(&mut self) -> Result<Expr> {
        let operand = self.cast_expr()?;
        if !self.at_words("castable", "as")? {
            return Ok(operand);
        }
        self.bump()?;
        self.bump()?;
        let target = self.single_type()?;
        Ok(Expr::Castable {
            operand: Box::new(operand),
            target,
        })
    }

    /// \[19\] `CastExpr ::= UnaryExpr ( "cast" "as" SingleType )?`
    fn cast_expr(&mut self) -> Result<Expr> {
        let operand = self.unary_expr()?;
        if !self.at_words("cast", "as")? {
            return Ok(operand);
        }
        self.bump()?;
        self.bump()?;
        let target = self.single_type()?;
        Ok(Expr::Cast {
            operand: Box::new(operand),
            target,
        })
    }

    /// \[20\] `UnaryExpr ::= ("-" | "+")* ValueExpr`. Only `-` is supported.
    fn unary_expr(&mut self) -> Result<Expr> {
        match self.peek()? {
            Tok::Minus => {
                self.bump()?;
                Ok(Expr::Negate(Box::new(self.unary_expr()?)))
            }
            Tok::Plus => Err(self.error_here(ErrorKind::Unsupported, "unary `+`")),
            _ => self.path_expr(),
        }
    }
}
