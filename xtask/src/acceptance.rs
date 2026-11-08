//! What the reference validator of a rule set makes of the findings of its
//! stylesheets: which ones reject a document.
//!
//! For the Factur-X and French rule sets ([`Profiles::Listed`]), a finding
//! rejects when its level is `error`: a failed assert unless its flag is
//! `warning` or `information` (a failed assert without a flag included, as the
//! Mustang validator reads it), a successful report only with the flag
//! `fatal` or `error`.
//!
//! For the XRechnung rule sets ([`Profiles::KositScenarios`]), as the KoSIT
//! validator runs its configuration (`scenarios.xml`, the validator's
//! `ScenarioRepository`, `ComputeAcceptanceAction` and the report stylesheet
//! `default-report.xsl` the configuration's `xrechnung-report.xsl` imports):
//!
//! - a document must match exactly one scenario: the `match` expression of
//!   every scenario is evaluated on it, and when none is true, or more than
//!   one, the validator takes its fallback scenario, whose acceptance is
//!   always false: the document is rejected and no stylesheet runs on it;
//! - the scenario's `validateWithSchematron` stylesheets run, in order;
//! - the level of each failed assert and successful report is `error` when
//!   its flag or role is `fatal` or `error`, `warning` when one is `warning`
//!   or `warn`, `information` when one is `information` or `info`, `error`
//!   otherwise (no flag and no role included); then the scenario's
//!   `customLevel` elements give the rule ids they list (the `id`, or
//!   `UNSPECIFIC` without one) their level instead;
//! - the document is accepted when no finding has the level `error` (the
//!   scenario's `acceptMatch`, `/rep:report/rep:assessment[1]/rep:accept[1]`,
//!   on the report). A document with XML Schema errors is rejected too (the
//!   validator then skips the Schematron): the schema is not read here, the
//!   tests compare with the schema errors of the bundle themselves.
//!
//! The `match` expressions of the configuration have three forms, which this
//! module evaluates with their XPath meaning, on its own parser (not the
//! engine it checks): `exists(/p:Root/p:Child[ . = 'X'])` (an element at the
//! path whose string value is `X`), `exists(/p:Root[p:A/p:B/text() = 'X'])`
//! (a root element with a text node child of an element at the relative path
//! equal to `X`) and `exists(/p:Root[p:A = 'X'])` (the string value of an
//! element at the relative path); any other form is refused.
//!
//! Two uses:
//!
//! - [`run`]: documents the reference pipeline of a rule set accepts
//!   (published examples, test-suite instances): for each, the stylesheets of
//!   its profile must report nothing that rejects it
//!   ([`crate::ruleset::Acceptance`]). With [`Profiles::Listed`], the profile is
//!   chosen by the specification identifier the document declares (BT-24):
//!   `cbc:CustomizationID` under the root of a UBL document,
//!   `ExchangedDocumentContext/GuidelineSpecifiedDocumentContextParameter/ID`
//!   in CII and CDAR; with [`Profiles::KositScenarios`], by the scenario the
//!   document matches.
//! - [`write_table`]: the reference acceptance of every case of a fetched
//!   corpus, stylesheet by stylesheet and as a whole ([`TABLE`]), which the
//!   tests of `einvoice-validate --rules` compare the verdicts of the bundles
//!   with; and [`check_bundle`]: the scenarios of the XRechnung bundles are the
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
    /// Its `namespace` elements: prefix and URI, in order.
    namespaces: Vec<(String, String)>,
    /// The text of its `match` element, as written.
    match_text: String,
    /// What its `match` requires.
    matching: Match,
    /// The stylesheets, by their location relative to the configuration.
    locations: Vec<String>,
    /// The XML schema of its `validateWithXmlSchema`, by its location relative
    /// to the configuration.
    schema: String,
    /// (rule id, level).
    levels: Vec<(String, String)>,
}

impl Scenario {
    /// Whether the document whose root element is `root` satisfies the
    /// `match` of the scenario.
    fn matches(&self, root: Node<'_, '_>) -> bool {
        self.matching.holds(root)
    }

    /// The level of the rule `id` in the documents the scenario matches.
    fn level(&self, id: &str) -> Option<&str> {
        self.levels
            .iter()
            .find(|(rule, _)| rule == id)
            .map(|(_, level)| level.as_str())
    }
}

/// The scenario of a configuration a document has.
#[derive(Debug)]
enum Selected<'s> {
    /// The one scenario whose `match` is true.
    One(&'s Scenario),
    /// None is: the fallback scenario, rejected.
    None,
    /// Several are: the fallback scenario, rejected.
    Several(Vec<&'s Scenario>),
}

impl<'s> Selected<'s> {
    /// The scenarios of `scenarios` whose `match` is true on the document
    /// whose root element is `root`: one is required.
    fn of(scenarios: &'s [Scenario], root: Node<'_, '_>) -> Self {
        let matching: Vec<&Scenario> = scenarios.iter().filter(|s| s.matches(root)).collect();
        match matching[..] {
            [one] => Self::One(one),
            [] => Self::None,
            _ => Self::Several(matching),
        }
    }

    const fn scenario(&self) -> Option<&'s Scenario> {
        match self {
            Self::One(s) => Some(s),
            Self::None | Self::Several(_) => None,
        }
    }

    /// The scenario column of [`TABLE`]: its name, `-` for none, `several: `
    /// and their names.
    fn column(&self) -> String {
        match self {
            Self::One(s) => s.name.clone(),
            Self::None => "-".to_owned(),
            Self::Several(all) => format!(
                "several: {}",
                all.iter()
                    .map(|s| s.name.as_str())
                    .collect::<Vec<_>>()
                    .join(" | ")
            ),
        }
    }
}

/// A finding of a golden: kind, rule id, flag and role.
type Found = (&'static str, Option<String>, Option<String>, Option<String>);

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
        outcome.cases += 1;
        let Some((stylesheets, kosit)) = profile(rule_set, case, &text, scenarios.as_ref())? else {
            // No scenario, or several: the fallback scenario rejects it.
            outcome.disagreements.push(Disagreement {
                case: case.id.clone(),
                kind: "rejected",
                rule: "no-scenario".to_owned(),
                saxon: "-".to_owned(),
            });
            continue;
        };
        let mut rejected = false;
        for stylesheet in stylesheets {
            for (kind, id, flag, role) in findings(&base, rule_set, stylesheet, case)? {
                let level = match kosit {
                    Some(scenario) => kosit_level(
                        id.as_deref(),
                        flag.as_deref(),
                        role.as_deref(),
                        Some(scenario),
                    ),
                    None => default_level(kind, flag.as_deref()),
                };
                if level == "error" {
                    rejected = true;
                    let disagreement = Disagreement {
                        case: case.id.clone(),
                        kind: "rejected",
                        rule: id.unwrap_or_else(|| "none".to_owned()),
                        saxon: format!("{}@{}", flag.as_deref().unwrap_or("none"), stylesheet.name),
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

/// The stylesheets of the profile of a document (its text), and for a rule
/// set with KoSIT scenarios the scenario it matches; `None` when it matches
/// none of them, or several.
fn profile<'r, 's>(
    rule_set: &'r RuleSet,
    case: &Case,
    text: &str,
    scenarios: Option<&'s (&'static str, Vec<Scenario>)>,
) -> Result<Option<(Vec<&'r Stylesheet>, Option<&'s Scenario>)>> {
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
            Some((stylesheets, None))
        }
        (Profiles::KositScenarios { .. }, Some((file, scenarios))) => {
            let doc = parse(text).map_err(|e| Error::new(format!("{}: {e}", case.id)))?;
            match Selected::of(scenarios, doc.root_element()).scenario() {
                Some(scenario) => Some((stylesheets_of(rule_set, scenario, file)?, Some(scenario))),
                None => None,
            }
        }
        (Profiles::KositScenarios { .. }, None) => {
            return Err(Error::new("scenarios not read"));
        }
    })
}

/// The stylesheets of `rule_set` that a scenario of the configuration `file`
/// runs, in its order.
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

/// The kind, rule id, flag and role of every failed assert and successful
/// report of the golden of a case for one stylesheet.
fn findings(
    base: &Path,
    rule_set: &RuleSet,
    stylesheet: &Stylesheet,
    case: &Case,
) -> Result<Vec<Found>> {
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
                l.attribute("flag").map(str::to_owned),
                l.attribute("role").map(str::to_owned),
            ))
        })
        .collect())
}

/// The level the KoSIT validator gives a finding in the documents a scenario
/// matches: the scenario's `customLevel` for its code (its id, `UNSPECIFIC`
/// without one), else the level of its flag and role, as `default-report.xsl`
/// reads them (failed asserts and successful reports alike). Without a
/// scenario (the stylesheet columns of [`TABLE`] for a document without one),
/// the level of the flag and role.
fn kosit_level<'a>(
    id: Option<&str>,
    flag: Option<&str>,
    role: Option<&str>,
    scenario: Option<&'a Scenario>,
) -> &'a str {
    if let Some(level) = scenario.and_then(|s| s.level(id.unwrap_or("UNSPECIFIC"))) {
        return level;
    }
    // `(@flag, @role) = (...)`: either of them is one of the values.
    let any = |values: &[&str]| {
        [flag, role]
            .into_iter()
            .flatten()
            .any(|v| values.contains(&v))
    };
    if any(&["fatal", "error"]) {
        "error"
    } else if any(&["warning", "warn"]) {
        "warning"
    } else if any(&["information", "info"]) {
        "information"
    } else {
        "error"
    }
}

/// The level of a finding for the Mustang validator and the French rules: a
/// failed assert is an error unless its flag is `warning` or `information` (a
/// failed assert without a flag included); a successful report is an error
/// only with the flag `fatal` or `error`.
fn default_level(kind: &str, flag: Option<&str>) -> &'static str {
    let error = if kind == "failed-assert" {
        !matches!(flag, Some("warning" | "information"))
    } else {
        matches!(flag, Some("fatal" | "error"))
    };
    if error { "error" } else { "other" }
}

/// Writes [`TABLE`] in the corpus directory of a fetched rule set: for every
/// case (whole documents, test cases, mutations), the scenario it matches
/// ([`Selected::column`]; `-` for a rule set without scenarios), then for each
/// stylesheet (a column named after its golden directory) `reject` when one of
/// its findings has the level `error`, `accept` when none has, `error` when
/// Saxon-HE stopped on the case, whether the scenario runs the stylesheet or
/// not; then `verdict`, the acceptance of the reference validator for a
/// schema-valid document, for a rule set with KoSIT scenarios: `reject` for a
/// document without a scenario (none or several), else `error` when Saxon-HE
/// stopped with one of the scenario's stylesheets, `reject` when one of them
/// rejects it, `accept` otherwise (`-` for the other rule sets, whose bundles
/// each run some of the stylesheets). Returns a summary line.
pub(crate) fn write_table(root: &Path, rule_set: &RuleSet) -> Result<String> {
    let base = rule_set.base(root);
    let cases = corpus::read_all(root, rule_set)?;
    let kosit = match rule_set.acceptance.as_ref().map(|a| &a.profiles) {
        Some(Profiles::KositScenarios { file, .. }) => Some((*file, scenarios(&base, file)?)),
        _ => None,
    };
    let mut table = String::from("case\tscenario");
    for stylesheet in rule_set.stylesheets {
        let _ = write!(table, "\t{}", stylesheet.golden_dir);
    }
    table.push_str("\tverdict\n");
    let mut rejected = vec![0usize; rule_set.stylesheets.len()];
    let (mut matched, mut unmatched, mut several) = (0, 0, 0);
    let (mut accepts, mut rejects, mut errors) = (0, 0, 0);
    for case in &cases {
        let text = util::read_string(&util::join(&base, &case.input))?;
        let doc = parse(&text).ok();
        let selected = match (&kosit, &doc) {
            (Some((_, scenarios)), Some(doc)) => Some(Selected::of(scenarios, doc.root_element())),
            (Some(_), None) => Some(Selected::None),
            (None, _) => None,
        };
        let scenario = selected.as_ref().and_then(Selected::scenario);
        match &selected {
            Some(Selected::One(_)) => matched += 1,
            Some(Selected::None) => unmatched += 1,
            Some(Selected::Several(_)) => several += 1,
            None => {}
        }
        let _ = write!(
            table,
            "{}\t{}",
            case.id,
            selected
                .as_ref()
                .map_or_else(|| "-".to_owned(), Selected::column)
        );
        let mut verdicts = Vec::with_capacity(rule_set.stylesheets.len());
        for (stylesheet, count) in rule_set.stylesheets.iter().zip(&mut rejected) {
            let verdict =
                stylesheet_verdict(&base, rule_set, stylesheet, case, kosit.is_some(), scenario)?;
            *count += usize::from(verdict == "reject");
            let _ = write!(table, "\t{verdict}");
            verdicts.push((stylesheet.name, verdict));
        }
        let verdict = match (&kosit, scenario) {
            (None, _) => "-",
            (Some(_), None) => "reject",
            (Some((file, _)), Some(scenario)) => {
                let runs = stylesheets_of(rule_set, scenario, file)?;
                let of = |s: &Stylesheet| {
                    verdicts
                        .iter()
                        .find(|(name, _)| *name == s.name)
                        .map_or("", |(_, v)| *v)
                };
                if runs.iter().any(|s| of(s) == "error") {
                    "error"
                } else if runs.iter().any(|s| of(s) == "reject") {
                    "reject"
                } else {
                    "accept"
                }
            }
        };
        match verdict {
            "accept" => accepts += 1,
            "reject" => rejects += 1,
            "error" => errors += 1,
            _ => {}
        }
        let _ = writeln!(table, "\t{verdict}");
    }
    let corpus_dir = util::join(&base, &rule_set.corpus_dir());
    util::write(&corpus_dir.join(TABLE), table.as_bytes())?;
    let rejections: Vec<String> = rule_set
        .stylesheets
        .iter()
        .zip(&rejected)
        .map(|(s, n)| format!("{} rejects {n}", s.name))
        .collect();
    let scenarios = if kosit.is_some() {
        format!(
            "{matched} matching one scenario, {unmatched} none, {several} several; the configuration accepts {accepts}, rejects {rejects}, stops on {errors}; "
        )
    } else {
        String::new()
    };
    Ok(format!(
        "acceptance written: {}: {} cases, {scenarios}{}",
        rule_set.name,
        cases.len(),
        rejections.join(", ")
    ))
}

/// The stylesheet column of [`TABLE`] for one case: `error` when Saxon-HE
/// stopped, `reject` when a finding has the level `error` (KoSIT's levels, with
/// those of `scenario`, for a rule set with KoSIT scenarios; otherwise
/// [`default_level`]), `accept` otherwise.
fn stylesheet_verdict(
    base: &Path,
    rule_set: &RuleSet,
    stylesheet: &Stylesheet,
    case: &Case,
    kosit: bool,
    scenario: Option<&Scenario>,
) -> Result<&'static str> {
    if util::join(base, &case.error_golden(rule_set, stylesheet)).is_file() {
        return Ok("error");
    }
    let rejects =
        findings(base, rule_set, stylesheet, case)?
            .iter()
            .any(|(kind, id, flag, role)| {
                let level = if kosit {
                    kosit_level(id.as_deref(), flag.as_deref(), role.as_deref(), scenario)
                } else {
                    default_level(kind, flag.as_deref())
                };
                level == "error"
            });
    Ok(if rejects { "reject" } else { "accept" })
}

/// Checks that the bundle the fetch script wrote for a rule set with KoSIT
/// scenarios has the scenarios of the configuration: its `[[scenario]]`
/// tables, after its stylesheets, are those of the scenarios of the
/// configuration that run its stylesheets, in their order, each with its
/// namespaces, its `match` as written, the bundle names of its stylesheets in
/// their order, and its `customLevel` rule ids (`error` written `fatal`,
/// `information` `other`); that the XML schema of each scenario is the one the
/// bundle runs (`ubl-2.1`: the UBL 2.1 schema of its root element; `cii-d16b`:
/// the CII D16B entry point, whose documents in the configuration are, byte for
/// byte, those `einvoice-xsd` embeds, [`CII_D16B_DIR`]) or the one it names
/// unchecked; and that its stylesheets read an unflagged failed assert as
/// fatal. Returns a summary line; nothing for another rule set.
/// The CII D16B schema of the CII scenarios of the KoSIT configuration, relative to
/// its directory.
const CII_D16B_LOCATION: &str = "resources/cii/16b/xsd/CrossIndustryInvoice_100pD16B.xsd";

/// The CII D16B schema documents `einvoice-xsd` embeds (the schema `cii-d16b`),
/// relative to the repository root, listed in its `SHA256SUMS`.
const CII_D16B_DIR: &str = "crates/einvoice-xsd/schemas/uncefact-cii-d16b";

/// Checks that the directory of the CII D16B entry point in the KoSIT configuration
/// `configuration` holds exactly the schema documents of [`CII_D16B_DIR`], byte for
/// byte: the bundle's `cii-d16b` is then the schema the configuration validates with.
fn same_cii_schemas(root: &Path, configuration: &Path) -> Result<()> {
    let ours_dir = util::join(root, CII_D16B_DIR);
    let sums = util::read_string(&ours_dir.join("SHA256SUMS"))?;
    let theirs_dir = util::join(
        configuration,
        CII_D16B_LOCATION
            .rsplit_once('/')
            .map_or("", |(dir, _)| dir),
    );
    let mut expected = Vec::new();
    for line in sums.lines() {
        let (sha, path) = line
            .split_once("  ")
            .ok_or_else(|| Error::new(format!("{CII_D16B_DIR}/SHA256SUMS: bad line {line:?}")))?;
        let ours = util::read(&util::join(&ours_dir, path))?;
        if util::sha256_hex(&ours) != sha {
            return Err(Error::new(format!(
                "{CII_D16B_DIR}/{path}: not its SHA256SUMS digest"
            )));
        }
        let name = path.rsplit('/').next().unwrap_or(path);
        let theirs = util::read(&theirs_dir.join(name))?;
        if theirs != ours {
            return Err(Error::new(format!(
                "{}: differs from {CII_D16B_DIR}/{path}: the configuration's CII D16B schema is not the one cii-d16b embeds",
                theirs_dir.join(name).display()
            )));
        }
        expected.push(name.to_owned());
    }
    expected.sort();
    let found = util::list_files(&theirs_dir)?;
    if found != expected {
        return Err(Error::new(format!(
            "{}: holds {found:?}, expected the CII D16B documents {expected:?}",
            theirs_dir.display()
        )));
    }
    Ok(())
}

pub(crate) fn check_bundle(root: &Path, rule_set: &RuleSet) -> Result<Option<String>> {
    let Some(Profiles::KositScenarios { file, bundle }) =
        rule_set.acceptance.as_ref().map(|a| &a.profiles)
    else {
        return Ok(None);
    };
    let base = rule_set.base(root);
    let path = format!("{bundle}/bundle.toml");
    let manifest = util::read_string(&util::join(&base, &path))?;
    let runs_ubl = manifest.contains("\nschema = \"ubl-2.1\"\n");
    let runs_cii = manifest.contains("\nschema = \"cii-d16b\"\n");
    let configuration = file.rsplit_once('/').map_or("", |(dir, _)| dir);
    // `root` names the root element of a scenario below.
    let repo = root;
    let unchecked = manifest
        .split('\n')
        .find_map(|l| l.strip_prefix("unchecked-schema = \""))
        .and_then(|l| l.strip_suffix('"'));
    let mut expected = String::new();
    let mut count = 0;
    for scenario in scenarios(&base, file)? {
        let Ok(stylesheets) = stylesheets_of(rule_set, &scenario, file) else {
            continue;
        };
        let (_, root) = scenario
            .matching
            .context
            .first()
            .ok_or_else(|| Error::new(format!("scenario {:?}: no root", scenario.name)))?;
        let schema_file = scenario
            .schema
            .rsplit('/')
            .next()
            .unwrap_or(&scenario.schema);
        let ubl = scenario.schema == format!("resources/ubl/2.1/xsd/maindoc/UBL-{root}-2.1.xsd");
        let cii = scenario.schema == CII_D16B_LOCATION;
        if runs_cii && cii {
            same_cii_schemas(repo, &util::join(&base, configuration))?;
        }
        let listed = unchecked.is_some_and(|u| u.contains(schema_file));
        if !(listed || runs_ubl && ubl || runs_cii && cii) {
            return Err(Error::new(format!(
                "{path}: scenario {:?} validates against {}, which the bundle neither runs (schema = \"ubl-2.1\" for a UBL root, \"cii-d16b\" for {CII_D16B_LOCATION}) nor names (unchecked-schema)",
                scenario.name, scenario.schema
            )));
        }
        let namespaces: Vec<String> = scenario
            .namespaces
            .iter()
            .map(|(p, u)| format!("{p}={u}"))
            .collect();
        let names: Vec<&str> = stylesheets.iter().map(|s| s.name).collect();
        let _ = write!(
            expected,
            "\n[[scenario]]\nname = \"{}\"\nnamespaces = \"{}\"\nmatch = \"{}\"\nstylesheets = \"{}\"\n",
            scenario.name,
            namespaces.join(" "),
            scenario.match_text,
            names.join(" ")
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
    let actual = manifest
        .find("\n[[scenario]]")
        .map_or("", |start| &manifest[start..]);
    if actual != expected {
        return Err(Error::new(format!(
            "{path}: its [[scenario]] tables are not the scenarios of {file}: expected\n{expected}\nfound\n{actual}"
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
        "bundle checked: {}: {path}, the {count} scenarios of {file} that run its stylesheets",
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

/// The text of an element: its text children, as written.
fn text_of(node: Node<'_, '_>) -> String {
    node.children().filter_map(|c| c.text()).collect()
}

/// The text of the one element `local` under `node`, trimmed.
fn one(node: Node<'_, '_>, local: &str) -> Option<String> {
    match elements(node, local)[..] {
        [element] => Some(text_of(element).trim().to_owned()),
        _ => None,
    }
}

/// The scenarios of `file` (relative to `base`).
fn scenarios(base: &Path, file: &str) -> Result<Vec<Scenario>> {
    let text = util::read_string(&util::join(base, file))?;
    let doc = parse(&text).map_err(|e| Error::new(format!("{file}: {e}")))?;
    let mut out = Vec::new();
    for scenario in elements(doc.root_element(), "scenario") {
        let name = elements(scenario, "name")
            .first()
            .map(|n| text_of(*n).trim().to_owned())
            .unwrap_or_default();
        let fail = |why: &str| Error::new(format!("{file}: scenario {name:?}: {why}"));
        let mut namespaces = Vec::new();
        for namespace in elements(scenario, "namespace") {
            let prefix = namespace
                .attribute("prefix")
                .ok_or_else(|| fail("a namespace without prefix"))?;
            let uri = text_of(namespace);
            if uri.is_empty() || uri.contains(char::is_whitespace) {
                return Err(fail(&format!("namespace {prefix}: {uri:?}")));
            }
            namespaces.push((prefix.to_owned(), uri));
        }
        let [matching] = elements(scenario, "match")[..] else {
            return Err(fail("not exactly one match"));
        };
        let match_text = text_of(matching);
        let matching = parse_match(&match_text, &namespaces).ok_or_else(|| {
            fail("match is not exists(/p:Root[/steps][[steps[/text()] | .] = '<identifier>'])")
        })?;
        let mut locations = Vec::new();
        for validate in elements(scenario, "validateWithSchematron") {
            let location = one(validate, "location")
                .ok_or_else(|| fail("a validateWithSchematron without exactly one location"))?;
            locations.push(location);
        }
        if locations.is_empty() {
            return Err(fail("no validateWithSchematron"));
        }
        let schema = match elements(scenario, "validateWithXmlSchema")[..] {
            [validate] => one(validate, "location"),
            _ => None,
        }
        .ok_or_else(|| fail("not exactly one validateWithXmlSchema with one location"))?;
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
            namespaces,
            match_text,
            matching,
            locations,
            schema,
            levels,
        });
    }
    if out.is_empty() {
        return Err(Error::new(format!("{file}: no scenario")));
    }
    Ok(out)
}

/// What the `match` of a scenario requires: an element at the path
/// `context` from the document node (the root element first), at the
/// relative path `relative` from which is an element whose string value (or,
/// with `text_nodes`, one of whose text node children) is `identifier`.
#[derive(Debug, PartialEq, Eq)]
struct Match {
    context: Vec<(String, String)>,
    relative: Vec<(String, String)>,
    text_nodes: bool,
    identifier: String,
}

impl Match {
    /// Whether it holds on the document whose root element is `root`.
    fn holds(&self, root: Node<'_, '_>) -> bool {
        let Some((first, rest)) = self.context.split_first() else {
            return false;
        };
        if !is_named(root, first) {
            return false;
        }
        let context = rest
            .iter()
            .fold(vec![root], |nodes, step| child_step(&nodes, step));
        let mut values = context.into_iter().flat_map(|n| {
            self.relative
                .iter()
                .fold(vec![n], |nodes, step| child_step(&nodes, step))
        });
        if self.text_nodes {
            values.any(|v| {
                v.children()
                    .any(|c| c.is_text() && c.text() == Some(self.identifier.as_str()))
            })
        } else {
            values.any(|v| string_value(v) == self.identifier)
        }
    }
}

/// Whether a node is an element with that namespace and local name.
fn is_named(node: Node<'_, '_>, (namespace, local): &(String, String)) -> bool {
    node.is_element()
        && node.tag_name().namespace().unwrap_or("") == namespace
        && node.tag_name().name() == local
}

/// The child elements of `nodes` named `step`, in document order.
fn child_step<'a, 'input>(
    nodes: &[Node<'a, 'input>],
    step: &(String, String),
) -> Vec<Node<'a, 'input>> {
    nodes
        .iter()
        .flat_map(|n| n.children().filter(|c| is_named(*c, step)))
        .collect()
}

/// The string value of an element: its descendant text, in document order.
fn string_value(node: Node<'_, '_>) -> String {
    node.descendants()
        .filter(Node::is_text)
        .filter_map(|n| n.text())
        .collect()
}

/// The forms of the configuration's matches:
/// `exists(/invoice:Invoice/cbc:CustomizationID[ . = 'X'])`,
/// `exists(/creditnote:CreditNote[ cbc:CustomizationID/text() = 'X'])`,
/// `exists(/rsm:CrossIndustryInvoice[rsm:A/ram:B/ram:ID/text() = 'X'])`, and
/// `exists(/p:R[p:A = 'X'])`: a path from the root, a predicate comparing `.`,
/// a relative path or its text nodes with one quoted literal, the prefixes
/// resolved with `namespaces`.
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
    let context: Vec<(String, String)> = steps.split('/').map(qname).collect::<Option<_>>()?;
    let left = left.trim();
    let (relative, text_nodes) = if left == "." {
        (Vec::new(), false)
    } else {
        let (path, text_nodes) = match left.strip_suffix("/text()") {
            Some(path) => (path, true),
            None => (left, false),
        };
        let relative: Vec<(String, String)> = path.split('/').map(qname).collect::<Option<_>>()?;
        (relative, text_nodes)
    };
    // `/p:Root[. = 'X']`: the string value of the whole document, which no
    // scenario tests.
    if context.len() + relative.len() < 2 {
        return None;
    }
    Some(Match {
        context,
        relative,
        text_nodes,
        identifier: identifier.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;
    use std::path::Path;

    use super::{
        Match, Selected, check_bundle, identify, kosit_level, parse, parse_match, run, scenarios,
        write_table,
    };
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

    /// The configuration of [`TABLE_SET`]: a scenario of both stylesheets with
    /// levels, one of `A` alone without (a credit note, matched by its text
    /// node), and a third that matches what the first matches when the
    /// document has a second identifier.
    const CONFIGURATION: &str = "<scenarios xmlns='http://www.xoev.de/de/validator/framework/1/scenarios'>\
        <scenario><name>One</name><namespace prefix='i'>urn:oasis:names:specification:ubl:schema:xsd:Invoice-2</namespace><namespace prefix='c'>urn:c</namespace>\
        <match>exists(/i:Invoice/c:CustomizationID[ . = 'urn:one']) </match>\
        <validateWithXmlSchema><resource><location>resources/ubl/2.1/xsd/maindoc/UBL-Invoice-2.1.xsd</location></resource></validateWithXmlSchema>\
        <validateWithSchematron><resource><location>a.xsl</location></resource></validateWithSchematron>\
        <validateWithSchematron><resource><location>b.xsl</location></resource></validateWithSchematron>\
        <createReport><customLevel level='warning'>R1</customLevel><customLevel level='error'>R2 R3</customLevel>\
        <customLevel level='information'>R5</customLevel></createReport></scenario>\
        <scenario><name>Plain</name><namespace prefix='n'>urn:oasis:names:specification:ubl:schema:xsd:CreditNote-2</namespace><namespace prefix='c'>urn:c</namespace>\
        <match>exists(/n:CreditNote[c:CustomizationID/text() = 'urn:plain'])</match>\
        <validateWithXmlSchema><resource><location>resources/ubl/2.1/xsd/maindoc/UBL-CreditNote-2.1.xsd</location></resource></validateWithXmlSchema>\
        <validateWithSchematron><resource><location>a.xsl</location></resource></validateWithSchematron></scenario>\
        <scenario><name>Other</name><namespace prefix='i'>urn:oasis:names:specification:ubl:schema:xsd:Invoice-2</namespace><namespace prefix='c'>urn:c</namespace>\
        <match>exists(/i:Invoice[c:CustomizationID/text() = 'urn:other'])</match>\
        <validateWithXmlSchema><resource><location>resources/ubl/2.1/xsd/maindoc/UBL-Invoice-2.1.xsd</location></resource></validateWithXmlSchema>\
        <validateWithSchematron><resource><location>b.xsl</location></resource></validateWithSchematron></scenario>\
        </scenarios>";

    /// Writes the configuration, the documents, their goldens and the index of
    /// [`TABLE_SET`] under `base`.
    fn write_fixture(base: &Path) {
        util::write(
            &util::join(base, "conf/scenarios.xml"),
            CONFIGURATION.as_bytes(),
        )
        .unwrap();
        let invoice = |ids: &str| {
            format!(
                "<Invoice xmlns='urn:oasis:names:specification:ubl:schema:xsd:Invoice-2' xmlns:c='urn:c'>{ids}</Invoice>"
            )
        };
        let id = |id: &str| format!("<c:CustomizationID>{id}</c:CustomizationID>");
        let credit_note = |content: &str| {
            format!(
                "<CreditNote xmlns='urn:oasis:names:specification:ubl:schema:xsd:CreditNote-2' xmlns:c='urn:c'><c:CustomizationID>{content}</c:CustomizationID></CreditNote>"
            )
        };
        // (document, finding of A, finding of B or an error)
        let documents = [
            ("one.xml", invoice(&id("urn:one")), "R1", Some("R2")),
            ("padded.xml", invoice(&id(" urn:one")), "R1", Some("R4")),
            ("other.xml", invoice(&id("urn:two")), "R4", None),
            // The text node is the identifier; a comment splits it in two.
            ("plain.xml", credit_note("urn:plain"), "R1", Some("R2")),
            (
                "split.xml",
                credit_note("urn:<!-- -->plain"),
                "R1",
                Some("R2"),
            ),
            // Two identifiers: two scenarios match.
            (
                "both.xml",
                invoice(&format!("{}{}", id("urn:one"), id("urn:other"))),
                "R1",
                Some("R1"),
            ),
            ("stops.xml", invoice(&id("urn:one")), "R1", None),
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
    /// each document matches, stylesheet by stylesheet, and the verdict of the
    /// configuration on the stylesheets of that scenario.
    #[test]
    fn the_table_follows_the_scenarios() {
        let root = std::env::temp_dir().join(format!("xtask-table-{}", std::process::id()));
        let base = root.join("base");
        write_fixture(&base);
        let summary = write_table(&root, &TABLE_SET).unwrap();
        let table = util::read_string(&util::join(&base, "corpus/acceptance.tsv")).unwrap();
        let parsed = scenarios(&base, "conf/scenarios.xml").unwrap();
        util::remove_dir(&root).unwrap();
        // R1 is a warning everywhere; R2 is one too, but the scenario One
        // makes it an error. A document whose identifier differs, by a space
        // included, or is split by a comment, matches no scenario: rejected,
        // whatever its findings. Plain runs A alone, whose findings are
        // warnings: accepted though B would reject it. Two scenarios: none.
        // B stopped on stops.xml, which One runs it on.
        assert_eq!(
            table,
            "case\tscenario\tgolden-a\tgolden-b\tverdict\n\
             docs/one.xml\tOne\taccept\treject\treject\n\
             docs/padded.xml\t-\taccept\taccept\treject\n\
             docs/other.xml\t-\taccept\terror\treject\n\
             docs/plain.xml\tPlain\taccept\taccept\taccept\n\
             docs/split.xml\t-\taccept\taccept\treject\n\
             docs/both.xml\tseveral: One | Other\taccept\taccept\treject\n\
             docs/stops.xml\tOne\taccept\terror\terror\n"
        );
        assert_eq!(
            summary,
            "acceptance written: synthetic-table: 7 cases, 3 matching one scenario, 3 none, 1 several; the configuration accepts 1, rejects 5, stops on 1; A rejects 0, B rejects 1"
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
        assert_eq!(
            parsed[0].match_text,
            "exists(/i:Invoice/c:CustomizationID[ . = 'urn:one']) "
        );
        assert_eq!(
            parsed[1].schema,
            "resources/ubl/2.1/xsd/maindoc/UBL-CreditNote-2.1.xsd"
        );
    }

    /// The bundle of a rule set with KoSIT scenarios must have the scenarios of
    /// the configuration, with the schemas it runs or names, and read unflagged
    /// failed asserts as fatal.
    #[test]
    fn bundles_have_the_scenarios_of_the_configuration() {
        let root = std::env::temp_dir().join(format!("xtask-bundle-{}", std::process::id()));
        let base = root.join("base");
        write_fixture(&base);
        let stylesheet = |name: &str| {
            format!(
                "\n[[stylesheet]]\nname = \"{name}\"\npath = \"{name}.xsl\"\nsha256 = \"0\"\nunflagged-assert = \"fatal\"\n"
            )
        };
        let invoice = "i=urn:oasis:names:specification:ubl:schema:xsd:Invoice-2 c=urn:c";
        let credit_note = "n=urn:oasis:names:specification:ubl:schema:xsd:CreditNote-2 c=urn:c";
        let good = format!(
            "format = 2\nname = \"x\"\nschema = \"ubl-2.1\"\n{}{}\n[[scenario]]\nname = \"One\"\nnamespaces = \"{invoice}\"\nmatch = \"exists(/i:Invoice/c:CustomizationID[ . = 'urn:one']) \"\nstylesheets = \"A B\"\nfatal = \"R2 R3\"\nwarning = \"R1\"\nother = \"R5\"\n\n[[scenario]]\nname = \"Plain\"\nnamespaces = \"{credit_note}\"\nmatch = \"exists(/n:CreditNote[c:CustomizationID/text() = 'urn:plain'])\"\nstylesheets = \"A\"\n\n[[scenario]]\nname = \"Other\"\nnamespaces = \"{invoice}\"\nmatch = \"exists(/i:Invoice[c:CustomizationID/text() = 'urn:other'])\"\nstylesheets = \"B\"\n",
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
        let unchecked = good.replace(
            "schema = \"ubl-2.1\"",
            "unchecked-schema = \"UBL (UBL-Invoice-2.1.xsd, UBL-CreditNote-2.1.xsd)\"",
        );
        let results = [
            check(&good),
            check(&unchecked),
            check(&good.replace("fatal = \"R2 R3\"", "fatal = \"R2\"")),
            check(&good.replace("other = \"R5\"\n", "")),
            check(&good.replacen("unflagged-assert = \"fatal\"\n", "", 1)),
            check(&good.replace("']) \"", "'])\"")),
            check(&good.replace("stylesheets = \"A B\"", "stylesheets = \"B A\"")),
            check(&good.replace("schema = \"ubl-2.1\"\n", "")),
            check(&unchecked.replace(", UBL-CreditNote-2.1.xsd", "")),
            check(&good.replace(
                " c=urn:c\"\nmatch = \"exists(/n:",
                "\"\nmatch = \"exists(/n:",
            )),
        ];
        util::remove_dir(&root).unwrap();
        let ok = Ok(Some(
            "bundle checked: synthetic-table: bundle/bundle.toml, the 3 scenarios of conf/scenarios.xml that run its stylesheets"
                .to_owned(),
        ));
        assert_eq!(results[0], ok);
        assert_eq!(results[1], ok);
        for result in &results[2..] {
            assert!(result.is_err(), "{result:?}");
        }
        assert!(
            results[4]
                .as_ref()
                .unwrap_err()
                .contains("2 stylesheets, 1 with unflagged-assert = \"fatal\""),
            "{:?}",
            results[4]
        );
        for result in [&results[7], &results[8]] {
            assert!(
                result
                    .as_ref()
                    .unwrap_err()
                    .contains("which the bundle neither runs"),
                "{result:?}"
            );
        }
        // Another rule set: nothing to check.
        assert_eq!(
            check_bundle(&root, &crate::ruleset::PEPPOL_UBL).map_err(|e| e.to_string()),
            Ok(None)
        );
    }

    /// The levels of `default-report.xsl`: flag or role, then the scenario's.
    #[test]
    fn kosit_levels() {
        let root = std::env::temp_dir().join(format!("xtask-levels-{}", std::process::id()));
        let base = root.join("base");
        write_fixture(&base);
        let parsed = scenarios(&base, "conf/scenarios.xml").unwrap();
        util::remove_dir(&root).unwrap();
        let one = Some(&parsed[0]);
        let level = |id, flag, role| kosit_level(id, flag, role, None);
        assert_eq!(level(Some("X"), Some("fatal"), None), "error");
        assert_eq!(level(Some("X"), Some("error"), None), "error");
        assert_eq!(level(Some("X"), Some("warning"), None), "warning");
        assert_eq!(level(Some("X"), Some("warn"), None), "warning");
        assert_eq!(level(Some("X"), Some("information"), None), "information");
        assert_eq!(level(Some("X"), Some("info"), None), "information");
        assert_eq!(level(Some("X"), None, None), "error");
        assert_eq!(level(Some("X"), Some("other"), None), "error");
        assert_eq!(level(Some("X"), None, Some("warn")), "warning");
        assert_eq!(level(Some("X"), Some("warning"), Some("error")), "error");
        assert_eq!(level(None, Some("info"), Some("warning")), "warning");
        // The scenario's levels first.
        assert_eq!(kosit_level(Some("R1"), Some("fatal"), None, one), "warning");
        assert_eq!(kosit_level(Some("R5"), None, None, one), "information");
        assert_eq!(kosit_level(Some("R2"), Some("warning"), None, one), "error");
        assert_eq!(kosit_level(None, Some("warning"), None, one), "warning");
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
                context: vec![q("urn:inv", "Invoice"), q("urn:cbc", "CustomizationID")],
                relative: vec![],
                text_nodes: false,
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
                context: vec![q("urn:inv", "CreditNote")],
                relative: vec![q("urn:cbc", "CustomizationID")],
                text_nodes: true,
                identifier: "urn:x".to_owned()
            })
        );
        assert_eq!(
            parse_match(
                "exists(/rsm:CrossIndustryInvoice[rsm:ExchangedDocumentContext/ram:GuidelineSpecifiedDocumentContextParameter/ram:ID/text() = 'urn:cen.eu:en16931:2017'])",
                &namespaces
            ),
            Some(Match {
                context: vec![q("urn:rsm", "CrossIndustryInvoice")],
                relative: vec![
                    q("urn:rsm", "ExchangedDocumentContext"),
                    q("urn:ram", "GuidelineSpecifiedDocumentContextParameter"),
                    q("urn:ram", "ID")
                ],
                text_nodes: true,
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

    /// What each form of `match` reads: the string value, or each text node
    /// child, as XPath does.
    #[test]
    fn matches_read_values_as_xpath_does() {
        let namespaces: Vec<(String, String)> = [("invoice", "urn:inv"), ("cbc", "urn:cbc")]
            .iter()
            .map(|(p, n)| ((*p).to_owned(), (*n).to_owned()))
            .collect();
        let holds = |expression: &str, document: &str| {
            let matching = parse_match(expression, &namespaces).unwrap();
            let doc = parse(document).unwrap();
            matching.holds(doc.root_element())
        };
        let value = "exists(/invoice:Invoice/cbc:CustomizationID[ . = 'urn:ab'])";
        let text = "exists(/invoice:Invoice[cbc:CustomizationID/text() = 'urn:ab'])";
        let relative = "exists(/invoice:Invoice[cbc:CustomizationID = 'urn:ab'])";
        let invoice = |content: &str| {
            format!("<i:Invoice xmlns:i='urn:inv' xmlns:c='urn:cbc'>{content}</i:Invoice>")
        };
        for (content, by_value, by_text) in [
            ("<c:CustomizationID>urn:ab</c:CustomizationID>", true, true),
            (
                "<c:CustomizationID> urn:ab</c:CustomizationID>",
                false,
                false,
            ),
            (
                "<c:CustomizationID>urn:<![CDATA[ab]]></c:CustomizationID>",
                true,
                true,
            ),
            (
                "<c:CustomizationID>urn:&#97;b</c:CustomizationID>",
                true,
                true,
            ),
            (
                "<c:CustomizationID>urn:<!-- -->ab</c:CustomizationID>",
                true,
                false,
            ),
            (
                "<c:CustomizationID>urn:<x>ab</x></c:CustomizationID>",
                true,
                false,
            ),
            (
                "<c:CustomizationID>no</c:CustomizationID><c:CustomizationID>urn:ab</c:CustomizationID>",
                true,
                true,
            ),
            (
                "<x><c:CustomizationID>urn:ab</c:CustomizationID></x>",
                false,
                false,
            ),
            ("<c:Other>urn:ab</c:Other>", false, false),
        ] {
            let document = invoice(content);
            assert_eq!(holds(value, &document), by_value, "{content}");
            assert_eq!(holds(relative, &document), by_value, "{content}");
            assert_eq!(holds(text, &document), by_text, "{content}");
        }
        // Another root element: no match.
        let other = "<i:CreditNote xmlns:i='urn:inv' xmlns:c='urn:cbc'><c:CustomizationID>urn:ab</c:CustomizationID></i:CreditNote>";
        assert!(!holds(value, other));
        assert!(!holds(text, other));
    }

    #[test]
    fn several_or_no_scenario() {
        let root = std::env::temp_dir().join(format!("xtask-select-{}", std::process::id()));
        let base = root.join("base");
        write_fixture(&base);
        let parsed = scenarios(&base, "conf/scenarios.xml").unwrap();
        util::remove_dir(&root).unwrap();
        let column = |document: &str| {
            let doc = parse(document).unwrap();
            Selected::of(&parsed, doc.root_element()).column()
        };
        let invoice = "<Invoice xmlns='urn:oasis:names:specification:ubl:schema:xsd:Invoice-2' xmlns:c='urn:c'>";
        assert_eq!(
            column(&format!(
                "{invoice}<c:CustomizationID>urn:one</c:CustomizationID></Invoice>"
            )),
            "One"
        );
        assert_eq!(column(&format!("{invoice}</Invoice>")), "-");
        assert_eq!(
            column(&format!(
                "{invoice}<c:CustomizationID>urn:other</c:CustomizationID><c:CustomizationID>urn:one</c:CustomizationID></Invoice>"
            )),
            "several: One | Other"
        );
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
