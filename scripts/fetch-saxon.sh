#!/usr/bin/env bash
# Downloads Saxon-HE, the reference engine of the oracle (`cargo xtask
# oracle`), from Maven Central and verifies its SHA-256. Saxon-HE is MPL-2.0
# and is not part of this repository: it is fetched into target/oracle/saxon/
# (or the directory given as first argument), outside the tracked tree.
#
#   scripts/fetch-saxon.sh            # target/oracle/saxon/Saxon-HE-9.9.1-8.jar
#   scripts/fetch-saxon.sh DIR        # DIR/Saxon-HE-9.9.1-8.jar
#
# An existing jar is verified, never trusted: a checksum mismatch, of a fresh
# download or of a cached file, is an error and nothing is overwritten.
# Keep the version and checksum in step with xtask/src/oracle.rs.
set -euo pipefail

version=9.9.1-8
sha256=504ad5c71b55a3fd04c5609419b5e51d51b419f26628b4b6254c38766487abdf
url="https://repo1.maven.org/maven2/net/sf/saxon/Saxon-HE/$version/Saxon-HE-$version.jar"

cd "$(git rev-parse --show-toplevel)"
dir=${1:-target/oracle/saxon}
jar="$dir/Saxon-HE-$version.jar"

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d' ' -f1
    else
        shasum -a 256 "$1" | cut -d' ' -f1
    fi
}

if [ -e "$jar" ]; then
    actual=$(sha256_of "$jar")
    if [ "$actual" != "$sha256" ]; then
        echo "fetch-saxon: $jar has SHA-256 $actual, expected $sha256; delete it and run again" >&2
        exit 1
    fi
    echo "fetch-saxon: $jar already present, checksum verified"
    exit 0
fi

mkdir -p "$dir"
tmp=$(mktemp "$dir/.download.XXXXXX")
trap 'rm -f "$tmp"' EXIT
curl --proto '=https' --tlsv1.2 -fsSL --retry 3 -o "$tmp" "$url"
actual=$(sha256_of "$tmp")
if [ "$actual" != "$sha256" ]; then
    echo "fetch-saxon: download from $url has SHA-256 $actual, expected $sha256" >&2
    exit 1
fi
mv "$tmp" "$jar"
trap - EXIT
echo "fetch-saxon: $jar downloaded, checksum verified"
