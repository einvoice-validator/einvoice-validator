//! Helpers shared by the tests of the workspace (not published):
//!
//! - [`workspace`]: the workspace crates a package is built from, with their licence
//!   expressions, as cargo resolves them for a set of features, for the tests that check
//!   that a crate's licence notices name exactly what it embeds;
//! - [`pdf`]: a writer of synthetic PDF files and Factur-X-like fixtures, for the tests
//!   of einvoice-pdf and of the crates that read PDF files through it.
//!
//! It builds for `wasm32-unknown-unknown` too, where the examples of those crates build
//! with their dev-dependencies.

pub mod pdf;
pub mod workspace;
