//! Client parity: what this PC's game has loaded, reported at join so the
//! server can compare it with its own pinned order (masters.json).
//!
//! The fingerprint reads the PC and changes nothing. It holds file NAMES and
//! hashes of name lists only: no paths, no file contents, no tokens. The same
//! comparison runs here as a "Same files as the server" health row that can
//! warn but never stops Play (0.1.143 rule). Switch: status.json
//! `launcher.parityReport:false` or rules `checks.parityReport:false`.
//!
//! Evidence labels: the edition is a GUESS from how many Creation Club files
//! are installed (INFERRED); the exe version, the active plugin order and the
//! file lists are read straight from the PC (CONFIRMED when sent).

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::Path;

use crate::health::{Check, Status};
use crate::serverorder::{self, ServerPlugin};

const SCHEMA: u32 = 1;
const MAX_LIST: usize = 500;
const MAX_CC: usize = 200;
const MAX_DIFF: usize = 20;
/// The Anniversary upgrade ships about 70 Creation Club files; a handful means single purchases.
const AE_LIKE_CC: usize = 50;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Fingerprint {
    pub schema: u32,
    pub launcher: String,
    /// SkyrimSE.exe file version, e.g. "1.6.1170.0".
    pub game_version: Option<String>,
    /// "AE" when 50+ Creation Club files are installed, else "SE" (INFERRED, see module docs).
    pub edition_guess: &'static str,
    pub cc_present_count: usize,
    /// cc* plugins in Data.
    pub cc_present: Vec<String>,
    /// cc* plugins switched on in plugins.txt.
    pub cc_active: Vec<String>,
    /// Plugin lines left in Skyrim.ccc (0 once Play has emptied it).
    pub ccc_lines: usize,
    /// cc* files the launcher set aside (.aetherial-dawn/disabled).
    pub cc_set_aside: Vec<String>,
    pub set_aside_count: usize,
    /// plugins.txt's switched-on plugins in file order (light ones included).
    pub active_order: Vec<String>,
    pub active_hash: String,
    /// The full plugins the game will load, base masters first (serverorder::game_order_with, Skyrim.ccc ignored).
    pub full_order: Vec<String>,
    pub full_hash: String,
    pub archive_count: usize,
    pub archives: Vec<String>,
    pub archives_hash: String,
    /// The launcher's own comparison with the server's list; None when it had no list.
    pub server_match: Option<bool>,
    pub server_diff: Vec<String>,
}

/// A file name that is safe to send: no path, no control characters.
fn name_ok(n: &str) -> bool {
    !n.is_empty() && n.len() <= 120 && !n.chars().any(|c| c.is_control() || matches!(c, '\\' | '/' | ':'))
}

fn hash_names(names: &[String]) -> String {
    let joined = names.iter().map(|n| n.to_ascii_lowercase()).collect::<Vec<_>>().join("\n");
    hex::encode(Sha256::digest(joined.as_bytes()))
}

fn is_cc(n: &str) -> bool {
    let l = n.to_ascii_lowercase();
    l.starts_with("cc") && [".esl", ".esm", ".esp"].iter().any(|x| l.ends_with(x))
}

/// Active plugin names in plugins.txt order (lines starting with `*`).
fn active_plugins(plugins_txt: &Path) -> Vec<String> {
    crate::loadorder::read_text(plugins_txt)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.trim().strip_prefix('*'))
        .map(|n| n.trim().to_string())
        .filter(|n| name_ok(n))
        .collect()
}

fn sorted_names(dir: &Path, keep: impl Fn(&str) -> bool) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| rd.flatten().filter(|e| e.path().is_file()).map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| name_ok(n) && keep(n)).collect())
        .unwrap_or_default();
    v.sort_by_key(|n| n.to_ascii_lowercase());
    v
}

/// File names under the launcher's set-aside folder (any stamp), once each.
fn set_aside_names(game_dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut stack = vec![game_dir.join(crate::aside::DISABLED_DIR)];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if let Some(n) = p.file_name().map(|n| n.to_string_lossy().into_owned()).filter(|n| name_ok(n)) {
                if !out.iter().any(|o| o.eq_ignore_ascii_case(&n)) {
                    out.push(n);
                }
            }
        }
    }
    out.sort_by_key(|n| n.to_ascii_lowercase());
    out
}

/// Where the game's plugin order differs from the server's: a plugin missing, or the wrong
/// plugin at a position. Names only; sizes and contents are the "serverorder" check's job.
pub fn diff_order(full_order: &[String], server: &[ServerPlugin]) -> Vec<String> {
    let mut out = Vec::new();
    for (i, s) in server.iter().enumerate() {
        match full_order.get(i) {
            None => out.push(format!("position {i}: the server loads {}, this PC loads nothing there", s.name)),
            Some(m) if !m.eq_ignore_ascii_case(&s.name) => out.push(format!("position {i}: the server loads {}, this PC loads {m}", s.name)),
            Some(_) => {}
        }
    }
    out
}

/// Reads the PC. `server` is the server's order when the launcher has it.
pub fn build(launcher: &str, game_dir: &Path, plugins_txt: &Path, server: Option<&[ServerPlugin]>) -> Fingerprint {
    let data = game_dir.join("Data");
    let game_version = crate::version::exe_version(&game_dir.join(crate::game::GAME_EXE)).map(|v| format!("{}.{}.{}.{}", v[0], v[1], v[2], v[3]));
    let cc_all = sorted_names(&data, is_cc);
    let cc_present_count = cc_all.len();
    let active_order: Vec<String> = active_plugins(plugins_txt).into_iter().take(MAX_LIST).collect();
    let cc_active: Vec<String> = active_order.iter().filter(|n| is_cc(n)).take(MAX_CC).cloned().collect();
    let ccc_lines = serverorder::ccc_line_count(game_dir);
    let aside = set_aside_names(game_dir);
    let full_order: Vec<String> = serverorder::game_order_with(game_dir, plugins_txt, false).into_iter().take(MAX_LIST).collect();
    let archives_all = sorted_names(&data, |n| n.to_ascii_lowercase().ends_with(".bsa"));
    let (server_match, server_diff) = match server.filter(|s| !s.is_empty()) {
        Some(s) => {
            let d = diff_order(&full_order, s);
            (Some(d.is_empty()), d.into_iter().take(MAX_DIFF).collect())
        }
        None => (None, Vec::new()),
    };
    Fingerprint {
        schema: SCHEMA,
        launcher: launcher.chars().filter(|c| !c.is_control()).take(32).collect(),
        game_version,
        edition_guess: if cc_present_count >= AE_LIKE_CC { "AE" } else { "SE" },
        cc_present_count,
        cc_present: cc_all.into_iter().take(MAX_CC).collect(),
        cc_active,
        ccc_lines,
        cc_set_aside: aside.iter().filter(|n| is_cc(n)).take(MAX_CC).cloned().collect(),
        set_aside_count: aside.len(),
        active_hash: hash_names(&active_order),
        active_order,
        full_hash: hash_names(&full_order),
        full_order,
        archive_count: archives_all.len(),
        archives_hash: hash_names(&archives_all),
        archives: archives_all.into_iter().take(MAX_LIST).collect(),
        server_match,
        server_diff,
    }
}

/// The health row. Ok when the game's order is the server's; Warn when it differs or Creation
/// Club plugins are switched on; Info when there was no server list. Never Fail: it cannot stop Play.
pub fn check(fp: &Fingerprint) -> Check {
    const TITLE: &str = "Same game files as the server";
    let mut items: Vec<String> = fp.server_diff.iter().take(5).cloned().collect();
    if !fp.cc_active.is_empty() {
        items.push(format!("{} Creation Club plugin(s) are switched on in plugins.txt (the server loads none)", fp.cc_active.len()));
    }
    let (status, detail) = match fp.server_match {
        None => (Status::Info, "The server's plugin list wasn't available, so this PC's files weren't compared.".to_string()),
        Some(true) if items.is_empty() => (Status::Ok, "This PC loads the server's plugins in the server's order.".to_string()),
        _ => (Status::Warn, "This PC's plugins differ from the server's. Play still starts; staff see the difference to find invisible items and sync problems.".to_string()),
    };
    Check { id: "parity", title: TITLE, status, detail, items }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tes4(flags: u32) -> Vec<u8> {
        let mut h = b"TES4".to_vec();
        h.extend_from_slice(&[0, 0, 0, 0]);
        h.extend_from_slice(&flags.to_le_bytes());
        h.extend_from_slice(&[0; 12]);
        h
    }

    fn sp(n: &str) -> ServerPlugin {
        ServerPlugin { name: n.into(), size: None, sha256: None, crc32: None }
    }

    /// A game folder with the base masters, one server plugin, two cc files and a mod archive.
    fn pc(tag: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let d = std::env::temp_dir().join(format!("ad-parity-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("Data")).unwrap();
        for b in serverorder::BASE {
            std::fs::write(d.join("Data").join(b), tes4(1)).unwrap();
        }
        std::fs::write(d.join("Data/AetherialDawn.esp"), tes4(0)).unwrap();
        std::fs::write(d.join("Data/ccBGSSSE001-Fish.esm"), tes4(1)).unwrap();
        std::fs::write(d.join("Data/ccQDRSSE001-SurvivalMode.esl"), tes4(0x200)).unwrap();
        std::fs::write(d.join("Data/Mod.bsa"), b"x").unwrap();
        let txt = d.join("plugins.txt");
        std::fs::write(&txt, "*AetherialDawn.esp\r\n*ccQDRSSE001-SurvivalMode.esl\r\nUnused.esp\r\n").unwrap();
        (d, txt)
    }

    fn server() -> Vec<ServerPlugin> {
        serverorder::BASE.iter().map(|b| sp(b)).chain([sp("AetherialDawn.esp")]).collect()
    }

    #[test]
    fn a_matching_pc_reports_names_hashes_and_no_difference() {
        let (d, txt) = pc("match");
        let fp = build("0.1.148", &d, &txt, Some(&server()));
        assert_eq!(fp.server_match, Some(true), "{:?}", fp.server_diff);
        assert_eq!(fp.edition_guess, "SE");
        assert_eq!(fp.cc_present_count, 2);
        assert_eq!(fp.cc_active, vec!["ccQDRSSE001-SurvivalMode.esl".to_string()]);
        assert_eq!(fp.active_order, vec!["AetherialDawn.esp".to_string(), "ccQDRSSE001-SurvivalMode.esl".to_string()]);
        assert_eq!(fp.archives, vec!["Mod.bsa".to_string()]);
        assert_eq!(fp.active_hash.len(), 64);
        assert_eq!(fp.full_order.last().map(String::as_str), Some("AetherialDawn.esp"));
        // A switched-on cc plugin is worth a warning even when the full order matches.
        let c = check(&fp);
        assert_eq!(c.status, Status::Warn);
        assert!(c.items.iter().any(|i| i.contains("Creation Club")));
    }

    #[test]
    fn a_missing_server_plugin_is_named_and_only_warns() {
        let (d, txt) = pc("missing");
        std::fs::remove_file(d.join("Data/AetherialDawn.esp")).unwrap();
        let fp = build("0.1.148", &d, &txt, Some(&server()));
        assert_eq!(fp.server_match, Some(false));
        assert_eq!(fp.server_diff, vec!["position 5: the server loads AetherialDawn.esp, this PC loads nothing there".to_string()]);
        let c = check(&fp);
        assert_eq!(c.status, Status::Warn, "parity can warn but never fail, so it cannot stop Play");
    }

    #[test]
    fn no_server_list_is_info_and_the_edition_follows_the_cc_count() {
        let (d, txt) = pc("info");
        for i in 0..50 {
            std::fs::write(d.join(format!("Data/ccXXXSSE{i:03}-Thing.esl")), tes4(0x200)).unwrap();
        }
        let fp = build("0.1.148", &d, &txt, None);
        assert_eq!(fp.server_match, None);
        assert_eq!(check(&fp).status, Status::Info);
        assert_eq!(fp.edition_guess, "AE");
    }

    #[test]
    fn nothing_but_names_leaves_the_pc() {
        let (d, txt) = pc("names");
        std::fs::create_dir_all(d.join(".aetherial-dawn/disabled/1/Data")).unwrap();
        std::fs::write(d.join(".aetherial-dawn/disabled/1/Data/ccAside.esl"), b"x").unwrap();
        let json = serde_json::to_string(&build("0.1.148", &d, &txt, Some(&server()))).unwrap();
        assert!(json.contains("\"ccSetAside\":[\"ccAside.esl\"]"), "{json}");
        assert!(!json.contains(&d.display().to_string()), "no game folder path in the report");
        assert!(!json.contains('\\'), "{json}");
        assert!(!name_ok("..\\evil.esp") && !name_ok("C:evil.esp") && name_ok("Good Name.esp"));
    }

    #[test]
    fn a_wrong_plugin_at_a_position_is_a_difference() {
        let order = vec!["Skyrim.esm".to_string(), "Other.esp".to_string()];
        let d = diff_order(&order, &[sp("Skyrim.esm"), sp("Update.esm")]);
        assert_eq!(d, vec!["position 1: the server loads Update.esm, this PC loads Other.esp".to_string()]);
    }

    /// The JSON the launcher sends is what contract/launcher-server.json (client-parity) says it is:
    /// every required field present, no field the contract doesn't list.
    #[test]
    fn the_sent_fields_are_exactly_the_contracts() {
        let (d, txt) = pc("contract");
        let sent = serde_json::to_value(build("0.1.148", &d, &txt, Some(&server()))).unwrap();
        let contract: serde_json::Value = serde_json::from_str(include_str!("../../contract/launcher-server.json")).unwrap();
        let e = contract["endpoints"].as_array().unwrap().iter().find(|e| e["id"] == "client-parity").expect("contract entry");
        let keys = |k: &str| e["request"][k].as_object().map(|o| o.keys().cloned().collect::<Vec<_>>()).unwrap_or_default();
        let (req, opt) = (keys("required"), keys("optional"));
        for k in &req {
            assert!(sent.get(k).is_some(), "required field {k} is not sent");
        }
        for k in sent.as_object().unwrap().keys() {
            assert!(req.contains(k) || opt.contains(k), "{k} is sent but not in the contract");
        }
    }
}
