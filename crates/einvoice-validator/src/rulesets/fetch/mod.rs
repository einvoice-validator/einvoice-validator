//! Fetching a family of rule sets: its pinned files downloaded (or taken from a local
//! directory), each checked against its pinned digest, the bundles assembled from them
//! as the fetch script of the family assembles them, then installed in the cache in one
//! rename.

mod archive;
mod assemble;
mod http;
mod peppol;
mod tls;

use std::fmt;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use sha2::{Digest as _, Sha256};

use super::cache::{CacheStatus, DOWNLOADS_DIR, RULES_DIR, RuleSetCache, WORK_DIR, unknown};
use super::catalogue::{self, Algorithm, FamilySpec, FetchedRuleSet, Kind, Source};
use crate::Error;

/// The default largest download, and largest file read from an archive: 64 MiB (the
/// largest pinned download is under 10 MiB).
pub const DEFAULT_MAX_SIZE: u64 = 64 << 20;
/// The largest tar archive once decompressed.
const MAX_TAR: usize = 256 << 20;

/// A callback that receives the progress messages of a fetch, one line each.
pub type Progress = Arc<dyn Fn(&str) + Send + Sync>;

/// How [`RuleSetCache::fetch`] gets the files.
#[derive(Clone)]
pub struct FetchOptions {
    mirror: Option<String>,
    from: Vec<PathBuf>,
    attempts: u32,
    retry_delay: Duration,
    timeout: Duration,
    max_size: u64,
    progress: Option<Progress>,
}

impl fmt::Debug for FetchOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FetchOptions")
            .field("mirror", &self.mirror)
            .field("from", &self.from)
            .field("attempts", &self.attempts)
            .field("retry_delay", &self.retry_delay)
            .field("timeout", &self.timeout)
            .field("max_size", &self.max_size)
            .field("progress", &self.progress.is_some())
            .finish()
    }
}

impl Default for FetchOptions {
    fn default() -> Self {
        Self {
            mirror: None,
            from: Vec::new(),
            attempts: 5,
            retry_delay: Duration::from_secs(2),
            timeout: Duration::from_secs(60),
            max_size: DEFAULT_MAX_SIZE,
            progress: None,
        }
    }
}

impl FetchOptions {
    /// The defaults: from the publishers, 5 attempts per file two seconds apart
    /// doubling, a 60-second network timeout, files of at most [`DEFAULT_MAX_SIZE`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Downloads from a mirror: `https://host/path` is fetched as `<base>/host/path`.
    /// The digests are checked all the same. `base` is an `https://` URL, or an
    /// `http://` one to a loopback address.
    #[must_use]
    pub fn mirror(mut self, base: impl Into<String>) -> Self {
        self.mirror = Some(base.into());
        self
    }

    /// Takes a pinned file from the directory `dir` when it holds it under its name
    /// ([`Source::file`], the name the repository's fetch scripts give their downloads),
    /// instead of downloading it; its digest is checked all the same. Directories are
    /// tried in the order given.
    #[must_use]
    pub fn from_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.from.push(dir.into());
        self
    }

    /// The attempts per download (at least 1): an interrupted or failed download (an
    /// I/O error, HTTP 408, 429 or 5xx) is tried again, resuming where it stopped when
    /// the server allows it.
    #[must_use]
    pub fn attempts(mut self, attempts: u32) -> Self {
        self.attempts = attempts.max(1);
        self
    }

    /// The delay before the second attempt, doubled before each further one (at most
    /// a minute), unless the server asks for another (`Retry-After`, at most two
    /// minutes).
    #[must_use]
    pub const fn retry_delay(mut self, delay: Duration) -> Self {
        self.retry_delay = delay;
        self
    }

    /// The network timeout: to connect, and between two reads or writes.
    #[must_use]
    pub const fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// The largest download, and largest file read from an archive, in bytes.
    #[must_use]
    pub const fn max_size(mut self, bytes: u64) -> Self {
        self.max_size = bytes;
        self
    }

    /// Receives a line for each step: what is downloaded or taken from where, what is
    /// checked, built and installed.
    #[must_use]
    pub fn progress(mut self, progress: impl Fn(&str) + Send + Sync + 'static) -> Self {
        self.progress = Some(Arc::new(progress));
        self
    }

    fn say(&self, message: &str) {
        if let Some(progress) = &self.progress {
            progress(message);
        }
    }

    fn max_size_usize(&self) -> usize {
        usize::try_from(self.max_size).unwrap_or(usize::MAX)
    }
}

/// What [`RuleSetCache::fetch`] did for one family.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Fetched {
    /// The family (`factur-x`).
    pub family: &'static str,
    /// Its rule sets, now in the cache.
    pub rule_sets: Vec<&'static str>,
    /// Its release directory in the cache.
    pub dir: PathBuf,
    /// Whether they were all in the cache already (nothing was fetched).
    pub already_cached: bool,
}

impl RuleSetCache {
    /// Fetches the rule sets `name` names: a rule set, an alias (`peppol`, `xrechnung`)
    /// or a family (`factur-x`, `xrechnung`, `fr-ctc`, `peppol`). A rule set is fetched
    /// with the other rule sets of its family, which come from the same files. A family
    /// already in the cache, every file checked, is left as it is. Otherwise its pinned
    /// files are taken from the directories of [`FetchOptions::from_dir`] or downloaded
    /// from their publishers (or the mirror), each checked against its pinned digest;
    /// its bundles are assembled exactly as the fetch scripts of the repository assemble
    /// them and installed in the cache in one rename; the downloads are then removed.
    ///
    /// Peppol BIS Billing publishes its rules as Schematron only: its bundle is built by
    /// compiling them as ph-schematron compiles them, as the reference validators do,
    /// with the ISO Schematron skeleton of ph-schematron (the pinned jar of
    /// ph-schematron-isosch, from Maven Central) run in Rust by einvoice-schematron: no
    /// Java. The compiled stylesheets must have their pinned SHA-256.
    ///
    /// # Errors
    ///
    /// [`Error::Bundle`] for a name that names no fetched rule set; [`Error::Fetch`]
    /// when a file cannot be obtained, does not have its pinned digest, or the bundles
    /// cannot be built from it; [`Error::Io`] when the cache cannot be written.
    pub fn fetch(&self, name: &str, options: &FetchOptions) -> Result<Vec<Fetched>, Error> {
        let rule_sets = catalogue::named(name)?;
        if rule_sets.is_empty() {
            return Err(unknown(name));
        }
        let mut families: Vec<&'static FamilySpec> = Vec::new();
        for r in &rule_sets {
            if !families.iter().any(|f| std::ptr::eq(*f, r.spec.family)) {
                families.push(r.spec.family);
            }
        }
        let _lock = self.lock()?;
        families
            .into_iter()
            .map(|family| self.fetch_family(family, options))
            .collect()
    }

    fn fetch_family(
        &self,
        family: &'static FamilySpec,
        options: &FetchOptions,
    ) -> Result<Fetched, Error> {
        let members: Vec<&'static FetchedRuleSet> = catalogue::fetched_rule_sets()?
            .iter()
            .filter(|r| std::ptr::eq(r.spec.family, family))
            .collect();
        let first = members
            .first()
            .ok_or_else(|| Error::internal(format!("family {} has no rule set", family.name)))?;
        let dir = self.release_dir(first);
        let names = members.iter().map(|r| r.name()).collect();
        if members
            .iter()
            .all(|r| self.status(r) == CacheStatus::Cached)
        {
            options.say(&format!(
                "{} {}: already in the cache ({})",
                family.title,
                first.version(),
                dir.display()
            ));
            return Ok(Fetched {
                family: family.name,
                rule_sets: names,
                dir,
                already_cached: true,
            });
        }
        let fetcher = Fetcher::new(self, options)?;
        let sources = first.sources();
        let files = match family.kind {
            Kind::FacturX | Kind::XRechnung | Kind::FrCtc => {
                let source = sources
                    .first()
                    .ok_or_else(|| Error::internal(format!("{}: no source", family.name)))?;
                let bytes = fetcher.obtain(source)?;
                assemble::assemble(family, &members, &bytes, options.max_size_usize())?
            }
            Kind::Peppol => {
                let [archive, isosch] = sources else {
                    return Err(Error::internal("peppol: two sources expected"));
                };
                let bytes = fetcher.obtain(archive)?;
                let schematron = peppol::schematron(family, &bytes)?;
                let jar = fetcher.obtain(isosch)?;
                let skeleton = peppol::skeleton(family, &jar)?;
                peppol::build(family, &members, &schematron, &skeleton, options)?
            }
        };
        self.install(first.release(), &files)?;
        for r in &members {
            let status = self.status(r);
            if status != CacheStatus::Cached {
                return Err(Error::internal(format!(
                    "{}: installed, but {status:?}",
                    r.name()
                )));
            }
        }
        options.say(&format!(
            "{} {}: {} rule set{} installed in {}",
            family.title,
            first.version(),
            members.len(),
            if members.len() == 1 { "" } else { "s" },
            dir.display()
        ));
        fetcher.remove_downloads(sources);
        // Only when empty: another fetch may have its files there.
        for dir in [DOWNLOADS_DIR, WORK_DIR] {
            let _ = std::fs::remove_dir(self.root().join(dir));
        }
        Ok(Fetched {
            family: family.name,
            rule_sets: names,
            dir,
            already_cached: false,
        })
    }

    /// Writes `files` (path relative to the release directory, content) into a new
    /// directory next to the release directory, then puts it in place by renames.
    fn install(&self, release: &str, files: &[(String, Vec<u8>)]) -> Result<(), Error> {
        let rules = self.root().join(RULES_DIR);
        let io = |path: &Path| {
            let path = path.to_path_buf();
            move |source| Error::Io { path, source }
        };
        std::fs::create_dir_all(&rules).map_err(io(&rules))?;
        let stamp = unique();
        let staging = rules.join(format!(".new-{release}-{stamp}"));
        for (path, bytes) in files {
            let mut file = staging.clone();
            file.extend(path.split('/'));
            if let Some(parent) = file.parent() {
                std::fs::create_dir_all(parent).map_err(io(parent))?;
            }
            let mut out = std::fs::File::create(&file).map_err(io(&file))?;
            out.write_all(bytes)
                .and_then(|()| out.sync_all())
                .map_err(io(&file))?;
        }
        let target = rules.join(release);
        if target.exists() {
            let old = rules.join(format!(".old-{release}-{stamp}"));
            std::fs::rename(&target, &old).map_err(io(&target))?;
            std::fs::rename(&staging, &target).map_err(io(&staging))?;
            std::fs::remove_dir_all(&old).map_err(io(&old))?;
        } else {
            std::fs::rename(&staging, &target).map_err(io(&staging))?;
        }
        Ok(())
    }
}

/// A name no other process uses at the same time: the process id and the time.
pub(crate) fn unique() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("{}-{nanos}", std::process::id())
}

/// The lower-case hexadecimal digest of `bytes`.
pub(crate) fn digest(algorithm: Algorithm, bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let raw: Vec<u8> = match algorithm {
        Algorithm::Sha256 => Sha256::digest(bytes).to_vec(),
    };
    let mut hex = String::with_capacity(raw.len() * 2);
    for byte in raw {
        hex.push(char::from(HEX[usize::from(byte >> 4)]));
        hex.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    hex
}

/// Checks that `bytes` (`what`) has the pinned SHA-256 `expected`.
pub(crate) fn check_sha256(what: &str, bytes: &[u8], expected: &str) -> Result<(), Error> {
    let actual = digest(Algorithm::Sha256, bytes);
    if actual == expected {
        Ok(())
    } else {
        Err(Error::Fetch {
            message: format!("{what} has SHA-256 {actual}, expected {expected} (pinned): refused"),
        })
    }
}

/// Gets the pinned files of a fetch.
struct Fetcher<'a> {
    downloads: PathBuf,
    options: &'a FetchOptions,
    client: http::Client,
}

impl<'a> Fetcher<'a> {
    fn new(cache: &RuleSetCache, options: &'a FetchOptions) -> Result<Self, Error> {
        let downloads = cache.root().join(DOWNLOADS_DIR);
        std::fs::create_dir_all(&downloads).map_err(|source| Error::Io {
            path: downloads.clone(),
            source,
        })?;
        Ok(Self {
            downloads,
            options,
            client: http::Client {
                timeout: options.timeout,
            },
        })
    }

    /// The URL `source` is downloaded from: its own, or the mirror's.
    fn url(&self, source: &Source) -> String {
        match &self.options.mirror {
            None => source.url().to_owned(),
            Some(base) => {
                let rest = source
                    .url()
                    .split_once("://")
                    .map_or(source.url(), |(_, rest)| rest);
                format!("{}/{rest}", base.trim_end_matches('/'))
            }
        }
    }

    fn check(source: &Source, what: &str, bytes: &[u8]) -> Result<(), Error> {
        let actual = digest(source.algorithm_kind(), bytes);
        if actual == source.digest() {
            Ok(())
        } else {
            Err(Error::Fetch {
                message: format!(
                    "{what} has {} {actual}, expected {} (pinned): refused",
                    source.algorithm(),
                    source.digest()
                ),
            })
        }
    }

    /// The content of `source`, its digest checked: from a directory of
    /// [`FetchOptions::from_dir`], from a complete download of an earlier fetch, or
    /// downloaded.
    fn obtain(&self, source: &Source) -> Result<Vec<u8>, Error> {
        let read = |path: &Path| {
            std::fs::read(path).map_err(|e| Error::Io {
                path: path.to_path_buf(),
                source: e,
            })
        };
        for dir in &self.options.from {
            let path = dir.join(source.file());
            if path.is_file() {
                let bytes = read(&path)?;
                Self::check(source, &path.display().to_string(), &bytes)?;
                self.options.say(&format!(
                    "{}: {} taken from {}, {} verified",
                    source.what(),
                    source.file(),
                    dir.display(),
                    source.algorithm()
                ));
                return Ok(bytes);
            }
        }
        let done = self.downloads.join(source.file());
        if done.is_file() {
            let bytes = read(&done)?;
            if Self::check(source, "", &bytes).is_ok() {
                self.options.say(&format!(
                    "{}: downloaded earlier, {} verified",
                    source.what(),
                    source.algorithm()
                ));
                return Ok(bytes);
            }
            let _ = std::fs::remove_file(&done);
        }
        self.download(source, &done)
    }

    /// Downloads `source` into `done`, through `<done>.part`, resuming an interrupted
    /// download.
    fn download(&self, source: &Source, done: &Path) -> Result<Vec<u8>, Error> {
        let url = self.url(source);
        let part = done.with_file_name(format!("{}.part", source.file()));
        let fetch_error = |message: String| Error::Fetch { message };
        // A resumed download that does not have the digest is tried once more afresh:
        // the part may be of another file.
        for fresh in [false, true] {
            if fresh {
                let _ = std::fs::remove_file(&part);
            }
            let resumed = self.transfer(&url, &part, source)?;
            let bytes = std::fs::read(&part).map_err(|e| Error::Io {
                path: part.clone(),
                source: e,
            })?;
            match Self::check(source, &format!("the download from {url}"), &bytes) {
                Ok(()) => {
                    std::fs::rename(&part, done).map_err(|e| Error::Io {
                        path: part.clone(),
                        source: e,
                    })?;
                    self.options.say(&format!(
                        "{}: downloaded from {url} ({} bytes), {} verified",
                        source.what(),
                        bytes.len(),
                        source.algorithm()
                    ));
                    return Ok(bytes);
                }
                Err(e) if resumed && !fresh => {
                    self.options
                        .say(&format!("{e}; downloading it again from the start"));
                }
                Err(e) => {
                    let _ = std::fs::remove_file(&part);
                    return Err(e);
                }
            }
        }
        Err(fetch_error(format!("{url}: not downloaded")))
    }

    /// Transfers `url` into `part`, appending to what it holds when the server can
    /// resume, with the retries of the options. Returns whether the content was resumed
    /// from an earlier part.
    fn transfer(&self, url: &str, part: &Path, source: &Source) -> Result<bool, Error> {
        let options = self.options;
        let mut resumed = false;
        let mut last = String::new();
        for attempt in 1..=options.attempts {
            if attempt > 1 {
                let delay = options
                    .retry_delay
                    .saturating_mul(1 << (attempt - 2).min(16))
                    .min(Duration::from_secs(60));
                options.say(&format!(
                    "{last}; trying again in {} s (attempt {attempt} of {})",
                    delay.as_secs_f32(),
                    options.attempts
                ));
                std::thread::sleep(delay);
            }
            let offset = std::fs::metadata(part).map_or(0, |m| m.len());
            if offset == 0 {
                options.say(&format!("{}: downloading {url}", source.what()));
            } else {
                options.say(&format!(
                    "{}: resuming the download of {url} at byte {offset}",
                    source.what()
                ));
            }
            match self.attempt(url, part, offset) {
                Ok(appended) => return Ok(resumed || appended),
                Err(e) if e.retryable => {
                    resumed |= offset > 0;
                    last = e.message;
                    if let Some(seconds) = e.retry_after {
                        let delay = Duration::from_secs(seconds.min(120));
                        options.say(&format!(
                            "{last}; the server asks to wait {} s",
                            delay.as_secs()
                        ));
                        std::thread::sleep(delay);
                    }
                }
                Err(e) => {
                    return Err(Error::Fetch { message: e.message });
                }
            }
        }
        Err(Error::Fetch {
            message: format!("{last} ({} attempts)", options.attempts),
        })
    }

    /// One request. Returns whether the body was appended to an earlier part.
    fn attempt(&self, url: &str, part: &Path, offset: u64) -> Result<bool, http::HttpError> {
        let fatal = |message: String| http::HttpError {
            message,
            retryable: false,
            retry_after: None,
        };
        let mut response = self.client.get(url, offset)?;
        let append = match response.status {
            200 => false,
            206 if offset > 0 => {
                // `bytes <first>-<last>/<length>`: it must start where the part stops.
                let range = response.header("content-range").unwrap_or("");
                let first = range
                    .strip_prefix("bytes ")
                    .and_then(|r| r.split('-').next())
                    .and_then(|n| n.trim().parse::<u64>().ok());
                if first != Some(offset) {
                    return Err(fatal(format!(
                        "{url}: resumed at {range:?}, not at byte {offset}"
                    )));
                }
                true
            }
            // The part is already complete: the digest decides.
            416 if offset > 0 => return Ok(true),
            status if http::retryable_status(status) => {
                return Err(http::HttpError {
                    message: format!("{url}: HTTP {status}"),
                    retryable: true,
                    retry_after: http::retry_after(&response),
                });
            }
            status => {
                return Err(fatal(format!(
                    "{} (from {url}): HTTP {status}",
                    response.url
                )));
            }
        };
        let limit = self.options.max_size;
        let room = if append {
            limit.saturating_sub(offset)
        } else {
            limit
        };
        if response.content_length().is_some_and(|n| n > room) {
            return Err(fatal(format!(
                "{url}: {} bytes announced, more than the limit of {limit} bytes",
                response.content_length().unwrap_or(0) + if append { offset } else { 0 }
            )));
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .append(append)
            .truncate(!append)
            .open(part)
            .map_err(|e| fatal(format!("{}: {e}", part.display())))?;
        let copied = response.copy_to(&mut file, room);
        let synced = file.sync_all();
        copied?;
        synced.map_err(|e| fatal(format!("{}: {e}", part.display())))?;
        Ok(append)
    }

    /// Removes the downloads of `sources` once their bundles are installed.
    fn remove_downloads(&self, sources: &[Source]) {
        for source in sources {
            let _ = std::fs::remove_file(self.downloads.join(source.file()));
        }
    }
}

#[cfg(test)]
mod tests;
