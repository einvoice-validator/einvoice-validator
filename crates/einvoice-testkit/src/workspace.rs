//! The workspace crates a package is built from.
//!
//! [`dependencies`] asks `cargo tree` (with `--locked --offline`) for the resolved
//! dependency graph of one package, its normal and build dependencies on every target,
//! for one set of features, and keeps the workspace members in it. `cargo tree` rather
//! than `cargo metadata`: the metadata resolves the features of the whole workspace at
//! once, so it cannot give the graph of one package built without its default features
//! while another member depends on it with them.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A workspace crate in the dependency graph of a package.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Crate {
    /// The package name, such as `einvoice-xsd`.
    pub name: String,
    /// Its licence expression (`license` in its manifest), such as `MIT`.
    pub license: String,
    /// The directory of its manifest.
    pub dir: PathBuf,
}

impl Crate {
    /// The terms of the licence expression, which joins them with `AND` only.
    ///
    /// # Errors
    ///
    /// When the expression uses anything but `AND` (`OR`, `WITH`, parentheses).
    pub fn license_terms(&self) -> Result<Vec<&str>, String> {
        let terms: Vec<&str> = self.license.split(" AND ").collect();
        if terms.iter().all(|t| {
            !t.is_empty()
                && t.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".-+".contains(&b))
        }) {
            Ok(terms)
        } else {
            Err(format!(
                "{}: licence expression {:?}: only terms joined by AND are expected",
                self.name, self.license
            ))
        }
    }
}

/// The features a package is resolved with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Features<'a> {
    /// Its default features.
    Default,
    /// Exactly these features, the default ones off.
    Only(&'a [&'a str]),
}

/// The root of the workspace.
#[must_use]
pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .map(Path::to_path_buf)
        .unwrap_or_default()
}

/// Cargo: the one running the tests (`CARGO`), else `cargo` on the `PATH`.
fn cargo() -> Command {
    Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
}

/// `cargo tree --locked --offline` in the workspace with `args`, one line per package.
pub(crate) fn tree(args: &[&str]) -> Result<Vec<String>, String> {
    let output = cargo()
        .arg("tree")
        .arg("--manifest-path")
        .arg(root().join("Cargo.toml"))
        .args(["--locked", "--offline", "--prefix", "none"])
        .args(args)
        .output()
        .map_err(|e| format!("cargo tree {args:?}: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "cargo tree {args:?}: {}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let stdout =
        String::from_utf8(output.stdout).map_err(|e| format!("cargo tree {args:?}: {e}"))?;
    Ok(stdout
        .lines()
        .map(|line| line.strip_suffix(" (*)").unwrap_or(line).to_owned())
        .filter(|line| !line.is_empty())
        .collect())
}

/// The name and the directory of a package as `cargo tree` writes it (`{p}`): `NAME vVERSION`,
/// then ` (DIR)` for a path package.
fn package(text: &str) -> Result<(String, Option<PathBuf>), String> {
    let (name, rest) = text
        .split_once(" v")
        .ok_or_else(|| format!("cargo tree: not a package: {text:?}"))?;
    let dir = rest
        .split_once(" (")
        .and_then(|(_, dir)| dir.strip_suffix(')'))
        .map(PathBuf::from);
    Ok((name.to_owned(), dir))
}

/// The names of the members of the workspace.
///
/// # Errors
///
/// When `cargo tree` fails or writes something unexpected.
pub fn members() -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    for line in tree(&["--workspace", "--depth", "0", "--format", "{p}"])? {
        names.push(package(&line)?.0);
    }
    names.sort();
    names.dedup();
    Ok(names)
}

/// The workspace crates `package` is built from, itself included, with `features`: the
/// members in its resolved graph of normal and build dependencies, on every target,
/// sorted by name.
///
/// # Errors
///
/// When `cargo tree` fails or writes something unexpected, or a member of the graph is
/// not a path package.
pub fn dependencies(package_name: &str, features: Features<'_>) -> Result<Vec<Crate>, String> {
    let members = members()?;
    let mut args = vec![
        "--package",
        package_name,
        "--edges",
        "normal,build",
        "--target",
        "all",
        "--format",
        "{l}|{p}",
    ];
    let list;
    if let Features::Only(only) = features {
        list = only.join(",");
        args.push("--no-default-features");
        if !only.is_empty() {
            args.extend(["--features", list.as_str()]);
        }
    }
    let mut crates: Vec<Crate> = Vec::new();
    for line in tree(&args)? {
        let (license, text) = line
            .split_once('|')
            .ok_or_else(|| format!("cargo tree: no licence: {line:?}"))?;
        let (name, dir) = package(text)?;
        if !members.contains(&name) || crates.iter().any(|c| c.name == name) {
            continue;
        }
        let dir = dir.ok_or_else(|| format!("{name}: a member that is not a path package"))?;
        crates.push(Crate {
            name,
            license: license.to_owned(),
            dir,
        });
    }
    crates.sort();
    if !crates.iter().any(|c| c.name == package_name) {
        return Err(format!("{package_name}: not in its own graph"));
    }
    Ok(crates)
}

/// The names in the last parenthesised list of `text`, separated by `, `: what a licence
/// notice such as "the code of a and of the crates it is built from (b, c)" names.
///
/// # Errors
///
/// When `text` has no such list.
pub fn named_crates(text: &str) -> Result<Vec<String>, String> {
    let open = text
        .rfind('(')
        .ok_or_else(|| format!("no parenthesised list in {text:?}"))?;
    let close = text[open..]
        .find(')')
        .ok_or_else(|| format!("unclosed list in {text:?}"))?;
    let mut names: Vec<String> = text[open + 1..open + close]
        .split(", ")
        .map(str::to_owned)
        .collect();
    names.sort();
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::{Crate, Features, dependencies, members, named_crates, package};

    #[test]
    fn reads_cargo_tree_packages() {
        assert_eq!(
            package("einvoice-xsd v0.2.0 (/w/crates/einvoice-xsd)").unwrap(),
            (
                "einvoice-xsd".to_owned(),
                Some("/w/crates/einvoice-xsd".into())
            )
        );
        assert_eq!(
            package("memchr v2.8.3").unwrap(),
            ("memchr".to_owned(), None)
        );
        assert!(package("memchr").is_err());
    }

    #[test]
    fn reads_the_list_of_a_notice() {
        assert_eq!(
            named_crates("the code of a (crate b) and of the crates it is built from (d, c)")
                .unwrap(),
            ["c", "d"]
        );
        assert!(named_crates("no list").is_err());
    }

    #[test]
    fn licence_terms_are_joined_by_and() {
        let with = |license: &str| Crate {
            name: "x".into(),
            license: license.into(),
            dir: ".".into(),
        };
        assert_eq!(
            with("MIT AND EUPL-1.2").license_terms().unwrap(),
            ["MIT", "EUPL-1.2"]
        );
        assert!(with("MIT OR Apache-2.0").license_terms().is_err());
        assert!(with("(MIT AND X)").license_terms().is_err());
    }

    #[test]
    fn this_crate_is_built_from_itself_only() {
        assert!(members().unwrap().contains(&"einvoice-testkit".to_owned()));
        let deps = dependencies("einvoice-testkit", Features::Default).unwrap();
        let names: Vec<&str> = deps.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["einvoice-testkit"]);
        assert_eq!(deps[0].license, "MIT");
        assert!(deps[0].dir.join("Cargo.toml").is_file());
    }
}
