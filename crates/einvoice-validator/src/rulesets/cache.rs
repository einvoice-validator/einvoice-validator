//! The cache of fetched rule sets: where it is, what it holds, and loading a rule set
//! from it, every file checked.

use std::fs::File;
use std::path::{Path, PathBuf};

use super::catalogue::{self, FetchedRuleSet};
use crate::{Error, MANIFEST_FILE, RuleSet, Validator};

/// The environment variable that sets the cache directory, overriding the platform's.
pub const CACHE_ENV: &str = "EINVOICE_VALIDATOR_CACHE";

/// The directory of the assembled bundles, under the cache root.
pub(crate) const RULES_DIR: &str = "rules";
/// The directory of the downloads in progress or not yet assembled.
pub(crate) const DOWNLOADS_DIR: &str = "downloads";
/// The directory of the tools a fetch runs (Apache Maven and its local repository).
pub(crate) const TOOLS_DIR: &str = "tools";
/// The directory of temporary work.
pub(crate) const WORK_DIR: &str = "work";
/// The lock file of the cache.
const LOCK_FILE: &str = ".lock";
/// The cache directory tag (<https://bford.info/cachedir/>).
const TAG_FILE: &str = "CACHEDIR.TAG";
const TAG: &str = "Signature: 8a477f597d28d172789f06886806bc55
# This file is a cache directory tag created by einvoice-validator.
# For information about cache directory tags, see https://bford.info/cachedir/
";

/// What the cache holds of a fetched rule set.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CacheStatus {
    /// Its bundle is in the cache, its manifest the catalogue's and every file with the
    /// SHA-256 the manifest records.
    Cached,
    /// It has not been fetched.
    Missing,
    /// Something is in its place, but not its bundle as fetched: why.
    Invalid(String),
}

/// The cache of the rule sets fetched from their publishers ([`fetched_rule_sets`]):
/// a directory holding, under `rules/`, the bundle of each fetched rule set, laid out
/// as the fetch scripts of the repository lay them out
/// ([`FetchedRuleSet::bundle_path`]).
///
/// The default directory is the platform's cache directory: `$XDG_CACHE_HOME`, else
/// `~/.cache`, on Linux and the BSDs; `~/Library/Caches` on macOS; `%LOCALAPPDATA%` on
/// Windows (with `einvoice-validator/` under it, and `cache/` under that on Windows);
/// the environment variable [`CACHE_ENV`] (`EINVOICE_VALIDATOR_CACHE`) overrides it.
///
/// Nothing is ever downloaded except by `RuleSetCache::fetch` (feature `fetch`):
/// loading a rule set that is not in the cache is [`Error::NotCached`]. A loaded rule
/// set is checked in full on every load: its manifest must be, byte for byte, the one
/// the catalogue of this version gives, and every file must have the SHA-256 that
/// manifest records, so a cache can be used offline and is never trusted.
///
/// [`fetched_rule_sets`]: crate::fetched_rule_sets
#[derive(Debug, Clone)]
pub struct RuleSetCache {
    root: PathBuf,
}

impl RuleSetCache {
    /// The cache in the default directory ([`RuleSetCache::default_dir`]).
    ///
    /// # Errors
    ///
    /// As [`RuleSetCache::default_dir`].
    pub fn new() -> Result<Self, Error> {
        Ok(Self::at(Self::default_dir()?))
    }

    /// The cache in the directory `root` (created when something is fetched).
    #[must_use]
    pub fn at(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let root = std::path::absolute(&root).unwrap_or(root);
        Self { root }
    }

    /// The default cache directory: [`CACHE_ENV`] when it is set and not empty, else
    /// the platform's cache directory (see [`RuleSetCache`]).
    ///
    /// # Errors
    ///
    /// [`Error::Fetch`] when there is no home or cache directory to put it in.
    pub fn default_dir() -> Result<PathBuf, Error> {
        if let Some(dir) = std::env::var_os(CACHE_ENV).filter(|v| !v.is_empty()) {
            return Ok(PathBuf::from(dir));
        }
        platform_dir().ok_or_else(|| Error::Fetch {
            message: format!(
                "no cache directory: neither the platform's cache directory nor {CACHE_ENV} is set"
            ),
        })
    }

    /// The cache directory.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The directory of the bundle of `rule_set` in the cache, whether it is there or
    /// not: what `einvoice-validate --bundle` takes.
    #[must_use]
    pub fn bundle_dir(&self, rule_set: &FetchedRuleSet) -> PathBuf {
        let mut dir = self.root.join(RULES_DIR);
        dir.extend(rule_set.bundle_path().split('/'));
        dir
    }

    /// The release directory of the family of `rule_set` in the cache.
    #[cfg(feature = "fetch")]
    pub(crate) fn release_dir(&self, rule_set: &FetchedRuleSet) -> PathBuf {
        self.root.join(RULES_DIR).join(rule_set.release())
    }

    /// What the cache holds of `rule_set`: its manifest and every file it names are
    /// checked (read and hashed), the stylesheets are not prepared.
    #[must_use]
    pub fn status(&self, rule_set: &FetchedRuleSet) -> CacheStatus {
        let dir = self.bundle_dir(rule_set);
        let manifest = match std::fs::read(dir.join(MANIFEST_FILE)) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return CacheStatus::Missing,
            Err(e) => return CacheStatus::Invalid(format!("{MANIFEST_FILE}: {e}")),
        };
        if manifest != rule_set.manifest().as_bytes() {
            return CacheStatus::Invalid(format!(
                "{MANIFEST_FILE} is not the manifest of {} {}",
                rule_set.name(),
                rule_set.version()
            ));
        }
        let parsed = match RuleSet::parse(rule_set.manifest()) {
            Ok(parsed) => parsed,
            Err(e) => return CacheStatus::Invalid(e.to_string()),
        };
        let files = parsed
            .stylesheets()
            .iter()
            .map(|s| (s.path(), s.sha256()))
            .chain(parsed.resources().iter().map(|r| (r.path(), r.sha256())))
            .chain(
                parsed
                    .schema_documents()
                    .iter()
                    .map(|d| (d.path(), d.sha256())),
            );
        for (path, sha256) in files {
            let mut file = dir.clone();
            file.extend(path.split('/'));
            match std::fs::read(&file) {
                Ok(bytes) if crate::validator::sha256_hex(&bytes) == sha256 => {}
                Ok(_) => return CacheStatus::Invalid(format!("{path}: not the pinned SHA-256")),
                Err(e) => return CacheStatus::Invalid(format!("{path}: {e}")),
            }
        }
        CacheStatus::Cached
    }

    /// Loads the fetched rule set `name` (or the alias `peppol`) from the cache.
    ///
    /// # Errors
    ///
    /// [`Error::Bundle`] for a name that is no fetched rule set; [`Error::NotCached`]
    /// when it has not been fetched; [`Error::Bundle`] when the cache holds something
    /// else in its place (a manifest that is not the catalogue's), and otherwise as
    /// [`Validator::from_bundle_dir`], which checks the SHA-256 of every file.
    pub fn load(&self, name: &str) -> Result<Validator, Error> {
        let rule_set = catalogue::fetched_rule_set(name, None)?.ok_or_else(|| unknown(name))?;
        self.load_rule_set(rule_set)
    }

    /// Loads `rule_set` from the cache, as [`RuleSetCache::load`].
    ///
    /// # Errors
    ///
    /// As [`RuleSetCache::load`].
    pub fn load_rule_set(&self, rule_set: &FetchedRuleSet) -> Result<Validator, Error> {
        let dir = self.bundle_dir(rule_set);
        let manifest = match std::fs::read(dir.join(MANIFEST_FILE)) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(Error::NotCached {
                    name: rule_set.name().to_owned(),
                    dir: self.root.clone(),
                });
            }
            Err(source) => {
                return Err(Error::Io {
                    path: dir.join(MANIFEST_FILE),
                    source,
                });
            }
        };
        if manifest != rule_set.manifest().as_bytes() {
            return Err(Error::bundle(format!(
                "{}: not the manifest of {} {} that this version fetches; fetch it again",
                dir.join(MANIFEST_FILE).display(),
                rule_set.name(),
                rule_set.version()
            )));
        }
        Validator::from_bundle_dir(&dir)
    }

    /// Removes everything this crate put in the cache: the bundles, the downloads, the
    /// tools and the temporary files (and the directory itself, when nothing else is in
    /// it). Other files in the directory are left alone.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] when something cannot be removed.
    pub fn clear(&self) -> Result<(), Error> {
        if !self.root.exists() {
            return Ok(());
        }
        let lock = self.lock()?;
        for dir in [RULES_DIR, DOWNLOADS_DIR, TOOLS_DIR, WORK_DIR] {
            remove_dir(&self.root.join(dir))?;
        }
        remove_file(&self.root.join(TAG_FILE))?;
        drop(lock);
        remove_file(&self.root.join(LOCK_FILE))?;
        // Only when empty: the directory may be one the user keeps other files in.
        let _ = std::fs::remove_dir(&self.root);
        Ok(())
    }

    /// Creates the cache directory, with its tag, and takes its lock: one fetch or
    /// clear at a time (readers take no lock: what they read is checked, and a bundle
    /// appears by a rename).
    pub(crate) fn lock(&self) -> Result<File, Error> {
        let io = |path: &Path| {
            let path = path.to_path_buf();
            move |source| Error::Io { path, source }
        };
        std::fs::create_dir_all(&self.root).map_err(io(&self.root))?;
        let tag = self.root.join(TAG_FILE);
        if !tag.exists() {
            std::fs::write(&tag, TAG).map_err(io(&tag))?;
        }
        let path = self.root.join(LOCK_FILE);
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(io(&path))?;
        file.lock().map_err(io(&path))?;
        Ok(file)
    }
}

fn remove_dir(dir: &Path) -> Result<(), Error> {
    match std::fs::remove_dir_all(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(Error::Io {
            path: dir.to_path_buf(),
            source,
        }),
    }
}

fn remove_file(file: &Path) -> Result<(), Error> {
    match std::fs::remove_file(file) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(Error::Io {
            path: file.to_path_buf(),
            source,
        }),
    }
}

/// The error for a name that is no fetched rule set.
pub(crate) fn unknown(name: &str) -> Error {
    let names = catalogue::fetched_rule_sets()
        .map(|all| {
            all.iter()
                .map(FetchedRuleSet::name)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    Error::bundle(format!(
        "no fetched rule set named {name:?} (fetched rule sets: {names}; aliases: {})",
        catalogue::ALIASES
            .iter()
            .map(|(a, _)| *a)
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

/// The platform's cache directory for this crate.
fn platform_dir() -> Option<PathBuf> {
    let var = |name: &str| {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
    };
    if cfg!(windows) {
        return var("LOCALAPPDATA").map(|d| d.join("einvoice-validator").join("cache"));
    }
    if cfg!(target_os = "macos") {
        return var("HOME").map(|h| h.join("Library").join("Caches").join("einvoice-validator"));
    }
    var("XDG_CACHE_HOME")
        .or_else(|| var("HOME").map(|h| h.join(".cache")))
        .map(|d| d.join("einvoice-validator"))
}

impl Validator {
    /// The fetched rule set `name` (see [`fetched_rule_sets`](crate::fetched_rule_sets)),
    /// from the cache in its default directory ([`RuleSetCache::new`]), checked in full.
    /// Never downloads anything: fetch it first with `RuleSetCache::fetch` (feature
    /// `fetch`), or `einvoice-validate rules fetch NAME`.
    ///
    /// # Errors
    ///
    /// As [`RuleSetCache::new`] and [`RuleSetCache::load`]; [`Error::NotCached`] when
    /// it has not been fetched.
    pub fn from_cache(name: &str) -> Result<Self, Error> {
        RuleSetCache::new()?.load(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("einvoice-cache-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn an_empty_cache_has_nothing() {
        let cache = RuleSetCache::at(temp("empty"));
        for r in catalogue::fetched_rule_sets().unwrap() {
            assert_eq!(cache.status(r), CacheStatus::Missing, "{}", r.name());
            match cache.load_rule_set(r) {
                Err(Error::NotCached { name, dir }) => {
                    assert_eq!(name, r.name());
                    assert_eq!(dir, cache.root());
                }
                other => panic!("{}: {other:?}", r.name()),
            }
        }
        assert!(matches!(cache.load("nope"), Err(Error::Bundle { .. })));
        cache.clear().unwrap();
        assert!(!cache.root().exists());
    }

    #[test]
    fn bundle_dirs_follow_the_fetch_scripts() {
        let cache = RuleSetCache::at("/c");
        let dir = |n: &str| {
            let r = catalogue::fetched_rule_set(n, None).unwrap().unwrap();
            let release = r.release().to_owned();
            (cache.bundle_dir(r), release)
        };
        // Made absolute: on Windows, with the current drive.
        let rules = std::path::absolute("/c").unwrap().join("rules");
        let (en16931, release) = dir("factur-x-en16931");
        assert_eq!(en16931, rules.join(&release).join("en16931"));
        let (peppol, release) = dir("peppol");
        assert_eq!(peppol, rules.join(release));
    }

    #[test]
    fn a_tampered_bundle_is_not_loaded() {
        let cache = RuleSetCache::at(temp("tampered"));
        let r = catalogue::fetched_rule_set("factur-x-minimum", None)
            .unwrap()
            .unwrap();
        let dir = cache.bundle_dir(r);
        std::fs::create_dir_all(&dir).unwrap();
        // Another manifest.
        std::fs::write(dir.join(MANIFEST_FILE), "format = 1\n").unwrap();
        assert!(matches!(cache.status(r), CacheStatus::Invalid(_)));
        let error = cache.load_rule_set(r).unwrap_err().to_string();
        assert!(error.contains("fetch it again"), "{error}");
        // The catalogue's manifest, but the files missing or altered.
        std::fs::write(dir.join(MANIFEST_FILE), r.manifest()).unwrap();
        let CacheStatus::Invalid(why) = cache.status(r) else {
            panic!("{:?}", cache.status(r))
        };
        assert!(why.contains("FACTUR-X_MINIMUM.xslt"), "{why}");
        std::fs::write(dir.join("FACTUR-X_MINIMUM.xslt"), "<x/>").unwrap();
        let CacheStatus::Invalid(why) = cache.status(r) else {
            panic!()
        };
        assert!(why.contains("SHA-256"), "{why}");
        assert!(matches!(
            cache.load_rule_set(r),
            Err(Error::Checksum { .. })
        ));
        // clear removes it all, and leaves other files alone.
        std::fs::write(cache.root().join("mine.txt"), "keep").unwrap();
        cache.clear().unwrap();
        assert!(!cache.root().join(RULES_DIR).exists());
        assert!(cache.root().join("mine.txt").exists());
        std::fs::remove_dir_all(cache.root()).unwrap();
    }
}
