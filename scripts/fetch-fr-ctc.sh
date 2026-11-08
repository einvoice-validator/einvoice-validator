#!/usr/bin/env bash
# Builds the inputs of the tests of the French e-invoicing rules (FNFE-MPE
# "Schematrons FR CTC", for the formats and profiles of the French standard
# XP Z12-012) outside the tracked tree: the compiled stylesheets of the
# package, the example documents of the standard, and the rule-set bundles.
# The licence statements of the package disagree (the Apache License 2.0 in
# its documentation, "European Union Public Licence (EUPL) version 1.4.0" in
# its Schematron files) and the example documents are published by AFNOR with
# all rights reserved, so none of these files, nor anything derived from them,
# is part of this repository; every input is checked against the checksum
# below.
#
#   scripts/fetch-fr-ctc.sh            # into target/fr-ctc/
#   scripts/fetch-fr-ctc.sh DIR        # into DIR/
#
# Steps, into DIR/fr-ctc-1.4.0.04/ (always afresh), downloads cached in
# target/downloads/ (shared with scripts/fetch-facturx.sh; a cached file is
# checked, never trusted):
#  1. The package V1.4.0.04 from fnfe-mpe.org, extracted into source/pack/
#     (without the __MACOSX/ and .DS_Store entries). Every compiled stylesheet
#     (*.xslt) and every code list it reads (*_codedb.xml) of the package must
#     have one of the SHA-256 below: CII (EN 16931 as the package ships it,
#     EXTENDED-CTC-FR, the BR-FR rules and their WARNING version), UBL (the
#     same), CDAR (the life-cycle statuses, and WARNING), Flux 10
#     (e-reporting), and the Factur-X 1.09.2 profiles BASIC WL, EN16931 and
#     EXTENDED as the package builds them (fixFR04).
#  2. The example documents of XP Z12-012 (annex B) and XP Z12-014 (annex B),
#     from fnfe-mpe.org, sorted by syntax into
#     source/examples/<z12-012|z12-014>/<cii|ubl|cdar>/ by their file names
#     (*_CII_Commentee.xml, *_UBL_Commentee.xml, *-CDV-*.xml; any other XML
#     file is an error).
#  3. The CEN EN 16931 examples vendored in this repository (CII examples,
#     UBL examples and test files, checked by `cargo xtask manifest`), copied
#     to source/cen-cii-examples/, source/cen-ubl-examples/ and
#     source/cen-ubl-testfiles/: the base documents of the mutation
#     catalogues.
#  4. The rule-set bundles of einvoice-validator, each a directory with its
#     bundle.toml and the stylesheets it names, in the order the package
#     documents (EN 16931 or EXTENDED-CTC-FR rules, then the BR-FR rules):
#     cii-en16931, cii-extended-ctc-fr, ubl-en16931, ubl-extended-ctc-fr, cdar,
#     each also as <name>-warning with the WARNING version of the French rules,
#     factur-x-basic-wl, factur-x-en16931, factur-x-extended (a failed assert
#     of their Factur-X stylesheet without a flag is fatal, as the Mustang
#     validator reads it: unflagged-assert = "fatal"), and flux10.
#     `einvoice-validate --bundle DIR/fr-ctc-1.4.0.04/cii-en16931 FILE`.
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
pack=1.4.0.04
pack_sha256=9fddfbbb081ec969374dcefd24615fb1d736fdd974155890fc24a413afd0c01e
z12_012=1.4
z12_012_sha256=c8fa494a5aca1b571d85ccad6aaf1ea9326d526e43f1834605ee72a1e75ff85e
z12_014=1.4
z12_014_sha256=d3acfbc2344bce5e9d47607f5264bdbfbb65e695a9c0b9abdcc3407ad1bee4d1
cii_en16931_sha256=0b234dea2bbfee739b7761e607a992c17fab88773014ef56355b6158cfb1cc53
cii_extended_ctc_fr_sha256=7d766dadc198ec21a7238f546621113e61d2fdbaced65f263cafe368f96131d3
cii_br_fr_sha256=02c9b97965438c1f587269a07daa5191b15efd3408f913b57bd35570f439ef10
cii_br_fr_warning_sha256=cc76195015e9f2ef522ac303b8fb6521f1b7ce40852bd280bd1f55c211a8c026
ubl_en16931_sha256=39f9d282867f1a49e7708d9e29a53da89643e1ee56f10cec1ebcf1277595fcbd
ubl_extended_ctc_fr_sha256=5a30860a106680629afbbd6159d829bf5f353b835fd9eda24ea8e8e13587d4f9
ubl_br_fr_sha256=5ab722085cf69301acdecbb70b38620524488c602316bfba34b00fb78deb8631
ubl_br_fr_warning_sha256=40289c58f7f2f0c821afe19b8b80d851d5feb51952e3e2c464466e3086097589
cdar_sha256=9dc332d7375583043a525961471b3df33e1dc074ed467b6bb9d4edaabc59ecdd
cdar_warning_sha256=b7c9839671c0e5bbe4de09a003a7b7b4084652663bd23bc533ad661462896dfe
flux10_sha256=16af5027a6d515af6533ed58babdaf4feb3e0d06d9aab02782f9ac36ddb4879f
fx_basic_wl_sha256=e71093e4ac9e582ce7166cd7318cbef47743af5d16741c140fee862ef28986bf
fx_basic_wl_codedb_sha256=a34cd66dda00a9260368fa02dad97e966ded1c1ad100787a5318074328d20abb
fx_en16931_sha256=67f03050e3add4c83dc93de36e4501f6a11aae1ac790b3f9558feb0581019bcb
fx_en16931_codedb_sha256=70339a41d71b02bb97683f8eea04acbd764e077a88a446b32860d89c3351b62f
fx_extended_sha256=8c56149a6421fd5bb0fe26a72e64e7936b69813673b3163bca5b247afdeefb8e
fx_extended_codedb_sha256=6e23aedbba0e3b32b3b1211d145186193215f56f755201555306c42327e75fbc
# --- end of pins ------------------------------------------------------------

pack_name=2026_09_04_FNFE_SCHEMATRONS_FR_CTC_V$pack
pack_url=https://fnfe-mpe.org/wp-content/uploads/2026/09/$pack_name.zip
z12_012_url=https://fnfe-mpe.org/wp-content/uploads/2026/07/FR_and_ENG_XP_Z12-012_Annexes_A_et_B_EXEMPLES_V$z12_012.zip
z12_014_url=https://fnfe-mpe.org/wp-content/uploads/2026/07/FR_and_ENG_XP_Z12-014_CAS_USAGE_Annexes_A_et_B_EXEMPLES_V$z12_014.zip

# The stylesheets the bundles name: name in the bundles, path in the package,
# SHA-256 (code lists are named after their stylesheet, see `side_file`).
stylesheets=(
    EN16931-CII CII/EN16931/2xslt/EN16931-CII-validation.xslt "$cii_en16931_sha256"
    EXTENDED-CTC-FR-CII CII/EXTENDED-CTC-FR/2xslt/EXTENDED-CTC-FR-CII.xslt "$cii_extended_ctc_fr_sha256"
    BR-FR-Flux2-CII CII/EN16931/2xslt/BR-FR-Flux2-Schematron-CII.xslt "$cii_br_fr_sha256"
    BR-FR-Flux2-CII_WARNING CII/EN16931/2xslt/BR-FR-Flux2-Schematron-CII_WARNING.xslt "$cii_br_fr_warning_sha256"
    EN16931-UBL UBL/EN16931/2xslt/EN16931-UBL-validation.xslt "$ubl_en16931_sha256"
    EXTENDED-CTC-FR-UBL UBL/EXTENDED-CTC-FR/2xslt/EXTENDED-CTC-FR-UBL.xslt "$ubl_extended_ctc_fr_sha256"
    BR-FR-Flux2-UBL UBL/EN16931/2xslt/BR-FR-Flux2-Schematron-UBL.xslt "$ubl_br_fr_sha256"
    BR-FR-Flux2-UBL_WARNING UBL/EN16931/2xslt/BR-FR-Flux2-Schematron-UBL_WARNING.xslt "$ubl_br_fr_warning_sha256"
    BR-FR-CDV-CDAR CDAR/2xslt/BR-FR-CDV-Schematron-CDAR.xslt "$cdar_sha256"
    BR-FR-CDV-CDAR_WARNING CDAR/2xslt/BR-FR-CDV-Schematron-CDAR_WARNING.xslt "$cdar_warning_sha256"
    Flux10 Flux10/Flux10.xslt "$flux10_sha256"
    FACTUR-X_BASIC-WL Factur-X_1.09.2.fixFR04/BASICWL/2xslt/FACTUR-X_BASIC-WL.xslt "$fx_basic_wl_sha256"
    FACTUR-X_EN16931 Factur-X_1.09.2.fixFR04/EN16931/2xslt/FACTUR-X_EN16931.xslt "$fx_en16931_sha256"
    FACTUR-X_EXTENDED Factur-X_1.09.2.fixFR04/EXTENDED/2xslt/FACTUR-X_EXTENDED.xslt "$fx_extended_sha256"
)
# Every other compiled stylesheet or code list of the package is one of these
# files, byte for byte (the package repeats the BR-FR rules in each profile).
extra_sha256=(
    "$fx_basic_wl_codedb_sha256" "$fx_en16931_codedb_sha256" "$fx_extended_codedb_sha256"
)

# The bundles: directory, schema (or -), title, stylesheets in order.
bundles=(
    "cii-en16931|-|EN 16931 CII and the French rules|EN16931-CII BR-FR-Flux2-CII"
    "cii-en16931-warning|-|EN 16931 CII and the French rules (WARNING version)|EN16931-CII BR-FR-Flux2-CII_WARNING"
    "cii-extended-ctc-fr|-|EXTENDED-CTC-FR CII and the French rules|EXTENDED-CTC-FR-CII BR-FR-Flux2-CII"
    "cii-extended-ctc-fr-warning|-|EXTENDED-CTC-FR CII and the French rules (WARNING version)|EXTENDED-CTC-FR-CII BR-FR-Flux2-CII_WARNING"
    "ubl-en16931|ubl-2.1|EN 16931 UBL and the French rules|EN16931-UBL BR-FR-Flux2-UBL"
    "ubl-en16931-warning|ubl-2.1|EN 16931 UBL and the French rules (WARNING version)|EN16931-UBL BR-FR-Flux2-UBL_WARNING"
    "ubl-extended-ctc-fr|ubl-2.1|EXTENDED-CTC-FR UBL and the French rules|EXTENDED-CTC-FR-UBL BR-FR-Flux2-UBL"
    "ubl-extended-ctc-fr-warning|ubl-2.1|EXTENDED-CTC-FR UBL and the French rules (WARNING version)|EXTENDED-CTC-FR-UBL BR-FR-Flux2-UBL_WARNING"
    "cdar|-|Life-cycle statuses (CDAR)|BR-FR-CDV-CDAR"
    "cdar-warning|-|Life-cycle statuses (CDAR, WARNING version)|BR-FR-CDV-CDAR_WARNING"
    "factur-x-basic-wl|-|Factur-X BASIC WL and the French rules|FACTUR-X_BASIC-WL BR-FR-Flux2-CII"
    "factur-x-en16931|-|Factur-X EN16931 and the French rules|FACTUR-X_EN16931 BR-FR-Flux2-CII"
    "factur-x-extended|-|Factur-X EXTENDED and the French rules|FACTUR-X_EXTENDED BR-FR-Flux2-CII"
    "flux10|-|E-reporting (Flux 10)|Flux10"
)

cd "$(git rev-parse --show-toplevel)"
root=$PWD
dir=${1:-target/fr-ctc}
mkdir -p "$dir" target/downloads
dir=$(cd "$dir" && pwd)
downloads=$root/target/downloads
release=$dir/fr-ctc-$pack

say() { echo "fetch-fr-ctc: $*"; }
fail() {
    echo "fetch-fr-ctc: $*" >&2
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
            echo "fetch-fr-ctc: cannot download $url" >&2
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

# stylesheet NAME: "PATH SHA256" of the stylesheet NAME.
stylesheet() {
    local i
    for ((i = 0; i < ${#stylesheets[@]}; i += 3)); do
        if [ "${stylesheets[i]}" = "$1" ]; then
            echo "${stylesheets[i + 1]} ${stylesheets[i + 2]}"
            return 0
        fi
    done
    fail "no stylesheet $1"
}

# factur_x NAME: whether the stylesheet NAME is a Factur-X profile (FACTUR-X_<P>),
# whose failed asserts without a flag are fatal, as the Mustang validator reads
# them (unflagged-assert = "fatal" in the bundles).
factur_x() {
    case $1 in
    FACTUR-X_*) return 0 ;;
    *) return 1 ;;
    esac
}

# side_file PATH: the code list a Factur-X stylesheet reads with document()
# (FACTUR-X_<P>.xslt reads FACTUR-X_<P>_codedb.xml next to it), or nothing.
side_file() {
    case $1 in
    */FACTUR-X_*.xslt) echo "${1%.xslt}_codedb.xml" ;;
    esac
}

# --- 1. the package -----------------------------------------------------------

pack_zip=$downloads/$pack_name.zip
fetch "$pack_url" "$pack_zip" "$pack_sha256"
mkdir -p "$tmp/unzip-pack" "$tmp/release/source"
unzip -q "$pack_zip" -x '__MACOSX/*' '*.DS_Store' -d "$tmp/unzip-pack"
[ -d "$tmp/unzip-pack/$pack_name" ] || fail "$pack_zip has no $pack_name/"
mv "$tmp/unzip-pack/$pack_name" "$tmp/release/source/pack"
packdir=$tmp/release/source/pack
known=()
for ((i = 0; i < ${#stylesheets[@]}; i += 3)); do
    path=$packdir/${stylesheets[i + 1]}
    [ -f "$path" ] || fail "$pack_zip has no ${stylesheets[i + 1]}"
    actual=$(hash_of "$path")
    [ "$actual" = "${stylesheets[i + 2]}" ] ||
        fail "${stylesheets[i + 1]} has SHA-256 $actual, expected ${stylesheets[i + 2]}"
    known+=("${stylesheets[i + 2]}")
done
known+=("${extra_sha256[@]}")
count=0
while IFS= read -r -d '' file; do
    actual=$(hash_of "$file")
    case " ${known[*]} " in
    *" $actual "*) ;;
    *) fail "${file#"$packdir"/} has SHA-256 $actual, none of the pinned ones" ;;
    esac
    count=$((count + 1))
done < <(find "$packdir" -type f \( -name '*.xslt' -o -name '*_codedb.xml' \) -print0)
say "package $pack: $count compiled stylesheets and code lists verified"

# --- 2. the example documents -------------------------------------------------

# examples ZIP NAME: the documents of an annex archive into
# source/examples/NAME/<syntax>/ (flat; two documents with the same name in
# one syntax are an error).
examples() {
    local zip=$1 name=$2 out=$tmp/release/source/examples/$2 file base syntax count=0
    mkdir -p "$tmp/unzip-$name"
    unzip -q "$zip" -x '__MACOSX/*' -d "$tmp/unzip-$name"
    while IFS= read -r -d '' file; do
        base=${file##*/}
        case $base in
        *_CII_Commentee.xml) syntax=cii ;;
        *_UBL_Commentee.xml) syntax=ubl ;;
        *-CDV-*.xml) syntax=cdar ;;
        *) fail "$zip: ${file#"$tmp/unzip-$name"/}: neither CII, UBL nor CDAR by its name" ;;
        esac
        mkdir -p "$out/$syntax"
        [ ! -e "$out/$syntax/$base" ] || fail "$zip: two $syntax documents named $base"
        cp "$file" "$out/$syntax/"
        count=$((count + 1))
    done < <(find "$tmp/unzip-$name" -type f -name '*.xml' ! -name '._*' -print0)
    [ "$count" -gt 0 ] || fail "$zip: no example document"
    say "$count example documents of XP Z12-0${name#z12-0} extracted"
}

z12_012_zip=$downloads/XP_Z12-012_Annexes_A_et_B_EXEMPLES_V$z12_012.zip
z12_014_zip=$downloads/XP_Z12-014_CAS_USAGE_Annexes_A_et_B_EXEMPLES_V$z12_014.zip
fetch "$z12_012_url" "$z12_012_zip" "$z12_012_sha256"
fetch "$z12_014_url" "$z12_014_zip" "$z12_014_sha256"
examples "$z12_012_zip" z12-012
examples "$z12_014_zip" z12-014

# --- 3. the CEN examples (mutation bases) -------------------------------------

cp -R "$root/artefacts/cen-en16931-cii-1.3.17/cii/examples" "$tmp/release/source/cen-cii-examples"
cp -R "$root/artefacts/cen-en16931-ubl-1.3.17/ubl/examples" "$tmp/release/source/cen-ubl-examples"
cp -R "$root/artefacts/cen-en16931-ubl-1.3.17/test/testfiles" "$tmp/release/source/cen-ubl-testfiles"

# --- 4. the bundles -----------------------------------------------------------

for entry in "${bundles[@]}"; do
    IFS='|' read -r name schema title names <<<"$entry"
    bundle=$tmp/release/$name
    mkdir -p "$bundle"
    {
        echo "# $title, FNFE-MPE package $pack: the rule-set bundle"
        echo "# manifest of einvoice-validator, as scripts/fetch-fr-ctc.sh and"
        echo "# einvoice-validate rules fetch write it (einvoice-validate --bundle DIR)."
        side=()
        for stylesheet_name in $names; do
            read -r path _ <<<"$(stylesheet "$stylesheet_name")"
            file=$(side_file "$path")
            [ -z "$file" ] || side+=("$file")
        done
        for file in "${side[@]}"; do
            base=${file##*/}
            echo "# The stylesheet ${base%_codedb.xml}.xslt reads, with document(), the code list"
            echo "# $base next to it, a resource of the bundle."
        done
        for stylesheet_name in $names; do
            if factur_x "$stylesheet_name"; then
                echo "# A failed assert of $stylesheet_name without a flag is fatal, as the"
                echo "# Mustang validator reads the Factur-X reports."
            fi
        done
        echo "format = 2"
        echo "name = \"fr-ctc-$name\""
        echo "title = \"$title\""
        echo "version = \"$pack\""
        [ "$schema" = - ] || echo "schema = \"$schema\""
        for stylesheet_name in $names; do
            read -r path sha256 <<<"$(stylesheet "$stylesheet_name")"
            cp "$packdir/$path" "$bundle/$stylesheet_name.xslt"
            file=$(side_file "$path")
            [ -z "$file" ] || cp "$packdir/$file" "$bundle/"
            echo
            echo "[[stylesheet]]"
            echo "name = \"$stylesheet_name\""
            echo "path = \"$stylesheet_name.xslt\""
            echo "sha256 = \"$sha256\""
            if factur_x "$stylesheet_name"; then
                echo "unflagged-assert = \"fatal\""
            fi
        done
        for file in "${side[@]}"; do
            echo
            echo "[[resource]]"
            echo "path = \"${file##*/}\""
            echo "sha256 = \"$(hash_of "$packdir/$file")\""
        done
    } >"$bundle/bundle.toml"
done

rm -rf "$release"
mv "$tmp/release" "$release"
say "$release written: source/ and ${#bundles[@]} bundles"
