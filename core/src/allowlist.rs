//! "Server's mods only" (Timothy, 2026-09-26: "disable anything that's not
//! its mod list"). Works out which files belong to mods on the list, so
//! tidying never touches them, and which files other mods put in Data
//! through Vortex, so they can be set aside before Play.
//!
//! A file belongs to a listed mod when the list names it (`check`), the
//! launcher installed it (`installed.json`), or Vortex deployed it from a mod
//! whose Nexus id is on the list or among the required mods. Loose files that
//! no tool recorded can't be traced to a mod, so they're left alone.

use std::collections::HashSet;
use std::path::Path;

use crate::manifest::Manifest;
use crate::modlist::{self, ModEntry, ModList};

/// Nexus ids of required mods that the launcher installs from GitHub but
/// players may have installed through Vortex instead: Skyrim Souls RE,
/// Crash Logger, Address Library.
const REQUIRED_NEXUS_IDS: [u64; 3] = [27859, 59818, 32444];

fn saved_list_path(game_dir: &Path) -> std::path::PathBuf {
    game_dir.join(modlist::MODS_DIR).join("server-list.json")
}

/// Keeps the server's mods.json next to the game, so tidying (which runs
/// without the network) knows the whole list.
pub fn save_server_list(game_dir: &Path, list: &ModList) {
    let p = saved_list_path(game_dir);
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if let Ok(b) = serde_json::to_vec_pretty(list) {
        let _ = std::fs::write(p, b);
    }
}

/// The built-in list plus the last server list seen.
pub fn listed(game_dir: &Path) -> Vec<ModEntry> {
    let server: Option<ModList> = std::fs::read(saved_list_path(game_dir)).ok().and_then(|b| serde_json::from_slice(&b).ok());
    modlist::merged(None, server.as_ref())
}

#[derive(Debug, Clone, PartialEq)]
pub struct VortexFile {
    /// Game-relative, "Data/..." with forward slashes.
    pub rel: String,
    /// Vortex's mod folder name, e.g. "SKSE Menu Framework-120352-3-18-1725000000".
    pub source: String,
}

/// The files Vortex deployed into Data, from its vortex.deployment.json.
pub fn vortex_files(game_dir: &Path) -> Vec<VortexFile> {
    let Ok(b) = std::fs::read(game_dir.join("Data").join("vortex.deployment.json")) else { return Vec::new() };
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(&b) else { return Vec::new() };
    v.get("files")
        .and_then(|f| f.as_array())
        .map(|files| {
            files
                .iter()
                .filter_map(|f| {
                    let rel = f.get("relPath")?.as_str()?.replace('\\', "/");
                    let source = f.get("source")?.as_str()?.to_string();
                    (!rel.is_empty() && !rel.contains("..")).then(|| VortexFile { rel: format!("Data/{}", rel.trim_start_matches('/')), source })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Names of required mods as they appear in Vortex's mod folder names, for
/// folders that don't carry the Nexus id (a manual install).
const REQUIRED_NAMES: [&str; 12] = [
    "skse64",
    "skyrim script extender",
    "address library",
    "engine fixes",
    "unofficial skyrim special edition patch",
    "skse menu framework",
    "imgui icons",
    "skyui",
    "skyrim souls",
    "crash logger",
    "display tweaks",
    "black screen",
];

fn listed_source(source: &str, ids: &[u64]) -> bool {
    let s = source.to_ascii_lowercase();
    // Vortex names mod folders "<name>-<nexus id>-<version>-<time>", or with
    // spaces ("SKSE Menu Framework 120352 3.18 ..."), so any number in the
    // name that equals a listed id counts. A wrong match only keeps a file.
    let numbers: Vec<&str> = s.split(|c: char| !c.is_ascii_digit()).filter(|t| !t.is_empty()).collect();
    ids.iter().any(|id| numbers.contains(&id.to_string().as_str())) || REQUIRED_NAMES.iter().any(|n| s.contains(n))
}

/// Files of required mods by name, however they were installed: the
/// framework's settings, fonts and themes next to its DLL, the patch's
/// loose files, and so on. The game can crash at start when a DLL stays and
/// these go (2026-09-26: SKSE Menu Framework without its fonts).
pub fn required_file(rel: &str) -> bool {
    let l = rel.replace('\\', "/").to_ascii_lowercase();
    let plugins = "data/skse/plugins/";
    if let Some(n) = l.strip_prefix(plugins) {
        return ["sksemenuframework", "fonts/", "enginefixes", "skyrimsoulsre", "crashlogger", "version-", "versionlib-", "ssedisplaytweaks"].iter().any(|p| n.starts_with(p));
    }
    let Some(n) = l.strip_prefix("data/") else { return false };
    let patch = "unofficial skyrim special edition patch";
    n.starts_with("skyui_se.")
        || n.starts_with(patch)
        || n.starts_with(&format!("bashtags/{patch}"))
        || n.starts_with(&format!("docs/{patch}"))
        || n.starts_with("interface/imguiicons/")
}

/// Vortex mod folders that must stay whole: listed ones, and any that
/// deployed a required file.
fn kept_sources(files: &[VortexFile], ids: &[u64], keep: &HashSet<String>) -> HashSet<String> {
    files
        .iter()
        .filter(|f| listed_source(&f.source, ids) || required_file(&f.rel) || keep.contains(&f.rel.to_ascii_lowercase()))
        .map(|f| f.source.clone())
        .collect()
}

fn ids(list: &[ModEntry]) -> Vec<u64> {
    let mut v: Vec<u64> = list.iter().filter_map(|m| m.nexus.as_ref().map(|n| n.mod_id)).collect();
    v.extend(REQUIRED_NEXUS_IDS);
    v
}

/// Lower-case game-relative paths ("data/skse/plugins/x.dll") of files that
/// belong to listed or required mods.
pub fn keep_set(game_dir: &Path) -> HashSet<String> {
    let list = listed(game_dir);
    let mut keep: HashSet<String> = HashSet::new();
    for m in &list {
        keep.extend(m.check.iter().map(|c| c.replace('\\', "/").to_ascii_lowercase()));
    }
    let installed = modlist::load_installed(game_dir);
    for m in &list {
        if let Some(rec) = installed.mods.get(&m.id) {
            keep.extend(rec.files.iter().chain(rec.skipped.iter()).map(|f| f.to_ascii_lowercase()));
        }
    }
    let ids = ids(&list);
    let files = vortex_files(game_dir);
    let sources = kept_sources(&files, &ids, &keep);
    for f in files {
        if sources.contains(&f.source) || required_file(&f.rel) {
            keep.insert(f.rel.to_ascii_lowercase());
        }
    }
    keep
}

/// Top-level plugin names ("unofficial skyrim special edition patch.esp")
/// among the kept files, for the load order.
pub fn kept_plugins(keep: &HashSet<String>) -> Vec<String> {
    keep.iter()
        .filter_map(|p| p.strip_prefix("data/"))
        .filter(|n| !n.contains('/') && (n.ends_with(".esp") || n.ends_with(".esm") || n.ends_with(".esl")))
        .map(str::to_string)
        .collect()
}

/// Files other mods deployed through Vortex that aren't on the list and
/// aren't the server's own files. Plugins and their archives are left to
/// the load order (they're switched off there instead).
pub fn unlisted_vortex_files(game_dir: &Path, manifest: &Manifest) -> Vec<String> {
    unlisted_with(game_dir, |rel| manifest.files.iter().any(|m| m.path.eq_ignore_ascii_case(rel)))
}

fn unlisted_with(game_dir: &Path, server_file: impl Fn(&str) -> bool) -> Vec<String> {
    let keep = keep_set(game_dir);
    let files = vortex_files(game_dir);
    // A mod is set aside whole or not at all: if any of its files stays
    // (kept, a plugin, a server file), all of them stay.
    let mut staying: HashSet<String> = HashSet::new();
    for f in &files {
        let l = f.rel.to_ascii_lowercase();
        let plugin_or_archive = [".esp", ".esm", ".esl", ".bsa"].iter().any(|x| l.ends_with(x));
        if keep.contains(&l) || plugin_or_archive || server_file(&f.rel) || required_file(&f.rel) {
            staying.insert(f.source.clone());
        }
    }
    let mut out: Vec<String> = files
        .into_iter()
        .filter(|f| !staying.contains(&f.source))
        .map(|f| f.rel)
        .filter(|rel| game_dir.join(rel).is_file())
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Puts back files an earlier "Only the server's mods" sweep set aside that
/// belong to mods it must keep (0.1.38 moved SKSE Menu Framework's fonts and
/// settings and the Unofficial Patch's loose files). Returns them.
pub fn restore_kept(game_dir: &Path) -> std::io::Result<Vec<String>> {
    let root = game_dir.join(crate::strays::DISABLED_DIR);
    let Ok(rd) = std::fs::read_dir(&root) else { return Ok(Vec::new()) };
    let keep = keep_set(game_dir);
    let mut stamps: Vec<std::path::PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir() && p.to_string_lossy().ends_with("-other-mods")).collect();
    stamps.sort();
    stamps.reverse();
    let mut back = Vec::new();
    for stamp in stamps {
        let mut stack = vec![stamp.clone()];
        while let Some(d) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&d) else { continue };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                    continue;
                }
                let Ok(rel) = p.strip_prefix(&stamp) else { continue };
                let rel_s = rel.to_string_lossy().replace('\\', "/");
                if !(keep.contains(&rel_s.to_ascii_lowercase()) || required_file(&rel_s)) {
                    continue;
                }
                let to = game_dir.join(rel);
                if to.exists() {
                    continue;
                }
                if let Some(parent) = to.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::rename(&p, &to)?;
                back.push(rel_s);
            }
        }
    }
    back.sort();
    Ok(back)
}

/// Puts everything the launcher set aside back where it was, unless
/// something else is there now. Returns how many files went back.
pub fn restore_all(game_dir: &Path) -> std::io::Result<usize> {
    let root = game_dir.join(crate::strays::DISABLED_DIR);
    let Ok(rd) = std::fs::read_dir(&root) else { return Ok(0) };
    let mut n = 0;
    let mut stamps: Vec<std::path::PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    // Newest first, so the latest copy of a file wins.
    stamps.sort();
    stamps.reverse();
    for stamp in stamps {
        let mut stack = vec![stamp.clone()];
        while let Some(d) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&d) else { continue };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                    continue;
                }
                let Ok(rel) = p.strip_prefix(&stamp) else { continue };
                let to = game_dir.join(rel);
                if to.exists() {
                    continue;
                }
                if let Some(parent) = to.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::rename(&p, &to)?;
                n += 1;
            }
        }
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_listed_vortex_mods_and_finds_others() {
        let t = tempfile::tempdir().unwrap();
        let g = t.path();
        let data = g.join("Data");
        for p in ["SKSE/Plugins/SKSEMenuFramework.dll", "SKSE/Plugins/SKSEMenuFramework/fonts/a.ttf", "Scripts/uimenubase.pex", "Meshes/armor/x.nif", "SKSE/Plugins/Other.dll", "Other.esp"] {
            let f = data.join(p);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, b"x").unwrap();
        }
        let dep = serde_json::json!({"files": [
            {"relPath": "SKSE\\Plugins\\SKSEMenuFramework.dll", "source": "SKSE Menu Framework-120352-3-18-1725000000"},
            {"relPath": "SKSE\\Plugins\\SKSEMenuFramework\\fonts\\a.ttf", "source": "SKSE Menu Framework-120352-3-18-1725000000"},
            {"relPath": "Scripts\\uimenubase.pex", "source": "Skyrim Souls RE-27859-2-4-0-1700000000"},
            {"relPath": "Meshes\\armor\\x.nif", "source": "Pretty Armor-99999-1-0-1700000000"},
            {"relPath": "SKSE\\Plugins\\Other.dll", "source": "Other-88888-1-0-1700000000"},
            {"relPath": "Other.esp", "source": "Other-88888-1-0-1700000000"}
        ]});
        std::fs::write(data.join("vortex.deployment.json"), serde_json::to_vec(&dep).unwrap()).unwrap();
        let keep = keep_set(g);
        assert!(keep.contains("data/skse/plugins/skseMenuFramework.dll".to_ascii_lowercase().as_str()));
        assert!(keep.contains("data/scripts/uimenubase.pex"));
        assert!(!keep.contains("data/meshes/armor/x.nif"));
        // Other's plugin stays (switched off in the load order instead), so
        // its files stay together; the SKSE sweep still moves its DLL alone,
        // which only leaves harmless support files.
        assert_eq!(unlisted_with(g, |_| false), ["Data/Meshes/armor/x.nif"]);
    }

    /// Timothy's PC, 2026-09-26: Vortex folder names with spaces, a manual
    /// Unofficial Patch, and a mod whose plugin stays.
    #[test]
    fn keeps_menu_framework_fonts_and_whole_mods() {
        let t = tempfile::tempdir().unwrap();
        let g = t.path();
        let data = g.join("Data");
        let files = [
            ("SKSE\\Plugins\\SKSEMenuFramework.dll", "SKSE Menu Framework 120352 3.18 2026-09-17T16-36Z cE8hkAlOT"),
            ("SKSE\\Plugins\\SKSEMenuFramework.ini", "SKSE Menu Framework 120352 3.18 2026-09-17T16-36Z cE8hkAlOT"),
            ("SKSE\\Plugins\\fonts\\fa-solid-900.ttf", "Some Other Name"),
            ("SKSE\\Plugins\\SKSEMenuFrameworkThemes\\modern.json", "SKSE Menu Framework 120352 3.18 2026-09-17T16-36Z cE8hkAlOT"),
            ("unofficial skyrim special edition patch.ini", "USSEP manual"),
            ("Docs\\Unofficial Skyrim Special Edition Patch Readme + Credits.html", "USSEP manual"),
            ("Textures\\a.dds", "Armor 77777 1.0"),
            ("Armor.esp", "Armor 77777 1.0"),
            ("Meshes\\b.nif", "Junk 55555 2.0"),
            ("SKSE\\Plugins\\SSEDisplayTweaks.ini", "black screen fix"),
        ];
        for (p, _) in files {
            let f = data.join(p.replace('\\', "/"));
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, b"x").unwrap();
        }
        let dep = serde_json::json!({"files": files.iter().map(|(p, s)| serde_json::json!({"relPath": p, "source": s})).collect::<Vec<_>>()});
        std::fs::write(data.join("vortex.deployment.json"), serde_json::to_vec(&dep).unwrap()).unwrap();
        // Only the mod with nothing that must stay goes.
        assert_eq!(unlisted_with(g, |_| false), ["Data/Meshes/b.nif"]);
        // What 0.1.38 moved comes back; other mods' files stay set aside.
        crate::strays::move_aside(g, &["Data/SKSE/Plugins/SKSEMenuFramework.ini".into(), "Data/SKSE/Plugins/fonts/fa-solid-900.ttf".into(), "Data/unofficial skyrim special edition patch.ini".into(), "Data/Meshes/b.nif".into()], "2026-09-26-21-55-53-UTC-other-mods").unwrap();
        assert_eq!(restore_kept(g).unwrap(), ["Data/SKSE/Plugins/SKSEMenuFramework.ini", "Data/SKSE/Plugins/fonts/fa-solid-900.ttf", "Data/unofficial skyrim special edition patch.ini"]);
        assert!(data.join("SKSE/Plugins/fonts/fa-solid-900.ttf").is_file());
        assert!(!data.join("Meshes/b.nif").exists());
    }

    #[test]
    fn restores_what_was_set_aside() {
        let t = tempfile::tempdir().unwrap();
        let g = t.path();
        std::fs::create_dir_all(g.join("Data/Meshes")).unwrap();
        std::fs::write(g.join("Data/Meshes/a.nif"), b"x").unwrap();
        crate::strays::move_aside(g, &["Data/Meshes/a.nif".into()], "s1").unwrap();
        assert!(!g.join("Data/Meshes/a.nif").exists());
        assert_eq!(restore_all(g).unwrap(), 1);
        assert!(g.join("Data/Meshes/a.nif").is_file());
    }
}
