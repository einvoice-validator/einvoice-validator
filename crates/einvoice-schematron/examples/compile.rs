//! Compiles a Schematron file to XSLT as ph-schematron 10.3.1 does, in Rust, and writes
//! the stylesheet; the messages of the ISO Schematron skeleton go to the standard error.
//! The files the schema includes are read from the directory of the file and below it;
//! the skeleton from `$EINVOICE_SCHEMATRON_SKELETON`, else from the test data of this
//! crate (`tests/data/skeleton/`).
//!
//! ```text
//! cargo run -p einvoice-schematron --example compile -- FILE.sch [OUT.xslt]
//! ```

use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Every file under `dir`, by `file:` URI.
fn sources(dir: &Path) -> Result<einvoice_schematron::Sources, String> {
    let mut sources = einvoice_schematron::Sources::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let entries = std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for entry in entries {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.is_dir() {
                pending.push(path);
            } else if let (Some(uri), Ok(text)) = (
                einvoice_xpath_eval::file_uri(&path),
                std::fs::read_to_string(&path),
            ) {
                sources = sources.with(uri, text);
            }
        }
    }
    Ok(sources)
}

fn run() -> Result<(), String> {
    let mut args = std::env::args_os().skip(1);
    let input = args
        .next()
        .map(PathBuf::from)
        .ok_or("usage: compile FILE.sch [OUT.xslt]")?;
    let output = args.next().map(PathBuf::from);
    let input = std::path::absolute(&input).map_err(|e| e.to_string())?;
    let text = std::fs::read_to_string(&input).map_err(|e| format!("{}: {e}", input.display()))?;
    let uri = einvoice_xpath_eval::file_uri(&input).ok_or("not a valid path")?;
    let dir = input.parent().ok_or("no directory")?;
    let skeleton_dir = std::env::var_os("EINVOICE_SCHEMATRON_SKELETON").map_or_else(
        || {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests")
                .join("data")
                .join("skeleton")
        },
        PathBuf::from,
    );
    let mut files = Vec::new();
    for (name, _) in einvoice_schematron::SKELETON_FILES {
        let path = skeleton_dir.join(name);
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        files.push((name.to_owned(), text));
    }
    let skeleton = einvoice_schematron::Skeleton::new(&files).map_err(|e| e.to_string())?;
    let compilation = skeleton
        .compile(&text, &uri, &sources(dir)?)
        .map_err(|e| e.to_string())?;
    for message in &compilation.messages {
        eprintln!("message: {message}");
    }
    if let Some(path) = output {
        std::fs::write(&path, &compilation.stylesheet)
            .map_err(|e| format!("{}: {e}", path.display()))
    } else {
        print!("{}", compilation.stylesheet);
        Ok(())
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("compile: {e}");
            ExitCode::FAILURE
        }
    }
}
