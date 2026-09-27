//! The install report sent on each Play (POST {AUTH_URL}/api/client-status),
//! so staff can see which mods a player is missing without asking for logs.
//! Names and counts only: never a file path, folder or account detail.

use std::collections::BTreeSet;
use std::path::Path;

use serde::Serialize;

use crate::modlist::ModEntry;
use crate::requirements as r;

/// Most names sent in one list.
const MAX_NAMES: usize = 200;

#[derive(Serialize, Debug, PartialEq)]
pub struct Report {
    pub launcher: String,
    pub mods: Mods,
    pub required: Required,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Mods {
    /// Mods the server's mods.json lists.
    pub served: usize,
    pub installed: usize,
    pub missing: Vec<String>,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Required {
    pub ok: bool,
    pub missing: Vec<String>,
}

/// The required mods every player needs, by name, and whether each is there.
pub fn required(game_dir: &Path, game_version: Option<&str>) -> Vec<(&'static str, bool)> {
    let mut out = vec![("SKSE64", r::skse_ok(game_dir))];
    if let Some(v) = game_version {
        out.push(("Address Library for SKSE Plugins", r::address_library_ok(game_dir, v)));
    }
    out.extend([
        ("Crash Logger", r::crash_logger_ok(game_dir)),
        ("Skyrim Souls RE", r::souls_ok(game_dir)),
        ("SSE Engine Fixes", r::engine_fixes_ok(game_dir) && r::engine_fixes_preload_ok(game_dir)),
        ("Unofficial Skyrim Special Edition Patch", r::ussep_ok(game_dir)),
        ("SKSE Menu Framework", r::menu_framework_ok(game_dir)),
        ("ImGui Icons", r::imgui_icons_ok(game_dir)),
        ("SkyUI", r::skyui_ok(game_dir)),
    ]);
    out
}

/// The report: `list` is the merged mod list, `served` the ids the server's
/// mods.json names, `installed` whether a listed mod is installed.
pub fn report(launcher: &str, list: &[ModEntry], served: &BTreeSet<String>, installed: impl Fn(&ModEntry) -> bool, required: &[(&str, bool)]) -> Report {
    let from_server: Vec<&ModEntry> = list.iter().filter(|m| served.contains(&m.id)).collect();
    let missing: Vec<String> = from_server.iter().filter(|m| !installed(m)).map(|m| m.name.clone()).collect();
    let req_missing: Vec<String> = required.iter().filter(|(_, ok)| !ok).map(|(n, _)| n.to_string()).collect();
    Report {
        launcher: launcher.to_string(),
        mods: Mods { served: from_server.len(), installed: from_server.len() - missing.len(), missing: missing.into_iter().take(MAX_NAMES).collect() },
        required: Required { ok: req_missing.is_empty(), missing: req_missing },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, name: &str) -> ModEntry {
        ModEntry { id: id.into(), name: name.into(), ..Default::default() }
    }

    #[test]
    fn counts_only_the_servers_mods_and_names_what_is_missing() {
        let list = vec![entry("address-library", "Address Library"), entry("fsmp", "Faster HDT-SMP"), entry("racemenu", "RaceMenu"), entry("skyui", "SkyUI")];
        let served: BTreeSet<String> = ["fsmp", "racemenu", "skyui"].map(String::from).into();
        let r = report("0.1.80", &list, &served, |m| m.id == "skyui", &[("SKSE64", true), ("Crash Logger", false)]);
        assert_eq!(r.mods, Mods { served: 3, installed: 1, missing: vec!["Faster HDT-SMP".into(), "RaceMenu".into()] });
        assert_eq!(r.required, Required { ok: false, missing: vec!["Crash Logger".into()] });
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["launcher"], "0.1.80");
        assert_eq!(json["required"]["ok"], false);
        // Nothing but names and counts.
        assert!(!json.to_string().contains('/') && !json.to_string().contains('\\'));
    }

    #[test]
    fn required_names_the_mods_on_an_empty_folder() {
        let t = tempfile::tempdir().unwrap();
        let req = required(t.path(), Some("1.6.1170.0"));
        assert!(req.iter().all(|(_, ok)| !ok));
        assert!(req.iter().any(|(n, _)| *n == "SKSE64"));
        assert!(req.iter().any(|(n, _)| n.starts_with("Address Library")));
    }
}
