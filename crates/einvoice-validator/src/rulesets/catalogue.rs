//! The catalogue of the rule sets fetched from their publishers: for each family, the
//! files to download (pinned URL and checksum), the release directory its bundles are
//! assembled in, and for each rule set its bundle and the manifest that bundle has,
//! byte for byte the one the fetch script of the family writes.
//!
//! Every value that can change with a release is a pin of the fetch script, referred to
//! by name (`{name}` in the templates below); nothing here repeats a version or a
//! checksum.

use std::fmt::Write as _;
use std::sync::OnceLock;

use super::pins::{self, Pins};
use crate::{Error, Syntax};

/// A digest algorithm of a pinned download.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Algorithm {
    Sha256,
    Sha512,
}

/// A file a family downloads.
#[derive(Debug)]
pub(crate) struct SourceSpec {
    /// What the file is, for messages (a template).
    pub(crate) what: &'static str,
    /// Its URL (a template).
    pub(crate) url: &'static str,
    /// Its file name in the download directory, as the fetch script names it (a
    /// template).
    pub(crate) file: &'static str,
    pub(crate) algorithm: Algorithm,
    /// The pin of its digest.
    pub(crate) digest: &'static str,
}

/// How the bundles of a family are assembled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    FacturX,
    XRechnung,
    FrCtc,
    Peppol,
}

/// A family of rule sets: the rule sets built from the same downloads, fetched
/// together.
#[derive(Debug)]
pub(crate) struct FamilySpec {
    pub(crate) kind: Kind,
    /// Its name (`factur-x`): `rules fetch` accepts it for the whole family.
    pub(crate) name: &'static str,
    pub(crate) title: &'static str,
    pub(crate) pins: Pins,
    /// The directory its bundles are assembled in (a template), as the fetch script
    /// names it under its output directory.
    pub(crate) release: &'static str,
    /// The version of its rule sets (a template).
    pub(crate) version: &'static str,
    /// Where its files come from (a template).
    pub(crate) publisher: &'static str,
    /// What is downloaded, in order.
    pub(crate) sources: &'static [SourceSpec],
    /// Whether assembling the bundles runs Java (the Schematron compiler).
    pub(crate) needs_java: bool,
}

/// A rule set of a family.
#[derive(Debug)]
pub(crate) struct RuleSetSpec {
    /// The name of its manifest, and of the rule set (`--rules`).
    pub(crate) name: &'static str,
    pub(crate) title: &'static str,
    pub(crate) family: &'static FamilySpec,
    /// Its bundle directory, relative to the release directory (empty: the release
    /// directory itself).
    pub(crate) dir: &'static str,
    /// The syntax of the documents it validates, when it is UBL or CII.
    pub(crate) syntax: Option<Syntax>,
}

// --- Factur-X ----------------------------------------------------------------------

static FACTURX: FamilySpec = FamilySpec {
    kind: Kind::FacturX,
    name: "factur-x",
    title: "Factur-X",
    pins: pins::FACTURX,
    release: "factur-x-{facturx}",
    version: "{facturx}",
    publisher: "the Mustang validator {mustang} (org.mustangproject:validator, Maven Central)",
    sources: &[SourceSpec {
        what: "the Mustang validator {mustang}",
        url: "https://repo1.maven.org/maven2/org/mustangproject/validator/{mustang}/validator-{mustang}.jar",
        file: "mustang-validator-{mustang}.jar",
        algorithm: Algorithm::Sha256,
        digest: "mustang_jar_sha256",
    }],
    needs_java: false,
};

/// The Factur-X profiles: bundle directory, Mustang file name, pins of the stylesheet
/// and of its code list.
pub(crate) const FACTURX_PROFILES: [(&str, &str, &str, &str); 5] = [
    (
        "minimum",
        "MINIMUM",
        "minimum_xslt_sha256",
        "minimum_codedb_sha256",
    ),
    (
        "basic-wl",
        "BASIC-WL",
        "basic_wl_xslt_sha256",
        "basic_wl_codedb_sha256",
    ),
    ("basic", "BASIC", "basic_xslt_sha256", "basic_codedb_sha256"),
    (
        "en16931",
        "EN16931",
        "en16931_xslt_sha256",
        "en16931_codedb_sha256",
    ),
    (
        "extended",
        "EXTENDED",
        "extended_xslt_sha256",
        "extended_codedb_sha256",
    ),
];

// --- XRechnung ---------------------------------------------------------------------

static XRECHNUNG: FamilySpec = FamilySpec {
    kind: Kind::XRechnung,
    name: "xrechnung",
    title: "XRechnung",
    pins: pins::XRECHNUNG,
    release: "xrechnung-{xrechnung}",
    version: "{xrechnung}",
    publisher: "the KoSIT validator configuration {configuration} for XRechnung {xrechnung} (projekte.kosit.org)",
    sources: &[SourceSpec {
        what: "the XRechnung {xrechnung} validator configuration {configuration}",
        url: "https://projekte.kosit.org/api/v4/projects/356/packages/maven/de/xeinkauf/xrechnung-{xrechnung}-validator-configuration/{configuration}/xrechnung-{xrechnung}-validator-configuration-{configuration}.zip",
        file: "xrechnung-{xrechnung}-validator-configuration-{configuration}.zip",
        algorithm: Algorithm::Sha256,
        digest: "configuration_sha256",
    }],
    needs_java: false,
};

/// The stylesheets of the configuration: name in the bundles, path in the
/// configuration (a template), pin of its SHA-256, pin of its version.
pub(crate) const XRECHNUNG_STYLESHEETS: [(&str, &str, &str, &str); 4] = [
    (
        "EN16931-UBL",
        "resources/ubl/2.1/xsl/EN16931-UBL-validation.xsl",
        "cen_ubl_sha256",
        "cen",
    ),
    (
        "XRechnung-UBL",
        "resources/xrechnung/{xrechnung}/xsl/XRechnung-UBL-validation.xsl",
        "xrechnung_ubl_sha256",
        "schematron",
    ),
    (
        "EN16931-CII",
        "resources/cii/16b/xsl/EN16931-CII-validation.xsl",
        "cen_cii_sha256",
        "cen",
    ),
    (
        "XRechnung-CII",
        "resources/xrechnung/{xrechnung}/xsl/XRechnung-CII-validation.xsl",
        "xrechnung_cii_sha256",
        "schematron",
    ),
];

/// The namespaces of the UBL invoice scenarios of the configuration, as their
/// `namespace` elements bind them (`rep` is the namespace of its validation reports).
const XRECHNUNG_UBL_INVOICE_NAMESPACES: &str = "cbc=urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2 invoice=urn:oasis:names:specification:ubl:schema:xsd:Invoice-2 rep=http://www.xoev.de/de/validator/varl/1";
/// The namespaces of the UBL credit note scenarios.
const XRECHNUNG_UBL_CREDIT_NOTE_NAMESPACES: &str = "cbc=urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2 creditnote=urn:oasis:names:specification:ubl:schema:xsd:CreditNote-2 rep=http://www.xoev.de/de/validator/varl/1";
/// The namespaces of the CII scenarios.
const XRECHNUNG_CII_NAMESPACES: &str = "rsm=urn:un:unece:uncefact:data:standard:CrossIndustryInvoice:100 ram=urn:un:unece:uncefact:data:standard:ReusableAggregateBusinessInformationEntity:100 rep=http://www.xoev.de/de/validator/varl/1";

/// A scenario of the configuration: syntax of the bundle, name, namespaces (`prefix=uri`
/// pairs), its `match` expression as written, the stylesheets it runs (bundle names, in
/// order), and the rule ids of the levels error, warning and information (separated by
/// spaces).
type XRechnungScenario = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
);

/// The scenarios of the configuration (`scenarios.xml`), in its order, as
/// `scripts/fetch-xrechnung.sh` lists them: the `[[scenario]]` tables of the bundles
/// (`fatal`, `warning`, `other`), each in the bundle of its syntax.
pub(crate) const XRECHNUNG_SCENARIOS: [XRechnungScenario; 11] = [
    (
        "ubl",
        "EN16931 XRechnung (UBL Invoice)",
        XRECHNUNG_UBL_INVOICE_NAMESPACES,
        "exists(/invoice:Invoice/cbc:CustomizationID[ . = 'urn:cen.eu:en16931:2017#compliant#urn:xeinkauf.de:kosit:xrechnung_3.0']) ",
        "EN16931-UBL XRechnung-UBL",
        "UBL-CR-646",
        "BR-CL-23 BR-CL-21",
        "",
    ),
    (
        "ubl",
        "EN16931 XRechnung Extension (UBL Invoice)",
        XRECHNUNG_UBL_INVOICE_NAMESPACES,
        "exists(/invoice:Invoice/cbc:CustomizationID[ . = 'urn:cen.eu:en16931:2017#compliant#urn:xeinkauf.de:kosit:xrechnung_3.0#conformant#urn:xeinkauf.de:kosit:extension:xrechnung_3.0'])",
        "EN16931-UBL XRechnung-UBL",
        "",
        "BR-CL-23",
        "BR-CL-21 BR-CL-24 BR-CL-10 BR-CL-11 BR-CL-25 BR-CL-26 BR-CO-16 UBL-CR-470 UBL-CR-646",
    ),
    (
        "ubl",
        "EN16931 XRechnung CVD (UBL Invoice)",
        XRECHNUNG_UBL_INVOICE_NAMESPACES,
        "exists(/invoice:Invoice/cbc:CustomizationID[ . = 'urn:cen.eu:en16931:2017#compliant#urn:xeinkauf.de:kosit:xrechnung_3.0#compliant#urn:xeinkauf.de:kosit:xrechnung:cvd_0.9']) ",
        "EN16931-UBL XRechnung-UBL",
        "UBL-CR-646",
        "BR-CL-23 BR-CL-21",
        "BR-CL-13",
    ),
    (
        "ubl",
        "EN16931 XRechnung (UBL CreditNote)",
        XRECHNUNG_UBL_CREDIT_NOTE_NAMESPACES,
        "exists(/creditnote:CreditNote[ cbc:CustomizationID/text() = 'urn:cen.eu:en16931:2017#compliant#urn:xeinkauf.de:kosit:xrechnung_3.0'])",
        "EN16931-UBL XRechnung-UBL",
        "",
        "BR-CL-23 BR-CL-21",
        "",
    ),
    (
        "ubl",
        "EN16931 XRechnung CVD (UBL CreditNote)",
        XRECHNUNG_UBL_CREDIT_NOTE_NAMESPACES,
        "exists(/creditnote:CreditNote[ cbc:CustomizationID/text() = 'urn:cen.eu:en16931:2017#compliant#urn:xeinkauf.de:kosit:xrechnung_3.0#compliant#urn:xeinkauf.de:kosit:xrechnung:cvd_0.9'])",
        "EN16931-UBL XRechnung-UBL",
        "UBL-CR-646",
        "BR-CL-23 BR-CL-21",
        "BR-CL-13",
    ),
    (
        "cii",
        "EN16931 XRechnung (CII)",
        XRECHNUNG_CII_NAMESPACES,
        "exists(/rsm:CrossIndustryInvoice[rsm:ExchangedDocumentContext/ram:GuidelineSpecifiedDocumentContextParameter/ram:ID/text() = 'urn:cen.eu:en16931:2017#compliant#urn:xeinkauf.de:kosit:xrechnung_3.0'])",
        "EN16931-CII XRechnung-CII",
        "CII-SR-452 CII-SR-453 CII-SR-454 CII-SR-465 CII-SR-466",
        "BR-CL-23 BR-CL-21",
        "CII-SR-475 CII-SR-476",
    ),
    (
        "cii",
        "EN16931 XRechnung Extension (CII)",
        XRECHNUNG_CII_NAMESPACES,
        "exists(/rsm:CrossIndustryInvoice[rsm:ExchangedDocumentContext/ram:GuidelineSpecifiedDocumentContextParameter/ram:ID/text() = 'urn:cen.eu:en16931:2017#compliant#urn:xeinkauf.de:kosit:xrechnung_3.0#conformant#urn:xeinkauf.de:kosit:extension:xrechnung_3.0'])",
        "EN16931-CII XRechnung-CII",
        "CII-SR-452 CII-SR-453 CII-SR-454 CII-SR-465 CII-SR-466",
        "BR-CL-23",
        "BR-CL-21 BR-CL-11 BR-CL-10 BR-CL-24 BR-CL-25 BR-CL-26 CII-SR-475 CII-SR-476",
    ),
    (
        "cii",
        "EN16931 XRechnung CVD (CII)",
        XRECHNUNG_CII_NAMESPACES,
        "exists(/rsm:CrossIndustryInvoice[rsm:ExchangedDocumentContext/ram:GuidelineSpecifiedDocumentContextParameter/ram:ID/text() = 'urn:cen.eu:en16931:2017#compliant#urn:xeinkauf.de:kosit:xrechnung_3.0#compliant#urn:xeinkauf.de:kosit:xrechnung:cvd_0.9'])",
        "EN16931-CII XRechnung-CII",
        "CII-SR-452 CII-SR-453 CII-SR-454 CII-SR-465 CII-SR-466",
        "BR-CL-23 BR-CL-21",
        "BR-CL-13 CII-SR-475 CII-SR-476",
    ),
    (
        "ubl",
        "EN16931 (UBL Invoice)",
        XRECHNUNG_UBL_INVOICE_NAMESPACES,
        "exists(/invoice:Invoice[ cbc:CustomizationID/text() = 'urn:cen.eu:en16931:2017' ])",
        "EN16931-UBL",
        "",
        "",
        "",
    ),
    (
        "ubl",
        "EN16931 (UBL CreditNote)",
        XRECHNUNG_UBL_CREDIT_NOTE_NAMESPACES,
        "exists(/creditnote:CreditNote[cbc:CustomizationID/text() = 'urn:cen.eu:en16931:2017'])",
        "EN16931-UBL",
        "",
        "",
        "",
    ),
    (
        "cii",
        "EN16931 (CII)",
        XRECHNUNG_CII_NAMESPACES,
        "exists(/rsm:CrossIndustryInvoice[ rsm:ExchangedDocumentContext/ram:GuidelineSpecifiedDocumentContextParameter/ram:ID/text() = 'urn:cen.eu:en16931:2017'])",
        "EN16931-CII",
        "",
        "",
        "",
    ),
];

/// The XML schema the CII scenarios of the configuration validate documents against
/// (`validateWithXmlSchema`), which the CII bundle does not run: the `unchecked-schema`
/// of its manifest.
pub(crate) const XRECHNUNG_CII_SCHEMA: &str =
    "UN/CEFACT CII D16B (CrossIndustryInvoice_100pD16B.xsd)";

// --- The French rules --------------------------------------------------------------

static FR_CTC: FamilySpec = FamilySpec {
    kind: Kind::FrCtc,
    name: "fr-ctc",
    title: "French e-invoicing rules (FNFE-MPE)",
    pins: pins::FR_CTC,
    release: "fr-ctc-{pack}",
    version: "{pack}",
    publisher: "the FNFE-MPE package Schematrons FR CTC {pack} (fnfe-mpe.org)",
    sources: &[SourceSpec {
        what: "the FNFE-MPE package {pack}",
        url: "https://fnfe-mpe.org/wp-content/uploads/2026/09/2026_09_04_FNFE_SCHEMATRONS_FR_CTC_V{pack}.zip",
        file: "2026_09_04_FNFE_SCHEMATRONS_FR_CTC_V{pack}.zip",
        algorithm: Algorithm::Sha256,
        digest: "pack_sha256",
    }],
    needs_java: false,
};

/// The top directory of the package (a template).
#[cfg(feature = "fetch")]
pub(crate) const FR_CTC_PACK_DIR: &str = "2026_09_04_FNFE_SCHEMATRONS_FR_CTC_V{pack}";

/// The stylesheets the bundles name: name in the bundles, path in the package, pin of
/// its SHA-256, pin of the SHA-256 of the code list it reads with `document()` (named
/// after it: `<path without .xslt>_codedb.xml`).
pub(crate) const FR_CTC_STYLESHEETS: [(&str, &str, &str, Option<&str>); 14] = [
    (
        "EN16931-CII",
        "CII/EN16931/2xslt/EN16931-CII-validation.xslt",
        "cii_en16931_sha256",
        None,
    ),
    (
        "EXTENDED-CTC-FR-CII",
        "CII/EXTENDED-CTC-FR/2xslt/EXTENDED-CTC-FR-CII.xslt",
        "cii_extended_ctc_fr_sha256",
        None,
    ),
    (
        "BR-FR-Flux2-CII",
        "CII/EN16931/2xslt/BR-FR-Flux2-Schematron-CII.xslt",
        "cii_br_fr_sha256",
        None,
    ),
    (
        "BR-FR-Flux2-CII_WARNING",
        "CII/EN16931/2xslt/BR-FR-Flux2-Schematron-CII_WARNING.xslt",
        "cii_br_fr_warning_sha256",
        None,
    ),
    (
        "EN16931-UBL",
        "UBL/EN16931/2xslt/EN16931-UBL-validation.xslt",
        "ubl_en16931_sha256",
        None,
    ),
    (
        "EXTENDED-CTC-FR-UBL",
        "UBL/EXTENDED-CTC-FR/2xslt/EXTENDED-CTC-FR-UBL.xslt",
        "ubl_extended_ctc_fr_sha256",
        None,
    ),
    (
        "BR-FR-Flux2-UBL",
        "UBL/EN16931/2xslt/BR-FR-Flux2-Schematron-UBL.xslt",
        "ubl_br_fr_sha256",
        None,
    ),
    (
        "BR-FR-Flux2-UBL_WARNING",
        "UBL/EN16931/2xslt/BR-FR-Flux2-Schematron-UBL_WARNING.xslt",
        "ubl_br_fr_warning_sha256",
        None,
    ),
    (
        "BR-FR-CDV-CDAR",
        "CDAR/2xslt/BR-FR-CDV-Schematron-CDAR.xslt",
        "cdar_sha256",
        None,
    ),
    (
        "BR-FR-CDV-CDAR_WARNING",
        "CDAR/2xslt/BR-FR-CDV-Schematron-CDAR_WARNING.xslt",
        "cdar_warning_sha256",
        None,
    ),
    ("Flux10", "Flux10/Flux10.xslt", "flux10_sha256", None),
    (
        "FACTUR-X_BASIC-WL",
        "Factur-X_1.09.2.fixFR04/BASICWL/2xslt/FACTUR-X_BASIC-WL.xslt",
        "fx_basic_wl_sha256",
        Some("fx_basic_wl_codedb_sha256"),
    ),
    (
        "FACTUR-X_EN16931",
        "Factur-X_1.09.2.fixFR04/EN16931/2xslt/FACTUR-X_EN16931.xslt",
        "fx_en16931_sha256",
        Some("fx_en16931_codedb_sha256"),
    ),
    (
        "FACTUR-X_EXTENDED",
        "Factur-X_1.09.2.fixFR04/EXTENDED/2xslt/FACTUR-X_EXTENDED.xslt",
        "fx_extended_sha256",
        Some("fx_extended_codedb_sha256"),
    ),
];

/// The bundles of the French rules: directory, whether it validates against the UBL
/// 2.1 schema, title, stylesheets in order.
pub(crate) const FR_CTC_BUNDLES: [(&str, bool, &str, &[&str]); 14] = [
    (
        "cii-en16931",
        false,
        "EN 16931 CII and the French rules",
        &["EN16931-CII", "BR-FR-Flux2-CII"],
    ),
    (
        "cii-en16931-warning",
        false,
        "EN 16931 CII and the French rules (WARNING version)",
        &["EN16931-CII", "BR-FR-Flux2-CII_WARNING"],
    ),
    (
        "cii-extended-ctc-fr",
        false,
        "EXTENDED-CTC-FR CII and the French rules",
        &["EXTENDED-CTC-FR-CII", "BR-FR-Flux2-CII"],
    ),
    (
        "cii-extended-ctc-fr-warning",
        false,
        "EXTENDED-CTC-FR CII and the French rules (WARNING version)",
        &["EXTENDED-CTC-FR-CII", "BR-FR-Flux2-CII_WARNING"],
    ),
    (
        "ubl-en16931",
        true,
        "EN 16931 UBL and the French rules",
        &["EN16931-UBL", "BR-FR-Flux2-UBL"],
    ),
    (
        "ubl-en16931-warning",
        true,
        "EN 16931 UBL and the French rules (WARNING version)",
        &["EN16931-UBL", "BR-FR-Flux2-UBL_WARNING"],
    ),
    (
        "ubl-extended-ctc-fr",
        true,
        "EXTENDED-CTC-FR UBL and the French rules",
        &["EXTENDED-CTC-FR-UBL", "BR-FR-Flux2-UBL"],
    ),
    (
        "ubl-extended-ctc-fr-warning",
        true,
        "EXTENDED-CTC-FR UBL and the French rules (WARNING version)",
        &["EXTENDED-CTC-FR-UBL", "BR-FR-Flux2-UBL_WARNING"],
    ),
    (
        "cdar",
        false,
        "Life-cycle statuses (CDAR)",
        &["BR-FR-CDV-CDAR"],
    ),
    (
        "cdar-warning",
        false,
        "Life-cycle statuses (CDAR, WARNING version)",
        &["BR-FR-CDV-CDAR_WARNING"],
    ),
    (
        "factur-x-basic-wl",
        false,
        "Factur-X BASIC WL and the French rules",
        &["FACTUR-X_BASIC-WL", "BR-FR-Flux2-CII"],
    ),
    (
        "factur-x-en16931",
        false,
        "Factur-X EN16931 and the French rules",
        &["FACTUR-X_EN16931", "BR-FR-Flux2-CII"],
    ),
    (
        "factur-x-extended",
        false,
        "Factur-X EXTENDED and the French rules",
        &["FACTUR-X_EXTENDED", "BR-FR-Flux2-CII"],
    ),
    ("flux10", false, "E-reporting (Flux 10)", &["Flux10"]),
];

/// Whether the stylesheet `name` of the package is a Factur-X profile (`FACTUR-X_<P>`),
/// whose failed asserts without a flag are fatal, as the Mustang validator reads them.
pub(crate) fn fr_ctc_factur_x(name: &str) -> bool {
    name.starts_with("FACTUR-X_")
}

/// The code list a Factur-X stylesheet of the package reads: its path, from the
/// stylesheet's.
pub(crate) fn fr_ctc_side_file(path: &str) -> Option<String> {
    let base = path.rsplit('/').next().unwrap_or(path);
    if base.starts_with("FACTUR-X_") && path.contains('/') {
        path.strip_suffix(".xslt")
            .map(|p| format!("{p}_codedb.xml"))
    } else {
        None
    }
}

// --- Peppol BIS Billing 3.0 ---------------------------------------------------------

static PEPPOL: FamilySpec = FamilySpec {
    kind: Kind::Peppol,
    name: "peppol",
    title: "Peppol BIS Billing",
    pins: pins::PEPPOL,
    release: "peppol-bis-billing-ubl-{tag}",
    version: "{tag}",
    publisher: "the Peppol BIS Billing {tag} release (OpenPeppol, github.com), its Schematron compiled with ph-schematron {plugin} (Apache Maven {maven}, Java)",
    sources: &[
        SourceSpec {
            what: "the Peppol BIS Billing {tag} release archive",
            url: "https://github.com/OpenPeppol/peppol-bis-invoice-3/archive/refs/tags/{tag}.tar.gz",
            file: "peppol-bis-invoice-3-{tag}.tar.gz",
            algorithm: Algorithm::Sha256,
            digest: "archive_sha256",
        },
        SourceSpec {
            what: "Apache Maven {maven}",
            url: "https://repo1.maven.org/maven2/org/apache/maven/apache-maven/{maven}/apache-maven-{maven}-bin.tar.gz",
            file: "apache-maven-{maven}-bin.tar.gz",
            algorithm: Algorithm::Sha512,
            digest: "maven_sha512",
        },
    ],
    needs_java: true,
};

/// The two compiled stylesheets of Peppol, in the order they run: name (and file name
/// without `.xslt`), pin of the SHA-256 of the compiled file.
#[cfg(feature = "fetch")]
pub(crate) const PEPPOL_STYLESHEETS: [(&str, &str); 2] = [
    ("CEN-EN16931-UBL", "cen_xslt_sha256"),
    ("PEPPOL-EN16931-UBL", "peppol_xslt_sha256"),
];

// --- The rule sets -----------------------------------------------------------------

const fn rule_set(
    name: &'static str,
    title: &'static str,
    family: &'static FamilySpec,
    dir: &'static str,
    syntax: Option<Syntax>,
) -> RuleSetSpec {
    RuleSetSpec {
        name,
        title,
        family,
        dir,
        syntax,
    }
}

const CII: Option<Syntax> = Some(Syntax::Cii);
const UBL: Option<Syntax> = Some(Syntax::Ubl);

/// Every fetched rule set, by family.
pub(crate) static RULE_SETS: [RuleSetSpec; 22] = [
    rule_set(
        "factur-x-minimum",
        "Factur-X MINIMUM",
        &FACTURX,
        "minimum",
        CII,
    ),
    rule_set(
        "factur-x-basic-wl",
        "Factur-X BASIC-WL",
        &FACTURX,
        "basic-wl",
        CII,
    ),
    rule_set("factur-x-basic", "Factur-X BASIC", &FACTURX, "basic", CII),
    rule_set(
        "factur-x-en16931",
        "Factur-X EN16931",
        &FACTURX,
        "en16931",
        CII,
    ),
    rule_set(
        "factur-x-extended",
        "Factur-X EXTENDED",
        &FACTURX,
        "extended",
        CII,
    ),
    rule_set("xrechnung-ubl", "XRechnung UBL", &XRECHNUNG, "ubl", UBL),
    rule_set("xrechnung-cii", "XRechnung CII", &XRECHNUNG, "cii", CII),
    rule_set(
        "fr-ctc-cii-en16931",
        FR_CTC_BUNDLES[0].2,
        &FR_CTC,
        FR_CTC_BUNDLES[0].0,
        CII,
    ),
    rule_set(
        "fr-ctc-cii-en16931-warning",
        FR_CTC_BUNDLES[1].2,
        &FR_CTC,
        FR_CTC_BUNDLES[1].0,
        CII,
    ),
    rule_set(
        "fr-ctc-cii-extended-ctc-fr",
        FR_CTC_BUNDLES[2].2,
        &FR_CTC,
        FR_CTC_BUNDLES[2].0,
        CII,
    ),
    rule_set(
        "fr-ctc-cii-extended-ctc-fr-warning",
        FR_CTC_BUNDLES[3].2,
        &FR_CTC,
        FR_CTC_BUNDLES[3].0,
        CII,
    ),
    rule_set(
        "fr-ctc-ubl-en16931",
        FR_CTC_BUNDLES[4].2,
        &FR_CTC,
        FR_CTC_BUNDLES[4].0,
        UBL,
    ),
    rule_set(
        "fr-ctc-ubl-en16931-warning",
        FR_CTC_BUNDLES[5].2,
        &FR_CTC,
        FR_CTC_BUNDLES[5].0,
        UBL,
    ),
    rule_set(
        "fr-ctc-ubl-extended-ctc-fr",
        FR_CTC_BUNDLES[6].2,
        &FR_CTC,
        FR_CTC_BUNDLES[6].0,
        UBL,
    ),
    rule_set(
        "fr-ctc-ubl-extended-ctc-fr-warning",
        FR_CTC_BUNDLES[7].2,
        &FR_CTC,
        FR_CTC_BUNDLES[7].0,
        UBL,
    ),
    rule_set(
        "fr-ctc-cdar",
        FR_CTC_BUNDLES[8].2,
        &FR_CTC,
        FR_CTC_BUNDLES[8].0,
        None,
    ),
    rule_set(
        "fr-ctc-cdar-warning",
        FR_CTC_BUNDLES[9].2,
        &FR_CTC,
        FR_CTC_BUNDLES[9].0,
        None,
    ),
    rule_set(
        "fr-ctc-factur-x-basic-wl",
        FR_CTC_BUNDLES[10].2,
        &FR_CTC,
        FR_CTC_BUNDLES[10].0,
        CII,
    ),
    rule_set(
        "fr-ctc-factur-x-en16931",
        FR_CTC_BUNDLES[11].2,
        &FR_CTC,
        FR_CTC_BUNDLES[11].0,
        CII,
    ),
    rule_set(
        "fr-ctc-factur-x-extended",
        FR_CTC_BUNDLES[12].2,
        &FR_CTC,
        FR_CTC_BUNDLES[12].0,
        CII,
    ),
    rule_set(
        "fr-ctc-flux10",
        FR_CTC_BUNDLES[13].2,
        &FR_CTC,
        FR_CTC_BUNDLES[13].0,
        None,
    ),
    rule_set(
        "peppol-bis-billing-ubl",
        "Peppol BIS Billing 3.0 UBL",
        &PEPPOL,
        "",
        UBL,
    ),
];

/// Every family, in the order of the catalogue.
#[cfg(any(test, feature = "fetch"))]
pub(crate) static FAMILIES: [&FamilySpec; 4] = [&FACTURX, &XRECHNUNG, &FR_CTC, &PEPPOL];

/// The rule sets an alias stands for: for a syntax, or for any (`None`).
pub(crate) type AliasTargets = &'static [(Option<Syntax>, &'static str)];

/// Names that stand for rule sets: a family for `rules fetch`, and for validation the
/// rule set of the document's syntax (the first of the list when it is neither).
pub(crate) const ALIASES: [(&str, AliasTargets); 2] = [
    ("peppol", &[(None, "peppol-bis-billing-ubl")]),
    (
        "xrechnung",
        &[(UBL, "xrechnung-ubl"), (CII, "xrechnung-cii")],
    ),
];

// --- Resolution --------------------------------------------------------------------

/// A pinned download, resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    what: String,
    url: String,
    file: String,
    algorithm: Algorithm,
    digest: String,
}

impl Source {
    /// What the file is (`the Mustang validator 2.26.0`).
    #[must_use]
    pub fn what(&self) -> &str {
        &self.what
    }

    /// The URL it is downloaded from, at its publisher.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Its file name, as the fetch scripts of the repository name it in their download
    /// directory (what `FetchOptions::from_dir`, feature `fetch`, looks
    /// for).
    #[must_use]
    pub fn file(&self) -> &str {
        &self.file
    }

    /// `SHA-256` or `SHA-512`: the digest pinned for it.
    #[must_use]
    pub const fn algorithm(&self) -> &'static str {
        match self.algorithm {
            Algorithm::Sha256 => "SHA-256",
            Algorithm::Sha512 => "SHA-512",
        }
    }

    #[cfg(any(test, feature = "fetch"))]
    pub(crate) const fn algorithm_kind(&self) -> Algorithm {
        self.algorithm
    }

    /// The pinned digest, lower-case hexadecimal.
    #[must_use]
    pub fn digest(&self) -> &str {
        &self.digest
    }

    /// A source of the tests.
    #[cfg(all(test, feature = "fetch"))]
    pub(crate) fn for_test(url: &str, file: &str, sha256: &str) -> Self {
        Self {
            what: format!("test file {file}"),
            url: url.to_owned(),
            file: file.to_owned(),
            algorithm: Algorithm::Sha256,
            digest: sha256.to_owned(),
        }
    }
}

/// A rule set this crate fetches from its publisher on demand
/// ([`RuleSetCache`](crate::RuleSetCache)): its files are not redistributed by the
/// project. Each is one bundle of a family; the rule sets of a family come from the
/// same downloads and are fetched together.
#[derive(Debug)]
pub struct FetchedRuleSet {
    pub(crate) spec: &'static RuleSetSpec,
    version: String,
    release: String,
    publisher: String,
    sources: Vec<Source>,
    manifest: String,
}

impl FetchedRuleSet {
    /// Its name, as `--rules` and [`Validator::from_cache`](crate::Validator::from_cache)
    /// take it, and as its manifest gives it (`factur-x-en16931`).
    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.spec.name
    }

    /// Its title (`Factur-X EN16931`).
    #[must_use]
    pub const fn title(&self) -> &'static str {
        self.spec.title
    }

    /// The version of the official artefacts (`1.09.2`).
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    /// The name of its family (`factur-x`, `xrechnung`, `fr-ctc`, `peppol`), which
    /// `RuleSetCache::fetch` (feature `fetch`) also takes.
    #[must_use]
    pub const fn family(&self) -> &'static str {
        self.spec.family.name
    }

    /// The title of its family (`Factur-X`).
    #[must_use]
    pub const fn family_title(&self) -> &'static str {
        self.spec.family.title
    }

    /// Where its files come from.
    #[must_use]
    pub fn publisher(&self) -> &str {
        &self.publisher
    }

    /// The files fetched for its family, with their pinned URLs and digests.
    #[must_use]
    pub fn sources(&self) -> &[Source] {
        &self.sources
    }

    /// Whether building its bundle runs Java: Peppol BIS Billing publishes its rules as
    /// Schematron only, which the reference validators compile with ph-schematron.
    #[must_use]
    pub const fn needs_java(&self) -> bool {
        self.spec.family.needs_java
    }

    /// The syntax of the documents it validates, when it is UBL or CII.
    #[must_use]
    pub const fn syntax(&self) -> Option<Syntax> {
        self.spec.syntax
    }

    /// The directory of its bundle, relative to the cache's rule-set directory:
    /// `<release>/<bundle>` (`factur-x-1.09.2/en16931`), as the fetch script of its
    /// family lays it out under its output directory.
    #[must_use]
    pub fn bundle_path(&self) -> String {
        if self.spec.dir.is_empty() {
            self.release.clone()
        } else {
            format!("{}/{}", self.release, self.spec.dir)
        }
    }

    /// The release directory of its family, relative to the cache's rule-set directory.
    #[cfg(any(test, feature = "fetch"))]
    pub(crate) fn release(&self) -> &str {
        &self.release
    }

    /// The manifest of its bundle (`bundle.toml`), as the fetch script writes it.
    #[must_use]
    pub fn manifest(&self) -> &str {
        &self.manifest
    }
}

static CATALOGUE: OnceLock<Result<Vec<FetchedRuleSet>, String>> = OnceLock::new();

/// Every rule set fetched on demand, in the order of the catalogue: the five Factur-X
/// profiles, XRechnung UBL and CII, the fourteen bundles of the French rules, Peppol BIS
/// Billing 3.0.
///
/// # Errors
///
/// [`Error::Internal`] if the embedded catalogue does not resolve against its pins, which
/// a test rules out.
pub fn fetched_rule_sets() -> Result<&'static [FetchedRuleSet], Error> {
    CATALOGUE
        .get_or_init(resolve_all)
        .as_deref()
        .map_err(|e| Error::internal(format!("the rule-set catalogue: {e}")))
}

/// The fetched rule set named `name`, or the one an alias names for a document of
/// `syntax` (`peppol`; `xrechnung`, which is `xrechnung-ubl` or `xrechnung-cii`).
///
/// # Errors
///
/// As [`fetched_rule_sets`].
pub fn fetched_rule_set(
    name: &str,
    syntax: Option<Syntax>,
) -> Result<Option<&'static FetchedRuleSet>, Error> {
    let all = fetched_rule_sets()?;
    let name = ALIASES
        .iter()
        .find(|(alias, _)| *alias == name)
        .map_or(name, |(_, targets)| {
            targets
                .iter()
                .find(|(s, _)| s.is_some() && *s == syntax)
                .or_else(|| targets.first())
                .map_or(name, |(_, target)| *target)
        });
    Ok(all.iter().find(|r| r.name() == name))
}

/// The aliases [`fetched_rule_set`] takes besides the names of the rule sets: `peppol`
/// (`peppol-bis-billing-ubl`) and `xrechnung` (`xrechnung-ubl` or `xrechnung-cii`, by
/// syntax).
#[must_use]
pub fn fetched_rule_set_aliases() -> Vec<&'static str> {
    ALIASES.iter().map(|(alias, _)| *alias).collect()
}

/// The rule sets that `name` names for fetching: one rule set, an alias, or a family
/// (every rule set of it). Empty for an unknown name.
#[cfg(any(test, feature = "fetch"))]
pub(crate) fn named(name: &str) -> Result<Vec<&'static FetchedRuleSet>, Error> {
    let all = fetched_rule_sets()?;
    if let Some((_, targets)) = ALIASES.iter().find(|(alias, _)| *alias == name) {
        return Ok(all
            .iter()
            .filter(|r| targets.iter().any(|(_, t)| *t == r.name()))
            .collect());
    }
    if FAMILIES.iter().any(|f| f.name == name) {
        return Ok(all.iter().filter(|r| r.family() == name).collect());
    }
    Ok(all.iter().filter(|r| r.name() == name).collect())
}

fn resolve_all() -> Result<Vec<FetchedRuleSet>, String> {
    RULE_SETS.iter().map(resolve).collect()
}

fn resolve(spec: &'static RuleSetSpec) -> Result<FetchedRuleSet, String> {
    let family = spec.family;
    let pins = family.pins;
    let sources = family
        .sources
        .iter()
        .map(|s| {
            Ok(Source {
                what: pins.expand(s.what)?,
                url: pins.expand(s.url)?,
                file: pins.expand(s.file)?,
                algorithm: s.algorithm,
                digest: pins.get(s.digest)?.to_owned(),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(FetchedRuleSet {
        spec,
        version: pins.expand(family.version)?,
        release: pins.expand(family.release)?,
        publisher: pins.expand(family.publisher)?,
        sources,
        manifest: manifest(spec)?,
    })
}

// --- Manifests ----------------------------------------------------------------------

/// The manifest of the bundle of `spec`, as the fetch script of its family writes it.
fn manifest(spec: &RuleSetSpec) -> Result<String, String> {
    match spec.family.kind {
        Kind::FacturX => manifest_facturx(spec),
        Kind::XRechnung => manifest_xrechnung(spec),
        Kind::FrCtc => manifest_fr_ctc(spec),
        Kind::Peppol => manifest_peppol(spec),
    }
}

/// As `scripts/fetch-facturx.sh` writes it.
fn manifest_facturx(spec: &RuleSetSpec) -> Result<String, String> {
    let p = |name: &str| spec.family.pins.get(name);
    let (dir, file, xslt, codedb) = FACTURX_PROFILES
        .iter()
        .find(|(dir, ..)| *dir == spec.dir)
        .ok_or_else(|| format!("{}: no Factur-X profile", spec.name))?;
    let (facturx, mustang) = (p("facturx")?, p("mustang")?);
    Ok(format!(
        "# Factur-X {facturx}, profile {file}, from the Mustang validator {mustang}: the
# rule-set bundle manifest of einvoice-validator, as scripts/fetch-facturx.sh and
# einvoice-validate rules fetch write it (einvoice-validate --bundle DIR). The
# stylesheet reads, with document(), the code list FACTUR-X_{file}_codedb.xml
# next to it, a resource of the bundle. A failed assert without a flag is fatal,
# as the Mustang validator reads the report.
format = 2
name = \"factur-x-{dir}\"
title = \"Factur-X {file}\"
version = \"{facturx}\"

[[stylesheet]]
name = \"FACTUR-X_{file}\"
path = \"FACTUR-X_{file}.xslt\"
sha256 = \"{xslt}\"
unflagged-assert = \"fatal\"

[[resource]]
path = \"FACTUR-X_{file}_codedb.xml\"
sha256 = \"{codedb}\"
",
        xslt = p(xslt)?,
        codedb = p(codedb)?,
    ))
}

/// As `scripts/fetch-xrechnung.sh` writes it.
fn manifest_xrechnung(spec: &RuleSetSpec) -> Result<String, String> {
    let p = |name: &str| spec.family.pins.get(name);
    let upper = spec.dir.to_ascii_uppercase();
    let mut m = format!(
        "# XRechnung {xrechnung} ({upper}), validator configuration {configuration}: the
# rule-set bundle manifest of einvoice-validator, as scripts/fetch-xrechnung.sh
# and einvoice-validate rules fetch write it (einvoice-validate --bundle DIR).
# As the KoSIT validator runs the configuration: a document must match exactly
# one of its scenarios (match), which names the stylesheets that run and gives
# rule ids levels of their own (customLevel: error is fatal, information
# other); a document that matches none is rejected; a failed assert without a
# flag is fatal.
",
        xrechnung = p("xrechnung")?,
        configuration = p("configuration")?,
    );
    if spec.dir == "cii" {
        m.push_str(
            "# The scenarios also validate the documents against the UN/CEFACT CII D16B
# XML schema, which this bundle does not run (unchecked-schema).
",
        );
    }
    let _ = write!(
        m,
        "format = 2\nname = \"xrechnung-{syntax}\"\ntitle = \"XRechnung {upper}\"\nversion = \"{xrechnung}\"\n",
        syntax = spec.dir,
        xrechnung = p("xrechnung")?,
    );
    if spec.dir == "ubl" {
        m.push_str("schema = \"ubl-2.1\"\n");
    } else {
        let _ = writeln!(m, "unchecked-schema = \"{XRECHNUNG_CII_SCHEMA}\"");
    }
    for (name, path, sha, version) in XRECHNUNG_STYLESHEETS {
        if !name.ends_with(&format!("-{upper}")) {
            continue;
        }
        let file = path.rsplit('/').next().unwrap_or(path);
        let _ = write!(
            m,
            "\n[[stylesheet]]\nname = \"{name}\"\npath = \"{file}\"\nsha256 = \"{}\"\nversion = \"{}\"\nunflagged-assert = \"fatal\"\n",
            p(sha)?,
            p(version)?
        );
    }
    for (syntax, name, namespaces, matching, stylesheets, fatal, warning, other) in
        XRECHNUNG_SCENARIOS
    {
        if syntax != spec.dir {
            continue;
        }
        let _ = write!(
            m,
            "\n[[scenario]]\nname = \"{name}\"\nnamespaces = \"{namespaces}\"\nmatch = \"{matching}\"\nstylesheets = \"{stylesheets}\"\n"
        );
        for (key, ids) in [("fatal", fatal), ("warning", warning), ("other", other)] {
            if !ids.is_empty() {
                let _ = writeln!(m, "{key} = \"{ids}\"");
            }
        }
    }
    Ok(m)
}

/// As `scripts/fetch-fr-ctc.sh` writes it.
fn manifest_fr_ctc(spec: &RuleSetSpec) -> Result<String, String> {
    let p = |name: &str| spec.family.pins.get(name);
    let (dir, ubl, title, names) = FR_CTC_BUNDLES
        .iter()
        .find(|(dir, ..)| *dir == spec.dir)
        .ok_or_else(|| format!("{}: no bundle of the French rules", spec.name))?;
    let pack = p("pack")?;
    let mut m = format!(
        "# {title}, FNFE-MPE package {pack}: the rule-set bundle
# manifest of einvoice-validator, as scripts/fetch-fr-ctc.sh and
# einvoice-validate rules fetch write it (einvoice-validate --bundle DIR).
"
    );
    let stylesheets = names
        .iter()
        .map(|name| {
            FR_CTC_STYLESHEETS
                .iter()
                .find(|(n, ..)| n == name)
                .ok_or_else(|| format!("no stylesheet {name}"))
        })
        .collect::<Result<Vec<_>, String>>()?;
    // The code lists, by file name, with the pin of their SHA-256.
    let mut side = Vec::new();
    for (_, path, _, codedb) in &stylesheets {
        if let Some(file) = fr_ctc_side_file(path) {
            let base = file.rsplit('/').next().unwrap_or(&file).to_owned();
            let pin = codedb.ok_or_else(|| format!("{path}: no pin of its code list"))?;
            side.push((base, pin));
        }
    }
    for (base, _) in &side {
        let stem = base.strip_suffix("_codedb.xml").unwrap_or(base);
        let _ = write!(
            m,
            "# The stylesheet {stem}.xslt reads, with document(), the code list\n# {base} next to it, a resource of the bundle.\n"
        );
    }
    for (name, ..) in &stylesheets {
        if fr_ctc_factur_x(name) {
            let _ = write!(
                m,
                "# A failed assert of {name} without a flag is fatal, as the\n# Mustang validator reads the Factur-X reports.\n"
            );
        }
    }
    let _ = write!(
        m,
        "format = 2\nname = \"fr-ctc-{dir}\"\ntitle = \"{title}\"\nversion = \"{pack}\"\n"
    );
    if *ubl {
        m.push_str("schema = \"ubl-2.1\"\n");
    }
    for (name, _, sha, _) in &stylesheets {
        let _ = write!(
            m,
            "\n[[stylesheet]]\nname = \"{name}\"\npath = \"{name}.xslt\"\nsha256 = \"{}\"\n",
            p(sha)?
        );
        if fr_ctc_factur_x(name) {
            m.push_str("unflagged-assert = \"fatal\"\n");
        }
    }
    for (base, pin) in &side {
        let _ = write!(
            m,
            "\n[[resource]]\npath = \"{base}\"\nsha256 = \"{}\"\n",
            p(pin)?
        );
    }
    Ok(m)
}

/// As `scripts/fetch-peppol.sh` writes it.
fn manifest_peppol(spec: &RuleSetSpec) -> Result<String, String> {
    let p = |name: &str| spec.family.pins.get(name);
    Ok(format!(
        "# Peppol BIS Billing {tag} (UBL): the rule-set bundle manifest of
# einvoice-validator, as scripts/fetch-peppol.sh and einvoice-validate rules
# fetch write it (einvoice-validate --bundle DIR).
format = 1
name = \"peppol-bis-billing-ubl\"
title = \"Peppol BIS Billing 3.0 UBL\"
version = \"{tag}\"
schema = \"ubl-2.1\"

[[stylesheet]]
name = \"CEN-EN16931-UBL\"
path = \"xslt/CEN-EN16931-UBL.xslt\"
sha256 = \"{cen_sha}\"
version = \"{cen}\"

[[stylesheet]]
name = \"PEPPOL-EN16931-UBL\"
path = \"xslt/PEPPOL-EN16931-UBL.xslt\"
sha256 = \"{peppol_sha}\"
",
        tag = p("tag")?,
        cen_sha = p("cen_xslt_sha256")?,
        cen = p("cen")?,
        peppol_sha = p("peppol_xslt_sha256")?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RuleSet;

    #[test]
    fn every_rule_set_resolves_and_its_manifest_parses() {
        let all = fetched_rule_sets().unwrap();
        assert_eq!(all.len(), RULE_SETS.len());
        for r in all {
            let parsed =
                RuleSet::parse(r.manifest()).unwrap_or_else(|e| panic!("{}: {e}", r.name()));
            assert_eq!(parsed.name(), r.name());
            assert_eq!(parsed.title(), r.title());
            assert_eq!(parsed.version(), r.version());
            assert_ne!(r.sources(), []);
            for s in r.sources() {
                assert!(s.url().starts_with("https://"), "{}", s.url());
                let length = match s.algorithm_kind() {
                    Algorithm::Sha256 => 64,
                    Algorithm::Sha512 => 128,
                };
                assert_eq!(s.digest().len(), length, "{}", s.url());
                assert!(!s.file().contains('/'), "{}", s.file());
            }
            assert_eq!(
                parsed.schema().is_some(),
                r.syntax() == Some(Syntax::Ubl),
                "{}",
                r.name()
            );
        }
        // Names are unique, and no name is an alias or a family.
        for (i, r) in all.iter().enumerate() {
            assert!(
                all[..i].iter().all(|o| o.name() != r.name()),
                "{}",
                r.name()
            );
            assert!(ALIASES.iter().all(|(a, _)| *a != r.name()));
            assert!(FAMILIES.iter().all(|f| f.name != r.name()));
            assert!(crate::BUILTIN_RULE_SETS.iter().all(|b| *b != r.name()));
        }
    }

    /// Where an unflagged failed assert is fatal, and where scenarios give rule ids their
    /// levels: as the reference validator of each family reads the reports.
    #[test]
    fn severities_of_the_reference_validators() {
        use crate::Severity;
        for r in fetched_rule_sets().unwrap() {
            let parsed = RuleSet::parse(r.manifest()).unwrap();
            for s in parsed.stylesheets() {
                // Mustang (Factur-X, and the Factur-X profiles of the French package)
                // and KoSIT (XRechnung) reject a document for an unflagged failed
                // assert; Peppol and the rest of the French rules flag every assert.
                let fatal = r.family() == "factur-x"
                    || r.family() == "xrechnung"
                    || s.name().starts_with("FACTUR-X_");
                let expected = if fatal {
                    Severity::Fatal
                } else {
                    Severity::Other
                };
                assert_eq!(s.unflagged_assert(), expected, "{} {}", r.name(), s.name());
            }
            let scenarios: Vec<(&str, Option<&[String]>)> = parsed
                .scenarios()
                .iter()
                .map(|s| (s.name(), s.stylesheets()))
                .collect();
            match r.name() {
                "xrechnung-ubl" | "xrechnung-cii" => {
                    // Every scenario of the configuration that runs its stylesheets, each
                    // with a match expression: the bundle selects by scenario.
                    assert_eq!(
                        scenarios.len(),
                        if r.name() == "xrechnung-ubl" { 7 } else { 4 }
                    );
                    assert!(parsed.selects_by_scenario());
                }
                _ => {
                    assert_eq!(scenarios, [], "{}", r.name());
                    assert!(!parsed.selects_by_scenario());
                }
            }
            let gap = (r.name() == "xrechnung-cii").then_some(XRECHNUNG_CII_SCHEMA);
            assert_eq!(parsed.unchecked_schema(), gap, "{}", r.name());
        }
        let ubl = RuleSet::parse(
            fetched_rule_set("xrechnung-ubl", None)
                .unwrap()
                .unwrap()
                .manifest(),
        )
        .unwrap();
        let standard = &ubl.scenarios()[0];
        assert_eq!(standard.name(), "EN16931 XRechnung (UBL Invoice)");
        assert_eq!(standard.level("UBL-CR-646"), Some(Severity::Fatal));
        assert_eq!(standard.level("BR-CL-23"), Some(Severity::Warning));
        assert_eq!(
            ubl.scenarios()[1].level("UBL-CR-646"),
            Some(Severity::Other)
        );
        let names = |s: &crate::Scenario| s.stylesheets().map(<[String]>::to_vec);
        assert_eq!(
            names(standard),
            Some(vec!["EN16931-UBL".to_owned(), "XRechnung-UBL".to_owned()])
        );
        // The scenario of the CEN rules alone runs the CEN stylesheet alone.
        let cen = &ubl.scenarios()[5];
        assert_eq!(cen.name(), "EN16931 (UBL Invoice)");
        assert_eq!(names(cen), Some(vec!["EN16931-UBL".to_owned()]));
        assert_eq!(cen.levels(), []);
    }

    #[test]
    fn aliases_and_families() {
        let name = |n: &str, s| fetched_rule_set(n, s).unwrap().map(FetchedRuleSet::name);
        assert_eq!(name("peppol", None), Some("peppol-bis-billing-ubl"));
        assert_eq!(name("xrechnung", Some(Syntax::Ubl)), Some("xrechnung-ubl"));
        assert_eq!(name("xrechnung", Some(Syntax::Cii)), Some("xrechnung-cii"));
        assert_eq!(name("xrechnung", None), Some("xrechnung-ubl"));
        assert_eq!(name("factur-x-basic", None), Some("factur-x-basic"));
        assert_eq!(name("nope", None), None);
        assert_eq!(named("factur-x").unwrap().len(), 5);
        assert_eq!(named("fr-ctc").unwrap().len(), 14);
        assert_eq!(named("xrechnung").unwrap().len(), 2);
        assert_eq!(named("peppol").unwrap().len(), 1);
        assert_eq!(named("xrechnung-cii").unwrap().len(), 1);
        assert!(named("cen-ubl").unwrap().is_empty());
    }

    #[test]
    fn side_files_of_the_french_package() {
        assert_eq!(
            fr_ctc_side_file("a/b/FACTUR-X_EN16931.xslt").as_deref(),
            Some("a/b/FACTUR-X_EN16931_codedb.xml")
        );
        assert_eq!(
            fr_ctc_side_file("CII/EN16931/2xslt/EN16931-CII-validation.xslt"),
            None
        );
        // Each stylesheet with a code list has its pin, and only those.
        for (name, path, _, codedb) in FR_CTC_STYLESHEETS {
            assert_eq!(fr_ctc_side_file(path).is_some(), codedb.is_some(), "{name}");
        }
    }
}
