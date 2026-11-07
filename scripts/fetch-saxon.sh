#!/usr/bin/env bash
# Downloads Saxon-HE, the reference engine of the oracles (`cargo xtask
# oracle` and the `saxon-oracle` examples), from Maven Central with the two
# jars it needs at run time, and verifies the SHA-256 of each. Saxon-HE is
# MPL-2.0, the XML Resolver is Apache-2.0; neither is part of this repository:
# they are fetched into target/oracle/saxon/ (or the directory given as first
# argument), outside the tracked tree.
#
#   scripts/fetch-saxon.sh            # into target/oracle/saxon/
#   scripts/fetch-saxon.sh DIR        # into DIR/
#
# The three jars, which the oracles put on the class path of
# net.sf.saxon.Transform:
#   Saxon-HE-12.10.jar                net.sf.saxon:Saxon-HE:12.10
#   xmlresolver-5.3.3.jar             org.xmlresolver:xmlresolver:5.3.3 (a dependency
#   xmlresolver-5.3.3-data.jar        of Saxon-HE 12.10 in its POM, with the `data` classifier)
#
# An existing jar is verified, never trusted: a checksum mismatch, of a fresh
# download or of a cached file, is an error and nothing is overwritten.
# Exit status: 0 done; 75 a download failed (no network); 1 anything else.
# Keep the versions and checksums in step with xtask/src/oracle.rs and the
# saxon-oracle examples of einvoice-xdm and einvoice-xpath-eval. The
# saxon-probes example of einvoice-xslt-exec reads them from the `jars` array
# below (one word per line, `$name` standing for a `name=` line): keep that form.
set -euo pipefail

version=12.10
resolver=5.3.3
maven=https://repo1.maven.org/maven2
# file name, SHA-256, URL
jars=(
    "Saxon-HE-$version.jar"
    b571af282f25d7301059f788b9a149aab8b5cdc14ef3d212dc5425d3dcbb9a97
    "$maven/net/sf/saxon/Saxon-HE/$version/Saxon-HE-$version.jar"
    "xmlresolver-$resolver.jar"
    1fe4d5b92f708dcdb82dbce12919e0171e6b5ca62c6dca6220483625098feb5f
    "$maven/org/xmlresolver/xmlresolver/$resolver/xmlresolver-$resolver.jar"
    "xmlresolver-$resolver-data.jar"
    b0c487ad2f3e558be8d829c916d2458d10aca6a5bafa7a4d0524b70845e48a5c
    "$maven/org/xmlresolver/xmlresolver/$resolver/xmlresolver-$resolver-data.jar"
)

cd "$(git rev-parse --show-toplevel)"
dir=${1:-target/oracle/saxon}

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d' ' -f1
    else
        shasum -a 256 "$1" | cut -d' ' -f1
    fi
}

tmp=''
trap '[ -z "$tmp" ] || rm -f "$tmp"' EXIT

# fetch NAME SHA256 URL
fetch() {
    local jar="$dir/$1" sha256=$2 url=$3 actual
    if [ -e "$jar" ]; then
        actual=$(sha256_of "$jar")
        if [ "$actual" != "$sha256" ]; then
            echo "fetch-saxon: $jar has SHA-256 $actual, expected $sha256; delete it and run again" >&2
            return 1
        fi
        echo "fetch-saxon: $jar already present, checksum verified"
        return 0
    fi
    mkdir -p "$dir"
    tmp=$(mktemp "$dir/.download.XXXXXX")
    if ! curl --proto '=https' --tlsv1.2 -fsSL --retry 3 -o "$tmp" "$url"; then
        echo "fetch-saxon: cannot download $url" >&2
        exit 75
    fi
    actual=$(sha256_of "$tmp")
    if [ "$actual" != "$sha256" ]; then
        echo "fetch-saxon: download from $url has SHA-256 $actual, expected $sha256" >&2
        return 1
    fi
    mv "$tmp" "$jar"
    tmp=''
    echo "fetch-saxon: $jar downloaded, checksum verified"
}

for ((i = 0; i < ${#jars[@]}; i += 3)); do
    fetch "${jars[i]}" "${jars[i + 1]}" "${jars[i + 2]}"
done
