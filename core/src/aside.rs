//! Files the launcher moves out of the game's way without deleting them
//! (plugins made for a newer Skyrim, broken stubs), and putting them back.

use std::path::{Path, PathBuf};

use crate::Result;

/// Where moved files go, inside the game folder, keeping their paths.
pub const DISABLED_DIR: &str = ".aetherial-dawn/disabled";

/// Moves the given files into `.aetherial-dawn/disabled/<stamp>/`, keeping
/// their paths, and returns that folder.
pub fn move_aside(game_dir: &Path, files: &[String], stamp: &str) -> Result<PathBuf> {
    let dest = game_dir.join(DISABLED_DIR).join(stamp);
    for rel in files {
        let from = game_dir.join(rel);
        if !from.is_file() {
            continue;
        }
        let to = dest.join(rel);
        if let Some(p) = to.parent() {
            std::fs::create_dir_all(p)?;
        }
        std::fs::rename(&from, &to)?;
    }
    Ok(dest)
}

/// Puts everything the launcher set aside back where it was, unless
/// something else is there now. Returns how many files went back.
pub fn restore_all(game_dir: &Path) -> std::io::Result<usize> {
    let root = game_dir.join(DISABLED_DIR);
    let Ok(rd) = std::fs::read_dir(&root) else { return Ok(0) };
    let mut n = 0;
    let mut stamps: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    // Newest first, so the latest copy of a file wins.
    stamps.sort();
    stamps.reverse();
    for stamp in stamps {
        let mut stack = vec![stamp.clone()];
        while let Some(d) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&d) else { continue };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                    continue;
                }
                let Ok(rel) = p.strip_prefix(&stamp) else { continue };
                let to = game_dir.join(rel);
                if to.exists() {
                    continue;
                }
                if let Some(parent) = to.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::rename(&p, &to)?;
                n += 1;
            }
        }
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moves_aside_and_restores() {
        let tmp = tempfile::tempdir().unwrap();
        let g = tmp.path();
        let p = g.join("Data/x.esp");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, b"x").unwrap();
        let dest = move_aside(g, &["Data/x.esp".into(), "Data/none.esp".into()], "t1").unwrap();
        assert!(dest.join("Data/x.esp").is_file() && !p.exists());
        assert_eq!(restore_all(g).unwrap(), 1);
        assert!(p.is_file());
    }
}
