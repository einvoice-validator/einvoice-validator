//! Sequence constructors: instructions, literal result elements and text.

use roxmltree::Node;

use super::declarations::BindingKind;
use super::strip::{Child, children};
use super::{Failed, InScope, Load, Loader, XSL, is_xsl};
use crate::MAX_NESTING;
use crate::error::LoadErrorKind;
use crate::ir::{
    ApplyMode, ApplyTemplates, Attribute, CallTemplate, Choose, Comment, ForEach, If, Instruction,
    LiteralAttribute, LiteralElement, Mode, NextMatch, Number, NumberLevel, Sequence, Text,
    TextOrigin, ValueOf, Variable, When,
};
use einvoice_xpath::{is_xml_whitespace, parse_avt, parse_expr, parse_pattern};

/// XSLT elements that are supported but not as instructions.
const DECLARATIONS: [&str; 8] = [
    "template",
    "function",
    "output",
    "param",
    "with-param",
    "when",
    "otherwise",
    "stylesheet",
];

impl<'input> Loader<'_, 'input> {
    /// The content of `node` as a sequence constructor.
    pub(crate) fn sequence_constructor(
        &mut self,
        node: Node<'_, 'input>,
    ) -> Load<Vec<Instruction>> {
        self.instructions(children(node))
    }

    /// A list of stripped children as instructions: a sequence constructor, one level
    /// deeper than the one that contains it, refused beyond [`MAX_NESTING`].
    pub(crate) fn instructions(&mut self, items: Vec<Child<'_, 'input>>) -> Load<Vec<Instruction>> {
        let Some(first) = items.first() else {
            return Ok(Vec::new());
        };
        if self.nesting >= MAX_NESTING {
            let line = match first {
                Child::Element(node) => self.line(*node),
                Child::Text { offset, .. } => self.line_at(*offset),
            };
            return Err(self.fail(line, LoadErrorKind::TooDeep { max: MAX_NESTING }));
        }
        self.nesting += 1;
        let instructions = super::each(items, |child| self.instruction(child));
        self.nesting -= 1;
        instructions
    }

    /// One instruction. Each kind is read by a function of its own, so that the
    /// recursion over nested instructions holds, at each level, only what that level's
    /// instruction needs (the call stack of a debug build keeps every local of a
    /// function).
    fn instruction(&mut self, child: Child<'_, 'input>) -> Load<Instruction> {
        let node = match child {
            Child::Text { value, offset } => {
                return Ok(Instruction::Text(Text {
                    line: self.line_at(offset),
                    value,
                    origin: TextOrigin::Literal,
                }));
            }
            Child::Element(node) => node,
        };
        if !is_xsl(node) {
            return self.literal_element(node);
        }
        match node.tag_name().name() {
            "apply-templates" => self.apply_templates(node),
            "next-match" => self.next_match(node),
            "call-template" => self.call_template(node),
            "choose" => self.choose(node),
            "if" => self.if_instruction(node),
            "for-each" => self.for_each(node),
            "variable" => self.local_variable(node),
            "sequence" => self.sequence(node),
            "value-of" => self.value_of(node),
            "attribute" => self.attribute(node),
            "comment" => self.comment(node),
            "text" => self.xsl_text(node).map(Instruction::Text),
            "number" => self.number(node),
            name if DECLARATIONS.contains(&name) => Err(self.not_allowed_here(node)),
            _ => Err(self.unsupported_element(node)),
        }
    }

    /// The error for a declaration used as an instruction.
    fn not_allowed_here(&mut self, node: Node<'_, 'input>) -> Failed {
        let message = format!("{} is not allowed here (XTSE0010)", self.element_name(node));
        self.invalid(node, message)
    }

    /// `xsl:next-match`.
    fn next_match(&mut self, node: Node<'_, 'input>) -> Load<Instruction> {
        self.check_attributes(node, &[])?;
        Ok(Instruction::NextMatch(NextMatch {
            line: self.line(node),
            params: self.with_params(node)?,
        }))
    }

    /// `xsl:call-template`.
    fn call_template(&mut self, node: Node<'_, 'input>) -> Load<Instruction> {
        let attributes = self.check_attributes(node, &["name"]);
        let name = self.required_qname(node, "name").map(Box::new);
        let params = self.with_params(node);
        let (name, params) = (name?, params?);
        attributes?;
        Ok(Instruction::CallTemplate(CallTemplate {
            line: self.line(node),
            name: *name,
            params,
        }))
    }

    /// `xsl:if`.
    fn if_instruction(&mut self, node: Node<'_, 'input>) -> Load<Instruction> {
        let (test, body) = self.test_and_body(node)?;
        Ok(Instruction::If(If {
            line: self.line(node),
            test: *test,
            body,
        }))
    }

    /// `xsl:for-each`.
    fn for_each(&mut self, node: Node<'_, 'input>) -> Load<Instruction> {
        let attributes = self.check_attributes(node, &["select"]);
        let select = self
            .required_xpath(node, "select", parse_expr)
            .map(Box::new);
        let body = self.sequence_constructor(node);
        let (select, body) = (select?, body?);
        attributes?;
        Ok(Instruction::ForEach(ForEach {
            line: self.line(node),
            select: *select,
            body,
        }))
    }

    /// A local `xsl:variable`.
    fn local_variable(&mut self, node: Node<'_, 'input>) -> Load<Instruction> {
        self.variable(node, BindingKind::Variable)
            .map(Instruction::Variable)
    }

    /// `xsl:sequence`.
    fn sequence(&mut self, node: Node<'_, 'input>) -> Load<Instruction> {
        let attributes = self.check_attributes(node, &["select"]);
        let select = self.required_xpath(node, "select", parse_expr);
        let empty = self.empty_content(node);
        let (select, ()) = (select?, empty?);
        attributes?;
        Ok(Instruction::Sequence(Sequence {
            line: self.line(node),
            select,
        }))
    }

    /// `xsl:comment`.
    fn comment(&mut self, node: Node<'_, 'input>) -> Load<Instruction> {
        self.check_attributes(node, &[])?;
        Ok(Instruction::Comment(Comment {
            line: self.line(node),
            body: self.sequence_constructor(node)?,
        }))
    }

    /// The `xsl:with-param` children of `node`, which may have no other child.
    fn with_params(&mut self, node: Node<'_, 'input>) -> Load<Vec<Variable>> {
        let params = super::each(children(node), |child| match child {
            Child::Element(e) if e.has_tag_name((XSL, "with-param")) => {
                self.variable(e, BindingKind::Param)
            }
            Child::Element(e)
                if is_xsl(e)
                    && !DECLARATIONS.contains(&e.tag_name().name())
                    && e.tag_name().name() != "variable" =>
            {
                Err(self.unsupported_element(e))
            }
            Child::Element(e) => {
                let message = format!(
                    "{} is not allowed in {} (XTSE0010)",
                    self.element_name(e),
                    self.element_name(node)
                );
                Err(self.invalid(e, message))
            }
            Child::Text { offset, .. } => {
                let line = self.line_at(offset);
                let message = format!(
                    "text is not allowed in {} (XTSE0010)",
                    self.element_name(node)
                );
                Err(self.fail(line, LoadErrorKind::Invalid(message)))
            }
        })?;
        for (i, param) in params.iter().enumerate() {
            if params[..i]
                .iter()
                .any(|p| p.name.same_expanded_name(&param.name))
            {
                let message = format!("parameter `{}` is passed twice (XTSE0670)", param.name);
                return Err(self.fail(param.line, LoadErrorKind::Invalid(message)));
            }
        }
        Ok(params)
    }

    /// `xsl:apply-templates`.
    fn apply_templates(&mut self, node: Node<'_, 'input>) -> Load<Instruction> {
        let attributes = self.check_attributes(node, &["select", "mode"]);
        let select = self
            .xpath(node, "select", parse_expr)
            .map(|e| e.map(Box::new));
        let mode = match node
            .attribute("mode")
            .map(|v| v.trim_matches(is_xml_whitespace))
        {
            None | Some("#default") => Ok(ApplyMode::Mode(Mode::Default)),
            Some("#current") => Ok(ApplyMode::Current),
            Some(token) if token.starts_with('#') => {
                Err(self.unsupported_value(node, "mode", token))
            }
            Some(token) => self
                .qname_value(node, "mode", token)
                .map(|q| ApplyMode::Mode(Mode::Named(q))),
        };
        let params = self.with_params(node);
        let (select, mode, params) = (select?, mode?, params?);
        attributes?;
        Ok(Instruction::ApplyTemplates(ApplyTemplates {
            line: self.line(node),
            select: select.map(|e| *e),
            mode,
            params,
        }))
    }

    /// `test` and body of `xsl:if` or `xsl:when`.
    fn test_and_body(
        &mut self,
        node: Node<'_, 'input>,
    ) -> Load<(Box<einvoice_xpath::Expr>, Vec<Instruction>)> {
        let attributes = self.check_attributes(node, &["test"]);
        let test = self.required_xpath(node, "test", parse_expr).map(Box::new);
        let body = self.sequence_constructor(node);
        let (test, body) = (test?, body?);
        attributes?;
        Ok((test, body))
    }

    /// `xsl:choose`: one or more `xsl:when`, then at most one `xsl:otherwise`.
    fn choose(&mut self, node: Node<'_, 'input>) -> Load<Instruction> {
        self.check_attributes(node, &[])?;
        let mut whens = Vec::new();
        let mut otherwise = None;
        let mut failed = false;
        for child in children(node) {
            let result = match child {
                Child::Element(e) if e.has_tag_name((XSL, "when")) && otherwise.is_none() => {
                    self.test_and_body(e).map(|(test, body)| {
                        whens.push(When {
                            line: self.line(e),
                            test: *test,
                            body,
                        });
                    })
                }
                Child::Element(e)
                    if e.has_tag_name((XSL, "otherwise"))
                        && otherwise.is_none()
                        && !whens.is_empty() =>
                {
                    self.check_attributes(e, &[])
                        .and_then(|()| self.sequence_constructor(e))
                        .map(|body| otherwise = Some(body))
                }
                Child::Element(e) => {
                    let message = format!(
                        "{} is not allowed here in xsl:choose (XTSE0010)",
                        self.element_name(e)
                    );
                    Err(self.invalid(e, message))
                }
                Child::Text { offset, .. } => {
                    let line = self.line_at(offset);
                    Err(self.fail(
                        line,
                        LoadErrorKind::Invalid(
                            "text is not allowed in xsl:choose (XTSE0010)".into(),
                        ),
                    ))
                }
            };
            failed |= result.is_err();
        }
        if failed {
            return Err(Failed);
        }
        if whens.is_empty() {
            return Err(self.invalid(
                node,
                "xsl:choose needs at least one xsl:when (XTSE0010)".into(),
            ));
        }
        Ok(Instruction::Choose(Choose {
            line: self.line(node),
            whens,
            otherwise,
        }))
    }

    /// `xsl:value-of`.
    fn value_of(&mut self, node: Node<'_, 'input>) -> Load<Instruction> {
        let attributes = self.check_attributes(node, &["select", "separator"]);
        let select = self
            .xpath(node, "select", parse_expr)
            .map(|e| e.map(Box::new));
        let separator = self.xpath(node, "separator", parse_avt);
        let body = self.sequence_constructor(node);
        let (select, separator, body) = (select?, separator?, body?);
        attributes?;
        if select.is_some() && !body.is_empty() {
            return Err(self.invalid(
                node,
                "xsl:value-of has both a `select` attribute and content (XTSE0870)".into(),
            ));
        }
        Ok(Instruction::ValueOf(ValueOf {
            line: self.line(node),
            select: select.map(|e| *e),
            separator,
            body,
        }))
    }

    /// `xsl:attribute`.
    fn attribute(&mut self, node: Node<'_, 'input>) -> Load<Instruction> {
        let attributes = self.check_attributes(node, &["name", "select"]);
        let name = self
            .required(node, "name")
            .and_then(|_| self.xpath(node, "name", parse_avt)?.ok_or(Failed));
        let select = self
            .xpath(node, "select", parse_expr)
            .map(|e| e.map(Box::new));
        let body = self.sequence_constructor(node);
        let (name, select, body) = (name?, select?, body?);
        attributes?;
        if select.is_some() && !body.is_empty() {
            return Err(self.invalid(
                node,
                "xsl:attribute has both a `select` attribute and content (XTSE0840)".into(),
            ));
        }
        Ok(Instruction::Attribute(Attribute {
            line: self.line(node),
            name,
            namespaces: self.in_scope_namespaces(node, false),
            select: select.map(|e| *e),
            body,
        }))
    }

    /// `xsl:text`: text only.
    fn xsl_text(&mut self, node: Node<'_, 'input>) -> Load<Text> {
        self.check_attributes(node, &[])?;
        let mut value = String::new();
        for child in children(node) {
            match child {
                Child::Text { value: text, .. } => value.push_str(&text),
                Child::Element(e) => {
                    let message = format!(
                        "{} is not allowed in xsl:text (XTSE0010)",
                        self.element_name(e)
                    );
                    return Err(self.invalid(e, message));
                }
            }
        }
        Ok(Text {
            line: self.line(node),
            value,
            origin: TextOrigin::XslText,
        })
    }

    /// `xsl:number` with `level` (`single` or `multiple`) and `count`.
    fn number(&mut self, node: Node<'_, 'input>) -> Load<Instruction> {
        let attributes = self.check_attributes(node, &["level", "count"]);
        let level = match node
            .attribute("level")
            .map(|v| v.trim_matches(is_xml_whitespace))
        {
            None | Some("single") => Ok(NumberLevel::Single),
            Some("multiple") => Ok(NumberLevel::Multiple),
            Some(other) => Err(self.unsupported_value(node, "level", other)),
        };
        let count = self.xpath(node, "count", parse_pattern);
        let empty = self.empty_content(node);
        let (level, count, ()) = (level?, count?, empty?);
        attributes?;
        Ok(Instruction::Number(Number {
            line: self.line(node),
            level,
            count,
        }))
    }

    /// A literal result element: attributes are attribute value templates; attributes in
    /// the XSLT namespace (`xsl:use-attribute-sets`, `xsl:exclude-result-prefixes`, ...)
    /// are not supported.
    fn literal_element(&mut self, node: Node<'_, 'input>) -> Load<Instruction> {
        let mut failed = false;
        let mut attributes = Vec::new();
        for attribute in node.attributes() {
            let name = self.attribute_qname(&attribute);
            let line = self.line_at(attribute.range().start);
            if attribute.namespace() == Some(XSL) {
                let element = self.element_name(node);
                self.fail(
                    line,
                    LoadErrorKind::UnsupportedAttribute {
                        element,
                        attribute: name.to_string(),
                    },
                );
                failed = true;
                continue;
            }
            match parse_avt(attribute.value(), &InScope(node)) {
                Ok(value) => attributes.push(LiteralAttribute { name, value }),
                Err(error) => {
                    let element = self.element_name(node);
                    self.fail(
                        line,
                        LoadErrorKind::XPath {
                            element,
                            attribute: name.to_string(),
                            error,
                        },
                    );
                    failed = true;
                }
            }
        }
        let namespaces = self.in_scope_namespaces(node, true);
        let body = self.sequence_constructor(node);
        if failed {
            return Err(Failed);
        }
        Ok(Instruction::LiteralElement(LiteralElement {
            line: self.line(node),
            name: self.element_qname(node),
            attributes,
            namespaces,
            body: body?,
        }))
    }
}
