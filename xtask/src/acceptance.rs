//! Documents the reference pipeline of a rule set accepts (published
//! examples, test-suite instances): for each, the stylesheets of its profile
//! must report nothing that rejects it ([`crate::ruleset::Acceptance`]).
//!
//! The profile is chosen by the specification identifier the document
//! declares (BT-24): `cbc:CustomizationID` under the root of a UBL document,
//! `ExchangedDocumentContext/GuidelineSpecifiedDocumentContextParameter/ID`
//! in CII and CDAR. With [`Profiles::KositScenarios`], the scenario of the
//! KoSIT validator configuration whose `match` names the root element and the
//! identifier gives the stylesheets (its `validateWithSchematron` resources)
//! and the levels that override the flags of rule ids (its `customLevel`
//! elements), as the KoSIT validator applies them.

use std::path::Path;

use roxmltree::{Document, Node, ParsingOptions};

use crate::corpus::Case;
use crate::crosscheck::{self, Disagreement};
use crate::ruleset::{Profiles, RuleSet, Stylesheet};
use crate::util::{self, Error, Result};

const SCENARIOS_NS: &str = "http://www.xoev.de/de/validator/framework/1/scenarios";

/// What the check found.
#[derive(Debug, Default)]
pub(crate) struct Outcome {
    pub(crate) cases: usize,
    /// Documents no stylesheet of their profile rejects.
    pub(crate) agreeing: usize,
    pub(crate) disagreements: Vec<Disagreement>,
}

/// One scenario of a KoSIT validator configuration.
#[derive(Debug)]
struct Scenario {
    name: String,
    /// The local name of the root element its `match` requires.
    root: String,
    /// The specification identifier its `match` requires.
    identifier: String,
    /// The stylesheets, by their location relative to the configuration.
    locations: Vec<String>,
    /// (rule id, level).
    levels: Vec<(String, String)>,
}

/// Checks every whole-document case under the acceptance prefixes.
pub(crate) fn run(root: &Path, rule_set: &RuleSet, cases: &[Case]) -> Result<Outcome> {
    let Some(acceptance) = &rule_set.acceptance else {
        return Ok(Outcome::default());
    };
    let base = rule_set.base(root);
    let scenarios = match acceptance.profiles {
        Profiles::Listed(_) => None,
        Profiles::KositScenarios(file) => Some((file, scenarios(&base, file)?)),
    };
    let mut outcome = Outcome::default();
    for case in cases {
        if case.expectation.is_some()
            || !acceptance
                .cases
                .iter()
                .any(|p| case.id.starts_with(&format!("{p}/")))
        {
            continue;
        }
        let text = util::read_string(&util::join(&base, &case.input))?;
        let (root_name, identifier) =
            identify(&text).map_err(|e| Error::new(format!("{}: {e}", case.id)))?;
        let (stylesheets, levels) =
            profile(rule_set, case, &root_name, &identifier, scenarios.as_ref())?;
        outcome.cases += 1;
        let mut rejected = false;
        for stylesheet in stylesheets {
            let findings =
                crosscheck::findings_with_kind(root, rule_set, std::iter::once(stylesheet), case)?;
            for (kind, id, flag) in findings {
                let level = levels
                    .iter()
                    .find(|(rule, _)| *rule == id)
                    .map_or_else(|| default_level(kind, &flag), |(_, level)| level.as_str());
                if level == "error" {
                    rejected = true;
                    let disagreement = Disagreement {
                        case: case.id.clone(),
                        kind: "rejected",
                        rule: id,
                        saxon: format!("{flag}@{}", stylesheet.name),
                    };
                    if !outcome.disagreements.contains(&disagreement) {
                        outcome.disagreements.push(disagreement);
                    }
                }
            }
        }
        if !rejected {
            outcome.agreeing += 1;
        }
    }
    Ok(outcome)
}

/// The stylesheets of a document's profile, and the levels that override the
/// flags of rule ids: `(rule id, level)`.
type Profile<'r, 's> = (Vec<&'r Stylesheet>, &'s [(String, String)]);

/// The stylesheets of the profile of a document, and the levels that override
/// the flags of its rule ids.
fn profile<'r, 's>(
    rule_set: &'r RuleSet,
    case: &Case,
    root_name: &str,
    identifier: &str,
    scenarios: Option<&'s (&'static str, Vec<Scenario>)>,
) -> Result<Profile<'r, 's>> {
    let Some(acceptance) = &rule_set.acceptance else {
        return Err(Error::new("no acceptance"));
    };
    let identifier = identifier.to_owned();
    let root_name = root_name.to_owned();
    Ok(match (&acceptance.profiles, scenarios) {
        (Profiles::Listed(profiles), _) => {
            let (_, names) = profiles
                .iter()
                .find(|(id, _)| *id == identifier)
                .ok_or_else(|| {
                    Error::new(format!(
                        "{}: specification identifier {identifier:?}: no profile of {} lists it",
                        case.id, rule_set.name
                    ))
                })?;
            let stylesheets = names
                .iter()
                .map(|n| {
                    rule_set
                        .stylesheet(n)
                        .ok_or_else(|| Error::new(format!("no stylesheet {n}")))
                })
                .collect::<Result<_>>()?;
            (stylesheets, &[])
        }
        (Profiles::KositScenarios(_), Some((file, scenarios))) => {
            let scenario = scenarios
                .iter()
                .find(|s| s.root == root_name && s.identifier == identifier)
                .ok_or_else(|| {
                    Error::new(format!(
                        "{}: no scenario of {file} matches {root_name} {identifier:?}",
                        case.id
                    ))
                })?;
            let dir = file.rsplit_once('/').map_or("", |(d, _)| d);
            let stylesheets = scenario
                .locations
                .iter()
                .map(|l| {
                    let xslt = format!("{dir}/{l}");
                    rule_set
                        .stylesheets
                        .iter()
                        .find(|s| s.xslt == xslt)
                        .ok_or_else(|| {
                            Error::new(format!(
                                "scenario {:?}: {l} is not a stylesheet of {}",
                                scenario.name, rule_set.name
                            ))
                        })
                })
                .collect::<Result<_>>()?;
            (stylesheets, scenario.levels.as_slice())
        }
        (Profiles::KositScenarios(_), None) => {
            return Err(Error::new("scenarios not read"));
        }
    })
}

/// The level of a finding without an override: a failed assert is an error
/// unless its flag is `warning` or `information` (a failed assert without a
/// flag included, as the KoSIT validator and the Mustang validator read it); a
/// successful report is an error only with the flag `fatal` or `error`.
fn default_level(kind: &str, flag: &str) -> &'static str {
    let error = if kind == "failed-assert" {
        !matches!(flag, "warning" | "information")
    } else {
        matches!(flag, "fatal" | "error")
    };
    if error { "error" } else { "other" }
}

fn parse(text: &str) -> Result<Document<'_>> {
    let options = ParsingOptions {
        allow_dtd: false,
        nodes_limit: u32::MAX,
        ..ParsingOptions::default()
    };
    Document::parse_with_options(text, options).map_err(|e| Error::new(format!("{e}")))
}

fn child<'a, 'input>(node: Node<'a, 'input>, local: &str) -> Option<Node<'a, 'input>> {
    node.children()
        .find(|c| c.is_element() && c.tag_name().name() == local)
}

/// The local name of the root element and the specification identifier.
fn identify(text: &str) -> Result<(String, String)> {
    let doc = parse(text)?;
    let root = doc.root_element();
    let id = child(root, "CustomizationID")
        .or_else(|| {
            child(root, "ExchangedDocumentContext")
                .and_then(|c| child(c, "GuidelineSpecifiedDocumentContextParameter"))
                .and_then(|c| child(c, "ID"))
        })
        .ok_or_else(|| Error::new("no specification identifier (BT-24)"))?;
    let identifier: String = id.children().filter_map(|c| c.text()).collect();
    Ok((
        root.tag_name().name().to_owned(),
        identifier.trim().to_owned(),
    ))
}

/// The descendants of `node` named `local` in the scenarios namespace.
fn elements<'a, 'input>(node: Node<'a, 'input>, local: &str) -> Vec<Node<'a, 'input>> {
    node.descendants()
        .filter(|n| {
            n.is_element()
                && n.tag_name().namespace() == Some(SCENARIOS_NS)
                && n.tag_name().name() == local
        })
        .collect()
}

/// The scenarios of `file` (relative to `base`).
fn scenarios(base: &Path, file: &str) -> Result<Vec<Scenario>> {
    let text = util::read_string(&util::join(base, file))?;
    let doc = parse(&text).map_err(|e| Error::new(format!("{file}: {e}")))?;
    let mut out = Vec::new();
    let text_of = |node: Node<'_, '_>| -> String {
        node.children()
            .filter_map(|c| c.text())
            .collect::<String>()
            .trim()
            .to_owned()
    };
    for scenario in elements(doc.root_element(), "scenario") {
        let name = elements(scenario, "name")
            .first()
            .map(|n| text_of(*n))
            .unwrap_or_default();
        let fail = |why: &str| Error::new(format!("{file}: scenario {name:?}: {why}"));
        let [matching] = elements(scenario, "match")[..] else {
            return Err(fail("not exactly one match"));
        };
        let (root, identifier) = parse_match(&text_of(matching))
            .ok_or_else(|| fail("match is not exists(/<prefix>:<root>[...'<identifier>'...])"))?;
        let mut locations = Vec::new();
        for validate in elements(scenario, "validateWithSchematron") {
            let [location] = elements(validate, "location")[..] else {
                return Err(fail(
                    "a validateWithSchematron without exactly one location",
                ));
            };
            locations.push(text_of(location));
        }
        if locations.is_empty() {
            return Err(fail("no validateWithSchematron"));
        }
        let mut levels = Vec::new();
        for level in elements(scenario, "customLevel") {
            let value = level
                .attribute("level")
                .ok_or_else(|| fail("customLevel without level"))?;
            if !["error", "warning", "information"].contains(&value) {
                return Err(fail(&format!("customLevel level {value:?}")));
            }
            levels.push((text_of(level), value.to_owned()));
        }
        out.push(Scenario {
            name,
            root,
            identifier,
            locations,
            levels,
        });
    }
    if out.is_empty() {
        return Err(Error::new(format!("{file}: no scenario")));
    }
    Ok(out)
}

/// `exists(/invoice:Invoice/cbc:CustomizationID[ . = 'X'])` and the other
/// forms of the configuration: (`Invoice`, `X`). Exactly one quoted literal.
fn parse_match(text: &str) -> Option<(String, String)> {
    let path = text.trim().strip_prefix("exists(/")?.strip_suffix(')')?;
    let step = path.split(['/', '[']).next()?;
    let (_, root) = step.split_once(':')?;
    let mut literals = path.split('\'');
    let _before = literals.next()?;
    let identifier = literals.next()?;
    let _after = literals.next()?;
    if literals.next().is_some() || root.is_empty() || identifier.is_empty() {
        return None;
    }
    Some((root.to_owned(), identifier.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::{identify, parse_match, run};
    use crate::corpus::Case;
    use crate::crosscheck::Disagreement;
    use crate::ruleset::{Acceptance, Generator, Origin, Profiles, RECORD, RuleSet, Stylesheet};
    use crate::util;

    /// A negative control: what rejects a document, and what does not.
    #[test]
    fn rejections_follow_the_flags() {
        const SET: RuleSet = RuleSet {
            name: "synthetic-acceptance",
            origin: Origin::Vendored { licence: "LICENSE" },
            source: "src",
            stylesheets: &[Stylesheet {
                name: "S",
                xslt: "s.xslt",
                golden_dir: "golden",
                sha256_pin: None,
                side_files: &[],
                generator: Generator::IsoSkeleton,
            }],
            document_dirs: &[],
            vefa_dirs: &[],
            xmute_dirs: &[],
            xmute_schematrons: &[],
            ignored: &[],
            mutation_catalogue: None,
            mutation_bases: &[],
            acceptance: Some(Acceptance {
                cases: &["ex"],
                profiles: Profiles::Listed(&[("urn:p", &["S"])]),
            }),
            record: RECORD,
        };
        let root = std::env::temp_dir().join(format!("xtask-acceptance-{}", std::process::id()));
        let document = "<Invoice xmlns:cbc='urn:cbc'><cbc:CustomizationID>urn:p</cbc:CustomizationID></Invoice>";
        // (case, finding): an unflagged failed assert and a fatal successful
        // report reject; a warning failed assert and an unflagged successful
        // report do not.
        let findings = [
            ("a", "failed-assert id=\"R1\" location=\"/\" test=\"t\""),
            (
                "b",
                "failed-assert flag=\"warning\" id=\"R2\" location=\"/\" test=\"t\"",
            ),
            (
                "c",
                "successful-report flag=\"fatal\" id=\"R3\" location=\"/\" test=\"t\"",
            ),
            ("d", "successful-report id=\"R4\" location=\"/\" test=\"t\""),
        ];
        let mut cases = Vec::new();
        for (name, finding) in findings {
            let input = format!("src/{name}.xml");
            util::write(&util::join(&root, &input), document.as_bytes()).unwrap();
            let golden = format!("svrl-canonical 1\nschematron-output\n{finding}\n  text \"m\"\n");
            util::write(
                &root.join(format!(
                    "corpus/synthetic-acceptance/golden/ex/{name}.xml.svrl.txt"
                )),
                golden.as_bytes(),
            )
            .unwrap();
            cases.push(Case {
                id: format!("ex/{name}.xml"),
                input,
                expectation: None,
                mutation: false,
            });
        }
        // Outside the prefixes: not checked.
        cases.push(Case {
            id: "other/e.xml".to_owned(),
            input: "src/missing.xml".to_owned(),
            expectation: None,
            mutation: false,
        });
        let outcome = run(&root, &SET, &cases).unwrap();
        util::remove_dir(&root).unwrap();
        assert_eq!(outcome.cases, 4);
        assert_eq!(outcome.agreeing, 2);
        let d = |case: &str, rule: &str, flag: &str| Disagreement {
            case: case.to_owned(),
            kind: "rejected",
            rule: rule.to_owned(),
            saxon: format!("{flag}@S"),
        };
        assert_eq!(
            outcome.disagreements,
            [d("ex/a.xml", "R1", "none"), d("ex/c.xml", "R3", "fatal")]
        );
    }

    #[test]
    fn scenario_matches() {
        assert_eq!(
            parse_match(
                "exists(/invoice:Invoice/cbc:CustomizationID[ . = 'urn:cen.eu:en16931:2017#compliant#urn:xeinkauf.de:kosit:xrechnung_3.0']) "
            ),
            Some((
                "Invoice".to_owned(),
                "urn:cen.eu:en16931:2017#compliant#urn:xeinkauf.de:kosit:xrechnung_3.0".to_owned()
            ))
        );
        assert_eq!(
            parse_match(
                "exists(/rsm:CrossIndustryInvoice[rsm:ExchangedDocumentContext/ram:GuidelineSpecifiedDocumentContextParameter/ram:ID/text() = 'urn:cen.eu:en16931:2017'])"
            ),
            Some((
                "CrossIndustryInvoice".to_owned(),
                "urn:cen.eu:en16931:2017".to_owned()
            ))
        );
        assert_eq!(parse_match("exists(/a:B[c = 'x' or c = 'y'])"), None);
        assert_eq!(parse_match("true()"), None);
    }

    #[test]
    fn specification_identifiers() {
        let ubl = "<Invoice xmlns='urn:oasis:names:specification:ubl:schema:xsd:Invoice-2' xmlns:cbc='urn:cbc'><cbc:CustomizationID> urn:x </cbc:CustomizationID></Invoice>";
        assert_eq!(
            identify(ubl).unwrap(),
            ("Invoice".to_owned(), "urn:x".to_owned())
        );
        let cii = "<r:CrossIndustryInvoice xmlns:r='urn:r' xmlns:a='urn:a'><r:ExchangedDocumentContext><a:GuidelineSpecifiedDocumentContextParameter><a:ID>urn:y</a:ID></a:GuidelineSpecifiedDocumentContextParameter></r:ExchangedDocumentContext></r:CrossIndustryInvoice>";
        assert_eq!(
            identify(cii).unwrap(),
            ("CrossIndustryInvoice".to_owned(), "urn:y".to_owned())
        );
        assert!(identify("<a/>").is_err());
    }
}
