//! `einvoice-validate`: validates e-invoices against the official EN 16931 validation
//! artefacts, with the `einvoice-validator` library. See `--help` and the README of the
//! crate for the options, the output formats and the exit codes.

mod args;
mod json;
mod render;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::io::{IsTerminal as _, Read as _, Write};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;

use einvoice_validator::{BUILTIN_RULE_SETS, Error, RuleSet, ValidateOptions, Validator, file_uri};

use crate::args::{Command, Format, Input, Options, Rules, UsageError};
use crate::json::Json;
use crate::render::{Outcome, Status, Style};

const NAME: &str = "einvoice-validate";
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The stack of each worker thread: as large as a main thread's on Linux, for deeply
/// nested documents.
const STACK_SIZE: usize = 8 << 20;

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let code = match args::parse(&args) {
        Err(UsageError(message)) => {
            eprintln!("{NAME}: {message}\nTry '{NAME} --help' for more information.");
            2
        }
        Ok(Command::Help) => print(&help()),
        Ok(Command::Version) => print(&version()),
        Ok(Command::Licences) => print(&licences()),
        Ok(Command::Validate(options)) => run(&options),
    };
    ExitCode::from(code)
}

/// Writes to standard output; exit code 0, or 2 when it cannot be written.
fn print(text: &str) -> u8 {
    let mut stdout = std::io::stdout().lock();
    match stdout
        .write_all(text.as_bytes())
        .and_then(|()| stdout.flush())
    {
        Ok(()) => 0,
        Err(e) => output_failed(&e),
    }
}

/// Exit code 2 for an output that cannot be written; silent when the reader has gone
/// (a closed pipe).
fn output_failed(e: &std::io::Error) -> u8 {
    if e.kind() != std::io::ErrorKind::BrokenPipe {
        eprintln!("{NAME}: cannot write the output: {e}");
    }
    2
}

fn builtin_list() -> String {
    BUILTIN_RULE_SETS
        .iter()
        .map(|name| match RuleSet::builtin(name) {
            Ok(r) => format!(
                "  {name:<8} {} {}{}",
                r.title(),
                r.version(),
                r.schema()
                    .map_or_else(String::new, |s| format!(", with the {s} schema"))
            ),
            Err(e) => format!("  {name:<8} ({e})"),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn version() -> String {
    format!(
        "{NAME} {VERSION}\nBuilt-in rule sets:\n{}\n",
        builtin_list()
    )
}

/// The MIT licence of this crate's code: the text of the library crates' (a test checks
/// it), which `einvoice_validator::licences` lists for their code.
const MIT: &str = include_str!("../LICENSES/MIT.txt");

/// What the MIT licence covers in this binary.
const MIT_COVERS: &str = "the code of einvoice-validate (crate einvoice-cli) and of the crates \
    of the project it is built from (einvoice-validator, einvoice-svrl, einvoice-xdm, \
    einvoice-xpath, einvoice-xpath-eval, einvoice-xsd, einvoice-xslt, einvoice-xslt-exec)";

/// The licence notices of everything the binary embeds, with their full texts: the
/// code (MIT), then the other licences `einvoice_validator::licences` lists for this
/// build (the CEN stylesheet of the built-in rule set, the UBL 2.1 schemas).
fn licences() -> String {
    let licences = einvoice_validator::licences();
    let mut out = format!(
        "{NAME} {VERSION}: the licences of the works embedded in this program.
Each section names a licence, what it covers, then its full text.
"
    );
    let total = licences.len();
    for (i, licence) in licences.iter().enumerate() {
        let (covers, text) = if licence.id == "MIT" {
            (MIT_COVERS, MIT)
        } else {
            (licence.covers, licence.text)
        };
        let _ = write!(
            out,
            "\n== {n}/{total} {id}\nCovers: {covers}.\n\n{text}",
            n = i + 1,
            id = licence.id,
        );
        if !text.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

fn help() -> String {
    format!(
        "{NAME} {VERSION}
Validates e-invoices against the official EN 16931 validation artefacts: XML
well-formedness, the XML schema, then the official compiled Schematron rules.

Usage: {NAME} [OPTIONS] FILE...

Arguments:
  FILE...              Documents to validate; - reads standard input.

Options:
  -r, --rules NAME     Built-in rule set (default: {default}).
  -b, --bundle DIR     Rule-set bundle in DIR (DIR/bundle.toml), instead of --rules.
  -f, --format FORMAT  Output format: text (default), json or svrl.
  -j, --jobs N         Validate N documents at a time (default: 1).
  -h, --help           Print this help.
  -V, --version        Print the version and the built-in rule sets.
      --licences       Print the licences of everything embedded in this program
                       (its code, the CEN stylesheet, the UBL 2.1 schemas), with
                       their full texts. Also --licenses.

Built-in rule sets:
{builtins}

Output formats:
  text  One line per document with its verdict, then its schema errors and its
        findings. Coloured only on a terminal, never when NO_COLOR is set.
  json  One JSON object for the whole run: format \"{NAME}\",
        format_version {json}, documented in the README of the einvoice-cli crate.
  svrl  The SVRL report of each stylesheet as it wrote it, one XML document after
        another; problems go to standard error.

Exit status (the highest that applies):
  0  every document is valid
  1  a document is invalid: schema errors, fatal findings, or not well-formed XML
  2  usage error, or a file or bundle that cannot be read or is not valid
  3  an unsupported construct (document or rule set), a stylesheet error on a
     document, or an internal error
",
        default = args::DEFAULT_RULES,
        builtins = builtin_list(),
        json = render::JSON_FORMAT_VERSION,
    )
}

fn run(options: &Options) -> u8 {
    let validator = match &options.rules {
        Rules::Builtin(name) => Validator::builtin(name),
        Rules::Bundle(dir) => Validator::from_bundle_dir(dir),
    };
    let validator = match validator {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{NAME}: {e}");
            return render::load_exit_code(&e);
        }
    };
    let colour = options.format == Format::Text
        && std::io::stdout().is_terminal()
        && std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty());
    let mut stdin = None;
    if options.inputs.contains(&Input::Stdin) {
        let mut bytes = Vec::new();
        stdin = Some(
            std::io::stdin()
                .lock()
                .read_to_end(&mut bytes)
                .map(|_| bytes),
        );
    }
    let mut writer = Output::new(options.format, validator.rule_set(), Style { colour });
    let mut code = 0;
    let failed = writer
        .start()
        .err()
        .or_else(|| validate_all(&validator, options, stdin.as_ref(), &mut writer).err());
    if let Some(e) = failed {
        return code.max(output_failed(&e));
    }
    code = writer
        .statuses
        .iter()
        .map(|s| s.exit_code())
        .max()
        .unwrap_or(0);
    match writer.finish(code) {
        Ok(()) => code,
        Err(e) => code.max(output_failed(&e)),
    }
}

/// Validates every input on `options.jobs` threads and writes the results in the order
/// of the command line as they become available.
fn validate_all(
    validator: &Validator,
    options: &Options,
    stdin: Option<&std::io::Result<Vec<u8>>>,
    writer: &mut Output,
) -> std::io::Result<()> {
    let next = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let (sender, receiver) = mpsc::channel::<(usize, Outcome)>();
    let workers = options.jobs.get().min(options.inputs.len());
    std::thread::scope(|scope| {
        for _ in 0..workers {
            let sender = sender.clone();
            let (next, stop) = (&next, &stop);
            let spawned = std::thread::Builder::new()
                .stack_size(STACK_SIZE)
                .spawn_scoped(scope, move || {
                    while !stop.load(Ordering::Relaxed) {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        let Some(input) = options.inputs.get(index) else {
                            break;
                        };
                        let outcome = validate(validator, input, stdin);
                        if sender.send((index, outcome)).is_err() {
                            break;
                        }
                    }
                });
            if let Err(e) = spawned {
                stop.store(true, Ordering::Relaxed);
                return Err(e);
            }
        }
        drop(sender);
        let mut pending = BTreeMap::new();
        let mut written = 0;
        for (index, outcome) in &receiver {
            pending.insert(index, outcome);
            while let Some(outcome) = pending.remove(&written) {
                if let Err(e) = writer.document(&outcome) {
                    stop.store(true, Ordering::Relaxed);
                    return Err(e);
                }
                written += 1;
            }
        }
        Ok(())
    })
}

fn validate(
    validator: &Validator,
    input: &Input,
    stdin: Option<&std::io::Result<Vec<u8>>>,
) -> Outcome {
    match input {
        Input::Stdin => {
            let result = match stdin {
                Some(Ok(bytes)) => validator.validate(bytes),
                Some(Err(e)) => Err(Error::Io {
                    path: "-".into(),
                    source: std::io::Error::new(e.kind(), e.to_string()),
                }),
                None => Err(Error::Io {
                    path: "-".into(),
                    source: std::io::Error::other("standard input was not read"),
                }),
            };
            Outcome {
                name: "-".to_owned(),
                result,
            }
        }
        Input::File(path) => {
            let name = path.display().to_string();
            let result = std::fs::read(path)
                .map_err(|source| Error::Io {
                    path: path.clone(),
                    source,
                })
                .and_then(|bytes| {
                    let uri = std::path::absolute(path).ok().and_then(|p| file_uri(&p));
                    let options = uri.as_deref().map_or_else(ValidateOptions::new, |u| {
                        ValidateOptions::new().document_uri(u)
                    });
                    // The validation keeps the buffer rather than copying its text.
                    validator.validate_owned_with(bytes, &options)
                });
            Outcome { name, result }
        }
    }
}

/// Writes the results in the chosen format, document by document.
struct Output {
    format: Format,
    style: Style,
    rule_set: Json,
    statuses: Vec<Status>,
    stdout: std::io::Stdout,
}

impl std::fmt::Debug for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Output")
            .field("format", &self.format)
            .field("documents", &self.statuses.len())
            .finish_non_exhaustive()
    }
}

impl Output {
    fn new(format: Format, rule_set: &RuleSet, style: Style) -> Self {
        Self {
            format,
            style,
            rule_set: render::json_rule_set(rule_set),
            statuses: Vec::new(),
            stdout: std::io::stdout(),
        }
    }

    fn out(&self, text: &str) -> std::io::Result<()> {
        let mut stdout = self.stdout.lock();
        stdout.write_all(text.as_bytes())?;
        stdout.flush()
    }

    fn start(&self) -> std::io::Result<()> {
        if self.format != Format::Json {
            return Ok(());
        }
        let mut head = String::from("{\n");
        let fields = [
            ("format", Json::string(NAME)),
            ("format_version", Json::Number(render::JSON_FORMAT_VERSION)),
            (
                "tool",
                Json::Object(vec![
                    ("name", Json::string(NAME)),
                    ("version", Json::string(VERSION)),
                ]),
            ),
            ("rule_set", self.rule_set.clone()),
        ];
        for (key, value) in fields {
            json::indent(1, &mut head);
            json::write_string(key, &mut head);
            head.push_str(": ");
            value.write(1, &mut head);
            head.push_str(",\n");
        }
        head.push_str("  \"documents\": [");
        self.out(&head)
    }

    fn document(&mut self, outcome: &Outcome) -> std::io::Result<()> {
        let first = self.statuses.is_empty();
        self.statuses.push(Status::of(&outcome.result));
        match self.format {
            Format::Text => self.out(&render::text(outcome, self.style)),
            Format::Svrl => {
                let (out, err) = render::svrl(outcome);
                if !err.is_empty() {
                    eprint!("{err}");
                }
                self.out(&out)
            }
            Format::Json => {
                let mut text = String::from(if first { "\n    " } else { ",\n    " });
                render::json(outcome).write(2, &mut text);
                self.out(&text)
            }
        }
    }

    fn finish(&self, code: u8) -> std::io::Result<()> {
        match self.format {
            Format::Text if self.statuses.len() > 1 => {
                self.out(&render::text_summary(&self.statuses))
            }
            Format::Text | Format::Svrl => Ok(()),
            Format::Json => {
                let mut tail = String::from("\n  ],\n  \"summary\": ");
                render::json_summary(&self.statuses).write(1, &mut tail);
                tail.push_str(",\n  \"exit_code\": ");
                tail.push_str(&code.to_string());
                tail.push_str("\n}\n");
                self.out(&tail)
            }
        }
    }
}
