//! The loaded stylesheet.
//!
//! Every element of the stylesheet becomes one IR node that keeps its source line. XPath
//! attributes are parsed trees, `as` attributes parsed sequence types, attribute value
//! templates parsed templates; names are resolved [`QName`]s.

use std::collections::BTreeSet;
use std::sync::Arc;

use einvoice_xpath::{Avt, Expr, Pattern, PatternAlternative, QName, SequenceType};

use crate::priority::Priority;

/// A loaded stylesheet (`xsl:stylesheet` or `xsl:transform`).
#[derive(Debug, Clone, PartialEq)]
pub struct Stylesheet {
    /// The `version` attribute (always `2.0`: other versions are refused).
    pub version: String,
    /// The `xsl:output` declaration, if any.
    pub output: Option<Output>,
    /// Global `xsl:param` and `xsl:variable` declarations, in document order.
    pub globals: Vec<GlobalVariable>,
    /// Templates, in document order (the declaration order of conflict resolution).
    pub templates: Vec<Template>,
    /// `xsl:function` declarations, in document order.
    pub functions: Vec<Function>,
    /// Top-level elements outside the XSLT namespace: user-defined data, which XSLT
    /// ignores (XSLT 2.0 section 3.6.2). Kept so that nothing is dropped unseen.
    pub data_elements: Vec<DataElement>,
}

/// One template rule: an alternative of a template's pattern, in a given mode.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rule<'a> {
    /// Index of the template in [`Stylesheet::templates`].
    pub template: usize,
    /// Index of the alternative in the template's pattern (and in [`Template::rules`]).
    pub alternative: usize,
    /// The alternative.
    pub pattern: &'a PatternAlternative,
    /// Its priority.
    pub priority: Priority,
}

impl Stylesheet {
    /// The modes that templates name, plus the default mode when a template is in it
    /// (`#all` templates add none).
    #[must_use]
    pub fn modes(&self) -> BTreeSet<Mode> {
        self.templates
            .iter()
            .filter(|t| t.pattern.is_some())
            .flat_map(|t| match &t.modes {
                TemplateModes::All => Vec::new(),
                TemplateModes::Listed(modes) => modes.clone(),
            })
            .collect()
    }

    /// The rules that apply in `mode`: one per alternative of each matching template whose
    /// modes include `mode` (or that is in `#all` modes), in declaration order, template by
    /// template and alternative by alternative. Choosing among them (priority, then
    /// declaration order) is left to the evaluator.
    #[must_use]
    pub fn rules(&self, mode: &Mode) -> Vec<Rule<'_>> {
        let mut rules = Vec::new();
        for (index, template) in self.templates.iter().enumerate() {
            let Some(pattern) = &template.pattern else {
                continue;
            };
            let in_mode = match &template.modes {
                TemplateModes::All => true,
                TemplateModes::Listed(modes) => modes.contains(mode),
            };
            if !in_mode {
                continue;
            }
            for (rule, alternative) in template.rules.iter().zip(&pattern.alternatives) {
                rules.push(Rule {
                    template: index,
                    alternative: rule.alternative,
                    pattern: alternative,
                    priority: rule.priority,
                });
            }
        }
        rules
    }

    /// The template named `name`.
    #[must_use]
    pub fn named_template(&self, name: &QName) -> Option<&Template> {
        self.templates
            .iter()
            .find(|t| t.name.as_ref().is_some_and(|n| n.same_expanded_name(name)))
    }

    /// The function named `name` with `arity` parameters.
    #[must_use]
    pub fn function(&self, name: &QName, arity: usize) -> Option<&Function> {
        self.functions
            .iter()
            .find(|f| f.name.same_expanded_name(name) && f.params.len() == arity)
    }
}

/// `xsl:output`. Only the attributes the official stylesheets use are accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Output {
    /// Source line.
    pub line: u32,
    /// `method` (only `xml` is accepted).
    pub method: Option<OutputMethod>,
    /// `indent`
    pub indent: Option<bool>,
    /// `omit-xml-declaration`
    pub omit_xml_declaration: Option<bool>,
    /// `standalone`
    pub standalone: Option<Standalone>,
}

/// The serialization method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMethod {
    /// `xml`
    Xml,
}

/// The `standalone` serialization parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standalone {
    /// `yes`
    Yes,
    /// `no`
    No,
    /// `omit`
    Omit,
}

/// A global variable or parameter.
#[derive(Debug, Clone, PartialEq)]
pub struct GlobalVariable {
    /// `xsl:param` or `xsl:variable`.
    pub kind: GlobalKind,
    /// The declaration.
    pub variable: Variable,
}

/// Whether a global binding is a parameter (settable from outside) or a variable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlobalKind {
    /// `xsl:param`
    Param,
    /// `xsl:variable`
    Variable,
}

/// A variable binding: `xsl:variable`, `xsl:param` or `xsl:with-param`.
#[derive(Debug, Clone, PartialEq)]
pub struct Variable {
    /// Source line.
    pub line: u32,
    /// The name (no namespace when unprefixed).
    pub name: QName,
    /// The `as` sequence type.
    pub as_type: Option<SequenceType>,
    /// Where the value comes from.
    pub value: VariableValue,
}

/// The value of a variable binding (XSLT 2.0 section 9.3).
#[derive(Debug, Clone, PartialEq)]
pub enum VariableValue {
    /// `select="..."`
    Select(Expr),
    /// A non-empty sequence constructor: a temporary tree without `as`, a sequence with it.
    Content(Vec<Instruction>),
    /// Neither: the empty string without `as`, the empty sequence with it. For a
    /// parameter, no default value.
    Absent,
}

/// A mode a template can be in, or an `xsl:apply-templates` can select.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Mode {
    /// The unnamed mode (`#default`, or no `mode` attribute).
    Default,
    /// A named mode.
    Named(QName),
}

/// The modes of a template (its `mode` attribute).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplateModes {
    /// `#all`: every mode.
    All,
    /// The listed modes (at least one; `[Default]` without `mode` attribute).
    Listed(Vec<Mode>),
}

/// The mode of an `xsl:apply-templates`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyMode {
    /// A given mode (`#default` or a name).
    Mode(Mode),
    /// `#current`: the mode of the current template rule.
    Current,
}

/// `xsl:template`.
#[derive(Debug, Clone, PartialEq)]
pub struct Template {
    /// Source line.
    pub line: u32,
    /// The `name`, for templates called with `xsl:call-template`.
    pub name: Option<QName>,
    /// The `match` pattern.
    pub pattern: Option<Pattern>,
    /// The modes the template is in (meaningful only with a pattern).
    pub modes: TemplateModes,
    /// The explicit `priority`, if any.
    pub priority: Option<Priority>,
    /// One rule per alternative of the pattern, in order (empty without pattern): a pattern
    /// `A | B` is treated as two template rules sharing the body (XSLT 2.0 section 6.4).
    pub rules: Vec<TemplateRule>,
    /// The `xsl:param` declarations.
    pub params: Vec<Variable>,
    /// The body.
    pub body: Vec<Instruction>,
}

/// The priority of one alternative of a template's pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TemplateRule {
    /// Index of the alternative in the pattern.
    pub alternative: usize,
    /// The explicit priority of the template, or the default priority of the alternative.
    pub priority: Priority,
    /// Whether `priority` is the default priority (no `priority` attribute).
    pub is_default: bool,
}

/// `xsl:function`.
#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    /// Source line.
    pub line: u32,
    /// The name (always prefixed, so never in no namespace).
    pub name: QName,
    /// The parameters, in order (their value is always [`VariableValue::Absent`]).
    pub params: Vec<Variable>,
    /// The `as` result type.
    pub as_type: Option<SequenceType>,
    /// The body.
    pub body: Vec<Instruction>,
}

/// A top-level element outside the XSLT namespace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataElement {
    /// Source line.
    pub line: u32,
    /// The element name.
    pub name: QName,
}

/// One instruction of a sequence constructor.
#[derive(Debug, Clone, PartialEq)]
pub enum Instruction {
    /// `xsl:apply-templates`
    ApplyTemplates(ApplyTemplates),
    /// `xsl:next-match`
    NextMatch(NextMatch),
    /// `xsl:call-template`
    CallTemplate(CallTemplate),
    /// `xsl:choose`
    Choose(Choose),
    /// `xsl:if`
    If(If),
    /// `xsl:for-each`
    ForEach(ForEach),
    /// A local `xsl:variable`.
    Variable(Variable),
    /// `xsl:sequence`
    Sequence(Sequence),
    /// `xsl:value-of`
    ValueOf(ValueOf),
    /// `xsl:attribute`
    Attribute(Attribute),
    /// `xsl:comment`
    Comment(Comment),
    /// `xsl:text`, or a text node of the stylesheet that survived whitespace stripping.
    Text(Text),
    /// `xsl:number`
    Number(Number),
    /// A literal result element.
    LiteralElement(LiteralElement),
}

/// `xsl:apply-templates`.
#[derive(Debug, Clone, PartialEq)]
pub struct ApplyTemplates {
    /// Source line.
    pub line: u32,
    /// `select`; `None` stands for the default, `child::node()`.
    pub select: Option<Expr>,
    /// `mode`; [`Mode::Default`] without the attribute.
    pub mode: ApplyMode,
    /// `xsl:with-param` children.
    pub params: Vec<Variable>,
}

/// `xsl:next-match`.
#[derive(Debug, Clone, PartialEq)]
pub struct NextMatch {
    /// Source line.
    pub line: u32,
    /// `xsl:with-param` children.
    pub params: Vec<Variable>,
}

/// `xsl:call-template`.
#[derive(Debug, Clone, PartialEq)]
pub struct CallTemplate {
    /// Source line.
    pub line: u32,
    /// The called template.
    pub name: QName,
    /// `xsl:with-param` children.
    pub params: Vec<Variable>,
}

/// `xsl:choose`.
#[derive(Debug, Clone, PartialEq)]
pub struct Choose {
    /// Source line.
    pub line: u32,
    /// The `xsl:when` branches, in order (at least one).
    pub whens: Vec<When>,
    /// The `xsl:otherwise` body, if present.
    pub otherwise: Option<Vec<Instruction>>,
}

/// `xsl:when`.
#[derive(Debug, Clone, PartialEq)]
pub struct When {
    /// Source line.
    pub line: u32,
    /// `test`
    pub test: Expr,
    /// The body.
    pub body: Vec<Instruction>,
}

/// `xsl:if`.
#[derive(Debug, Clone, PartialEq)]
pub struct If {
    /// Source line.
    pub line: u32,
    /// `test`
    pub test: Expr,
    /// The body.
    pub body: Vec<Instruction>,
}

/// `xsl:for-each`.
#[derive(Debug, Clone, PartialEq)]
pub struct ForEach {
    /// Source line.
    pub line: u32,
    /// `select`
    pub select: Expr,
    /// The body.
    pub body: Vec<Instruction>,
}

/// `xsl:sequence`.
#[derive(Debug, Clone, PartialEq)]
pub struct Sequence {
    /// Source line.
    pub line: u32,
    /// `select`
    pub select: Expr,
}

/// `xsl:value-of`.
#[derive(Debug, Clone, PartialEq)]
pub struct ValueOf {
    /// Source line.
    pub line: u32,
    /// `select`; without it, the value comes from the body.
    pub select: Option<Expr>,
    /// `separator`; `None` means the default (a space with `select`, nothing without).
    pub separator: Option<Avt>,
    /// The body (empty when `select` is present).
    pub body: Vec<Instruction>,
}

/// `xsl:attribute`.
#[derive(Debug, Clone, PartialEq)]
pub struct Attribute {
    /// Source line.
    pub line: u32,
    /// `name`
    pub name: Avt,
    /// The namespaces in scope on the element, which resolve the prefix of the computed
    /// name (the implicit `xml` namespace excepted). Equal sets are shared.
    pub namespaces: Arc<[NamespaceBinding]>,
    /// `select`; without it, the value comes from the body.
    pub select: Option<Expr>,
    /// The body (empty when `select` is present).
    pub body: Vec<Instruction>,
}

/// `xsl:comment`.
#[derive(Debug, Clone, PartialEq)]
pub struct Comment {
    /// Source line.
    pub line: u32,
    /// The body.
    pub body: Vec<Instruction>,
}

/// Literal text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Text {
    /// Source line.
    pub line: u32,
    /// The text (possibly empty for `<xsl:text/>`).
    pub value: String,
    /// Where it comes from.
    pub origin: TextOrigin,
}

/// Where a [`Text`] comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextOrigin {
    /// An `xsl:text` element.
    XslText,
    /// A text node of a sequence constructor.
    Literal,
}

/// `xsl:number` (default format `1`).
#[derive(Debug, Clone, PartialEq)]
pub struct Number {
    /// Source line.
    pub line: u32,
    /// `level`
    pub level: NumberLevel,
    /// `count`; `None` counts nodes of the same kind and name as the context node.
    pub count: Option<Pattern>,
}

/// The `level` of `xsl:number`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumberLevel {
    /// `single` (the default)
    Single,
    /// `multiple`
    Multiple,
}

/// A literal result element.
#[derive(Debug, Clone, PartialEq)]
pub struct LiteralElement {
    /// Source line.
    pub line: u32,
    /// The element name.
    pub name: QName,
    /// The attributes, in document order, as attribute value templates.
    pub attributes: Vec<LiteralAttribute>,
    /// The namespace nodes copied to the result (XSLT 2.0 section 11.1.3): every namespace
    /// in scope on the element except the XSLT namespace and the implicit `xml` namespace.
    /// Equal sets are shared.
    pub namespaces: Arc<[NamespaceBinding]>,
    /// The body.
    pub body: Vec<Instruction>,
}

/// An attribute of a literal result element.
#[derive(Debug, Clone, PartialEq)]
pub struct LiteralAttribute {
    /// The attribute name.
    pub name: QName,
    /// The value.
    pub value: Avt,
}

/// A namespace binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamespaceBinding {
    /// The prefix; `None` for the default namespace.
    pub prefix: Option<String>,
    /// The namespace URI.
    pub uri: String,
}
