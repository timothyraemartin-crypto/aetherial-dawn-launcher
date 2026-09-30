//! What uninstalling the launcher puts back on the player's PC (D18,
//! `docs/qa/PLAN-D18-UNINSTALL.md`, the "Settings only" default):
//!
//! - U1: Steam may update Skyrim again (read-only cleared, auto-update on).
//! - U2: nothing; the game stays on the server's build.
//! - U3: files the launcher set aside go back; the empty folder goes.
//! - U4: nothing; the launcher's mods stay in Data.
//! - U5: the player's own plugins.txt and loadorder.txt come back.
//! - U6: an emptied Skyrim.ccc comes back.
//! - U7: left to the installer's own "delete app data" box.
//! - U8: the game's remembered login and session are forgotten, as when
//!   signing out (`settings::clear_login`); the server address stays.
//! - U9-U11: nothing; the repaired Skyrim ini files (and their backups), the
//!   Windows graphics-card preference and Skyrim Platform's folders stay.
//!
//! Every step runs on its own and only reports what went wrong, so one
//! failure never stops the others or the uninstall.

use std::path::Path;

/// Skyrim Special Edition's Steam app id.
const SKYRIM_APP: u32 = 489830;
const LISTS: [&str; 2] = ["plugins.txt", "loadorder.txt"];

/// Runs every step. `game_dir` is the Skyrim folder from the launcher's
/// settings, `saves_dir` is `%LOCALAPPDATA%\Skyrim Special Edition`. Returns
/// one line per step for the uninstall log.
pub fn cleanup(game_dir: Option<&Path>, saves_dir: Option<&Path>) -> Vec<String> {
    let mut out = Vec::new();
    match game_dir.filter(|d| d.is_dir()) {
        None => out.push("U1 U3 U6: no Skyrim folder in the settings; skipped".into()),
        Some(game) => {
            // Only a manifest the launcher itself held (after putting the
            // server's build in place) is released; a player's own Steam
            // setting is never changed.
            out.push(if !crate::version::made_by_launcher(game) {
                "U1: the launcher didn't hold Steam updates here; left as it is".into()
            } else {
                match crate::version::release_updates(game, SKYRIM_APP) {
                    Ok(Some(_)) => "U1: Steam may update Skyrim again".into(),
                    Ok(None) => "U1: no Steam manifest found; nothing to do".into(),
                    Err(e) => format!("U1: couldn't change the Steam manifest ({e})"),
                }
            });
            out.push(match crate::allowlist::restore_all(game) {
                Ok(n) => {
                    let gone = remove_empty_tree(&game.join(crate::strays::DISABLED_DIR));
                    format!("U3: {n} set-aside file(s) put back; folder {}", if gone { "removed" } else { "kept (files are still in it)" })
                }
                Err(e) => format!("U3: couldn't put the set-aside files back ({e})"),
            });
            crate::settings::clear_login(game);
            out.push("U8: the game's remembered login and session forgotten".into());
            out.push(match crate::serverorder::restore_ccc(game) {
                Ok(true) => "U6: Skyrim.ccc put back".into(),
                Ok(false) => "U6: nothing to put back".into(),
                Err(e) => format!("U6: couldn't put Skyrim.ccc back ({e})"),
            });
        }
    }
    match saves_dir.filter(|d| d.is_dir()) {
        None => out.push("U5: no Skyrim Special Edition folder in local app data; skipped".into()),
        Some(saves) => {
            for name in LISTS {
                out.push(format!("U5: {name}: {}", restore_list(&saves.join(name))));
            }
        }
    }
    out
}

/// Moves `<list>.aetherial-dawn-backup` back over `<list>`. The backup is
/// the player's own list from before the launcher's first change.
fn restore_list(list: &Path) -> String {
    let backup = list.with_extension("txt.aetherial-dawn-backup");
    if !backup.is_file() {
        return "no backup; left as it is".into();
    }
    match std::fs::rename(&backup, list) {
        Ok(()) => "the player's own list put back".into(),
        Err(e) => format!("couldn't put it back ({e})"),
    }
}

/// Removes `dir` if it holds only empty folders. Returns whether it is gone.
fn remove_empty_tree(dir: &Path) -> bool {
    fn prune(d: &Path) -> bool {
        let Ok(rd) = std::fs::read_dir(d) else { return false };
        let mut empty = true;
        for e in rd.flatten() {
            let p = e.path();
            if !(p.is_dir() && prune(&p)) {
                empty = false;
            }
        }
        empty && std::fs::remove_dir(d).is_ok()
    }
    !dir.exists() || prune(dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// The same library and player files `docs/qa/sandbox-fixture.ps1` builds.
    fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let t = tempfile::tempdir().unwrap();
        let lib = t.path().join("SteamLibrary/steamapps");
        let game = lib.join("common/Skyrim Special Edition");
        let saves = t.path().join("saves");
        let w = |p: PathBuf, s: &str| {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, s).unwrap();
        };
        let acf = lib.join("appmanifest_489830.acf");
        w(acf.clone(), "\"AppState\"\n{\n\t\"appid\"\t\t\"489830\"\n\t\"AutoUpdateBehavior\"\t\t\"1\"\n}\n");
        let mut perm = std::fs::metadata(&acf).unwrap().permissions();
        perm.set_readonly(true);
        std::fs::set_permissions(&acf, perm).unwrap();
        w(game.join("SkyrimSE.exe"), "exe");
        // The record the launcher writes after putting the server's build in place.
        w(game.join(".aetherial-dawn/game.json"), r#"{"version":"1.6.1170.0","depots":[],"files":[],"manual":false}"#);
        w(game.join(".aetherial-dawn/mods/feed-mod.json"), "{}");
        w(game.join(".aetherial-dawn/disabled/1727000000-strays/Data/PlayersOwn.esp"), "own");
        w(game.join("Data/FeedMod.esp"), "feed");
        w(game.join("Data/VortexOwned.esp"), "vortex");
        w(game.join("Skyrim.ccc"), "");
        w(game.join(crate::settings::AUTH_DATA_PATH), "//{}");
        w(game.join("Skyrim.ccc.aetherial-dawn-backup"), "ccBGSSSE001-Fish.esm");
        w(saves.join("plugins.txt"), "*FeedMod.esp");
        w(saves.join("plugins.txt.aetherial-dawn-backup"), "*PlayersOwn.esp");
        w(saves.join("loadorder.txt"), "Skyrim.esm\nFeedMod.esp");
        w(saves.join("loadorder.txt.aetherial-dawn-backup"), "Skyrim.esm\nPlayersOwn.esp");
        (t, game, saves)
    }

    fn read(p: &Path) -> String {
        std::fs::read_to_string(p).unwrap()
    }

    #[test]
    fn settings_only_puts_back_what_the_launcher_changed_and_nothing_else() {
        let (_t, game, saves) = fixture();
        let log = cleanup(Some(&game), Some(&saves));
        assert!(!log.iter().any(|l| l.contains("couldn't")), "{log:?}");
        // U1
        let acf = game.parent().unwrap().parent().unwrap().join("appmanifest_489830.acf");
        assert!(!std::fs::metadata(&acf).unwrap().permissions().readonly());
        assert!(read(&acf).contains("\"AutoUpdateBehavior\"\t\t\"0\""), "{}", read(&acf));
        // U2 and U4: untouched.
        assert_eq!(read(&game.join("SkyrimSE.exe")), "exe");
        assert_eq!(read(&game.join("Data/FeedMod.esp")), "feed");
        assert_eq!(read(&game.join("Data/VortexOwned.esp")), "vortex");
        // U3
        assert_eq!(read(&game.join("Data/PlayersOwn.esp")), "own");
        assert!(!game.join(crate::strays::DISABLED_DIR).exists());
        // U5
        assert_eq!(read(&saves.join("plugins.txt")), "*PlayersOwn.esp");
        assert_eq!(read(&saves.join("loadorder.txt")), "Skyrim.esm\nPlayersOwn.esp");
        assert!(!saves.join("plugins.txt.aetherial-dawn-backup").exists());
        assert!(!saves.join("loadorder.txt.aetherial-dawn-backup").exists());
        // U6
        assert_eq!(read(&game.join("Skyrim.ccc")), "ccBGSSSE001-Fish.esm");
        assert!(!game.join("Skyrim.ccc.aetherial-dawn-backup").exists());
        // U8
        assert!(!game.join(crate::settings::AUTH_DATA_PATH).exists());
    }

    #[test]
    fn running_it_twice_changes_nothing_more() {
        let (_t, game, saves) = fixture();
        cleanup(Some(&game), Some(&saves));
        let before = (read(&saves.join("plugins.txt")), read(&game.join("Skyrim.ccc")));
        let log = cleanup(Some(&game), Some(&saves));
        assert!(!log.iter().any(|l| l.contains("couldn't")), "{log:?}");
        assert_eq!(before, (read(&saves.join("plugins.txt")), read(&game.join("Skyrim.ccc"))));
    }

    #[test]
    fn a_newer_file_in_the_way_keeps_the_set_aside_copy() {
        let (_t, game, saves) = fixture();
        std::fs::write(game.join("Data/PlayersOwn.esp"), "newer").unwrap();
        let log = cleanup(Some(&game), Some(&saves));
        assert_eq!(read(&game.join("Data/PlayersOwn.esp")), "newer");
        assert!(game.join(".aetherial-dawn/disabled/1727000000-strays/Data/PlayersOwn.esp").is_file());
        assert!(log.iter().any(|l| l.starts_with("U3: 0 ") && l.contains("kept")), "{log:?}");
    }

    #[test]
    fn missing_folders_are_skipped_without_failing() {
        let t = tempfile::tempdir().unwrap();
        let log = cleanup(None, Some(&t.path().join("nope")));
        assert_eq!(log.len(), 2, "{log:?}");
        assert!(log.iter().all(|l| l.contains("skipped")), "{log:?}");
    }

    #[test]
    fn one_failing_step_does_not_stop_the_rest() {
        let (_t, game, saves) = fixture();
        // A folder where plugins.txt should be: that step fails, the rest run.
        std::fs::remove_file(saves.join("plugins.txt")).unwrap();
        std::fs::create_dir(saves.join("plugins.txt")).unwrap();
        let log = cleanup(Some(&game), Some(&saves));
        assert!(log.iter().any(|l| l.starts_with("U5: plugins.txt: couldn't")), "{log:?}");
        assert!(saves.join("plugins.txt.aetherial-dawn-backup").is_file());
        assert_eq!(read(&saves.join("loadorder.txt")), "Skyrim.esm\nPlayersOwn.esp");
        assert_eq!(read(&game.join("Skyrim.ccc")), "ccBGSSSE001-Fish.esm");
        assert_eq!(read(&game.join("Data/PlayersOwn.esp")), "own");
    }

    #[test]
    fn a_manifest_the_launcher_never_held_is_left_alone() {
        for marker in [None, Some(r#"{"version":"1.6.1170.0","depots":[],"files":[],"manual":true}"#)] {
            let (_t, game, saves) = fixture();
            match marker {
                None => std::fs::remove_file(game.join(".aetherial-dawn/game.json")).unwrap(),
                Some(m) => std::fs::write(game.join(".aetherial-dawn/game.json"), m).unwrap(),
            }
            let acf = game.parent().unwrap().parent().unwrap().join("appmanifest_489830.acf");
            let before = read(&acf);
            let log = cleanup(Some(&game), Some(&saves));
            assert!(log[0].contains("didn't hold"), "{log:?}");
            assert_eq!(read(&acf), before);
            assert!(std::fs::metadata(&acf).unwrap().permissions().readonly());
            // The other steps still run.
            assert_eq!(read(&saves.join("plugins.txt")), "*PlayersOwn.esp");
        }
    }
}
