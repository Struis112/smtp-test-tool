//! OS-native font discovery for non-Latin scripts.
//!
//! eframe's bundled fonts cover Latin + Cyrillic + Greek only.  CJK
//! (zh / ja / ko), Arabic, Devanagari, Thai, and a few other scripts
//! render as tofu (`■■`) by default.
//!
//! The 2026 industry-standard fix is to add a script-specific font from
//! the user's OS to egui's per-glyph fallback chain:
//!
//! * `egui::FontDefinitions` carries a `families: BTreeMap<FontFamily,
//!   Vec<String>>`; each `Vec` is the fallback order for that family.
//!   On a glyph miss, egui walks the list and uses the first font whose
//!   coverage includes the code point.
//! * We query [`fontdb`] for the well-known OS-typical family name per
//!   script (e.g. "Yu Gothic UI" on Windows, "Hiragino Sans" on macOS,
//!   "Noto Sans CJK JP" on Linux), read the file bytes, and append
//!   that font to both Proportional and Monospace fallback chains.
//! * Latin glyphs keep coming from the bundled fonts; non-Latin
//!   glyphs come from the appended OS fonts.  No 30 MB of Noto
//!   shipped in our binary.
//!
//! When [`load_for_locale`] runs against a locale that needs no extra
//! fonts (en / nl / de / ... / ru / el), it short-circuits and never
//! touches the OS - no startup penalty for the 99% case.
//!
//! Limitations to revisit:
//! * Arabic / Persian / Hebrew need bidirectional text shaping which
//!   egui only partially supports today.  Strings will render with the
//!   right glyphs but layout may not perfectly match RTL conventions.
//! * Some Linux containers ship no CJK / Arabic fonts at all; the
//!   user must install Noto manually.  We log a warning when nothing
//!   matched the locale's needs.

use eframe::egui;
use std::sync::Arc;

/// Per-locale list of OS-typical font family names to try, in
/// descending priority order.  The first match (any OS) wins; if
/// none of the listed names is present, the caller logs a warning
/// and the GUI falls back to tofu for missing glyphs.
fn candidates_for_locale(code: &str) -> &'static [&'static str] {
    // BCP-47-normalised codes only - i18n::normalise has already run.
    match code {
        // Chinese (Simplified).
        "zh" | "zh-cn" => &[
            "Microsoft YaHei UI", // Windows
            "Microsoft YaHei",
            "PingFang SC",      // macOS
            "Noto Sans CJK SC", // Linux Noto
            "Source Han Sans SC",
            "WenQuanYi Zen Hei",
        ],
        // Chinese (Traditional).
        "zh-tw" | "zh-hk" => &[
            "Microsoft JhengHei UI",
            "Microsoft JhengHei",
            "PingFang TC",
            "Noto Sans CJK TC",
            "Source Han Sans TC",
        ],
        // Japanese.
        "ja" => &[
            "Yu Gothic UI",
            "Yu Gothic",
            "Meiryo UI",
            "Meiryo",
            "Hiragino Sans",
            "Hiragino Kaku Gothic ProN",
            "Noto Sans CJK JP",
            "IPAexGothic",
        ],
        // Korean.
        "ko" => &[
            "Malgun Gothic",
            "Apple SD Gothic Neo",
            "AppleGothic",
            "Noto Sans CJK KR",
            "NanumGothic",
        ],
        // Arabic, Persian, Urdu (all use Arabic script).
        "ar" | "fa" | "ur" => &[
            "Segoe UI", // covers Arabic on Windows 10+
            "Tahoma",
            "Geeza Pro", // macOS
            "Damascus",
            "Noto Sans Arabic", // Linux Noto
            "Noto Naskh Arabic",
            "Amiri",
        ],
        // Hebrew.
        "he" | "iw" => &["Segoe UI", "Arial Hebrew", "Noto Sans Hebrew"],
        // Devanagari (Hindi, Marathi).
        "hi" | "mr" => &[
            "Mangal",
            "Nirmala UI",
            "Kohinoor Devanagari",
            "Devanagari Sangam MN",
            "Noto Sans Devanagari",
        ],
        // Bengali.
        "bn" => &[
            "Vrinda",
            "Nirmala UI",
            "Bangla Sangam MN",
            "Noto Sans Bengali",
        ],
        // Tamil.
        "ta" => &["Latha", "Nirmala UI", "Tamil Sangam MN", "Noto Sans Tamil"],
        // Telugu.
        "te" => &[
            "Gautami",
            "Nirmala UI",
            "Telugu Sangam MN",
            "Noto Sans Telugu",
        ],
        // Thai.
        "th" => &[
            "Leelawadee UI",
            "Leelawadee",
            "Thonburi",
            "Noto Sans Thai",
            "Garuda",
        ],
        // Latin / Cyrillic / Greek covered by bundled fonts already.
        _ => &[],
    }
}

/// Augment the egui font tables with OS-installed fonts for the given
/// locale, in place.  Returns `Ok(n)` with the number of fonts added
/// to the fallback chain; `Ok(0)` means either the locale needs no
/// extra fonts, or none of the candidates were installed.
pub fn load_for_locale(ctx: &egui::Context, code: &str) -> Result<usize, String> {
    let candidates = candidates_for_locale(&code.to_lowercase());
    if candidates.is_empty() {
        return Ok(0);
    }

    let mut db = fontdb::Database::new();
    db.load_system_fonts();

    let mut defs = egui::FontDefinitions::default();
    let mut added = Vec::new();

    for &family_name in candidates {
        let query = fontdb::Query {
            families: &[fontdb::Family::Name(family_name)],
            ..fontdb::Query::default()
        };
        let id = match db.query(&query) {
            Some(id) => id,
            None => continue,
        };
        let bytes_opt: Option<Vec<u8>> = db.with_face_data(id, |data, _index| data.to_vec());
        let bytes = match bytes_opt {
            Some(b) => b,
            None => continue,
        };

        let egui_name = format!("os:{family_name}");
        defs.font_data.insert(
            egui_name.clone(),
            Arc::new(egui::FontData::from_owned(bytes)),
        );
        // Append - egui walks the family list in order, so bundled
        // fonts keep first dibs on Latin glyphs.
        defs.families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .push(egui_name.clone());
        defs.families
            .entry(egui::FontFamily::Monospace)
            .or_default()
            .push(egui_name.clone());
        added.push(family_name);
    }

    if added.is_empty() {
        tracing::warn!(
            "no OS font found for locale '{code}'; non-Latin glyphs will render as tofu \
             (expected candidates: {candidates:?})"
        );
    } else {
        tracing::info!(
            "loaded {} OS font(s) for locale '{code}': {:?}",
            added.len(),
            added
        );
        ctx.set_fonts(defs);
    }
    Ok(added.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latin_locales_short_circuit_with_no_candidates() {
        for code in ["en", "nl", "de", "fr", "es", "ru", "el", "uk", "sv"] {
            assert!(
                candidates_for_locale(code).is_empty(),
                "{code} should not need extra fonts"
            );
        }
    }

    #[test]
    fn cjk_locales_have_per_os_candidates() {
        for code in ["zh", "zh-cn", "zh-tw", "ja", "ko"] {
            assert!(
                !candidates_for_locale(code).is_empty(),
                "{code} needs CJK fonts"
            );
        }
    }

    #[test]
    fn arabic_family_locales_share_candidates() {
        // ar, fa, ur all use the Arabic script -> same font candidates.
        let ar = candidates_for_locale("ar");
        assert_eq!(ar, candidates_for_locale("fa"));
        assert_eq!(ar, candidates_for_locale("ur"));
    }
}
