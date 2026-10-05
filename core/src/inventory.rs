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
    // An entry checked only next to SkyrimSE.exe (Engine Fixes' preloader,
    // which shares its Nexus mod with the Data part) needs those files in a
    // Vortex record too, not just a deployed file from the same mod.
    let checks: Vec<&String> = if data_checks.is_empty() { m.check.iter().collect() } else { data_checks };
    Some(checks.iter().all(|c| mine.iter().any(|f| matches(c, &f.rel))))
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

/// Who holds the files of one mod the launcher installed straight into the
/// game folder (its ledger, `.aetherial-dawn/mods/installed.json`), against
/// Vortex's deployment records (Codex 5910357069: hand direct-Data packages
/// to Vortex without two owners). Read-only: nothing is moved or removed.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Ownership {
    pub id: String,
    pub name: String,
    /// Placed by the launcher, in the game folder, listed by no Vortex record.
    pub launcher_only: usize,
    /// Placed by the launcher and also listed by a Vortex record: two owners.
    pub both: usize,
    /// The Vortex mod folders those shared files come from.
    pub vortex_sources: Vec<String>,
    /// In the ledger but no longer in the game folder.
    pub gone: usize,
    /// Those files, game-relative.
    pub missing_paths: Vec<String>,
    /// Left alone at install because Vortex already had them there.
    pub left_to_vortex: usize,
}

/// Every mod in the launcher's ledger, with its files split by owner.
pub fn ownership(game_dir: &Path) -> Vec<Ownership> {
    let files = vortex_files(game_dir);
    let deployed: std::collections::HashMap<String, &str> = files.iter().map(|f| (f.rel.to_ascii_lowercase(), f.source.as_str())).collect();
    crate::modlist::load_installed(game_dir)
        .mods
        .into_iter()
        .map(|(id, m)| {
            let mut o = Ownership { id, name: m.name, launcher_only: 0, both: 0, vortex_sources: Vec::new(), gone: 0, missing_paths: Vec::new(), left_to_vortex: m.skipped.len() };
            for rel in &m.files {
                if crate::modlist::safe_rel(rel).is_none() || !game_dir.join(rel).is_file() {
                    o.gone += 1;
                    o.missing_paths.push(rel.clone());
                } else if let Some(source) = deployed.get(&rel.replace('\\', "/").to_ascii_lowercase()) {
                    o.both += 1;
                    if !o.vortex_sources.iter().any(|s| s == source) {
                        o.vortex_sources.push(source.to_string());
                    }
                } else {
                    o.launcher_only += 1;
                }
            }
            o.vortex_sources.sort();
            o.missing_paths.sort();
            o
        })
        .collect()
}

/// The ownership result as a versioned receipt for the diagnostics report
/// (Codex 5917004813). Deterministic for the same inputs: no time, no
/// absolute paths, no account; everything sorted. Deployment records prove
/// which Vortex folder a file came from, not that the mod is installed or
/// enabled in the Aetherial Dawn profile, so those stay "unknown" here.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnershipReceipt {
    pub schema: &'static str,
    pub launcher_version: String,
    /// False when an input couldn't be read or changed while it was read;
    /// `errors` then says which.
    pub complete: bool,
    pub errors: Vec<String>,
    /// Each input read, game-relative, with its sha256 (None when absent).
    pub inputs: Vec<ReceiptInput>,
    pub totals: OwnershipTotals,
    /// Vortex profile evidence isn't part of this receipt.
    pub vortex_installed: &'static str,
    pub vortex_enabled: &'static str,
    pub mods: Vec<Ownership>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceiptInput {
    pub path: String,
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnershipTotals {
    pub mods: usize,
    pub launcher_only_mods: usize,
    pub two_owner_mods: usize,
    pub mods_with_missing_files: usize,
    pub files_launcher_only: usize,
    pub files_two_owners: usize,
    pub files_missing: usize,
    pub files_left_to_vortex: usize,
}

/// The ledger and every Vortex deployment record, game-relative, sorted.
fn ownership_inputs(game_dir: &Path) -> Vec<String> {
    let mut out = vec![format!("{}/installed.json", crate::modlist::MODS_DIR), "Data/vortex.deployment.json".to_string()];
    if let Ok(entries) = std::fs::read_dir(game_dir) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let l = name.to_ascii_lowercase();
            if l.starts_with("vortex.deployment.") && l.ends_with(".json") && e.path().is_file() {
                out.push(name);
            }
        }
    }
    out.sort();
    out
}

fn read_inputs(game_dir: &Path, errors: &mut Vec<String>) -> Vec<ReceiptInput> {
    ownership_inputs(game_dir)
        .into_iter()
        .map(|path| {
            let bytes = std::fs::read(game_dir.join(&path)).ok();
            if let Some(b) = &bytes {
                let ok = match serde_json::from_slice::<serde_json::Value>(b) {
                    Ok(v) if path.ends_with("installed.json") => v.get("mods").is_none_or(|m| m.is_object()),
                    Ok(v) => v.get("files").is_some_and(|f| f.is_array()),
                    Err(_) => false,
                };
                if !ok {
                    errors.push(format!("{path}: not readable as expected"));
                }
            }
            ReceiptInput { path, sha256: bytes.map(|b| hex::encode(Sha256::digest(b))) }
        })
        .collect()
}

/// Reads the inputs, works out ownership, and reads the inputs again: if
/// they changed in between (Vortex deploying, the launcher installing), it
/// tries once more, then reports the receipt as incomplete. Writes nothing.
pub fn ownership_receipt(game_dir: &Path, launcher_version: &str) -> OwnershipReceipt {
    let mut attempt = 0;
    loop {
        let mut errors = Vec::new();
        let before = read_inputs(game_dir, &mut errors);
        let mods = ownership(game_dir);
        let after = read_inputs(game_dir, &mut Vec::new());
        attempt += 1;
        if before != after {
            if attempt < 2 {
                continue;
            }
            errors.push("the ledger or a Vortex deployment record changed while it was read".into());
        }
        let sum = |f: fn(&Ownership) -> usize| mods.iter().map(f).sum();
        let totals = OwnershipTotals {
            mods: mods.len(),
            launcher_only_mods: mods.iter().filter(|o| o.both == 0 && o.launcher_only > 0).count(),
            two_owner_mods: mods.iter().filter(|o| o.both > 0).count(),
            mods_with_missing_files: mods.iter().filter(|o| o.gone > 0).count(),
            files_launcher_only: sum(|o| o.launcher_only),
            files_two_owners: sum(|o| o.both),
            files_missing: sum(|o| o.gone),
            files_left_to_vortex: sum(|o| o.left_to_vortex),
        };
        return OwnershipReceipt {
            schema: "modOwnership/1",
            launcher_version: launcher_version.to_string(),
            complete: errors.is_empty(),
            errors,
            inputs: after,
            totals,
            vortex_installed: "unknown",
            vortex_enabled: "unknown",
            mods,
        };
    }
}

/// One line for the mods window and the log.
pub fn describe_ownership(list: &[Ownership], have_record: bool) -> String {
    if list.is_empty() {
        return "Installed by the launcher: none".into();
    }
    let only = list.iter().filter(|o| o.both == 0 && o.launcher_only > 0).count();
    let shared: Vec<&str> = list.iter().filter(|o| o.both > 0).map(|o| o.name.as_str()).collect();
    let mut out = format!("Installed by the launcher: {} mods · {} only in the launcher's files", list.len(), only);
    if !have_record {
        out.push_str(" · Vortex: no deployment found");
    } else if shared.is_empty() {
        out.push_str(" · none also deployed by Vortex");
    } else {
        let names = if shared.len() > 5 { format!("{} and {} more", shared[..5].join(", "), shared.len() - 5) } else { shared.join(", ") };
        out.push_str(&format!(" · {} also deployed by Vortex (two owners): {names}", shared.len()));
    }
    let gone = list.iter().filter(|o| o.gone > 0).count();
    if gone > 0 {
        out.push_str(&format!(" · {gone} with files no longer in the game folder"));
    }
    out
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
        assert_eq!(builtins.len(), 14);
        // Vortex's 11, deployed from folders named the way Vortex names them.
        // Engine Fixes' root preloader is never Vortex's (2026-10-05 split).
        let mut deployed = Vec::new();
        for m in builtins.iter().filter(|m| m.id != "address-library" && m.id != "mcm-helper" && m.id != "engine-fixes-preloader") {
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
        assert_eq!(absent, vec!["address-library", "engine-fixes-preloader", "mcm-helper"]);
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

    fn ledger(game: &Path, mods: &[(&str, &[&str], &[&str])]) {
        let mut all = crate::modlist::Installed::default();
        for (id, files, skipped) in mods {
            all.mods.insert(id.to_string(), crate::modlist::InstalledMod {
                name: format!("{id} mod"),
                files: files.iter().map(|f| f.to_string()).collect(),
                skipped: skipped.iter().map(|f| f.to_string()).collect(),
                ..Default::default()
            });
        }
        let p = game.join(crate::modlist::MODS_DIR);
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(p.join("installed.json"), serde_json::to_vec(&all).unwrap()).unwrap();
    }

    #[test]
    fn each_launcher_file_is_counted_by_who_else_claims_it() {
        let t = tempfile::tempdir().unwrap();
        let g = t.path();
        for f in ["Data/a.esp", "Data/Scripts/a.pex", "Data/b.esp", "Data/B/Tex.dds"] {
            put(g, f);
        }
        ledger(g, &[
            ("a", &["Data/a.esp", "Data/Scripts/a.pex"], &[]),
            ("b", &["Data/b.esp", "Data/B/Tex.dds", "Data/gone.esp"], &["Data/b.ini"]),
        ]);
        // Before Vortex deploys anything here.
        let own = ownership(g);
        assert_eq!(own.iter().map(|o| (o.launcher_only, o.both, o.gone)).collect::<Vec<_>>(), [(2, 0, 0), (2, 0, 1)]);
        assert!(!has_vortex_record(g));
        assert_eq!(describe_ownership(&own, false), "Installed by the launcher: 2 mods · 2 only in the launcher's files · Vortex: no deployment found · 1 with files no longer in the game folder");
        // Vortex deploys b's texture (paths compared the way Windows does).
        let dep = serde_json::json!({ "files": [
            { "relPath": "B\\tex.DDS", "source": "B Textures-123-1-0" },
            { "relPath": "other.esp", "source": "Other-9-1" },
        ] });
        std::fs::write(g.join("Data/vortex.deployment.json"), serde_json::to_vec(&dep).unwrap()).unwrap();
        let own = ownership(g);
        let b = own.iter().find(|o| o.id == "b").unwrap();
        assert_eq!((b.launcher_only, b.both, b.gone, b.left_to_vortex), (1, 1, 1, 1));
        assert_eq!(b.vortex_sources, ["B Textures-123-1-0"]);
        assert_eq!(own.iter().find(|o| o.id == "a").unwrap().both, 0);
        assert_eq!(describe_ownership(&own, has_vortex_record(g)), "Installed by the launcher: 2 mods · 1 only in the launcher's files · 1 also deployed by Vortex (two owners): b mod · 1 with files no longer in the game folder");
        // Nothing was written by the report.
        assert!(!g.join("Data/b.esp.aetherial-part").exists());
        assert_eq!(describe_ownership(&[], true), "Installed by the launcher: none");
    }

    /// Every file under a folder with its bytes, to prove nothing changed.
    fn snapshot(dir: &Path) -> Vec<(String, Vec<u8>)> {
        let mut out = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap().flatten() {
                if e.file_type().unwrap().is_dir() {
                    stack.push(e.path());
                } else {
                    out.push((e.path().strip_prefix(dir).unwrap().to_string_lossy().into_owned(), std::fs::read(e.path()).unwrap()));
                }
            }
        }
        out.sort();
        out
    }

    #[test]
    fn the_ownership_receipt_is_stable_private_read_only_and_honest_about_gaps() {
        let t = tempfile::tempdir().unwrap();
        let g = t.path();
        put(g, "Data/a.esp");
        put(g, "Data/Shared.dds");
        ledger(g, &[("a", &["Data/a.esp", "Data/Shared.dds", "Data/gone.esp"], &["Data/a.ini"])]);
        let dep = serde_json::json!({ "files": [{ "relPath": "SHARED.dds", "source": "Shared-5-1" }] });
        std::fs::write(g.join("Data/vortex.deployment.json"), serde_json::to_vec(&dep).unwrap()).unwrap();
        std::fs::write(g.join("vortex.deployment.dinput.json"), br#"{"files":[]}"#).unwrap();
        let before = snapshot(g);
        let r = ownership_receipt(g, "0.1.99");
        assert_eq!(snapshot(g), before, "the receipt writes nothing");
        assert!(r.complete, "{:?}", r.errors);
        assert_eq!((r.schema, r.vortex_installed, r.vortex_enabled), ("modOwnership/1", "unknown", "unknown"));
        assert_eq!(r.inputs.iter().map(|i| i.path.as_str()).collect::<Vec<_>>(), [".aetherial-dawn/mods/installed.json", "Data/vortex.deployment.json", "vortex.deployment.dinput.json"]);
        assert!(r.inputs.iter().all(|i| i.sha256.as_ref().is_some_and(|h| h.len() == 64)));
        let t2 = &r.totals;
        assert_eq!((t2.mods, t2.two_owner_mods, t2.files_launcher_only, t2.files_two_owners, t2.files_missing, t2.files_left_to_vortex), (1, 1, 1, 1, 1, 1));
        assert_eq!(r.mods[0].missing_paths, ["Data/gone.esp"]);
        assert_eq!(r.mods[0].vortex_sources, ["Shared-5-1"]);
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(json, serde_json::to_string(&ownership_receipt(g, "0.1.99")).unwrap(), "the same inputs give the same receipt");
        assert!(!json.contains(&*g.to_string_lossy()), "no absolute path in the receipt");
        assert!(json.contains("\"modOwnership/1\"") && json.contains("\"twoOwnerMods\":1"));
        // A record that isn't one, and a broken ledger, make it incomplete and say so.
        std::fs::write(g.join("vortex.deployment.dinput.json"), b"not json").unwrap();
        std::fs::write(g.join(".aetherial-dawn/mods/installed.json"), b"{\"mods\": 3}").unwrap();
        let r = ownership_receipt(g, "0.1.99");
        assert!(!r.complete);
        assert_eq!(r.errors, [".aetherial-dawn/mods/installed.json: not readable as expected", "vortex.deployment.dinput.json: not readable as expected"]);
        // No ledger and no Vortex: complete, empty, and the absent inputs named.
        let t = tempfile::tempdir().unwrap();
        let r = ownership_receipt(t.path(), "0.1.99");
        assert!(r.complete && r.mods.is_empty());
        assert!(r.inputs.iter().all(|i| i.sha256.is_none()) && r.inputs.len() == 2);
    }
}
