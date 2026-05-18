# AGENTS.md — Working agreement for AI coding agents on this repo

> **Loaded automatically by Claude Code, Cursor, Aider, and most other
> agent harnesses.** Every contributor (human or AI) MUST read this file
> before changing code. Violations are merge-blockers.

---

## 1. Ground rules (hard requirements)

1. **Quality over quantity.** One feature done well beats three half-baked
   features. If you cannot finish something to the standard below in the
   current session, leave it out and open a tracking issue instead.

2. **Verify "latest" against live sources.** Before adding or upgrading any
   dependency, language version, GitHub Action, or framework, confirm the
   current stable release from an authoritative source:
   - Rust crates: `cargo search <crate> --limit 1` or
     `curl -s https://crates.io/api/v1/crates/<crate> | jq -r .crate.max_stable_version`
   - GitHub Actions: check the action's repo `Releases` page (or
     `gh release view --repo owner/repo --json tagName`).
   - Rust toolchain: `https://forge.rust-lang.org/infra/channel-layout.html`
     or `rustup check`.
   - Do **not** trust prior agent memory for version numbers.

3. **Accessibility is the bare minimum, not a stretch goal.** Every UI
   surface (desktop GUI, web pages, generated docs site, CLI output) MUST
   meet **WCAG 2.2 Level AAA** at a minimum:
   - Text contrast ≥ 7:1 against its background (≥ 4.5:1 for large text).
   - All information conveyed by colour MUST also have a textual cue
     (`[PASS]`, `[FAIL]`, icons with labels, etc.). Colour is never the
     only signal.
   - Full keyboard operability with a visible focus indicator.
   - No content flashes more than 3× per second.
   - Form fields have visible, programmatic labels (not placeholder-only).
   - Live regions / status messages announced to assistive tech (egui ⇒
     AccessKit, web ⇒ `aria-live`).
   - Honour `prefers-reduced-motion` and `prefers-contrast`.

4. **Dark + light mode, on every OS, always.** Every UI MUST detect and
   follow the operating-system appearance setting (Windows registry,
   macOS `AppleInterfaceStyle`, GNOME/KDE/Cosmic, web
   `prefers-color-scheme`). A manual override MUST also be available, and
   the chosen theme MUST persist between sessions.

5. **No shortcuts, even if they look like overkill.** Hand-rolled JSON
   parser when serde exists? No. Single-file 3000-line module to "save
   time"? No. The right tool, modular code, real tests, real error
   handling. If a solution feels too clever, it is wrong.

6. **Commit early, commit often, atomic commits.** Every logically
   independent change is its own commit with a [Conventional Commits]
   message (`feat:`, `fix:`, `chore:`, `docs:`, `refactor:`, `test:`,
   `ci:`, `perf:`). Never bundle unrelated changes. This is what lets us
   `git revert` cleanly when something breaks. Push to a feature branch,
   open a PR, let CI run; merge only when green.

7. **Polish counts.** GUI spacing, web typography, CLI output alignment,
   error message wording — all of it is part of the product. If it looks
   amateur, it is broken.

[Conventional Commits]: https://www.conventionalcommits.org/

---

## 2. Stack of record (so agents don't churn it)

| Layer            | Choice                | Why                                            |
| ---------------- | --------------------- | ---------------------------------------------- |
| Language         | Rust (edition 2021)   | Safety, single static binary, modern tooling.  |
| MSRV             | 1.75                  | Enables let-else, async fn in traits.          |
| TLS              | `rustls` + ring       | Pure Rust, no OpenSSL on host.                 |
| SMTP             | `lettre` 0.11+        | De-facto Rust SMTP client.                     |
| IMAP / POP3      | hand-rolled on rustls | Owns the wire trace for diagnostics.           |
| CLI parsing      | `clap` 4 derive       | Standard.                                      |
| Config           | `serde` + `toml`      | Human-editable, IT-friendly.                   |
| Logging          | `tracing` family      | One subscriber, many sinks (CLI, GUI, file).   |
| Desktop GUI      | `eframe`/`egui`       | Single binary, AccessKit, OS theme follow.     |
| Web (if needed)  | not yet decided       | When added: must meet rule #3 from day one.    |

Before changing any of the above, open an issue with rationale; never
silently swap.

---

## 3. Definition of Done for any change

A pull request is **only** ready to merge when **all** of these are true:

- [ ] Builds clean on Linux + macOS + Windows in CI.
- [ ] `cargo fmt --all -- --check` passes.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo test --all-features` passes.
- [ ] `cargo deny check` passes (advisories, licenses, sources, bans).
- [ ] If user-facing: screenshots in dark **and** light mode attached to
      the PR, plus a paragraph describing the keyboard path through the
      new UI.
- [ ] If protocol-affecting: example real-world server diagnostic added
      to `tests/diagnostics.rs`.
- [ ] `CHANGELOG.md` updated under `## [Unreleased]`.
- [ ] No `unwrap()` / `expect()` in non-test code without a
      `// SAFETY:`-style comment justifying it.
- [ ] No new dependency added without verifying it is the latest stable
      (rule #2) and that `cargo deny` accepts its licence.

---

## 4. Commit / branch workflow

```
main          ← protected, always green, always shippable
└── feat/x    ← short-lived branches, squash-merge via PR
```

- One PR = one concern.
- Commit message body explains *why*, not *what* (the diff shows what).
- Reference issues with `Refs #N` or `Closes #N`.
- Tag releases with `vX.Y.Z`; CI then builds and publishes binaries +
  the crate to crates.io.

---

## 5. When you (an AI agent) are blocked

- Do not invent API surfaces. Read the actual crate docs (`cargo doc
  --open` or docs.rs) before guessing.
- If a build fails, paste the **exact** error in your reply and fix
  the smallest possible thing first — do not refactor under cover of a
  bug fix.
- If you broke something, `git status` and `git diff` before doing
  anything else. If unsure, `git stash` and ask the user.
- Tell the user the truth, including "I can't verify X right now
  because Y". Do not bluff.
