//! What decides whether Skyrim draws its textures: the game's own archives and
//! the few ini values that govern texture, LOD and streaming. Read-only. The
//! session report prints it so a "missing textures" or "pop-in" report from a
//! player shows the cause (a missing or empty base archive, a loose-file
//! override, an odd streaming value) without asking for files.
//!
//! Only a fixed list of numeric/boolean setting names is printed, never paths
//! or free text from the ini files.

use std::path::Path;

/// The base game's archives in Data (SkyrimSE 1.6.x). A missing or empty one
/// means missing meshes, textures, animations or UI.
pub const BASE_ARCHIVES: [&str; 20] = [
    "Skyrim - Animations.bsa",
    "Skyrim - Interface.bsa",
    "Skyrim - Meshes0.bsa",
    "Skyrim - Meshes1.bsa",
    "Skyrim - Misc.bsa",
    "Skyrim - Patch.bsa",
    "Skyrim - Shaders.bsa",
    "Skyrim - Sounds.bsa",
    "Skyrim - Textures0.bsa",
    "Skyrim - Textures1.bsa",
    "Skyrim - Textures2.bsa",
    "Skyrim - Textures3.bsa",
    "Skyrim - Textures4.bsa",
    "Skyrim - Textures5.bsa",
    "Skyrim - Textures6.bsa",
    "Skyrim - Textures7.bsa",
    "Skyrim - Textures8.bsa",
    "Update.bsa",
    "Dawnguard.bsa",
    "Dragonborn.bsa",
];

/// Settings that change how textures, LODs and loaded objects stream in.
const KEYS: [&str; 22] = [
    "iTexMipMapSkip",
    "iMaxAnisotropy",
    "iTextureUpgradeDistance0",
    "iTextureUpgradeDistance1",
    "iTextureUpgradeDistance2",
    "iShadowMapResolution",
    "fShadowDistance",
    "fPostLoadUpdateTimeMS",
    "iPreloadSizeLimit",
    "uGridsToLoad",
    "uExterior Cell Buffer",
    "fMeshLODFadePercentDefault",
    "fLODFadeOutMultObjects",
    "fLODFadeOutMultItems",
    "fLODFadeOutMultActors",
    "fSkyCellRefFadeDistance",
    "bInvalidateOlderFiles",
    "bUseTAA",
    "bFXAAEnabled",
    "iMultiSample",
    "iPresentInterval",
    "bBorderless",
];

/// Base archives that are missing or empty in `data_dir`, in list order.
pub fn missing_archives(data_dir: &Path) -> Vec<String> {
    BASE_ARCHIVES
        .iter()
        .filter(|name| std::fs::metadata(data_dir.join(name)).map(|m| m.len() == 0).unwrap_or(true))
        .map(|n| n.to_string())
        .collect()
}

/// `section.key=value` for each listed setting found in the ini text. Values
/// that are not short numbers/booleans are skipped.
pub fn settings(text: &str) -> Vec<String> {
    let mut section = String::new();
    let mut out = Vec::new();
    for line in text.lines() {
        let l = line.trim();
        if let Some(s) = l.strip_prefix('[').and_then(|s| s.split(']').next()) {
            section = s.to_string();
        } else if let Some((k, v)) = l.split_once('=') {
            let (k, v) = (k.trim(), v.trim());
            let plain = !v.is_empty() && v.len() <= 12 && v.chars().all(|c| c.is_ascii_digit() || c == '.' || c == '-');
            if plain && KEYS.iter().any(|w| w.eq_ignore_ascii_case(k)) {
                out.push(format!("[{section}] {k}={v}"));
            }
        }
    }
    out
}

/// Counts the files below `dir`, stopping at `cap`.
fn count(dir: &Path, cap: usize) -> usize {
    let mut n = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_symlink() {
                continue;
            }
            if ft.is_dir() {
                stack.push(e.path());
            } else {
                n += 1;
                if n >= cap {
                    return n;
                }
            }
        }
    }
    n
}

/// The report block. `documents` is the Documents folder (My Games\...).
pub fn report(game_dir: &Path, documents: Option<&Path>) -> String {
    let data = game_dir.join("Data");
    let mut o = String::from("Texture check:\n");
    let missing = missing_archives(&data);
    if missing.is_empty() {
        o.push_str(&format!("  all {} base game archives are there\n", BASE_ARCHIVES.len()));
    } else {
        o.push_str(&format!("  MISSING OR EMPTY base archives: {}\n", missing.join(", ")));
    }
    for folder in ["textures", "meshes"] {
        let n = count(&data.join(folder), 5000);
        if n > 0 {
            o.push_str(&format!("  loose files in Data/{folder}: {}{}\n", n, if n >= 5000 { "+" } else { "" }));
        }
    }
    if let Some(docs) = documents {
        for ini in crate::gameini::ini_paths(docs) {
            let Ok(text) = std::fs::read_to_string(&ini) else { continue };
            let name = ini.file_name().unwrap().to_string_lossy().into_owned();
            let found = settings(&text);
            if !found.is_empty() {
                o.push_str(&format!("  {name}: {}\n", found.join("; ")));
            }
        }
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_or_empty_base_archive_is_named() {
        let d = tempfile::tempdir().unwrap();
        for n in BASE_ARCHIVES {
            std::fs::write(d.path().join(n), b"x").unwrap();
        }
        assert!(missing_archives(d.path()).is_empty());
        std::fs::remove_file(d.path().join("Skyrim - Textures3.bsa")).unwrap();
        std::fs::write(d.path().join("Skyrim - Meshes1.bsa"), b"").unwrap();
        assert_eq!(missing_archives(d.path()), vec!["Skyrim - Meshes1.bsa", "Skyrim - Textures3.bsa"]);
    }

    #[test]
    fn only_listed_plain_values_are_printed() {
        let t = "[General]\r\nfPostLoadUpdateTimeMS=500\r\nsLanguage=ENGLISH\r\n[Display]\r\nfShadowDistance=10000.0000\r\niSize H=1080\r\nbUseTAA=1\r\n[Archive]\r\nbInvalidateOlderFiles=C:\\secret\\path\r\n";
        assert_eq!(settings(t), vec!["[General] fPostLoadUpdateTimeMS=500", "[Display] fShadowDistance=10000.0000", "[Display] bUseTAA=1"]);
    }

    #[test]
    fn the_report_shows_missing_archives_loose_textures_and_settings() {
        let g = tempfile::tempdir().unwrap();
        let data = g.path().join("Data");
        std::fs::create_dir_all(data.join("textures").join("actors")).unwrap();
        std::fs::write(data.join("textures").join("actors").join("a.dds"), b"x").unwrap();
        let docs = tempfile::tempdir().unwrap();
        let my = docs.path().join("My Games").join("Skyrim Special Edition");
        std::fs::create_dir_all(&my).unwrap();
        std::fs::write(my.join("Skyrim.ini"), "[General]\nfPostLoadUpdateTimeMS=500\n").unwrap();
        let r = report(g.path(), Some(docs.path()));
        assert!(r.contains("MISSING OR EMPTY base archives: Skyrim - Animations.bsa"), "{r}");
        assert!(r.contains("loose files in Data/textures: 1"), "{r}");
        assert!(r.contains("Skyrim.ini: [General] fPostLoadUpdateTimeMS=500"), "{r}");
    }
}
