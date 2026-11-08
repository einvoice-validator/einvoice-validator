//! The rule sets whose files the project does not redistribute, fetched from their
//! publishers on demand: Factur-X (the five profiles, from the Mustang validator on
//! Maven Central), XRechnung (UBL and CII, from the KoSIT validator configuration), the
//! French rules (the fourteen bundles of the FNFE-MPE package) and Peppol BIS Billing
//! 3.0 (its Schematron, compiled with ph-schematron as the reference validators do).
//!
//! The catalogue ([`fetched_rule_sets`]) gives, for each, the pinned URL and digest of
//! every file, taken from the fetch scripts of the repository, and the manifest of its
//! bundle. [`RuleSetCache`] keeps the fetched bundles; with the feature `fetch`,
//! [`RuleSetCache::fetch`] downloads and assembles them. Nothing is downloaded
//! otherwise, and nothing on WebAssembly.

mod cache;
mod catalogue;
#[cfg(feature = "fetch")]
mod fetch;
mod pins;

pub use cache::{CACHE_ENV, CacheStatus, RuleSetCache};
pub use catalogue::{
    FetchedRuleSet, Source, fetched_rule_set, fetched_rule_set_aliases, fetched_rule_sets,
};
#[cfg(feature = "fetch")]
pub use fetch::{DEFAULT_MAX_SIZE, FetchOptions, Fetched, Progress};
