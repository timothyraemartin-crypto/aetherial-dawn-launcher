//! The install report sent on each Play (POST {AUTH_URL}/api/client-status),
//! so staff can see which mods a player is missing without asking for logs.
//! Names and counts only: never a file path, folder or account detail.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Instant;

use serde::Serialize;

use crate::modlist::ModEntry;
use crate::requirements as r;

/// Most names sent in one list, and the longest name (the bot's limits).
const MAX_NAMES: usize = 100;
const MAX_NAME: usize = 120;

#[derive(Serialize, Debug, PartialEq)]
pub struct Report {
    pub launcher: String,
    pub mods: Mods,
    pub required: Required,
    /// Where this Play attempt got to and where it stopped; only sent when
    /// the player shares health reports.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub play: Option<PlayTrace>,
}

/// How far one Play attempt got, so staff can tell where a player got stuck.
/// Stage names are the fixed step names the launcher shows ("Getting your
/// mods ready"); times are milliseconds spent in each. No paths or accounts.
#[derive(Serialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct PlayTrace {
    /// The step the attempt was in when it was last updated.
    pub stage: String,
    /// The step that failed; absent while the attempt is going well.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failed_at: Option<String>,
    /// Whether the installed Skyrim is the version the server needs; null if unknown.
    pub game_version_ok: Option<bool>,
    pub timings_ms: BTreeMap<String, u64>,
    #[serde(skip)]
    entered: Option<Instant>,
}

impl PlayTrace {
    pub fn new() -> PlayTrace {
        let mut t = PlayTrace::default();
        t.enter("Starting");
        t
    }

    fn close(&mut self) {
        if let Some(at) = self.entered.take() {
            let ms = at.elapsed().as_millis() as u64;
            *self.timings_ms.entry(self.stage.clone()).or_insert(0) += ms;
        }
    }

    /// The attempt moves on to the next step.
    pub fn enter(&mut self, stage: &str) {
        self.close();
        self.stage = clean(stage);
        self.entered = Some(Instant::now());
    }

    pub fn game_version(&mut self, ok: bool) {
        self.game_version_ok = Some(ok);
    }

    /// The attempt stopped in the current step.
    pub fn fail(&mut self) {
        self.close();
        self.failed_at = Some(self.stage.clone());
    }

    /// A copy with the running step's time counted so far.
    pub fn snapshot(&self) -> PlayTrace {
        let mut t = self.clone();
        t.close();
        t
    }
}

impl Report {
    pub fn with_play(mut self, trace: PlayTrace) -> Report {
        self.play = Some(trace);
        self
    }
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Mods {
    /// Mods the server's mods.json lists; null when the launcher couldn't
    /// read mods.json (so it isn't taken for "nothing missing").
    pub served: Option<usize>,
    pub installed: Option<usize>,
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

/// A name as the bot takes it: no ':' or control characters, at most 120
/// characters.
fn clean(name: &str) -> String {
    name.chars().filter(|c| *c != ':' && !c.is_control()).take(MAX_NAME).collect::<String>().trim().to_string()
}

/// The report: `list` is the merged mod list, `served` the ids the server's
/// mods.json names (None when it couldn't be read), `installed` whether a
/// listed mod is installed.
pub fn report(launcher: &str, list: &[ModEntry], served: Option<&BTreeSet<String>>, installed: impl Fn(&ModEntry) -> bool, required: &[(&str, bool)]) -> Report {
    let mods = match served {
        Some(served) => {
            let from_server: Vec<&ModEntry> = list.iter().filter(|m| served.contains(&m.id)).collect();
            let missing: Vec<&ModEntry> = from_server.iter().copied().filter(|m| !installed(m)).collect();
            Mods { served: Some(from_server.len()), installed: Some(from_server.len() - missing.len()), missing: missing.iter().map(|m| clean(&m.name)).take(MAX_NAMES).collect() }
        }
        None => Mods { served: None, installed: None, missing: vec![] },
    };
    let req_missing: Vec<String> = required.iter().filter(|(_, ok)| !ok).map(|(n, _)| clean(n)).collect();
    Report { launcher: launcher.to_string(), mods, required: Required { ok: req_missing.is_empty(), missing: req_missing }, play: None }
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
        let r = report("0.1.80", &list, Some(&served), |m| m.id == "skyui", &[("SKSE64", true), ("Crash Logger", false)]);
        assert_eq!(r.mods, Mods { served: Some(3), installed: Some(1), missing: vec!["Faster HDT-SMP".into(), "RaceMenu".into()] });
        assert_eq!(r.required, Required { ok: false, missing: vec!["Crash Logger".into()] });
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["launcher"], "0.1.80");
        assert_eq!(json["required"]["ok"], false);
        // Nothing but names and counts.
        assert!(!json.to_string().contains('/') && !json.to_string().contains('\\'));
    }

    #[test]
    fn names_fit_the_bots_limits_and_an_unread_list_is_null() {
        let mut list: Vec<ModEntry> = (0..150).map(|i| entry(&format!("m{i}"), &format!("Mod {i}"))).collect();
        list.push(entry("colon", &format!("Moons: and Stars{}", "x".repeat(200))));
        let served: BTreeSet<String> = list.iter().map(|m| m.id.clone()).collect();
        let r = report("0.1.81", &list, Some(&served), |_| false, &[]);
        assert_eq!(r.mods.served, Some(151));
        assert_eq!(r.mods.missing.len(), 100);
        assert!(r.required.ok);
        let long = clean(&list[150].name);
        assert!(!long.contains(':') && long.chars().count() == 120 && long.starts_with("Moons and Stars"));
        let none = report("0.1.81", &list, None, |_| false, &[]);
        let json = serde_json::to_value(&none).unwrap();
        assert!(json["mods"]["served"].is_null() && json["mods"]["installed"].is_null());
    }

    #[test]
    fn a_play_trace_names_the_step_it_stopped_in_and_times_each() {
        let mut t = PlayTrace::new();
        t.game_version(true);
        t.enter("Installing SKSE");
        std::thread::sleep(std::time::Duration::from_millis(15));
        t.enter("Getting your mods ready");
        t.fail();
        assert_eq!(t.stage, "Getting your mods ready");
        assert_eq!(t.failed_at.as_deref(), Some("Getting your mods ready"));
        assert!(t.timings_ms["Installing SKSE"] >= 10);
        assert!(t.timings_ms.contains_key("Starting") && t.timings_ms.contains_key("Getting your mods ready"));
        let list = vec![entry("skyui", "SkyUI")];
        let json = serde_json::to_value(report("0.1.99", &list, None, |_| false, &[]).with_play(t)).unwrap();
        assert_eq!(json["play"]["stage"], "Getting your mods ready");
        assert_eq!(json["play"]["failedAt"], "Getting your mods ready");
        assert_eq!(json["play"]["gameVersionOk"], true);
        assert!(json["play"]["timingsMs"]["Installing SKSE"].as_u64().is_some());
        assert!(!json["play"].to_string().contains('/') && !json["play"].to_string().contains('\\'));
    }

    #[test]
    fn a_report_without_a_trace_has_no_play_key_and_a_good_attempt_has_no_failed_at() {
        let list = vec![entry("skyui", "SkyUI")];
        assert!(serde_json::to_value(report("0.1.99", &list, None, |_| false, &[])).unwrap().get("play").is_none());
        let mut t = PlayTrace::new();
        t.enter("Starting Skyrim through SKSE");
        let json = serde_json::to_value(t.snapshot()).unwrap();
        assert!(json.get("failedAt").is_none() && json["gameVersionOk"].is_null());
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
