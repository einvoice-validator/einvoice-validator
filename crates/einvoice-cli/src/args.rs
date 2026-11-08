//! The command line: options and documents.

use std::ffi::{OsStr, OsString};
use std::num::NonZeroUsize;
use std::path::PathBuf;

use einvoice_validator::DEFAULT_MAX_DEPTH;

/// The built-in rule set used without `--rules` or `--bundle` for a document whose root
/// element names no known syntax (see [`Rules::Detect`]).
pub(crate) const DEFAULT_RULES: &str = "cen-ubl";

/// What the command line asks for.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Command {
    Help,
    Version,
    Licences,
    Validate(Options),
    /// `rules ...`: the rule sets fetched on demand and their cache.
    Rules(RulesCommand),
}

/// A `rules` subcommand.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RulesCommand {
    /// `rules list`: every rule set, built in or fetched, and whether it is cached.
    List,
    /// `rules fetch NAME...` or `rules fetch --all`, with `--from DIR` (repeatable).
    Fetch {
        names: Vec<String>,
        all: bool,
        from: Vec<PathBuf>,
    },
    /// `rules path [NAME]`: the cache directory, or the bundle directory of NAME.
    Path(Option<String>),
    /// `rules clear`: empties the cache.
    Clear,
}

/// Where the rule set comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Rules {
    /// `--rules NAME`: a built-in rule set, a fetched one (from the cache) or an alias.
    Named(String),
    Bundle(PathBuf),
    /// Neither `--rules` nor `--bundle`: for each document, the built-in rule set of the
    /// syntax its root element names (`einvoice_validator::Syntax`), else
    /// [`DEFAULT_RULES`].
    Detect,
}

/// The output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Format {
    Text,
    Json,
    Svrl,
}

/// A document to validate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Input {
    Stdin,
    File(PathBuf),
}

/// The largest `--max-depth`: the worker threads' stacks grow with it (4 KiB a level).
pub(crate) const MAX_DEPTH: u32 = 100_000;

/// The options of a validation run.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Options {
    pub(crate) rules: Rules,
    pub(crate) format: Format,
    pub(crate) jobs: NonZeroUsize,
    /// The maximum element depth of a document (`--max-depth`).
    pub(crate) max_depth: u32,
    /// `--fetch`: a fetched rule set that is not in the cache may be downloaded.
    pub(crate) fetch: bool,
    /// `--fail-closed`: a finding that is neither fatal nor a warning is fatal
    /// (`einvoice_validator::ValidateOptions::fail_closed`).
    pub(crate) fail_closed: bool,
    pub(crate) inputs: Vec<Input>,
}

/// A command line that does not parse; the message says why.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct UsageError(pub(crate) String);

/// The options seen so far.
#[derive(Debug, Default)]
struct Seen {
    rules: Option<String>,
    bundle: Option<PathBuf>,
    format: Option<Format>,
    jobs: Option<NonZeroUsize>,
    max_depth: Option<u32>,
    fetch: bool,
    fail_closed: bool,
}

impl Seen {
    /// Records the value of the option `name` (one that takes a value).
    fn set(&mut self, name: &str, value: &OsStr) -> Result<(), UsageError> {
        let utf8 = || {
            value
                .to_str()
                .ok_or_else(|| usage(format!("{name}: the value is not valid Unicode")))
        };
        match name {
            "-r" | "--rules" => set(&mut self.rules, utf8()?.to_owned(), "--rules"),
            "-b" | "--bundle" => set(&mut self.bundle, PathBuf::from(value), "--bundle"),
            "-f" | "--format" => {
                let format = match utf8()? {
                    "text" => Format::Text,
                    "json" => Format::Json,
                    "svrl" => Format::Svrl,
                    other => {
                        return Err(usage(format!(
                            "unknown format {other:?} (expected text, json or svrl)"
                        )));
                    }
                };
                set(&mut self.format, format, "--format")
            }
            "--max-depth" => {
                let n = utf8()?
                    .parse::<u32>()
                    .ok()
                    .filter(|&n| (1..=MAX_DEPTH).contains(&n))
                    .ok_or_else(|| {
                        usage(format!(
                            "{name}: expected a positive integer of at most {MAX_DEPTH}"
                        ))
                    })?;
                set(&mut self.max_depth, n, "--max-depth")
            }
            _ => {
                let n = utf8()?
                    .parse::<NonZeroUsize>()
                    .map_err(|_| usage(format!("{name}: expected a positive integer")))?;
                set(&mut self.jobs, n, "--jobs")
            }
        }
    }

    fn finish(self, inputs: Vec<Input>) -> Result<Command, UsageError> {
        let rules = match (self.rules, self.bundle) {
            (Some(_), Some(_)) => {
                return Err(usage("--rules and --bundle exclude each other".to_owned()));
            }
            (_, Some(dir)) => Rules::Bundle(dir),
            (Some(name), None) => Rules::Named(name),
            (None, None) => Rules::Detect,
        };
        if inputs.is_empty() {
            return Err(usage(
                "no document given (use - to read standard input)".to_owned(),
            ));
        }
        if inputs.iter().filter(|i| **i == Input::Stdin).count() > 1 {
            return Err(usage(
                "standard input (-) can only be given once".to_owned(),
            ));
        }
        Ok(Command::Validate(Options {
            rules,
            format: self.format.unwrap_or(Format::Text),
            jobs: self.jobs.unwrap_or(NonZeroUsize::MIN),
            max_depth: self.max_depth.unwrap_or(DEFAULT_MAX_DEPTH),
            fetch: self.fetch,
            fail_closed: self.fail_closed,
            inputs,
        }))
    }
}

/// Parses the arguments (without the program name). `--help`, `--version` and
/// `--licences` win over anything after them. A first argument `rules` is the
/// `rules` command (a file of that name is given as `./rules` or after `--`).
pub(crate) fn parse(args: &[OsString]) -> Result<Command, UsageError> {
    if args.first().is_some_and(|a| a == "rules") {
        return parse_rules(&args[1..]);
    }
    let mut seen = Seen::default();
    let mut inputs = Vec::new();
    let mut only_files = false;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if only_files || arg == "-" || !arg.as_encoded_bytes().starts_with(b"-") {
            inputs.push(input(arg));
            continue;
        }
        if arg == "--" {
            only_files = true;
            continue;
        }
        let text = arg
            .to_str()
            .ok_or_else(|| usage(format!("unknown option {}", arg.to_string_lossy())))?;
        // `--name=value`, `--name value`, `-xvalue` or `-x value`.
        let (name, inline): (&str, Option<&str>) = if text.starts_with("--") {
            match text.split_once('=') {
                Some((name, value)) => (name, Some(value)),
                None => (text, None),
            }
        } else {
            let split = text.char_indices().nth(2).map_or(text.len(), |(i, _)| i);
            let (name, rest) = text.split_at(split);
            (name, (!rest.is_empty()).then_some(rest))
        };
        match name {
            "--fetch" | "--fail-closed" => {
                if inline.is_some() {
                    return Err(usage(format!("{name} takes no value")));
                }
                let flag = if name == "--fetch" {
                    &mut seen.fetch
                } else {
                    &mut seen.fail_closed
                };
                if *flag {
                    return Err(usage(format!("{name} given twice")));
                }
                *flag = true;
                continue;
            }
            "-h" | "--help" | "-V" | "--version" | "--licences" | "--licenses" => {
                if inline.is_some() {
                    return Err(usage(format!("{name} takes no value")));
                }
                return Ok(match name {
                    "-h" | "--help" => Command::Help,
                    "-V" | "--version" => Command::Version,
                    _ => Command::Licences,
                });
            }
            "-r" | "--rules" | "-b" | "--bundle" | "-f" | "--format" | "-j" | "--jobs"
            | "--max-depth" => {}
            _ => return Err(usage(format!("unknown option {name}"))),
        }
        let value: OsString = match inline {
            Some(v) => OsString::from(v),
            None => args
                .next()
                .cloned()
                .ok_or_else(|| usage(format!("{name} needs a value")))?,
        };
        seen.set(name, &value)?;
    }
    seen.finish(inputs)
}

/// Parses the arguments of `rules` (after it).
fn parse_rules(args: &[OsString]) -> Result<Command, UsageError> {
    fn text(arg: &OsString) -> Result<&str, UsageError> {
        arg.to_str().ok_or_else(|| {
            usage(format!(
                "rules: {} is not valid Unicode",
                arg.to_string_lossy()
            ))
        })
    }
    let Some((sub, rest)) = args.split_first() else {
        return Err(usage(
            "rules: expected list, fetch, path or clear".to_owned(),
        ));
    };
    if rest.iter().any(|a| a == "-h" || a == "--help") || sub == "-h" || sub == "--help" {
        return Ok(Command::Help);
    }
    let none = |name: &str| -> Result<(), UsageError> {
        match rest.first() {
            Some(extra) => Err(usage(format!(
                "rules {name}: unexpected argument {}",
                extra.to_string_lossy()
            ))),
            None => Ok(()),
        }
    };
    match text(sub)? {
        "list" => none("list").map(|()| Command::Rules(RulesCommand::List)),
        "clear" => none("clear").map(|()| Command::Rules(RulesCommand::Clear)),
        "path" => match rest {
            [] => Ok(Command::Rules(RulesCommand::Path(None))),
            [name] => Ok(Command::Rules(RulesCommand::Path(Some(
                text(name)?.to_owned(),
            )))),
            [_, extra, ..] => Err(usage(format!(
                "rules path: unexpected argument {}",
                extra.to_string_lossy()
            ))),
        },
        "fetch" => {
            let (mut names, mut all, mut from) = (Vec::new(), false, Vec::new());
            let mut rest = rest.iter();
            while let Some(arg) = rest.next() {
                let arg_text = text(arg)?;
                if arg_text == "--all" {
                    if all {
                        return Err(usage("rules fetch: --all given twice".to_owned()));
                    }
                    all = true;
                } else if arg_text == "--from" {
                    let dir = rest
                        .next()
                        .ok_or_else(|| usage("rules fetch: --from needs a value".to_owned()))?;
                    from.push(PathBuf::from(dir));
                } else if let Some(dir) = arg_text.strip_prefix("--from=") {
                    from.push(PathBuf::from(dir));
                } else if arg_text.starts_with('-') {
                    return Err(usage(format!("rules fetch: unknown option {arg_text}")));
                } else {
                    names.push(arg_text.to_owned());
                }
            }
            match (all, names.is_empty()) {
                (true, false) => Err(usage(
                    "rules fetch: --all and rule-set names exclude each other".to_owned(),
                )),
                (false, true) => Err(usage(
                    "rules fetch: name the rule sets to fetch, or --all".to_owned(),
                )),
                _ => Ok(Command::Rules(RulesCommand::Fetch { names, all, from })),
            }
        }
        other => Err(usage(format!(
            "rules: unknown command {other:?} (expected list, fetch, path or clear)"
        ))),
    }
}

fn input(arg: &OsStr) -> Input {
    if arg == "-" {
        Input::Stdin
    } else {
        Input::File(PathBuf::from(arg))
    }
}

fn set<T>(slot: &mut Option<T>, value: T, name: &str) -> Result<(), UsageError> {
    if slot.is_some() {
        return Err(usage(format!("{name} given twice")));
    }
    *slot = Some(value);
    Ok(())
}

fn usage(message: String) -> UsageError {
    UsageError(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(args: &[&str]) -> Result<Command, UsageError> {
        let args: Vec<OsString> = args.iter().map(OsString::from).collect();
        parse(&args)
    }

    fn options(args: &[&str]) -> Options {
        match run(args) {
            Ok(Command::Validate(o)) => o,
            other => panic!("{other:?}"),
        }
    }

    fn error(args: &[&str]) -> String {
        match run(args) {
            Err(UsageError(m)) => m,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn defaults() {
        let o = options(&["a.xml"]);
        assert_eq!(o.rules, Rules::Detect);
        assert_eq!(o.format, Format::Text);
        assert_eq!(o.jobs.get(), 1);
        assert_eq!(o.max_depth, DEFAULT_MAX_DEPTH);
        assert_eq!(o.inputs, [Input::File("a.xml".into())]);
    }

    #[test]
    fn maximum_depth() {
        assert_eq!(options(&["--max-depth", "1000", "a"]).max_depth, 1000);
        assert_eq!(options(&["--max-depth=1", "a"]).max_depth, 1);
        assert_eq!(
            options(&["--max-depth", "100000", "a"]).max_depth,
            MAX_DEPTH
        );
        for bad in ["0", "-1", "x", "100001", "4294967296", ""] {
            assert!(
                error(&["--max-depth", bad, "a"])
                    .starts_with("--max-depth: expected a positive integer"),
                "{bad}"
            );
        }
        assert_eq!(
            error(&["--max-depth", "5", "--max-depth", "6", "a"]),
            "--max-depth given twice"
        );
        assert_eq!(error(&["a", "--max-depth"]), "--max-depth needs a value");
    }

    #[test]
    fn option_forms() {
        for args in [
            &["-f", "json", "a", "-", "b"][..],
            &["--format", "json", "a", "-", "b"],
            &["--format=json", "a", "-", "b"],
            &["-fjson", "a", "-", "b"],
            &["a", "-f", "json", "-", "b"],
        ] {
            let o = options(args);
            assert_eq!(o.format, Format::Json, "{args:?}");
            assert_eq!(
                o.inputs,
                [
                    Input::File("a".into()),
                    Input::Stdin,
                    Input::File("b".into())
                ],
                "{args:?}"
            );
        }
        let o = options(&["-r", "cen-ubl", "-j", "4", "--", "-f", "-"]);
        assert_eq!(o.jobs.get(), 4);
        assert_eq!(o.inputs, [Input::File("-f".into()), Input::Stdin]);
        let o = options(&["--bundle", "dir", "--format=svrl", "x"]);
        assert_eq!(o.rules, Rules::Bundle("dir".into()));
        assert_eq!(o.format, Format::Svrl);
    }

    #[test]
    fn fail_closed_option() {
        assert!(!options(&["a"]).fail_closed);
        let o = options(&["--rules", "factur-x-en16931", "--fail-closed", "a"]);
        assert!(o.fail_closed);
        assert!(!o.fetch);
        assert!(options(&["a", "--fail-closed", "--fetch"]).fail_closed);
        assert_eq!(
            error(&["--fail-closed=yes", "a"]),
            "--fail-closed takes no value"
        );
        assert_eq!(
            error(&["--fail-closed", "--fail-closed", "a"]),
            "--fail-closed given twice"
        );
    }

    #[test]
    fn fetch_option() {
        assert!(!options(&["a"]).fetch);
        let o = options(&["--rules", "peppol", "--fetch", "a"]);
        assert!(o.fetch);
        assert_eq!(o.rules, Rules::Named("peppol".to_owned()));
        assert_eq!(error(&["--fetch=1", "a"]), "--fetch takes no value");
        assert_eq!(error(&["--fetch", "--fetch", "a"]), "--fetch given twice");
    }

    #[test]
    fn rules_command() {
        let rules = |args: &[&str]| match run(args) {
            Ok(Command::Rules(c)) => c,
            other => panic!("{args:?}: {other:?}"),
        };
        assert_eq!(rules(&["rules", "list"]), RulesCommand::List);
        assert_eq!(rules(&["rules", "clear"]), RulesCommand::Clear);
        assert_eq!(rules(&["rules", "path"]), RulesCommand::Path(None));
        assert_eq!(
            rules(&["rules", "path", "peppol"]),
            RulesCommand::Path(Some("peppol".to_owned()))
        );
        assert_eq!(
            rules(&[
                "rules",
                "fetch",
                "peppol",
                "--from",
                "d1",
                "xrechnung",
                "--from=d2"
            ]),
            RulesCommand::Fetch {
                names: vec!["peppol".to_owned(), "xrechnung".to_owned()],
                all: false,
                from: vec!["d1".into(), "d2".into()],
            }
        );
        assert_eq!(
            rules(&["rules", "fetch", "--all"]),
            RulesCommand::Fetch {
                names: Vec::new(),
                all: true,
                from: Vec::new(),
            }
        );
        assert_eq!(run(&["rules", "--help"]), Ok(Command::Help));
        assert_eq!(run(&["rules", "fetch", "x", "-h"]), Ok(Command::Help));
        // Only as the first argument: elsewhere it is a file.
        assert_eq!(
            options(&["a", "rules"]).inputs,
            [Input::File("a".into()), Input::File("rules".into())]
        );
        assert_eq!(
            options(&["--", "rules"]).inputs,
            [Input::File("rules".into())]
        );
        let cases: [(&[&str], &str); 9] = [
            (&["rules"], "rules: expected list, fetch, path or clear"),
            (
                &["rules", "x"],
                "rules: unknown command \"x\" (expected list, fetch, path or clear)",
            ),
            (&["rules", "list", "x"], "rules list: unexpected argument x"),
            (
                &["rules", "path", "a", "b"],
                "rules path: unexpected argument b",
            ),
            (
                &["rules", "fetch"],
                "rules fetch: name the rule sets to fetch, or --all",
            ),
            (
                &["rules", "fetch", "--all", "--all"],
                "rules fetch: --all given twice",
            ),
            (
                &["rules", "fetch", "--all", "a"],
                "rules fetch: --all and rule-set names exclude each other",
            ),
            (
                &["rules", "fetch", "a", "--from"],
                "rules fetch: --from needs a value",
            ),
            (&["rules", "fetch", "-x"], "rules fetch: unknown option -x"),
        ];
        for (args, expected) in cases {
            assert_eq!(error(args), expected, "{args:?}");
        }
    }

    #[test]
    fn help_and_version() {
        assert_eq!(run(&["--help"]), Ok(Command::Help));
        assert_eq!(run(&["a", "-h", "--bogus"]), Ok(Command::Help));
        assert_eq!(run(&["-V"]), Ok(Command::Version));
        assert_eq!(run(&["--version"]), Ok(Command::Version));
        assert_eq!(run(&["--licences"]), Ok(Command::Licences));
        assert_eq!(run(&["--licenses"]), Ok(Command::Licences));
        assert_eq!(
            run(&["a.xml", "--licences", "--bogus"]),
            Ok(Command::Licences)
        );
        assert_eq!(
            run(&[]).unwrap_err().0,
            "no document given (use - to read standard input)"
        );
    }

    #[test]
    fn usage_errors() {
        let cases: [(&[&str], &str); 13] = [
            (&["--bogus", "a"], "unknown option --bogus"),
            (&["-x", "a"], "unknown option -x"),
            (&["-f"], "-f needs a value"),
            (&["-f", "xml", "a"], "unknown format \"xml\""),
            (&["-j", "0", "a"], "-j: expected a positive integer"),
            (&["--jobs=many", "a"], "--jobs: expected a positive integer"),
            (
                &["-r", "a", "-b", "d", "x"],
                "--rules and --bundle exclude each other",
            ),
            (&["-f", "json", "-f", "text", "a"], "--format given twice"),
            (&["-", "-"], "standard input (-) can only be given once"),
            (&["--help=x"], "--help takes no value"),
            (&["--licences=x"], "--licences takes no value"),
            (&["--licence"], "unknown option --licence"),
            (&["-f", "json"], "no document given"),
        ];
        for (args, expected) in cases {
            let message = error(args);
            assert!(message.starts_with(expected), "{args:?}: {message}");
        }
    }
}
