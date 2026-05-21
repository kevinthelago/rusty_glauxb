/// Configure egui fonts for multi-script support.
///
/// Egui's built-in font (Ubuntu Light) covers Latin, Greek, and Cyrillic only.
/// This function loads system fonts as fallbacks so that Hindi/Devanagari,
/// Arabic, Hebrew, CJK, and other scripts render correctly in place names,
/// console output, and HUD labels.
///
/// Fonts are pushed in ascending priority order — each new font fills in
/// glyphs that earlier fonts lack, leaving existing glyphs unchanged.
pub fn configure(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    for entry in fallback_fonts() {
        if let Some(data) = load_first(&entry.paths) {
            let fd = egui::FontData {
                font:  std::borrow::Cow::Owned(data),
                index: entry.index,
                tweak: egui::FontTweak::default(),
            };
            fonts.font_data.insert(entry.name.clone(), fd);

            // Push to proportional family — checked in order, fills missing glyphs.
            if let Some(fam) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                fam.push(entry.name.clone());
            }
            // Same for monospace (used by the console and labels).
            if let Some(fam) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
                fam.push(entry.name.clone());
            }

            tracing::debug!(font = %entry.name, "loaded fallback font");
        } else {
            tracing::trace!(font = %entry.name, "fallback font not found (skipped)");
        }
    }

    ctx.set_fonts(fonts);
}

// ─── Internal ─────────────────────────────────────────────────────────────────

struct FontEntry {
    name:  String,
    /// Candidate paths tried in order; first readable file wins.
    paths: Vec<std::path::PathBuf>,
    /// Index within a TrueType Collection (.ttc).  0 for plain .ttf.
    index: u32,
}

fn fe(name: &str, paths: Vec<std::path::PathBuf>, index: u32) -> FontEntry {
    FontEntry { name: name.to_owned(), paths, index }
}

fn load_first(paths: &[std::path::PathBuf]) -> Option<Vec<u8>> {
    paths.iter().find_map(|p| std::fs::read(p).ok())
}

/// Returns the list of fallback fonts to attempt, ordered from lowest to
/// highest priority (last entry wins for any glyph it covers).
fn fallback_fonts() -> Vec<FontEntry> {
    // ── Platform-specific system font directories ──────────────────────────

    #[cfg(target_os = "windows")]
    let sys = std::path::PathBuf::from("C:/Windows/Fonts");

    #[cfg(target_os = "macos")]
    let sys = std::path::PathBuf::from("/Library/Fonts");

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let sys = std::path::PathBuf::from("/usr/share/fonts");

    // User font directory (cross-platform): ~/fonts or platform-standard.
    let user_fonts: Vec<std::path::PathBuf> = [
        dirs::home_dir().map(|h| h.join("fonts")),
        dirs::data_local_dir().map(|d| d.join("fonts")),
        #[cfg(target_os = "linux")]
        dirs::home_dir().map(|h| h.join(".fonts")),
    ]
    .into_iter()
    .flatten()
    .collect();

    // Helper: build candidate list from system dir + user dirs.
    let candidates = |names: &[&str]| -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        for name in names {
            out.push(sys.join(name));
            for ud in &user_fonts {
                out.push(ud.join(name));
            }
        }
        out
    };

    vec![
        // ── Arabic, Hebrew, Thai, Vietnamese ──────────────────────────────
        // Arial ships on Windows and macOS and covers a very broad range of
        // scripts including Arabic, Hebrew, Greek, Cyrillic, and Thai.
        fe("arial", candidates(&[
            "arial.ttf",
            "Arial.ttf",
            "Arial Unicode MS.ttf",
            "ArialUnicodeMS.ttf",
        ]), 0),

        // ── Indian subcontinent scripts ───────────────────────────────────
        // Nirmala UI: covers Devanagari (Hindi, Marathi, Sanskrit, Nepali),
        // Bengali, Gujarati, Gurmukhi, Kannada, Malayalam, Odia, Sinhala,
        // Tamil, Telugu — the most comprehensive Indian-script font on Windows.
        fe("nirmala", candidates(&[
            "Nirmala.ttc",
            "nirmala.ttf",
            "NirmalaUI-Reg.ttf",
            // Linux / Noto equivalents
            "NotoSansDevanagari-Regular.ttf",
            "NotoSansDevanagari-VF.ttf",
            // macOS
            "ITFDevanagari.ttf",
        ]), 0),

        // ── African scripts (Ebrima: N'Ko, Tifinagh, Vai, etc.) ──────────
        fe("ebrima", candidates(&[
            "ebrima.ttf",
            "Ebrima.ttf",
        ]), 0),

        // ── CJK: Chinese (Simplified + Traditional), Japanese, Korean ─────
        // msyh = Microsoft YaHei (Simplified Chinese / Japanese)
        fe("cjk_simplified", candidates(&[
            "msyh.ttc",
            "Msyh.ttc",
            "NotoSansCJK-Regular.ttc",
            "NotoSansSC-Regular.otf",
            // macOS
            "PingFang.ttc",
            // Linux
            "WenQuanYiMicroHei.ttf",
        ]), 0),

        // msjh = Microsoft JhengHei (Traditional Chinese)
        fe("cjk_traditional", candidates(&[
            "msjh.ttc",
            "Msjh.ttc",
            "NotoSansTC-Regular.otf",
        ]), 0),

        // Noto Sans JP covers Japanese kana + kanji (already on this system).
        fe("noto_jp", candidates(&[
            "NotoSansJP-VF.ttf",
            "NotoSansJP-Regular.otf",
        ]), 0),

        // ── Korean ───────────────────────────────────────────────────────
        fe("korean", candidates(&[
            "malgun.ttf",
            "Malgun.ttf",
            "NotoSansKR-Regular.otf",
        ]), 0),
    ]
}
