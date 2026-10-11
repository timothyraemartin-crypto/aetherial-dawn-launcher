//! "No mods": before every launch the launcher looks through the Skyrim folder
//! and refuses to start the game while anything is installed that the server
//! did not give it. The allowed set is exactly the game itself, what SKSE and
//! SkyMP need, and the files in `client/manifest.json`. Anything else is a mod.
//!
//! What counts (see decisions.md for the reasons):
//! - plugins (`.esp/.esm/.esl`) in `Data` other than the server's base masters,
//!   the official Creation Club files Bethesda lists in `Skyrim.ccc`, and
//!   `_ResourcePack.esl`;
//! - archives (`.bsa/.ba2`) other than the game's own and Creation Club's;
//! - SKSE plugin DLLs other than SkyMP's (`MpClientPlugin`, `SkyrimPlatform`);
//! - loose files under `Data` (meshes, textures, sound, interface and so on)
//!   that are not in the SkyMP file list.
//!
//! `Data/Platform`, `Data/Scripts` (SKSE and SkyMP's Papyrus files) and
//! `Data/Video` are SkyMP's, SKSE's or the game's, and are not looked at.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    /// "plugin", "archive", "dll" or "files".
    pub kind: &'static str,
    /// The file's name, or for loose files the folder under Data.
    pub name: String,
    /// How many files (1 except for a folder of loose files).
    pub count: usize,
}

fn lower(p: &str) -> String {
    p.replace('\\', "/").to_ascii_lowercase()
}

/// Official Creation Club file names: `cc<author>sse<number>-<name>` (a few
/// use `_` after the number, such as `cckrtsse001_altar`).
fn is_cc(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    let Some(rest) = n.strip_prefix("cc") else { return false };
    let Some(pos) = rest.find("sse") else { return false };
    pos > 0
        && rest[..pos].bytes().all(|b| b.is_ascii_alphabetic())
        && rest[pos + 3..].bytes().take_while(u8::is_ascii_digit).count() >= 3
        && rest[pos + 3..].trim_start_matches(|c: char| c.is_ascii_digit()).starts_with(['-', '_'])
}

fn is_game_archive(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.starts_with("skyrim - ") || matches!(n.as_str(), "dawnguard.bsa" | "dragonborn.bsa" | "hearthfires.bsa" | "update.bsa" | "marketplacetextures.bsa" | "_resourcepack.bsa") || is_cc(&n)
}

fn ccc_names(game_dir: &Path) -> HashSet<String> {
    std::fs::read_to_string(game_dir.join("Skyrim.ccc")).map(|t| t.lines().map(|l| l.trim().to_ascii_lowercase()).filter(|l| !l.is_empty()).collect()).unwrap_or_default()
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else {
            out.push(p);
        }
    }
}

/// One extra file: what kind it is and where it sits (relative to the game folder, `/` separators).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extra {
    pub kind: &'static str,
    pub rel: String,
}

/// Every file in the game folder that is not the game, SKSE or SkyMP.
/// `masters` are the server's base plugin names; `manifest` the paths in its
/// client file list (relative to the game folder, e.g. `Data/Platform/...`).
pub fn extras(game_dir: &Path, masters: &[String], manifest: &[String]) -> Vec<Extra> {
    let data = game_dir.join("Data");
    let masters: HashSet<String> = masters.iter().map(|m| m.to_ascii_lowercase()).collect();
    let manifest: HashSet<String> = manifest.iter().map(|m| lower(m)).collect();
    let ccc = ccc_names(game_dir);
    let mut found = Vec::new();

    // Plugins and archives sit directly in Data.
    if let Ok(rd) = std::fs::read_dir(&data) {
        let mut names: Vec<String> = rd.flatten().filter(|e| e.path().is_file()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        names.sort_by_key(|n| n.to_ascii_lowercase());
        for name in names {
            let l = name.to_ascii_lowercase();
            if manifest.contains(&format!("data/{l}")) {
                continue;
            }
            let ext = l.rsplit('.').next().unwrap_or("");
            match ext {
                "esp" | "esm" | "esl" if !(masters.contains(&l) || ccc.contains(&l) || is_cc(&l) || l == "_resourcepack.esl") => {
                    found.push(Extra { kind: "plugin", rel: format!("Data/{name}") })
                }
                "bsa" | "ba2" if !is_game_archive(&l) => found.push(Extra { kind: "archive", rel: format!("Data/{name}") }),
                _ => {}
            }
        }
    }

    // SKSE plugin DLLs.
    let mut dlls = Vec::new();
    walk(&data.join("SKSE"), &mut dlls);
    let mut dll_names: Vec<String> = dlls
        .iter()
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("dll")))
        .filter_map(|p| p.strip_prefix(game_dir).ok().map(|r| r.to_string_lossy().replace('\\', "/")))
        .filter(|rel| {
            let base = rel.rsplit('/').next().unwrap_or("").to_ascii_lowercase();
            !(manifest.contains(&lower(rel)) || base == "mpclientplugin.dll" || base == "skyrimplatform.dll")
        })
        .collect();
    dll_names.sort_by_key(|n| n.to_ascii_lowercase());
    for rel in dll_names {
        found.push(Extra { kind: "dll", rel });
    }

    // Loose files under Data (not Platform, Scripts, Video, SKSE).
    let mut all = Vec::new();
    walk(&data, &mut all);
    all.sort();
    for p in all {
        let Ok(rel) = p.strip_prefix(&data) else { continue };
        let rel = rel.to_string_lossy().replace('\\', "/");
        let Some((top, _)) = rel.split_once('/') else { continue };
        let top_l = top.to_ascii_lowercase();
        if matches!(top_l.as_str(), "platform" | "scripts" | "video" | "skse" | "shadercache") {
            continue;
        }
        if rel.rsplit('/').next().is_some_and(|f| f.starts_with("__folder_managed_by_")) || manifest.contains(&format!("data/{}", rel.to_ascii_lowercase())) {
            continue;
        }
        found.push(Extra { kind: "files", rel: format!("Data/{rel}") });
    }
    found
}

/// What stands in the way, for the player: single files by name, loose files
/// as one entry per top folder under Data.
pub fn scan(game_dir: &Path, masters: &[String], manifest: &[String]) -> Vec<Finding> {
    let mut out = Vec::new();
    let mut loose: BTreeMap<String, usize> = BTreeMap::new();
    for e in extras(game_dir, masters, manifest) {
        if e.kind == "files" {
            let top = e.rel.split('/').nth(1).unwrap_or("").to_string();
            *loose.entry(top).or_default() += 1;
        } else {
            out.push(Finding { kind: e.kind, name: e.rel.rsplit('/').next().unwrap_or(&e.rel).to_string(), count: 1 });
        }
    }
    for (top, count) in loose {
        out.push(Finding { kind: "files", name: top, count });
    }
    out
}

/// One line for the player: what is in the way, at most `max` names.
pub fn describe(found: &[Finding], max: usize) -> String {
    let shown: Vec<String> = found
        .iter()
        .take(max)
        .map(|f| if f.kind == "files" { format!("{} ({} files)", f.name, f.count) } else { f.name.clone() })
        .collect();
    let more = found.len().saturating_sub(max);
    format!("{}{}", shown.join(", "), if more > 0 { format!(", and {more} more") } else { String::new() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(root: &Path, rel: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, b"x").unwrap();
    }

    fn masters() -> Vec<String> {
        ["Skyrim.esm", "Update.esm", "Dawnguard.esm", "HearthFires.esm", "Dragonborn.esm"].map(String::from).to_vec()
    }

    #[test]
    fn a_game_with_nothing_but_the_game_skse_and_skymp_is_clean() {
        let d = tempfile::tempdir().unwrap();
        for f in ["Data/Skyrim.esm", "Data/Update.esm", "Data/Dawnguard.esm", "Data/Skyrim - Textures0.bsa", "Data/ccBGSSSE001-Fish.esm", "Data/ccBGSSSE001-Fish.bsa", "Data/_ResourcePack.esl", "Data/MarketplaceTextures.bsa", "Data/SKSE/Plugins/MpClientPlugin.dll", "Data/SKSE/Plugins/SkyrimPlatform.dll", "Data/SKSE/Plugins/versionlib-1-6-1170-0.bin", "Data/Platform/Plugins/x.js", "Data/Scripts/Game.pex", "Data/Video/BGS_Logo.bik", "Data/Interface/CombatAlertOverlayMenu.swf", "Data/meshes/__folder_managed_by_vortex"] {
            touch(d.path(), f);
        }
        std::fs::write(d.path().join("Skyrim.ccc"), "ccBGSSSE001-Fish.esm\r\n").unwrap();
        let manifest = vec!["Data/Interface/CombatAlertOverlayMenu.swf".to_string()];
        assert_eq!(scan(d.path(), &masters(), &manifest), vec![]);
    }

    #[test]
    fn extra_plugins_archives_dlls_and_loose_files_are_found() {
        let d = tempfile::tempdir().unwrap();
        for f in ["Data/Skyrim.esm", "Data/SkyUI_SE.esp", "Data/Cool Mod.bsa", "Data/SKSE/Plugins/CrashLogger.dll", "Data/SKSE/Plugins/Sub/Other.dll", "Data/textures/a.dds", "Data/textures/b.dds", "Data/sound/x.wav"] {
            touch(d.path(), f);
        }
        let found = scan(d.path(), &masters(), &[]);
        let kinds: Vec<(&str, &str, usize)> = found.iter().map(|f| (f.kind, f.name.as_str(), f.count)).collect();
        assert_eq!(kinds, vec![("archive", "Cool Mod.bsa", 1), ("plugin", "SkyUI_SE.esp", 1), ("dll", "CrashLogger.dll", 1), ("dll", "Other.dll", 1), ("files", "sound", 1), ("files", "textures", 2)]);
        assert_eq!(describe(&found, 3), "Cool Mod.bsa, SkyUI_SE.esp, CrashLogger.dll, and 3 more");
    }

    #[test]
    fn a_file_the_server_lists_is_not_a_mod_and_names_match_in_any_case() {
        let d = tempfile::tempdir().unwrap();
        touch(d.path(), "Data/Custom/Thing.dds");
        touch(d.path(), "Data/skyrim.ESM");
        touch(d.path(), "Data/SKSE/Plugins/mpclientplugin.dll");
        let manifest = vec!["Data\\Custom\\thing.dds".to_string()];
        assert_eq!(scan(d.path(), &masters(), &manifest), vec![]);
    }

    #[test]
    fn creation_club_names_are_recognised_but_lookalikes_are_not() {
        assert!(is_cc("ccBGSSSE001-Fish.esm"));
        assert!(is_cc("ccqdrsse002-firewood.esl"));
        assert!(is_cc("ccVSVSSE004-BeAFarmer.esl"));
        assert!(is_cc("cckrtsse001_altar.bsa"));
        assert!(!is_cc("cc-nothing.esp"));
        assert!(!is_cc("CommunityOverlays1_0T30.esp"));
        assert!(!is_cc("ccsse1-x.esp"));
    }

    #[test]
    fn a_missing_data_folder_gives_nothing() {
        let d = tempfile::tempdir().unwrap();
        assert!(scan(d.path(), &masters(), &[]).is_empty());
    }
}
