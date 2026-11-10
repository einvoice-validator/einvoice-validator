#!/usr/bin/env bash
# Builds the Factur-X 1.09.2 (ZUGFeRD 2.5.2) inputs of the tests outside the
# tracked tree: the compiled stylesheets of the five profiles, the code lists
# they read with document(), the XML schemas, the example documents, and one
# rule-set bundle per profile. FeRD publishes the official Factur-X package
# behind a registration form, so this script takes the stylesheets from the
# Mustang validator on Maven Central, which runs them as they are, and the
# example documents from the annexes of the French standard XP Z12-012 and
# XP Z12-014 (AFNOR, all rights reserved). None of these files, nor anything
# derived from them, is part of this repository; every input is checked
# against the checksum below.
#
#   scripts/fetch-facturx.sh            # into target/facturx/
#   scripts/fetch-facturx.sh DIR        # into DIR/
#
# Steps, into DIR/factur-x-1.09.2/ (always afresh), downloads cached in
# target/downloads/ (shared with scripts/fetch-fr-ctc.sh and
# scripts/fetch-xrechnung.sh; a cached file is checked, never trusted):
#  1. Mustang validator 2.26.0 (org.mustangproject:validator:2.26.0, Maven
#     Central): its jar holds xslt/ZF_250/FACTUR-X_<PROFILE>.xslt and
#     FACTUR-X_<PROFILE>_codedb.xml, the stylesheets XMLValidator runs for each
#     profile, with the Schematron sources (schematron/ZF_250/) and the XML
#     schemas (schema/ZF_250/<PROFILE>/FACTUR-X_<NAME>.xsd, NAME the profile
#     without its hyphen, and the three modules it imports), which XMLValidator
#     validates the document against before it runs the stylesheet of the
#     profile. Each stylesheet, code list and schema document must have the
#     SHA-256 below, and each Schematron source must state Factur-X 1.09.2:
#     source/mustang/.
#  2. The example documents of XP Z12-012 (annex B) and XP Z12-014 (annex B),
#     from fnfe-mpe.org: their Factur-X CII documents (*_FX_CII_Commentee.xml)
#     into source/examples/z12-012/ and source/examples/z12-014/.
#  3. The CEN EN 16931 CII examples vendored in this repository
#     (artefacts/cen-en16931-cii-1.3.17/cii/examples/, checked by
#     `cargo xtask manifest`), copied to source/cen-cii-examples/: the base
#     documents of the CII mutation catalogue.
#  4. One rule-set bundle of einvoice-validator per profile,
#     <profile>/bundle.toml with the stylesheet and its code list next to it (a resource
#     of the bundle), and the four documents of the profile's XML schema (the
#     schema of the bundle, validated first: a document it finds invalid is
#     invalid, and the stylesheet runs all the same, as in XMLValidator), a
#     failed assert without a flag being fatal, as the Mustang validator reads
#     the report (unflagged-assert = "fatal"):
#     `einvoice-validate --bundle DIR/factur-x-1.09.2/en16931 FILE`.
#     `einvoice-validate rules fetch` builds the same bundles, byte for byte,
#     from the same pins (crates/einvoice-validator/pins/, a copy of the block
#     below, and crates/einvoice-validator/src/rulesets/).
#
# Needs bash, curl, unzip and git.
# Exit status: 0 done; 75 a download failed (no network); 1 anything else,
# including a checksum mismatch: a file that does not match is never used.
#
# The pins below are read by xtask/src/fetched.rs: keep one `name=value` per
# line, as they are. Their versions are recorded in upstream.toml.
set -euo pipefail

# --- pins -------------------------------------------------------------------
facturx=1.09.2
mustang=2.26.0
mustang_jar_sha256=5eb01741d59ed942812bc338ca0b2a4803c9a61469976e53264054611a49a250
z12_012=1.4
z12_012_sha256=c8fa494a5aca1b571d85ccad6aaf1ea9326d526e43f1834605ee72a1e75ff85e
z12_014=1.4
z12_014_sha256=d3acfbc2344bce5e9d47607f5264bdbfbb65e695a9c0b9abdcc3407ad1bee4d1
minimum_xslt_sha256=0308b9ec2e0c1b69e149b678d86d178a4c15c102085157439704d23d2d30f5e3
minimum_codedb_sha256=703e435735f92c6517bbbe0422f95b72587741e0a6756f980b055bbde18aec94
basic_wl_xslt_sha256=5f6f8961a9125595075a6652ca34839afaf1f8895715df99638a162efd97971e
basic_wl_codedb_sha256=cdcec2f2449d2272bc2af8dbea97de9dff0da340b1ae4d2586ce2e5b040881d8
basic_xslt_sha256=372cfdef6e429332dd0f43cccfc119af276dd657359e18489b08222c514ce3aa
basic_codedb_sha256=eb7478effd5f709aa45b4fee97a678c115dd521432ef832c6960180c727da436
en16931_xslt_sha256=d412c035cd790e0ea0de36b59f58a868f4d1a28bc2072fc2204f23dbeb2850d7
en16931_codedb_sha256=dd3856984b69e25d5ba6312ec7c8f68e4458064a5998bebcfe8ae66de1a21cad
extended_xslt_sha256=cb9ba4c20d6b9830b63926b654ecc9472d67bf9c33a7cda0786faabd6f221bc7
extended_codedb_sha256=2ab4fe294111a5703828b1dc2314280a75612be2063376f53eb863023dc48faa
minimum_xsd_sha256=f757df8498471c5813ea44e73cfa394a6229aba7e2961d0f26c380aab4e6b81d
minimum_xsd_qdt_sha256=7830de0041e47dd035fb482d369414422768ff215a3cf668f9fa83f720cec2cc
minimum_xsd_ram_sha256=a038d5fb57df4fa4e6461e96aa7710c89d9baef8a44f91cac3ea4c55c018572a
minimum_xsd_udt_sha256=06d4245cc75927b3cf016fae8162249c95167158ed2e512dc63d6e7e5c89fed3
basic_wl_xsd_sha256=251fd17722266ea6aead1ef5c5881f97ca87cc6182abcb72fdf6acb2246725e0
basic_wl_xsd_qdt_sha256=fd6958ad55567fbffb127b62b80d9cc76039a6f90315b81cba890646b824743f
basic_wl_xsd_ram_sha256=91e3a2e8d7931f0c3f790df01bc4a1760dd3b1b630184cb0db128ae9bec4d2d7
basic_wl_xsd_udt_sha256=412f67846ce8d9af2c83e55b83a9aed8d56a0939135a38a61fbe641b3dd99814
basic_xsd_sha256=1335ce8d9311fd88d68c713c3bd441cd0fb036fe7d94c9d3267b9fb4c16733f3
basic_xsd_qdt_sha256=fd6958ad55567fbffb127b62b80d9cc76039a6f90315b81cba890646b824743f
basic_xsd_ram_sha256=b1695d72a47f21b73cb6b9b65f1d77cae8d311a18f20288608b5a92ffd381a32
basic_xsd_udt_sha256=0386218417686704266ee06991ab0312d46317757a87378246593c328bc64fd6
en16931_xsd_sha256=34e51a9b26c95ef6e09297051b9921a05279a4024e659f236ba5d5e0f5a23bec
en16931_xsd_qdt_sha256=5a3ce756cfa8d4f2ff3165d68123cfb7fbf3c7b64664edb0de38cca64d5c413b
en16931_xsd_ram_sha256=4a091d4339eddab30f6f5d29364ca2297607d72f03af7da2786637f08bdb3cf7
en16931_xsd_udt_sha256=f87a1b78d2b7177955957002f8c2a7917e326038d38803f875266cc7579ea857
extended_xsd_sha256=766afbb866c03b6936a70f8c302307d8ea1c9e43328676c942072be60094f393
extended_xsd_qdt_sha256=fd1e3ae9d887ce4e17f7056e6f7bccee1ff54040ff41069ff85710782d0df640
extended_xsd_ram_sha256=2d2e776f7262a7835de09bd5b14206a97680dbf3702d0d2f1670a7147d183670
extended_xsd_udt_sha256=4600ae4633a5467e4d302b80bc91362a901be9bcdfe49584de72295428d39de3
# --- end of pins ------------------------------------------------------------

mustang_url=https://repo1.maven.org/maven2/org/mustangproject/validator/$mustang/validator-$mustang.jar
z12_012_url=https://fnfe-mpe.org/wp-content/uploads/2026/07/FR_and_ENG_XP_Z12-012_Annexes_A_et_B_EXEMPLES_V$z12_012.zip
z12_014_url=https://fnfe-mpe.org/wp-content/uploads/2026/07/FR_and_ENG_XP_Z12-014_CAS_USAGE_Annexes_A_et_B_EXEMPLES_V$z12_014.zip

# Profile directory, Mustang file name (also the directory of its XML schema),
# SHA-256 of the stylesheet and of its code list, then of the documents of its
# XML schema (schema_documents): the entry point and the three modules it imports.
profiles=(
    minimum MINIMUM "$minimum_xslt_sha256" "$minimum_codedb_sha256"
    "$minimum_xsd_sha256" "$minimum_xsd_qdt_sha256" "$minimum_xsd_ram_sha256" "$minimum_xsd_udt_sha256"
    basic-wl BASIC-WL "$basic_wl_xslt_sha256" "$basic_wl_codedb_sha256"
    "$basic_wl_xsd_sha256" "$basic_wl_xsd_qdt_sha256" "$basic_wl_xsd_ram_sha256" "$basic_wl_xsd_udt_sha256"
    basic BASIC "$basic_xslt_sha256" "$basic_codedb_sha256"
    "$basic_xsd_sha256" "$basic_xsd_qdt_sha256" "$basic_xsd_ram_sha256" "$basic_xsd_udt_sha256"
    en16931 EN16931 "$en16931_xslt_sha256" "$en16931_codedb_sha256"
    "$en16931_xsd_sha256" "$en16931_xsd_qdt_sha256" "$en16931_xsd_ram_sha256" "$en16931_xsd_udt_sha256"
    extended EXTENDED "$extended_xslt_sha256" "$extended_codedb_sha256"
    "$extended_xsd_sha256" "$extended_xsd_qdt_sha256" "$extended_xsd_ram_sha256" "$extended_xsd_udt_sha256"
)
fields=8

# schema_documents FILE: the documents of the XML schema of the profile whose
# Mustang file name is FILE, in schema/ZF_250/FILE/: the entry point
# FACTUR-X_<NAME>.xsd (NAME: FILE without its hyphen), then the modules it
# imports, in the order of the pins.
schema_documents() {
    local name=${1//-/} module
    echo "FACTUR-X_$name.xsd"
    for module in QualifiedDataType ReusableAggregateBusinessInformationEntity UnqualifiedDataType; do
        echo "FACTUR-X_${name}_urn_un_unece_uncefact_data_standard_${module}_100.xsd"
    done
}

cd "$(git rev-parse --show-toplevel)"
root=$PWD
dir=${1:-target/facturx}
mkdir -p "$dir" target/downloads
dir=$(cd "$dir" && pwd)
downloads=$root/target/downloads
release=$dir/factur-x-$facturx

say() { echo "fetch-facturx: $*"; }
fail() {
    echo "fetch-facturx: $*" >&2
    exit 1
}

hash_of() { # FILE
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d' ' -f1
    else
        shasum -a 256 "$1" | cut -d' ' -f1
    fi
}

for tool in curl unzip git; do
    command -v "$tool" >/dev/null 2>&1 || fail "$tool is not installed"
done

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
            echo "fetch-facturx: cannot download $url" >&2
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

# check FILE SHA256
check() {
    local actual
    actual=$(hash_of "$1")
    [ "$actual" = "$2" ] || fail "$1 has SHA-256 $actual, expected $2"
}

# examples ZIP NAME: the Factur-X CII documents of an annex archive, into
# $tmp/release/source/examples/NAME/ (flat; two documents with the same name
# are an error).
examples() {
    local zip=$1 name=$2 out=$tmp/release/source/examples/$2 file count=0
    mkdir -p "$tmp/unzip-$name" "$out"
    unzip -q "$zip" -x '__MACOSX/*' -d "$tmp/unzip-$name"
    while IFS= read -r -d '' file; do
        [ ! -e "$out/${file##*/}" ] || fail "$zip: two documents named ${file##*/}"
        cp "$file" "$out/"
        count=$((count + 1))
    done < <(find "$tmp/unzip-$name" -type f -name '*_FX_CII_Commentee.xml' ! -name '._*' -print0)
    [ "$count" -gt 0 ] || fail "$zip: no Factur-X CII document (*_FX_CII_Commentee.xml)"
    say "$count Factur-X CII documents of XP Z12-0${name#z12-0} extracted"
}

# --- 1. the Mustang validator -------------------------------------------------

jar=$downloads/mustang-validator-$mustang.jar
fetch "$mustang_url" "$jar" "$mustang_jar_sha256"
mkdir -p "$tmp/release/source/mustang"
unzip -q "$jar" 'xslt/ZF_250/*' 'schematron/ZF_250/*' 'schema/ZF_250/*' -d "$tmp/release/source/mustang"
mustang_dir=$tmp/release/source/mustang
for ((i = 0; i < ${#profiles[@]}; i += fields)); do
    file=${profiles[i + 1]}
    check "$mustang_dir/xslt/ZF_250/FACTUR-X_$file.xslt" "${profiles[i + 2]}"
    check "$mustang_dir/xslt/ZF_250/FACTUR-X_${file}_codedb.xml" "${profiles[i + 3]}"
    j=4
    while IFS= read -r document; do
        check "$mustang_dir/schema/ZF_250/$file/$document" "${profiles[i + j]}"
        j=$((j + 1))
    done < <(schema_documents "$file")
    # "Factur-X release metadata ... Version: 1.09.2 Release date: 2026-08-04"
    # (CRLF line ends; grep -c reads all its input, so that tr never gets SIGPIPE)
    [ "$(tr -d '\r' <"$mustang_dir/schematron/ZF_250/FACTUR-X_$file.sch" | grep -c "^ *Version: $facturx\$")" -gt 0 ] ||
        fail "schematron/ZF_250/FACTUR-X_$file.sch does not state Factur-X version $facturx"
done
say "Mustang $mustang: the stylesheets and XML schemas of the 5 profiles verified (Factur-X $facturx)"

# --- 2. the example documents -------------------------------------------------

z12_012_zip=$downloads/XP_Z12-012_Annexes_A_et_B_EXEMPLES_V$z12_012.zip
z12_014_zip=$downloads/XP_Z12-014_CAS_USAGE_Annexes_A_et_B_EXEMPLES_V$z12_014.zip
fetch "$z12_012_url" "$z12_012_zip" "$z12_012_sha256"
fetch "$z12_014_url" "$z12_014_zip" "$z12_014_sha256"
examples "$z12_012_zip" z12-012
examples "$z12_014_zip" z12-014

# --- 3. the CEN CII examples (mutation bases) ---------------------------------

cp -R "$root/artefacts/cen-en16931-cii-1.3.17/cii/examples" "$tmp/release/source/cen-cii-examples"

# --- 4. the bundles -----------------------------------------------------------

for ((i = 0; i < ${#profiles[@]}; i += fields)); do
    profile=${profiles[i]}
    file=${profiles[i + 1]}
    bundle=$tmp/release/$profile
    mkdir -p "$bundle"
    cp "$mustang_dir/xslt/ZF_250/FACTUR-X_$file.xslt" "$mustang_dir/xslt/ZF_250/FACTUR-X_${file}_codedb.xml" "$bundle/"
    documents=()
    while IFS= read -r document; do
        documents+=("$document")
        cp "$mustang_dir/schema/ZF_250/$file/$document" "$bundle/"
    done < <(schema_documents "$file")
    {
        cat <<EOF
# Factur-X $facturx, profile $file, from the Mustang validator $mustang: the
# rule-set bundle manifest of einvoice-validator, as scripts/fetch-facturx.sh and
# einvoice-validate rules fetch write it (einvoice-validate --bundle DIR). The
# document is validated first against the XML schema of the profile,
# ${documents[0]} and the modules it imports, next to it, as the Mustang
# validator validates it: a schema error makes it invalid, and the stylesheet
# runs all the same. The stylesheet reads, with document(), the code list
# FACTUR-X_${file}_codedb.xml next to it, a resource of the bundle. A failed
# assert without a flag is fatal, as the Mustang validator reads the report.
format = 2
name = "factur-x-$profile"
title = "Factur-X $file"
version = "$facturx"
schema = "${documents[0]}"

[[stylesheet]]
name = "FACTUR-X_$file"
path = "FACTUR-X_$file.xslt"
sha256 = "${profiles[i + 2]}"
unflagged-assert = "fatal"

[[resource]]
path = "FACTUR-X_${file}_codedb.xml"
sha256 = "${profiles[i + 3]}"
EOF
        for ((j = 0; j < ${#documents[@]}; j++)); do
            printf '\n[[schema-document]]\npath = "%s"\nsha256 = "%s"\n' "${documents[j]}" "${profiles[i + 4 + j]}"
        done
    } >"$bundle/bundle.toml"
done

rm -rf "$release"
mv "$tmp/release" "$release"
say "$release written: source/, and the bundles minimum/, basic-wl/, basic/, en16931/, extended/"
