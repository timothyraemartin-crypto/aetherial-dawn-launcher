//! A temp folder that belongs to our game and nothing else. Skyrim Platform's
//! built-in browser keeps its profile in `<TEMP>\Skyrim Platform`, and every
//! SkyMP client on the PC (another server's launcher, a dev build, a client
//! still running or crashed) uses the same `%TEMP%`. Leftovers there, or
//! files another client holds open, then reach our game. Play starts the game
//! with TEMP and TMP pointed at a fresh folder under `.aetherial-dawn/game-temp`
//! (a new `run-<stamp>` per Play; older ones are deleted, and one that cannot
//! be deleted is simply not reused). The player's own %TEMP% is not touched.
//! Switch: status.json `launcher.isolateTemp`: false, or rules `checks.isolateTemp`: false.

use std::path::{Path, PathBuf};

const ROOT: &str = ".aetherial-dawn/game-temp";

pub fn root(game_dir: &Path) -> PathBuf {
    game_dir.join(ROOT)
}

/// Deletes the earlier run folders (best effort, a locked one stays) and
/// makes a fresh `run-<stamp>` that is checked writable. Returns it, or why not.
pub fn prepare(game_dir: &Path, stamp: &str) -> Result<PathBuf, String> {
    let root = root(game_dir);
    if let Ok(rd) = std::fs::read_dir(&root) {
        for e in rd.flatten() {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
    let run = root.join(format!("run-{stamp}"));
    // Skyrim Platform creates this itself; making it here lets the probe and the "was it used" check agree.
    crate::browsercheck::temp_probe(&run.join("Skyrim Platform"))?;
    Ok(run)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_play_gets_a_fresh_folder_and_old_ones_go() {
        let g = tempfile::tempdir().unwrap();
        let a = prepare(g.path(), "1").unwrap();
        std::fs::write(a.join("Skyrim Platform/stale-profile"), b"x").unwrap();
        let b = prepare(g.path(), "2").unwrap();
        assert!(b.join("Skyrim Platform").is_dir());
        assert!(!a.exists(), "the previous run's folder is deleted");
        assert!(!b.join("Skyrim Platform/stale-profile").exists());
    }

    #[test]
    fn a_folder_that_cannot_be_made_is_an_error_not_a_panic() {
        let g = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(g.path().join(".aetherial-dawn")).unwrap();
        std::fs::write(g.path().join(ROOT), b"a file where the folder should be").unwrap();
        assert!(prepare(g.path(), "1").is_err());
    }
}
