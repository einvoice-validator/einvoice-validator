//! The command line: options and documents.

use std::ffi::{OsStr, OsString};
use std::num::NonZeroUsize;
use std::path::PathBuf;

/// The built-in rule set used without `--rules` or `--bundle`.
pub(crate) const DEFAULT_RULES: &str = "cen-ubl";

/// What the command line asks for.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Command {
    Help,
    Version,
    Licences,
    Validate(Options),
}

/// Where the rule set comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Rules {
    Builtin(String),
    Bundle(PathBuf),
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

/// The options of a validation run.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Options {
    pub(crate) rules: Rules,
    pub(crate) format: Format,
    pub(crate) jobs: NonZeroUsize,
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
            (Some(name), None) => Rules::Builtin(name),
            (None, None) => Rules::Builtin(DEFAULT_RULES.to_owned()),
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
            inputs,
        }))
    }
}

/// Parses the arguments (without the program name). `--help`, `--version` and
/// `--licences` win over anything after them.
pub(crate) fn parse(args: &[OsString]) -> Result<Command, UsageError> {
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
            "-r" | "--rules" | "-b" | "--bundle" | "-f" | "--format" | "-j" | "--jobs" => {}
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
        assert_eq!(o.rules, Rules::Builtin("cen-ubl".to_owned()));
        assert_eq!(o.format, Format::Text);
        assert_eq!(o.jobs.get(), 1);
        assert_eq!(o.inputs, [Input::File("a.xml".into())]);
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
