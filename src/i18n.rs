//! Hand-rolled translation lookup.
//!
//! Loads `locales/<code>.toml` files at compile time via [`include_str!`]
//! and exposes a [`t`] function for runtime lookups by dotted key.
//!
//! ## Design choices
//!
//! * **No external i18n crate.** `rust-i18n` and `fluent` would each
//!   bring macro magic and several extra dependencies; the lookup we
//!   need is a single `HashMap::get`. Keeping this hand-rolled also
//!   keeps the failure mode honest: missing keys fall back to English
//!   first, then to the literal key string, never to a panic.
//!
//! * **TOML, not YAML.** We already depend on `toml` for the config
//!   file, so there's nothing extra to pull in.  Translators get
//!   per-section grouping and inline comments for free.
//!
//! * **Compile-time embed.** Every shipped locale is baked into the
//!   binary; no external files to find at runtime, no missing-locale
//!   surprise on a stripped install.
//!
//! ## Key naming
//!
//! Dotted, lowercase, snake-cased.  Two top-level namespaces today:
//!
//! * `diagnostics.*` — server-reply hint table contents.
//! * `ui.*` — labels, buttons, tab names, tooltips, prompts.
//!
//! Codes with dots in them (`5.7.60`) are written as `5_7_60` to
//! avoid colliding with TOML's section delimiter.

use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::sync::RwLock;

/// Marker for the always-available, base-language fallback.  Every key
/// MUST exist under this code; lookups for any other locale fall back
/// here when a key is missing.
pub const BASE: &str = "en";

// ---- shipped locale files (compile-time embedded) -----------------------
// Add a new line here AND a matching entry in `LOCALES` below when adding
// a translation.  The build will fail on missing file, which is the
// behaviour we want - a stray entry in LOCALES with no source file would
// be a quieter footgun.
// Native-quality (hand-maintained or native-reviewed):
const EN_TOML: &str = include_str!("../locales/en.toml");
const NL_TOML: &str = include_str!("../locales/nl.toml");
// Machine-translated (each file's locale.status_note documents this):
const BG_TOML: &str = include_str!("../locales/bg.toml");
const CS_TOML: &str = include_str!("../locales/cs.toml");
const DA_TOML: &str = include_str!("../locales/da.toml");
const DE_TOML: &str = include_str!("../locales/de.toml");
const EL_TOML: &str = include_str!("../locales/el.toml");
const ES_TOML: &str = include_str!("../locales/es.toml");
const FI_TOML: &str = include_str!("../locales/fi.toml");
const FR_TOML: &str = include_str!("../locales/fr.toml");
const HR_TOML: &str = include_str!("../locales/hr.toml");
const HU_TOML: &str = include_str!("../locales/hu.toml");
const ID_TOML: &str = include_str!("../locales/id.toml");
const IT_TOML: &str = include_str!("../locales/it.toml");
const NO_TOML: &str = include_str!("../locales/no.toml");
const PL_TOML: &str = include_str!("../locales/pl.toml");
const PT_TOML: &str = include_str!("../locales/pt.toml");
const RO_TOML: &str = include_str!("../locales/ro.toml");
const RU_TOML: &str = include_str!("../locales/ru.toml");
const SK_TOML: &str = include_str!("../locales/sk.toml");
const SR_TOML: &str = include_str!("../locales/sr.toml");
const SV_TOML: &str = include_str!("../locales/sv.toml");
const TR_TOML: &str = include_str!("../locales/tr.toml");
const UK_TOML: &str = include_str!("../locales/uk.toml");
const VI_TOML: &str = include_str!("../locales/vi.toml");

/// Codes of every shipped locale.  Order = alphabetical except `en` first
/// (it is the base / fallback target).  Wired by [`available_locales`].
const LOCALES: &[(&str, &str)] = &[
    ("en", EN_TOML),
    ("bg", BG_TOML),
    ("cs", CS_TOML),
    ("da", DA_TOML),
    ("de", DE_TOML),
    ("el", EL_TOML),
    ("es", ES_TOML),
    ("fi", FI_TOML),
    ("fr", FR_TOML),
    ("hr", HR_TOML),
    ("hu", HU_TOML),
    ("id", ID_TOML),
    ("it", IT_TOML),
    ("nl", NL_TOML),
    ("no", NO_TOML),
    ("pl", PL_TOML),
    ("pt", PT_TOML),
    ("ro", RO_TOML),
    ("ru", RU_TOML),
    ("sk", SK_TOML),
    ("sr", SR_TOML),
    ("sv", SV_TOML),
    ("tr", TR_TOML),
    ("uk", UK_TOML),
    ("vi", VI_TOML),
];

// ---- runtime tables ------------------------------------------------------
type FlatTable = HashMap<String, String>;

static TABLES: Lazy<HashMap<&'static str, FlatTable>> = Lazy::new(|| {
    let mut out: HashMap<&'static str, FlatTable> = HashMap::new();
    for (code, src) in LOCALES {
        let parsed: toml::Value =
            toml::from_str(src).unwrap_or_else(|e| panic!("locale '{code}' has invalid TOML: {e}"));
        let mut flat = FlatTable::new();
        flatten("", &parsed, &mut flat);
        out.insert(*code, flat);
    }
    out
});

static CURRENT: Lazy<RwLock<String>> = Lazy::new(|| RwLock::new(BASE.into()));

/// Recursively flatten a TOML [`Value`] into `dotted.key -> string`
/// entries.  Non-string leaves are silently skipped; tables are
/// descended into; arrays are ignored (we don't need them).
fn flatten(prefix: &str, v: &toml::Value, out: &mut FlatTable) {
    match v {
        toml::Value::String(s) => {
            out.insert(prefix.to_string(), s.clone());
        }
        toml::Value::Table(t) => {
            for (k, v) in t {
                let next = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                flatten(&next, v, out);
            }
        }
        _ => { /* ignore */ }
    }
}

/// Set the active locale.  Pass a 2- or 5-letter code (`"nl"`,
/// `"pt-br"`); unsupported codes silently fall back to [`BASE`] so the
/// app cannot end up with no language at all.
pub fn set_locale(code: &str) {
    let normalised = normalise(code);
    let chosen = if TABLES.contains_key(normalised.as_str()) {
        normalised
    } else {
        BASE.to_string()
    };
    if let Ok(mut w) = CURRENT.write() {
        *w = chosen;
    }
}

/// Return the active locale's BCP-47-ish code (e.g. `"en"`, `"nl"`).
pub fn current_locale() -> String {
    CURRENT
        .read()
        .ok()
        .map(|g| g.clone())
        .unwrap_or_else(|| BASE.to_string())
}

/// Sorted list of every locale code shipped with this binary.
pub fn available_locales() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = TABLES.keys().copied().collect();
    v.sort();
    v
}

/// True iff this binary ships a translation table for `code`
/// (after the same normalisation [`set_locale`] applies).
pub fn is_supported(code: &str) -> bool {
    TABLES.contains_key(normalise(code).as_str())
}

/// Return the human-readable name of `code` as written in `code`'s OWN
/// language (e.g. `native_name("nl")` -> `"Nederlands"`).  Reads the
/// `locale.native_name` key from the requested locale's table
/// regardless of the currently-active locale; useful for building a
/// language selector that shows every option in its own script.
/// Falls back to the bare code when no translation ships.
pub fn native_name(code: &str) -> String {
    let n = normalise(code);
    TABLES
        .get(n.as_str())
        .and_then(|m| m.get("locale.native_name"))
        .cloned()
        .unwrap_or_else(|| code.to_string())
}

/// Look up `key` in the active locale.  Falls back to [`BASE`], then
/// to the literal `key` string, so the GUI never displays a bare empty
/// string for an unknown key (a missing translation surfaces visibly
/// as the dotted key name, which is easier to diagnose than blank UI).
pub fn t(key: &str) -> String {
    let locale = current_locale();
    if let Some(table) = TABLES.get(locale.as_str()) {
        if let Some(s) = table.get(key) {
            return s.clone();
        }
    }
    if locale != BASE {
        if let Some(table) = TABLES.get(BASE) {
            if let Some(s) = table.get(key) {
                return s.clone();
            }
        }
    }
    key.to_string()
}

/// Same as [`t`] but performs `{placeholder}` substitution on the
/// resulting string.  `args` is treated as a list of `(name, value)`
/// pairs.  Unknown placeholders stay literal so a translator typo
/// surfaces in the UI rather than getting swallowed.
pub fn t_with(key: &str, args: &[(&str, &str)]) -> String {
    let mut s = t(key);
    for (name, value) in args {
        s = s.replace(&format!("{{{name}}}"), value);
    }
    s
}

/// Normalise `"nl_NL.UTF-8"`, `"NL-nl"`, `"nl"` → `"nl"`.
/// Two-letter result is matched against [`LOCALES`].
fn normalise(code: &str) -> String {
    code.split(['_', '-', '.'])
        .next()
        .unwrap_or(code)
        .to_lowercase()
}

// =========================================================================
// Test-only helpers
// =========================================================================
// `set_locale` mutates global state (the active locale).  Cargo runs tests
// in parallel by default, so any two tests that each set a locale and read
// strings can interleave and see each other's locale.  Solution: a
// process-wide mutex that tests acquire before touching the locale.  The
// mutex itself is exposed via `LocaleTestGuard` so a test only needs to
// write `let _g = LocaleTestGuard::set("nl");`.
//
// Non-locale tests are unaffected and continue to run in parallel.
#[cfg(test)]
pub(crate) mod testing {
    use super::{set_locale, BASE};
    use std::sync::{Mutex, MutexGuard, OnceLock};

    fn lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    /// Serialises every test that touches the active locale.  Hold the
    /// returned guard for the duration of the assertions; on drop the
    /// active locale is reset to [`BASE`] and the mutex is released.
    pub struct LocaleTestGuard {
        _lock: MutexGuard<'static, ()>,
    }

    impl LocaleTestGuard {
        /// Acquire the lock, then switch the active locale to `code`.
        /// Poisoned mutexes are recovered transparently - a panicking
        /// test must not deadlock every other locale-dependent test.
        pub fn set(code: &str) -> Self {
            let lock = lock().lock().unwrap_or_else(|p| p.into_inner());
            set_locale(code);
            Self { _lock: lock }
        }
    }

    impl Drop for LocaleTestGuard {
        fn drop(&mut self) {
            set_locale(BASE);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use super::testing::LocaleTestGuard;

    #[test]
    fn base_locale_is_always_supported() {
        assert!(is_supported(BASE));
        assert!(available_locales().contains(&BASE));
    }

    #[test]
    fn normalise_strips_region_and_codeset() {
        assert_eq!(normalise("nl_NL.UTF-8"), "nl");
        assert_eq!(normalise("NL-nl"), "nl");
        assert_eq!(normalise("EN"), "en");
        assert_eq!(normalise("zh-CN"), "zh");
    }

    #[test]
    fn unsupported_locale_falls_back_to_base() {
        let _g = LocaleTestGuard::set("klingon");
        assert_eq!(current_locale(), BASE);
    }

    #[test]
    fn t_falls_back_to_base_when_key_missing_in_active_locale() {
        // 'app.name' is intentionally defined ONLY in en.toml; nl.toml
        // omits it on purpose so this test exercises the fallback.
        let _g = LocaleTestGuard::set("nl");
        let s = t("app.name");
        assert!(
            !s.is_empty() && s != "app.name",
            "expected English fallback for missing nl key, got: {s:?}"
        );
    }

    #[test]
    fn t_returns_literal_key_for_unknown_anywhere() {
        let _g = LocaleTestGuard::set(BASE);
        let s = t("this.key.does.not.exist.anywhere");
        assert_eq!(s, "this.key.does.not.exist.anywhere");
    }

    #[test]
    fn t_with_substitutes_placeholders() {
        // Synthetic test; en.toml ships 'test.fixture.greeting' = "Hello, {name}!".
        let _g = LocaleTestGuard::set("en");
        let s = t_with("test.fixture.greeting", &[("name", "world")]);
        assert_eq!(s, "Hello, world!");
    }
}
