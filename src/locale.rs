//! OS-level locale detection.
//!
//! Wraps `sys-locale` so the rest of the crate sees a single function
//! that always returns a normalised two-letter language code (or `None`
//! when the OS could not advertise one — rare; usually a container
//! without `LANG` set).
//!
//! Region tags (`_NL`, `-NL`) and codeset suffixes (`.UTF-8`) are
//! stripped because our translation files are keyed by language only.
//! A future expansion to region variants (`pt-BR` vs `pt-PT`) would
//! return the unmodified BCP-47 string instead.

/// Detect the OS's preferred user locale.
///
/// Returns `Some("nl")`, `Some("en")`, … on success.  Returns `None`
/// if `sys-locale` could not read any platform-specific source
/// (Windows `GetUserDefaultLocaleName`, macOS `CFLocaleCopyCurrent`,
/// or Unix `$LC_ALL` / `$LC_MESSAGES` / `$LANG`).
pub fn detect() -> Option<String> {
    sys_locale::get_locale().map(|raw| normalise(&raw))
}

/// Strip region and codeset from a raw locale string.
fn normalise(code: &str) -> String {
    code.split(['_', '-', '.'])
        .next()
        .unwrap_or(code)
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalise_handles_common_forms() {
        assert_eq!(normalise("nl_NL.UTF-8"), "nl");
        assert_eq!(normalise("nl-NL"), "nl");
        assert_eq!(normalise("NL"), "nl");
        assert_eq!(normalise("en"), "en");
        assert_eq!(normalise("pt_BR"), "pt");
        assert_eq!(normalise("zh-CN"), "zh");
        assert_eq!(normalise(""), "");
    }

    #[test]
    fn detect_returns_two_letter_code_or_none() {
        // We cannot assert WHAT detect() returns - it depends on the
        // CI runner's locale.  But whatever it returns, the length
        // must be 0 or 2-3 ASCII lowercase letters, never a leak of
        // region/codeset.
        if let Some(c) = detect() {
            assert!(c.chars().all(|ch| ch.is_ascii_lowercase()));
            assert!(c.len() <= 3, "detect() returned non-language code: {c:?}");
        }
    }
}
