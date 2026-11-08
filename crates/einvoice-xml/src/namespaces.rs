//! The namespace declarations in scope.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use crate::Error;

/// No binding.
pub(crate) const NONE: u32 = u32::MAX;

/// Up to this many bindings in scope, a prefix is looked up by a linear scan; beyond, in
/// the index (so that a document with very many declarations stays linear in time).
const SCAN_LIMIT: usize = 16;

struct Binding<'a> {
    /// The prefix declared, `""` for the default namespace.
    prefix: &'a str,
    /// The namespace name, as normalised (empty to undeclare the default namespace).
    uri: Cow<'a, str>,
    /// The binding of the same prefix that this one hides, or [`NONE`].
    hidden: u32,
}

/// The bindings in scope, innermost last, with an index from each prefix to its
/// innermost binding, and the set of distinct declarations seen in the document (for
/// [`crate::Limits::namespaces`]).
pub(crate) struct Scope<'a> {
    bindings: Vec<Binding<'a>>,
    innermost: HashMap<&'a str, u32>,
    distinct: HashSet<(&'a str, Cow<'a, str>)>,
}

impl<'a> Scope<'a> {
    pub(crate) fn new() -> Self {
        Self {
            bindings: Vec::new(),
            innermost: HashMap::new(),
            distinct: HashSet::new(),
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.bindings.len()
    }

    /// Declares `prefix` (`""` for the default namespace). `limit` bounds the number of
    /// distinct (prefix, namespace) pairs declared in the document, the `xml` prefix
    /// counted.
    pub(crate) fn push(
        &mut self,
        prefix: &'a str,
        uri: Cow<'a, str>,
        limit: u32,
    ) -> Result<(), Error> {
        let key = (prefix, uri);
        if !self.distinct.contains(&key) {
            if self.distinct.len().saturating_add(1) >= limit as usize {
                return Err(Error::NamespacesLimitReached);
            }
            self.distinct.insert(key.clone());
        }
        let (prefix, uri) = key;
        let index =
            u32::try_from(self.bindings.len()).map_err(|_| Error::NamespacesLimitReached)?;
        let hidden = self.innermost.insert(prefix, index).unwrap_or(NONE);
        self.bindings.push(Binding {
            prefix,
            uri,
            hidden,
        });
        Ok(())
    }

    /// Removes the bindings after the first `len`.
    pub(crate) fn truncate(&mut self, len: usize) {
        while self.bindings.len() > len {
            let Some(binding) = self.bindings.pop() else {
                break;
            };
            if binding.hidden == NONE {
                self.innermost.remove(binding.prefix);
            } else {
                self.innermost.insert(binding.prefix, binding.hidden);
            }
        }
    }

    /// The innermost binding of `prefix`, or [`NONE`].
    pub(crate) fn lookup(&self, prefix: &str) -> u32 {
        if self.bindings.len() <= SCAN_LIMIT {
            for (index, binding) in self.bindings.iter().enumerate().rev() {
                if binding.prefix == prefix {
                    return u32::try_from(index).unwrap_or(NONE);
                }
            }
            NONE
        } else {
            self.innermost.get(prefix).copied().unwrap_or(NONE)
        }
    }

    /// Whether `prefix` has a binding at or after `start`.
    pub(crate) fn declared_since(&self, start: usize, prefix: &str) -> bool {
        let index = self.innermost.get(prefix).copied().unwrap_or(NONE);
        index != NONE && index as usize >= start
    }

    /// The namespace name of a binding (empty for an undeclared default namespace).
    pub(crate) fn uri(&self, index: u32) -> &str {
        self.bindings
            .get(index as usize)
            .map_or("", |binding| &binding.uri)
    }

    /// The prefix and namespace name of each binding from `start` on.
    pub(crate) fn since(&self, start: usize) -> impl ExactSizeIterator<Item = (&'a str, &str)> {
        self.bindings
            .get(start..)
            .unwrap_or_default()
            .iter()
            .map(|binding| (binding.prefix, binding.uri.as_ref()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bindings_hide_and_are_restored() {
        let mut scope = Scope::new();
        for n in 0..40 {
            let prefixes = ["", "a", "b"];
            scope
                .push(prefixes[n % 3], Cow::Owned(format!("u{n}")), 65536)
                .unwrap();
        }
        // Linear scan and index agree.
        for len in (0..=40).rev() {
            scope.truncate(len);
            for prefix in ["", "a", "b", "c"] {
                let expected = (0..len).rev().find(|n| ["", "a", "b"][n % 3] == prefix);
                let found = scope.lookup(prefix);
                assert_eq!(
                    expected.map(|n| format!("u{n}")),
                    (found != NONE).then(|| scope.uri(found).to_owned()),
                    "{len} {prefix}"
                );
                let indexed = scope.innermost.get(prefix).copied().unwrap_or(NONE);
                assert_eq!(found, indexed);
            }
        }
    }

    #[test]
    fn distinct_declarations_are_limited() {
        let mut scope = Scope::new();
        scope.push("a", Cow::Borrowed("u"), 3).unwrap();
        scope.push("a", Cow::Borrowed("u"), 3).unwrap();
        scope.push("b", Cow::Borrowed("u"), 3).unwrap();
        assert_eq!(
            scope.push("c", Cow::Borrowed("u"), 3),
            Err(Error::NamespacesLimitReached)
        );
        scope.push("b", Cow::Borrowed("u"), 3).unwrap();
    }
}
