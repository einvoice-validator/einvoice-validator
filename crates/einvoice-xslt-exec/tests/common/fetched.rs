//! The corpora of the fetched rule sets other than Peppol (`corpus/README.md`): Factur-X,
//! XRechnung and the French rules. None of their files is committed: the fetch script of
//! each family downloads the pinned inputs into `target/<family>/`, then
//! `cargo xtask <family>` extracts the cases and writes the Saxon-HE goldens of every
//! stylesheet there, and a stamp, `<corpus>/complete.txt`, last. A corpus is used only when
//! its stamp is there and agrees: the numbers of cases and mutation cases, and each
//! stylesheet with the SHA-256 pinned in the fetch script. The files a stylesheet reads with
//! `document()` (a code list next to it) have their SHA-256 pinned there too, and are all a
//! transformation may read.

#![allow(
    dead_code,
    reason = "shared by the equivalence tests of the families, each using a part"
)]

use std::path::{Path, PathBuf};

use einvoice_xdm::{AtomicType, AtomicValue};
use einvoice_xslt_exec::{DocumentResolver, Executable, TransformOptions};
use sha2::{Digest, Sha256};

use super::{Expected, MutationCase, depth, first_difference, join, read};

/// A stylesheet of a rule set: name, compiled XSLT relative to the base directory, golden
/// directory relative to the corpus directory, pin of its SHA-256, and the file it reads
/// with `document()`, next to it, with the pin of its SHA-256.
pub(crate) struct StylesheetSpec {
    pub(crate) name: &'static str,
    pub(crate) xslt: &'static str,
    pub(crate) golden_dir: &'static str,
    pub(crate) pin: &'static str,
    pub(crate) side_file: Option<(&'static str, &'static str)>,
}

/// A rule set: its name, its corpus directory relative to the base directory of its
/// family, and its stylesheets.
pub(crate) struct RuleSetSpec {
    pub(crate) name: &'static str,
    pub(crate) corpus: &'static str,
    pub(crate) stylesheets: &'static [StylesheetSpec],
}

/// A family: its name (`cargo xtask <name>`), its cache directory relative to the
/// workspace root, its fetch script (which pins every input), the variable that makes a
/// missing cache an error, and its rule sets.
pub(crate) struct FamilySpec {
    pub(crate) name: &'static str,
    pub(crate) base: &'static str,
    pub(crate) script: &'static str,
    pub(crate) required_var: &'static str,
    pub(crate) rule_sets: &'static [RuleSetSpec],
}

const fn stylesheet(
    name: &'static str,
    xslt: &'static str,
    golden_dir: &'static str,
    pin: &'static str,
) -> StylesheetSpec {
    StylesheetSpec {
        name,
        xslt,
        golden_dir,
        pin,
        side_file: None,
    }
}

const fn with_codes(
    name: &'static str,
    xslt: &'static str,
    golden_dir: &'static str,
    pin: &'static str,
    codes: (&'static str, &'static str),
) -> StylesheetSpec {
    StylesheetSpec {
        name,
        xslt,
        golden_dir,
        pin,
        side_file: Some(codes),
    }
}

/// Factur-X 1.09.2: the five profiles of the Mustang validator.
pub(crate) const FACTURX: FamilySpec = FamilySpec {
    name: "facturx",
    base: "target/facturx/factur-x-1.09.2",
    script: "scripts/fetch-facturx.sh",
    required_var: "EINVOICE_FACTURX_REQUIRED",
    rule_sets: &[RuleSetSpec {
        name: "factur-x-1.09.2",
        corpus: "corpus",
        stylesheets: &[
            with_codes(
                "FACTUR-X_MINIMUM",
                "minimum/FACTUR-X_MINIMUM.xslt",
                "golden-minimum",
                "minimum_xslt_sha256",
                ("FACTUR-X_MINIMUM_codedb.xml", "minimum_codedb_sha256"),
            ),
            with_codes(
                "FACTUR-X_BASIC-WL",
                "basic-wl/FACTUR-X_BASIC-WL.xslt",
                "golden-basic-wl",
                "basic_wl_xslt_sha256",
                ("FACTUR-X_BASIC-WL_codedb.xml", "basic_wl_codedb_sha256"),
            ),
            with_codes(
                "FACTUR-X_BASIC",
                "basic/FACTUR-X_BASIC.xslt",
                "golden-basic",
                "basic_xslt_sha256",
                ("FACTUR-X_BASIC_codedb.xml", "basic_codedb_sha256"),
            ),
            with_codes(
                "FACTUR-X_EN16931",
                "en16931/FACTUR-X_EN16931.xslt",
                "golden-en16931",
                "en16931_xslt_sha256",
                ("FACTUR-X_EN16931_codedb.xml", "en16931_codedb_sha256"),
            ),
            with_codes(
                "FACTUR-X_EXTENDED",
                "extended/FACTUR-X_EXTENDED.xslt",
                "golden-extended",
                "extended_xslt_sha256",
                ("FACTUR-X_EXTENDED_codedb.xml", "extended_codedb_sha256"),
            ),
        ],
    }],
};

/// XRechnung 3.0.2: the EN 16931 and XRechnung stylesheets of the KoSIT configuration.
pub(crate) const XRECHNUNG: FamilySpec = FamilySpec {
    name: "xrechnung",
    base: "target/xrechnung/xrechnung-3.0.2",
    script: "scripts/fetch-xrechnung.sh",
    required_var: "EINVOICE_XRECHNUNG_REQUIRED",
    rule_sets: &[
        RuleSetSpec {
            name: "xrechnung-ubl-3.0.2",
            corpus: "corpus-ubl",
            stylesheets: &[
                stylesheet(
                    "EN16931-UBL",
                    "source/configuration/resources/ubl/2.1/xsl/EN16931-UBL-validation.xsl",
                    "golden-cen",
                    "cen_ubl_sha256",
                ),
                stylesheet(
                    "XRechnung-UBL",
                    "source/configuration/resources/xrechnung/3.0.2/xsl/XRechnung-UBL-validation.xsl",
                    "golden-xrechnung",
                    "xrechnung_ubl_sha256",
                ),
            ],
        },
        RuleSetSpec {
            name: "xrechnung-cii-3.0.2",
            corpus: "corpus-cii",
            stylesheets: &[
                stylesheet(
                    "EN16931-CII",
                    "source/configuration/resources/cii/16b/xsl/EN16931-CII-validation.xsl",
                    "golden-cen",
                    "cen_cii_sha256",
                ),
                stylesheet(
                    "XRechnung-CII",
                    "source/configuration/resources/xrechnung/3.0.2/xsl/XRechnung-CII-validation.xsl",
                    "golden-xrechnung",
                    "xrechnung_cii_sha256",
                ),
            ],
        },
    ],
};

/// The French rules, FNFE-MPE package 1.4.0.04: CII, UBL and CDAR.
pub(crate) const FR_CTC: FamilySpec = FamilySpec {
    name: "fr-ctc",
    base: "target/fr-ctc/fr-ctc-1.4.0.04",
    script: "scripts/fetch-fr-ctc.sh",
    required_var: "EINVOICE_FR_CTC_REQUIRED",
    rule_sets: &[
        RuleSetSpec {
            name: "fr-ctc-cii-1.4.0.04",
            corpus: "corpus-cii",
            stylesheets: &[
                stylesheet(
                    "EN16931-CII",
                    "cii-en16931/EN16931-CII.xslt",
                    "golden-en16931",
                    "cii_en16931_sha256",
                ),
                stylesheet(
                    "EXTENDED-CTC-FR-CII",
                    "cii-extended-ctc-fr/EXTENDED-CTC-FR-CII.xslt",
                    "golden-extended-ctc-fr",
                    "cii_extended_ctc_fr_sha256",
                ),
                stylesheet(
                    "BR-FR-Flux2-CII",
                    "cii-en16931/BR-FR-Flux2-CII.xslt",
                    "golden-br-fr",
                    "cii_br_fr_sha256",
                ),
                stylesheet(
                    "BR-FR-Flux2-CII_WARNING",
                    "cii-en16931-warning/BR-FR-Flux2-CII_WARNING.xslt",
                    "golden-br-fr-warning",
                    "cii_br_fr_warning_sha256",
                ),
                with_codes(
                    "FACTUR-X_BASIC-WL",
                    "factur-x-basic-wl/FACTUR-X_BASIC-WL.xslt",
                    "golden-factur-x-basic-wl",
                    "fx_basic_wl_sha256",
                    ("FACTUR-X_BASIC-WL_codedb.xml", "fx_basic_wl_codedb_sha256"),
                ),
                with_codes(
                    "FACTUR-X_EN16931",
                    "factur-x-en16931/FACTUR-X_EN16931.xslt",
                    "golden-factur-x-en16931",
                    "fx_en16931_sha256",
                    ("FACTUR-X_EN16931_codedb.xml", "fx_en16931_codedb_sha256"),
                ),
                with_codes(
                    "FACTUR-X_EXTENDED",
                    "factur-x-extended/FACTUR-X_EXTENDED.xslt",
                    "golden-factur-x-extended",
                    "fx_extended_sha256",
                    ("FACTUR-X_EXTENDED_codedb.xml", "fx_extended_codedb_sha256"),
                ),
            ],
        },
        RuleSetSpec {
            name: "fr-ctc-ubl-1.4.0.04",
            corpus: "corpus-ubl",
            stylesheets: &[
                stylesheet(
                    "EN16931-UBL",
                    "ubl-en16931/EN16931-UBL.xslt",
                    "golden-en16931",
                    "ubl_en16931_sha256",
                ),
                stylesheet(
                    "EXTENDED-CTC-FR-UBL",
                    "ubl-extended-ctc-fr/EXTENDED-CTC-FR-UBL.xslt",
                    "golden-extended-ctc-fr",
                    "ubl_extended_ctc_fr_sha256",
                ),
                stylesheet(
                    "BR-FR-Flux2-UBL",
                    "ubl-en16931/BR-FR-Flux2-UBL.xslt",
                    "golden-br-fr",
                    "ubl_br_fr_sha256",
                ),
                stylesheet(
                    "BR-FR-Flux2-UBL_WARNING",
                    "ubl-en16931-warning/BR-FR-Flux2-UBL_WARNING.xslt",
                    "golden-br-fr-warning",
                    "ubl_br_fr_warning_sha256",
                ),
            ],
        },
        RuleSetSpec {
            name: "fr-ctc-cdar-1.4.0.04",
            corpus: "corpus-cdar",
            stylesheets: &[
                stylesheet(
                    "BR-FR-CDV-CDAR",
                    "cdar/BR-FR-CDV-CDAR.xslt",
                    "golden",
                    "cdar_sha256",
                ),
                stylesheet(
                    "BR-FR-CDV-CDAR_WARNING",
                    "cdar-warning/BR-FR-CDV-CDAR_WARNING.xslt",
                    "golden-warning",
                    "cdar_warning_sha256",
                ),
            ],
        },
    ],
};

/// The stack a case deeper than the default maximum depth may use, run with the maximum
/// raised.
const RAISED_MAX_STACK: u32 = 32 << 20;

/// The clock of every oracle run (Saxon-HE `-now:`), given to the engine too.
pub(crate) const FIXED_NOW: &str = "2000-01-01T00:00:00Z";

/// Whether a missing cache must fail.
pub(crate) fn required(family: &FamilySpec) -> bool {
    std::env::var_os(family.required_var).is_some_and(|v| !v.is_empty())
}

/// One case of a corpus: its id and its input document (absolute path).
#[derive(Debug, Clone)]
pub(crate) struct Case {
    pub(crate) id: String,
    pub(crate) input: String,
}

/// A stylesheet of a cached corpus.
pub(crate) struct Stylesheet {
    pub(crate) name: &'static str,
    pub(crate) xslt: PathBuf,
    pub(crate) golden_dir: PathBuf,
    /// The file it reads with `document()`: its `file:` URI and its text.
    pub(crate) side_file: Option<(String, String)>,
}

/// A cached corpus: its cases (`cases.tsv`), its mutation cases (`mutations.tsv`) and
/// its stylesheets.
pub(crate) struct Corpus {
    pub(crate) name: &'static str,
    pub(crate) cases: Vec<Case>,
    pub(crate) mutations: Vec<MutationCase>,
    pub(crate) stylesheets: Vec<Stylesheet>,
}

fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut hex, b| {
            let _ = write!(hex, "{b:02x}");
            hex
        })
}

/// The `name=value` lines between the pin markers of a fetch script.
fn pins(root: &Path, script: &str) -> Result<Vec<(String, String)>, String> {
    let text = read(&join(root, script))?;
    let mut lines = text.lines().skip_while(|l| !l.starts_with("# --- pins "));
    lines.next().ok_or_else(|| format!("{script}: no pins"))?;
    Ok(lines
        .take_while(|l| !l.starts_with("# --- end of pins "))
        .filter_map(|l| l.split_once('='))
        .map(|(n, v)| (n.to_owned(), v.to_owned()))
        .collect())
}

/// The file `file` next to the stylesheet `xslt`, with the SHA-256 `expected`: its
/// `file:` URI and its text.
fn side_file(
    xslt: &Path,
    file: &str,
    expected: &str,
    rebuild: &str,
) -> Result<(String, String), String> {
    let path = xslt
        .parent()
        .map(|d| d.join(file))
        .ok_or_else(|| format!("{}: no directory", xslt.display()))?;
    let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    if sha256_hex(&bytes) != expected {
        return Err(format!(
            "{}: not the pinned SHA-256 {expected}; {rebuild}",
            path.display()
        ));
    }
    let uri =
        einvoice_xpath_eval::file_uri(&std::path::absolute(&path).map_err(|e| e.to_string())?)
            .ok_or_else(|| format!("{}: no file URI", path.display()))?;
    let text = String::from_utf8(bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok((uri, text))
}

/// The cached corpora of a family, `None` when one is absent or incomplete (no stamp).
/// An error when one is there but does not agree with its stamp or the pins.
pub(crate) fn corpora(root: &Path, family: &FamilySpec) -> Result<Option<Vec<Corpus>>, String> {
    let base = join(root, family.base);
    let pins = pins(root, family.script)?;
    let pin = |name: &str| {
        pins.iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.clone())
            .ok_or_else(|| format!("{}: no pin {name}", family.script))
    };
    let rebuild = format!("rebuild the cache with scripts/check.sh {}", family.name);
    let mut out = Vec::new();
    for spec in family.rule_sets {
        let corpus = base.join(spec.corpus);
        let stamp_path = corpus.join("complete.txt");
        if !stamp_path.exists() {
            return Ok(None);
        }
        let stamp = read(&stamp_path)?;
        let mut stylesheets = Vec::new();
        for s in spec.stylesheets {
            let expected = pin(s.pin)?;
            let xslt = join(&base, s.xslt);
            let bytes = std::fs::read(&xslt).map_err(|e| format!("{}: {e}", xslt.display()))?;
            let actual = sha256_hex(&bytes);
            if actual != expected {
                return Err(format!(
                    "{}: SHA-256 {actual}, expected {expected} ({} in {}); {rebuild}",
                    xslt.display(),
                    s.pin,
                    family.script
                ));
            }
            if !stamp
                .lines()
                .any(|l| l == format!("stylesheet {} {expected}", s.name))
            {
                return Err(format!(
                    "{}: the goldens were not made with the pinned {}; {rebuild}",
                    stamp_path.display(),
                    s.name
                ));
            }
            let side_file = match s.side_file {
                None => None,
                Some((file, file_pin)) => Some(side_file(&xslt, file, &pin(file_pin)?, &rebuild)?),
            };
            stylesheets.push(Stylesheet {
                name: s.name,
                xslt,
                golden_dir: corpus.join(s.golden_dir),
                side_file,
            });
        }
        let cases = read(&corpus.join("cases.tsv"))?
            .lines()
            .skip(1)
            .map(|line| {
                let mut columns = line.split('\t');
                let (Some(id), Some(input)) = (columns.next(), columns.next()) else {
                    return Err(format!("cases.tsv: bad line {line:?}"));
                };
                let input = std::path::absolute(join(&base, input)).map_err(|e| e.to_string())?;
                let input = input
                    .to_str()
                    .ok_or_else(|| format!("{}: not UTF-8", input.display()))?
                    .to_owned();
                Ok(Case {
                    id: id.to_owned(),
                    input,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let mutations = if corpus.join("mutations.tsv").exists() {
            super::mutation_cases(&corpus, &base)?
        } else {
            Vec::new()
        };
        for (what, count) in [("cases", cases.len()), ("mutations", mutations.len())] {
            if !stamp.lines().any(|l| l == format!("{what} {count}")) {
                return Err(format!(
                    "{}: does not record {count} {what}; {rebuild}",
                    stamp_path.display()
                ));
            }
        }
        out.push(Corpus {
            name: spec.name,
            cases,
            mutations,
            stylesheets,
        });
    }
    Ok(Some(out))
}

/// What a stylesheet may read with `document()`: its side file, by URI, and nothing else.
struct SideFile<'a>(Option<&'a (String, String)>);

impl DocumentResolver for SideFile<'_> {
    fn resolve(&self, uri: &str) -> Option<String> {
        self.0
            .filter(|(u, _)| u == uri)
            .map(|(_, text)| text.clone())
    }
}

/// The stylesheet, loaded and prepared, with its location as static base URI (against
/// which `document()` resolves the URI of its side file).
pub(crate) fn prepare(stylesheet: &Stylesheet) -> Result<Executable, String> {
    let executable = super::prepare(&stylesheet.xslt)?;
    let uri = einvoice_xpath_eval::file_uri(
        &std::path::absolute(&stylesheet.xslt).map_err(|e| e.to_string())?,
    )
    .ok_or_else(|| format!("{}: no file URI", stylesheet.xslt.display()))?;
    Ok(executable.with_base_uri(uri))
}

/// The golden of case `id` of a stylesheet: its canonical SVRL or the error that stopped
/// Saxon-HE.
pub(crate) fn expected(stylesheet: &Stylesheet, id: &str) -> Result<Expected, String> {
    super::expected(&stylesheet.golden_dir, id)
}

/// What the engine gives on the document at `path`, as Saxon-HE ran it: the clock fixed
/// at [`FIXED_NOW`], only the side file to read, and a document deeper than the default
/// maximum depth run again with the maximum raised (the default must refuse it).
pub(crate) fn run(
    executable: &Executable,
    stylesheet: &Stylesheet,
    path: &str,
) -> Result<Result<String, einvoice_xslt_exec::TransformError>, String> {
    let text = read(Path::new(path))?;
    let uri = einvoice_xpath_eval::file_uri(Path::new(path));
    let AtomicValue::DateTime(now) = AtomicValue::String(FIXED_NOW.to_owned())
        .cast(AtomicType::DateTime)
        .map_err(|e| e.to_string())?
    else {
        return Err("the fixed clock is not a dateTime".to_owned());
    };
    let resolver = SideFile(stylesheet.side_file.as_ref());
    let options = |max_depth: u32| TransformOptions {
        current_date_time: Some(now),
        resolver: Some(&resolver),
        max_depth,
        max_nesting: einvoice_xslt_exec::max_nesting_for_depth(max_depth),
        ..TransformOptions::default()
    };
    let result = executable.transform_with(
        text.as_str(),
        uri.as_deref(),
        &options(einvoice_xpath_eval::DEFAULT_MAX_DEPTH),
    );
    let deeper = depth(&text) > einvoice_xpath_eval::DEFAULT_MAX_DEPTH;
    let result = match result {
        Err(e) if deeper => {
            if e.code() != "LIMIT" {
                return Err(format!("{path}: deeper than the default maximum, but {e}"));
            }
            // On a thread with the stack the raised maximum needs (some stylesheets need
            // more per level than the CEN ones).
            let side_file = stylesheet.side_file.as_ref();
            let (text, uri) = (&text, &uri);
            std::thread::scope(|scope| {
                std::thread::Builder::new()
                    .stack_size(RAISED_MAX_STACK as usize + (1 << 20))
                    .spawn_scoped(scope, move || {
                        let resolver = SideFile(side_file);
                        let raised = TransformOptions {
                            current_date_time: Some(now),
                            resolver: Some(&resolver),
                            max_depth: super::RAISED_MAX_DEPTH,
                            max_nesting: einvoice_xslt_exec::max_nesting_for_depth(
                                super::RAISED_MAX_DEPTH,
                            ),
                            max_stack: RAISED_MAX_STACK,
                            ..TransformOptions::default()
                        };
                        executable
                            .transform_with(text.as_str(), uri.as_deref(), &raised)
                            .map(|t| t.output)
                    })
                    .map_err(|e| e.to_string())?
                    .join()
                    .map_err(|_| "the deep case panicked".to_owned())
            })?
            .map(|output| einvoice_xslt_exec::Transformation {
                output,
                warnings: Vec::new(),
            })
        }
        other => {
            if deeper {
                return Err(format!(
                    "{path}: deeper than the default maximum, and accepted"
                ));
            }
            other
        }
    };
    Ok(result.map(|t| {
        if let Ok(Some(run_time)) = einvoice_svrl::run_time(&t.output) {
            assert_eq!(
                run_time, FIXED_NOW,
                "{path}: the clock is not the fixed one"
            );
        }
        t.output
    }))
}

/// Compares what the engine gives on one document with the golden: `None` when equal
/// (an error with the same code counts as equal), else the difference.
pub(crate) fn compare(
    executable: &Executable,
    stylesheet: &Stylesheet,
    id: &str,
    input: &str,
) -> Result<Option<String>, String> {
    let ours = run(executable, stylesheet, input)?;
    Ok(match (expected(stylesheet, id)?, ours) {
        (Expected::Svrl(golden), Ok(svrl)) => {
            let canonical = super::canonical(&svrl, input)?;
            (canonical != golden).then(|| first_difference(&golden, &canonical))
        }
        (Expected::Svrl(_), Err(e)) => Some(format!("  saxon: a report\n  ours:  error {e}")),
        (Expected::Error { code, .. }, Ok(_)) => {
            Some(format!("  saxon: error {code}\n  ours:  a report"))
        }
        (Expected::Error { code, message }, Err(e)) => (e.code() != code)
            .then(|| format!("  saxon: error {code} {message}\n  ours:  error {e}")),
    })
}

/// Runs every stylesheet of every corpus of the family on every case and mutation case,
/// on all the threads available; prints the counts and the first differences; returns
/// the number of cases that differ or fail.
pub(crate) fn check_family(family: &FamilySpec) -> usize {
    let root = super::workspace_root();
    let corpora = match corpora(&root, family) {
        Ok(Some(corpora)) => corpora,
        Ok(None) => {
            let message = format!(
                "no {} corpus in {}: build it with scripts/check.sh {} (needs Java and network access)",
                family.name, family.base, family.name
            );
            assert!(
                !required(family),
                "{message}; {} is set",
                family.required_var
            );
            eprintln!("SKIPPED: {message}");
            return 0;
        }
        Err(e) => panic!("{e}"),
    };
    let mut problems = 0;
    for corpus in &corpora {
        let all: Vec<(&str, &str)> = corpus
            .cases
            .iter()
            .map(|c| (c.id.as_str(), c.input.as_str()))
            .chain(
                corpus
                    .mutations
                    .iter()
                    .map(|m| (m.id.as_str(), m.input.as_str())),
            )
            .collect();
        for stylesheet in &corpus.stylesheets {
            let executable = prepare(stylesheet).unwrap_or_else(|e| panic!("{e}"));
            let next = std::sync::atomic::AtomicUsize::new(0);
            let found = std::sync::Mutex::new(Vec::new());
            let threads = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
            std::thread::scope(|scope| {
                for _ in 0..threads {
                    // Each worker on a thread with the stack a spawned thread has by
                    // default, as a validation runs.
                    std::thread::Builder::new()
                        .stack_size(2 << 20)
                        .spawn_scoped(scope, || {
                            loop {
                                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                let Some((id, input)) = all.get(i) else {
                                    break;
                                };
                                let outcome = match compare(&executable, stylesheet, id, input) {
                                    Ok(None) => None,
                                    Ok(Some(d)) => Some(format!("DIFFERENT {id}\n{d}")),
                                    Err(e) => Some(format!("FAILED {id}: {e}")),
                                };
                                if let Some(o) = outcome
                                    && let Ok(mut v) = found.lock()
                                {
                                    v.push((i, o));
                                }
                            }
                        })
                        .unwrap_or_else(|e| panic!("{e}"));
                }
            });
            let mut found = found.into_inner().unwrap_or_default();
            found.sort();
            eprintln!(
                "{} {}: {} cases ({} mutation cases): {} equal, {} not",
                corpus.name,
                stylesheet.name,
                all.len(),
                corpus.mutations.len(),
                all.len() - found.len(),
                found.len()
            );
            for (_, f) in found.iter().take(15) {
                eprintln!("{} {f}", stylesheet.name);
            }
            problems += found.len();
        }
    }
    problems
}
