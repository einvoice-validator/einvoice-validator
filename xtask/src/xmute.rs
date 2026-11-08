//! The documents XML Mutate generates from the unit tests of the XRechnung
//! Schematron (`scripts/fetch-xrechnung.sh`): each is one mutation of a test,
//! with the comment `This is the active mutation configuration: <name>` just
//! before the `<?xmute?>` instruction that produced it. That instruction
//! states the expectation: `schematron-invalid` lists the rules that must
//! fail, `schematron-valid` the rules that must not, each as
//! `<schematron>:<rule id>`, separated by spaces or commas, where
//! `<schematron>` is the symbolic name the XRechnung build gives a stylesheet
//! (`xrubl`, `xrcii`).

use roxmltree::{Document, Node, NodeType, ParsingOptions};

use crate::util::{Error, Result, normalize_space};
use crate::vefa::{Expectation, ExpectationKind};

const ACTIVE: &str = "This is the active mutation configuration:";

/// The expectation of a generated document: `scope` holds the stylesheet the
/// rule ids are about, `error` the rules that must fail, `success` those that
/// must not, `description` the instruction's id and description.
pub(crate) fn expectation(text: &str, schematrons: &[(&str, &str)]) -> Result<Expectation> {
    let options = ParsingOptions {
        allow_dtd: false,
        nodes_limit: u32::MAX,
        ..ParsingOptions::default()
    };
    let doc =
        Document::parse_with_options(text, options).map_err(|e| Error::new(format!("{e}")))?;
    let active: Vec<Node<'_, '_>> = doc
        .descendants()
        .filter(|n| n.is_comment() && n.text().is_some_and(|t| t.trim().starts_with(ACTIVE)))
        .collect();
    let [comment] = active.as_slice() else {
        return Err(Error::new(format!(
            "{} comments `{ACTIVE}`, expected 1",
            active.len()
        )));
    };
    let pi = comment
        .next_siblings()
        .skip(1)
        .find(|n| !(n.is_text() && n.text().is_some_and(|t| t.trim().is_empty())))
        .filter(|n| n.node_type() == NodeType::PI)
        .and_then(|n| n.pi())
        .filter(|pi| pi.target == "xmute")
        .ok_or_else(|| Error::new("no <?xmute?> instruction after the active comment"))?;
    let attributes = pseudo_attributes(pi.value.unwrap_or(""))?;
    let get = |name: &str| {
        attributes
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    };
    let mut stylesheet: Option<&str> = None;
    let mut ids = |list: Option<&str>| -> Result<Vec<String>> {
        let mut out = Vec::new();
        for item in list
            .unwrap_or("")
            .split([',', ' ', '\t', '\n', '\r'])
            .filter(|s| !s.is_empty())
        {
            let (name, id) = item
                .split_once(':')
                .filter(|(n, id)| !n.is_empty() && !id.is_empty())
                .ok_or_else(|| Error::new(format!("{item:?}: expected <schematron>:<rule id>")))?;
            let (_, target) = schematrons
                .iter()
                .find(|(n, _)| *n == name)
                .ok_or_else(|| Error::new(format!("{item:?}: unknown Schematron {name}")))?;
            if stylesheet.is_some_and(|s| s != *target) {
                return Err(Error::new("expectations about two stylesheets"));
            }
            stylesheet = Some(target);
            out.push(id.to_owned());
        }
        Ok(out)
    };
    let error = ids(get("schematron-invalid"))?;
    let success = ids(get("schematron-valid"))?;
    let description = normalize_space(&format!(
        "{}: {}",
        get("id").unwrap_or(""),
        get("description").unwrap_or("")
    ));
    Ok(Expectation {
        kind: ExpectationKind::Xmute,
        scope: stylesheet.map(str::to_owned).into_iter().collect(),
        success,
        error,
        warning: Vec::new(),
        description,
    })
}

/// `name="value"` pairs (double or single quotes), separated by whitespace.
fn pseudo_attributes(text: &str) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    let mut rest = text.trim_start();
    while !rest.is_empty() {
        let (name, after) = rest
            .split_once('=')
            .ok_or_else(|| Error::new(format!("<?xmute {text}?>: expected name=\"value\"")))?;
        let name = name.trim();
        let after = after.trim_start();
        let quote = after
            .chars()
            .next()
            .filter(|c| *c == '"' || *c == '\'')
            .ok_or_else(|| Error::new(format!("<?xmute {text}?>: unquoted value of {name}")))?;
        let body = &after[1..];
        let end = body
            .find(quote)
            .ok_or_else(|| Error::new(format!("<?xmute {text}?>: unterminated value of {name}")))?;
        if name.is_empty() || name.contains(char::is_whitespace) {
            return Err(Error::new(format!("<?xmute {text}?>: bad name {name:?}")));
        }
        out.push((name.to_owned(), body[..end].to_owned()));
        rest = body[end + 1..].trim_start();
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::expectation;
    use crate::vefa::ExpectationKind;

    const SCHEMATRONS: &[(&str, &str)] = &[("xrubl", "XRechnung-UBL"), ("xrcii", "XRechnung-CII")];

    #[test]
    fn the_active_instruction_states_the_expectation() {
        let text = r#"<?xml version="1.0"?>
<a>
  <?xmute mutator="code" values="1,2" schematron-valid="xrcii:BR-DE-17" id="x" description="valid"?>
  <!--This is the active mutation configuration: t-3-code-9 -->
  <?xmute mutator="code" values="9" schematron-invalid="xrcii:BR-TMP-3,xrcii:PEPPOL-R120 xrcii:BR-DE-23-a" id="y" description="invalid  codes"?>
  <b>9</b>
</a>"#;
        let e = expectation(text, SCHEMATRONS).unwrap();
        assert_eq!(e.kind, ExpectationKind::Xmute);
        assert_eq!(e.scope, ["XRechnung-CII"]);
        assert_eq!(e.error, ["BR-TMP-3", "PEPPOL-R120", "BR-DE-23-a"]);
        assert_eq!(e.success, Vec::<String>::new());
        assert_eq!(e.description, "y: invalid codes");
    }

    #[test]
    fn malformed_documents_are_refused() {
        for text in [
            "<a/>",
            "<a><!--This is the active mutation configuration: x--><b/></a>",
            r#"<a><!--This is the active mutation configuration: x--><?xmute schematron-valid="zz:R1"?></a>"#,
            r#"<a><!--This is the active mutation configuration: x--><?xmute schematron-valid="xrubl:R1" schematron-invalid="xrcii:R2"?></a>"#,
        ] {
            assert!(expectation(text, SCHEMATRONS).is_err(), "{text}");
        }
    }
}
