//! Helpers shared by the tests of the workspace (not published):
//!
//! - [`workspace`]: the workspace crates a package is built from, with their licence
//!   expressions, as cargo resolves them for a set of features, for the tests that check
//!   that a crate's licence notices name exactly what it embeds.

pub mod workspace;
