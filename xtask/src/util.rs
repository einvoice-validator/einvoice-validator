//! Errors, file system helpers and hashing shared by the commands.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// A failure, with enough context to act on it.
#[derive(Debug)]
pub(crate) struct Error(String);

impl Error {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<einvoice_svrl::Error> for Error {
    fn from(e: einvoice_svrl::Error) -> Self {
        Self(e.to_string())
    }
}

pub(crate) type Result<T> = std::result::Result<T, Error>;

/// The repository root: the parent of this crate's directory.
pub(crate) fn repo_root() -> PathBuf {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .map_or_else(|| manifest_dir.to_path_buf(), Path::to_path_buf)
}

/// Joins a `/`-separated relative path onto `base`, component by component, so
/// that the same relative paths work on every platform.
pub(crate) fn join(base: &Path, relative: &str) -> PathBuf {
    let mut path = base.to_path_buf();
    for part in relative.split('/').filter(|p| !p.is_empty()) {
        path.push(part);
    }
    path
}

pub(crate) fn read(path: &Path) -> Result<Vec<u8>> {
    fs::read(path).map_err(|e| Error::new(format!("cannot read {}: {e}", path.display())))
}

pub(crate) fn read_string(path: &Path) -> Result<String> {
    let bytes = read(path)?;
    String::from_utf8(bytes)
        .map_err(|e| Error::new(format!("{} is not UTF-8: {e}", path.display())))
}

/// Writes `contents`, creating the parent directories.
pub(crate) fn write(path: &Path, contents: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| Error::new(format!("cannot create {}: {e}", parent.display())))?;
    }
    fs::write(path, contents)
        .map_err(|e| Error::new(format!("cannot write {}: {e}", path.display())))
}

/// Removes a directory tree if it exists.
pub(crate) fn remove_dir(path: &Path) -> Result<()> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(Error::new(format!("cannot remove {}: {e}", path.display()))),
    }
}

/// Every regular file under `dir`, as sorted `/`-separated paths relative to
/// `dir`. A missing directory is an empty list. Symbolic links and non-UTF-8
/// names are refused: the corpus must be plain files with portable names.
pub(crate) fn list_files(dir: &Path) -> Result<Vec<String>> {
    let mut files = Vec::new();
    if dir.exists() {
        walk(dir, "", &mut files)?;
    }
    files.sort();
    Ok(files)
}

fn walk(dir: &Path, prefix: &str, files: &mut Vec<String>) -> Result<()> {
    let entries =
        fs::read_dir(dir).map_err(|e| Error::new(format!("cannot list {}: {e}", dir.display())))?;
    for entry in entries {
        let entry = entry.map_err(|e| Error::new(format!("cannot list {}: {e}", dir.display())))?;
        let name = entry.file_name();
        let name = name
            .to_str()
            .ok_or_else(|| Error::new(format!("non-UTF-8 file name in {}", dir.display())))?;
        let relative = if prefix.is_empty() {
            name.to_owned()
        } else {
            format!("{prefix}/{name}")
        };
        let kind = entry
            .file_type()
            .map_err(|e| Error::new(format!("cannot stat {}: {e}", entry.path().display())))?;
        if kind.is_dir() {
            walk(&entry.path(), &relative, files)?;
        } else if kind.is_file() {
            files.push(relative);
        } else {
            return Err(Error::new(format!(
                "{} is neither a regular file nor a directory",
                entry.path().display()
            )));
        }
    }
    Ok(())
}

/// Lower-case hexadecimal SHA-256 of `bytes`.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(64);
    for byte in digest {
        hex.push(char::from(HEX[usize::from(byte >> 4)]));
        hex.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    hex
}

const HEX: &[u8; 16] = b"0123456789abcdef";

/// XPath `normalize-space`: strips leading and trailing XML whitespace
/// (space, tab, CR, LF) and collapses inner runs to one space. Other Unicode
/// spaces (no-break space, ...) are content and are kept.
pub(crate) fn normalize_space(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for word in text
        .split([' ', '\t', '\r', '\n'])
        .filter(|w| !w.is_empty())
    {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{normalize_space, sha256_hex};

    #[test]
    fn sha256_of_known_input() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn normalize_space_follows_xpath() {
        assert_eq!(normalize_space("  a \t\r\n b  "), "a b");
        assert_eq!(normalize_space("\u{a0}a\u{a0}"), "\u{a0}a\u{a0}");
        assert_eq!(normalize_space(" \n "), "");
    }
}
