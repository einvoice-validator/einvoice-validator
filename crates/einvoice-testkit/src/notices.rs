//! The third-party notices of the programs the project distributes.
//!
//! The `einvoice-validate` binary and the npm package are statically built with crates from
//! crates.io, whose licences ask for their copyright and permission notices to be included
//! with copies of the program. [`generate`] writes, for one [`Artefact`], the notice of each
//! crate compiled into it: the crates of its resolved graph of normal dependencies (`cargo
//! tree --locked --offline -e normal,no-proc-macro`, the same graph as
//! [`crate::workspace::dependencies`], without the build dependencies and the procedural
//! macros, which run on the build machine and are not linked), for its target, without
//! the members of the workspace. For each crate, its licence expression, the licence
//! chosen from it ([`choose`]: MIT where there is a choice), its authors as its manifest
//! names them, and the text of each chosen licence, read from the crate's own package in
//! the local cargo registry (`cargo metadata --locked --offline` gives its directory).
//!
//! The files are committed (`ARTEFACTS`); the tests of `einvoice-cli` and
//! `einvoice-wasm` regenerate them and fail on a difference, so a change of the
//! dependency graph needs them regenerated:
//!
//! ```text
//! cargo run --locked -p einvoice-testkit --bin third-party-notices -- --write
//! ```

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::json::{self, Value};
use crate::workspace;

/// A program the project distributes, and the file of its third-party notices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Artefact {
    /// The package it is built from, with its default features.
    pub package: &'static str,
    /// What the notices are for, as their title names it.
    pub program: &'static str,
    /// The target platform it is built for (`cargo tree --target`): a triple, or `all`
    /// for a program built on any platform.
    pub target: &'static str,
    /// The notices file, relative to the root of the workspace.
    pub file: &'static str,
}

/// The `einvoice-validate` binary, built on any platform.
pub const CLI: Artefact = Artefact {
    package: "einvoice-cli",
    program: "einvoice-validate (crate einvoice-cli), on any target",
    target: "all",
    file: "crates/einvoice-cli/THIRD-PARTY-NOTICES",
};

/// The npm package `einvoice-validator`: the WebAssembly module of `einvoice-wasm`.
pub const NPM: Artefact = Artefact {
    package: "einvoice-wasm",
    program: "the npm package einvoice-validator (the WebAssembly module of crate \
              einvoice-wasm, wasm32-unknown-unknown)",
    target: "wasm32-unknown-unknown",
    file: "crates/einvoice-wasm/THIRD-PARTY-NOTICES",
};

/// Every artefact with third-party notices.
pub const ARTEFACTS: [Artefact; 2] = [CLI, NPM];

/// The licences a crate may be used under here, each with a phrase of its text that
/// identifies its file in a package (compared with the white space collapsed). A crate
/// whose licence expression offers none of them, alone or with others of them, is
/// refused: using it is a decision to record (in `deny.toml` as well).
const KNOWN: [(&str, &str); 2] = [
    (
        "MIT",
        "Permission is hereby granted, free of charge, to any person obtaining a copy",
    ),
    ("Unicode-3.0", "UNICODE LICENSE V3"),
];

/// The licence text of a crate, from its package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Text {
    /// The licence, such as `MIT`.
    pub licence: String,
    /// The file of the package it is read from, such as `LICENSE-MIT`.
    pub file: String,
    /// The text, with LF line ends, ending with a line end.
    pub text: String,
}

/// A third-party crate compiled into an artefact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThirdParty {
    pub name: String,
    pub version: String,
    /// Its licence expression, as its manifest writes it.
    pub license: String,
    /// The licences chosen from it, all of which apply (joined by AND).
    pub chosen: Vec<String>,
    /// Its authors, as its manifest names them.
    pub authors: Vec<String>,
    /// Its repository, as its manifest names it.
    pub repository: Option<String>,
    /// The text of each chosen licence, in the order of `chosen`.
    pub texts: Vec<Text>,
}

/// The terms of a licence expression in disjunctive normal form: one set of terms per
/// alternative, all of a set applying together. `AND` binds tighter than `OR`.
///
/// # Errors
///
/// When the expression is not made of licence identifiers, `AND`, `OR` and parentheses
/// (`WITH` and `+` are not expected here).
pub fn alternatives(expression: &str) -> Result<Vec<BTreeSet<String>>, String> {
    let spaced = expression.replace('(', " ( ").replace(')', " ) ");
    let tokens: Vec<&str> = spaced.split_whitespace().collect();
    let mut at = 0;
    let result = or(&tokens, &mut at, 0)?;
    if at == tokens.len() {
        Ok(result)
    } else {
        Err(format!(
            "licence expression {expression:?}: unexpected {:?}",
            tokens[at]
        ))
    }
}

type Dnf = Vec<BTreeSet<String>>;

fn or(tokens: &[&str], at: &mut usize, depth: usize) -> Result<Dnf, String> {
    let mut result = and(tokens, at, depth)?;
    while tokens.get(*at) == Some(&"OR") {
        *at += 1;
        result.extend(and(tokens, at, depth)?);
    }
    Ok(result)
}

fn and(tokens: &[&str], at: &mut usize, depth: usize) -> Result<Dnf, String> {
    let mut result = primary(tokens, at, depth)?;
    while tokens.get(*at) == Some(&"AND") {
        *at += 1;
        let right = primary(tokens, at, depth)?;
        let mut product = Vec::new();
        for left in &result {
            for r in &right {
                product.push(left.union(r).cloned().collect());
            }
        }
        result = product;
    }
    Ok(result)
}

fn primary(tokens: &[&str], at: &mut usize, depth: usize) -> Result<Dnf, String> {
    let token = *tokens
        .get(*at)
        .ok_or("licence expression: a licence expected at the end")?;
    *at += 1;
    if token == "(" {
        if depth > 16 {
            return Err("licence expression: parentheses nested too deep".to_owned());
        }
        let inner = or(tokens, at, depth + 1)?;
        if tokens.get(*at) != Some(&")") {
            return Err("licence expression: ( without )".to_owned());
        }
        *at += 1;
        return Ok(inner);
    }
    let identifier = !token.is_empty()
        && !matches!(token, "AND" | "OR" | "WITH" | ")")
        && token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-".contains(&b));
    if identifier {
        Ok(vec![BTreeSet::from([token.to_owned()])])
    } else {
        Err(format!("licence expression: unexpected {token:?}"))
    }
}

/// The licences to use a crate under, from its licence expression: the alternative made
/// only of licences of `KNOWN`, the one with the fewest terms (the first of those), in
/// the order of `KNOWN`. `MIT OR Apache-2.0` gives `MIT`; `(MIT OR Apache-2.0) AND
/// Unicode-3.0` gives `MIT`, `Unicode-3.0`.
///
/// # Errors
///
/// When the expression is not valid, or no alternative is made of known licences only.
pub fn choose(expression: &str) -> Result<Vec<String>, String> {
    let known = |term: &String| KNOWN.iter().any(|(id, _)| id == term);
    let chosen = alternatives(expression)?
        .into_iter()
        .filter(|terms| terms.iter().all(known))
        .min_by_key(BTreeSet::len)
        .ok_or_else(|| {
            format!("licence expression {expression:?}: no alternative of known licences")
        })?;
    Ok(KNOWN
        .iter()
        .map(|(id, _)| (*id).to_owned())
        .filter(|id| chosen.contains(id))
        .collect())
}

/// `text` with every run of white space made one space.
fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The text of `licence` among `files` (name, content) of a package root: the licence
/// files (named LICENSE, LICENCE or COPYING, then anything) holding its phrase; of
/// several, those whose name has the licence's name (`MIT`, `UNICODE`), then the first
/// by name.
///
/// # Errors
///
/// When the licence is not known, or no file holds its text.
pub fn licence_text(licence: &str, files: &[(String, String)]) -> Result<Text, String> {
    let phrase = KNOWN
        .iter()
        .find(|(id, _)| *id == licence)
        .map(|(_, phrase)| *phrase)
        .ok_or_else(|| format!("{licence}: not a known licence"))?;
    let key = licence
        .split(['-', '.'])
        .next()
        .unwrap_or(licence)
        .to_ascii_uppercase();
    let mut matches: Vec<&(String, String)> = files
        .iter()
        .filter(|(name, content)| {
            let upper = name.to_ascii_uppercase();
            ["LICENSE", "LICENCE", "COPYING"]
                .iter()
                .any(|p| upper.starts_with(p))
                && collapse(content).contains(phrase)
        })
        .collect();
    matches.sort_by_key(|(name, _)| (!name.to_ascii_uppercase().contains(&key), name.clone()));
    let (file, content) = matches
        .first()
        .ok_or_else(|| format!("no {licence} text among the licence files of the package"))?;
    let mut text = content.replace("\r\n", "\n");
    if !text.ends_with('\n') {
        text.push('\n');
    }
    Ok(Text {
        licence: licence.to_owned(),
        file: file.clone(),
        text,
    })
}

/// The files at the root of a package directory, with their contents, sorted by name.
fn root_files(dir: &Path) -> Result<Vec<(String, String)>, String> {
    let mut files = Vec::new();
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let upper = name.to_ascii_uppercase();
        let licence_like = ["LICENSE", "LICENCE", "COPYING"]
            .iter()
            .any(|p| upper.starts_with(p));
        if !licence_like || !entry.path().is_file() {
            continue;
        }
        let content = std::fs::read_to_string(entry.path())
            .map_err(|e| format!("{}: {e}", entry.path().display()))?;
        files.push((name, content));
    }
    files.sort();
    Ok(files)
}

/// Cargo: the one running the tests (`CARGO`), else `cargo` on the `PATH`.
fn cargo() -> Command {
    Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
}

/// A package of `cargo metadata`, as the notices need it.
struct Package {
    name: String,
    version: String,
    license: Option<String>,
    authors: Vec<String>,
    repository: Option<String>,
    /// Its source (`registry+...`), none for a path package.
    source: Option<String>,
    dir: PathBuf,
}

/// Every package of the resolve, from `cargo metadata --locked --offline`.
fn metadata() -> Result<Vec<Package>, String> {
    let output = cargo()
        .arg("metadata")
        .arg("--manifest-path")
        .arg(workspace::root().join("Cargo.toml"))
        .args(["--locked", "--offline", "--format-version", "1"])
        .output()
        .map_err(|e| format!("cargo metadata: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "cargo metadata: {}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let text = String::from_utf8(output.stdout).map_err(|e| format!("cargo metadata: {e}"))?;
    let document = json::parse(&text)?;
    let field = |p: &Value, key: &str| p.get(key).and_then(Value::as_str).map(str::to_owned);
    let mut packages = Vec::new();
    for p in document
        .get("packages")
        .and_then(Value::as_array)
        .ok_or("cargo metadata: no packages")?
    {
        let name = field(p, "name").ok_or("cargo metadata: a package without a name")?;
        let version = field(p, "version").ok_or_else(|| format!("{name}: no version"))?;
        let manifest = field(p, "manifest_path").ok_or_else(|| format!("{name}: no manifest"))?;
        let dir = Path::new(&manifest)
            .parent()
            .ok_or_else(|| format!("{name}: manifest {manifest:?}"))?
            .to_path_buf();
        let authors = p
            .get("authors")
            .and_then(Value::as_array)
            .unwrap_or_default()
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
        packages.push(Package {
            license: field(p, "license"),
            repository: field(p, "repository"),
            source: field(p, "source"),
            authors,
            name,
            version,
            dir,
        });
    }
    Ok(packages)
}

/// The names and versions of the packages in the graph of normal dependencies of
/// `package` (default features) on `target`, without procedural macros, sorted.
fn graph(package: &str, target: &str) -> Result<BTreeSet<(String, String)>, String> {
    let lines = workspace::tree(&[
        "--package",
        package,
        "--edges",
        "normal,no-proc-macro",
        "--target",
        target,
        "--format",
        "{p}",
    ])?;
    let mut out = BTreeSet::new();
    for line in lines {
        let (name, rest) = line
            .split_once(" v")
            .ok_or_else(|| format!("cargo tree: not a package: {line:?}"))?;
        let version = rest.split(' ').next().unwrap_or(rest);
        out.insert((name.to_owned(), version.to_owned()));
    }
    Ok(out)
}

/// The third-party crates compiled into `artefact`, sorted by name and version, each
/// with its licence chosen and its texts.
///
/// # Errors
///
/// When cargo fails, a crate is not from a registry, or its licence cannot be used
/// ([`choose`]) or has no text in its package ([`licence_text`]).
pub fn third_party(artefact: &Artefact) -> Result<Vec<ThirdParty>, String> {
    let members = workspace::members()?;
    let packages = metadata()?;
    let mut crates = Vec::new();
    for (name, version) in graph(artefact.package, artefact.target)? {
        if members.contains(&name) {
            continue;
        }
        let p = packages
            .iter()
            .find(|p| p.name == name && p.version == version)
            .ok_or_else(|| format!("{name} {version}: not in cargo metadata"))?;
        if !p
            .source
            .as_deref()
            .is_some_and(|s| s.starts_with("registry+"))
        {
            return Err(format!("{name} {version}: not from a registry"));
        }
        let license = p
            .license
            .clone()
            .ok_or_else(|| format!("{name} {version}: no licence expression"))?;
        let chosen = choose(&license).map_err(|e| format!("{name} {version}: {e}"))?;
        let files = root_files(&p.dir)?;
        let texts = chosen
            .iter()
            .map(|l| licence_text(l, &files).map_err(|e| format!("{name} {version}: {e}")))
            .collect::<Result<_, _>>()?;
        crates.push(ThirdParty {
            name,
            version,
            license,
            chosen,
            authors: p.authors.clone(),
            repository: p.repository.clone(),
            texts,
        });
    }
    Ok(crates)
}

/// The command that regenerates the notices files.
pub const REGENERATE: &str =
    "cargo run --locked -p einvoice-testkit --bin third-party-notices -- --write";

/// `text` on lines of at most 80 characters (but for longer words), broken at spaces.
fn wrap(text: &str) -> String {
    let mut out = String::new();
    let mut line = 0;
    for word in text.split_whitespace() {
        if line > 0 && line + 1 + word.chars().count() > 80 {
            out.push('\n');
            line = 0;
        } else if line > 0 {
            out.push(' ');
            line += 1;
        }
        out.push_str(word);
        line += word.chars().count();
    }
    out
}

/// The notices of `artefact` for `crates`, as its file holds them.
#[must_use]
pub fn render(artefact: &Artefact, crates: &[ThirdParty]) -> String {
    let targets = if artefact.target == "all" {
        "every target".to_owned()
    } else {
        format!("the target {}", artefact.target)
    };
    let package = artefact.package;
    let about = format!(
        "The crates from crates.io compiled into it: the normal dependencies of {package} \
         with its default features, as Cargo.lock resolves them for {targets}, without the \
         procedural macros and the crates of this project (cargo tree -p {package} -e \
         normal,no-proc-macro --target {target}). Each crate is used under the licence \
         shown, chosen from its licence expression where it offers a choice; its notice \
         follows: its authors as its manifest names them, then the text of that licence \
         as its package holds it.",
        target = artefact.target,
    );
    let mut out = format!(
        "{title}\n\n{about}\n\nGenerated from Cargo.lock and the packages of the crates \
         by\n{REGENERATE}\nDo not edit.\n\nCrates ({count}):\n",
        title = wrap(&format!("Third-party notices of {}", artefact.program)),
        about = wrap(&about),
        count = crates.len(),
    );
    let width = crates
        .iter()
        .map(|c| c.name.len() + c.version.len() + 1)
        .max()
        .unwrap_or(0);
    for c in crates {
        let id = format!("{} {}", c.name, c.version);
        let _ = writeln!(out, "  {id:<width$}  {}", c.chosen.join(" AND "));
    }
    for c in crates {
        let _ = write!(
            out,
            "\n== {} {}\nLicence: {}",
            c.name,
            c.version,
            c.chosen.join(" AND ")
        );
        if c.chosen.join(" AND ") != c.license {
            let _ = write!(out, " (chosen from {})", c.license);
        }
        out.push('\n');
        if !c.authors.is_empty() {
            let _ = writeln!(out, "Authors: {}", c.authors.join(", "));
        }
        if let Some(repository) = &c.repository {
            let _ = writeln!(out, "Repository: {repository}");
        }
        for t in &c.texts {
            let _ = write!(out, "\n-- {}, from {}\n\n{}", t.licence, t.file, t.text);
        }
    }
    out
}

/// The notices of `artefact`, generated afresh.
///
/// # Errors
///
/// As [`third_party`].
pub fn generate(artefact: &Artefact) -> Result<String, String> {
    Ok(render(artefact, &third_party(artefact)?))
}

/// The path of the notices file of `artefact`.
#[must_use]
pub fn path(artefact: &Artefact) -> PathBuf {
    workspace::root().join(artefact.file)
}

/// Whether the committed notices file of `artefact` is the one [`generate`] writes.
///
/// # Errors
///
/// When it is not (or cannot be read), or the generation fails.
pub fn check(artefact: &Artefact) -> Result<(), String> {
    let fresh = generate(artefact)?;
    let path = path(artefact);
    let committed =
        std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", artefact.file))?;
    if committed == fresh {
        Ok(())
    } else {
        // The first line that differs, or the first line one of them lacks.
        let line = committed
            .lines()
            .zip(fresh.lines())
            .position(|(a, b)| a != b)
            .unwrap_or_else(|| committed.lines().count().min(fresh.lines().count()));
        Err(format!(
            "{}: not up to date with Cargo.lock (first difference at line {}): run {REGENERATE}",
            artefact.file,
            line + 1
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{ARTEFACTS, alternatives, choose, collapse, licence_text, render};
    use std::collections::BTreeSet;

    fn set(terms: &[&str]) -> BTreeSet<String> {
        terms.iter().map(|t| (*t).to_owned()).collect()
    }

    #[test]
    fn expressions_in_disjunctive_normal_form() {
        assert_eq!(alternatives("MIT").unwrap(), [set(&["MIT"])]);
        assert_eq!(
            alternatives("MIT OR Apache-2.0").unwrap(),
            [set(&["MIT"]), set(&["Apache-2.0"])]
        );
        assert_eq!(
            alternatives("(MIT OR Apache-2.0) AND Unicode-3.0").unwrap(),
            [
                set(&["MIT", "Unicode-3.0"]),
                set(&["Apache-2.0", "Unicode-3.0"])
            ]
        );
        // AND binds tighter than OR.
        assert_eq!(
            alternatives("A OR B AND C").unwrap(),
            [set(&["A"]), set(&["B", "C"])]
        );
        assert_eq!(
            alternatives("(A OR B) AND (C OR D)").unwrap(),
            [
                set(&["A", "C"]),
                set(&["A", "D"]),
                set(&["B", "C"]),
                set(&["B", "D"])
            ]
        );
        for bad in [
            "",
            "MIT OR",
            "(MIT",
            "MIT)",
            "MIT WITH X",
            "GPL-2.0+",
            "AND MIT",
            "A B",
        ] {
            assert!(alternatives(bad).is_err(), "{bad:?}");
        }
        let deep = "(".repeat(40) + "MIT" + &")".repeat(40);
        assert!(alternatives(&deep).is_err());
    }

    #[test]
    fn mit_is_chosen_where_offered() {
        assert_eq!(choose("MIT").unwrap(), ["MIT"]);
        assert_eq!(choose("Apache-2.0 OR MIT").unwrap(), ["MIT"]);
        assert_eq!(choose("0BSD OR MIT OR Apache-2.0").unwrap(), ["MIT"]);
        assert_eq!(choose("Unlicense OR MIT").unwrap(), ["MIT"]);
        assert_eq!(
            choose("(MIT OR Apache-2.0) AND Unicode-3.0").unwrap(),
            ["MIT", "Unicode-3.0"]
        );
        assert_eq!(choose("MIT OR (MIT AND Unicode-3.0)").unwrap(), ["MIT"]);
        assert!(choose("Apache-2.0").is_err());
        assert!(choose("MIT AND Apache-2.0").is_err());
        assert!(choose("MIT OR").is_err());
    }

    const MIT: &str = "MIT License\r\n\r\nCopyright (c) 2020 Someone\r\n\r\nPermission is \
                       hereby granted, free of charge, to any person obtaining a\r\ncopy of \
                       this software";

    fn files(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter()
            .map(|(n, c)| ((*n).to_owned(), (*c).to_owned()))
            .collect()
    }

    #[test]
    fn licence_texts_are_found_by_their_content() {
        // Line ends made LF, a line end added; the phrase found across a line break.
        let t = licence_text("MIT", &files(&[("LICENSE", MIT)])).unwrap();
        assert_eq!(t.file, "LICENSE");
        assert!(
            t.text
                .starts_with("MIT License\n\nCopyright (c) 2020 Someone\n")
        );
        assert!(t.text.ends_with("this software\n") && !t.text.contains('\r'));
        // The file named after the licence first; an expression-only LICENSE is no text.
        let both = files(&[
            ("COPYING", "dual-licensed under the Unlicense and MIT"),
            ("LICENSE", MIT),
            ("LICENSE-MIT.md", MIT),
            ("LICENSE-APACHE", "Apache License"),
        ]);
        assert_eq!(licence_text("MIT", &both).unwrap().file, "LICENSE-MIT.md");
        let unicode = files(&[
            ("LICENSE-MIT", MIT),
            ("LICENSE-UNICODE", "UNICODE LICENSE V3\n"),
        ]);
        assert_eq!(
            licence_text("Unicode-3.0", &unicode).unwrap().file,
            "LICENSE-UNICODE"
        );
        assert!(licence_text("MIT", &files(&[("LICENSE", "MIT OR Apache-2.0")])).is_err());
        assert!(licence_text("MIT", &files(&[("README", MIT)])).is_err());
        assert!(licence_text("Apache-2.0", &both).is_err());
    }

    #[test]
    fn paragraphs_are_wrapped_at_80_characters() {
        assert_eq!(super::wrap("a  b\nc"), "a b c");
        let text = "word ".repeat(40);
        let wrapped = super::wrap(&text);
        assert!(wrapped.lines().all(|l| l.len() <= 80 && !l.ends_with(' ')));
        assert_eq!(wrapped.lines().next().unwrap().len(), 79);
        assert_eq!(collapse(&wrapped), collapse(&text));
        let long = "x".repeat(90);
        assert_eq!(super::wrap(&format!("a {long} b")), format!("a\n{long}\nb"));
    }

    #[test]
    fn notices_list_then_give_each_crate() {
        let crates = vec![super::ThirdParty {
            name: "a".into(),
            version: "1.0.0".into(),
            license: "MIT OR Apache-2.0".into(),
            chosen: vec!["MIT".into()],
            authors: vec!["A <a@example.org>".into()],
            repository: Some("https://example.org/a".into()),
            texts: vec![super::Text {
                licence: "MIT".into(),
                file: "LICENSE-MIT".into(),
                text: "Copyright A\n".into(),
            }],
        }];
        let text = render(&ARTEFACTS[0], &crates);
        assert!(text.starts_with("Third-party notices of einvoice-validate"));
        assert!(text.contains("\nCrates (1):\n  a 1.0.0  MIT\n"));
        assert!(text.ends_with(
            "\n== a 1.0.0\nLicence: MIT (chosen from MIT OR Apache-2.0)\nAuthors: A \
             <a@example.org>\nRepository: https://example.org/a\n\n-- MIT, from \
             LICENSE-MIT\n\nCopyright A\n"
        ));
    }
}
