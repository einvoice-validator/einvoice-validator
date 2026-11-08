//! Local-only check on published sample files, which are not redistributable and so are
//! kept outside the repository.
//!
//! `EINVOICE_PDF_SAMPLES` lists one or more manifest files (separated like `PATH`). Each
//! line of a manifest is `PDF<TAB>EXPECTED`, paths absolute or relative to the manifest:
//! `EXPECTED` is the published XML file, which the extracted invoice must equal byte for
//! byte, or `error:<Kind>` (such as `error:Encrypted` or `error:Unsupported`), the
//! [`Error`] variant [`extract`] must return. Without the variable the test prints
//! SKIPPED and passes.

#![cfg(test)]

use std::path::{Path, PathBuf};

use einvoice_pdf::{Error, INVOICE_FILE_NAMES, attachments, extract};

const VARIABLE: &str = "EINVOICE_PDF_SAMPLES";

fn kind(error: &Error) -> &'static str {
    match error {
        Error::NotPdf => "NotPdf",
        Error::Encrypted => "Encrypted",
        Error::Malformed { .. } => "Malformed",
        Error::Unsupported { .. } => "Unsupported",
        Error::LimitExceeded { .. } => "LimitExceeded",
        Error::NoInvoice { .. } => "NoInvoice",
        Error::Ambiguous { .. } => "Ambiguous",
        Error::Metadata(_) => "Metadata",
        _ => "other",
    }
}

fn resolve(manifest: &Path, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        manifest.parent().unwrap().join(path)
    }
}

#[test]
fn published_samples() {
    let Some(manifests) = std::env::var_os(VARIABLE) else {
        eprintln!("SKIPPED: {VARIABLE} is not set");
        return;
    };
    let mut identical = 0;
    let mut refused = 0;
    let mut failures = Vec::new();
    for manifest in std::env::split_paths(&manifests) {
        let text = std::fs::read_to_string(&manifest)
            .unwrap_or_else(|e| panic!("{}: {e}", manifest.display()));
        for line in text.lines().filter(|l| !l.is_empty()) {
            let (pdf, expected) = line
                .split_once('\t')
                .unwrap_or_else(|| panic!("malformed manifest line: {line}"));
            let pdf_path = resolve(&manifest, pdf);
            let bytes =
                std::fs::read(&pdf_path).unwrap_or_else(|e| panic!("{}: {e}", pdf_path.display()));
            let result = extract(&bytes);
            if let Some(expected_kind) = expected.strip_prefix("error:") {
                match result {
                    Err(error) if kind(&error) == expected_kind => {
                        refused += 1;
                        eprintln!("refused as expected: {}: {error}", pdf_path.display());
                    }
                    other => failures.push(format!(
                        "{}: expected error:{expected_kind}, got {other:?}",
                        pdf_path.display()
                    )),
                }
                continue;
            }
            let published = std::fs::read(resolve(&manifest, expected)).unwrap();
            let embedded = match result {
                Ok(embedded) => embedded,
                Err(error) => {
                    failures.push(format!("{}: {error}", pdf_path.display()));
                    continue;
                }
            };
            if embedded.xml.data != published {
                failures.push(format!(
                    "{}: the extracted XML ({} bytes) differs from {expected} ({} bytes)",
                    pdf_path.display(),
                    embedded.xml.data.len(),
                    published.len()
                ));
                continue;
            }
            assert!(INVOICE_FILE_NAMES.contains(&embedded.xml.file_name.as_str()));
            let metadata = embedded.metadata.as_ref().expect("XMP metadata");
            assert_eq!(
                metadata.pdfa_part.as_deref(),
                Some("3"),
                "{}",
                pdf_path.display()
            );
            let invoice = metadata.invoice.as_ref().expect("Factur-X XMP properties");
            assert_eq!(
                invoice.document_file_name.as_deref(),
                Some(embedded.xml.file_name.as_str())
            );
            // attachments() finds the same file, with the same content.
            let all = attachments(&bytes).unwrap();
            assert!(
                all.iter().any(|a| a == &embedded.xml),
                "{}: attachments() misses the invoice",
                pdf_path.display()
            );
            identical += 1;
        }
    }
    eprintln!(
        "{identical} extracted invoices byte-identical to the published XML, {refused} refused as expected, {} failures",
        failures.len()
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert!(identical + refused > 0, "{VARIABLE}: no sample listed");
}
