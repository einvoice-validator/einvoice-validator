//! The canonical SVRL form (`corpus/SVRL-CANONICAL.md`) is implemented by the
//! `einvoice-svrl` crate, shared with the engine's equivalence tests so that
//! both sides of a comparison use the same code.

pub(crate) use einvoice_svrl::{canonicalize, is_document_uri, parse};
