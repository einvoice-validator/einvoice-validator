//! Recursive-descent parser, following the grammar of XPath 2.0 (appendix A.1 of the
//! specification; production numbers are cited as \[n\]).
//!
//! Precedence, loosest first: `,` \[2\]; `for`/`some`/`every`/`if` \[3\]; `or` \[8\]; `and`
//! \[9\]; comparisons \[10\] (not associative); `to` \[11\] (not associative); `+ -` \[12\];
//! `* div idiv mod` \[13\]; `|` \[14\]; `instance of` \[16\]; `castable as` \[18\]; unary `-`
//! \[20\]; paths \[25\]; predicates and primaries \[38\], \[41\].
//!
//! The binary operators of productions \[8\] to \[14\] are read by one loop with an
//! operator stack ([`Parser::binary`]), and the operators of \[16\] to \[20\] by one
//! function ([`Parser::operand`]): the parser recurses only where the grammar nests (a
//! parenthesized expression, an argument, a predicate, the parts of `for`, `some`,
//! `every` and `if`), never along a chain of operators or of unary minus signs, and each
//! level of nesting costs a few frames. Every parsed expression carries its depth
//! ([`Parsed`]), computed bottom-up, and both the depth and the nesting of the parser
//! are bounded by [`MAX_DEPTH`](crate::MAX_DEPTH): beyond it the parse fails with
//! [`ErrorKind::TooDeep`].

mod avt;
mod path;
mod pattern;
mod types;

use std::collections::VecDeque;

use crate::MAX_DEPTH;
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

/// A parsed expression and its depth (see [`MAX_DEPTH`](crate::MAX_DEPTH)). The
/// expression is boxed: the parser passes it up from level to level, where it becomes
/// the boxed operand of the expression above, and a small value keeps the frames of the
/// recursion small.
#[derive(Debug)]
pub(crate) struct Parsed {
    pub(crate) expr: Box<Expr>,
    pub(crate) depth: u32,
}

impl Parsed {
    fn new(expr: Expr, depth: u32) -> Self {
        Self {
            expr: Box::new(expr),
            depth,
        }
    }

    /// A leaf: a literal, a variable reference, `.`, a call without argument.
    fn leaf(expr: Expr) -> Self {
        Self::new(expr, 1)
    }
}

/// A binary operator of productions \[8\] to \[14\].
#[derive(Debug, Clone, Copy)]
enum BinaryOp {
    Or,
    And,
    Comparison(ComparisonOp),
    /// A node comparison (\[24\] `NodeComp`: `is`, `<<`, `>>`), refused where it would
    /// apply.
    NodeComparison,
    Range,
    Arithmetic(ArithmeticOp),
    Union,
}

impl BinaryOp {
    /// The binding strength, loosest first.
    const fn precedence(self) -> u8 {
        match self {
            Self::Or => 1,
            Self::And => 2,
            Self::Comparison(_) | Self::NodeComparison => 3,
            Self::Range => 4,
            Self::Arithmetic(ArithmeticOp::Add | ArithmeticOp::Subtract) => 5,
            Self::Arithmetic(_) => 6,
            Self::Union => 7,
        }
    }

    /// Whether `a op b op c` is `(a op b) op c`; otherwise (comparisons \[10\] and `to`
    /// \[11\]) the second operator ends the expression.
    const fn is_associative(self) -> bool {
        !matches!(
            self,
            Self::Comparison(_) | Self::NodeComparison | Self::Range
        )
    }

    /// The expression `lhs op rhs`; `None` for a node comparison, which is never built.
    fn build(self, lhs: Box<Expr>, rhs: Box<Expr>) -> Option<Expr> {
        Some(match self {
            Self::Or => Expr::Or(lhs, rhs),
            Self::And => Expr::And(lhs, rhs),
            Self::Comparison(op) => Expr::Comparison { op, lhs, rhs },
            Self::NodeComparison => return None,
            Self::Range => Expr::Range(lhs, rhs),
            Self::Arithmetic(op) => Expr::Arithmetic { op, lhs, rhs },
            Self::Union => Expr::Union(lhs, rhs),
        })
    }
}

/// The parser state: a lexer, a lookahead buffer, the namespace bindings and the nesting
/// of the expressions being read.
pub(crate) struct Parser<'a> {
    src: &'a str,
    lexer: Lexer<'a>,
    lookahead: VecDeque<Token>,
    namespaces: &'a dyn Namespaces,
    /// How many expressions (\[3\] `ExprSingle`, and parenthesized patterns) are being
    /// read one inside another: the recursion of the parser.
    nesting: u32,
}

impl<'a> Parser<'a> {
    /// A parser that reads `src` from byte `pos`.
    pub(crate) fn new(src: &'a str, pos: usize, namespaces: &'a dyn Namespaces) -> Self {
        Self {
            src,
            lexer: Lexer::new(src, pos),
            lookahead: VecDeque::new(),
            namespaces,
            nesting: 0,
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

    /// Consumes the current token if it is `tok`.
    fn eat(&mut self, tok: &Tok) -> Result<bool> {
        if self.peek()? == tok {
            self.bump()?;
            Ok(true)
        } else {
            Ok(false)
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

    /// The error for an expression deeper than [`MAX_DEPTH`], found at `offset`.
    fn too_deep(&self, offset: usize) -> Error {
        self.error_at(
            ErrorKind::TooDeep,
            offset,
            format!("expression nested deeper than the maximum depth of {MAX_DEPTH}"),
        )
    }

    /// `depth`, if it is within [`MAX_DEPTH`]; `offset` is where the expression that deep
    /// was found.
    fn checked(&self, depth: u32, offset: usize) -> Result<u32> {
        if depth > MAX_DEPTH {
            Err(self.too_deep(offset))
        } else {
            Ok(depth)
        }
    }

    /// One level deeper in the recursion of the parser, refused beyond [`MAX_DEPTH`]
    /// before anything is read at that level.
    fn enter(&mut self) -> Result<()> {
        if self.nesting >= MAX_DEPTH {
            let offset = self.offset()?;
            return Err(self.too_deep(offset));
        }
        self.nesting += 1;
        Ok(())
    }

    /// Back from a level entered with [`Parser::enter`].
    fn leave(&mut self) {
        self.nesting = self.nesting.saturating_sub(1);
    }

    /// One more level over the deepest of `depths`: the depth of an expression whose
    /// operands have those depths.
    fn above(&self, depths: impl IntoIterator<Item = u32>, offset: usize) -> Result<u32> {
        let deepest = depths.into_iter().max().unwrap_or(0);
        self.checked(deepest.saturating_add(1), offset)
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
    pub(crate) fn expr(&mut self) -> Result<Parsed> {
        let offset = self.offset()?;
        let first = self.expr_single()?;
        if *self.peek()? != Tok::Comma {
            return Ok(first);
        }
        self.sequence(first, offset)
    }

    /// The items after the first of a sequence (\[2\] `Expr` with a comma).
    fn sequence(&mut self, first: Parsed, offset: usize) -> Result<Parsed> {
        let mut items = Items::new(first);
        while self.eat(&Tok::Comma)? {
            let item = self.expr_single()?;
            items.push(item);
        }
        let depth = self.above([items.deepest], offset)?;
        Ok(Parsed::new(Expr::Sequence(items.exprs), depth))
    }

    /// \[3\] `ExprSingle ::= ForExpr | QuantifiedExpr | IfExpr | OrExpr`. The keywords are
    /// recognised only when followed by `$` (or `(` for `if`): otherwise they are names.
    /// Every nested expression is read through here: this is where the nesting of the
    /// parser is counted.
    pub(crate) fn expr_single(&mut self) -> Result<Parsed> {
        self.enter()?;
        let parsed = match self.single_kind()? {
            Single::For => self.for_expr(),
            Single::Quantified => self.quantified_expr(),
            Single::If => self.if_expr(),
            Single::Binary => self.binary(),
        }?;
        self.leave();
        Ok(parsed)
    }

    /// Which form of \[3\] `ExprSingle` starts at the current token.
    fn single_kind(&mut self) -> Result<Single> {
        let keyword = match self.peek()? {
            Tok::Name {
                prefix: None,
                local,
            } if matches!(
                local.as_str(),
                "for" | "some" | "every" | "let" | "if" | "typeswitch"
            ) =>
            {
                local.clone()
            }
            _ => return Ok(Single::Binary),
        };
        let next_is_dollar = *self.peek2()? == Tok::Dollar;
        let next_is_paren = *self.peek2()? == Tok::LParen;
        Ok(match keyword.as_str() {
            "for" if next_is_dollar => Single::For,
            "some" | "every" if next_is_dollar => Single::Quantified,
            "let" if next_is_dollar => {
                return Err(self.error_here(ErrorKind::Unsupported, "`let` expression (XPath 3.0)"));
            }
            "if" if next_is_paren => Single::If,
            "typeswitch" if next_is_paren => {
                return Err(self.error_here(ErrorKind::Unsupported, "`typeswitch` expression"));
            }
            _ => Single::Binary,
        })
    }

    /// `$name in ExprSingle ("," $name in ExprSingle)*`, shared by \[5\] and \[6\], with
    /// the depth of each binding sequence.
    fn bindings(&mut self) -> Result<Bound> {
        let mut bound = Bound::default();
        loop {
            let variable = self.binding_variable()?;
            let sequence = self.expr_single()?;
            if !self.binding_end(&mut bound, variable, sequence)? {
                return Ok(bound);
            }
        }
    }

    /// `$name in` of a binding.
    fn binding_variable(&mut self) -> Result<QName> {
        self.expect(&Tok::Dollar, "`$`")?;
        let variable = self.qname(NameRole::NoNamespace, "a variable name")?;
        self.expect_word("in")?;
        Ok(variable)
    }

    /// Records a binding; whether another one follows (after a comma). Each binding is
    /// one level deeper than the one before it: one too many is refused at once.
    fn binding_end(
        &mut self,
        bound: &mut Bound,
        variable: QName,
        sequence: Parsed,
    ) -> Result<bool> {
        bound.bindings.push(Binding {
            variable,
            sequence: *sequence.expr,
        });
        bound.depths.push(sequence.depth);
        let offset = self.offset()?;
        let levels = u32::try_from(bound.depths.len()).unwrap_or(u32::MAX);
        self.checked(levels.saturating_add(1), offset)?;
        self.eat(&Tok::Comma)
    }

    /// The depth of a `for`, `some` or `every` expression: each binding is one level
    /// over the next one, the last over the body (as `for $a in A, $b in B return C` is
    /// `for $a in A return for $b in B return C`).
    fn bound_depth(&self, depths: &[u32], body: u32, offset: usize) -> Result<u32> {
        let mut depth = body;
        for &sequence in depths.iter().rev() {
            depth = self.above([depth, sequence], offset)?;
        }
        Ok(depth)
    }

    /// \[4\] `ForExpr ::= SimpleForClause "return" ExprSingle`, \[5\] `SimpleForClause`.
    fn for_expr(&mut self) -> Result<Parsed> {
        let offset = self.bump()?.start;
        let bound = self.bindings()?;
        self.expect_word("return")?;
        let body = self.expr_single()?;
        self.for_end(bound, body, offset)
    }

    fn for_end(&self, bound: Bound, body: Parsed, offset: usize) -> Result<Parsed> {
        let depth = self.bound_depth(&bound.depths, body.depth, offset)?;
        let expr = Expr::For {
            bindings: bound.bindings,
            body: body.expr,
        };
        Ok(Parsed::new(expr, depth))
    }

    /// \[6\] `QuantifiedExpr ::= ("some" | "every") ... "satisfies" ExprSingle`.
    fn quantified_expr(&mut self) -> Result<Parsed> {
        let (quantifier, offset) = self.quantifier()?;
        let bound = self.bindings()?;
        self.expect_word("satisfies")?;
        let satisfies = self.expr_single()?;
        self.quantified_end(quantifier, bound, satisfies, offset)
    }

    fn quantifier(&mut self) -> Result<(Quantifier, usize)> {
        let token = self.bump()?;
        let quantifier = if token.tok.is_word("some") {
            Quantifier::Some
        } else {
            Quantifier::Every
        };
        Ok((quantifier, token.start))
    }

    fn quantified_end(
        &self,
        quantifier: Quantifier,
        bound: Bound,
        satisfies: Parsed,
        offset: usize,
    ) -> Result<Parsed> {
        let depth = self.bound_depth(&bound.depths, satisfies.depth, offset)?;
        let expr = Expr::Quantified {
            quantifier,
            bindings: bound.bindings,
            satisfies: satisfies.expr,
        };
        Ok(Parsed::new(expr, depth))
    }

    /// \[7\] `IfExpr ::= "if" "(" Expr ")" "then" ExprSingle "else" ExprSingle`.
    fn if_expr(&mut self) -> Result<Parsed> {
        let offset = self.if_start()?;
        let condition = self.expr()?;
        self.if_then()?;
        let then = self.expr_single()?;
        self.expect_word("else")?;
        let otherwise = self.expr_single()?;
        self.if_end([condition, then, otherwise], offset)
    }

    /// `if (`; the offset of `if`.
    fn if_start(&mut self) -> Result<usize> {
        let offset = self.bump()?.start;
        self.expect(&Tok::LParen, "`(`")?;
        Ok(offset)
    }

    /// `) then`
    fn if_then(&mut self) -> Result<()> {
        self.expect(&Tok::RParen, "`)`")?;
        self.expect_word("then")
    }

    fn if_end(&self, parts: [Parsed; 3], offset: usize) -> Result<Parsed> {
        let depth = self.above(parts.iter().map(|p| p.depth), offset)?;
        let [condition, then, otherwise] = parts;
        let expr = Expr::If {
            condition: condition.expr,
            then: then.expr,
            otherwise: otherwise.expr,
        };
        Ok(Parsed::new(expr, depth))
    }

    /// The binary operator at the current token, if any (productions \[8\] to \[14\]).
    /// The operators outside the subset are refused: `intersect` and `except` (\[15\])
    /// and the `union` keyword (\[14\]) here, node comparisons (\[24\]) by
    /// [`Parser::binary`] where they would apply.
    fn binary_op(&mut self) -> Result<Option<BinaryOp>> {
        use CompareKind::{Eq, Ge, Gt, Le, Lt, Ne};
        let general = |kind| Some(BinaryOp::Comparison(ComparisonOp::General(kind)));
        let value = |kind| Some(BinaryOp::Comparison(ComparisonOp::Value(kind)));
        let arithmetic = |op| Some(BinaryOp::Arithmetic(op));
        let op = match self.peek()? {
            Tok::Pipe => Some(BinaryOp::Union),
            Tok::Star => arithmetic(ArithmeticOp::Multiply),
            Tok::Plus => arithmetic(ArithmeticOp::Add),
            Tok::Minus => arithmetic(ArithmeticOp::Subtract),
            Tok::Eq => general(Eq),
            Tok::Ne => general(Ne),
            Tok::Lt => general(Lt),
            Tok::Le => general(Le),
            Tok::Gt => general(Gt),
            Tok::Ge => general(Ge),
            Tok::Precedes | Tok::Follows => Some(BinaryOp::NodeComparison),
            Tok::Name {
                prefix: None,
                local,
            } => match local.as_str() {
                "intersect" | "except" => {
                    let message = format!("`{local}` operator");
                    return Err(self.error_here(ErrorKind::Unsupported, message));
                }
                "union" => {
                    return Err(self.error_here(
                        ErrorKind::Unsupported,
                        "`union` operator (only `|` is supported)",
                    ));
                }
                "div" => arithmetic(ArithmeticOp::Divide),
                "idiv" => arithmetic(ArithmeticOp::IntegerDivide),
                "mod" => arithmetic(ArithmeticOp::Modulo),
                "to" => Some(BinaryOp::Range),
                "eq" => value(Eq),
                "ne" => value(Ne),
                "lt" => value(Lt),
                "le" => value(Le),
                "gt" => value(Gt),
                "ge" => value(Ge),
                "is" => Some(BinaryOp::NodeComparison),
                "and" => Some(BinaryOp::And),
                "or" => Some(BinaryOp::Or),
                _ => None,
            },
            _ => None,
        };
        Ok(op)
    }

    /// Productions \[8\] `OrExpr` to \[14\] `UnionExpr`, read without recursion: operands
    /// ([`Parser::operand`]) and operators in one loop, an operator applied as soon as a
    /// looser one, or the end, follows it. Left-associative chains (`a + b - c`, `a or b
    /// or c`) are as long as the text makes them; a second comparison or `to` at the same
    /// level ends the expression, as the grammar says (`a = b = c` leaves `= c` unread).
    fn binary(&mut self) -> Result<Parsed> {
        let mut chain = Chain::default();
        let first = self.operand()?;
        chain.operands.push(first);
        while self.chain_operator(&mut chain)? {
            let operand = self.operand()?;
            chain.operands.push(operand);
        }
        self.chain_end(chain)
    }

    /// After an operand of [`Parser::binary`]: reads the operator that follows, if any,
    /// applying the operators before it that bind at least as tightly; whether an
    /// operand follows.
    fn chain_operator(&mut self, chain: &mut Chain) -> Result<bool> {
        let Some(op) = self.binary_op()? else {
            return Ok(false);
        };
        let precedence = op.precedence();
        while let Some(&(top, _)) = chain.operators.last() {
            if top.precedence() < precedence {
                break;
            }
            if top.precedence() == precedence && !op.is_associative() {
                return Ok(false);
            }
            self.reduce(chain)?;
        }
        if let BinaryOp::NodeComparison = op {
            let found = match self.peek()? {
                Tok::Name { .. } => "`is`".to_owned(),
                other => other.describe(),
            };
            return Err(self.error_here(ErrorKind::Unsupported, format!("node comparison {found}")));
        }
        let offset = self.bump()?.start;
        chain.operators.push((op, offset));
        Ok(true)
    }

    /// The end of [`Parser::binary`]: the operators left are applied.
    fn chain_end(&self, mut chain: Chain) -> Result<Parsed> {
        while !chain.operators.is_empty() {
            self.reduce(&mut chain)?;
        }
        chain
            .operands
            .pop()
            .ok_or_else(|| self.error_at(ErrorKind::Syntax, 0, "no operand"))
    }

    /// Applies the last operator to the last two operands.
    fn reduce(&self, chain: &mut Chain) -> Result<()> {
        let internal = || self.error_at(ErrorKind::Syntax, 0, "operator without operands");
        let (op, offset) = chain.operators.pop().ok_or_else(internal)?;
        let rhs = chain.operands.pop().ok_or_else(internal)?;
        let lhs = chain.operands.pop().ok_or_else(internal)?;
        let depth = self.above([lhs.depth, rhs.depth], offset)?;
        let expr = op.build(lhs.expr, rhs.expr).ok_or_else(internal)?;
        chain.operands.push(Parsed::new(expr, depth));
        Ok(())
    }

    /// Whether the current and next tokens are the unprefixed names `first` and `second`.
    fn at_words(&mut self, first: &str, second: &str) -> Result<bool> {
        Ok(self.peek()?.is_word(first) && self.peek2()?.is_word(second))
    }

    /// The operand of a binary operator: \[16\] `InstanceofExpr ::= TreatExpr ("instance"
    /// "of" SequenceType)?`, \[17\] `TreatExpr ::= CastableExpr ("treat" "as"
    /// SequenceType)?` (not supported), \[18\] `CastableExpr ::= CastExpr ("castable" "as"
    /// SingleType)?`, \[19\] `CastExpr ::= UnaryExpr ("cast" "as" SingleType)?` and \[20\]
    /// `UnaryExpr ::= ("-" | "+")* ValueExpr` (only `-`), without recursion.
    fn operand(&mut self) -> Result<Parsed> {
        let negations = self.negations()?;
        let value = self.path_expr()?;
        self.postfix(&negations, value)
    }

    /// The `-` signs before a \[20\] `UnaryExpr`, by offset; `+` is refused. Each sign is
    /// one level: one too many is refused at once.
    fn negations(&mut self) -> Result<Vec<usize>> {
        let mut negations = Vec::new();
        loop {
            match self.peek()? {
                Tok::Minus => {
                    let offset = self.bump()?.start;
                    let levels = u32::try_from(negations.len()).unwrap_or(u32::MAX);
                    self.checked(levels.saturating_add(2), offset)?;
                    negations.push(offset);
                }
                Tok::Plus => return Err(self.error_here(ErrorKind::Unsupported, "unary `+`")),
                _ => return Ok(negations),
            }
        }
    }

    /// The unary minus signs (`negations`) applied to `value`, then the `cast as`,
    /// `castable as`, `treat as` and `instance of` that follow.
    fn postfix(&mut self, negations: &[usize], value: Parsed) -> Result<Parsed> {
        let mut operand = value;
        for &offset in negations.iter().rev() {
            let depth = self.above([operand.depth], offset)?;
            operand = Parsed::new(Expr::Negate(operand.expr), depth);
        }
        if self.at_words("cast", "as")? {
            let offset = self.bump()?.start;
            self.bump()?;
            let target = self.single_type()?;
            let depth = self.above([operand.depth], offset)?;
            let expr = Expr::Cast {
                operand: operand.expr,
                target,
            };
            operand = Parsed::new(expr, depth);
        }
        if self.at_words("castable", "as")? {
            let offset = self.bump()?.start;
            self.bump()?;
            let target = self.single_type()?;
            let depth = self.above([operand.depth], offset)?;
            let expr = Expr::Castable {
                operand: operand.expr,
                target,
            };
            operand = Parsed::new(expr, depth);
        }
        if self.at_words("treat", "as")? {
            return Err(self.error_here(ErrorKind::Unsupported, "`treat as` expression"));
        }
        if self.at_words("instance", "of")? {
            let offset = self.bump()?.start;
            self.bump()?;
            let target = self.sequence_type()?;
            let depth = self.above([operand.depth], offset)?;
            let expr = Expr::InstanceOf {
                operand: operand.expr,
                target,
            };
            operand = Parsed::new(expr, depth);
        }
        Ok(operand)
    }
}

/// The form of a \[3\] `ExprSingle`.
#[derive(Debug, Clone, Copy)]
enum Single {
    For,
    Quantified,
    If,
    Binary,
}

/// The items of a sequence being read, and the depth of the deepest.
#[derive(Debug)]
struct Items {
    exprs: Vec<Expr>,
    deepest: u32,
}

impl Items {
    fn new(first: Parsed) -> Self {
        Self {
            deepest: first.depth,
            exprs: vec![*first.expr],
        }
    }

    fn push(&mut self, item: Parsed) {
        self.deepest = self.deepest.max(item.depth);
        self.exprs.push(*item.expr);
    }
}

/// The bindings of a `for`, `some` or `every` being read, and the depth of each.
#[derive(Debug, Default)]
struct Bound {
    bindings: Vec<Binding>,
    depths: Vec<u32>,
}

/// The operands and operators of [`Parser::binary`] not yet applied.
#[derive(Debug, Default)]
struct Chain {
    operands: Vec<Parsed>,
    operators: Vec<(BinaryOp, usize)>,
}
