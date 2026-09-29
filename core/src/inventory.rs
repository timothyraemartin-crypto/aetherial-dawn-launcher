//! Two inventories, never mixed (Codex's Package A, PR #7, 2026-09-28).
//!
//! **Game files** says whether a mod's files are in the game folder (the
//! launcher's own check, `ModEntry::installed`). **Vortex** says whether
//! Vortex deployed the mod: its check files are in `Data/vortex.deployment.json`
//! from a Vortex mod folder carrying the mod's Nexus id. Vortex's deployment
//! record is read, never written. It proves "deployed by Vortex", not
//! "enabled in the Aetherial Dawn profile"; profile membership is only ever
//! reported by the Aetherial Dawn Vortex extension (Package C), and until
//! it has reported, the profile count is unknown, never guessed.
//!
//! A small number of mods missing from the game files is never proof that
//! Vortex has the rest: Timothy's profile had 11 packages while the launcher
//! counted 36 of 37 feed mods as installed.

use std::path::Path;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::allowlist::{vortex_files, VortexFile};
use crate::modlist::{ModEntry, ModList};

/// Where a mod stands in each inventory.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Standing {
    /// Its files are in the game folder (however they got there).
    pub game_files: bool,
    /// Vortex deployed it; None when Vortex has no deployment record here.
    pub vortex_deployed: Option<bool>,
}

/// Counts for a list, each naming what it measures.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    pub listed: usize,
    /// Mods whose files are in the game folder.
    pub game_files_present: usize,
    /// Mods Vortex deployed (from its deployment record); None without one.
    pub vortex_deployed: Option<usize>,
    /// Mods enabled and deployed in the Aetherial Dawn profile, as the
    /// extension reported them; None until it has.
    pub vortex_profile: Option<usize>,
}

impl Counts {
    /// Plain words for the mods window and the log.
    pub fn describe(&self) -> String {
        let mut out = format!("Game files: {} of {} present", self.game_files_present, self.listed);
        match self.vortex_deployed {
            Some(n) => out.push_str(&format!(" · Vortex: {n} of {} deployed", self.listed)),
            None => out.push_str(" · Vortex: no deployment found"),
        }
        match self.vortex_profile {
            Some(n) => out.push_str(&format!(" · Aetherial Dawn profile: {n} of {} enabled and deployed", self.listed)),
            None => out.push_str(" · Aetherial Dawn profile: not checked yet"),
        }
        out
    }
}

fn nexus_id_in(source: &str, id: u64) -> bool {
    let id = id.to_string();
    source.split(|c: char| !c.is_ascii_digit()).any(|t| t == id)
}

/// A game-relative path against a check that may hold one `*` in its file name.
fn matches(check: &str, rel: &str) -> bool {
    let (c, r) = (check.replace('\\', "/").to_ascii_lowercase(), rel.replace('\\', "/").to_ascii_lowercase());
    match c.split_once('*') {
        None => c == r,
        Some((head, tail)) => r.len() >= head.len() + tail.len() && r.starts_with(head) && r.ends_with(tail) && !r[head.len()..r.len() - tail.len()].contains('/'),
    }
}

/// Whether Vortex deployed this mod: it has a Nexus id, a Vortex mod folder
/// carrying that id deployed files, and every check file under Data came
/// from such a folder. None when there's no deployment record.
pub fn vortex_deployed(m: &ModEntry, files: &[VortexFile], have_record: bool) -> Option<bool> {
    if !have_record {
        return None;
    }
    let Some(n) = &m.nexus else { return Some(false) };
    let mine: Vec<&VortexFile> = files.iter().filter(|f| nexus_id_in(&f.source, n.mod_id)).collect();
    if mine.is_empty() {
        return Some(false);
    }
    let data_checks: Vec<&String> = m.check.iter().filter(|c| c.replace('\\', "/").to_ascii_lowercase().starts_with("data/")).collect();
    Some(data_checks.iter().all(|c| mine.iter().any(|f| matches(c, &f.rel))))
}

pub fn standing(m: &ModEntry, game_dir: &Path, files: &[VortexFile], have_record: bool) -> Standing {
    Standing { game_files: m.installed(game_dir), vortex_deployed: vortex_deployed(m, files, have_record) }
}

/// Whether Vortex keeps a Data or game-root deployment record here.
pub fn has_vortex_record(game_dir: &Path) -> bool {
    game_dir.join("Data").join("vortex.deployment.json").is_file()
        || std::fs::read_dir(game_dir).map(|entries| entries.flatten().any(|entry| {
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            name.starts_with("vortex.deployment.") && name.ends_with(".json") && entry.path().is_file()
        })).unwrap_or(false)
}

/// Both inventories for a list.
pub fn count(list: &[ModEntry], game_dir: &Path) -> (Vec<Standing>, Counts) {
    let record = has_vortex_record(game_dir);
    let files = vortex_files(game_dir);
    let st: Vec<Standing> = list.iter().map(|m| standing(m, game_dir, &files, record)).collect();
    let counts = Counts {
        listed: list.len(),
        game_files_present: st.iter().filter(|s| s.game_files).count(),
        vortex_deployed: record.then(|| st.iter().filter(|s| s.vortex_deployed == Some(true)).count()),
        vortex_profile: None,
    };
    (st, counts)
}

/// What the launcher was served as the mod list: the bytes' sha256, the
/// list's own revision when it names one, and its size. Shown in the mods
/// window and logged, so a count is always tied to the exact list it counts.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FeedReceipt {
    pub sha256: String,
    pub revision: Option<String>,
    pub entries: usize,
}

impl FeedReceipt {
    pub fn of(bytes: &[u8], list: &ModList) -> FeedReceipt {
        FeedReceipt { sha256: hex::encode(Sha256::digest(bytes)), revision: list.revision.clone(), entries: list.mods.len() }
    }

    pub fn describe(&self) -> String {
        let short = &self.sha256[..12.min(self.sha256.len())];
        match &self.revision {
            Some(r) => format!("mod list revision {r}, {} entries, sha256 {short}…", self.entries),
            None => format!("mod list without a revision, {} entries, sha256 {short}…", self.entries),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modlist::{builtin, NexusRef};

    fn vf(rel: &str, source: &str) -> VortexFile {
        VortexFile { rel: rel.into(), source: source.into() }
    }

    fn put(game: &Path, rel: &str) {
        let p = game.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        if rel.to_ascii_lowercase().ends_with(".esp") || rel.to_ascii_lowercase().ends_with(".esm") {
            // A sound plugin header, so the game-files check counts it.
            let mut b = b"TES4".to_vec();
            b.extend(18u32.to_le_bytes());
            b.extend([0u8; 16]);
            b.extend(b"HEDR");
            b.extend(12u16.to_le_bytes());
            b.extend(1.71f32.to_le_bytes());
            b.extend([0u8; 8]);
            std::fs::write(p, b).unwrap();
        } else {
            std::fs::write(p, b"x").unwrap();
        }
    }

    /// Codex's verified facts (PR #7, 5875667961): Vortex holds the 11
    /// built-ins in Timothy's screenshot and none of the 37 feed mods;
    /// Address Library and MCM Helper are the two built-ins it lacks; the
    /// game files hold 36 of the 37 feed mods (RaceMenu is the one missing).
    #[test]
    fn the_verified_11_37_2_are_counted_as_two_inventories() {
        let t = tempfile::tempdir().unwrap();
        let game = t.path();
        let builtins = builtin(Some("1.6.1170.0"));
        assert_eq!(builtins.len(), 13);
        // Vortex's 11, deployed from folders named the way Vortex names them.
        let mut deployed = Vec::new();
        for m in builtins.iter().filter(|m| m.id != "address-library" && m.id != "mcm-helper") {
            let n = m.nexus.as_ref().unwrap();
            let source = format!("{}-{}-1-0-1727000000", m.name, n.mod_id);
            for c in &m.check {
                let rel = c.replace('*', "1");
                put(game, &rel);
                deployed.push(serde_json::json!({ "relPath": rel.trim_start_matches("Data/"), "source": source }));
            }
            if m.check.is_empty() {
                deployed.push(serde_json::json!({ "relPath": format!("{}.txt", m.id), "source": source }));
            }
        }
        std::fs::write(game.join("Data/vortex.deployment.json"), serde_json::to_vec(&serde_json::json!({ "files": deployed })).unwrap()).unwrap();
        // The 37 feed mods: 36 with their files in Data (put there by the
        // launcher, not Vortex), RaceMenu without.
        let mut feed = Vec::new();
        for i in 0..37u64 {
            let id = if i == 31 { "racemenu".to_string() } else { format!("feed-{i:02}") };
            let m = ModEntry { id: id.clone(), name: id.clone(), nexus: Some(NexusRef { mod_id: 700_000 + i, file: Some(1), pick: None }), check: vec![format!("Data/{id}.esp")], ..Default::default() };
            if id != "racemenu" {
                put(game, &m.check[0]);
            }
            feed.push(m);
        }
        let (_, f) = count(&feed, game);
        assert_eq!(f, Counts { listed: 37, game_files_present: 36, vortex_deployed: Some(0), vortex_profile: None });
        let deployed_builtins: Vec<String> = {
            let files = vortex_files(game);
            builtins.iter().filter(|m| vortex_deployed(m, &files, true) == Some(true)).map(|m| m.id.clone()).collect()
        };
        assert_eq!(deployed_builtins.len(), 11, "{deployed_builtins:?}");
        let absent: Vec<&str> = builtins.iter().map(|m| m.id.as_str()).filter(|id| !deployed_builtins.iter().any(|d| d == id)).collect();
        assert_eq!(absent, vec!["address-library", "mcm-helper"]);
        // The words never let the small game-files gap stand for Vortex.
        assert_eq!(f.describe(), "Game files: 36 of 37 present · Vortex: 0 of 37 deployed · Aetherial Dawn profile: not checked yet");
    }

    #[test]
    fn without_a_deployment_record_vortex_is_unknown_not_zero_or_all() {
        let t = tempfile::tempdir().unwrap();
        let m = ModEntry { id: "a".into(), nexus: Some(NexusRef { mod_id: 5, file: None, pick: None }), check: vec!["Data/a.esp".into()], ..Default::default() };
        let (st, c) = count(&[m], t.path());
        assert_eq!(st[0].vortex_deployed, None);
        assert_eq!(c.vortex_deployed, None);
        assert!(c.describe().contains("Vortex: no deployment found"));
    }

    #[test]
    fn a_check_file_from_another_mods_folder_does_not_count() {
        let m = ModEntry { id: "a".into(), nexus: Some(NexusRef { mod_id: 266, file: None, pick: None }), check: vec!["Data/A.esp".into(), "Data/SKSE/Plugins/a.dll".into()], ..Default::default() };
        // 266 inside 12660 is not 266; and one check file from another mod.
        let files = [vf("Data/A.esp", "Something-12660-1"), vf("Data/SKSE/Plugins/a.dll", "USSEP-266-4-3-9c-1")];
        assert_eq!(vortex_deployed(&m, &files, true), Some(false));
        let files = [vf("Data/a.esp", "USSEP 266 4.3.8a"), vf("Data/SKSE/Plugins/A.dll", "USSEP-266-4-3-9c-1")];
        assert_eq!(vortex_deployed(&m, &files, true), Some(true));
    }

    #[test]
    fn a_preset_slot_check_matches_any_slot() {
        assert!(matches("Data/SKSE/Plugins/SmoothCam/Presets/Modern*.json", "Data/SKSE/Plugins/SmoothCam/Presets/Modern3.json"));
        assert!(!matches("Data/SKSE/Plugins/SmoothCam/Presets/Modern*.json", "Data/SKSE/Plugins/SmoothCam/Presets/x/Modern3.json"));
    }

    #[test]
    fn the_receipt_names_the_exact_list() {
        let bytes = br#"{"revision":"3.4.6","mods":[{"id":"a","name":"A","url":"https://x/a.zip"}]}"#;
        let list: ModList = serde_json::from_slice(bytes).unwrap();
        let r = FeedReceipt::of(bytes, &list);
        assert_eq!(r.revision.as_deref(), Some("3.4.6"));
        assert_eq!(r.entries, 1);
        assert_eq!(r.sha256.len(), 64);
        assert!(r.describe().starts_with("mod list revision 3.4.6, 1 entries, sha256 "));
        let bare: ModList = serde_json::from_slice(br#"{"mods":[]}"#).unwrap();
        assert!(FeedReceipt::of(b"{\"mods\":[]}", &bare).describe().starts_with("mod list without a revision"));
    }
}
