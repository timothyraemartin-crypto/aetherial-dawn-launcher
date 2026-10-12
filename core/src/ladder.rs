//! The "never connected" ladder. A game that ran, reached the main menu and
//! never loaded a world (see `netcheck::never_connected`) is written down in
//! `.aetherial-dawn/connect-ladder.json`. The next Play then also sets the
//! official Creation Club files in Data aside for the session (the same
//! journal as other mods, `profile.rs`), so a PC whose Creation Club content
//! upsets the game gets a run without it with no hand steps. A game that
//! reaches a world clears the record. Switch: status.json `launcher.connectLadder`: false.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::modguard::Extra;

const FILE: &str = ".aetherial-dawn/connect-ladder.json";

fn path(game_dir: &Path) -> PathBuf {
    game_dir.join(FILE)
}

/// How many games in a row ran to the main menu and never connected (0 when
/// there is no record or it cannot be read).
pub fn level(game_dir: &Path) -> u32 {
    std::fs::read(path(game_dir)).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()).and_then(|v| v.get("never").and_then(|n| n.as_u64())).unwrap_or(0).min(99) as u32
}

/// After a game: `never` adds one to the level, `connected` (a world loaded)
/// clears it, anything else (a short run, a crash) leaves it. Returns the new level.
pub fn record(game_dir: &Path, never: bool, connected: bool, now_secs: u64) -> u32 {
    let now = level(game_dir);
    if connected {
        let _ = std::fs::remove_file(path(game_dir));
        return 0;
    }
    if !never {
        return now;
    }
    let next = (now + 1).min(99);
    let body = serde_json::json!({ "never": next, "at": now_secs });
    if let Ok(b) = serde_json::to_vec_pretty(&body) {
        let _ = std::fs::create_dir_all(game_dir.join(".aetherial-dawn"));
        let _ = crate::atomicfile::write(&path(game_dir), &b);
    }
    next
}

/// The official Creation Club files directly in Data (plugins and archives),
/// except any the server itself lists (`keep`: its plugin names).
pub fn cc_extras(game_dir: &Path, keep: &[String]) -> Vec<Extra> {
    let keep: HashSet<String> = keep.iter().map(|k| k.to_ascii_lowercase()).collect();
    let keep_stems: HashSet<String> = keep.iter().map(|k| k.rsplit_once('.').map_or(k.clone(), |(s, _)| s.to_string())).collect();
    let Ok(rd) = std::fs::read_dir(game_dir.join("Data")) else { return vec![] };
    let mut names: Vec<String> = rd.flatten().filter(|e| e.path().is_file()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    names.sort_by_key(|n| n.to_ascii_lowercase());
    names
        .into_iter()
        .filter(|n| {
            let l = n.to_ascii_lowercase();
            let (stem, ext) = l.rsplit_once('.').unwrap_or((l.as_str(), ""));
            crate::modguard::is_cc(&l) && matches!(ext, "esm" | "esl" | "esp" | "bsa" | "ba2") && !keep.contains(&l) && !keep_stems.contains(stem)
        })
        .map(|n| Extra { kind: "cc", rel: format!("Data/{n}") })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ad-ladder-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("Data")).unwrap();
        d
    }

    #[test]
    fn a_game_that_never_connected_raises_the_level_and_a_world_clears_it() {
        let g = tmp("level");
        assert_eq!(level(&g), 0);
        assert_eq!(record(&g, true, false, 10), 1);
        assert_eq!(record(&g, true, false, 20), 2);
        assert_eq!(record(&g, false, false, 30), 2, "a short run or a crash changes nothing");
        assert_eq!(level(&g), 2);
        assert_eq!(record(&g, false, true, 40), 0);
        assert_eq!(level(&g), 0);
        let _ = std::fs::remove_dir_all(&g);
    }

    #[test]
    fn an_unreadable_record_counts_as_none() {
        let g = tmp("bad");
        std::fs::create_dir_all(g.join(".aetherial-dawn")).unwrap();
        std::fs::write(g.join(FILE), b"{not json").unwrap();
        assert_eq!(level(&g), 0);
        let _ = std::fs::remove_dir_all(&g);
    }

    #[test]
    fn only_official_creation_club_files_are_picked_and_the_servers_own_are_kept() {
        let g = tmp("cc");
        for f in ["ccbgssse001-fish.esm", "ccbgssse001-fish.bsa", "cckrtsse001_altar.esl", "ccbgssse002-exoticarrows.esl", "Skyrim.esm", "_ResourcePack.esl", "AetherialDawn.esp", "MyMod.esp", "cctest.esp", "Skyrim - Textures0.bsa"] {
            std::fs::write(g.join("Data").join(f), b"x").unwrap();
        }
        let picked: Vec<String> = cc_extras(&g, &["ccBGSSSE002-ExoticArrows.esl".to_string()]).into_iter().map(|e| e.rel).collect();
        assert_eq!(picked, ["Data/ccbgssse001-fish.bsa", "Data/ccbgssse001-fish.esm", "Data/cckrtsse001_altar.esl"]);
        let _ = std::fs::remove_dir_all(&g);
    }
}
