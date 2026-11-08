//! The supply-chain checks of the programs and library the project distributes: what the
//! third-party crates of their graphs contain beyond Rust, against the audit of
//! `docs/dependencies.md`.
//!
//! The graphs ([`SHIPPED`]) are those of `einvoice-cli` (its default features, and all of
//! them), of `einvoice-validator` with all its features, and of `einvoice-wasm` for
//! WebAssembly: normal and build dependencies (`cargo tree --locked --offline -e
//! normal,build`), procedural macros included (they run on the build machine), on every
//! target. Each third-party crate of them, read from its package in the local cargo
//! registry, must:
//!
//! - ship no C, C++ or assembly source, object file or library ([`NATIVE_EXTENSIONS`]);
//! - have no build dependency that builds native code ([`NATIVE_BUILD_CRATES`], also
//!   denied by `deny.toml`), and no build script that compiles or links native code
//!   ([`NATIVE_BUILD_MARKERS`]);
//! - be listed in the table of crates of `docs/dependencies.md`, its build script (if it
//!   has one) in the table of build scripts, its inline assembly (`asm!`, `global_asm!`,
//!   `naked_asm!`, if any) in the table of inline assembly: a new crate, a new build
//!   script or new inline assembly fails the check until it is audited there, and an
//!   entry that no longer matches the graph fails it too.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::json::{self, Value};
use crate::workspace;

/// A graph of crates the project distributes: a package, its features, its target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Graph {
    pub package: &'static str,
    /// The `cargo tree` arguments that select its features.
    pub features: &'static [&'static str],
    /// The target (`cargo tree --target`): a triple, or `all`.
    pub target: &'static str,
}

/// The graphs checked: the binary (default features, then all), the library with every
/// feature, the WebAssembly module.
pub const SHIPPED: [Graph; 4] = [
    Graph {
        package: "einvoice-cli",
        features: &[],
        target: "all",
    },
    Graph {
        package: "einvoice-cli",
        features: &["--all-features"],
        target: "all",
    },
    Graph {
        package: "einvoice-validator",
        features: &["--all-features"],
        target: "all",
    },
    Graph {
        package: "einvoice-wasm",
        features: &["--all-features"],
        target: "wasm32-unknown-unknown",
    },
];

/// The extensions of the files no package of the graphs may ship: C, C++ and assembly
/// sources and headers, object files and libraries (compared without case, but for the
/// assembly `.S` and `.s`, both refused).
pub const NATIVE_EXTENSIONS: [&str; 20] = [
    "c", "cc", "cpp", "cxx", "c++", "h", "hh", "hpp", "hxx", "s", "asm", "nasm", "inc", "o", "obj",
    "a", "lib", "so", "dylib", "dll",
];

/// The crates that build or link native code from a build script (also in the `[bans]`
/// of `deny.toml`): none may be a build dependency in the graphs.
pub const NATIVE_BUILD_CRATES: [&str; 23] = [
    "autotools",
    "aws-lc-fips-sys",
    "aws-lc-rs",
    "aws-lc-sys",
    "bindgen",
    "boring-sys",
    "bzip2-sys",
    "cc",
    "cmake",
    "cxx",
    "cxx-build",
    "libz-sys",
    "lzma-sys",
    "nasm-rs",
    "openssl-src",
    "openssl-sys",
    "pkg-config",
    "ring",
    "sha1-asm",
    "sha2-asm",
    "system-deps",
    "vcpkg",
    "zstd-sys",
];

/// Text in a build script that compiles or links native code: the APIs of the crates
/// above, compilers, and the cargo instructions that link a native library.
pub const NATIVE_BUILD_MARKERS: [&str; 13] = [
    "cc::",
    "cmake::",
    "bindgen::",
    "pkg_config::",
    "vcpkg::",
    "cxx_build::",
    "nasm",
    "\"gcc\"",
    "\"clang\"",
    "\"cl.exe\"",
    "rustc-link-lib",
    "rustc-link-search",
    "rustc-cdylib-link-arg",
];

/// A third-party crate of the graphs, and what it holds beyond Rust.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Audited {
    pub name: String,
    pub version: String,
    /// The graphs it is in, by index in [`SHIPPED`].
    pub graphs: BTreeSet<usize>,
    /// Its build script, relative to the package, if any.
    pub build_script: Option<String>,
    /// The files with inline assembly, relative to the package, with the count of each.
    pub inline_asm: Vec<(String, usize)>,
    /// The files with a native extension, relative to the package.
    pub native_files: Vec<String>,
    /// Its build dependencies that build native code.
    pub native_build_dependencies: Vec<String>,
    /// The markers of native builds found in its build script.
    pub native_build_markers: Vec<String>,
    /// The number of `unsafe` tokens in its Rust sources (comments included: a measure of
    /// where to look, not an exact count of unsafe blocks).
    pub unsafe_tokens: usize,
}

/// Cargo: the one running the tests (`CARGO`), else `cargo` on the `PATH`.
fn cargo() -> Command {
    Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
}

/// A package of `cargo metadata`, as the audit needs it.
struct Package {
    source: Option<String>,
    dir: PathBuf,
    build_script: Option<PathBuf>,
    build_dependencies: Vec<String>,
}

/// The packages of `cargo metadata --locked --offline`, by name and version.
fn metadata() -> Result<BTreeMap<(String, String), Package>, String> {
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
    let mut packages = BTreeMap::new();
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
        let build_script = p
            .get("targets")
            .and_then(Value::as_array)
            .unwrap_or_default()
            .iter()
            .find(|t| {
                t.get("kind")
                    .and_then(Value::as_array)
                    .unwrap_or_default()
                    .iter()
                    .any(|k| k.as_str() == Some("custom-build"))
            })
            .and_then(|t| field(t, "src_path"))
            .map(PathBuf::from);
        let build_dependencies = p
            .get("dependencies")
            .and_then(Value::as_array)
            .unwrap_or_default()
            .iter()
            .filter(|d| d.get("kind").and_then(Value::as_str) == Some("build"))
            .filter_map(|d| field(d, "name"))
            .collect();
        packages.insert(
            (name, version),
            Package {
                source: field(p, "source"),
                dir,
                build_script,
                build_dependencies,
            },
        );
    }
    Ok(packages)
}

/// The names and versions of the packages of `graph`, sorted.
fn graph(graph: &Graph) -> Result<BTreeSet<(String, String)>, String> {
    let mut args = vec!["--package", graph.package];
    args.extend_from_slice(graph.features);
    args.extend_from_slice(&[
        "--edges",
        "normal,build",
        "--target",
        graph.target,
        "--format",
        "{p}",
    ]);
    let mut out = BTreeSet::new();
    for line in workspace::tree(&args)? {
        let (name, rest) = line
            .split_once(" v")
            .ok_or_else(|| format!("cargo tree: not a package: {line:?}"))?;
        let version = rest.split(' ').next().unwrap_or(rest);
        out.insert((name.to_owned(), version.to_owned()));
    }
    Ok(out)
}

/// Every file under `dir`, sorted (symbolic links are not followed).
fn files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(next) = pending.pop() {
        let entries = std::fs::read_dir(&next).map_err(|e| format!("{}: {e}", next.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("{}: {e}", next.display()))?;
            let kind = entry
                .file_type()
                .map_err(|e| format!("{}: {e}", entry.path().display()))?;
            if kind.is_dir() {
                pending.push(entry.path());
            } else if kind.is_file() {
                out.push(entry.path());
            }
        }
    }
    out.sort();
    Ok(out)
}

/// The number of inline assembly invocations in Rust source `text`: `asm!`,
/// `global_asm!` and `naked_asm!` followed by a delimiter (a mention in a comment, such
/// as ``[`core::arch::asm!`]``, is not one).
#[must_use]
pub fn inline_asm_count(text: &str) -> usize {
    let bytes = text.as_bytes();
    let mut count = 0;
    let mut from = 0;
    while let Some(at) = text[from..].find("asm!") {
        let start = from + at;
        from = start + 4;
        // The macro's name: `asm`, or `global_asm` / `naked_asm`, as a whole identifier.
        let before = &text[..start];
        let name_start = ["global_", "naked_", ""]
            .iter()
            .find_map(|p| before.strip_suffix(p).map(str::len))
            .unwrap_or(start);
        let identifier_before = name_start
            .checked_sub(1)
            .and_then(|i| bytes.get(i))
            .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_');
        if identifier_before {
            continue;
        }
        let after = text[from..].trim_start();
        if after.starts_with(['(', '{', '[']) {
            count += 1;
        }
    }
    count
}

/// The number of `unsafe` tokens in Rust source `text`.
#[must_use]
pub fn unsafe_count(text: &str) -> usize {
    let bytes = text.as_bytes();
    text.match_indices("unsafe")
        .filter(|(at, _)| {
            let before = at
                .checked_sub(1)
                .and_then(|i| bytes.get(i))
                .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_');
            let after = bytes
                .get(at + "unsafe".len())
                .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_');
            !before && !after
        })
        .count()
}

/// Whether `path` has a native extension ([`NATIVE_EXTENSIONS`]).
#[must_use]
pub fn is_native(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| NATIVE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

fn relative(path: &Path, dir: &Path) -> String {
    path.strip_prefix(dir)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Every third-party crate of the [`SHIPPED`] graphs, sorted by name and version, with
/// what its package holds beyond Rust.
///
/// # Errors
///
/// When cargo fails, a crate is not from a registry, or its package cannot be read.
pub fn audit() -> Result<Vec<Audited>, String> {
    let members = workspace::members()?;
    let packages = metadata()?;
    let mut graphs: BTreeMap<(String, String), BTreeSet<usize>> = BTreeMap::new();
    for (i, g) in SHIPPED.iter().enumerate() {
        for key in graph(g)? {
            if !members.contains(&key.0) {
                graphs.entry(key).or_default().insert(i);
            }
        }
    }
    let mut out = Vec::new();
    for ((name, version), in_graphs) in graphs {
        let p = packages
            .get(&(name.clone(), version.clone()))
            .ok_or_else(|| format!("{name} {version}: not in cargo metadata"))?;
        if !p
            .source
            .as_deref()
            .is_some_and(|s| s.starts_with("registry+"))
        {
            return Err(format!("{name} {version}: not from a registry"));
        }
        let mut inline_asm = Vec::new();
        let mut native_files = Vec::new();
        let mut unsafe_tokens = 0;
        for file in files(&p.dir)? {
            if is_native(&file) {
                native_files.push(relative(&file, &p.dir));
            }
            if file.extension().and_then(|e| e.to_str()) == Some("rs") {
                let bytes = std::fs::read(&file).map_err(|e| format!("{}: {e}", file.display()))?;
                let text = String::from_utf8_lossy(&bytes);
                let n = inline_asm_count(&text);
                if n > 0 {
                    inline_asm.push((relative(&file, &p.dir), n));
                }
                unsafe_tokens += unsafe_count(&text);
            }
        }
        let mut native_build_markers = Vec::new();
        if let Some(script) = &p.build_script {
            // The build script, and the other files of its directory when it has its own.
            let dir = script.parent().unwrap_or(&p.dir);
            let sources = if dir == p.dir {
                vec![script.clone()]
            } else {
                files(dir)?
            };
            for source in sources {
                if source.extension().and_then(|e| e.to_str()) != Some("rs") {
                    continue;
                }
                let text = std::fs::read_to_string(&source)
                    .map_err(|e| format!("{}: {e}", source.display()))?;
                for marker in NATIVE_BUILD_MARKERS {
                    if text.contains(marker) {
                        native_build_markers
                            .push(format!("{marker} in {}", relative(&source, &p.dir)));
                    }
                }
            }
        }
        let native_build_dependencies = p
            .build_dependencies
            .iter()
            .filter(|d| NATIVE_BUILD_CRATES.contains(&d.as_str()))
            .cloned()
            .collect();
        out.push(Audited {
            build_script: p.build_script.as_ref().map(|s| relative(s, &p.dir)),
            graphs: in_graphs,
            inline_asm,
            native_files,
            native_build_dependencies,
            native_build_markers,
            unsafe_tokens,
            name,
            version,
        });
    }
    Ok(out)
}

/// The audit document, relative to the root of the workspace.
pub const DOCUMENT: &str = "docs/dependencies.md";

/// The crate names of the table rows (`` | `name` | ... ``) of each `## ` section of a
/// Markdown document, by section title.
#[must_use]
pub fn listed(markdown: &str) -> BTreeMap<String, BTreeSet<String>> {
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut section = String::new();
    for line in markdown.lines() {
        if let Some(title) = line.strip_prefix("## ") {
            title.trim().clone_into(&mut section);
            continue;
        }
        let Some(cell) = line
            .strip_prefix("| `")
            .and_then(|rest| rest.split_once('`'))
            .map(|(name, _)| name)
        else {
            continue;
        };
        out.entry(section.clone())
            .or_default()
            .insert(cell.to_owned());
    }
    out
}

/// The sections of [`DOCUMENT`] whose tables [`check`] compares with the graphs.
pub const CRATES_SECTION: &str = "Third-party crates";
pub const BUILD_SCRIPTS_SECTION: &str = "Build scripts";
pub const INLINE_ASM_SECTION: &str = "Inline assembly";

/// What is wrong with the graphs: native files, native builds, and differences with the
/// tables of [`DOCUMENT`] (`markdown`). Empty when all is well.
#[must_use]
pub fn problems(crates: &[Audited], markdown: &str) -> Vec<String> {
    let mut out = Vec::new();
    for c in crates {
        let id = format!("{} {}", c.name, c.version);
        for file in &c.native_files {
            out.push(format!("{id}: ships the native file {file}"));
        }
        for d in &c.native_build_dependencies {
            out.push(format!("{id}: build dependency {d} builds native code"));
        }
        for m in &c.native_build_markers {
            out.push(format!("{id}: its build script builds native code ({m})"));
        }
    }
    let tables = listed(markdown);
    let compare = |section: &str, actual: BTreeSet<String>, what: &str, out: &mut Vec<String>| {
        let documented = tables.get(section).cloned().unwrap_or_default();
        for name in actual.difference(&documented) {
            out.push(format!(
                "{name}: {what}, not in the table \"{section}\" of {DOCUMENT}: audit it there"
            ));
        }
        for name in documented.difference(&actual) {
            out.push(format!(
                "{name}: in the table \"{section}\" of {DOCUMENT}, but not {what}: remove it"
            ));
        }
    };
    compare(
        CRATES_SECTION,
        crates.iter().map(|c| c.name.clone()).collect(),
        "a crate of the shipped graphs",
        &mut out,
    );
    compare(
        BUILD_SCRIPTS_SECTION,
        crates
            .iter()
            .filter(|c| c.build_script.is_some())
            .map(|c| c.name.clone())
            .collect(),
        "a crate of the shipped graphs with a build script",
        &mut out,
    );
    compare(
        INLINE_ASM_SECTION,
        crates
            .iter()
            .filter(|c| !c.inline_asm.is_empty())
            .map(|c| c.name.clone())
            .collect(),
        "a crate of the shipped graphs with inline assembly",
        &mut out,
    );
    out
}

/// [`audit`], then [`problems`] against [`DOCUMENT`].
///
/// # Errors
///
/// When the audit cannot be made or the document cannot be read.
pub fn check() -> Result<(Vec<Audited>, Vec<String>), String> {
    let crates = audit()?;
    let path = workspace::root().join(DOCUMENT);
    let markdown =
        std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let problems = problems(&crates, &markdown);
    Ok((crates, problems))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_inline_assembly_not_its_mentions() {
        assert_eq!(inline_asm_count("core::arch::asm!(\"nop\");"), 1);
        assert_eq!(inline_asm_count("asm! { \"x\" }\nglobal_asm!(\"y\");"), 2);
        assert_eq!(inline_asm_count("naked_asm![\"ret\"]"), 1);
        assert_eq!(inline_asm_count("/// See [`core::arch::asm!`]."), 0);
        assert_eq!(inline_asm_count("my_asm!(x); wasm!(y)"), 0);
        assert_eq!(inline_asm_count("no assembly here"), 0);
    }

    #[test]
    fn counts_unsafe_tokens() {
        assert_eq!(unsafe_count("unsafe { x } unsafe fn y() {}"), 2);
        assert_eq!(unsafe_count("unsafe_code, not_unsafe, unsafely"), 0);
    }

    #[test]
    fn recognises_native_files() {
        for native in ["a.c", "x/y.S", "b.s", "c.H", "lib.a", "z.asm", "w.cpp"] {
            assert!(is_native(Path::new(native)), "{native}");
        }
        for other in ["a.rs", "Cargo.toml", "README.md", "LICENSE", "x.json"] {
            assert!(!is_native(Path::new(other)), "{other}");
        }
    }

    #[test]
    fn reads_the_tables_by_section() {
        let markdown = "# T\n\n## Third-party crates\n\n| Crate | x |\n|---|---|\n\
                        | `memchr` | SIMD |\n| `regex` | - |\n\n## Build scripts\n\n\
                        | `libc` | cfgs |\n";
        let tables = listed(markdown);
        assert_eq!(
            tables.get(CRATES_SECTION).unwrap(),
            &BTreeSet::from(["memchr".to_owned(), "regex".to_owned()])
        );
        assert_eq!(
            tables.get(BUILD_SCRIPTS_SECTION).unwrap(),
            &BTreeSet::from(["libc".to_owned()])
        );
    }

    #[test]
    fn reports_new_and_stale_entries_and_native_code() {
        let audited = |name: &str, build: bool, asm: bool| Audited {
            name: name.to_owned(),
            version: "1.0.0".to_owned(),
            graphs: BTreeSet::from([0]),
            build_script: build.then(|| "build.rs".to_owned()),
            inline_asm: if asm {
                vec![("src/x.rs".to_owned(), 1)]
            } else {
                Vec::new()
            },
            native_files: Vec::new(),
            native_build_dependencies: Vec::new(),
            native_build_markers: Vec::new(),
            unsafe_tokens: 0,
        };
        let mut crates = vec![
            audited("a", true, false),
            audited("b", false, true),
            audited("c", false, false),
        ];
        let markdown = "## Third-party crates\n| `a` |\n| `b` |\n| `c` |\n\
                        ## Build scripts\n| `a` |\n## Inline assembly\n| `b` |\n";
        assert_eq!(problems(&crates, markdown), Vec::<String>::new());
        crates.push(audited("d", true, true));
        crates[0].native_files.push("src/x.c".to_owned());
        let found = problems(
            &crates,
            &markdown.replace("| `c` |\n", "| `c` |\n| `e` |\n"),
        );
        assert_eq!(found.len(), 5, "{found:#?}");
        assert!(found[0].contains("native file src/x.c"));
        assert!(found.iter().any(|p| p.starts_with("e: in the table")));
        assert_eq!(found.iter().filter(|p| p.starts_with("d: ")).count(), 3);
    }
}
