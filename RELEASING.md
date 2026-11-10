# Releasing

## Versioning policy

- Versions follow [Semantic Versioning 2.0.0](https://semver.org/spec/v2.0.0.html).
- One version for the whole workspace: it is set once, in `[workspace.package]` of the root
  `Cargo.toml`. Every crate inherits it (`version.workspace = true`), and the crates depend on
  each other by path at that same version. All crates are released together.
- The version stays at `0.x` until the public API is stable. Within `0.x`, a minor version
  (`0.2.0` after `0.1.3`) may break the API; a patch version (`0.1.4`) does not.
- Each release states which versions of the official rule sets it was tested against: the
  release script writes them at the top of the release's changelog section, from the rule sets
  in use in `upstream.toml`.
- Pre-releases (`0.2.0-rc.1`) are published on GitHub as pre-releases.

## Changelog

Every change that users can notice adds an entry under `## [Unreleased]` in
[CHANGELOG.md](CHANGELOG.md) ([Keep a Changelog 1.1](https://keepachangelog.com/en/1.1.0/):
`Added`, `Changed`, `Deprecated`, `Removed`, `Fixed`, `Security`). On pull requests, CI fails
when files other than documentation (`*.md`, `docs/`, `LICENSE`) and tests (`tests/`
directories) change while that section does not. For a change users cannot notice, a
maintainer adds the label `no-changelog`.

## Making a release

From an up-to-date clone with signing configured (`git config user.signingkey`, and
`gpg.format` for SSH keys):

```sh
scripts/release.sh --dry-run 0.1.0      # every check, then the changes that would be made
scripts/release.sh 0.1.0                # the same, then a signed commit and a signed tag v0.1.0
git push --atomic origin main v0.1.0    # printed by the script, which never pushes
```

`scripts/release.sh` refuses to run unless:

- `HEAD` is the branch `main`, identical to `origin/main` after a fetch;
- the working tree is clean, untracked files included;
- the version is valid SemVer, without `v`, and greater than the current one;
- the tag does not exist, locally or on `origin`;
- the `Unreleased` section of `CHANGELOG.md` has entries;
- every pin of `upstream.toml` matches the repository (`scripts/freshness.sh pins`);
- `scripts/check.sh` passes.

It then sets the workspace version (root `Cargo.toml`, the path dependencies between crates,
`Cargo.lock`, and the `version` of the npm package, `crates/einvoice-wasm/npm/package.json`:
every tracked `package.json`), moves the `Unreleased` entries into a new section `## [X.Y.Z] - YYYY-MM-DD`
headed by the rule-set versions, updates the compare links at the bottom of the changelog,
commits `Release X.Y.Z` (signed) and creates the signed annotated tag `vX.Y.Z`, whose message
holds the release notes. The changes are prepared and checked in a temporary worktree first:
a failure leaves the working tree untouched.

Pushing the tag starts [`.github/workflows/release.yml`](.github/workflows/release.yml). It
checks that the tag, the workspace version and the changelog section agree and that the
tagged commit is on `main` (`scripts/release.sh --verify vX.Y.Z`), publishes the crates on
crates.io (below), then publishes the GitHub release with that changelog section as its notes.
If the verification fails, fix the cause on `main` and release the next patch version; a pushed
tag is never moved. If the publication on crates.io fails (a rate limit, a new crate, an outage),
fix that and re-run the failed jobs of the run: the versions already on crates.io are skipped.

The release scripts are tested in throw-away repositories by `scripts/test-release.sh` (run in
CI by the `workflows` job of `ci.yml`).

## Publishing on crates.io

The release workflow publishes the crates, with crates.io trusted publishing
([RFC 3691](https://rust-lang.github.io/rfcs/3691-trusted-publishing-cratesio.html)): no API
token is stored anywhere. Its job `crates-io` runs in the GitHub environment `release`, which
admits the tags `v*` only, and is the only job allowed an OpenID Connect token; the action
`rust-lang/crates-io-auth-action` exchanges it for a crates.io token that expires after 30
minutes (and is revoked at the end of the job). crates.io accepts that token only for the
crates configured to trust this repository, the workflow `release.yml` and the environment
`release`.

The job runs `scripts/release.sh --publish vX.Y.Z` on the tagged commit. It checks that the
tag and the workspace version agree, reads the crates.io index, skips every crate already
published at that version, and publishes the others with `cargo publish --locked --workspace`
(the crates already published left out with `--exclude`): every crate but those with
`publish = false`, in dependency order. A refusal by crates.io's rate limits (HTTP 429) is
retried after the time crates.io gives, else after 60 s doubled at each refusal (at most 600 s),
10 attempts at most and only within 25 minutes of the start (`RELEASE_PUBLISH_DEADLINE`, in
seconds), the token living 30; any other failure stops the job. Re-running the job resumes
where it stopped.

The crates not published are `xtask` (repository tooling), `einvoice-testkit` (helpers of the
tests; the crates depend on it by path without a version, which cargo leaves out of their
published manifests) and `einvoice-wasm` (the source of the npm package, published on npm
below).

### A new crate

Trusted publishing only publishes new versions of crates that exist: crates.io cannot be
configured for a crate before its first version is published. So `--publish` refuses a
publishable crate that is not on crates.io yet, before publishing anything, and names it
(`einvoice-pdf` and `einvoice-xml`, added after 0.2.0, are the first such crates). For a
release that adds a crate, its owner publishes its first version from the tagged commit with
their own token (`cargo login`, a token with the scopes `publish-new` and
`trusted-publishing`), then configures trusted publishing for it; this can be done before
pushing the tag (once `scripts/release.sh` has created it), or after the workflow has refused,
followed by a re-run of the failed jobs:

```sh
git checkout vX.Y.Z
scripts/release.sh --publish vX.Y.Z NEW-CRATE...      # cargo publish --locked -p NEW-CRATE..., same retries
scripts/release.sh --trusted-publishing NEW-CRATE...  # trust release.yml in the environment release, listed back
```

Name with the new crate every workspace crate it depends on that is not published at that
version yet. `--trusted-publishing` without a crate checks every publishable crate: it keeps
an identical configuration and adds a missing one through the crates.io API
(`/api/v1/trusted_publishing/github_configs`), then lists each crate's configurations back.
New crates are subject to their own, stricter rate limit on crates.io; the retries above apply.

### Order of the crates

`cargo publish --workspace` packages and verifies every crate before uploading any, then
publishes them in dependency order and waits for each to be available before publishing those
that depend on it (dev-dependencies included). It uploads them in this order, which also works
one at a time (`cargo publish --locked -p NAME`):
`einvoice-bigint`, `einvoice-pdf`, `einvoice-svrl`, `einvoice-xml`, `einvoice-xpath`,
`einvoice-xdm`, `einvoice-xsd`, `einvoice-xslt`, `einvoice-schematron`, `einvoice-xpath-eval`,
`einvoice-xslt-exec`, `einvoice-validator`, `einvoice-cli`. `einvoice-pdf` must be published
before `einvoice-validator`, which depends on it (feature `pdf`); `einvoice-bigint` and
`einvoice-xml` depend on no other crate of the workspace. A published version cannot be
replaced: a mistake is fixed by the next patch version (and the faulty version yanked).

## Publishing on npm

The npm package `einvoice-validator` is built from `crates/einvoice-wasm` at the version of the
workspace (`scripts/release.sh` sets the `version` of `crates/einvoice-wasm/npm/package.json`
with the crates'). From the tagged commit, with the pinned tools:

```sh
crates/einvoice-wasm/npm.sh tools               # wasm-bindgen-cli and wasm-opt, pinned
crates/einvoice-wasm/npm.sh build test pack     # target/npm/einvoice-validator/, tested
(cd target/npm/einvoice-validator && npm publish)
```
