//! The modules of a stylesheet (XSLT 2.0 section 3.10): the principal module and the
//! modules it imports, read before any is walked, their import precedences, the
//! namespace aliases they declare, and the merge of their declarations.
//!
//! `xsl:include` is not supported, nor is importing the same module twice.

use std::collections::BTreeMap;

use roxmltree::Document;

use super::{Alias, XSL, is_xsl, static_eval};
use crate::ModuleResolver;
use crate::error::{LoadError, LoadErrorKind};
use crate::ir::{GlobalVariable, Module, Stylesheet};

/// The most modules a stylesheet may have.
pub(crate) const MAX_MODULES: usize = 64;

/// A module read: its URI, its text, the modules it imports (indices, in order) and
/// its import precedence.
#[derive(Debug)]
pub(crate) struct Source {
    pub(crate) uri: Option<String>,
    pub(crate) text: String,
    pub(crate) imports: Vec<usize>,
    pub(crate) precedence: u32,
}

/// The 1-based line of a byte offset of `text`.
fn line_of(text: &str, offset: usize) -> u32 {
    let line = 1 + text
        .get(..offset)
        .map_or(0, |before| before.matches('\n').count());
    u32::try_from(line).unwrap_or(u32::MAX)
}

/// Resolves a URI reference against a base URI (RFC 3986 section 5.2, without the query
/// and fragment cases): an absolute reference is kept, a relative one joined to the
/// directory of the base and its `.` and `..` segments removed.
#[must_use]
pub fn resolve_uri(reference: &str, base: Option<&str>) -> String {
    let has_scheme = reference.split_once(':').is_some_and(|(scheme, _)| {
        let mut chars = scheme.chars();
        chars.next().is_some_and(|c| c.is_ascii_alphabetic())
            && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    });
    let Some(base) = base.filter(|_| !has_scheme) else {
        return reference.to_owned();
    };
    if reference.is_empty() {
        return base.to_owned();
    }
    let joined = if reference.starts_with('/') {
        match base.split_once(':') {
            Some((scheme, _)) => format!("{scheme}:{reference}"),
            None => reference.to_owned(),
        }
    } else {
        let directory = base.rfind('/').map_or("", |i| &base[..=i]);
        format!("{directory}{reference}")
    };
    let (prefix, path) = joined.split_once(':').unwrap_or(("", &joined));
    let mut segments: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "." => {}
            ".." => {
                if segments.len() > 1 {
                    segments.pop();
                }
            }
            s => segments.push(s),
        }
    }
    let path = segments.join("/");
    if prefix.is_empty() {
        path
    } else {
        format!("{prefix}:{path}")
    }
}

/// The `xsl:import` children of a module's stylesheet element that `use-when` keeps:
/// their `href` and the offset of the element.
fn imports_of<'a>(doc: &'a Document<'_>) -> Vec<(Option<&'a str>, usize)> {
    let root = doc.root_element();
    if !is_xsl(root) {
        return Vec::new();
    }
    root.children()
        .filter(|n| n.has_tag_name((XSL, "import")))
        .filter(|n| {
            n.attribute("use-when")
                .is_none_or(|expr| static_eval::use_when(expr, *n).unwrap_or(true))
        })
        .map(|n| (n.attribute("href"), n.range().start))
        .collect()
}

/// The module an `xsl:import` of the module `index` names, read through `resolver`, or
/// what is wrong: no `href`, no resolver, a module importing itself (`chain` holds the
/// modules that import `index`, directly or not), a module imported twice, too many
/// modules, a module that cannot be read.
fn imported(
    sources: &[Source],
    index: usize,
    chain: &[usize],
    href: Option<&str>,
    resolver: Option<&dyn ModuleResolver>,
) -> Result<Source, LoadErrorKind> {
    let Some(href) = href else {
        return Err(LoadErrorKind::MissingAttribute {
            element: "xsl:import".to_owned(),
            attribute: "href".to_owned(),
        });
    };
    // Without a resolver (`load`), xsl:import is outside the subset.
    let Some(resolver) = resolver else {
        return Err(LoadErrorKind::UnsupportedElement {
            element: "xsl:import".to_owned(),
        });
    };
    let absolute = resolve_uri(href, sources[index].uri.as_deref());
    let is = |i: &usize| sources[*i].uri.as_deref() == Some(absolute.as_str());
    if chain.iter().chain(std::iter::once(&index)).any(is) {
        return Err(LoadErrorKind::Invalid(format!(
            "`{absolute}` imports itself (XTSE0210)"
        )));
    }
    if (0..sources.len()).any(|i| is(&i)) {
        return Err(LoadErrorKind::Invalid(format!(
            "`{absolute}` is imported twice, which is not supported"
        )));
    }
    if sources.len() >= MAX_MODULES {
        return Err(LoadErrorKind::Invalid(format!(
            "more than {MAX_MODULES} stylesheet modules"
        )));
    }
    let text = resolver.resolve(&absolute).ok_or_else(|| {
        LoadErrorKind::Invalid(format!("xsl:import: cannot read `{absolute}` (XTSE0165)"))
    })?;
    Ok(Source {
        uri: Some(absolute),
        text,
        imports: Vec::new(),
        precedence: 0,
    })
}

/// Reads the principal module and, through `resolver`, the modules it imports, depth
/// first, and gives each its import precedence ([`number_precedences`]). Every error
/// found is returned.
pub(crate) fn discover(
    text: &str,
    uri: Option<&str>,
    resolver: Option<&dyn ModuleResolver>,
) -> Result<Vec<Source>, Vec<LoadError>> {
    let mut sources = vec![Source {
        uri: uri.map(str::to_owned),
        text: text.to_owned(),
        imports: Vec::new(),
        precedence: 0,
    }];
    let mut errors = Vec::new();
    // Depth first: the modules to read the imports of, with their chain of importers.
    let mut pending: Vec<(usize, Vec<usize>)> = vec![(0, Vec::new())];
    while let Some((index, chain)) = pending.pop() {
        let module_uri = sources[index].uri.clone();
        let text = sources[index].text.clone();
        let doc = match Document::parse(&text) {
            Ok(doc) => doc,
            Err(e) => {
                errors.push(LoadError {
                    module: module_uri,
                    line: e.pos().row,
                    kind: LoadErrorKind::Xml(e.to_string()),
                });
                continue;
            }
        };
        let mut children = Vec::new();
        for (href, offset) in imports_of(&doc) {
            match imported(&sources, index, &chain, href, resolver) {
                Ok(source) => {
                    sources.push(source);
                    children.push(sources.len() - 1);
                }
                Err(kind) => errors.push(LoadError {
                    module: module_uri.clone(),
                    line: line_of(&text, offset),
                    kind,
                }),
            }
        }
        let mut child_chain = chain;
        child_chain.push(index);
        // Depth first in import order: the first import is read next.
        for &child in children.iter().rev() {
            pending.push((child, child_chain.clone()));
        }
        sources[index].imports = children;
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    number_precedences(&mut sources);
    Ok(sources)
}

/// The import precedences: the modules numbered from 0 in post-order of the import tree
/// (a module after the modules it imports, the modules a module imports in the order it
/// imports them), so that the principal module has the highest.
fn number_precedences(sources: &mut [Source]) {
    let mut order = Vec::with_capacity(sources.len());
    let mut stack: Vec<(usize, usize)> = vec![(0, 0)];
    while let Some((index, next)) = stack.pop() {
        if let Some(&child) = sources[index].imports.get(next) {
            stack.push((index, next + 1));
            stack.push((child, 0));
        } else {
            order.push(index);
        }
    }
    for (precedence, index) in order.into_iter().enumerate() {
        sources[index].precedence = u32::try_from(precedence).unwrap_or(u32::MAX);
    }
}

/// The namespace aliases the modules declare (top-level `xsl:namespace-alias`), the one
/// of the highest import precedence for each literal namespace; two of the same
/// precedence for one literal namespace with different targets are an error
/// (XTSE0810), as are undeclared prefixes (XTSE0812).
pub(crate) fn aliases(
    sources: &[Source],
    documents: &[Document<'_>],
) -> (Vec<Alias>, Vec<LoadError>) {
    let mut errors = Vec::new();
    // By literal namespace: the alias, its precedence.
    let mut chosen: BTreeMap<String, (Alias, u32)> = BTreeMap::new();
    for (source, doc) in sources.iter().zip(documents) {
        let root = doc.root_element();
        if !is_xsl(root) {
            continue;
        }
        for node in root
            .children()
            .filter(|n| n.has_tag_name((XSL, "namespace-alias")))
        {
            if node
                .attribute("use-when")
                .is_some_and(|expr| !static_eval::use_when(expr, node).unwrap_or(true))
            {
                continue;
            }
            let line = line_of(&source.text, node.range().start);
            let mut error = |message: String| {
                errors.push(LoadError {
                    module: source.uri.clone(),
                    line,
                    kind: LoadErrorKind::Invalid(message),
                });
            };
            let resolve = |attribute: &str| -> Result<(Option<String>, String), String> {
                let Some(value) = node.attribute(attribute) else {
                    return Err(format!(
                        "xsl:namespace-alias needs a `{attribute}` attribute"
                    ));
                };
                let value = value.trim_matches(einvoice_xpath::is_xml_whitespace);
                if value == "#default" {
                    let uri = node.lookup_namespace_uri(None).unwrap_or("");
                    return Ok((None, uri.to_owned()));
                }
                match node.lookup_namespace_uri(Some(value)) {
                    Some(uri) if einvoice_xpath::is_ncname(value) => {
                        Ok((Some(value.to_owned()), uri.to_owned()))
                    }
                    _ => Err(format!(
                        "prefix `{value}` of xsl:namespace-alias/@{attribute} is not declared (XTSE0812)"
                    )),
                }
            };
            let (literal, target) = match (resolve("stylesheet-prefix"), resolve("result-prefix")) {
                (Ok((_, literal)), Ok(target)) => (literal, target),
                (Err(e), _) | (_, Err(e)) => {
                    error(e);
                    continue;
                }
            };
            let alias = Alias {
                literal: literal.clone(),
                target: target.1,
                prefix: target.0,
            };
            match chosen.get(&literal) {
                Some((existing, precedence)) if *precedence == source.precedence => {
                    if existing.target != alias.target {
                        error(format!(
                            "two xsl:namespace-alias of the same precedence for `{literal}` (XTSE0810)"
                        ));
                    }
                }
                Some((_, precedence)) if *precedence > source.precedence => {}
                _ => {
                    chosen.insert(literal, (alias, source.precedence));
                }
            }
        }
    }
    (chosen.into_values().map(|(a, _)| a).collect(), errors)
}

/// Merges the declarations of the modules, `parts` in the order of `sources`: the
/// templates in increasing import precedence (in declaration order within a module); of
/// the global variables, functions and named templates of one name, the one of the
/// highest precedence (two of the same precedence were refused in their module);
/// at most one `xsl:output` in the whole stylesheet.
pub(crate) fn merge(
    sources: &[Source],
    parts: Vec<Stylesheet>,
) -> Result<Stylesheet, Vec<LoadError>> {
    let mut by_precedence: Vec<(u32, Option<String>, Stylesheet)> = sources
        .iter()
        .map(|s| (s.precedence, s.uri.clone()))
        .zip(parts)
        .map(|((p, uri), part)| (p, uri, part))
        .collect();
    by_precedence.sort_by_key(|(p, _, _)| *p);
    let modules: Vec<Module> = sources
        .iter()
        .map(|s| Module {
            uri: s.uri.clone(),
            precedence: s.precedence,
        })
        .collect();
    let mut merged = Stylesheet {
        version: "2.0".to_owned(),
        output: None,
        globals: Vec::new(),
        templates: Vec::new(),
        functions: Vec::new(),
        data_elements: Vec::new(),
        modules,
    };
    let mut errors = Vec::new();
    for (_, uri, part) in by_precedence.into_iter().rev() {
        // From the highest precedence down: what a module of higher precedence declares
        // overrides the same name in the modules it imports.
        if let Some(output) = part.output {
            if merged.output.is_some() {
                errors.push(LoadError {
                    module: uri,
                    line: output.line,
                    kind: LoadErrorKind::Invalid(
                        "xsl:output in more than one stylesheet module is not supported".into(),
                    ),
                });
            } else {
                merged.output = Some(output);
            }
        }
        for global in part.globals {
            let overridden = merged.globals.iter().any(|g: &GlobalVariable| {
                g.variable.name.same_expanded_name(&global.variable.name)
            });
            if !overridden {
                merged.globals.push(global);
            }
        }
        for function in part.functions {
            let overridden = merged.functions.iter().any(|f| {
                f.name.same_expanded_name(&function.name) && f.params.len() == function.params.len()
            });
            if !overridden {
                merged.functions.push(function);
            }
        }
        // Lower precedence first in the end: prepended.
        let mut templates = part.templates;
        templates.append(&mut merged.templates);
        merged.templates = templates;
        let mut data = part.data_elements;
        data.append(&mut merged.data_elements);
        merged.data_elements = data;
    }
    // Globals and functions in increasing precedence, each module's in declaration order.
    merged
        .globals
        .sort_by_key(|g| sources.get(g.module).map_or(0, |s| s.precedence));
    merged
        .functions
        .sort_by_key(|f| sources.get(f.module).map_or(0, |s| s.precedence));
    if errors.is_empty() {
        Ok(merged)
    } else {
        Err(errors)
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_uri;

    #[test]
    fn uris_resolve_against_the_base() {
        assert_eq!(resolve_uri("b.xsl", Some("file:/d/a.xsl")), "file:/d/b.xsl");
        assert_eq!(
            resolve_uri("../c/b.xsl", Some("file:/d/e/a.xsl")),
            "file:/d/c/b.xsl"
        );
        assert_eq!(
            resolve_uri("file:/x.xsl", Some("file:/d/a.xsl")),
            "file:/x.xsl"
        );
        assert_eq!(resolve_uri("/x.xsl", Some("file:/d/a.xsl")), "file:/x.xsl");
        assert_eq!(resolve_uri("x.xsl", None), "x.xsl");
        assert_eq!(resolve_uri("", Some("file:/d/a.xsl")), "file:/d/a.xsl");
    }
}
