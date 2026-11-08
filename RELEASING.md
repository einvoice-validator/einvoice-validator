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
`Cargo.lock`), moves the `Unreleased` entries into a new section `## [X.Y.Z] - YYYY-MM-DD`
headed by the rule-set versions, updates the compare links at the bottom of the changelog,
commits `Release X.Y.Z` (signed) and creates the signed annotated tag `vX.Y.Z`, whose message
holds the release notes. The changes are prepared and checked in a temporary worktree first:
a failure leaves the working tree untouched.

Pushing the tag starts [`.github/workflows/release.yml`](.github/workflows/release.yml). It
checks that the tag, the workspace version and the changelog section agree and that the
tagged commit is on `main` (`scripts/release.sh --verify vX.Y.Z`), then publishes the GitHub
release with that changelog section as its notes. If the workflow fails, fix the cause on
`main` and release the next patch version; a pushed tag is never moved.

The release scripts are tested in throw-away repositories by `scripts/test-release.sh` (run in
CI by the `workflows` job of `ci.yml`).

## Publishing on crates.io

The release workflow does not publish on crates.io. Once the tag is pushed and the workflow
has passed, a maintainer with a crates.io token publishes every crate of the release from the
tagged commit:

```sh
git checkout vX.Y.Z
cargo publish --locked --workspace      # every crate but xtask (publish = false), in dependency order
```

`cargo publish --workspace` publishes the crates in dependency order and waits for each to be
available before publishing those that depend on it (dev-dependencies included). It
uploads them in this order, which also works one at a time (`cargo publish --locked -p NAME`):
`einvoice-svrl`, `einvoice-xdm`, `einvoice-xpath`, `einvoice-xsd`, `einvoice-xpath-eval`,
`einvoice-xslt`, `einvoice-xslt-exec`, `einvoice-validator`, `einvoice-cli`. A published
version cannot be replaced: a mistake is fixed by the next patch version (and the faulty
version yanked).

Publishing from the release workflow with crates.io trusted publishing
([RFC 3691](https://rust-lang.github.io/rfcs/3691-trusted-publishing-cratesio.html)), with a
short-lived token obtained through OpenID Connect instead of a long-lived API token, is not
set up yet.
