//! The required Nexus collection (Timothy, 2026-10-13: the launcher only
//! checks that the collection's mods are installed and the load order is
//! right; it no longer bundles third-party mods).
//!
//! The list is a signed copy: built in (`data/collection-nzwgrw-r1.json`) and
//! replaceable by the `collection` object in the signed `launcher-rules.json`,
//! so a new Nexus revision needs no launcher release (`scripts/collection-
//! rules.sh`). Nothing is downloaded, installed or removed here. The result
//! is one health row; it warns by default and stops Play only when the list
//! says `"level": "block"`.
//!
//! Plugin names are compared the way the rest of the launcher names them:
//! "Unofficial Skyrim Special Edition Patch.esp" and its dash-joined alias
//! (aliases.rs) are the same plugin, so the space-name fix and this check
//! agree.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::aliases::alias_name;
use crate::health::{Check, Status};

const BUILT_IN: &str = include_str!("../data/collection-nzwgrw-r1.json");
const MAX_MODS: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    #[default]
    Warn,
    Block,
    Off,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CollMod {
    pub name: String,
    #[serde(default)]
    pub mod_id: u64,
    #[serde(default)]
    pub file_id: u64,
    #[serde(default)]
    pub version: String,
    /// Game-relative paths (files or folders) that must all exist for the mod to count as installed.
    #[serde(default)]
    pub files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Collection {
    pub slug: String,
    pub name: String,
    pub revision: u32,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub level: Level,
    pub mods: Vec<CollMod>,
    /// The collection's plugins in load order.
    #[serde(default)]
    pub plugin_order: Vec<String>,
}

impl Collection {
    pub fn built_in() -> Collection {
        Collection::parse(BUILT_IN.as_bytes()).expect("built-in collection is valid")
    }

    /// Reads a collection object. Entries that could point outside the game folder are refused.
    pub fn parse(bytes: &[u8]) -> Result<Collection, String> {
        let mut c: Collection = serde_json::from_slice(bytes).map_err(|e| format!("collection: {e}"))?;
        if c.mods.len() > MAX_MODS {
            return Err("collection: too many mods".into());
        }
        for m in &c.mods {
            if m.name.trim().is_empty() || m.files.is_empty() {
                return Err(format!("collection: mod '{}' needs a name and files", m.name));
            }
            for f in &m.files {
                if f.contains('\\') || crate::manifest::safe_relative(f).is_err() {
                    return Err(format!("collection: unsafe path '{f}'"));
                }
            }
        }
        c.plugin_order.retain(|p| crate::serverorder::plain_plugin_name(p));
        Ok(c)
    }
}

/// A path under `root`, matched case-insensitively one segment at a time (Windows is, tests on Linux are not).
fn find_ci(root: &Path, rel: &str) -> Option<PathBuf> {
    let mut cur = root.to_path_buf();
    for seg in rel.split('/').filter(|s| !s.is_empty()) {
        let direct = cur.join(seg);
        if direct.exists() {
            cur = direct;
            continue;
        }
        let hit = std::fs::read_dir(&cur).ok()?.flatten().find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(seg))?;
        cur = hit.path();
    }
    Some(cur)
}

/// Same plugin: equal ignoring case, or equal after the launcher's dash-joined alias.
fn same_plugin(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b) || alias_name(a).eq_ignore_ascii_case(&alias_name(b))
}

/// A mod's file is there directly, or (for a plugin with spaces) under its alias name.
fn present(game_dir: &Path, rel: &str) -> bool {
    if find_ci(game_dir, rel).is_some() {
        return true;
    }
    let (dir, file) = rel.rsplit_once('/').unwrap_or(("", rel));
    let is_plugin = crate::serverorder::plain_plugin_name(file);
    let alias = alias_name(file);
    is_plugin && alias != file && find_ci(game_dir, &format!("{dir}/{alias}")).is_some()
}

/// Active plugins of plugins.txt, in order.
fn active_plugins(plugins_txt: &Path) -> Vec<String> {
    crate::loadorder::read_text(plugins_txt)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.trim().strip_prefix('*'))
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .collect()
}

/// The health row for this collection. `plugins_txt` None skips the order part.
pub fn check(c: &Collection, game_dir: &Path, plugins_txt: Option<&Path>) -> Check {
    let title = "Required mods (Nexus collection)";
    if c.level == Level::Off {
        return Check { id: "collection", title, status: Status::Info, detail: "The server's launcher rules switched this check off.".into(), items: vec![] };
    }
    let mut items = Vec::new();
    for m in &c.mods {
        if !m.files.iter().all(|f| present(game_dir, f)) {
            let v = if m.version.is_empty() { String::new() } else { format!(" {}", m.version) };
            items.push(format!("Missing: {}{v}", m.name));
        }
    }
    if let Some(txt) = plugins_txt {
        let active = active_plugins(txt);
        let mut last: Option<(usize, &str)> = None;
        for want in &c.plugin_order {
            let Some(pos) = active.iter().position(|a| same_plugin(a, want)) else {
                // Only a complaint when the file is installed but not switched on.
                if present(game_dir, &format!("Data/{want}")) {
                    items.push(format!("Not switched on in your load order: {want}"));
                }
                continue;
            };
            if let Some((lp, lname)) = last {
                if pos < lp {
                    items.push(format!("Out of order: {want} must load after {lname}"));
                }
            }
            last = Some((pos, want));
        }
    }
    let rev = format!("revision {}", c.revision);
    if items.is_empty() {
        return Check { id: "collection", title, status: Status::Ok, detail: format!("All {} mods of {} ({rev}) are installed and in order.", c.mods.len(), c.name), items };
    }
    let status = if c.level == Level::Block { Status::Fail } else { Status::Warn };
    let link = if c.url.is_empty() { String::new() } else { format!(" Get them from the collection: {}", c.url) };
    Check { id: "collection", title, status, detail: format!("{} of the collection {} ({rev}) is missing or out of order.{link}", items.len(), c.name), items }
}

#[cfg(test)]
mod tests {
    use super::*;

    const USSEP: &str = "Unofficial Skyrim Special Edition Patch.esp";

    fn install_all(dir: &Path, c: &Collection) {
        for m in &c.mods {
            for f in &m.files {
                let p = dir.join(f);
                std::fs::create_dir_all(p.parent().unwrap()).unwrap();
                if !f.contains('.') {
                    std::fs::create_dir_all(&p).unwrap();
                } else {
                    std::fs::write(&p, b"x").unwrap();
                }
            }
        }
    }

    #[test]
    fn built_in_matches_nexus_revision_one() {
        let c = Collection::built_in();
        assert_eq!((c.slug.as_str(), c.revision, c.mods.len()), ("nzwgrw", 1, 8));
        assert_eq!(c.level, Level::Warn);
        assert!(c.mods.iter().any(|m| m.name.contains("Skyrim Souls RE") && m.file_id == 810395));
        assert!(c.plugin_order.iter().any(|p| p == USSEP));
    }

    #[test]
    fn everything_installed_and_in_order_is_ok() {
        let t = tempfile::tempdir().unwrap();
        let c = Collection::built_in();
        install_all(t.path(), &c);
        let txt = t.path().join("plugins.txt");
        std::fs::write(&txt, format!("# x\r\n*Skyrim.esm\r\n*{USSEP}\r\n*SkyUI_SE.esp\r\n")).unwrap();
        let r = check(&c, t.path(), Some(&txt));
        assert_eq!(r.status, Status::Ok, "{:?}", r.items);
    }

    #[test]
    fn missing_mod_warns_and_names_it() {
        let t = tempfile::tempdir().unwrap();
        let c = Collection::built_in();
        install_all(t.path(), &c);
        std::fs::remove_file(t.path().join("Data/SKSE/Plugins/SkyrimSoulsRE.dll")).unwrap();
        let r = check(&c, t.path(), None);
        assert_eq!(r.status, Status::Warn);
        assert_eq!(r.items, vec!["Missing: Skyrim Souls RE - Unpaused Menus 3.2.0".to_string()]);
    }

    #[test]
    fn block_level_fails_and_off_hides() {
        let t = tempfile::tempdir().unwrap();
        let mut c = Collection::built_in();
        c.level = Level::Block;
        assert_eq!(check(&c, t.path(), None).status, Status::Fail);
        c.level = Level::Off;
        assert_eq!(check(&c, t.path(), None).status, Status::Info);
    }

    #[test]
    fn out_of_order_is_reported() {
        let t = tempfile::tempdir().unwrap();
        let c = Collection::built_in();
        install_all(t.path(), &c);
        let txt = t.path().join("plugins.txt");
        std::fs::write(&txt, format!("*SkyUI_SE.esp\r\n*{USSEP}\r\n")).unwrap();
        let r = check(&c, t.path(), Some(&txt));
        assert_eq!(r.status, Status::Warn);
        assert_eq!(r.items, vec![format!("Out of order: SkyUI_SE.esp must load after {USSEP}")]);
    }

    #[test]
    fn plugin_with_spaces_counts_under_its_alias_name() {
        // The launcher runs USSEP as "Unofficial-Skyrim-Special-Edition-Patch.esp"
        // (aliases.rs); both the file and the plugins.txt line may carry either name.
        let t = tempfile::tempdir().unwrap();
        let c = Collection::built_in();
        install_all(t.path(), &c);
        let data = t.path().join("Data");
        std::fs::rename(data.join(USSEP), data.join("Unofficial-Skyrim-Special-Edition-Patch.esp")).unwrap();
        let txt = t.path().join("plugins.txt");
        std::fs::write(&txt, "*Unofficial-Skyrim-Special-Edition-Patch.esp\r\n*SkyUI_SE.esp\r\n").unwrap();
        assert_eq!(check(&c, t.path(), Some(&txt)).status, Status::Ok);
        std::fs::write(&txt, "*SkyUI_SE.esp\r\n*Unofficial-Skyrim-Special-Edition-Patch.esp\r\n").unwrap();
        assert_eq!(check(&c, t.path(), Some(&txt)).status, Status::Warn);
    }

    #[test]
    fn installed_but_switched_off_is_named() {
        let t = tempfile::tempdir().unwrap();
        let c = Collection::built_in();
        install_all(t.path(), &c);
        let txt = t.path().join("plugins.txt");
        std::fs::write(&txt, format!("{USSEP}\r\n*SkyUI_SE.esp\r\n")).unwrap();
        let r = check(&c, t.path(), Some(&txt));
        assert_eq!(r.items, vec![format!("Not switched on in your load order: {USSEP}")]);
    }

    #[test]
    fn unsafe_paths_and_bad_files_are_refused() {
        let bad = br#"{"slug":"a","name":"a","revision":2,"mods":[{"name":"x","files":["../evil.dll"]}]}"#;
        assert!(Collection::parse(bad).is_err());
        assert!(Collection::parse(b"{}").is_err());
    }
}
