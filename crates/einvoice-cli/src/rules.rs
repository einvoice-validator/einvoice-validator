//! The rule sets fetched on demand: the `rules` command (`list`, `fetch`, `path`,
//! `clear`), and loading a fetched rule set for a validation, from the cache, fetching it
//! first only with `--fetch`.

use std::fmt::Write as _;
#[cfg(feature = "fetch")]
use std::path::PathBuf;

use einvoice_validator::{
    BUILTIN_RULE_SETS, CacheStatus, Error, FetchedRuleSet, RuleSet, RuleSetCache, Validator,
    fetched_rule_set_aliases, fetched_rule_sets,
};

use crate::NAME;
use crate::args::RulesCommand;

/// The environment variable naming a mirror of the publishers' files.
#[cfg(feature = "fetch")]
const MIRROR_ENV: &str = "EINVOICE_VALIDATOR_MIRROR";

/// Runs a `rules` command; its exit status.
pub(crate) fn run(command: &RulesCommand) -> u8 {
    let result = match command {
        RulesCommand::List => list(),
        RulesCommand::Fetch { names, all, from } => fetch_command(names, *all, from),
        RulesCommand::Path(name) => path(name.as_deref()),
        RulesCommand::Clear => clear(),
    };
    match result {
        Ok(text) => crate::print(&text),
        Err(message) => {
            eprintln!("{NAME}: {message}");
            2
        }
    }
}

fn cache() -> Result<RuleSetCache, String> {
    RuleSetCache::new().map_err(|e| e.to_string())
}

fn all() -> Result<&'static [FetchedRuleSet], String> {
    fetched_rule_sets().map_err(|e| e.to_string())
}

/// The families, in the order of the catalogue, each with its rule sets.
fn families() -> Result<Vec<(&'static str, Vec<&'static FetchedRuleSet>)>, String> {
    let mut families: Vec<(&'static str, Vec<&'static FetchedRuleSet>)> = Vec::new();
    for r in all()? {
        match families.iter_mut().find(|(f, _)| *f == r.family()) {
            Some((_, members)) => members.push(r),
            None => families.push((r.family(), vec![r])),
        }
    }
    Ok(families)
}

fn list() -> Result<String, String> {
    let cache = cache()?;
    let mut out = String::from("Built-in rule sets:\n");
    for name in BUILTIN_RULE_SETS {
        match RuleSet::builtin(name) {
            Ok(r) => {
                let _ = writeln!(out, "  {name:<36} {:<10} {}", r.version(), r.title());
            }
            Err(e) => {
                let _ = writeln!(out, "  {name:<36} ({e})");
            }
        }
    }
    let _ = write!(
        out,
        "\nRule sets fetched on demand from their publishers: not redistributed with this
program, downloaded only by `{NAME} rules fetch` or --fetch.
Cache: {}
  {:<36} {:<10} {:<8} TITLE
",
        cache.root().display(),
        "NAME",
        "VERSION",
        "CACHED"
    );
    let mut invalid = Vec::new();
    for r in all()? {
        let cached = match cache.status(r) {
            CacheStatus::Cached => "yes",
            CacheStatus::Missing => "no",
            CacheStatus::Invalid(why) => {
                invalid.push(format!("  {}: {why}", r.name()));
                "invalid"
            }
            _ => "unknown",
        };
        let _ = writeln!(
            out,
            "  {:<36} {:<10} {cached:<8} {}",
            r.name(),
            r.version(),
            r.title()
        );
    }
    if !invalid.is_empty() {
        let _ = writeln!(
            out,
            "Not the bundles as fetched (fetch them again):\n{}",
            invalid.join("\n")
        );
    }
    out.push_str(&sources()?);
    let _ = write!(
        out,
        "\nAliases: {}; with --rules, an alias of two rule sets
picks the one of each document's syntax (UBL or CII).
Fetch: {NAME} rules fetch NAME|FAMILY... or --all
",
        aliases().join(", ")
    );
    Ok(out)
}

/// The sources of each family: publisher, URLs and digests.
fn sources() -> Result<String, String> {
    let mut out =
        String::from("\nSources (a family is fetched as a whole, from the same files):\n");
    for (family, members) in families()? {
        let Some(first) = members.first() else {
            continue;
        };
        let _ = writeln!(
            out,
            "  {family:<10} {} ({} rule set{}){}",
            first.publisher(),
            members.len(),
            if members.len() == 1 { "" } else { "s" },
            if first.needs_java() {
                "; needs Java and network access to Maven Central"
            } else {
                ""
            }
        );
        for source in first.sources() {
            let _ = writeln!(
                out,
                "             {} ({} {})",
                source.url(),
                source.algorithm(),
                source.digest()
            );
        }
    }
    Ok(out)
}

/// Each alias, with the rule sets it stands for: `xrechnung (xrechnung-ubl or ...)`.
fn aliases() -> Vec<String> {
    use einvoice_validator::Syntax;
    fetched_rule_set_aliases()
        .iter()
        .map(|alias| {
            let targets: Vec<&str> = all()
                .unwrap_or_default()
                .iter()
                .filter(|r| {
                    [None, Some(Syntax::Ubl), Some(Syntax::Cii)]
                        .iter()
                        .any(|s| {
                            einvoice_validator::fetched_rule_set(alias, *s)
                                .ok()
                                .flatten()
                                .is_some_and(|t| t.name() == r.name())
                        })
                })
                .map(FetchedRuleSet::name)
                .collect();
            format!("{alias} ({})", targets.join(" or "))
        })
        .collect()
}

fn path(name: Option<&str>) -> Result<String, String> {
    let cache = cache()?;
    let Some(name) = name else {
        return Ok(format!("{}\n", cache.root().display()));
    };
    if BUILTIN_RULE_SETS.contains(&name) {
        return Err(format!(
            "{name} is built in: it is not in the cache and has no directory"
        ));
    }
    let rule_set = einvoice_validator::fetched_rule_set(name, None)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| unknown(name))?;
    let dir = cache.bundle_dir(rule_set);
    match cache.status(rule_set) {
        CacheStatus::Cached => Ok(format!("{}\n", dir.display())),
        CacheStatus::Missing => Err(format!(
            "{} is not in the cache ({}): {NAME} rules fetch {}",
            rule_set.name(),
            dir.display(),
            rule_set.name()
        )),
        CacheStatus::Invalid(why) => Err(format!(
            "{}: {why}; fetch it again: {NAME} rules fetch {}",
            dir.display(),
            rule_set.name()
        )),
        _ => Err(format!("{}: unknown cache status", dir.display())),
    }
}

fn clear() -> Result<String, String> {
    let cache = cache()?;
    cache.clear().map_err(|e| e.to_string())?;
    Ok(format!(
        "{NAME}: the rule-set cache {} is empty\n",
        cache.root().display()
    ))
}

/// The message for a name that is no rule set.
pub(crate) fn unknown(name: &str) -> String {
    let fetched: Vec<&str> = all()
        .unwrap_or_default()
        .iter()
        .map(FetchedRuleSet::name)
        .collect();
    format!(
        "no rule set named {name:?} (built in: {}; fetched on demand: {}; aliases: {}; see {NAME} rules list)",
        if BUILTIN_RULE_SETS.is_empty() {
            "none".to_owned()
        } else {
            BUILTIN_RULE_SETS.join(", ")
        },
        fetched.join(", "),
        fetched_rule_set_aliases().join(", ")
    )
}

/// What to tell after a validation could not load a rule set, beyond the error itself.
pub(crate) fn hint(error: &Error) -> Option<String> {
    let Error::NotCached { name, .. } = error else {
        return None;
    };
    let rule_set = einvoice_validator::fetched_rule_set(name, None)
        .ok()
        .flatten()?;
    Some(format!(
        "{NAME}: {} {} is not redistributed with this program: it is downloaded from {},
only when asked. Fetch it once with
  {NAME} rules fetch {}
or validate with --fetch to download it in this run.",
        rule_set.title(),
        rule_set.version(),
        rule_set.publisher(),
        rule_set.name()
    ))
}

/// Loads a fetched rule set from the cache; when it is not there (or not as fetched)
/// and `fetch` is set, fetches its family first.
pub(crate) fn load(rule_set: &FetchedRuleSet, fetch: bool) -> Result<Validator, Error> {
    let cache = RuleSetCache::new()?;
    if fetch && cache.status(rule_set) != CacheStatus::Cached {
        fetch_family(&cache, rule_set.name(), &[])?;
    }
    cache.load_rule_set(rule_set)
}

#[cfg(feature = "fetch")]
fn options(from: &[PathBuf]) -> einvoice_validator::FetchOptions {
    let mut options =
        einvoice_validator::FetchOptions::new().progress(|line| eprintln!("{NAME}: {line}"));
    if let Some(mirror) = std::env::var(MIRROR_ENV).ok().filter(|m| !m.is_empty()) {
        options = options.mirror(mirror);
    }
    // As the fetch scripts of the repository: JAVA names the Java runtime.
    if let Some(java) = std::env::var_os("JAVA").filter(|j| !j.is_empty()) {
        options = options.java(java);
    }
    for dir in from {
        options = options.from_dir(dir);
    }
    options
}

#[cfg(feature = "fetch")]
fn fetch_family(
    cache: &RuleSetCache,
    name: &str,
    from: &[PathBuf],
) -> Result<Vec<einvoice_validator::Fetched>, Error> {
    cache.fetch(name, &options(from))
}

#[cfg(not(feature = "fetch"))]
fn fetch_family(_: &RuleSetCache, name: &str, _: &[std::path::PathBuf]) -> Result<Vec<()>, Error> {
    Err(Error::Fetch {
        message: format!(
            "{name}: this build of {NAME} cannot download (built without the feature fetch)"
        ),
    })
}

fn fetch_command(
    names: &[String],
    all_families: bool,
    from: &[std::path::PathBuf],
) -> Result<String, String> {
    let cache = cache()?;
    let names: Vec<String> = if all_families {
        families()?.into_iter().map(|(f, _)| f.to_owned()).collect()
    } else {
        names.to_vec()
    };
    let mut out = String::new();
    let mut failed = Vec::new();
    for name in &names {
        if BUILTIN_RULE_SETS.contains(&name.as_str()) {
            let _ = writeln!(out, "{name}: built in, nothing to fetch");
            continue;
        }
        match fetch_family(&cache, name, from) {
            #[cfg(feature = "fetch")]
            Ok(fetched) => {
                for f in fetched {
                    let _ = writeln!(
                        out,
                        "{}: {} in {}{}",
                        f.family,
                        f.rule_sets.join(", "),
                        f.dir.display(),
                        if f.already_cached {
                            " (already in the cache)"
                        } else {
                            ""
                        }
                    );
                }
            }
            #[cfg(not(feature = "fetch"))]
            Ok(_) => {}
            Err(e) => {
                eprintln!("{NAME}: {name}: {e}");
                failed.push(name.as_str());
            }
        }
    }
    if failed.is_empty() {
        Ok(out)
    } else {
        // What was fetched is reported all the same.
        let _ = crate::print(&out);
        Err(format!("not fetched: {}", failed.join(", ")))
    }
}
