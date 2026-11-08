//! The official rule sets the corpus covers, and where their files live.
//!
//! A rule set is either vendored (its upstream files are committed under
//! `artefacts/<name>/`, its corpus and goldens under `corpus/<name>/`) or
//! fetched (its upstream files may not be redistributed: a script of
//! `scripts/` downloads, checks and, if need be, compiles them into a cache
//! under `target/`, where the corpus and the goldens are generated too; only
//! the record of disagreements and a README are committed, under
//! `corpus/<name>/`). The fetched rule sets come in families, one per fetch
//! script ([`Family`]).
//!
//! Every path of a rule set is relative to its base directory: the repository
//! root for a vendored rule set, the cache directory for a fetched one. The
//! `input` column of `cases.tsv` is relative to the base too.

use std::path::{Path, PathBuf};

use crate::util;

/// Where the files of a rule set come from.
#[derive(Debug)]
pub(crate) enum Origin {
    /// Committed under `artefacts/<name>/`, with a `SHA256SUMS` manifest; the
    /// derived corpus and the goldens are committed under `corpus/<name>/`
    /// with a copy of the licence (relative to the vendored directory).
    Vendored { licence: &'static str },
    /// Built by the fetch script of its family into `base` (relative to the
    /// repository root); the corpus is generated in `base/corpus`, a
    /// directory of its own per rule set of the family.
    Fetched {
        base: &'static str,
        corpus: &'static str,
    },
}

/// How the reports of a stylesheet are laid out, which depends on the
/// Schematron compiler that wrote it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Generator {
    /// The ISO Schematron skeleton (CEN, Peppol, Factur-X, the French rules):
    /// reports in the canonical form as they are.
    IsoSkeleton,
    /// SchXslt (the XRechnung configuration): each report starts with an
    /// `svrl:metadata` element (the processor, the time of the run, the
    /// compiler and the time of the compilation), which canonical form 2
    /// represents; it is checked (`crate::svrl`).
    SchXslt,
}

/// One compiled stylesheet of a rule set, run on every case.
#[derive(Debug)]
pub(crate) struct Stylesheet {
    /// Short name, in messages, in the records and in the bundles.
    pub(crate) name: &'static str,
    /// The compiled XSLT, relative to the base directory.
    pub(crate) xslt: &'static str,
    /// Directory of its goldens, relative to the corpus directory.
    pub(crate) golden_dir: &'static str,
    /// For a fetched rule set: the name of the pin in the fetch script that
    /// holds the SHA-256 of the compiled XSLT.
    pub(crate) sha256_pin: Option<&'static str>,
    /// The files the stylesheet reads with `document()`, relative to its own
    /// directory, each with the pin of its SHA-256.
    pub(crate) side_files: &'static [(&'static str, &'static str)],
    pub(crate) generator: Generator,
}

/// Documents that the reference pipeline of the rule set accepts: published
/// examples and test-suite instances. For each, the stylesheets of its profile
/// (chosen by the specification identifier it declares, BT-24) must report
/// nothing that rejects it.
#[derive(Debug)]
pub(crate) struct Acceptance {
    /// The case id prefixes (document directories) the expectation covers.
    pub(crate) cases: &'static [&'static str],
    pub(crate) profiles: Profiles,
}

/// How the stylesheets of a document's profile are chosen.
#[derive(Debug)]
pub(crate) enum Profiles {
    /// Specification identifier, the stylesheets that run on it. A finding
    /// of level `error` rejects the document: a failed assert unless its flag
    /// is `warning` or `information`, a successful report with the flag
    /// `fatal` or `error` (`crate::acceptance`). A document whose identifier
    /// is not listed is an error.
    Listed(&'static [(&'static str, &'static [&'static str])]),
    /// The scenarios of a KoSIT validator configuration (`scenarios.xml`,
    /// relative to the base directory; the locations of its resources are
    /// relative to its directory): the one scenario whose `match` the document
    /// satisfies names the stylesheets, and its `customLevel` elements override
    /// the level of rule ids; a finding of level `error` (KoSIT's reading of
    /// the flag and the role when not overridden) rejects the document, and so
    /// does matching no scenario or several (`crate::acceptance`). `bundle` is
    /// the directory of the bundle the fetch script writes for the rule set,
    /// relative to the base directory: its `[[scenario]]` tables must be the
    /// scenarios of the configuration that run its stylesheets.
    KositScenarios {
        file: &'static str,
        bundle: &'static str,
    },
}

/// One rule set and the corpus built from it.
#[derive(Debug)]
pub(crate) struct RuleSet {
    /// Directory name under `corpus/` (and under `artefacts/` when vendored).
    pub(crate) name: &'static str,
    pub(crate) origin: Origin,
    /// The directory the document and VEFA directories are relative to,
    /// relative to the base directory.
    pub(crate) source: &'static str,
    /// The stylesheets run on every case; each has its own goldens.
    pub(crate) stylesheets: &'static [Stylesheet],
    /// Whole documents used as cases: (case id prefix, directory relative to
    /// the source directory). Every file below the directory is a case.
    pub(crate) document_dirs: &'static [(&'static str, &'static str)],
    /// VEFA unit test sets, relative to the source directory. Each `<test>`
    /// of each `<name>.xml` file becomes one case under `vefa/<directory name>/`.
    pub(crate) vefa_dirs: &'static [&'static str],
    /// Documents generated by XML Mutate: (case id prefix, directory relative
    /// to the source directory). Every file is a case, with the expectation of
    /// its active `<?xmute?>` instruction (`crate::xmute`).
    pub(crate) xmute_dirs: &'static [(&'static str, &'static str)],
    /// The symbolic Schematron names of the `<?xmute?>` instructions, with the
    /// stylesheet each stands for.
    pub(crate) xmute_schematrons: &'static [(&'static str, &'static str)],
    /// Files in those directories that are neither documents nor test sets,
    /// relative to the source directory. Each must exist.
    pub(crate) ignored: &'static [&'static str],
    /// The mutation catalogue applied to the rule set, relative to the
    /// repository root: one per syntax, as its paths name the elements of one
    /// syntax (`corpus/mutations.toml` for UBL, `corpus/mutations-cii.toml`
    /// for CII); none for a syntax without one.
    pub(crate) mutation_catalogue: Option<&'static str>,
    /// The base documents of the mutation catalogue: (base name, id of a
    /// whole-document case).
    pub(crate) mutation_bases: &'static [(&'static str, &'static str)],
    /// The documents the reference pipeline accepts, if any are known.
    pub(crate) acceptance: Option<Acceptance>,
    /// The record of the known disagreements between the goldens and the
    /// expectations, in the record directory.
    pub(crate) record: &'static str,
}

impl RuleSet {
    /// The base directory of the rule set's paths.
    pub(crate) fn base(&self, root: &Path) -> PathBuf {
        match self.origin {
            Origin::Vendored { .. } => root.to_path_buf(),
            Origin::Fetched { base, .. } => util::join(root, base),
        }
    }

    /// `artefacts/<name>`, relative to the repository root, when vendored.
    pub(crate) fn vendored_dir(&self) -> Option<String> {
        match self.origin {
            Origin::Vendored { .. } => Some(format!("artefacts/{}", self.name)),
            Origin::Fetched { .. } => None,
        }
    }

    /// The licence file, relative to the source directory, when vendored.
    pub(crate) fn licence(&self) -> Option<&'static str> {
        match self.origin {
            Origin::Vendored { licence } => Some(licence),
            Origin::Fetched { .. } => None,
        }
    }

    /// The corpus directory, relative to the base directory.
    pub(crate) fn corpus_dir(&self) -> String {
        match self.origin {
            Origin::Vendored { .. } => self.record_dir(),
            Origin::Fetched { corpus, .. } => corpus.to_owned(),
        }
    }

    /// `corpus/<name>`, relative to the repository root: the committed files of
    /// the rule set's corpus (all of it when vendored, the record of
    /// disagreements and the README when fetched).
    pub(crate) fn record_dir(&self) -> String {
        format!("corpus/{}", self.name)
    }

    /// The stylesheet named `name`.
    pub(crate) fn stylesheet(&self, name: &str) -> Option<&Stylesheet> {
        self.stylesheets.iter().find(|s| s.name == name)
    }
}

/// The fetched rule sets built by one script.
#[derive(Debug)]
pub(crate) struct Family {
    /// The `cargo xtask` command that builds its corpora.
    pub(crate) name: &'static str,
    /// The script that fills the cache and holds the pins, relative to the
    /// repository root.
    pub(crate) script: &'static str,
    /// The pin whose value ends the name of every rule set of the family.
    pub(crate) version_pin: &'static str,
    pub(crate) rule_sets: &'static [&'static RuleSet],
}

/// The record of disagreements with the VEFA expectations.
pub(crate) const VEFA_RECORD: &str = "vefa-disagreements.tsv";
/// The record of disagreements with the expectations of the fetched rule sets
/// (XML Mutate, accepted documents).
pub(crate) const RECORD: &str = "disagreements.tsv";

/// The mutation catalogue of the UBL rule sets.
pub(crate) const UBL_MUTATIONS: &str = "corpus/mutations.toml";
/// The mutation catalogue of the CII rule sets.
pub(crate) const CII_MUTATIONS: &str = "corpus/mutations-cii.toml";

/// The bases of the UBL catalogue in the CEN UBL examples, relative to the
/// source directory of a fetched rule set that copies them as `cen-examples/`
/// and `cen-testfiles/`.
const CEN_UBL_BASES: &[(&str, &str)] = &[
    ("invoice", "cen-testfiles/Invoice-Max_content.xml"),
    ("credit-note", "cen-testfiles/CreditNote-Max_content.xml"),
    ("allowances", "cen-examples/ubl-tc434-example2.xml"),
];
/// The bases of the CII catalogue in the CEN CII examples, for a fetched rule
/// set that copies them as `cen-examples/`.
const CEN_CII_BASES: &[(&str, &str)] = &[
    ("invoice", "cen-examples/CII_example5.xml"),
    ("single", "cen-examples/CII_example3.xml"),
    ("allowances", "cen-examples/CII_example2.xml"),
    ("zero", "cen-examples/CII_business_example_Z.xml"),
    ("outside", "cen-examples/XRechnung-O.xml"),
];

/// CEN EN 16931 validation artefacts for UBL, release 1.3.17.
pub(crate) const CEN_UBL: RuleSet = RuleSet {
    name: "cen-en16931-ubl-1.3.17",
    origin: Origin::Vendored {
        licence: "LICENSE.txt",
    },
    source: "artefacts/cen-en16931-ubl-1.3.17",
    stylesheets: &[Stylesheet {
        name: "EN16931-UBL-validation",
        xslt: "artefacts/cen-en16931-ubl-1.3.17/ubl/xslt/EN16931-UBL-validation.xslt",
        golden_dir: "golden",
        sha256_pin: None,
        side_files: &[],
        generator: Generator::IsoSkeleton,
    }],
    document_dirs: &[
        ("examples", "ubl/examples"),
        ("testfiles", "test/testfiles"),
    ],
    vefa_dirs: &["test/Invoice-unit-UBL", "test/CreditNote-unit-UBL"],
    xmute_dirs: &[],
    xmute_schematrons: &[],
    ignored: &[],
    mutation_catalogue: Some(UBL_MUTATIONS),
    mutation_bases: &[
        ("invoice", "testfiles/Invoice-Max_content.xml"),
        ("credit-note", "testfiles/CreditNote-Max_content.xml"),
        ("allowances", "examples/ubl-tc434-example2.xml"),
    ],
    acceptance: None,
    record: VEFA_RECORD,
};

/// CEN EN 16931 validation artefacts for UN/CEFACT CII D16B, release 1.3.17.
/// The upstream build runs the examples and the VEFA test sets of `test/cii`
/// (`buildconfig-cii.xml`); there are no other CII test documents.
pub(crate) const CEN_CII: RuleSet = RuleSet {
    name: "cen-en16931-cii-1.3.17",
    origin: Origin::Vendored {
        licence: "LICENSE.txt",
    },
    source: "artefacts/cen-en16931-cii-1.3.17",
    stylesheets: &[Stylesheet {
        name: "EN16931-CII-validation",
        xslt: "artefacts/cen-en16931-cii-1.3.17/cii/xslt/EN16931-CII-validation.xslt",
        golden_dir: "golden",
        sha256_pin: None,
        side_files: &[],
        generator: Generator::IsoSkeleton,
    }],
    document_dirs: &[("examples", "cii/examples")],
    vefa_dirs: &["test/cii"],
    xmute_dirs: &[],
    xmute_schematrons: &[],
    ignored: &[],
    mutation_catalogue: Some(CII_MUTATIONS),
    mutation_bases: &[
        ("invoice", "examples/CII_example5.xml"),
        ("single", "examples/CII_example3.xml"),
        ("allowances", "examples/CII_example2.xml"),
        ("zero", "examples/CII_business_example_Z.xml"),
        ("outside", "examples/XRechnung-O.xml"),
    ],
    acceptance: None,
    record: VEFA_RECORD,
};

/// Peppol BIS Billing 3.0.22, UBL syntax: its own rules and the
/// CEN EN 16931 rules it embeds, both compiled by `scripts/fetch-peppol.sh`.
pub(crate) const PEPPOL_UBL: RuleSet = RuleSet {
    name: "peppol-bis-billing-ubl-3.0.22",
    origin: Origin::Fetched {
        base: "target/peppol/peppol-bis-billing-ubl-3.0.22",
        corpus: "corpus",
    },
    source: "source/rules",
    stylesheets: &[
        Stylesheet {
            name: "PEPPOL-EN16931-UBL",
            xslt: "xslt/PEPPOL-EN16931-UBL.xslt",
            golden_dir: "golden-peppol",
            sha256_pin: Some("peppol_xslt_sha256"),
            side_files: &[],
            generator: Generator::IsoSkeleton,
        },
        Stylesheet {
            name: "CEN-EN16931-UBL",
            xslt: "xslt/CEN-EN16931-UBL.xslt",
            golden_dir: "golden-cen",
            sha256_pin: Some("cen_xslt_sha256"),
            side_files: &[],
            generator: Generator::IsoSkeleton,
        },
    ],
    document_dirs: &[
        ("examples", "examples"),
        ("snippets", "snippets"),
        ("national-examples", "national-examples"),
    ],
    vefa_dirs: &[
        "unit-UBL-PEPPOL",
        "unit-UBL-DE",
        "unit-UBL-DK",
        "unit-UBL-GR",
        "unit-UBL-IT",
        "unit-UBL-NL",
        "unit-UBL-NO",
        "unit-UBL-SE",
    ],
    xmute_dirs: &[],
    xmute_schematrons: &[],
    ignored: &[
        "national-examples/README.adoc",
        // Every test is commented out, and the upstream build, which runs the
        // `*.xml` files of a test folder, does not run it.
        "unit-UBL-PEPPOL/PEPPOL-COMMON-R048.xm",
    ],
    mutation_catalogue: Some(UBL_MUTATIONS),
    mutation_bases: &[
        ("invoice", "examples/base-example.xml"),
        ("credit-note", "examples/base-creditnote-correction.xml"),
        ("allowances", "examples/Allowance-example.xml"),
    ],
    acceptance: None,
    record: VEFA_RECORD,
};

/// The specification identifiers (BT-24) of the Factur-X profiles, as the
/// Mustang validator maps them to the stylesheets (`XMLValidator`), with the
/// ZUGFeRD 2 equivalents.
const FACTURX_PROFILES: &[(&str, &[&str])] = &[
    ("urn:factur-x.eu:1p0:minimum", &["FACTUR-X_MINIMUM"]),
    ("urn:zugferd.de:2p0:minimum", &["FACTUR-X_MINIMUM"]),
    ("urn:factur-x.eu:1p0:basicwl", &["FACTUR-X_BASIC-WL"]),
    ("urn:zugferd.de:2p0:basicwl", &["FACTUR-X_BASIC-WL"]),
    (
        "urn:cen.eu:en16931:2017#compliant#urn:factur-x.eu:1p0:basic",
        &["FACTUR-X_BASIC"],
    ),
    (
        "urn:cen.eu:en16931:2017#compliant#urn:zugferd.de:2p0:basic",
        &["FACTUR-X_BASIC"],
    ),
    ("urn:cen.eu:en16931:2017", &["FACTUR-X_EN16931"]),
    (
        "urn:cen.eu:en16931:2017#conformant#urn:factur-x.eu:1p0:extended",
        &["FACTUR-X_EXTENDED"],
    ),
    (
        "urn:cen.eu:en16931:2017#conformant#urn:zugferd.de:2p0:extended",
        &["FACTUR-X_EXTENDED"],
    ),
];

/// A Factur-X stylesheet of the Mustang validator, in the bundle of its profile.
macro_rules! facturx {
    ($name:literal, $dir:literal, $pin:literal, $codedb:literal, $codedb_pin:literal) => {
        Stylesheet {
            name: $name,
            xslt: concat!($dir, "/", $name, ".xslt"),
            golden_dir: concat!("golden-", $dir),
            sha256_pin: Some($pin),
            side_files: &[($codedb, $codedb_pin)],
            generator: Generator::IsoSkeleton,
        }
    };
}

/// Factur-X 1.09.2 (ZUGFeRD 2.5.2), CII: the stylesheets of its five profiles
/// as the Mustang validator 2.26.0 runs them, fetched by
/// `scripts/fetch-facturx.sh`. Each runs on every case.
pub(crate) const FACTURX: RuleSet = RuleSet {
    name: "factur-x-1.09.2",
    origin: Origin::Fetched {
        base: "target/facturx/factur-x-1.09.2",
        corpus: "corpus",
    },
    source: "source",
    stylesheets: &[
        facturx!(
            "FACTUR-X_MINIMUM",
            "minimum",
            "minimum_xslt_sha256",
            "FACTUR-X_MINIMUM_codedb.xml",
            "minimum_codedb_sha256"
        ),
        facturx!(
            "FACTUR-X_BASIC-WL",
            "basic-wl",
            "basic_wl_xslt_sha256",
            "FACTUR-X_BASIC-WL_codedb.xml",
            "basic_wl_codedb_sha256"
        ),
        facturx!(
            "FACTUR-X_BASIC",
            "basic",
            "basic_xslt_sha256",
            "FACTUR-X_BASIC_codedb.xml",
            "basic_codedb_sha256"
        ),
        facturx!(
            "FACTUR-X_EN16931",
            "en16931",
            "en16931_xslt_sha256",
            "FACTUR-X_EN16931_codedb.xml",
            "en16931_codedb_sha256"
        ),
        facturx!(
            "FACTUR-X_EXTENDED",
            "extended",
            "extended_xslt_sha256",
            "FACTUR-X_EXTENDED_codedb.xml",
            "extended_codedb_sha256"
        ),
    ],
    document_dirs: &[
        ("z12-012", "examples/z12-012"),
        ("z12-014", "examples/z12-014"),
        ("cen-examples", "cen-cii-examples"),
    ],
    vefa_dirs: &[],
    xmute_dirs: &[],
    xmute_schematrons: &[],
    ignored: &[],
    mutation_catalogue: Some(CII_MUTATIONS),
    mutation_bases: CEN_CII_BASES,
    acceptance: Some(Acceptance {
        cases: &["z12-012", "z12-014"],
        profiles: Profiles::Listed(FACTURX_PROFILES),
    }),
    record: RECORD,
};

/// The documents XML Mutate 0.6 writes for a mutation whose name contains
/// `--` (a negative value of a `code` mutator): the comment that names the
/// active mutation then contains `--`, so the document is not well-formed XML
/// and no XML processor reads it.
const XMUTE_UBL_NOT_WELL_FORMED: &[&str] = &[
    "xmute/ubl/ubl-cn-br-de-27-test-777-code--0-1-.xml",
    "xmute/ubl/ubl-cn-peppol-en16931-r040-1016-code--0.02.xml",
    "xmute/ubl/ubl-cn-peppol-en16931-r040-1020-code--0.02.xml",
    "xmute/ubl/ubl-cn-peppol-en16931-r055-1-1064-code--2048.44.xml",
    "xmute/ubl/ubl-cn-peppol-en16931-r055-1-1068-code--2048.44.xml",
    "xmute/ubl/ubl-cn-peppol-en16931-r055-2-1071-code--2000.xml",
    "xmute/ubl/ubl-cn-peppol-en16931-r055-2-1075-code--2000.xml",
    "xmute/ubl/ubl-cn-peppol-en16931-r121-1116-code--2.xml",
    "xmute/ubl/ubl-inv-br-de-27-test-147-code--0-1-.xml",
    "xmute/ubl/ubl-inv-peppol-en16931-r040-543-code--0.02.xml",
    "xmute/ubl/ubl-inv-peppol-en16931-r040-547-code--0.02.xml",
    "xmute/ubl/ubl-inv-peppol-en16931-r055-1-591-code--2048.44.xml",
    "xmute/ubl/ubl-inv-peppol-en16931-r055-1-595-code--2048.44.xml",
    "xmute/ubl/ubl-inv-peppol-en16931-r055-2-598-code--2000.xml",
    "xmute/ubl/ubl-inv-peppol-en16931-r055-2-602-code--2000.xml",
    "xmute/ubl/ubl-inv-peppol-en16931-r121-643-code--2.xml",
];
/// The same, for CII.
const XMUTE_CII_NOT_WELL_FORMED: &[&str] = &[
    "xmute/cii/cii-br-de-27-test-131-code--0-1-.xml",
    "xmute/cii/cii-peppol-en16931-r055-570-code--123.45.xml",
    "xmute/cii/cii-peppol-en16931-r055-572-code--22.04.xml",
    "xmute/cii/cii-peppol-en16931-r121-624-code--10.xml",
    "xmute/cii/cii-peppol-en16931-r121-627-code--10.xml",
];

/// A stylesheet of the XRechnung validator configuration.
macro_rules! kosit {
    ($name:literal, $location:literal, $golden:literal, $pin:literal) => {
        Stylesheet {
            name: $name,
            xslt: concat!("source/configuration/", $location),
            golden_dir: $golden,
            sha256_pin: Some($pin),
            side_files: &[],
            generator: Generator::SchXslt,
        }
    };
}

/// XRechnung 3.0.2, UBL (Invoice and CreditNote): the EN 16931 rules and the
/// XRechnung rules of the KoSIT validator configuration 2026-08-31 (XRechnung
/// Schematron 2.6.0), fetched by `scripts/fetch-xrechnung.sh`.
pub(crate) const XRECHNUNG_UBL: RuleSet = RuleSet {
    name: "xrechnung-ubl-3.0.2",
    origin: Origin::Fetched {
        base: "target/xrechnung/xrechnung-3.0.2",
        corpus: "corpus-ubl",
    },
    source: "source",
    stylesheets: &[
        kosit!(
            "EN16931-UBL",
            "resources/ubl/2.1/xsl/EN16931-UBL-validation.xsl",
            "golden-cen",
            "cen_ubl_sha256"
        ),
        kosit!(
            "XRechnung-UBL",
            "resources/xrechnung/3.0.2/xsl/XRechnung-UBL-validation.xsl",
            "golden-xrechnung",
            "xrechnung_ubl_sha256"
        ),
    ],
    document_dirs: &[
        ("testsuite", "testsuite/ubl"),
        ("cen-examples", "cen-ubl-examples"),
        ("cen-testfiles", "cen-ubl-testfiles"),
    ],
    vefa_dirs: &[],
    xmute_dirs: &[("xmute", "xmute/ubl")],
    xmute_schematrons: &[("xrubl", "XRechnung-UBL")],
    ignored: XMUTE_UBL_NOT_WELL_FORMED,
    mutation_catalogue: Some(UBL_MUTATIONS),
    mutation_bases: CEN_UBL_BASES,
    acceptance: Some(Acceptance {
        cases: &["testsuite"],
        profiles: Profiles::KositScenarios {
            file: "source/configuration/scenarios.xml",
            bundle: "ubl",
        },
    }),
    record: RECORD,
};

/// XRechnung 3.0.2, CII: as [`XRECHNUNG_UBL`].
pub(crate) const XRECHNUNG_CII: RuleSet = RuleSet {
    name: "xrechnung-cii-3.0.2",
    origin: Origin::Fetched {
        base: "target/xrechnung/xrechnung-3.0.2",
        corpus: "corpus-cii",
    },
    source: "source",
    stylesheets: &[
        kosit!(
            "EN16931-CII",
            "resources/cii/16b/xsl/EN16931-CII-validation.xsl",
            "golden-cen",
            "cen_cii_sha256"
        ),
        kosit!(
            "XRechnung-CII",
            "resources/xrechnung/3.0.2/xsl/XRechnung-CII-validation.xsl",
            "golden-xrechnung",
            "xrechnung_cii_sha256"
        ),
    ],
    document_dirs: &[
        ("testsuite", "testsuite/cii"),
        ("cen-examples", "cen-cii-examples"),
    ],
    vefa_dirs: &[],
    xmute_dirs: &[("xmute", "xmute/cii")],
    xmute_schematrons: &[("xrcii", "XRechnung-CII")],
    ignored: XMUTE_CII_NOT_WELL_FORMED,
    mutation_catalogue: Some(CII_MUTATIONS),
    mutation_bases: CEN_CII_BASES,
    acceptance: Some(Acceptance {
        cases: &["testsuite"],
        profiles: Profiles::KositScenarios {
            file: "source/configuration/scenarios.xml",
            bundle: "cii",
        },
    }),
    record: RECORD,
};

/// A stylesheet of the FNFE-MPE package, in the bundle `$dir` of
/// `scripts/fetch-fr-ctc.sh`.
macro_rules! fnfe {
    ($name:literal, $dir:literal, $golden:literal, $pin:literal) => {
        Stylesheet {
            name: $name,
            xslt: concat!($dir, "/", $name, ".xslt"),
            golden_dir: $golden,
            sha256_pin: Some($pin),
            side_files: &[],
            generator: Generator::IsoSkeleton,
        }
    };
    ($name:literal, $dir:literal, $golden:literal, $pin:literal, $codedb:literal, $codedb_pin:literal) => {
        Stylesheet {
            name: $name,
            xslt: concat!($dir, "/", $name, ".xslt"),
            golden_dir: $golden,
            sha256_pin: Some($pin),
            side_files: &[($codedb, $codedb_pin)],
            generator: Generator::IsoSkeleton,
        }
    };
}

/// The French rules for CII (FNFE-MPE package 1.4.0.04): EN 16931 as the
/// package ships it, EXTENDED-CTC-FR, the BR-FR rules and their WARNING
/// version, and the Factur-X profiles BASIC WL, EN16931 and EXTENDED as the
/// package builds them, fetched by `scripts/fetch-fr-ctc.sh`.
pub(crate) const FR_CTC_CII: RuleSet = RuleSet {
    name: "fr-ctc-cii-1.4.0.04",
    origin: Origin::Fetched {
        base: "target/fr-ctc/fr-ctc-1.4.0.04",
        corpus: "corpus-cii",
    },
    source: "source",
    stylesheets: &[
        fnfe!(
            "EN16931-CII",
            "cii-en16931",
            "golden-en16931",
            "cii_en16931_sha256"
        ),
        fnfe!(
            "EXTENDED-CTC-FR-CII",
            "cii-extended-ctc-fr",
            "golden-extended-ctc-fr",
            "cii_extended_ctc_fr_sha256"
        ),
        fnfe!(
            "BR-FR-Flux2-CII",
            "cii-en16931",
            "golden-br-fr",
            "cii_br_fr_sha256"
        ),
        fnfe!(
            "BR-FR-Flux2-CII_WARNING",
            "cii-en16931-warning",
            "golden-br-fr-warning",
            "cii_br_fr_warning_sha256"
        ),
        fnfe!(
            "FACTUR-X_BASIC-WL",
            "factur-x-basic-wl",
            "golden-factur-x-basic-wl",
            "fx_basic_wl_sha256",
            "FACTUR-X_BASIC-WL_codedb.xml",
            "fx_basic_wl_codedb_sha256"
        ),
        fnfe!(
            "FACTUR-X_EN16931",
            "factur-x-en16931",
            "golden-factur-x-en16931",
            "fx_en16931_sha256",
            "FACTUR-X_EN16931_codedb.xml",
            "fx_en16931_codedb_sha256"
        ),
        fnfe!(
            "FACTUR-X_EXTENDED",
            "factur-x-extended",
            "golden-factur-x-extended",
            "fx_extended_sha256",
            "FACTUR-X_EXTENDED_codedb.xml",
            "fx_extended_codedb_sha256"
        ),
    ],
    document_dirs: &[
        ("z12-012", "examples/z12-012/cii"),
        ("z12-014", "examples/z12-014/cii"),
        ("cen-examples", "cen-cii-examples"),
    ],
    vefa_dirs: &[],
    xmute_dirs: &[],
    xmute_schematrons: &[],
    ignored: &[],
    mutation_catalogue: Some(CII_MUTATIONS),
    mutation_bases: CEN_CII_BASES,
    acceptance: Some(Acceptance {
        cases: &["z12-012", "z12-014"],
        profiles: Profiles::Listed(&[
            (
                "urn:cen.eu:en16931:2017",
                &[
                    "EN16931-CII",
                    "FACTUR-X_EN16931",
                    "BR-FR-Flux2-CII",
                    "BR-FR-Flux2-CII_WARNING",
                ],
            ),
            (
                "urn:cen.eu:en16931:2017#conformant#urn.cpro.gouv.fr:1p0:extended-ctc-fr",
                &[
                    "EXTENDED-CTC-FR-CII",
                    "BR-FR-Flux2-CII",
                    "BR-FR-Flux2-CII_WARNING",
                ],
            ),
            (
                "urn:cen.eu:en16931:2017#conformant#urn:factur-x.eu:1p0:extended",
                &[
                    "FACTUR-X_EXTENDED",
                    "BR-FR-Flux2-CII",
                    "BR-FR-Flux2-CII_WARNING",
                ],
            ),
            (
                "urn:factur-x.eu:1p0:basicwl",
                &[
                    "FACTUR-X_BASIC-WL",
                    "BR-FR-Flux2-CII",
                    "BR-FR-Flux2-CII_WARNING",
                ],
            ),
        ]),
    }),
    record: RECORD,
};

/// The French rules for UBL (FNFE-MPE package 1.4.0.04): EN 16931 as the
/// package ships it, EXTENDED-CTC-FR, the BR-FR rules and their WARNING
/// version.
pub(crate) const FR_CTC_UBL: RuleSet = RuleSet {
    name: "fr-ctc-ubl-1.4.0.04",
    origin: Origin::Fetched {
        base: "target/fr-ctc/fr-ctc-1.4.0.04",
        corpus: "corpus-ubl",
    },
    source: "source",
    stylesheets: &[
        fnfe!(
            "EN16931-UBL",
            "ubl-en16931",
            "golden-en16931",
            "ubl_en16931_sha256"
        ),
        fnfe!(
            "EXTENDED-CTC-FR-UBL",
            "ubl-extended-ctc-fr",
            "golden-extended-ctc-fr",
            "ubl_extended_ctc_fr_sha256"
        ),
        fnfe!(
            "BR-FR-Flux2-UBL",
            "ubl-en16931",
            "golden-br-fr",
            "ubl_br_fr_sha256"
        ),
        fnfe!(
            "BR-FR-Flux2-UBL_WARNING",
            "ubl-en16931-warning",
            "golden-br-fr-warning",
            "ubl_br_fr_warning_sha256"
        ),
    ],
    document_dirs: &[
        ("z12-012", "examples/z12-012/ubl"),
        ("z12-014", "examples/z12-014/ubl"),
        ("cen-examples", "cen-ubl-examples"),
        ("cen-testfiles", "cen-ubl-testfiles"),
    ],
    vefa_dirs: &[],
    xmute_dirs: &[],
    xmute_schematrons: &[],
    ignored: &[],
    mutation_catalogue: Some(UBL_MUTATIONS),
    mutation_bases: CEN_UBL_BASES,
    acceptance: Some(Acceptance {
        cases: &["z12-012", "z12-014"],
        profiles: Profiles::Listed(&[
            (
                "urn:cen.eu:en16931:2017",
                &["EN16931-UBL", "BR-FR-Flux2-UBL", "BR-FR-Flux2-UBL_WARNING"],
            ),
            (
                "urn:cen.eu:en16931:2017#conformant#urn.cpro.gouv.fr:1p0:extended-ctc-fr",
                &[
                    "EXTENDED-CTC-FR-UBL",
                    "BR-FR-Flux2-UBL",
                    "BR-FR-Flux2-UBL_WARNING",
                ],
            ),
        ]),
    }),
    record: RECORD,
};

/// The French rules for the life-cycle statuses (CDAR) of the FNFE-MPE
/// package 1.4.0.04, and their WARNING version. There is no mutation
/// catalogue for this syntax.
pub(crate) const FR_CTC_CDAR: RuleSet = RuleSet {
    name: "fr-ctc-cdar-1.4.0.04",
    origin: Origin::Fetched {
        base: "target/fr-ctc/fr-ctc-1.4.0.04",
        corpus: "corpus-cdar",
    },
    source: "source",
    stylesheets: &[
        fnfe!("BR-FR-CDV-CDAR", "cdar", "golden", "cdar_sha256"),
        fnfe!(
            "BR-FR-CDV-CDAR_WARNING",
            "cdar-warning",
            "golden-warning",
            "cdar_warning_sha256"
        ),
    ],
    document_dirs: &[
        ("z12-012", "examples/z12-012/cdar"),
        ("z12-014", "examples/z12-014/cdar"),
    ],
    vefa_dirs: &[],
    xmute_dirs: &[],
    xmute_schematrons: &[],
    ignored: &[],
    mutation_catalogue: None,
    mutation_bases: &[],
    acceptance: Some(Acceptance {
        cases: &["z12-012", "z12-014"],
        profiles: Profiles::Listed(&[
            (
                "urn.cpro.gouv.fr:1p0:CDV:invoice",
                &["BR-FR-CDV-CDAR", "BR-FR-CDV-CDAR_WARNING"],
            ),
            (
                "urn.cpro.gouv.fr:1p0:CDV:einvoicingF2",
                &["BR-FR-CDV-CDAR", "BR-FR-CDV-CDAR_WARNING"],
            ),
        ]),
    }),
    record: RECORD,
};

/// The vendored rule sets: their corpus is committed.
pub(crate) const VENDORED: &[&RuleSet] = &[&CEN_UBL, &CEN_CII];

/// Peppol BIS Billing 3.0.
pub(crate) const PEPPOL: Family = Family {
    name: "peppol",
    script: "scripts/fetch-peppol.sh",
    version_pin: "tag",
    rule_sets: &[&PEPPOL_UBL],
};
/// Factur-X.
pub(crate) const FACTURX_FAMILY: Family = Family {
    name: "facturx",
    script: "scripts/fetch-facturx.sh",
    version_pin: "facturx",
    rule_sets: &[&FACTURX],
};
/// XRechnung.
pub(crate) const XRECHNUNG: Family = Family {
    name: "xrechnung",
    script: "scripts/fetch-xrechnung.sh",
    version_pin: "xrechnung",
    rule_sets: &[&XRECHNUNG_UBL, &XRECHNUNG_CII],
};
/// The French rules.
pub(crate) const FR_CTC: Family = Family {
    name: "fr-ctc",
    script: "scripts/fetch-fr-ctc.sh",
    version_pin: "pack",
    rule_sets: &[&FR_CTC_CII, &FR_CTC_UBL, &FR_CTC_CDAR],
};

/// The families of fetched rule sets.
pub(crate) const FAMILIES: &[&Family] = &[&PEPPOL, &FACTURX_FAMILY, &XRECHNUNG, &FR_CTC];

/// Every rule set, vendored or fetched.
pub(crate) const ALL: &[&RuleSet] = &[
    &CEN_UBL,
    &CEN_CII,
    &PEPPOL_UBL,
    &FACTURX,
    &XRECHNUNG_UBL,
    &XRECHNUNG_CII,
    &FR_CTC_CII,
    &FR_CTC_UBL,
    &FR_CTC_CDAR,
];

#[cfg(test)]
mod tests {
    use super::{ALL, FAMILIES, Origin, Profiles};

    /// Names, corpus directories and golden directories are unique, every
    /// fetched rule set belongs to exactly one family, and every stylesheet a
    /// profile or an XML Mutate Schematron names exists.
    #[test]
    fn rule_sets_are_consistent() {
        let mut names: Vec<&str> = ALL.iter().map(|r| r.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), ALL.len());
        let mut corpora = Vec::new();
        for rule_set in ALL {
            if let Origin::Fetched { base, corpus } = rule_set.origin {
                let families = FAMILIES
                    .iter()
                    .filter(|f| f.rule_sets.iter().any(|r| r.name == rule_set.name))
                    .count();
                assert_eq!(families, 1, "{}", rule_set.name);
                corpora.push(format!("{base}/{corpus}"));
                for stylesheet in rule_set.stylesheets {
                    assert!(stylesheet.sha256_pin.is_some(), "{}", stylesheet.name);
                }
            }
            let mut goldens: Vec<&str> =
                rule_set.stylesheets.iter().map(|s| s.golden_dir).collect();
            goldens.sort_unstable();
            goldens.dedup();
            assert_eq!(
                goldens.len(),
                rule_set.stylesheets.len(),
                "{}",
                rule_set.name
            );
            if let Some(acceptance) = &rule_set.acceptance
                && let Profiles::Listed(profiles) = acceptance.profiles
            {
                for (_, stylesheets) in profiles {
                    for name in *stylesheets {
                        assert!(rule_set.stylesheet(name).is_some(), "{name}");
                    }
                }
            }
            for (_, name) in rule_set.xmute_schematrons {
                assert!(rule_set.stylesheet(name).is_some(), "{name}");
            }
        }
        let count = corpora.len();
        corpora.sort();
        corpora.dedup();
        assert_eq!(corpora.len(), count);
    }
}
