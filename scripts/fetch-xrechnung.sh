#!/usr/bin/env bash
# Builds the XRechnung 3.0.2 inputs of the tests outside the tracked tree: the
# compiled stylesheets of the validator configuration published by KoSIT
# (the EN 16931 rules and the XRechnung rules, for UBL and CII), its test suite,
# the unit tests of the XRechnung Schematron expanded by XML Mutate, and the
# rule-set bundles. The XRechnung Schematron includes rules derived from
# Peppol BIS Billing 3.0, which publishes no licence, so, as for Peppol, none
# of these files, nor anything derived from them, is part of this
# repository; every input is checked against the checksum below. Everything
# is downloaded from KoSIT's GitLab (projekte.kosit.org).
#
#   scripts/fetch-xrechnung.sh            # into target/xrechnung/
#   scripts/fetch-xrechnung.sh DIR        # into DIR/
#
# Steps, into DIR/xrechnung-3.0.2/ (always afresh), downloads cached in
# target/downloads/ (shared with the other fetch scripts; a cached file is
# checked, never trusted):
#  1. The validator configuration 2026-08-31 (Maven package
#     de.xeinkauf:xrechnung-3.0.2-validator-configuration), extracted into
#     source/configuration/. The four stylesheets its scenarios run must have
#     the SHA-256 below: resources/ubl/2.1/xsl/EN16931-UBL-validation.xsl and
#     resources/cii/16b/xsl/EN16931-CII-validation.xsl (the CEN rules of the
#     release its changelog names, compiled by KoSIT), and
#     resources/xrechnung/3.0.2/xsl/XRechnung-{UBL,CII}-validation.xsl (the
#     XRechnung Schematron, whose title states its release).
#  2. The test suite 2026-08-31 (de.xeinkauf:xrechnung-3.0.2-testsuite): its
#     instances, documents that the configuration accepts, sorted by syntax
#     into source/testsuite/<ubl|cii>/<directory>/ (*_ubl.xml, *_uncefact.xml;
#     any other file is an error).
#  3. The source archive of the XRechnung Schematron release (tag v2.6.0, its
#     SHA-256 and the commit it records checked): its unit tests,
#     test/instances/{ubl-inv,ubl-cn,cii}/, into source/schematron-tests/.
#  4. XML Mutate 0.6 (de.kosit:xml-mutate, the tool the XRechnung Schematron
#     build tests its rules with) expands each test into its mutants, each
#     with the expectation of its active `<?xmute?>` instruction:
#     source/xmute/ubl/ and source/xmute/cii/. XML Mutate numbers the mutants
#     in the order it reads the tests, so it is given them one by one, set by
#     set, each sorted by name. The checksum of each set (the SHA-256 of its
#     `sha256sum` listing, sorted by name) is pinned below.
#  5. The CEN EN 16931 examples vendored in this repository (CII examples, UBL
#     examples and test files, checked by `cargo xtask manifest`), copied to
#     source/cen-cii-examples/, source/cen-ubl-examples/ and
#     source/cen-ubl-testfiles/: the base documents of the mutation
#     catalogues.
#  6. The rule-set bundles of einvoice-validator, ubl/ and cii/, each with its
#     bundle.toml and the two stylesheets (EN 16931, then XRechnung), and the
#     scenarios of the configuration for its syntax, as the KoSIT validator
#     runs them: the match expression of each, the stylesheets it runs (both,
#     or the EN 16931 ones alone), the levels it gives the rule ids it lists; a
#     document that matches no scenario is rejected; a failed assert without a
#     flag is fatal. The CII scenarios also validate against the UN/CEFACT CII
#     D16B XML schema, which the CII bundle does not run (unchecked-schema):
#     `einvoice-validate --bundle DIR/xrechnung-3.0.2/ubl FILE`.
#     `einvoice-validate rules fetch` builds the same bundles, byte for byte,
#     from the same pins (crates/einvoice-validator/pins/, a copy of the block
#     below, and crates/einvoice-validator/src/rulesets/).
#
# Needs bash, curl, tar, gzip, unzip, git and Java (CI uses Java 21).
# Exit status: 0 done; 75 a download failed (no network); 1 anything else,
# including a checksum mismatch: a file that does not match is never used.
#
# The pins below are read by xtask/src/fetched.rs: keep one `name=value` per
# line, as they are. Their versions are recorded in upstream.toml.
set -euo pipefail

# --- pins -------------------------------------------------------------------
xrechnung=3.0.2
schematron=2.6.0
cen=1.3.16
configuration=2026-08-31
configuration_sha256=2530cd107c414511c5d0462ec10f886910395abfca820db82e83d70bf01221a8
testsuite=2026-08-31
testsuite_sha256=7af6425617fffde28e2f479f332224350c270570c23a11e2d0d4ca8621a14257
schematron_archive_sha256=998976cbd42abba06fe897a915010ae4513cb1354b8a4447cdc89835c4d75b9b
schematron_commit=217c6cbf8ac9a99d780cefb4f044b2c2778d4744
xmute=0.6
xmute_sha256=b7cd05eb21c47e15dbb1d3ef38487aa65539b335e6d6c355015bbaf6ab573bec
xmute_ubl_sha256=93bc09b0f2fe35129490bc2676c19d1c09355fcc705781e91b1c5402e3f4ca61
xmute_cii_sha256=3175760746792080f6952e6303bd5ac2fec2183dfe906174e454254d68de51ce
cen_ubl_sha256=759f2d2e830c7fd1d5a6e8b4f61e5dbafc7a7805862c4ae1c749eeac6adde698
cen_cii_sha256=0911e927f13f9ae2cdc7f973643f27bf0ad3c5da02483c04a5b8a47eb4822199
xrechnung_ubl_sha256=e32d831668facbbd5136cc0119bf1dd6f92ac20738aa3b3f021586f5a305c738
xrechnung_cii_sha256=ec54f662803a87f527ec606d1320bc404f1657a848af6a76f37f32905e4aac40
# --- end of pins ------------------------------------------------------------

registry=https://projekte.kosit.org/api/v4/projects/356/packages/maven
configuration_url=$registry/de/xeinkauf/xrechnung-$xrechnung-validator-configuration/$configuration/xrechnung-$xrechnung-validator-configuration-$configuration.zip
testsuite_url=$registry/de/xeinkauf/xrechnung-$xrechnung-testsuite/$testsuite/xrechnung-$xrechnung-testsuite-$testsuite.zip
xmute_url=$registry/de/kosit/xml-mutate/$xmute/xml-mutate-$xmute.jar
schematron_url=https://projekte.kosit.org/xrechnung/xrechnung-schematron/-/archive/v$schematron/xrechnung-schematron-v$schematron.tar.gz

# Stylesheet name in the bundles, path in the configuration, SHA-256.
stylesheets=(
    EN16931-UBL resources/ubl/2.1/xsl/EN16931-UBL-validation.xsl "$cen_ubl_sha256"
    XRechnung-UBL "resources/xrechnung/$xrechnung/xsl/XRechnung-UBL-validation.xsl" "$xrechnung_ubl_sha256"
    EN16931-CII resources/cii/16b/xsl/EN16931-CII-validation.xsl "$cen_cii_sha256"
    XRechnung-CII "resources/xrechnung/$xrechnung/xsl/XRechnung-CII-validation.xsl" "$xrechnung_cii_sha256"
)

# The namespaces of the scenarios of the configuration, as their namespace
# elements bind them (rep: the namespace of its validation reports), by the
# document kind of the scenarios below.
namespaces_invoice="cbc=urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2 invoice=urn:oasis:names:specification:ubl:schema:xsd:Invoice-2 rep=http://www.xoev.de/de/validator/varl/1"
namespaces_creditnote="cbc=urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2 creditnote=urn:oasis:names:specification:ubl:schema:xsd:CreditNote-2 rep=http://www.xoev.de/de/validator/varl/1"
namespaces_cii="rsm=urn:un:unece:uncefact:data:standard:CrossIndustryInvoice:100 ram=urn:un:unece:uncefact:data:standard:ReusableAggregateBusinessInformationEntity:100 rep=http://www.xoev.de/de/validator/varl/1"
# The XML schema the CII scenarios validate against, which the CII bundle does
# not run (its unchecked-schema).
cii_schema="UN/CEFACT CII D16B (CrossIndustryInvoice_100pD16B.xsd)"

# The scenarios of the configuration (source/configuration/scenarios.xml), in
# its order: syntax of the bundle, name, document kind (its namespaces above),
# its match expression as written (a trailing space included), the bundle
# names of the stylesheets it runs, then the rule ids of the levels error,
# warning and information. They become the [[scenario]] tables of the bundles
# (match, stylesheets, fatal, warning, other); `cargo xtask xrechnung` checks
# them against scenarios.xml.
scenarios=(
    "ubl|EN16931 XRechnung (UBL Invoice)|invoice|exists(/invoice:Invoice/cbc:CustomizationID[ . = 'urn:cen.eu:en16931:2017#compliant#urn:xeinkauf.de:kosit:xrechnung_3.0']) |EN16931-UBL XRechnung-UBL|UBL-CR-646|BR-CL-23 BR-CL-21|"
    "ubl|EN16931 XRechnung Extension (UBL Invoice)|invoice|exists(/invoice:Invoice/cbc:CustomizationID[ . = 'urn:cen.eu:en16931:2017#compliant#urn:xeinkauf.de:kosit:xrechnung_3.0#conformant#urn:xeinkauf.de:kosit:extension:xrechnung_3.0'])|EN16931-UBL XRechnung-UBL||BR-CL-23|BR-CL-21 BR-CL-24 BR-CL-10 BR-CL-11 BR-CL-25 BR-CL-26 BR-CO-16 UBL-CR-470 UBL-CR-646"
    "ubl|EN16931 XRechnung CVD (UBL Invoice)|invoice|exists(/invoice:Invoice/cbc:CustomizationID[ . = 'urn:cen.eu:en16931:2017#compliant#urn:xeinkauf.de:kosit:xrechnung_3.0#compliant#urn:xeinkauf.de:kosit:xrechnung:cvd_0.9']) |EN16931-UBL XRechnung-UBL|UBL-CR-646|BR-CL-23 BR-CL-21|BR-CL-13"
    "ubl|EN16931 XRechnung (UBL CreditNote)|creditnote|exists(/creditnote:CreditNote[ cbc:CustomizationID/text() = 'urn:cen.eu:en16931:2017#compliant#urn:xeinkauf.de:kosit:xrechnung_3.0'])|EN16931-UBL XRechnung-UBL||BR-CL-23 BR-CL-21|"
    "ubl|EN16931 XRechnung CVD (UBL CreditNote)|creditnote|exists(/creditnote:CreditNote[ cbc:CustomizationID/text() = 'urn:cen.eu:en16931:2017#compliant#urn:xeinkauf.de:kosit:xrechnung_3.0#compliant#urn:xeinkauf.de:kosit:xrechnung:cvd_0.9'])|EN16931-UBL XRechnung-UBL|UBL-CR-646|BR-CL-23 BR-CL-21|BR-CL-13"
    "cii|EN16931 XRechnung (CII)|cii|exists(/rsm:CrossIndustryInvoice[rsm:ExchangedDocumentContext/ram:GuidelineSpecifiedDocumentContextParameter/ram:ID/text() = 'urn:cen.eu:en16931:2017#compliant#urn:xeinkauf.de:kosit:xrechnung_3.0'])|EN16931-CII XRechnung-CII|CII-SR-452 CII-SR-453 CII-SR-454 CII-SR-465 CII-SR-466|BR-CL-23 BR-CL-21|CII-SR-475 CII-SR-476"
    "cii|EN16931 XRechnung Extension (CII)|cii|exists(/rsm:CrossIndustryInvoice[rsm:ExchangedDocumentContext/ram:GuidelineSpecifiedDocumentContextParameter/ram:ID/text() = 'urn:cen.eu:en16931:2017#compliant#urn:xeinkauf.de:kosit:xrechnung_3.0#conformant#urn:xeinkauf.de:kosit:extension:xrechnung_3.0'])|EN16931-CII XRechnung-CII|CII-SR-452 CII-SR-453 CII-SR-454 CII-SR-465 CII-SR-466|BR-CL-23|BR-CL-21 BR-CL-11 BR-CL-10 BR-CL-24 BR-CL-25 BR-CL-26 CII-SR-475 CII-SR-476"
    "cii|EN16931 XRechnung CVD (CII)|cii|exists(/rsm:CrossIndustryInvoice[rsm:ExchangedDocumentContext/ram:GuidelineSpecifiedDocumentContextParameter/ram:ID/text() = 'urn:cen.eu:en16931:2017#compliant#urn:xeinkauf.de:kosit:xrechnung_3.0#compliant#urn:xeinkauf.de:kosit:xrechnung:cvd_0.9'])|EN16931-CII XRechnung-CII|CII-SR-452 CII-SR-453 CII-SR-454 CII-SR-465 CII-SR-466|BR-CL-23 BR-CL-21|BR-CL-13 CII-SR-475 CII-SR-476"
    "ubl|EN16931 (UBL Invoice)|invoice|exists(/invoice:Invoice[ cbc:CustomizationID/text() = 'urn:cen.eu:en16931:2017' ])|EN16931-UBL|||"
    "ubl|EN16931 (UBL CreditNote)|creditnote|exists(/creditnote:CreditNote[cbc:CustomizationID/text() = 'urn:cen.eu:en16931:2017'])|EN16931-UBL|||"
    "cii|EN16931 (CII)|cii|exists(/rsm:CrossIndustryInvoice[ rsm:ExchangedDocumentContext/ram:GuidelineSpecifiedDocumentContextParameter/ram:ID/text() = 'urn:cen.eu:en16931:2017'])|EN16931-CII|||"
)

cd "$(git rev-parse --show-toplevel)"
root=$PWD
dir=${1:-target/xrechnung}
mkdir -p "$dir" target/downloads
dir=$(cd "$dir" && pwd)
downloads=$root/target/downloads
release=$dir/xrechnung-$xrechnung

say() { echo "fetch-xrechnung: $*"; }
fail() {
    echo "fetch-xrechnung: $*" >&2
    exit 1
}

hash_of() { # FILE
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d' ' -f1
    else
        shasum -a 256 "$1" | cut -d' ' -f1
    fi
}

for tool in curl tar gzip unzip git; do
    command -v "$tool" >/dev/null 2>&1 || fail "$tool is not installed"
done
command -v "${JAVA:-java}" >/dev/null 2>&1 || fail "java is not installed: XML Mutate cannot run"

tmp=$(mktemp -d "$dir/.tmp.XXXXXX")
trap 'rm -rf "$tmp"' EXIT

# fetch URL FILE SHA256: FILE, downloaded if missing; its SHA-256 must be
# SHA256, whether it was just downloaded or already there.
fetch() {
    local url=$1 file=$2 digest=$3 actual
    if [ ! -e "$file" ]; then
        # Every failure is retried (an HTTP 504 or a reset connection
        # included), five times, five seconds apart, for two minutes at most.
        if ! curl --proto '=https' --tlsv1.2 -fsSL --retry 5 --retry-all-errors --retry-delay 5 \
            --retry-max-time 120 -o "$tmp/download" "$url"; then
            echo "fetch-xrechnung: cannot download $url" >&2
            exit 75
        fi
        actual=$(hash_of "$tmp/download")
        [ "$actual" = "$digest" ] ||
            fail "download from $url has SHA-256 $actual, expected $digest"
        mv "$tmp/download" "$file"
        say "$file downloaded, SHA-256 verified"
        return 0
    fi
    actual=$(hash_of "$file")
    [ "$actual" = "$digest" ] ||
        fail "$file has SHA-256 $actual, expected $digest; delete it and run again"
    say "$file already present, SHA-256 verified"
}

# listing_hash DIR: the SHA-256 of the `sha256sum` listing of the files of DIR,
# sorted by name (LC_ALL=C).
listing_hash() {
    (cd "$1" && find . -type f -print0 | LC_ALL=C sort -z | xargs -0 sha256sum) >"$tmp/listing"
    hash_of "$tmp/listing"
}

# --- 1. the validator configuration -------------------------------------------

configuration_zip=$downloads/xrechnung-$xrechnung-validator-configuration-$configuration.zip
fetch "$configuration_url" "$configuration_zip" "$configuration_sha256"
config=$tmp/release/source/configuration
mkdir -p "$config"
unzip -q "$configuration_zip" -d "$config"
for ((i = 0; i < ${#stylesheets[@]}; i += 3)); do
    path=$config/${stylesheets[i + 1]}
    [ -f "$path" ] || fail "$configuration_zip has no ${stylesheets[i + 1]}"
    actual=$(hash_of "$path")
    [ "$actual" = "${stylesheets[i + 2]}" ] ||
        fail "${stylesheets[i + 1]} has SHA-256 $actual, expected ${stylesheets[i + 2]}"
done
# "Schematron Version 2.6.0 - XRechnung 3.0.2 compatible - UBL - ..."
for syntax in UBL CII; do
    [ "$(grep -c "title=\"Schematron Version $schematron - XRechnung $xrechnung compatible" \
        "$config/resources/xrechnung/$xrechnung/xsl/XRechnung-$syntax-validation.xsl")" -gt 0 ] ||
        fail "XRechnung-$syntax-validation.xsl is not XRechnung Schematron $schematron"
done
# "## 2026-08-31 ... * Using [CEN Schematron Rules 1.3.16](...)"
[ "$(sed -n "/^## $configuration\$/,/^## /p" "$config/CHANGELOG.md" | grep -c "CEN Schematron Rules $cen\]")" -gt 0 ] ||
    fail "the changelog of the configuration $configuration does not name CEN Schematron Rules $cen"
say "configuration $configuration: the 4 stylesheets verified (CEN $cen, XRechnung Schematron $schematron)"

# --- 2. the test suite ---------------------------------------------------------

testsuite_zip=$downloads/xrechnung-$xrechnung-testsuite-$testsuite.zip
fetch "$testsuite_url" "$testsuite_zip" "$testsuite_sha256"
mkdir -p "$tmp/unzip-testsuite"
unzip -q "$testsuite_zip" -d "$tmp/unzip-testsuite"
count=0
while IFS= read -r -d '' file; do
    relative=${file#"$tmp/unzip-testsuite/instances/"}
    case ${file##*/} in
    *_ubl.xml) syntax=ubl ;;
    *_uncefact.xml) syntax=cii ;;
    *) fail "$testsuite_zip: instances/$relative: neither UBL nor CII by its name" ;;
    esac
    mkdir -p "$tmp/release/source/testsuite/$syntax/$(dirname "$relative")"
    cp "$file" "$tmp/release/source/testsuite/$syntax/$relative"
    count=$((count + 1))
done < <(find "$tmp/unzip-testsuite/instances" -type f -print0)
[ "$count" -gt 0 ] || fail "$testsuite_zip: no instance"
say "test suite $testsuite: $count instances"

# --- 3. the unit tests of the XRechnung Schematron ------------------------------

archive=$downloads/xrechnung-schematron-v$schematron.tar.gz
fetch "$schematron_url" "$archive" "$schematron_archive_sha256"
# Decompressed to a file, not piped: git reads the first header only, and a
# writer still feeding a pipe it has closed gets SIGPIPE, or EPIPE where SIGPIPE
# is ignored (GitHub's runners), "gzip: stdout: Broken pipe".
gzip -dc "$archive" >"$tmp/schematron.tar"
recorded=$(git get-tar-commit-id <"$tmp/schematron.tar") || fail "$archive records no commit id"
[ "$recorded" = "$schematron_commit" ] || fail "$archive is commit $recorded, expected $schematron_commit"
mkdir -p "$tmp/unzip-schematron"
tar -xf "$tmp/schematron.tar" -C "$tmp/unzip-schematron" --strip-components=1 --no-same-owner --no-same-permissions
tests=$tmp/release/source/schematron-tests
mkdir -p "$tests"
for set in ubl-inv ubl-cn cii; do
    [ -d "$tmp/unzip-schematron/test/instances/$set" ] || fail "$archive has no test/instances/$set/"
    mv "$tmp/unzip-schematron/test/instances/$set" "$tests/$set"
done
say "XRechnung Schematron $schematron (commit $schematron_commit): unit tests extracted"

# --- 4. XML Mutate ----------------------------------------------------------------

xmute_jar=$downloads/xml-mutate-$xmute.jar
fetch "$xmute_url" "$xmute_jar" "$xmute_sha256"
xsl=$config/resources/xrechnung/$xrechnung/xsl
# mutate SYNTAX PIN SETS...: the mutants of the test sets SETS into
# source/xmute/SYNTAX/, whose listing must have the SHA-256 PIN.
mutate() {
    local syntax=$1 expected=$2 out=$tmp/release/source/xmute/$1 actual status=0 args=()
    shift 2
    # The mutants of an earlier run are reused when their listing matches.
    if [ -d "$release/source/xmute/$syntax" ] &&
        [ "$(listing_hash "$release/source/xmute/$syntax")" = "$expected" ]; then
        mkdir -p "$(dirname "$out")"
        cp -R "$release/source/xmute/$syntax" "$out"
        say "XML Mutate $xmute: the $syntax mutants of the previous run reused, listing SHA-256 verified"
        return 0
    fi
    # XML Mutate numbers the mutants of a run with one counter, document after
    # document, in the order of its arguments, and writes the number into the
    # name of each mutant and into its active comment. Given a directory, it
    # takes the order the file system lists it in (ext4 orders entries by a
    # hash with a seed of its own per file system), so the files are given one
    # by one: set by set, each sorted by name (LC_ALL=C).
    for set in "$@"; do
        while IFS= read -r -d '' file; do
            args+=("$file")
        done < <(find "$tests/$set" -type f -name '*.xml' -print0 | LC_ALL=C sort -z)
    done
    [ "${#args[@]}" -gt 0 ] || fail "no unit test in $*"
    mkdir -p "$out"
    "${JAVA:-java}" -jar "$xmute_jar" --mode MUTATE --target "$out" \
        --schematron "xrubl=$xsl/XRechnung-UBL-validation.xsl" \
        --schematron "xrcii=$xsl/XRechnung-CII-validation.xsl" \
        "${args[@]}" >"$tmp/xmute-$syntax.log" 2>&1 || status=$?
    if [ "$status" -ne 0 ] || ! grep -q '^Result: SUCCESSFUL' "$tmp/xmute-$syntax.log"; then
        cat "$tmp/xmute-$syntax.log" >&2
        fail "XML Mutate failed on the $syntax tests (status $status)"
    fi
    actual=$(listing_hash "$out")
    [ "$actual" = "$expected" ] ||
        fail "the $syntax mutants have listing SHA-256 $actual, expected $expected"
    say "XML Mutate $xmute: $(find "$out" -type f | wc -l | tr -d ' ') $syntax mutants, listing SHA-256 verified"
}
mutate ubl "$xmute_ubl_sha256" ubl-inv ubl-cn
mutate cii "$xmute_cii_sha256" cii

# --- 5. the CEN examples (mutation bases) -----------------------------------------

cp -R "$root/artefacts/cen-en16931-cii-1.3.17/cii/examples" "$tmp/release/source/cen-cii-examples"
cp -R "$root/artefacts/cen-en16931-ubl-1.3.17/ubl/examples" "$tmp/release/source/cen-ubl-examples"
cp -R "$root/artefacts/cen-en16931-ubl-1.3.17/test/testfiles" "$tmp/release/source/cen-ubl-testfiles"

# --- 6. the bundles -----------------------------------------------------------------

for syntax in ubl cii; do
    upper=$(echo "$syntax" | tr '[:lower:]' '[:upper:]')
    bundle=$tmp/release/$syntax
    mkdir -p "$bundle"
    {
        echo "# XRechnung $xrechnung ($upper), validator configuration $configuration: the"
        echo "# rule-set bundle manifest of einvoice-validator, as scripts/fetch-xrechnung.sh"
        echo "# and einvoice-validate rules fetch write it (einvoice-validate --bundle DIR)."
        echo "# As the KoSIT validator runs the configuration: a document must match exactly"
        echo "# one of its scenarios (match), which names the stylesheets that run and gives"
        echo "# rule ids levels of their own (customLevel: error is fatal, information"
        echo "# other); a document that matches none is rejected; a failed assert without a"
        echo "# flag is fatal."
        if [ "$syntax" = cii ]; then
            echo "# The scenarios also validate the documents against the UN/CEFACT CII D16B"
            echo "# XML schema, which this bundle does not run (unchecked-schema)."
        fi
        echo "format = 2"
        echo "name = \"xrechnung-$syntax\""
        echo "title = \"XRechnung $upper\""
        echo "version = \"$xrechnung\""
        if [ "$syntax" = cii ]; then
            echo "unchecked-schema = \"$cii_schema\""
        else
            echo "schema = \"ubl-2.1\""
        fi
        for ((i = 0; i < ${#stylesheets[@]}; i += 3)); do
            case ${stylesheets[i]} in *-"$upper") ;; *) continue ;; esac
            file=${stylesheets[i + 1]##*/}
            cp "$config/${stylesheets[i + 1]}" "$bundle/$file"
            echo
            echo "[[stylesheet]]"
            echo "name = \"${stylesheets[i]}\""
            echo "path = \"$file\""
            echo "sha256 = \"${stylesheets[i + 2]}\""
            case ${stylesheets[i]} in
            EN16931-*) echo "version = \"$cen\"" ;;
            *) echo "version = \"$schematron\"" ;;
            esac
            echo "unflagged-assert = \"fatal\""
        done
        for entry in "${scenarios[@]}"; do
            IFS='|' read -r scenario_syntax name kind match names fatal warning other <<<"$entry"
            [ "$scenario_syntax" = "$syntax" ] || continue
            case $kind in
            invoice) namespaces=$namespaces_invoice ;;
            creditnote) namespaces=$namespaces_creditnote ;;
            *) namespaces=$namespaces_cii ;;
            esac
            echo
            echo "[[scenario]]"
            echo "name = \"$name\""
            echo "namespaces = \"$namespaces\""
            echo "match = \"$match\""
            echo "stylesheets = \"$names\""
            [ -z "$fatal" ] || echo "fatal = \"$fatal\""
            [ -z "$warning" ] || echo "warning = \"$warning\""
            [ -z "$other" ] || echo "other = \"$other\""
        done
    } >"$bundle/bundle.toml"
done

rm -rf "$release"
mv "$tmp/release" "$release"
say "$release written: source/, and the bundles ubl/ and cii/"
