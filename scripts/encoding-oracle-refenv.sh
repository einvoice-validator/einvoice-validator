#!/usr/bin/env bash
# Runs the encoding-oracle example of einvoice-xml in its reference environment,
# in a container: the only environment in which tests/encoding/expected.tsv may
# be rewritten (the example refuses --write in any other), and the one in which
# every result is checked. That environment is the runner of the oracle
# workflow, which the header of expected.tsv names: libxml2 2.9.14 built with
# iconv and ICU (`xmllint --version`: Iconv ICU ISO8859X), on Ubuntu 24.04, and
# a Java 21 runtime for Saxon-HE 12.10 (the example compares the version of the
# Java specification; the runner has Temurin 21, the image Ubuntu's OpenJDK 21).
#
#   scripts/encoding-oracle-refenv.sh            # check tests/encoding/expected.tsv
#   scripts/encoding-oracle-refenv.sh --write    # rewrite it
#
# Every argument is passed to the example. The container engine is
# $CONTAINER_ENGINE, else podman, else docker.
#
# The image (built once, then reused; its tag ends with a hash of its recipe):
#  * ubuntu:24.04, pinned by digest, for the amd64 platform of the runner (on
#    another architecture the engine must be able to run amd64 images);
#  * the packages below, each pinned by version, installed from the Ubuntu
#    archive as it was at the snapshot below (snapshot.ubuntu.com), where those
#    versions stay installable; apt authenticates every download with the
#    archive's signed InRelease, the first two commands before the image has a
#    CA store to check TLS with (as the image's own http sources run without
#    TLS), the others with it;
#  * rustup (rustup-init, SHA-256 checked) and the toolchain of
#    rust-toolchain.toml.
# The example is built in the container, with that toolchain, against the
# container's C library: a binary built on the host would need the host's
# glibc (newer than Ubuntu 24.04's on a recent system) or a host musl target,
# and is no Linux binary at all on macOS. The repository is mounted read-write
# at its own path, which the example's paths are compiled with, and the
# container runs as the calling user, so that the files it writes (the table,
# target/) are the user's. Cargo's registry cache and build directory are kept
# in target/refenv/, apart from the host's builds.
#
# Saxon-HE is fetched on the host first, by scripts/fetch-saxon.sh (checksums
# verified; the example checks them again). Needs network access the first
# time (image, crates, Saxon-HE). The image takes about 1.2 GB (the toolchain,
# gcc as the linker, the JRE); its first build takes minutes, mostly spent
# downloading from the snapshot server.
set -euo pipefail

ubuntu=docker.io/library/ubuntu:24.04@sha256:fe81560990fae86dc4743202b2cb4d3df98d0bcf949173cecef15ce1127ef852
snapshot=20261110T000000Z
ca_certificates=ca-certificates=20260816ubuntu0.24.04.2
packages=(
    libxml2=2.9.14+dfsg-1.3ubuntu3.10
    libxml2-utils=2.9.14+dfsg-1.3ubuntu3.10
    libicu74=74.2-1ubuntu3.1
    openjdk-21-jre-headless=21.0.13.0.3+1-1~24.04.1
    curl=8.5.0-2ubuntu10.16
    gcc=4:13.2.0-7ubuntu1
    libc6-dev=2.39-0ubuntu8.9
)
rustup=1.29.3
rustup_sha256=b1c8cb14e30889e7e2999e6b69e62d9e9b9ffc0f4df8180564d306d9680e6c55

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)
cd "$root"

channel=$(sed -n 's/^channel *= *"\([^"]*\)".*/\1/p' rust-toolchain.toml)
[ -n "$channel" ] || { echo "no channel in rust-toolchain.toml" >&2; exit 1; }

engine=${CONTAINER_ENGINE:-}
if [ -z "$engine" ]; then
    if command -v podman >/dev/null 2>&1; then
        engine=podman
    elif command -v docker >/dev/null 2>&1; then
        engine=docker
    else
        echo "neither podman nor docker is installed (or set CONTAINER_ENGINE)" >&2
        exit 1
    fi
fi
command -v "$engine" >/dev/null 2>&1 || { echo "$engine is not installed" >&2; exit 1; }

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum | cut -d' ' -f1
    else
        shasum -a 256 | cut -d' ' -f1
    fi
}

containerfile=$(cat <<EOF
FROM $ubuntu
ENV DEBIAN_FRONTEND=noninteractive
RUN echo 'APT::Snapshot "$snapshot";' >/etc/apt/apt.conf.d/50snapshot \\
 && apt-get -qq -o Acquire::Retries=5 -o Acquire::https::Verify-Peer=false update \\
 && apt-get -qq -o Acquire::Retries=5 -o Acquire::https::Verify-Peer=false install -y --no-install-recommends $ca_certificates >/dev/null \\
 && apt-get -qq -o Acquire::Retries=5 update \\
 && apt-get -qq -o Acquire::Retries=5 install -y --no-install-recommends ${packages[*]} >/dev/null \\
 && rm -rf /var/lib/apt/lists/*
ENV RUSTUP_HOME=/opt/rustup PATH=/opt/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
RUN curl --proto '=https' --tlsv1.2 --retry 5 -sSfo /tmp/rustup-init https://static.rust-lang.org/rustup/archive/$rustup/x86_64-unknown-linux-gnu/rustup-init \\
 && echo "$rustup_sha256  /tmp/rustup-init" | sha256sum -c --quiet - \\
 && chmod +x /tmp/rustup-init \\
 && CARGO_HOME=/opt/cargo /tmp/rustup-init -y -q --no-modify-path --profile minimal --default-toolchain none \\
 && CARGO_HOME=/opt/cargo rustup toolchain install --no-self-update $channel --profile minimal --component rustfmt,clippy \\
 && rm /tmp/rustup-init \\
 && chmod -R a+rX /opt/rustup /opt/cargo
EOF
)
image=einvoice-validator-refenv:$(printf '%s\n' "$containerfile" | sha256_of | cut -c1-16)

if ! "$engine" image inspect "$image" >/dev/null 2>&1; then
    echo "building the image $image with $engine"
    context=$(mktemp -d)
    trap 'rm -rf "$context"' EXIT
    printf '%s\n' "$containerfile" >"$context/Containerfile"
    "$engine" build --platform linux/amd64 -t "$image" -f "$context/Containerfile" "$context"
fi

scripts/fetch-saxon.sh

# The calling user in the container: rootless podman maps it with keep-id; a
# rootless docker daemon maps the container's root to it; otherwise the
# container runs with its ids.
user=(--user "$(id -u):$(id -g)")
if "$engine" --version 2>/dev/null | grep -qi podman; then
    if [ "$("$engine" info --format '{{.Host.Security.Rootless}}')" = true ]; then
        user=(--userns=keep-id)
    fi
elif "$engine" info --format '{{.SecurityOptions}}' 2>/dev/null | grep -q 'name=rootless'; then
    user=()
fi

mkdir -p target/refenv/cargo target/refenv/target
echo "running the encoding-oracle example of einvoice-xml in $image with $engine"
exec "$engine" run --rm --platform linux/amd64 --security-opt label=disable "${user[@]}" \
    -v "$root:$root" -w "$root" \
    -e HOME=/tmp \
    -e CARGO_HOME="$root/target/refenv/cargo" \
    -e CARGO_TARGET_DIR="$root/target/refenv/target" \
    "$image" \
    cargo run --locked -p einvoice-xml --example encoding-oracle -- "$@"
