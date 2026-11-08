//! The canonical SVRL form (`corpus/SVRL-CANONICAL.md`) is implemented by the
//! `einvoice-svrl` crate, shared with the engine's equivalence tests so that
//! both sides of a comparison use the same code.
//!
//! Reports written by SchXslt (the XRechnung configuration,
//! [`Generator::SchXslt`]) start with an `svrl:metadata` element, which
//! canonical form 2 represents (its run time written `{run-time}`). [`check`]
//! canonicalises a report after checking that it is what its generator writes:
//! version 1 for the ISO skeleton, version 2 with one `svrl:metadata`, the
//! first child after the namespace prefixes, for SchXslt, and, for a report of
//! the oracle, the time of the run equal to the clock it fixes ([`FIXED_NOW`]).
//! Anything else is an error, as in the canonical form itself: nothing is
//! trimmed silently.

use roxmltree::{Document, Node, ParsingOptions};

pub(crate) use einvoice_svrl::{is_document_uri, parse};

use crate::ruleset::Generator;
use crate::util::{Error, Result};

/// The `current-dateTime()` of every oracle run (Saxon-HE `-now:`), so that a
/// report that records the time of its run is the same on every run.
pub(crate) const FIXED_NOW: &str = "2000-01-01T00:00:00Z";

const SVRL_NS: &str = "http://purl.oclc.org/dsdl/svrl";

/// The canonical form of a report written by a stylesheet of `generator`.
/// With `fixed_clock`, the time of the run of a SchXslt report must be
/// [`FIXED_NOW`] (a report of the oracle); without, it is not checked (a
/// report of an engine that does not fix its clock).
pub(crate) fn check(
    raw: &str,
    generator: Generator,
    document_uri: &dyn Fn(&str) -> bool,
    fixed_clock: bool,
) -> Result<String> {
    let canonical = einvoice_svrl::canonicalize(raw, document_uri)?;
    let version_2 = canonical.starts_with(einvoice_svrl::HEADER_V2);
    match (generator, version_2) {
        (Generator::IsoSkeleton, false) => return Ok(canonical),
        (Generator::IsoSkeleton, true) => {
            return Err(Error::new(
                "SVRL: svrl:metadata in a report of the ISO Schematron skeleton",
            ));
        }
        (Generator::SchXslt, false) => {
            return Err(Error::new("SVRL: no svrl:metadata in a report of SchXslt"));
        }
        (Generator::SchXslt, true) => {}
    }
    let options = ParsingOptions {
        allow_dtd: false,
        nodes_limit: u32::MAX,
        ..ParsingOptions::default()
    };
    let doc =
        Document::parse_with_options(raw, options).map_err(|e| Error::new(format!("SVRL: {e}")))?;
    let first = doc
        .root_element()
        .children()
        .filter(Node::is_element)
        .find(|n| {
            n.tag_name().namespace() != Some(SVRL_NS)
                || n.tag_name().name() != "ns-prefix-in-attribute-values"
        })
        .ok_or_else(|| Error::new("SVRL: an empty report"))?;
    if first.tag_name().namespace() != Some(SVRL_NS) || first.tag_name().name() != "metadata" {
        return Err(Error::new(
            "SVRL: svrl:metadata is not the first child after the namespace prefixes",
        ));
    }
    if fixed_clock {
        let run_time = einvoice_svrl::run_time(raw)?.unwrap_or_default();
        if run_time != FIXED_NOW {
            return Err(Error::new(format!(
                "SVRL: svrl:metadata/dct:created is {run_time:?}, not the fixed clock {FIXED_NOW}"
            )));
        }
    }
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::{FIXED_NOW, check};
    use crate::ruleset::Generator;

    fn report(created: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<svrl:schematron-output xmlns:svrl="http://purl.oclc.org/dsdl/svrl" title="t">
   <svrl:ns-prefix-in-attribute-values prefix="cbc" uri="urn:x"/>
   <svrl:metadata xmlns:dct="http://purl.org/dc/terms/" xmlns:skos="http://www.w3.org/2004/02/skos/core#">
      <dct:creator><dct:Agent><skos:prefLabel>SAXON/HE 12.10</skos:prefLabel></dct:Agent></dct:creator>
      <dct:created>{created}</dct:created>
   </svrl:metadata>
   <svrl:active-pattern documents="file:/w/c1.xml" id="p" name="p"/>
   <svrl:fired-rule context="/"/>
</svrl:schematron-output>
"#
        )
    }

    #[test]
    fn schxslt_reports_are_version_2() {
        let canonical = check(
            &report(FIXED_NOW),
            Generator::SchXslt,
            &|uri| uri == "file:/w/c1.xml",
            true,
        )
        .unwrap();
        assert_eq!(
            canonical,
            "svrl-canonical 2\nschematron-output title=\"t\"\nns-prefix-in-attribute-values prefix=\"cbc\" uri=\"urn:x\"\nmetadata\n  {http://purl.org/dc/terms/}creator\n    {http://purl.org/dc/terms/}Agent\n      {http://www.w3.org/2004/02/skos/core#}prefLabel \"SAXON/HE 12.10\"\n  {http://purl.org/dc/terms/}created \"{run-time}\"\nactive-pattern documents=\"{document}\" id=\"p\" name=\"p\"\nfired-rule context=\"/\"\n"
        );
    }

    #[test]
    fn anything_else_is_refused() {
        let uri = |_: &str| true;
        let error = check(
            &report("2026-11-08T10:00:00Z"),
            Generator::SchXslt,
            &uri,
            true,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("not the fixed clock"), "{error}");
        assert!(
            check(
                &report("2026-11-08T10:00:00Z"),
                Generator::SchXslt,
                &uri,
                false
            )
            .is_ok()
        );
        let error = check(&report(FIXED_NOW), Generator::IsoSkeleton, &uri, true)
            .unwrap_err()
            .to_string();
        assert!(error.contains("ISO Schematron skeleton"), "{error}");
        let without = report(FIXED_NOW).replace("svrl:metadata", "svrl:text");
        assert!(check(&without, Generator::SchXslt, &uri, true).is_err());
        let late = report(FIXED_NOW).replace(
            "<svrl:fired-rule context=\"/\"/>",
            "<svrl:fired-rule context=\"/\"/><svrl:ns-prefix-in-attribute-values prefix=\"a\" uri=\"urn:a\"/>",
        );
        assert!(check(&late, Generator::SchXslt, &uri, true).is_ok());
        let not_first = report(FIXED_NOW).replace(
            "<svrl:ns-prefix-in-attribute-values prefix=\"cbc\" uri=\"urn:x\"/>",
            "<svrl:active-pattern documents=\"file:/w/c1.xml\" id=\"q\"/>",
        );
        assert!(check(&not_first, Generator::SchXslt, &uri, true).is_err());
    }
}
