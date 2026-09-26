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

fn listed_source(source: &str, ids: &[u64]) -> bool {
    let s = source.to_ascii_lowercase();
    // Vortex names mod folders "<name>-<nexus id>-<version>-<time>".
    ids.iter().any(|id| s.contains(&format!("-{id}-"))) || s.starts_with("skse64") || s.contains("skyrim script extender")
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
    for f in vortex_files(game_dir) {
        if listed_source(&f.source, &ids) {
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
    let list = listed(game_dir);
    let ids = ids(&list);
    let keep = keep_set(game_dir);
    let mut out: Vec<String> = vortex_files(game_dir)
        .into_iter()
        .filter(|f| !listed_source(&f.source, &ids))
        .map(|f| f.rel)
        .filter(|rel| {
            let l = rel.to_ascii_lowercase();
            let plugin_or_archive = [".esp", ".esm", ".esl", ".bsa"].iter().any(|x| l.ends_with(x));
            !plugin_or_archive && !keep.contains(&l) && !server_file(rel) && game_dir.join(rel).is_file()
        })
        .collect();
    out.sort();
    out.dedup();
    out
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
        assert_eq!(unlisted_with(g, |_| false), ["Data/Meshes/armor/x.nif", "Data/SKSE/Plugins/Other.dll"]);
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
