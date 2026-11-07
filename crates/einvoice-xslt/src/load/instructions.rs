//! Sequence constructors: instructions, literal result elements and text.

use roxmltree::Node;

use super::declarations::BindingKind;
use super::strip::{Child, children};
use super::{Failed, InScope, Load, Loader, XSL, is_xsl};
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

    /// A list of stripped children as instructions.
    pub(crate) fn instructions(&mut self, items: Vec<Child<'_, 'input>>) -> Load<Vec<Instruction>> {
        super::each(items, |child| self.instruction(child))
    }

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
            return self.literal_element(node).map(Instruction::LiteralElement);
        }
        Ok(match node.tag_name().name() {
            "apply-templates" => Instruction::ApplyTemplates(self.apply_templates(node)?),
            "next-match" => {
                self.check_attributes(node, &[])?;
                Instruction::NextMatch(NextMatch {
                    line: self.line(node),
                    params: self.with_params(node)?,
                })
            }
            "call-template" => {
                let attributes = self.check_attributes(node, &["name"]);
                let name = self.required_qname(node, "name");
                let params = self.with_params(node);
                let (name, params) = (name?, params?);
                attributes?;
                Instruction::CallTemplate(CallTemplate {
                    line: self.line(node),
                    name,
                    params,
                })
            }
            "choose" => Instruction::Choose(self.choose(node)?),
            "if" => {
                let (test, body) = self.test_and_body(node)?;
                Instruction::If(If {
                    line: self.line(node),
                    test,
                    body,
                })
            }
            "for-each" => {
                let attributes = self.check_attributes(node, &["select"]);
                let select = self.required_xpath(node, "select", parse_expr);
                let body = self.sequence_constructor(node);
                let (select, body) = (select?, body?);
                attributes?;
                Instruction::ForEach(ForEach {
                    line: self.line(node),
                    select,
                    body,
                })
            }
            "variable" => Instruction::Variable(self.variable(node, BindingKind::Variable)?),
            "sequence" => {
                let attributes = self.check_attributes(node, &["select"]);
                let select = self.required_xpath(node, "select", parse_expr);
                let empty = self.empty_content(node);
                let (select, ()) = (select?, empty?);
                attributes?;
                Instruction::Sequence(Sequence {
                    line: self.line(node),
                    select,
                })
            }
            "value-of" => Instruction::ValueOf(self.value_of(node)?),
            "attribute" => Instruction::Attribute(self.attribute(node)?),
            "comment" => {
                self.check_attributes(node, &[])?;
                Instruction::Comment(Comment {
                    line: self.line(node),
                    body: self.sequence_constructor(node)?,
                })
            }
            "text" => Instruction::Text(self.xsl_text(node)?),
            "number" => Instruction::Number(self.number(node)?),
            name if DECLARATIONS.contains(&name) => {
                let message = format!("{} is not allowed here (XTSE0010)", self.element_name(node));
                return Err(self.invalid(node, message));
            }
            _ => return Err(self.unsupported_element(node)),
        })
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
    fn apply_templates(&mut self, node: Node<'_, 'input>) -> Load<ApplyTemplates> {
        let attributes = self.check_attributes(node, &["select", "mode"]);
        let select = self.xpath(node, "select", parse_expr);
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
        Ok(ApplyTemplates {
            line: self.line(node),
            select,
            mode,
            params,
        })
    }

    /// `test` and body of `xsl:if` or `xsl:when`.
    fn test_and_body(
        &mut self,
        node: Node<'_, 'input>,
    ) -> Load<(einvoice_xpath::Expr, Vec<Instruction>)> {
        let attributes = self.check_attributes(node, &["test"]);
        let test = self.required_xpath(node, "test", parse_expr);
        let body = self.sequence_constructor(node);
        let (test, body) = (test?, body?);
        attributes?;
        Ok((test, body))
    }

    /// `xsl:choose`: one or more `xsl:when`, then at most one `xsl:otherwise`.
    fn choose(&mut self, node: Node<'_, 'input>) -> Load<Choose> {
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
                            test,
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
        Ok(Choose {
            line: self.line(node),
            whens,
            otherwise,
        })
    }

    /// `xsl:value-of`.
    fn value_of(&mut self, node: Node<'_, 'input>) -> Load<ValueOf> {
        let attributes = self.check_attributes(node, &["select", "separator"]);
        let select = self.xpath(node, "select", parse_expr);
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
        Ok(ValueOf {
            line: self.line(node),
            select,
            separator,
            body,
        })
    }

    /// `xsl:attribute`.
    fn attribute(&mut self, node: Node<'_, 'input>) -> Load<Attribute> {
        let attributes = self.check_attributes(node, &["name", "select"]);
        let name = self
            .required(node, "name")
            .and_then(|_| self.xpath(node, "name", parse_avt)?.ok_or(Failed));
        let select = self.xpath(node, "select", parse_expr);
        let body = self.sequence_constructor(node);
        let (name, select, body) = (name?, select?, body?);
        attributes?;
        if select.is_some() && !body.is_empty() {
            return Err(self.invalid(
                node,
                "xsl:attribute has both a `select` attribute and content (XTSE0840)".into(),
            ));
        }
        Ok(Attribute {
            line: self.line(node),
            name,
            namespaces: self.in_scope_namespaces(node, false),
            select,
            body,
        })
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
    fn number(&mut self, node: Node<'_, 'input>) -> Load<Number> {
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
        Ok(Number {
            line: self.line(node),
            level,
            count,
        })
    }

    /// A literal result element: attributes are attribute value templates; attributes in
    /// the XSLT namespace (`xsl:use-attribute-sets`, `xsl:exclude-result-prefixes`, ...)
    /// are not supported.
    fn literal_element(&mut self, node: Node<'_, 'input>) -> Load<LiteralElement> {
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
        Ok(LiteralElement {
            line: self.line(node),
            name: self.element_qname(node),
            attributes,
            namespaces,
            body: body?,
        })
    }
}
