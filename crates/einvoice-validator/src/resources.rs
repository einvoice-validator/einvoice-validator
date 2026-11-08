//! The resources of a bundle (`[[resource]]`): the files its stylesheets read with
//! `document()`, checked and decoded when the bundle is loaded, given to the stylesheets
//! by URI and nothing else.

use einvoice_xslt_exec::DocumentResolver;

use crate::Error;
use crate::manifest::ResourceEntry;
use crate::validator::{decode, sha256_hex};

/// The URI scheme of the files of a bundle given in memory
/// ([`Validator::from_bundle`](crate::Validator::from_bundle)): a stylesheet at `a/b.xsl`
/// in the bundle has the base URI `bundle:/a/b.xsl`, and `document('c.xml')` in it asks
/// for `bundle:/a/c.xml`. A bundle read from a directory has `file:` URIs.
pub(crate) const BUNDLE_URI: &str = "bundle:/";

/// The resources of a bundle, by URI: what `document()` can load. A URI that names none
/// of them cannot be retrieved (`FODC0002`), as in the reference engine for a missing
/// file: the stylesheets read no other file and nothing from the network.
#[derive(Debug, Default)]
pub(crate) struct Resources {
    /// The absolute URI of each resource, and its text.
    by_uri: Vec<(String, String)>,
}

impl Resources {
    /// Adds a resource: its SHA-256 checked against the manifest's, its text decoded as
    /// a document is (UTF-8). `uri` is the absolute URI stylesheets ask for it by.
    pub(crate) fn add(
        &mut self,
        entry: &ResourceEntry,
        bytes: &[u8],
        uri: String,
    ) -> Result<(), Error> {
        let actual = sha256_hex(bytes);
        if actual != entry.sha256() {
            return Err(Error::Checksum {
                path: entry.path().to_owned(),
                expected: entry.sha256().to_owned(),
                actual,
            });
        }
        let text =
            decode(bytes).map_err(|e| Error::bundle(format!("resource {}: {e}", entry.path())))?;
        self.by_uri.push((uri, text.to_owned()));
        Ok(())
    }
}

impl DocumentResolver for Resources {
    fn resolve(&self, uri: &str) -> Option<String> {
        self.by_uri
            .iter()
            .find(|(u, _)| u == uri)
            .map(|(_, text)| text.clone())
    }
}
