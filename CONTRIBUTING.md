# Contributing

Thank you for considering a contribution. Before you start, **read
[`AGENTS.md`](AGENTS.md)** — it codifies the non-negotiable rules
every change (human or AI) must respect.

## Quick path

1. Fork, branch off `main` (`feat/<slug>` or `fix/<slug>`).
2. Run the same gates CI runs before pushing:
   ```sh
   cargo fmt --all -- --check
   cargo clippy --all-targets --all-features -- -D warnings
   cargo test  --all-features
   cargo deny  check                # cargo install cargo-deny
   ```
3. Use [Conventional Commits](https://www.conventionalcommits.org/)
   (`feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `ci:`, `chore:`,
   `perf:`). One concern per commit, one concern per PR.
4. Update `CHANGELOG.md` under `## [Unreleased]`.
5. Open a PR; CI must be green before review.

## What the reviewer will check

The Definition of Done is in [`AGENTS.md §3`](AGENTS.md). In short:

- Builds clean on Linux / macOS / Windows.
- `fmt`, `clippy -D warnings`, `test`, `cargo deny` all pass.
- If user-facing: screenshots **in both dark and light mode**
  attached to the PR description, plus a description of the
  keyboard path through any new UI.
- If protocol-affecting: a real-world server reply added as a
  fixture to `tests/diagnostics.rs`.
- No `unwrap()` / `expect()` in non-test code without a `// SAFETY:`
  comment justifying it.
- No new dependency added without verifying it is the latest stable
  on crates.io (`cargo search <name>` or
  `curl -s https://crates.io/api/v1/crates/<name>`) and that
  `cargo deny check` accepts its licence.

## Reporting a security issue

Please do **not** open a public issue. Email the maintainer using the
address in the `Cargo.toml` `repository` page on crates.io, or open a
private security advisory on GitHub.
