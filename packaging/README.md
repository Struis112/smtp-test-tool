# `packaging/` - Package-manager manifests

Static manifest templates for the three package managers we ship to:

| Manager  | Platform | Manifest         | User command                                                 |
|----------|----------|------------------|--------------------------------------------------------------|
| WinGet   | Windows  | `winget/*.yaml`  | `winget install Struis112.SmtpTestTool`                      |
| Scoop    | Windows  | `scoop/*.json`   | `scoop bucket add struis112 https://github.com/Struis112/scoop-bucket && scoop install smtp-test-tool` |
| Homebrew | mac/Linux| `homebrew/*.rb`  | `brew tap struis112/tap && brew install smtp-test-tool`      |

## How they stay in sync

The `Update package manifests` job in `.github/workflows/release.yml`
runs after the release artifacts are uploaded.  It:

1. Downloads the published `.sha256` sidecar files from the GitHub
   release.
2. Substitutes the new version number and hashes into each manifest
   template under `packaging/`.
3. Commits the rewritten files back to `main` with a
   `chore(packaging): refresh manifests for vX.Y.Z` message (skipping
   CI to avoid loops).

Manifests are therefore **always correct for the latest tagged
release** without anyone manually editing them.

## Publishing to the actual registries

The manifests live in this repo as the source of truth, but each
registry needs them mirrored to a different location:

### WinGet

PR the three YAML files to
[`microsoft/winget-pkgs`](https://github.com/microsoft/winget-pkgs)
under `manifests/s/Struis112/SmtpTestTool/<version>/`.  The Microsoft
validation bot will run, an `[Approved]` label appears, and the
package is live within a few hours.

A `wingetcreate submit` one-liner is the recommended path; the
[`fivetran/winget-create-action`](https://github.com/fivetran/winget-create-action)
GitHub Action wraps it.  We do **not** auto-PR (cross-repo PATs are
risky for a hobby project); the maintainer copies the YAMLs and
submits the PR manually after each release.

### Scoop

Mirror `scoop/smtp-test-tool.json` to the
[`Struis112/scoop-bucket`](https://github.com/Struis112/scoop-bucket)
repo, under `bucket/smtp-test-tool.json`.  Users add the bucket once
and `scoop update smtp-test-tool` picks up new versions
automatically.

### Homebrew

Mirror `homebrew/smtp-test-tool.rb` to
[`Struis112/homebrew-tap`](https://github.com/Struis112/homebrew-tap)
under `Formula/smtp-test-tool.rb`.  Users add the tap once and
`brew upgrade smtp-test-tool` picks up new versions automatically.

## Why three managers and not just `cargo install`?

* **Discoverability.**  `winget search smtp` should find us.
* **No Rust toolchain needed.**  Most IT staff who use this tool will
  not have `cargo` installed.
* **Free.**  All three managers cost zero per AGENTS.md \u00a72.

## Why not snap / flatpak / AUR?

Open to PRs.  We left them out of v0.1.6 because:

* Snap requires Canonical account + snapcraft.io publishing setup.
* Flatpak needs a Flathub manifest PR with a longer review queue.
* AUR is community-maintained; anyone can publish a PKGBUILD pointing
  at our GitHub release tarball.  We will not maintain it ourselves
  to keep the package surface small.
