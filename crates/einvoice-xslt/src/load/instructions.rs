//! Sequence constructors: instructions, literal result elements and text.

use roxmltree::Node;

use super::declarations::BindingKind;
use super::strip::Child;
use super::{Failed, InScope, Load, Loader, XSL, is_xsl};
use crate::MAX_NESTING;
use crate::error::LoadErrorKind;
use crate::ir::{
    ApplyMode, ApplyTemplates, Attribute, CallTemplate, Choose, Comment, Copy, CopyOf, Element,
    ForEach, If, Instruction, LiteralAttribute, LiteralElement, Message, Mode, NamespaceBinding,
    NamespaceNode, NextMatch, Number, NumberLevel, ProcessingInstruction, Sequence, Text,
    TextOrigin, ValueOf, Variable, When,
};
use einvoice_xpath::{
    QName, XML_NAMESPACE, is_xml_whitespace, parse_avt, parse_expr, parse_pattern,
};

/// XSLT elements that are supported but not as instructions.
const DECLARATIONS: [&str; 10] = [
    "template",
    "function",
    "output",
    "param",
    "with-param",
    "when",
    "otherwise",
    "stylesheet",
    "import",
    "namespace-alias",
];

impl<'input> Loader<'_, 'input> {
    /// The content of `node` as a sequence constructor.
    pub(crate) fn sequence_constructor(
        &mut self,
        node: Node<'_, 'input>,
    ) -> Load<Vec<Instruction>> {
        self.instructions(self.children(node))
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
            "copy" => self.copy(node),
            "copy-of" => self.copy_of(node),
            "element" => self.element(node),
            "namespace" => self.namespace(node),
            "processing-instruction" => self.processing_instruction(node),
            "message" => self.message(node),
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
        let params = super::each(self.children(node), |child| match child {
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
        for child in self.children(node) {
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
        let attributes = self.check_attributes(node, &["name", "namespace", "select"]);
        let name = self
            .required(node, "name")
            .and_then(|_| self.xpath(node, "name", parse_avt)?.ok_or(Failed));
        let namespace = self.xpath(node, "namespace", parse_avt);
        let select = self
            .xpath(node, "select", parse_expr)
            .map(|e| e.map(Box::new));
        let body = self.sequence_constructor(node);
        let (name, namespace, select, body) = (name?, namespace?, select?, body?);
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
            namespace,
            namespaces: self.in_scope_namespaces(node, false),
            select: select.map(|e| *e),
            body,
        }))
    }

    /// `xsl:text`: text only.
    fn xsl_text(&mut self, node: Node<'_, 'input>) -> Load<Text> {
        self.check_attributes(node, &[])?;
        let mut value = String::new();
        for child in self.children(node) {
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
    /// are not supported, except `xsl:use-when`, already evaluated. Names in a literal
    /// namespace of `xsl:namespace-alias` are written in its target namespace, and the
    /// namespace nodes copied are those XSLT 2.0 sections 11.1.3 and 11.1.4 keep. An
    /// element in an extension namespace is an extension instruction, refused.
    fn literal_element(&mut self, node: Node<'_, 'input>) -> Load<Instruction> {
        let element_namespace = node.tag_name().namespace().unwrap_or("");
        if self
            .extension_namespaces
            .iter()
            .any(|uri| uri == element_namespace)
        {
            let message = format!(
                "extension instruction {} is not supported",
                self.element_name(node)
            );
            return Err(self.invalid(node, message));
        }
        let mut failed = false;
        let mut attributes = Vec::new();
        for attribute in node.attributes() {
            let name = self.attribute_qname(&attribute);
            let line = self.line_at(attribute.range().start);
            if attribute.namespace() == Some(XSL) {
                if attribute.name() == "use-when" {
                    continue;
                }
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
                Ok(value) => attributes.push(LiteralAttribute {
                    name: self.aliased(name),
                    value,
                }),
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
        let namespaces = self.literal_namespaces(node);
        let body = self.sequence_constructor(node);
        if failed {
            return Err(Failed);
        }
        let name = self.element_qname(node);
        Ok(Instruction::LiteralElement(LiteralElement {
            line: self.line(node),
            name: self.aliased(name),
            attributes,
            namespaces,
            body: body?,
        }))
    }

    /// A name of a literal result element or of one of its attributes as written to the
    /// result: in the target namespace with the result prefix when its namespace is the
    /// literal namespace of an alias (a name in no namespace is never aliased here: an
    /// alias of `#default` without a default namespace is not supported).
    fn aliased(&self, name: QName) -> QName {
        let namespace = name.namespace.as_deref().unwrap_or("");
        match self.module.aliases.iter().find(|a| a.literal == namespace) {
            Some(alias) if name.namespace.is_some() => QName {
                prefix: alias.prefix.clone(),
                namespace: (!alias.target.is_empty()).then(|| alias.target.clone()),
                local: name.local,
            },
            _ => name,
        }
    }

    /// The namespace nodes a literal result element copies to the result: those in scope
    /// on it except the `xml` namespace, the XSLT namespace, the extension namespaces of
    /// the module and the literal namespaces of the aliases; a namespace that is the
    /// target of an alias is copied whatever else it is.
    fn literal_namespaces(&mut self, node: Node<'_, 'input>) -> std::sync::Arc<[NamespaceBinding]> {
        let aliases = std::sync::Arc::clone(&self.module.aliases);
        let set: Vec<NamespaceBinding> = node
            .namespaces()
            .filter(|ns| ns.uri() != XML_NAMESPACE)
            .filter(|ns| {
                let uri = ns.uri();
                aliases.iter().any(|a| a.target == uri)
                    || !(uri == XSL
                        || self.extension_namespaces.iter().any(|e| e == uri)
                        || aliases.iter().any(|a| a.literal == uri))
            })
            .map(|ns| NamespaceBinding {
                prefix: ns.name().map(str::to_owned),
                uri: ns.uri().to_owned(),
            })
            .collect();
        self.share(set)
    }

    /// `xsl:copy` (no attributes supported).
    fn copy(&mut self, node: Node<'_, 'input>) -> Load<Instruction> {
        self.check_attributes(node, &[])?;
        Ok(Instruction::Copy(Copy {
            line: self.line(node),
            body: self.sequence_constructor(node)?,
        }))
    }

    /// `xsl:copy-of` with `select` only.
    fn copy_of(&mut self, node: Node<'_, 'input>) -> Load<Instruction> {
        let attributes = self.check_attributes(node, &["select"]);
        let select = self.required_xpath(node, "select", parse_expr);
        let empty = self.empty_content(node);
        let (select, ()) = (select?, empty?);
        attributes?;
        Ok(Instruction::CopyOf(CopyOf {
            line: self.line(node),
            select,
        }))
    }

    /// `xsl:element` with `name` and `namespace`.
    fn element(&mut self, node: Node<'_, 'input>) -> Load<Instruction> {
        let attributes = self.check_attributes(node, &["name", "namespace"]);
        let name = self
            .required(node, "name")
            .and_then(|_| self.xpath(node, "name", parse_avt)?.ok_or(Failed));
        let namespace = self.xpath(node, "namespace", parse_avt);
        let body = self.sequence_constructor(node);
        let (name, namespace, body) = (name?, namespace?, body?);
        attributes?;
        Ok(Instruction::Element(Element {
            line: self.line(node),
            name,
            namespace,
            namespaces: self.in_scope_namespaces(node, false),
            body,
        }))
    }

    /// `xsl:namespace` with `name` and `select` or content.
    fn namespace(&mut self, node: Node<'_, 'input>) -> Load<Instruction> {
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
                "xsl:namespace has both a `select` attribute and content (XTSE0910)".into(),
            ));
        }
        Ok(Instruction::Namespace(NamespaceNode {
            line: self.line(node),
            name,
            select: select.map(|e| *e),
            body,
        }))
    }

    /// `xsl:processing-instruction` with `name` and `select` or content.
    fn processing_instruction(&mut self, node: Node<'_, 'input>) -> Load<Instruction> {
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
                "xsl:processing-instruction has both a `select` attribute and content (XTSE0880)"
                    .into(),
            ));
        }
        Ok(Instruction::ProcessingInstruction(ProcessingInstruction {
            line: self.line(node),
            name,
            select: select.map(|e| *e),
            body,
        }))
    }

    /// `xsl:message` with `terminate` and content.
    fn message(&mut self, node: Node<'_, 'input>) -> Load<Instruction> {
        let attributes = self.check_attributes(node, &["terminate"]);
        let terminate = self.xpath(node, "terminate", parse_avt);
        let body = self.sequence_constructor(node);
        let (terminate, body) = (terminate?, body?);
        attributes?;
        Ok(Instruction::Message(Message {
            line: self.line(node),
            terminate,
            body,
        }))
    }
}
