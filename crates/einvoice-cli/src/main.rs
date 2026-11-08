//! `einvoice-validate`: validates e-invoices against the official EN 16931 validation
//! artefacts, with the `einvoice-validator` library. See `--help` and the README of the
//! crate for the options, the output formats and the exit codes.

mod args;
mod json;
mod render;
mod rules;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::io::{IsTerminal as _, Read as _, Write};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;

use einvoice_validator::{
    BUILTIN_RULE_SETS, Error, FetchedRuleSet, RuleSet, Syntax, ValidateOptions, Validator,
    file_uri, pdf,
};

use crate::args::{Command, Format, Input, Options, Rules, UsageError};
use crate::json::Json;
use crate::render::{Outcome, Status, Style};

const NAME: &str = "einvoice-validate";
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The stack of each worker thread, for documents at most `max_depth` deep: at least as
/// large as a main thread's on Linux (8 MiB), and else 1 MiB plus 4 KiB per level of the
/// nesting of templates and functions the library allows the stylesheets for that depth
/// ([`einvoice_validator::max_nesting_for_depth`]). The official stylesheets need less
/// than 3 KiB per level, debug builds included; a stylesheet that needs more per level
/// is stopped by the stack it may use ([`max_stack`]) rather than overflowing it.
fn stack_size(max_depth: u32) -> usize {
    const MIB: usize = 1 << 20;
    let nesting =
        usize::try_from(einvoice_validator::max_nesting_for_depth(max_depth)).unwrap_or(usize::MAX);
    nesting
        .saturating_mul(4 << 10)
        .saturating_add(MIB)
        .max(8 * MIB)
}

/// The stack the stylesheets may use on a worker thread with `stack` bytes of stack
/// ([`ValidateOptions::max_stack`]): all of it but 1 MiB, for the caller and the deepest
/// invocation.
fn max_stack(stack: usize) -> u32 {
    u32::try_from(stack.saturating_sub(1 << 20)).unwrap_or(u32::MAX)
}

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
        Ok(Command::Rules(command)) => rules::run(&command),
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

/// The fetched rule sets, by family: one line per family.
fn fetched_list() -> String {
    let Ok(all) = einvoice_validator::fetched_rule_sets() else {
        return "  (the catalogue does not load)".to_owned();
    };
    let mut lines: Vec<(String, Vec<&str>)> = Vec::new();
    for r in all {
        let head = format!("{} {}", r.family_title(), r.version());
        match lines.iter_mut().find(|(h, _)| *h == head) {
            Some((_, names)) => names.push(r.name()),
            None => lines.push((head, vec![r.name()])),
        }
    }
    let mut out = Vec::new();
    for (head, names) in lines {
        // Wrapped at 80 columns, under the family.
        let mut line = format!("  {head}:");
        for name in names {
            if line.len() + name.len() + 2 > 80 {
                out.push(format!("{line},"));
                line = format!("    {name}");
            } else if line.ends_with(':') {
                line = format!("{line} {name}");
            } else {
                line = format!("{line}, {name}");
            }
        }
        out.push(line);
    }
    out.join(
        "
",
    )
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
    of the project it is built from (einvoice-validator, einvoice-pdf, einvoice-svrl, \
    einvoice-xdm, einvoice-xml, einvoice-xpath, einvoice-xpath-eval, einvoice-xsd, \
    einvoice-xslt, einvoice-xslt-exec)";

/// The notices of the third-party crates the binary is compiled from (crates.io), with
/// the texts of their licences: generated from `Cargo.lock` and the packages of those
/// crates (`einvoice_testkit::notices`; a test checks that it is up to date).
const THIRD_PARTY_NOTICES: &str = include_str!("../THIRD-PARTY-NOTICES");

/// The licence notices of everything the binary embeds, with their full texts: the
/// code (MIT), then the other licences `einvoice_validator::licences` lists for this
/// build (the CEN stylesheets of the built-in rule sets, the UBL 2.1 schemas), then the
/// notices of the third-party crates it is compiled from.
fn licences() -> String {
    let licences = einvoice_validator::licences();
    let mut out = format!(
        "{NAME} {VERSION}: the licences of the works embedded in this program.
Each section names a licence, what it covers, then its full text. The notices of
the third-party crates the program is compiled from follow them.
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
    out.push_str("\n== Third-party crates\n\n");
    out.push_str(THIRD_PARTY_NOTICES);
    out
}

fn help() -> String {
    format!(
        "{NAME} {VERSION}
Validates e-invoices against the official EN 16931 validation artefacts: XML
well-formedness, the XML schema, then the official compiled Schematron rules.

Usage: {NAME} [OPTIONS] FILE...
       {NAME} rules list|fetch|path|clear

Arguments:
  FILE...              Documents to validate; - reads standard input. A PDF file
                       (Factur-X, ZUGFeRD, Order-X) is validated through the
                       invoice XML it embeds.

Options:
  -r, --rules NAME     Rule set: built in, or fetched on demand and read from the
                       cache (see below). Default: chosen for each document from
                       its root element: cen-ubl for a UBL Invoice or CreditNote,
                       cen-cii for a CII CrossIndustryInvoice, {default} otherwise.
      --fetch          Allow --rules to download a fetched rule set that is not
                       in the cache (otherwise that is an error saying how to
                       fetch it). Nothing is ever downloaded without it or the
                       rules fetch command.
  -b, --bundle DIR     Rule-set bundle in DIR (DIR/bundle.toml), instead of --rules.
  -f, --format FORMAT  Output format: text (default), json or svrl.
  -j, --jobs N         Validate N documents at a time (default: 1).
      --max-depth N    Refuse a document with an element nested deeper than N
                       (default: {max_depth}, as libxml2; the document element is at
                       depth 1).
  -h, --help           Print this help.
  -V, --version        Print the version and the built-in rule sets.
      --licences       Print the licences of everything embedded in this program
                       (its code, the CEN stylesheets, the UBL 2.1 schemas, the
                       third-party crates), with their full texts. Also --licenses.

Built-in rule sets:
{builtins}

Rule sets fetched on demand (not redistributed with this program: downloaded
from their publishers at pinned URLs, checked against pinned SHA-256, built into
the cache, only when asked; then used offline, checked on every load):
{fetched}
  Aliases: peppol, and xrechnung (xrechnung-ubl or xrechnung-cii, from each
  document's root element).

Commands:
  rules list           The rule sets, their versions and sources, and which ones
                       the cache holds.
  rules fetch NAME...  Download and build NAME (a rule set, an alias, or a family:
                       factur-x, xrechnung, fr-ctc, peppol) with the other rule
                       sets of its family. --all: every family. --from DIR: take
                       the published files from DIR when it has them (by their
                       file names, checked all the same). Peppol BIS Billing is
                       published as Schematron only: building it compiles it with
                       ph-schematron, which needs Java and access to Maven Central.
  rules path [NAME]    Print the cache directory, or the bundle directory of NAME
                       (usable with --bundle).
  rules clear          Remove everything this program put in the cache.

Environment:
  EINVOICE_VALIDATOR_CACHE   The cache directory. Default: XDG_CACHE_HOME or
                             ~/.cache (Linux), ~/Library/Caches (macOS),
                             %LOCALAPPDATA% (Windows), with einvoice-validator/.
  EINVOICE_VALIDATOR_MIRROR  Fetch https://HOST/PATH from MIRROR/HOST/PATH instead.
  JAVA, JAVA_HOME            The Java runtime that compiles the Peppol Schematron.

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
  2  usage error, or a file or bundle that cannot be read or is not valid (a
     PDF file without one embedded invoice, or malformed)
  3  an unsupported construct (document, PDF file or rule set), a document
     beyond a limit (--max-depth), a stylesheet error on a document, or an
     internal error
",
        default = args::DEFAULT_RULES,
        builtins = builtin_list(),
        fetched = fetched_list(),
        json = render::JSON_FORMAT_VERSION,
        max_depth = einvoice_validator::DEFAULT_MAX_DEPTH,
    )
}

fn run(options: &Options) -> u8 {
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
    // The rule set of each input, by its index in `validators`.
    let (validators, assigned) = match load(options, stdin.as_ref()) {
        Ok(loaded) => loaded,
        Err(e) => {
            eprintln!("{NAME}: {e}");
            if let Some(hint) = rules::hint(&e) {
                eprintln!("{hint}");
            }
            return render::load_exit_code(&e);
        }
    };
    let colour = options.format == Format::Text
        && std::io::stdout().is_terminal()
        && std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty());
    // The rule set of the report as a whole: the first document's.
    let first = assigned.first().copied().unwrap_or(0);
    let Some(head) = validators.get(first) else {
        eprintln!("{NAME}: internal error: no rule set loaded");
        return 3;
    };
    let mut writer = Output::new(options.format, head.rule_set(), Style { colour });
    let mut code = 0;
    let failed = writer.start().err().or_else(|| {
        validate_all(&validators, &assigned, options, stdin.as_ref(), &mut writer).err()
    });
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

/// The rule sets of a run, and for each input the index of its own.
type Loaded = (Vec<Validator>, Vec<usize>);

/// The syntax of each input, from its root element (or the invoice XML of a PDF file).
fn syntaxes(options: &Options, stdin: Option<&std::io::Result<Vec<u8>>>) -> Vec<Option<Syntax>> {
    options
        .inputs
        .iter()
        .map(|input| match input {
            Input::Stdin => stdin
                .and_then(|r| r.as_ref().ok())
                .and_then(|bytes| detect(bytes)),
            Input::File(path) => detect_file(path),
        })
        .collect()
}

/// The rule sets for `--rules NAME` of a rule set fetched on demand (or an alias), from
/// the cache: for an alias of one rule set per syntax (`xrechnung`), the one of each
/// document's syntax. `None` when NAME is no fetched rule set.
fn load_fetched(
    name: &str,
    options: &Options,
    stdin: Option<&std::io::Result<Vec<u8>>>,
) -> Result<Option<Loaded>, Error> {
    let Some(single) = einvoice_validator::fetched_rule_set(name, None)? else {
        return Ok(None);
    };
    let by_syntax = |s| einvoice_validator::fetched_rule_set(name, Some(s));
    let per_syntax = [Syntax::Ubl, Syntax::Cii]
        .into_iter()
        .map(by_syntax)
        .collect::<Result<Vec<_>, Error>>()?
        .iter()
        .any(|r| r.is_some_and(|r| r.name() != single.name()));
    let targets: Vec<&FetchedRuleSet> = if per_syntax {
        syntaxes(options, stdin)
            .into_iter()
            .map(|syntax| Ok(einvoice_validator::fetched_rule_set(name, syntax)?.unwrap_or(single)))
            .collect::<Result<_, Error>>()?
    } else {
        vec![single; options.inputs.len()]
    };
    let mut loaded: Vec<(&str, Validator)> = Vec::new();
    let mut assigned = Vec::with_capacity(targets.len());
    for target in targets {
        let index = if let Some(index) = loaded.iter().position(|(n, _)| *n == target.name()) {
            index
        } else {
            loaded.push((target.name(), rules::load(target, options.fetch)?));
            loaded.len() - 1
        };
        assigned.push(index);
    }
    Ok(Some((
        loaded.into_iter().map(|(_, v)| v).collect(),
        assigned,
    )))
}

/// The rule sets the run needs, loaded once, and for each input the index of its own:
/// the one named by `--rules` (built in, or fetched on demand and read from the cache)
/// or `--bundle`, or, without them, the built-in rule set of the syntax each document's
/// root element names ([`Syntax::detect`]), else [`args::DEFAULT_RULES`].
fn load(
    options: &Options,
    stdin: Option<&std::io::Result<Vec<u8>>>,
) -> Result<(Vec<Validator>, Vec<usize>), Error> {
    let names: Vec<&str> = match &options.rules {
        Rules::Named(name) if BUILTIN_RULE_SETS.contains(&name.as_str()) => {
            return Ok((
                vec![Validator::builtin(name)?],
                vec![0; options.inputs.len()],
            ));
        }
        Rules::Named(name) => {
            return load_fetched(name, options, stdin)?.ok_or_else(|| Error::Bundle {
                message: rules::unknown(name),
            });
        }
        Rules::Bundle(dir) => {
            return Ok((
                vec![Validator::from_bundle_dir(dir)?],
                vec![0; options.inputs.len()],
            ));
        }
        Rules::Detect => syntaxes(options, stdin)
            .into_iter()
            .map(|syntax| syntax.map_or(args::DEFAULT_RULES, Syntax::builtin_rule_set))
            .collect(),
    };
    let mut loaded: Vec<(&str, Validator)> = Vec::new();
    let mut assigned = Vec::with_capacity(names.len());
    for name in names {
        let index = if let Some(index) = loaded.iter().position(|(n, _)| *n == name) {
            index
        } else {
            loaded.push((name, Validator::builtin(name)?));
            loaded.len() - 1
        };
        assigned.push(index);
    }
    Ok((loaded.into_iter().map(|(_, v)| v).collect(), assigned))
}

/// The syntax of the document in the file at `path`, from its start (the whole file
/// when the start is not enough); for a PDF file, of the invoice XML it embeds. `None`
/// when it cannot be read, as the validation then reports.
fn detect_file(path: &std::path::Path) -> Option<Syntax> {
    const HEAD: u64 = 64 << 10;
    let mut head = Vec::new();
    let file = std::fs::File::open(path).ok()?;
    file.take(HEAD).read_to_end(&mut head).ok()?;
    if pdf::is_pdf(&head) {
        return detect(&std::fs::read(path).ok()?);
    }
    match Syntax::detect(&head) {
        Some(syntax) => Some(syntax),
        None if head.len() as u64 == HEAD => Syntax::detect(&std::fs::read(path).ok()?),
        None => None,
    }
}

/// The syntax of a document, or of the invoice XML a PDF file embeds.
fn detect(bytes: &[u8]) -> Option<Syntax> {
    if pdf::is_pdf(bytes) {
        Syntax::detect(&pdf::extract(bytes).ok()?.xml.data)
    } else {
        Syntax::detect(bytes)
    }
}

/// Validates every input on `options.jobs` threads, each with the rule set `assigned`
/// to it in `validators`, and writes the results in the order of the command line as
/// they become available.
fn validate_all(
    validators: &[Validator],
    assigned: &[usize],
    options: &Options,
    stdin: Option<&std::io::Result<Vec<u8>>>,
    writer: &mut Output,
) -> std::io::Result<()> {
    let next = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let (sender, receiver) = mpsc::channel::<(usize, Outcome)>();
    let workers = options.jobs.get().min(options.inputs.len());
    let stack = stack_size(options.max_depth);
    let limits = ValidateOptions::new()
        .max_depth(options.max_depth)
        .max_stack(max_stack(stack));
    std::thread::scope(|scope| {
        for _ in 0..workers {
            let sender = sender.clone();
            let (next, stop) = (&next, &stop);
            let spawned =
                std::thread::Builder::new()
                    .stack_size(stack)
                    .spawn_scoped(scope, move || {
                        while !stop.load(Ordering::Relaxed) {
                            let index = next.fetch_add(1, Ordering::Relaxed);
                            let Some(input) = options.inputs.get(index) else {
                                break;
                            };
                            let Some(validator) =
                                assigned.get(index).and_then(|&v| validators.get(v))
                            else {
                                break;
                            };
                            let outcome = validate(validator, input, stdin, &limits);
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
        if written == options.inputs.len() {
            Ok(())
        } else {
            Err(std::io::Error::other(format!(
                "internal error: {written} of {} documents validated",
                options.inputs.len()
            )))
        }
    })
}

fn validate(
    validator: &Validator,
    input: &Input,
    stdin: Option<&std::io::Result<Vec<u8>>>,
    limits: &ValidateOptions<'_>,
) -> Outcome {
    let rule_set = validator.rule_set().name().to_owned();
    let mut pdf_info = None;
    let (name, result) = match input {
        Input::Stdin => {
            let result = match stdin {
                Some(Ok(bytes)) if pdf::is_pdf(bytes) => embedded_xml(bytes, &mut pdf_info)
                    .and_then(|xml| validator.validate_owned_with(xml, limits)),
                Some(Ok(bytes)) => validator.validate_with(bytes, limits),
                Some(Err(e)) => Err(Error::Io {
                    path: "-".into(),
                    source: std::io::Error::new(e.kind(), e.to_string()),
                }),
                None => Err(Error::Io {
                    path: "-".into(),
                    source: std::io::Error::other("standard input was not read"),
                }),
            };
            ("-".to_owned(), result)
        }
        Input::File(path) => {
            let name = path.display().to_string();
            let result = std::fs::read(path)
                .map_err(|source| Error::Io {
                    path: path.clone(),
                    source,
                })
                .and_then(|bytes| {
                    // The document's URI: the file's, the PDF's for the XML it embeds.
                    let uri = std::path::absolute(path).ok().and_then(|p| file_uri(&p));
                    let options = uri.as_deref().map_or(*limits, |u| limits.document_uri(u));
                    let bytes = if pdf::is_pdf(&bytes) {
                        embedded_xml(&bytes, &mut pdf_info)?
                    } else {
                        bytes
                    };
                    // The validation keeps the buffer rather than copying its text.
                    validator.validate_owned_with(bytes, &options)
                });
            (name, result)
        }
    };
    Outcome {
        name,
        rule_set,
        pdf: pdf_info,
        result,
    }
}

/// The invoice XML embedded in the PDF file `bytes`; what the PDF says about it goes to
/// `info`, without the XML.
fn embedded_xml(bytes: &[u8], info: &mut Option<pdf::Embedded>) -> Result<Vec<u8>, Error> {
    let mut embedded = pdf::extract(bytes)?;
    let xml = std::mem::take(&mut embedded.xml.data);
    *info = Some(embedded);
    Ok(xml)
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

#[cfg(test)]
mod tests {
    use super::{max_stack, stack_size};

    #[test]
    fn worker_stacks_grow_with_the_maximum_depth() {
        const MIB: usize = 1 << 20;
        assert_eq!(stack_size(einvoice_validator::DEFAULT_MAX_DEPTH), 8 * MIB);
        assert_eq!(stack_size(1), 8 * MIB);
        // 1 MiB and 4 KiB for each of the 2 * 100000 + 64 levels of nesting.
        assert_eq!(stack_size(100_000), MIB + 200_064 * 4096);
        assert_eq!(max_stack(8 * MIB), 7 << 20);
        assert_eq!(max_stack(stack_size(100_000)), 200_064 * 4096);
    }
}
