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

fn yes() -> bool {
    true
}

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
    /// More game-relative prefixes this mod owns (loose files, archives) that the no-mods guard must leave alone.
    #[serde(default)]
    pub allow: Vec<String>,
    /// The launcher sets this mod's files aside for the session, like any other mod (rules `hold: true`; kill switch: remove it).
    /// Used while a collection mod crashes the game on a PC (Actor Limit Fix on 1.6.1170, 2026-10-14).
    #[serde(default)]
    pub hold: bool,
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
    /// Before Play, switch on every installed collection plugin in plugins.txt (Vortex leaves some off).
    #[serde(default)]
    pub enable_plugins: bool,
    /// Report a collection plugin that is switched on while a master it names is off or loads after it (rules `collection.masterCheck`).
    #[serde(default = "yes")]
    pub master_check: bool,
    /// The server's own plugins (masters.json), set by Play. Their order and on/off state belong to the server order (serverorder.rs).
    #[serde(skip)]
    pub pinned: Vec<String>,
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
            // No files means the mod is listed but cannot be checked yet (its file names are unknown).
            if m.name.trim().is_empty() {
                return Err("collection: a mod needs a name".into());
            }
            for f in m.files.iter().chain(&m.allow) {
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
    a.eq_ignore_ascii_case(b) || run_key(a) == run_key(b)
}

/// The name with the launcher's changes undone: dash alias for a spaced name, and no "-AD" before the extension
/// (the rewritten copy of a plugin whose masters needed new names, aliases.rs `run_name`).
fn run_key(n: &str) -> String {
    let a = alias_name(n).to_ascii_lowercase();
    match a.rsplit_once('.') {
        Some((stem, ext)) => format!("{}.{ext}", stem.strip_suffix("-ad").unwrap_or(stem)),
        None => a,
    }
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

impl Collection {
    /// True when the no-mods guard (modguard.rs) must leave this file alone because
    /// the required collection owns it: its plugins (also under the alias name),
    /// the archives and inis named like those plugins, its detection files and
    /// folders, and its `allow` prefixes. Nothing is allowed when the check is off.
    pub fn owns(&self, kind: &str, rel: &str) -> bool {
        if self.level == Level::Off {
            return false;
        }
        let rel_l = rel.to_ascii_lowercase();
        let file = rel_l.rsplit('/').next().unwrap_or("");
        let under = |p: &str| {
            let p = p.to_ascii_lowercase();
            rel_l == p || rel_l.starts_with(&format!("{p}/"))
        };
        // `files` are exact paths or folders; `allow` entries are plain prefixes (a folder, or a file-name start).
        if self.mods.iter().filter(|m| !m.hold).any(|m| m.files.iter().any(|p| under(p)) || m.allow.iter().any(|p| rel_l.starts_with(&p.to_ascii_lowercase()))) {
            return true;
        }
        if kind == "plugin" || kind == "archive" {
            let stem = |n: &str| n.rsplit_once('.').map_or(n.to_string(), |(s, _)| s.to_string()).to_ascii_lowercase();
            let fs = stem(file);
            return self.plugin_order.iter().any(|p| {
                let ps = stem(p);
                let pa = stem(&alias_name(p));
                kind == "plugin" && same_plugin(file, p) || [&ps, &pa].iter().any(|x| fs == **x || fs.starts_with(&format!("{x} - ")))
            });
        }
        false
    }
}


impl Collection {
    /// The name the plugins.txt line for a collection plugin must carry, when the collection should ask for it at all:
    /// None when no client-loadable copy is in Data (a spaced name without its alias), when the server pins the plugin,
    /// or when the launcher keeps it off (serverorder::kept_off). Those are the launcher's call, never a missing mod.
    fn wanted_name(&self, game_dir: &Path, want: &str) -> Option<String> {
        let name = [want.to_string(), alias_name(want)].into_iter().find(|n| crate::loadorder::client_can_load_name(n) && present(game_dir, &format!("Data/{n}")))?;
        if self.pinned.iter().any(|p| same_plugin(p, &name)) || crate::serverorder::kept_off(game_dir, &name) {
            return None;
        }
        Some(name)
    }
}

/// A row item that can stop Play when the level is `block`: a missing mod or a plugin that is switched off.
pub fn is_blocker(item: &str) -> bool {
    item.starts_with("Missing: ") || item.starts_with("Not switched on")
}

/// What Play tells the player when the collection stops it: the blocking items (at most 6) and where to get them.
pub fn block_message(c: &Collection, items: &[String]) -> String {
    let list: Vec<&str> = items.iter().filter(|i| is_blocker(i)).map(|s| s.as_str()).take(6).collect();
    let all = items.iter().filter(|i| is_blocker(i)).count();
    let more = if all > list.len() { format!(" and {} more", all - list.len()) } else { String::new() };
    let link = if c.url.is_empty() { String::new() } else { format!(" Get them with Vortex from the collection: {}/revisions/{}.", c.url, c.revision) };
    format!("You need the {} mod collection (revision {}) to join. {}{more}.{link} Install or switch them on, then press Play again.", c.name, c.revision, list.join("; "))
}

/// Switches on every installed collection plugin in plugins.txt, adding the ones the file does not list
/// (at the end). Existing order is left alone; a plugin the client cannot load by name is skipped.
/// Returns the names it changed. A copy of the old file is kept beside it (loadorder::write_list).
pub fn enable_plugins(c: &Collection, game_dir: &Path, plugins_txt: &Path) -> crate::Result<Vec<String>> {
    if c.level == Level::Off || !c.enable_plugins {
        return Ok(Vec::new());
    }
    let file = crate::loadorder::ListFile::read(plugins_txt)?;
    let text = file.as_ref().map_or("", |f| f.text.as_str());
    let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
    let mut changed = Vec::new();
    for want in &c.plugin_order {
        // The name the game client can load; plugins the server pins or the launcher keeps off are left alone.
        let Some(name) = c.wanted_name(game_dir, want) else { continue };
        let at = lines.iter().position(|l| same_plugin(l.trim().trim_start_matches('*').trim(), &name));
        match at {
            Some(i) if lines[i].trim().starts_with('*') => {}
            Some(i) => {
                lines[i] = format!("*{}", lines[i].trim());
                changed.push(name);
            }
            None => {
                lines.push(format!("*{name}"));
                changed.push(name);
            }
        }
    }
    if !changed.is_empty() {
        let mut new = lines.join(nl);
        new.push_str(nl);
        crate::loadorder::write_list(plugins_txt, file.as_ref(), &new)?;
    }
    Ok(changed)
}

/// Collection plugins that are switched on while a master they name is not: the game stops with a missing-master
/// error (or loads the plugin before its master) and MoreCraftableEquipment_USSEP.esp, whose master is the spaced
/// USSEP name, is the known case (rev 7). Only the plugins of `pluginOrder` count; the base game and Creation Club
/// masters are always there. Warn-only wording, never a blocker (`is_blocker`).
fn master_problems(c: &Collection, game_dir: &Path, active: &[String]) -> Vec<String> {
    let data = game_dir.join("Data");
    let mut out = Vec::new();
    for (i, a) in active.iter().enumerate() {
        if !c.plugin_order.iter().any(|w| same_plugin(w, a)) {
            continue;
        }
        let Some(file) = find_ci(&data, a) else { continue };
        for m in crate::loadorder::masters(&file).unwrap_or_default() {
            if crate::aliases::shipped_with_game(&m) || same_plugin(&m, a) {
                continue;
            }
            if active[..i].iter().any(|x| same_plugin(x, &m)) {
                continue;
            }
            if active[i + 1..].iter().any(|x| same_plugin(x, &m)) {
                out.push(format!("Master loads after it: {a} needs {m} above it"));
            } else {
                out.push(format!("Master not switched on: {a} needs {m}"));
            }
        }
    }
    out
}

/// The health row for this collection. `plugins_txt` None skips the order part.
pub fn check(c: &Collection, game_dir: &Path, plugins_txt: Option<&Path>) -> Check {
    let title = "Required mods (Nexus collection)";
    if c.level == Level::Off {
        return Check { id: "collection", title, status: Status::Info, detail: "The server's launcher rules switched this check off.".into(), items: vec![] };
    }
    let mut items = Vec::new();
    let mut found = Vec::new();
    for m in &c.mods {
        let v = if m.version.is_empty() { String::new() } else { format!(" {}", m.version) };
        if m.hold {
            found.push(format!("Set aside by the launcher for now: {}{v}", m.name));
        } else if m.files.is_empty() {
            found.push(format!("Not checked: {}{v}", m.name));
        } else if m.files.iter().all(|f| present(game_dir, f)) {
            found.push(format!("Installed: {}{v}", m.name));
        } else {
            items.push(format!("Missing: {}{v}", m.name));
        }
    }
    if let Some(txt) = plugins_txt {
        let active = active_plugins(txt);
        let mut last: Option<(usize, &str)> = None;
        for want in &c.plugin_order {
            // Only plugins the player controls count: not the server's (their order is the server's), not ones the launcher keeps off.
            let Some(name) = c.wanted_name(game_dir, want) else { continue };
            let Some(pos) = active.iter().position(|a| same_plugin(a, &name)) else {
                items.push(format!("Not switched on in your load order: {want}"));
                continue;
            };
            if let Some((lp, lname)) = last {
                if pos < lp {
                    items.push(format!("Out of order: {want} must load after {lname}"));
                }
            }
            last = Some((pos, want));
        }
        if c.master_check {
            items.extend(master_problems(c, game_dir, &active));
        }
    }
    let rev = format!("revision {}", c.revision);
    let problems = items.len();
    // The row lists every mod of the collection, problems first, so nothing looks left out.
    items.extend(found);
    if problems == 0 {
        return Check { id: "collection", title, status: Status::Ok, detail: format!("All {} mods of {} ({rev}) are installed and in order.", c.mods.len(), c.name), items };
    }
    // Only a missing mod or a plugin that is off can stop Play; load order alone never does.
    let blocking = items.iter().take(problems).any(|i| is_blocker(i));
    let status = if c.level == Level::Block && blocking { Status::Fail } else { Status::Warn };
    let link = if c.url.is_empty() { String::new() } else { format!(" Get them from the collection: {}", c.url) };
    Check { id: "collection", title, status, detail: format!("{problems} of the collection {} ({rev}) is missing or out of order.{link}", c.name), items }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn problems(r: &Check) -> Vec<String> {
        r.items.iter().filter(|i| !i.starts_with("Installed: ")).cloned().collect()
    }

    const USSEP: &str = "Unofficial Skyrim Special Edition Patch.esp";
    const ALIAS: &str = "Unofficial-Skyrim-Special-Edition-Patch.esp";

    /// The launcher's dash-named copy of USSEP, the one name the game client can load (aliases.rs).
    fn add_alias(dir: &Path) {
        std::fs::copy(dir.join("Data").join(USSEP), dir.join("Data").join(ALIAS)).unwrap();
    }

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
        assert_eq!(problems(&r), vec!["Missing: Skyrim Souls RE - Unpaused Menus 3.2.0".to_string()]);
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
        add_alias(t.path());
        let txt = t.path().join("plugins.txt");
        std::fs::write(&txt, format!("*SkyUI_SE.esp\r\n*{ALIAS}\r\n")).unwrap();
        let r = check(&c, t.path(), Some(&txt));
        assert_eq!(r.status, Status::Warn);
        assert_eq!(problems(&r), vec![format!("Out of order: SkyUI_SE.esp must load after {USSEP}")]);
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
        add_alias(t.path());
        let txt = t.path().join("plugins.txt");
        std::fs::write(&txt, format!("{ALIAS}\r\n*SkyUI_SE.esp\r\n")).unwrap();
        let r = check(&c, t.path(), Some(&txt));
        assert_eq!(problems(&r), vec![format!("Not switched on in your load order: {USSEP}")]);
    }

    #[test]
    fn the_no_mods_guard_leaves_the_collections_files_alone() {
        let c = Collection::built_in();
        for (k, rel) in [
            ("plugin", "Data/SkyUI_SE.esp"),
            ("plugin", "Data/Unofficial Skyrim Special Edition Patch.esp"),
            ("plugin", "Data/Unofficial-Skyrim-Special-Edition-Patch.esp"),
            ("archive", "Data/Unofficial Skyrim Special Edition Patch.bsa"),
            ("archive", "Data/Unofficial-Skyrim-Special-Edition-Patch - Textures.bsa"),
            ("dll", "Data/SKSE/Plugins/SkyrimSoulsRE.dll"),
            ("dll", "Data/skse/plugins/enginefixes.dll"),
            ("files", "Data/SKSE/Plugins/ImGuiIcons/a.png"),
            ("files", "Data/Interface/skyui/x.swf"),
        ] {
            assert!(c.owns(k, rel), "{rel}");
        }
        for (k, rel) in [("plugin", "Data/Other.esp"), ("dll", "Data/SKSE/Plugins/Cheat.dll"), ("files", "Data/Meshes/x.nif"), ("archive", "Data/SkyUI_Other.bsa")] {
            assert!(!c.owns(k, rel), "{rel}");
        }
        let mut off = c.clone();
        off.level = Level::Off;
        assert!(!off.owns("plugin", "Data/SkyUI_SE.esp"));
    }

    #[test]
    fn unsafe_paths_and_bad_files_are_refused() {
        let bad = br#"{"slug":"a","name":"a","revision":2,"mods":[{"name":"x","files":["../evil.dll"]}]}"#;
        assert!(Collection::parse(bad).is_err());
        assert!(Collection::parse(b"{}").is_err());
    }

    #[test]
    fn block_stops_only_for_a_missing_mod_or_a_plugin_that_is_off() {
        let t = tempfile::tempdir().unwrap();
        let mut c = Collection::built_in();
        c.level = Level::Block;
        install_all(t.path(), &c);
        add_alias(t.path());
        let txt = t.path().join("plugins.txt");
        // Out of order alone: warn, never a stop.
        std::fs::write(&txt, format!("*SkyUI_SE.esp\r\n*{ALIAS}\r\n")).unwrap();
        assert_eq!(check(&c, t.path(), Some(&txt)).status, Status::Warn);
        // A mod that is not there: stop, and the message names it with the collection link.
        std::fs::remove_file(t.path().join("Data/SKSE/Plugins/SkyrimSoulsRE.dll")).unwrap();
        let r = check(&c, t.path(), None);
        assert_eq!(r.status, Status::Fail);
        let msg = block_message(&c, &r.items);
        assert!(msg.contains("Missing: Skyrim Souls RE") && msg.contains("nexusmods.com") && !msg.contains("Installed:"), "{msg}");
        // A plugin installed but switched off: stop.
        std::fs::write(t.path().join("Data/SKSE/Plugins/SkyrimSoulsRE.dll"), b"x").unwrap();
        std::fs::write(&txt, format!("*{ALIAS}\r\nSkyUI_SE.esp\r\n")).unwrap();
        assert_eq!(check(&c, t.path(), Some(&txt)).status, Status::Fail);
        // Warn level never stops.
        c.level = Level::Warn;
        assert_eq!(check(&c, t.path(), Some(&txt)).status, Status::Warn);
    }

    #[test]
    fn enabling_switches_plugins_on_adds_missing_ones_and_keeps_the_order() {
        let t = tempfile::tempdir().unwrap();
        let mut c = Collection::built_in();
        install_all(t.path(), &c);
        let txt = t.path().join("plugins.txt");
        std::fs::write(&txt, "# x\r\n*Skyrim.esm\r\nSkyUI_SE.esp\r\n").unwrap();
        // Off by default: nothing changes.
        assert!(enable_plugins(&c, t.path(), &txt).unwrap().is_empty());
        c.enable_plugins = true;
        let done = enable_plugins(&c, t.path(), &txt).unwrap();
        // The spaced USSEP file cannot be switched on by name; only the launcher's alias copy can.
        assert_eq!(done, ["SkyUI_SE.esp"]);
        std::fs::write(t.path().join("Data").join(alias_name(USSEP)), b"x").unwrap();
        assert_eq!(enable_plugins(&c, t.path(), &txt).unwrap(), [alias_name(USSEP)]);
        let text = std::fs::read_to_string(&txt).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[..3], ["# x", "*Skyrim.esm", "*SkyUI_SE.esp"]);
        assert_eq!(lines[3], format!("*{}", alias_name(USSEP)));
        assert!(text.contains("\r\n"));
        // A second run has nothing left to do.
        assert!(enable_plugins(&c, t.path(), &txt).unwrap().is_empty());
        // A mod with no known files is listed, not checked, and cannot block.
        let mut c2 = c.clone();
        c2.mods.push(CollMod { name: "Mystery".into(), mod_id: 1, file_id: 1, version: String::new(), files: vec![], allow: vec![], hold: false });
        let r = check(&c2, t.path(), None);
        assert!(r.items.iter().any(|i| i == "Not checked: Mystery"), "{:?}", r.items);
    }

    /// A plugin file whose TES4 header carries the given record flags.
    fn write_plugin(dir: &Path, name: &str, flags: u32) {
        let mut h = b"TES4".to_vec();
        h.extend_from_slice(&[0u8; 4]);
        h.extend_from_slice(&flags.to_le_bytes());
        h.extend_from_slice(&[0u8; 12]);
        std::fs::write(dir.join("Data").join(name), h).unwrap();
    }

    #[test]
    fn the_check_leaves_alone_what_the_launcher_decides() {
        let t = tempfile::tempdir().unwrap();
        let mut c = Collection::built_in();
        c.level = Level::Block;
        install_all(t.path(), &c);
        let txt = t.path().join("plugins.txt");
        // USSEP only under its spaced name: no client can load it, so nobody is asked to switch it on.
        std::fs::write(&txt, "*SkyUI_SE.esp\r\n").unwrap();
        let r = check(&c, t.path(), Some(&txt));
        assert_eq!((r.status, problems(&r)), (Status::Ok, Vec::<String>::new()));
        // A full master the server does not load stays off (serverorder::kept_off): not a problem, not enabled.
        write_plugin(t.path(), "SkyUI_SE.esp", 0x1);
        std::fs::write(&txt, "SkyUI_SE.esp\r\n").unwrap();
        assert_eq!(check(&c, t.path(), Some(&txt)).status, Status::Ok);
        c.enable_plugins = true;
        assert!(enable_plugins(&c, t.path(), &txt).unwrap().is_empty());
        // A plugin the server pins is the server order's business, not the collection's.
        write_plugin(t.path(), "SkyUI_SE.esp", 0);
        c.pinned = vec!["skyui_se.esp".into()];
        assert_eq!(check(&c, t.path(), Some(&txt)).status, Status::Ok);
        assert!(enable_plugins(&c, t.path(), &txt).unwrap().is_empty());
        // Not pinned and an ordinary plugin: it is the player's, so off means a stop and Play switches it on.
        c.pinned.clear();
        assert_eq!(check(&c, t.path(), Some(&txt)).status, Status::Fail);
        assert_eq!(enable_plugins(&c, t.path(), &txt).unwrap(), vec!["SkyUI_SE.esp".to_string()]);
    }

    fn plugin_with_masters(ms: &[&str]) -> Vec<u8> {
        let mut sub = b"HEDR".to_vec();
        sub.extend(12u16.to_le_bytes());
        sub.extend(1.7f32.to_le_bytes());
        sub.extend([0u8; 8]);
        for m in ms {
            sub.extend(b"MAST");
            sub.extend(((m.len() + 1) as u16).to_le_bytes());
            sub.extend(m.as_bytes());
            sub.push(0);
            sub.extend(b"DATA");
            sub.extend(8u16.to_le_bytes());
            sub.extend([0u8; 8]);
        }
        let mut b = b"TES4".to_vec();
        b.extend((sub.len() as u32).to_le_bytes());
        b.extend([0u8; 16]);
        b.extend(sub);
        b
    }

    #[test]
    fn the_launchers_run_names_count_as_the_collection_plugin() {
        assert!(same_plugin("MoreCraftableEquipment_USSEP.esp", "MoreCraftableEquipment_USSEP-AD.esp"));
        assert!(same_plugin(USSEP, ALIAS));
        assert!(!same_plugin("MoreCraftableEquipment.esp", "MoreCraftableEquipment_USSEP.esp"));
    }

    #[test]
    fn a_plugin_on_with_its_master_off_or_below_it_is_named() {
        // Rev 7: MoreCraftableEquipment_USSEP.esp names the spaced USSEP as a master.
        let t = tempfile::tempdir().unwrap();
        let mut c = Collection::built_in();
        c.plugin_order = vec![USSEP.into(), "MoreCraftableEquipment_USSEP.esp".into()];
        let data = t.path().join("Data");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("MoreCraftableEquipment_USSEP.esp"), plugin_with_masters(&["Skyrim.esm", USSEP, "ccBGSSSE001-Fish.esm"])).unwrap();
        std::fs::write(data.join(ALIAS), plugin_with_masters(&["Skyrim.esm"])).unwrap();
        let txt = t.path().join("plugins.txt");
        let master_items = |r: &Check| r.items.iter().filter(|i| i.starts_with("Master ")).cloned().collect::<Vec<_>>();
        // Master listed but off.
        std::fs::write(&txt, format!("{ALIAS}\r\n*MoreCraftableEquipment_USSEP.esp\r\n")).unwrap();
        assert_eq!(master_items(&check(&c, t.path(), Some(&txt))), vec![format!("Master not switched on: MoreCraftableEquipment_USSEP.esp needs {USSEP}")]);
        // On, but below the plugin that needs it.
        std::fs::write(&txt, format!("*MoreCraftableEquipment_USSEP.esp\r\n*{ALIAS}\r\n")).unwrap();
        assert_eq!(master_items(&check(&c, t.path(), Some(&txt))), vec![format!("Master loads after it: MoreCraftableEquipment_USSEP.esp needs {USSEP} above it")]);
        // On and above it (by alias name): nothing. Base game and Creation Club masters are never reported.
        std::fs::write(&txt, format!("*{ALIAS}\r\n*MoreCraftableEquipment_USSEP.esp\r\n")).unwrap();
        assert!(master_items(&check(&c, t.path(), Some(&txt))).is_empty());
        // Kill switch (rules collection.masterCheck:false).
        std::fs::write(&txt, format!("{ALIAS}\r\n*MoreCraftableEquipment_USSEP.esp\r\n")).unwrap();
        c.master_check = false;
        assert!(master_items(&check(&c, t.path(), Some(&txt))).is_empty());
    }

    #[test]
    fn a_held_mod_is_not_owned_and_is_not_reported_missing() {
        let t = tempfile::tempdir().unwrap();
        let mut c = Collection::built_in();
        install_all(t.path(), &c);
        let mod_files = c.mods[1].files.clone();
        let rel = mod_files[0].clone();
        assert!(c.owns("dll", &rel));
        c.mods[1].hold = true;
        // The no-mods guard now sets the file aside like any other mod's.
        assert!(!c.owns("dll", &rel));
        std::fs::remove_file(t.path().join(&rel)).ok();
        let r = check(&c, t.path(), None);
        assert!(!r.items.iter().any(|i| i.starts_with("Missing: ") && i.contains(&c.mods[1].name)), "{:?}", r.items);
        assert!(r.items.iter().any(|i| i.starts_with("Set aside by the launcher for now: ")));
    }
}
