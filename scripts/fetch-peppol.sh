#!/usr/bin/env bash
# Builds the Peppol BIS Billing 3.0 inputs of the tests outside the tracked
# tree: the release archive of the pinned tag, and its Schematron files
# compiled to XSLT. The Peppol BIS Billing 3.0 repository publishes no
# licence, so none of its files, nor anything derived from them, is part of
# this repository; this script rebuilds them from pinned inputs, every one
# checked against the checksum below.
#
#   scripts/fetch-peppol.sh            # into target/peppol/
#   scripts/fetch-peppol.sh DIR        # into DIR/
#
# Steps:
#  1. Download the GitHub archive of Peppol BIS Billing 3.0.22 (repository
#     OpenPeppol/peppol-bis-invoice-3, tag=3.0.22 below), check its SHA-256
#     and the commit id it records, and extract it into
#     DIR/peppol-bis-billing-ubl-3.0.22/source/ (always afresh).
#  2. Compile rules/sch/PEPPOL-EN16931-UBL.sch and rules/sch/CEN-EN16931-UBL.sch
#     with the ISO Schematron skeleton as packaged in ph-schematron:
#     ph-schematron-maven-plugin 10.3.1, goal `convert`, engine
#     iso-schematron, run by Apache Maven 3.9.16 (from Maven Central, SHA-512
#     checked) with strict checksums and a private local repository, DIR/m2/
#     (the user's Maven settings and repository are not used). Each result
#     must have the SHA-256 below; they go into
#     DIR/peppol-bis-billing-ubl-3.0.22/xslt/. When both files are already
#     there with the right checksums, Maven is not run.
#  3. Write DIR/peppol-bis-billing-ubl-3.0.22/bundle.toml, the rule-set
#     bundle manifest of einvoice-validator naming both stylesheets with
#     their pinned SHA-256, CEN rules first, as Peppol validation runs them,
#     the CEN stylesheet with the version its Schematron file states, which
#     must be the pin cen below:
#     `einvoice-validate --bundle DIR/peppol-bis-billing-ubl-3.0.22 FILE`.
#     `einvoice-validate rules fetch peppol` runs the same steps, from the
#     same pins (crates/einvoice-validator/pins/, a copy of the block below),
#     and writes the same bundle, byte for byte.
#
# Needs bash, curl, tar, gzip, git and Java (CI uses Java 21).
# Exit status: 0 done; 75 a download failed (no network); 1 anything else,
# including a checksum mismatch: a file that does not match is never used.
#
# The pins below are read by xtask/src/fetched.rs and by the Peppol
# equivalence test of crates/einvoice-xslt-exec: keep one `name=value` per
# line, as they are. Their versions are recorded in upstream.toml. For a new
# release, set tag, commit and archive_sha256, rename the directory
# peppol-bis-billing-ubl-<version> where upstream.toml lists it, and take the
# SHA-256 of the compiled stylesheets from a first run, after reviewing them.
set -euo pipefail

# --- pins -------------------------------------------------------------------
tag=3.0.22
commit=38d693d71d28d813781323a6ca97324edcc4d239
archive_sha256=0bec0d84c444fd5afa3a4a2d6462e162afc157560bd100052b7e589bd94ebd13
cen=1.3.16
maven=3.9.16
maven_sha512=831a8591fe20c8243b1dbe7d71e3244f31d1665b0804b2e825e38cbbe5ce0cafb8338851f90780735568773e0a6cd07bbec107cda0b896b008b861075358b6f6
plugin=10.3.1
peppol_xslt_sha256=8c318aad5a3fc1fc69897422e1d2f631d8894069be5ac20e3475c7e6a2b62aeb
cen_xslt_sha256=06a850a48703faa7e38cbcc7a8ded0f0645a941a0ee448598a02ae7197b86356
# --- end of pins ------------------------------------------------------------

archive_url=https://github.com/OpenPeppol/peppol-bis-invoice-3/archive/refs/tags/$tag.tar.gz
maven_url=https://repo1.maven.org/maven2/org/apache/maven/apache-maven/$maven/apache-maven-$maven-bin.tar.gz

cd "$(git rev-parse --show-toplevel)"
dir=${1:-target/peppol}
mkdir -p "$dir"
dir=$(cd "$dir" && pwd)
release=$dir/peppol-bis-billing-ubl-$tag

say() { echo "fetch-peppol: $*"; }
fail() {
    echo "fetch-peppol: $*" >&2
    exit 1
}

hash_of() { # ALGORITHM FILE
    if command -v "sha$1sum" >/dev/null 2>&1; then
        "sha$1sum" "$2" | cut -d' ' -f1
    else
        shasum -a "$1" "$2" | cut -d' ' -f1
    fi
}

for tool in curl tar gzip git; do
    command -v "$tool" >/dev/null 2>&1 || fail "$tool is not installed"
done

tmp=$(mktemp -d "$dir/.tmp.XXXXXX")
trap 'rm -rf "$tmp"' EXIT

# fetch URL FILE ALGORITHM DIGEST: FILE, downloaded if missing; its digest
# must be DIGEST, whether it was just downloaded or already there.
fetch() {
    local url=$1 file=$2 algorithm=$3 digest=$4 actual
    if [ ! -e "$file" ]; then
        mkdir -p "$(dirname "$file")"
        # Every failure is retried (an HTTP 504 or a reset connection
        # included), five times, five seconds apart, for two minutes at most.
        if ! curl --proto '=https' --tlsv1.2 -fsSL --retry 5 --retry-all-errors --retry-delay 5 \
            --retry-max-time 120 -o "$tmp/download" "$url"; then
            echo "fetch-peppol: cannot download $url" >&2
            exit 75
        fi
        actual=$(hash_of "$algorithm" "$tmp/download")
        [ "$actual" = "$digest" ] ||
            fail "download from $url has SHA-$algorithm $actual, expected $digest"
        mv "$tmp/download" "$file"
        say "$file downloaded, SHA-$algorithm verified"
        return 0
    fi
    actual=$(hash_of "$algorithm" "$file")
    [ "$actual" = "$digest" ] ||
        fail "$file has SHA-$algorithm $actual, expected $digest; delete it and run again"
    say "$file already present, SHA-$algorithm verified"
}

# --- 1. the release archive ---------------------------------------------------

archive=$dir/downloads/peppol-bis-invoice-3-$tag.tar.gz
fetch "$archive_url" "$archive" 256 "$archive_sha256"
# git reads the first header only, so gzip may die of SIGPIPE: the status of the
# pipeline is git's (the archive itself is already checked).
recorded=$(
    set +o pipefail
    gzip -dc "$archive" | git get-tar-commit-id
) || fail "$archive records no commit id"
[ "$recorded" = "$commit" ] || fail "$archive is commit $recorded, expected $commit"

mkdir "$tmp/source"
tar -xzf "$archive" -C "$tmp/source" --strip-components=1 --no-same-owner --no-same-permissions
for sch in PEPPOL-EN16931-UBL CEN-EN16931-UBL; do
    [ -f "$tmp/source/rules/sch/$sch.sch" ] || fail "$archive has no rules/sch/$sch.sch"
done
mkdir -p "$release"
rm -rf "$release/source"
mv "$tmp/source" "$release/source"
say "$release/source extracted (tag $tag, commit $commit)"

# --- 2. the compiled stylesheets ----------------------------------------------

xslt=(
    PEPPOL-EN16931-UBL "$peppol_xslt_sha256"
    CEN-EN16931-UBL "$cen_xslt_sha256"
)

compiled() { # true when every stylesheet is in place with its checksum
    local i
    for ((i = 0; i < ${#xslt[@]}; i += 2)); do
        [ -f "$release/xslt/${xslt[i]}.xslt" ] || return 1
        [ "$(hash_of 256 "$release/xslt/${xslt[i]}.xslt")" = "${xslt[i + 1]}" ] || return 1
    done
}

# --- 3. the bundle manifest ---------------------------------------------------

# Written afresh on every run, from the pins: the stylesheets it names have
# been checked against the same checksums just before.
write_bundle() {
    local cen_version
    # "<!--Schematron version 1.3.16 - Last update: ...-->"
    cen_version=$(sed -n 's/.*Schematron version \([0-9][0-9.]*[0-9]\) .*/\1/p' \
        "$release/source/rules/sch/CEN-EN16931-UBL.sch" | head -n 1)
    [ -n "$cen_version" ] || fail "rules/sch/CEN-EN16931-UBL.sch states no Schematron version"
    [ "$cen_version" = "$cen" ] ||
        fail "rules/sch/CEN-EN16931-UBL.sch states Schematron version $cen_version, expected $cen"
    cat >"$tmp/bundle.toml" <<EOF
# Peppol BIS Billing $tag (UBL): the rule-set bundle manifest of
# einvoice-validator, as scripts/fetch-peppol.sh and einvoice-validate rules
# fetch write it (einvoice-validate --bundle DIR).
format = 1
name = "peppol-bis-billing-ubl"
title = "Peppol BIS Billing 3.0 UBL"
version = "$tag"
schema = "ubl-2.1"

[[stylesheet]]
name = "CEN-EN16931-UBL"
path = "xslt/CEN-EN16931-UBL.xslt"
sha256 = "$cen_xslt_sha256"
version = "$cen"

[[stylesheet]]
name = "PEPPOL-EN16931-UBL"
path = "xslt/PEPPOL-EN16931-UBL.xslt"
sha256 = "$peppol_xslt_sha256"
EOF
    mv "$tmp/bundle.toml" "$release/bundle.toml"
    say "$release/bundle.toml written"
}

if compiled; then
    say "$release/xslt already compiled, SHA-256 verified"
    write_bundle
    exit 0
fi

command -v "${JAVA:-java}" >/dev/null 2>&1 || fail "java is not installed: Maven cannot run"
if [ -n "${JAVA:-}" ] && [ -z "${JAVA_HOME:-}" ]; then
    # Maven finds Java through JAVA_HOME or the PATH.
    PATH=$(dirname "$(command -v "$JAVA")"):$PATH
fi

maven_archive=$dir/downloads/apache-maven-$maven-bin.tar.gz
fetch "$maven_url" "$maven_archive" 512 "$maven_sha512"
rm -rf "$dir/tools/apache-maven-$maven"
mkdir -p "$dir/tools"
tar -xzf "$maven_archive" -C "$dir/tools" --no-same-owner --no-same-permissions
mvn=$dir/tools/apache-maven-$maven/bin/mvn
[ -x "$mvn" ] || fail "$maven_archive has no bin/mvn"

work=$tmp/compile
mkdir -p "$work/in" "$work/out" "$dir/m2"
for ((i = 0; i < ${#xslt[@]}; i += 2)); do
    cp "$release/source/rules/sch/${xslt[i]}.sch" "$work/in/"
done
# Only this repository: the user's ~/.m2/settings.xml (mirrors, proxies,
# repositories) and ~/.m2/repository are not used.
cat >"$work/settings.xml" <<EOF
<settings><localRepository>$dir/m2</localRepository></settings>
EOF
cat >"$work/pom.xml" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<project xmlns="http://maven.apache.org/POM/4.0.0">
  <modelVersion>4.0.0</modelVersion>
  <groupId>local.build</groupId>
  <artifactId>peppol-schematron-compile</artifactId>
  <version>1</version>
  <packaging>pom</packaging>
  <build>
    <plugins>
      <plugin>
        <groupId>com.helger.maven</groupId>
        <artifactId>ph-schematron-maven-plugin</artifactId>
        <version>$plugin</version>
        <executions>
          <execution>
            <id>convert</id>
            <phase>validate</phase>
            <goals><goal>convert</goal></goals>
            <configuration>
              <schematronDirectory>$work/in</schematronDirectory>
              <schematronPattern>*.sch</schematronPattern>
              <xsltDirectory>$work/out</xsltDirectory>
              <schematronEngine>iso-schematron</schematronEngine>
            </configuration>
          </execution>
        </executions>
      </plugin>
    </plugins>
  </build>
</project>
EOF
# Maven 3.9 downloads through the native HTTP transport of Maven Resolver
# (selected explicitly, so that the settings below apply): a failed request is
# retried five times, with an exponential back-off from five seconds, on an
# I/O error and on the HTTP statuses listed (by default only 429 and 503).
maven_retries=(
    -Dmaven.resolver.transport=native
    -Daether.connector.http.retryHandler.count=5
    -Daether.connector.http.retryHandler.interval=5000
    '-Daether.connector.http.retryHandler.serviceUnavailable=429,500,502,503,504'
)
say "compiling with ph-schematron-maven-plugin $plugin (Maven $maven)"
status=0
MAVEN_SKIP_RC=1 "$mvn" --batch-mode --strict-checksums --no-transfer-progress --quiet \
    "${maven_retries[@]}" \
    --settings "$work/settings.xml" --file "$work/pom.xml" validate >"$tmp/maven.log" 2>&1 || status=$?
if [ "$status" -ne 0 ]; then
    cat "$tmp/maven.log" >&2
    # Maven reports a failed download as "Could not transfer" or "Could not resolve".
    if grep -q 'Could not transfer\|Could not resolve\|Unknown host\|UnknownHostException' "$tmp/maven.log"; then
        echo "fetch-peppol: Maven could not download the plugin" >&2
        exit 75
    fi
    fail "Maven failed (status $status)"
fi

for ((i = 0; i < ${#xslt[@]}; i += 2)); do
    out=$work/out/${xslt[i]}.xslt
    [ -f "$out" ] || fail "Maven wrote no ${xslt[i]}.xslt"
    actual=$(hash_of 256 "$out")
    [ "$actual" = "${xslt[i + 1]}" ] ||
        fail "${xslt[i]}.xslt compiled with SHA-256 $actual, expected ${xslt[i + 1]}"
done
rm -rf "$release/xslt"
mv "$work/out" "$release/xslt"
say "$release/xslt compiled, SHA-256 verified"
write_bundle
