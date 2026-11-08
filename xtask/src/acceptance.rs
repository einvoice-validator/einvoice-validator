//! What the reference validator of a rule set makes of the findings of its
//! stylesheets: which ones reject a document.
//!
//! A finding rejects when its level is `error`: a failed assert unless its
//! flag is `warning` or `information` (a failed assert without a flag
//! included, as the KoSIT validator and the Mustang validator read it), a
//! successful report only with the flag `fatal` or `error`; for the XRechnung
//! rule sets, the scenario of the KoSIT validator configuration that the
//! document matches overrides the level of the rule ids its `customLevel`
//! elements list, as the KoSIT validator applies them.
//!
//! Two uses:
//!
//! - [`run`]: documents the reference pipeline of a rule set accepts
//!   (published examples, test-suite instances): for each, the stylesheets of
//!   its profile must report nothing that rejects it
//!   ([`crate::ruleset::Acceptance`]). The profile is chosen by the
//!   specification identifier the document declares (BT-24):
//!   `cbc:CustomizationID` under the root of a UBL document,
//!   `ExchangedDocumentContext/GuidelineSpecifiedDocumentContextParameter/ID`
//!   in CII and CDAR. With [`Profiles::KositScenarios`], the scenario whose
//!   `match` the document satisfies gives the stylesheets (its
//!   `validateWithSchematron` resources) and the levels.
//! - [`write_table`]: the reference acceptance of every case of a fetched
//!   corpus, stylesheet by stylesheet ([`TABLE`]), which the tests of
//!   `einvoice-validate --rules` compare the verdicts of the bundles with; and
//!   [`check_bundle`]: the levels the XRechnung bundles give rule ids are the
//!   configuration's.

use std::fmt::Write as _;
use std::path::Path;

use roxmltree::{Document, Node, ParsingOptions};

use crate::corpus::{self, Case};
use crate::crosscheck::Disagreement;
use crate::ruleset::{Profiles, RuleSet, Stylesheet};
use crate::svrl;
use crate::util::{self, Error, Result};

const SCENARIOS_NS: &str = "http://www.xoev.de/de/validator/framework/1/scenarios";

/// The reference acceptance of every case, relative to the corpus directory.
pub(crate) const TABLE: &str = "acceptance.tsv";

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
    /// The namespace and local name of the root element its `match` requires.
    root: (String, String),
    /// The path of the element holding the identifier, from the root: the
    /// namespace and local name of each step.
    path: Vec<(String, String)>,
    /// The specification identifier its `match` requires.
    identifier: String,
    /// The stylesheets, by their location relative to the configuration.
    locations: Vec<String>,
    /// (rule id, level).
    levels: Vec<(String, String)>,
}

impl Scenario {
    /// Whether the document whose root element is `root` satisfies the
    /// `match` of the scenario: the root element has its name, and an element
    /// at its path has the identifier as its string value, compared as it is
    /// written (the `text()` of the configuration's CII and credit note
    /// matches is read as the string value: the two differ only for an
    /// element with comments or child elements, which an identifier has not).
    fn matches(&self, root: Node<'_, '_>) -> bool {
        let name = |n: Node<'_, '_>, (ns, local): &(String, String)| {
            n.tag_name().namespace().unwrap_or("") == ns && n.tag_name().name() == local
        };
        if !name(root, &self.root) {
            return false;
        }
        let mut nodes = vec![root];
        for step in &self.path {
            nodes = nodes
                .iter()
                .flat_map(|n| n.children().filter(|c| c.is_element() && name(*c, step)))
                .collect();
        }
        nodes.iter().any(|n| string_value(*n) == self.identifier)
    }

    /// The level of the rule `id` in the documents the scenario matches.
    fn level(&self, id: &str) -> Option<&str> {
        self.levels
            .iter()
            .find(|(rule, _)| rule == id)
            .map(|(_, level)| level.as_str())
    }
}

/// The string value of an element: its descendant text, in document order.
fn string_value(node: Node<'_, '_>) -> String {
    node.descendants()
        .filter(Node::is_text)
        .filter_map(|n| n.text())
        .collect()
}

/// Checks every whole-document case under the acceptance prefixes.
pub(crate) fn run(root: &Path, rule_set: &RuleSet, cases: &[Case]) -> Result<Outcome> {
    let Some(acceptance) = &rule_set.acceptance else {
        return Ok(Outcome::default());
    };
    let base = rule_set.base(root);
    let scenarios = match acceptance.profiles {
        Profiles::Listed(_) => None,
        Profiles::KositScenarios { file, .. } => Some((file, scenarios(&base, file)?)),
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
        let (stylesheets, scenario) = profile(rule_set, case, &text, scenarios.as_ref())?;
        outcome.cases += 1;
        let mut rejected = false;
        for stylesheet in stylesheets {
            for (kind, id, flag) in findings(&base, rule_set, stylesheet, case)? {
                if level(kind, id.as_deref(), &flag, scenario) == "error" {
                    rejected = true;
                    let disagreement = Disagreement {
                        case: case.id.clone(),
                        kind: "rejected",
                        rule: id.unwrap_or_else(|| "none".to_owned()),
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

/// The stylesheets of the profile of a document (its text), and the scenario
/// that gives the levels of its rule ids, if any.
fn profile<'r, 's>(
    rule_set: &'r RuleSet,
    case: &Case,
    text: &str,
    scenarios: Option<&'s (&'static str, Vec<Scenario>)>,
) -> Result<(Vec<&'r Stylesheet>, Option<&'s Scenario>)> {
    let Some(acceptance) = &rule_set.acceptance else {
        return Err(Error::new("no acceptance"));
    };
    Ok(match (&acceptance.profiles, scenarios) {
        (Profiles::Listed(profiles), _) => {
            let (_, identifier) =
                identify(text).map_err(|e| Error::new(format!("{}: {e}", case.id)))?;
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
            (stylesheets, None)
        }
        (Profiles::KositScenarios { .. }, Some((file, scenarios))) => {
            let doc = parse(text).map_err(|e| Error::new(format!("{}: {e}", case.id)))?;
            let scenario = scenarios
                .iter()
                .find(|s| s.matches(doc.root_element()))
                .ok_or_else(|| {
                    Error::new(format!("{}: no scenario of {file} matches it", case.id))
                })?;
            (stylesheets_of(rule_set, scenario, file)?, Some(scenario))
        }
        (Profiles::KositScenarios { .. }, None) => {
            return Err(Error::new("scenarios not read"));
        }
    })
}

/// The stylesheets of `rule_set` that a scenario of the configuration `file`
/// runs.
fn stylesheets_of<'r>(
    rule_set: &'r RuleSet,
    scenario: &Scenario,
    file: &str,
) -> Result<Vec<&'r Stylesheet>> {
    let dir = file.rsplit_once('/').map_or("", |(d, _)| d);
    scenario
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
        .collect()
}

/// (kind, rule id, flag) of every failed assert and successful report of the
/// golden of a case for one stylesheet; the flag is `none` when there is none.
fn findings(
    base: &Path,
    rule_set: &RuleSet,
    stylesheet: &Stylesheet,
    case: &Case,
) -> Result<Vec<(&'static str, Option<String>, String)>> {
    let path = case.golden(rule_set, stylesheet);
    let text = util::read_string(&util::join(base, &path))?;
    let lines = svrl::parse(&text).map_err(|e| Error::new(format!("{path}: {e}")))?;
    Ok(lines
        .iter()
        .filter_map(|l| {
            let kind = match l.kind.as_str() {
                "failed-assert" => "failed-assert",
                "successful-report" => "successful-report",
                _ => return None,
            };
            Some((
                kind,
                l.attribute("id").map(str::to_owned),
                l.attribute("flag").unwrap_or("none").to_owned(),
            ))
        })
        .collect())
}

/// The level of a finding: the scenario's for its rule id, if it gives one,
/// else [`default_level`].
fn level<'a>(kind: &str, id: Option<&str>, flag: &str, scenario: Option<&'a Scenario>) -> &'a str {
    id.and_then(|id| scenario.and_then(|s| s.level(id)))
        .unwrap_or_else(|| default_level(kind, flag))
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

/// Writes [`TABLE`] in the corpus directory of a fetched rule set: for every
/// case (whole documents, test cases, mutations), the scenario it matches
/// (`-` for none, or a rule set without scenarios), then for each stylesheet
/// (a column named after its golden directory) `reject` when one of its
/// findings has the level `error`, `accept` when none has, `error` when
/// Saxon-HE stopped on the case. Returns a summary line.
pub(crate) fn write_table(root: &Path, rule_set: &RuleSet) -> Result<String> {
    let base = rule_set.base(root);
    let cases = corpus::read_all(root, rule_set)?;
    let scenarios = match rule_set.acceptance.as_ref().map(|a| &a.profiles) {
        Some(Profiles::KositScenarios { file, .. }) => scenarios(&base, file)?,
        _ => Vec::new(),
    };
    let mut table = String::from("case\tscenario");
    for stylesheet in rule_set.stylesheets {
        let _ = write!(table, "\t{}", stylesheet.golden_dir);
    }
    table.push('\n');
    let mut rejected = vec![0usize; rule_set.stylesheets.len()];
    let mut matched = 0;
    for case in &cases {
        let scenario = if scenarios.is_empty() {
            None
        } else {
            let text = util::read_string(&util::join(&base, &case.input))?;
            parse(&text)
                .ok()
                .and_then(|doc| scenarios.iter().find(|s| s.matches(doc.root_element())))
        };
        matched += usize::from(scenario.is_some());
        let _ = write!(
            table,
            "{}\t{}",
            case.id,
            scenario.map_or("-", |s| s.name.as_str())
        );
        for (stylesheet, count) in rule_set.stylesheets.iter().zip(&mut rejected) {
            let stopped = util::join(&base, &case.error_golden(rule_set, stylesheet)).is_file();
            let verdict = if stopped {
                "error"
            } else if findings(&base, rule_set, stylesheet, case)?
                .iter()
                .any(|(kind, id, flag)| level(kind, id.as_deref(), flag, scenario) == "error")
            {
                *count += 1;
                "reject"
            } else {
                "accept"
            };
            let _ = write!(table, "\t{verdict}");
        }
        table.push('\n');
    }
    let corpus_dir = util::join(&base, &rule_set.corpus_dir());
    util::write(&corpus_dir.join(TABLE), table.as_bytes())?;
    let rejections: Vec<String> = rule_set
        .stylesheets
        .iter()
        .zip(&rejected)
        .map(|(s, n)| format!("{} rejects {n}", s.name))
        .collect();
    Ok(format!(
        "acceptance written: {}: {} cases, {matched} matching a scenario; {}",
        rule_set.name,
        cases.len(),
        rejections.join(", ")
    ))
}

/// The namespace of a root element of a KoSIT scenario, as the bundles write
/// it.
fn bundle_namespace(root: &str) -> &'static str {
    match root {
        "Invoice" => "urn:oasis:names:specification:ubl:schema:xsd:Invoice-2",
        "CreditNote" => "urn:oasis:names:specification:ubl:schema:xsd:CreditNote-2",
        _ => "urn:un:unece:uncefact:data:standard:CrossIndustryInvoice:100",
    }
}

/// Checks that the bundle the fetch script wrote for a rule set with KoSIT
/// scenarios gives rule ids the levels of the configuration: its
/// `[[scenario]]` tables, after its stylesheets, are those of the scenarios
/// of the configuration that run its stylesheets and list `customLevel`
/// elements, in their order (`error` written `fatal`, `information` `other`),
/// and its stylesheets read an unflagged failed assert as fatal. Returns a
/// summary line; nothing for another rule set.
pub(crate) fn check_bundle(root: &Path, rule_set: &RuleSet) -> Result<Option<String>> {
    let Some(Profiles::KositScenarios { file, bundle }) =
        rule_set.acceptance.as_ref().map(|a| &a.profiles)
    else {
        return Ok(None);
    };
    let base = rule_set.base(root);
    let mut expected = String::new();
    let mut count = 0;
    for scenario in scenarios(&base, file)? {
        if scenario.levels.is_empty() || stylesheets_of(rule_set, &scenario, file).is_err() {
            continue;
        }
        let (namespace, local) = &scenario.root;
        if namespace != bundle_namespace(local) {
            return Err(Error::new(format!(
                "scenario {:?}: root element {{{namespace}}}{local}",
                scenario.name
            )));
        }
        let _ = write!(
            expected,
            "\n[[scenario]]\nname = \"{}\"\nnamespace = \"{namespace}\"\nroot = \"{local}\"\nidentifier = \"{}\"\n",
            scenario.name, scenario.identifier
        );
        for (level, key) in [
            ("error", "fatal"),
            ("warning", "warning"),
            ("information", "other"),
        ] {
            let ids: Vec<&str> = scenario
                .levels
                .iter()
                .filter(|(_, l)| l == level)
                .map(|(id, _)| id.as_str())
                .collect();
            if !ids.is_empty() {
                let _ = writeln!(expected, "{key} = \"{}\"", ids.join(" "));
            }
        }
        count += 1;
    }
    let path = format!("{bundle}/bundle.toml");
    let manifest = util::read_string(&util::join(&base, &path))?;
    let actual = manifest
        .find("\n[[scenario]]")
        .map_or("", |start| &manifest[start..]);
    if actual != expected {
        return Err(Error::new(format!(
            "{path}: its [[scenario]] tables are not the levels of {file}: expected\n{expected}\nfound\n{actual}"
        )));
    }
    let stylesheets = manifest.matches("\n[[stylesheet]]\n").count();
    let fatal = manifest.matches("\nunflagged-assert = \"fatal\"\n").count();
    if stylesheets != rule_set.stylesheets.len() || fatal != stylesheets {
        return Err(Error::new(format!(
            "{path}: {stylesheets} stylesheets, {fatal} with unflagged-assert = \"fatal\": every stylesheet of {} reads an unflagged failed assert as fatal",
            rule_set.name
        )));
    }
    Ok(Some(format!(
        "bundle checked: {}: {path}, {count} scenarios with the levels of {file}",
        rule_set.name
    )))
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
        let mut namespaces = Vec::new();
        for namespace in elements(scenario, "namespace") {
            let prefix = namespace
                .attribute("prefix")
                .ok_or_else(|| fail("a namespace without prefix"))?;
            namespaces.push((prefix.to_owned(), text_of(namespace)));
        }
        let [matching] = elements(scenario, "match")[..] else {
            return Err(fail("not exactly one match"));
        };
        let Match {
            root,
            path,
            identifier,
        } = parse_match(&text_of(matching), &namespaces).ok_or_else(|| {
            fail("match is not exists(/p:Root[/steps][[steps/text() | .] = '<identifier>'])")
        })?;
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
        let mut levels: Vec<(String, String)> = Vec::new();
        for level in elements(scenario, "customLevel") {
            let value = level
                .attribute("level")
                .ok_or_else(|| fail("customLevel without level"))?;
            if !["error", "warning", "information"].contains(&value) {
                return Err(fail(&format!("customLevel level {value:?}")));
            }
            // The validator compares each code with the whitespace-separated
            // tokens of the element.
            for id in text_of(level).split_whitespace() {
                if levels.iter().any(|(rule, _)| rule == id) {
                    return Err(fail(&format!("rule id {id:?} in two customLevel")));
                }
                levels.push((id.to_owned(), value.to_owned()));
            }
        }
        out.push(Scenario {
            name,
            root,
            path,
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

/// What the `match` of a scenario requires.
#[derive(Debug, PartialEq, Eq)]
struct Match {
    /// The namespace and local name of the root element.
    root: (String, String),
    /// The path from the root to the element holding the identifier.
    path: Vec<(String, String)>,
    identifier: String,
}

/// The forms of the configuration's matches:
/// `exists(/invoice:Invoice/cbc:CustomizationID[ . = 'X'])`,
/// `exists(/creditnote:CreditNote[ cbc:CustomizationID/text() = 'X'])`,
/// `exists(/rsm:CrossIndustryInvoice[rsm:A/ram:B/ram:ID/text() = 'X'])`: a
/// path from the root, a predicate comparing a relative path (or `.`) with
/// one quoted literal, the prefixes resolved with `namespaces`.
fn parse_match(text: &str, namespaces: &[(String, String)]) -> Option<Match> {
    let path = text.trim().strip_prefix("exists(/")?.strip_suffix(')')?;
    let (steps, predicate) = path.split_once('[')?;
    let predicate = predicate.strip_suffix(']')?;
    let (left, right) = predicate.split_once('=')?;
    let identifier = right.trim().strip_prefix('\'')?.strip_suffix('\'')?;
    if identifier.is_empty() || identifier.contains('\'') {
        return None;
    }
    let qname = |step: &str| -> Option<(String, String)> {
        let (prefix, local) = step.trim().split_once(':')?;
        let (_, namespace) = namespaces.iter().find(|(p, _)| p == prefix)?;
        let simple = |s: &str| {
            !s.is_empty()
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        };
        (simple(prefix) && simple(local)).then(|| (namespace.clone(), local.to_owned()))
    };
    let mut steps = steps.split('/');
    let root = qname(steps.next()?)?;
    let mut path: Vec<(String, String)> = steps.map(qname).collect::<Option<_>>()?;
    let left = left.trim();
    if left != "." {
        let relative = left.strip_suffix("/text()").unwrap_or(left);
        path.extend(relative.split('/').map(qname).collect::<Option<Vec<_>>>()?);
    }
    if path.is_empty() {
        return None;
    }
    Some(Match {
        root,
        path,
        identifier: identifier.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;
    use std::path::Path;

    use super::{Match, check_bundle, identify, parse, parse_match, run, scenarios, write_table};
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

    /// A rule set with KoSIT scenarios, of two stylesheets.
    const TABLE_SET: RuleSet = RuleSet {
        name: "synthetic-table",
        origin: Origin::Fetched {
            base: "base",
            corpus: "corpus",
        },
        source: "src",
        stylesheets: &[
            Stylesheet {
                name: "A",
                xslt: "conf/a.xsl",
                golden_dir: "golden-a",
                sha256_pin: None,
                side_files: &[],
                generator: Generator::SchXslt,
            },
            Stylesheet {
                name: "B",
                xslt: "conf/b.xsl",
                golden_dir: "golden-b",
                sha256_pin: None,
                side_files: &[],
                generator: Generator::SchXslt,
            },
        ],
        document_dirs: &[("docs", "docs")],
        vefa_dirs: &[],
        xmute_dirs: &[],
        xmute_schematrons: &[],
        ignored: &[],
        mutation_catalogue: None,
        mutation_bases: &[],
        acceptance: Some(Acceptance {
            cases: &["docs"],
            profiles: Profiles::KositScenarios {
                file: "conf/scenarios.xml",
                bundle: "bundle",
            },
        }),
        record: RECORD,
    };

    /// The configuration of [`TABLE_SET`]: a scenario with levels, one without.
    const CONFIGURATION: &str = "<scenarios xmlns='http://www.xoev.de/de/validator/framework/1/scenarios'>\
        <scenario><name>One</name><namespace prefix='i'>urn:oasis:names:specification:ubl:schema:xsd:Invoice-2</namespace><namespace prefix='c'>urn:c</namespace>\
        <match>exists(/i:Invoice/c:CustomizationID[ . = 'urn:one'])</match>\
        <validateWithSchematron><resource><location>a.xsl</location></resource></validateWithSchematron>\
        <validateWithSchematron><resource><location>b.xsl</location></resource></validateWithSchematron>\
        <createReport><customLevel level='warning'>R1</customLevel><customLevel level='error'>R2 R3</customLevel>\
        <customLevel level='information'>R5</customLevel></createReport></scenario>\
        <scenario><name>Plain</name><namespace prefix='i'>urn:oasis:names:specification:ubl:schema:xsd:Invoice-2</namespace><namespace prefix='c'>urn:c</namespace>\
        <match>exists(/i:Invoice/c:CustomizationID[ . = 'urn:plain'])</match>\
        <validateWithSchematron><resource><location>a.xsl</location></resource></validateWithSchematron></scenario>\
        </scenarios>";

    /// Writes the configuration, the documents, their goldens and the index of
    /// [`TABLE_SET`] under `base`.
    fn write_fixture(base: &Path) {
        util::write(
            &util::join(base, "conf/scenarios.xml"),
            CONFIGURATION.as_bytes(),
        )
        .unwrap();
        let invoice = |id: &str| {
            format!(
                "<Invoice xmlns='urn:oasis:names:specification:ubl:schema:xsd:Invoice-2'><CustomizationID xmlns='urn:c'>{id}</CustomizationID></Invoice>"
            )
        };
        // (document, finding of A, finding of B or an error)
        let documents = [
            ("one.xml", invoice("urn:one"), "R1", Some("R2")),
            ("padded.xml", invoice(" urn:one"), "R1", Some("R4")),
            ("other.xml", invoice("urn:two"), "R4", None),
        ];
        let mut cases =
            String::from("case\tinput\tkind\tscope\tsuccess\terror\twarning\tdescription\n");
        let golden = |id: &str| {
            format!(
                "svrl-canonical 1\nschematron-output\nfailed-assert flag=\"warning\" id=\"{id}\" location=\"/\" test=\"t\"\n  text \"m\"\n"
            )
        };
        for (name, text, a, b) in &documents {
            util::write(
                &util::join(base, &format!("src/docs/{name}")),
                text.as_bytes(),
            )
            .unwrap();
            let _ = writeln!(cases, "docs/{name}\tsrc/docs/{name}\tdocument");
            util::write(
                &util::join(base, &format!("corpus/golden-a/docs/{name}.svrl.txt")),
                golden(a).as_bytes(),
            )
            .unwrap();
            match b {
                Some(id) => util::write(
                    &util::join(base, &format!("corpus/golden-b/docs/{name}.svrl.txt")),
                    golden(id).as_bytes(),
                ),
                None => util::write(
                    &util::join(base, &format!("corpus/golden-b/docs/{name}.error.txt")),
                    b"error FORG0001\n",
                ),
            }
            .unwrap();
        }
        util::write(&util::join(base, "corpus/cases.tsv"), cases.as_bytes()).unwrap();
        util::write(
            &util::join(base, "corpus/mutations.tsv"),
            b"case\tinput\tbase\treason\n",
        )
        .unwrap();
    }

    /// The table of a corpus with KoSIT scenarios: the levels of the scenario
    /// each document matches, stylesheet by stylesheet.
    #[test]
    fn the_table_follows_the_scenarios() {
        let root = std::env::temp_dir().join(format!("xtask-table-{}", std::process::id()));
        let base = root.join("base");
        write_fixture(&base);
        let summary = write_table(&root, &TABLE_SET).unwrap();
        let table = util::read_string(&util::join(&base, "corpus/acceptance.tsv")).unwrap();
        let parsed = scenarios(&base, "conf/scenarios.xml").unwrap();
        util::remove_dir(&root).unwrap();
        // R1 is a warning everywhere; R2 is one too, but the scenario makes it an
        // error; a document whose identifier differs, by a space included,
        // matches no scenario.
        assert_eq!(
            table,
            "case\tscenario\tgolden-a\tgolden-b\n\
             docs/one.xml\tOne\taccept\treject\n\
             docs/padded.xml\t-\taccept\taccept\n\
             docs/other.xml\t-\taccept\terror\n"
        );
        assert_eq!(
            summary,
            "acceptance written: synthetic-table: 3 cases, 1 matching a scenario; A rejects 0, B rejects 1"
        );
        let levels: Vec<(&str, &str)> = parsed[0]
            .levels
            .iter()
            .map(|(id, l)| (id.as_str(), l.as_str()))
            .collect();
        assert_eq!(
            levels,
            [
                ("R1", "warning"),
                ("R2", "error"),
                ("R3", "error"),
                ("R5", "information")
            ]
        );
    }

    /// The bundle of a rule set with KoSIT scenarios must give rule ids the
    /// levels of the configuration, and read unflagged failed asserts as fatal.
    #[test]
    fn bundles_have_the_levels_of_the_configuration() {
        let root = std::env::temp_dir().join(format!("xtask-bundle-{}", std::process::id()));
        let base = root.join("base");
        write_fixture(&base);
        let stylesheet = |name: &str| {
            format!(
                "\n[[stylesheet]]\nname = \"{name}\"\npath = \"{name}.xsl\"\nsha256 = \"0\"\nunflagged-assert = \"fatal\"\n"
            )
        };
        let good = format!(
            "format = 2\nname = \"x\"\n{}{}\n[[scenario]]\nname = \"One\"\nnamespace = \"urn:oasis:names:specification:ubl:schema:xsd:Invoice-2\"\nroot = \"Invoice\"\nidentifier = \"urn:one\"\nfatal = \"R2 R3\"\nwarning = \"R1\"\nother = \"R5\"\n",
            stylesheet("A"),
            stylesheet("B")
        );
        let check = |manifest: &str| {
            util::write(
                &util::join(&base, "bundle/bundle.toml"),
                manifest.as_bytes(),
            )
            .unwrap();
            check_bundle(&root, &TABLE_SET).map_err(|e| e.to_string())
        };
        let results = [
            check(&good),
            check(&good.replace("fatal = \"R2 R3\"", "fatal = \"R2\"")),
            check(&good.replace("other = \"R5\"\n", "")),
            check(&good.replacen("unflagged-assert = \"fatal\"\n", "", 1)),
            check(&format!("{good}\n[[scenario]]\nname = \"Plain\"\n")),
        ];
        util::remove_dir(&root).unwrap();
        assert_eq!(
            results[0],
            Ok(Some(
                "bundle checked: synthetic-table: bundle/bundle.toml, 1 scenarios with the levels of conf/scenarios.xml"
                    .to_owned()
            ))
        );
        for result in &results[1..] {
            assert!(result.is_err(), "{result:?}");
        }
        assert!(
            results[3]
                .as_ref()
                .unwrap_err()
                .contains("2 stylesheets, 1 with unflagged-assert = \"fatal\""),
            "{:?}",
            results[3]
        );
        // Another rule set: nothing to check.
        assert_eq!(
            check_bundle(&root, &crate::ruleset::PEPPOL_UBL).map_err(|e| e.to_string()),
            Ok(None)
        );
    }

    #[test]
    fn scenario_matches() {
        let namespaces: Vec<(String, String)> = [
            ("invoice", "urn:inv"),
            ("cbc", "urn:cbc"),
            ("rsm", "urn:rsm"),
            ("ram", "urn:ram"),
        ]
        .iter()
        .map(|(p, n)| ((*p).to_owned(), (*n).to_owned()))
        .collect();
        let q = |ns: &str, local: &str| (ns.to_owned(), local.to_owned());
        assert_eq!(
            parse_match(
                "exists(/invoice:Invoice/cbc:CustomizationID[ . = 'urn:cen.eu:en16931:2017#compliant#urn:xeinkauf.de:kosit:xrechnung_3.0']) ",
                &namespaces
            ),
            Some(Match {
                root: q("urn:inv", "Invoice"),
                path: vec![q("urn:cbc", "CustomizationID")],
                identifier: "urn:cen.eu:en16931:2017#compliant#urn:xeinkauf.de:kosit:xrechnung_3.0"
                    .to_owned()
            })
        );
        assert_eq!(
            parse_match(
                "exists(/invoice:CreditNote[ cbc:CustomizationID/text() = 'urn:x'])",
                &namespaces
            ),
            Some(Match {
                root: q("urn:inv", "CreditNote"),
                path: vec![q("urn:cbc", "CustomizationID")],
                identifier: "urn:x".to_owned()
            })
        );
        assert_eq!(
            parse_match(
                "exists(/rsm:CrossIndustryInvoice[rsm:ExchangedDocumentContext/ram:GuidelineSpecifiedDocumentContextParameter/ram:ID/text() = 'urn:cen.eu:en16931:2017'])",
                &namespaces
            ),
            Some(Match {
                root: q("urn:rsm", "CrossIndustryInvoice"),
                path: vec![
                    q("urn:rsm", "ExchangedDocumentContext"),
                    q("urn:ram", "GuidelineSpecifiedDocumentContextParameter"),
                    q("urn:ram", "ID")
                ],
                identifier: "urn:cen.eu:en16931:2017".to_owned()
            })
        );
        for bad in [
            "exists(/rsm:B[c = 'x' or c = 'y'])",
            "exists(/rsm:B[rsm:c = 'x' or rsm:c = 'y'])",
            "exists(/unknown:B[. = 'x'])",
            "exists(/rsm:B[. = 'x'])",
            "exists(/rsm:B/ram:C[. = ''])",
            "true()",
        ] {
            assert_eq!(parse_match(bad, &namespaces), None, "{bad}");
        }
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
        assert!(parse("<a").is_err());
    }
}
