//! The stylesheet element and the declarations under it.

use std::collections::BTreeSet;

use roxmltree::Node;

use super::strip::{Child, children};
use super::{Failed, Load, Loader, is_xsl};
use crate::error::LoadErrorKind;
use crate::ir::{
    DataElement, Function, GlobalKind, GlobalVariable, Mode, Output, OutputMethod, Standalone,
    Stylesheet, Template, TemplateModes, TemplateRule, Variable, VariableValue,
};
use crate::priority::{Priority, default_priority};
use einvoice_xpath::{is_xml_whitespace, parse_expr, parse_pattern, parse_sequence_type};

/// What a variable-binding element may hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BindingKind {
    /// `xsl:variable`: `select` or content.
    Variable,
    /// `xsl:param` of a stylesheet or template, `xsl:with-param`: `select` or content.
    Param,
    /// `xsl:param` of a function: neither (XTSE0760).
    FunctionParam,
}

impl<'input> Loader<'_, 'input> {
    /// The document element: `xsl:stylesheet` or `xsl:transform`, version 2.0.
    pub(super) fn stylesheet(&mut self, root: Node<'_, 'input>) -> Load<Stylesheet> {
        if !is_xsl(root) || !matches!(root.tag_name().name(), "stylesheet" | "transform") {
            let name = self.element_name(root);
            return Err(self.invalid(
                root,
                format!("the document element is {name}, not xsl:stylesheet or xsl:transform (simplified stylesheets are not supported)"),
            ));
        }
        let attributes = self.check_attributes(root, &["version"]);
        let version = self
            .required(root, "version")?
            .trim_matches(is_xml_whitespace)
            .to_owned();
        if version != "2.0" {
            return Err(self.unsupported_value(root, "version", &version));
        }
        let mut stylesheet = Stylesheet {
            version,
            output: None,
            globals: Vec::new(),
            templates: Vec::new(),
            functions: Vec::new(),
            data_elements: Vec::new(),
        };
        let declarations = super::each(children(root), |child| {
            self.declaration(&child, &mut stylesheet)
        });
        attributes?;
        declarations?;
        self.check_duplicates(&stylesheet)?;
        Ok(stylesheet)
    }

    /// One child of the stylesheet element.
    fn declaration(&mut self, child: &Child<'_, 'input>, stylesheet: &mut Stylesheet) -> Load<()> {
        let node = match child {
            Child::Element(node) => *node,
            Child::Text { offset, .. } => {
                let line = self.line_at(*offset);
                return Err(self.fail(
                    line,
                    LoadErrorKind::Invalid(
                        "text is not allowed at the top level of a stylesheet (XTSE0120)".into(),
                    ),
                ));
            }
        };
        if !is_xsl(node) {
            if node.tag_name().namespace().is_none() {
                let message = format!(
                    "top-level element {} must be in a namespace (XTSE0130)",
                    self.element_name(node)
                );
                return Err(self.invalid(node, message));
            }
            let (line, name) = (self.line(node), self.element_qname(node));
            stylesheet.data_elements.push(DataElement { line, name });
            return Ok(());
        }
        match node.tag_name().name() {
            "template" => stylesheet.templates.push(self.template(node)?),
            "function" => stylesheet.functions.push(self.function(node)?),
            "param" => stylesheet.globals.push(GlobalVariable {
                kind: GlobalKind::Param,
                variable: self.variable(node, BindingKind::Param)?,
            }),
            "variable" => stylesheet.globals.push(GlobalVariable {
                kind: GlobalKind::Variable,
                variable: self.variable(node, BindingKind::Variable)?,
            }),
            "output" => {
                let output = self.output(node)?;
                if stylesheet.output.is_some() {
                    return Err(
                        self.invalid(node, "more than one xsl:output is not supported".into())
                    );
                }
                stylesheet.output = Some(output);
            }
            _ => return Err(self.unsupported_element(node)),
        }
        Ok(())
    }

    pub(crate) fn unsupported_element(&mut self, node: Node<'_, '_>) -> Failed {
        let (line, element) = (self.line(node), self.element_name(node));
        self.fail(line, LoadErrorKind::UnsupportedElement { element })
    }

    /// Duplicate global variables (XTSE0630), named templates (XTSE0660) and functions
    /// (XTSE0770).
    fn check_duplicates(&mut self, stylesheet: &Stylesheet) -> Load<()> {
        let mut failed = false;
        let mut seen = BTreeSet::new();
        for global in &stylesheet.globals {
            let v = &global.variable;
            if !seen.insert((v.name.namespace.clone(), v.name.local.clone())) {
                failed = true;
                self.fail(
                    v.line,
                    LoadErrorKind::Invalid(format!(
                        "global variable `{}` is declared twice (XTSE0630)",
                        v.name
                    )),
                );
            }
        }
        let mut seen = BTreeSet::new();
        for template in &stylesheet.templates {
            if let Some(name) = &template.name
                && !seen.insert((name.namespace.clone(), name.local.clone()))
            {
                failed = true;
                self.fail(
                    template.line,
                    LoadErrorKind::Invalid(format!(
                        "template `{name}` is declared twice (XTSE0660)"
                    )),
                );
            }
        }
        let mut seen = BTreeSet::new();
        for function in &stylesheet.functions {
            let key = (
                function.name.namespace.clone(),
                function.name.local.clone(),
                function.params.len(),
            );
            if !seen.insert(key) {
                failed = true;
                self.fail(
                    function.line,
                    LoadErrorKind::Invalid(format!(
                        "function `{}` with {} parameters is declared twice (XTSE0770)",
                        function.name,
                        function.params.len()
                    )),
                );
            }
        }
        if failed { Err(Failed) } else { Ok(()) }
    }

    /// `xsl:output`.
    fn output(&mut self, node: Node<'_, 'input>) -> Load<Output> {
        self.check_attributes(
            node,
            &["indent", "method", "omit-xml-declaration", "standalone"],
        )?;
        self.empty_content(node)?;
        let yes_no = |loader: &mut Self, name: &str| -> Load<Option<bool>> {
            match node
                .attribute(name)
                .map(|v| v.trim_matches(is_xml_whitespace))
            {
                None => Ok(None),
                Some("yes") => Ok(Some(true)),
                Some("no") => Ok(Some(false)),
                Some(other) => Err(loader.unsupported_value(node, name, other)),
            }
        };
        let indent = yes_no(self, "indent")?;
        let omit_xml_declaration = yes_no(self, "omit-xml-declaration")?;
        let method = match node
            .attribute("method")
            .map(|v| v.trim_matches(is_xml_whitespace))
        {
            None => None,
            Some("xml") => Some(OutputMethod::Xml),
            Some(other) => return Err(self.unsupported_value(node, "method", other)),
        };
        let standalone = match node
            .attribute("standalone")
            .map(|v| v.trim_matches(is_xml_whitespace))
        {
            None => None,
            Some("yes") => Some(Standalone::Yes),
            Some("no") => Some(Standalone::No),
            Some("omit") => Some(Standalone::Omit),
            Some(other) => return Err(self.unsupported_value(node, "standalone", other)),
        };
        Ok(Output {
            line: self.line(node),
            method,
            indent,
            omit_xml_declaration,
            standalone,
        })
    }

    /// Fails unless `node` has no content (after whitespace stripping).
    pub(crate) fn empty_content(&mut self, node: Node<'_, 'input>) -> Load<()> {
        if children(node).is_empty() {
            Ok(())
        } else {
            let message = format!("{} must be empty (XTSE0260)", self.element_name(node));
            Err(self.invalid(node, message))
        }
    }

    /// `xsl:variable`, `xsl:param` or `xsl:with-param`.
    pub(crate) fn variable(&mut self, node: Node<'_, 'input>, kind: BindingKind) -> Load<Variable> {
        let allowed: &[&str] = match kind {
            BindingKind::FunctionParam => &["name", "as"],
            BindingKind::Variable | BindingKind::Param => &["name", "select", "as"],
        };
        let attributes = self.check_attributes(node, allowed);
        let name = self.required_qname(node, "name").map(Box::new);
        let as_type = self
            .xpath(node, "as", parse_sequence_type)
            .map(|t| t.map(Box::new));
        let select = self
            .xpath(node, "select", parse_expr)
            .map(|e| e.map(Box::new));
        let content = self.sequence_constructor(node);
        let (name, as_type, select, content) = (name?, as_type?, select?, content?);
        attributes?;
        let (name, as_type, select) = (*name, as_type.map(|t| *t), select.map(|e| *e));
        let value = match (select, content.is_empty()) {
            (Some(_), false) => {
                let message = format!(
                    "{} has both a `select` attribute and content (XTSE0620)",
                    self.element_name(node)
                );
                return Err(self.invalid(node, message));
            }
            (Some(select), true) => VariableValue::Select(select),
            (None, false) if kind == BindingKind::FunctionParam => {
                return Err(self.invalid(
                    node,
                    "a function parameter cannot have a default value (XTSE0760)".into(),
                ));
            }
            (None, false) => VariableValue::Content(content),
            (None, true) => VariableValue::Absent,
        };
        Ok(Variable {
            line: self.line(node),
            name,
            as_type,
            value,
        })
    }

    /// Leading `xsl:param` children, then the sequence constructor (templates, functions).
    fn params_and_body(
        &mut self,
        node: Node<'_, 'input>,
        kind: BindingKind,
    ) -> Load<(Vec<Variable>, Vec<crate::ir::Instruction>)> {
        let all = children(node);
        let split = all
            .iter()
            .position(|c| !matches!(c, Child::Element(e) if e.has_tag_name((super::XSL, "param"))))
            .unwrap_or(all.len());
        let (param_children, rest) = all.split_at(split);
        let params = super::each(param_children.iter(), |child| match child {
            Child::Element(e) => self.variable(*e, kind),
            Child::Text { .. } => Err(Failed),
        });
        let body = self.instructions(rest.to_vec());
        let (params, body) = (params?, body?);
        let mut seen = BTreeSet::new();
        for param in &params {
            if !seen.insert((param.name.namespace.clone(), param.name.local.clone())) {
                let message = format!("parameter `{}` is declared twice (XTSE0580)", param.name);
                return Err(self.fail(param.line, LoadErrorKind::Invalid(message)));
            }
        }
        Ok((params, body))
    }

    /// `xsl:template`.
    fn template(&mut self, node: Node<'_, 'input>) -> Load<Template> {
        let attributes = self.check_attributes(node, &["match", "mode", "priority", "name"]);
        let pattern = self.xpath(node, "match", parse_pattern);
        let name = node
            .attribute("name")
            .map(|v| self.qname_value(node, "name", v))
            .transpose();
        let modes = self.template_modes(node);
        let priority = match node.attribute("priority") {
            None => Ok(None),
            Some(text) => Priority::parse(text).map(Some).ok_or_else(|| {
                self.invalid(
                    node,
                    format!("priority `{text}` is not a decimal number (XTSE0530)"),
                )
            }),
        };
        let content = self.params_and_body(node, BindingKind::Param);
        let (pattern, name, modes, priority, (params, body)) =
            (pattern?, name?, modes?, priority?, content?);
        attributes?;
        if pattern.is_none() {
            if name.is_none() {
                return Err(self.invalid(
                    node,
                    "xsl:template needs a `match` or a `name` attribute (XTSE0500)".into(),
                ));
            }
            if node.has_attribute("mode") || node.has_attribute("priority") {
                return Err(self.invalid(
                    node,
                    "`mode` and `priority` need a `match` attribute (XTSE0500)".into(),
                ));
            }
        }
        let rules = pattern.as_ref().map_or_else(Vec::new, |pattern| {
            pattern
                .alternatives
                .iter()
                .enumerate()
                .map(|(alternative, a)| TemplateRule {
                    alternative,
                    priority: priority.unwrap_or_else(|| default_priority(a)),
                    is_default: priority.is_none(),
                })
                .collect()
        });
        Ok(Template {
            line: self.line(node),
            name,
            pattern,
            modes,
            priority,
            rules,
            params,
            body,
        })
    }

    /// The `mode` attribute of `xsl:template`: `#all`, or a list of `#default` and names.
    fn template_modes(&mut self, node: Node<'_, 'input>) -> Load<TemplateModes> {
        let Some(value) = node.attribute("mode") else {
            return Ok(TemplateModes::Listed(vec![Mode::Default]));
        };
        let tokens: Vec<&str> = value
            .split(is_xml_whitespace)
            .filter(|t| !t.is_empty())
            .collect();
        if tokens == ["#all"] {
            return Ok(TemplateModes::All);
        }
        if tokens.is_empty() || tokens.contains(&"#all") {
            return Err(self.invalid(node, format!("invalid mode list `{value}` (XTSE0550)")));
        }
        let mut modes = Vec::new();
        for token in tokens {
            let mode = match token {
                "#default" => Mode::Default,
                _ if token.starts_with('#') => {
                    return Err(self.unsupported_value(node, "mode", token));
                }
                _ => Mode::Named(self.qname_value(node, "mode", token)?),
            };
            if modes.contains(&mode) {
                return Err(
                    self.invalid(node, format!("mode `{token}` is listed twice (XTSE0550)"))
                );
            }
            modes.push(mode);
        }
        Ok(TemplateModes::Listed(modes))
    }

    /// `xsl:function`.
    fn function(&mut self, node: Node<'_, 'input>) -> Load<Function> {
        let attributes = self.check_attributes(node, &["name", "as"]);
        let name = self.required_qname(node, "name");
        let as_type = self.xpath(node, "as", parse_sequence_type);
        let content = self.params_and_body(node, BindingKind::FunctionParam);
        let (name, as_type, (params, body)) = (name?, as_type?, content?);
        attributes?;
        if name.prefix.is_none() {
            return Err(self.invalid(
                node,
                format!("function name `{name}` must have a prefix (XTSE0740)"),
            ));
        }
        Ok(Function {
            line: self.line(node),
            name,
            params,
            as_type,
            body,
        })
    }
}
