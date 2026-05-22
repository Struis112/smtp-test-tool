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

## Translations

The tool ships translations for 25 languages under
`locales/<bcp47>.toml`.  As of v0.1.3 the set is:

* `en` — base, hand-maintained by the maintainer.  Every key the
  code looks up MUST exist here.
* `nl` — native-quality (the maintainer is a Dutch speaker).
* `bg`, `cs`, `da`, `de`, `el`, `es`, `fi`, `fr`, `hr`, `hu`, `id`,
  `it`, `no`, `pl`, `pt`, `ro`, `ru`, `sk`, `sr`, `sv`, `tr`, `uk`,
  `vi` — **machine-translated**, native review welcome.  Each
  file's `locale.status_note` says so in that file's own language.

### Reviewing a machine-translated file

1. Pick a file from the list above whose `status_note` is non-empty.
2. Read it side-by-side with `locales/en.toml` (same key order).
3. Focus on:
   * Natural phrasing (don't translate word-for-word from English).
   * Consistency with how the language's *own* tech press talks
     about email — Microsoft and Google ship localised admin
     panels; mirror their terminology where it makes sense.
   * Keep technical tokens in English: `SMTP AUTH`, `STARTTLS`,
     `XOAUTH2`, `MAIL FROM`, `App Password`, `Conditional Access`.
     That matches what users see in M365 admin and helps IT triage.
4. Open a PR that **also clears `locale.status_note`** if you can
   attest to native-quality.  Mention which strings you reviewed
   in the PR body.

### Adding a brand-new language

1. Copy `locales/en.toml` to `locales/<your-code>.toml`.
2. Translate the right-hand sides; keep the section structure and
   key names identical to en.toml.
3. Add an `include_str!` + an entry to `LOCALES` in
   `src/i18n.rs` (alphabetical-ish by code, doesn't matter for
   correctness).
4. Set a sensible `locale.native_name` (in your language) and
   `locale.english_name`.  Leave `locale.status_note` saying
   "machine-translated, native review welcome" if you used an LLM,
   or empty if you can attest to native quality.
5. `cargo build --all-features && cargo test --all-features` — the
   lazy init panics on invalid TOML, so the build is the first
   check.
6. Open a PR.

The **language selector** in the GUI (Advanced tab) only ever shows
the user's OS locale + English.  Adding more locales does NOT clutter
the UI for users who don't speak those languages — they only see
theirs.

## Reporting a security issue

Please do **not** open a public issue. Email the maintainer using the
address in the `Cargo.toml` `repository` page on crates.io, or open a
private security advisory on GitHub.
