//! The launcher as a profile: every file in the Skyrim folder that is not the
//! game, SKSE or SkyMP (core/src/modguard.rs) is moved into
//! `.aetherial-dawn/aside/<time>/` before the game starts and moved back when it
//! closes. A player keeps their mods; the game only ever sees the allowed
//! files. Moves stay on the same drive, so they are instant renames.
//!
//! The list of what was moved is written to `.aetherial-dawn/aside/journal.json`
//! before the first move, so a crash, a power cut or a launcher restart can
//! still put everything back (`restore`), which the launcher also runs when it
//! starts and no game is running.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::modguard::Extra;

const DIR: &str = ".aetherial-dawn/aside";
const JOURNAL: &str = "journal.json";

#[derive(Serialize, Deserialize, Debug, Default, Clone, PartialEq)]
struct Journal {
    /// Folder (under DIR) the files were moved into.
    folder: String,
    /// Paths relative to the game folder, `/` separators.
    moved: Vec<String>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Moved {
    pub files: usize,
    /// Files that could not be moved (name and why); the rest were.
    pub failed: Vec<String>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Restored {
    pub files: usize,
    /// Files left in the aside folder because something new sits at their place.
    pub kept: Vec<String>,
}

fn aside_dir(game_dir: &Path) -> PathBuf {
    game_dir.join(DIR)
}

fn journal_path(game_dir: &Path) -> PathBuf {
    aside_dir(game_dir).join(JOURNAL)
}

fn read_journal(game_dir: &Path) -> Option<Journal> {
    std::fs::read(journal_path(game_dir)).ok().and_then(|b| serde_json::from_slice(&b).ok())
}

fn write_journal(game_dir: &Path, j: &Journal) -> std::io::Result<()> {
    crate::atomicfile::write(&journal_path(game_dir), &serde_json::to_vec_pretty(j).map_err(std::io::Error::other)?)
}

/// Whether a set-aside is waiting to be put back.
pub fn pending(game_dir: &Path) -> bool {
    read_journal(game_dir).is_some_and(|j| !j.moved.is_empty())
}

/// Moves the extras aside. The journal is saved first, so whatever happens
/// next, `restore` knows where everything went. A file that cannot be moved is
/// reported and left where it is (the caller then refuses to start the game).
pub fn set_aside(game_dir: &Path, extras: &[Extra], stamp: &str) -> std::io::Result<Moved> {
    // Anything still waiting from an earlier session goes back first, so a
    // journal never holds two sessions.
    restore(game_dir)?;
    if extras.is_empty() {
        return Ok(Moved::default());
    }
    let folder = stamp.to_string();
    let dest = aside_dir(game_dir).join(&folder);
    std::fs::create_dir_all(&dest)?;
    let all: Vec<String> = extras.iter().map(|e| e.rel.clone()).collect();
    write_journal(game_dir, &Journal { folder: folder.clone(), moved: all })?;
    let mut done = Moved::default();
    let mut kept_moved = Vec::new();
    for e in extras {
        let from = game_dir.join(&e.rel);
        let to = dest.join(&e.rel);
        let moved = to.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| std::fs::rename(&from, &to));
        match moved {
            Ok(()) => {
                done.files += 1;
                kept_moved.push(e.rel.clone());
            }
            Err(err) => done.failed.push(format!("{} ({err})", e.rel)),
        }
    }
    // The journal lists only what really moved.
    write_journal(game_dir, &Journal { folder, moved: kept_moved })?;
    Ok(done)
}

/// Puts back everything the journal lists. A file whose place has something
/// new in it stays in the aside folder (and is reported); nothing is ever
/// overwritten.
pub fn restore(game_dir: &Path) -> std::io::Result<Restored> {
    let Some(j) = read_journal(game_dir) else { return Ok(Restored::default()) };
    let src = aside_dir(game_dir).join(&j.folder);
    let mut out = Restored::default();
    let mut left = Vec::new();
    for rel in &j.moved {
        let from = src.join(rel);
        let to = game_dir.join(rel);
        if !from.exists() {
            continue;
        }
        if to.exists() {
            out.kept.push(rel.clone());
            left.push(rel.clone());
            continue;
        }
        let back = to.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| std::fs::rename(&from, &to));
        match back {
            Ok(()) => out.files += 1,
            Err(_) => {
                out.kept.push(rel.clone());
                left.push(rel.clone());
            }
        }
    }
    if left.is_empty() {
        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_file(journal_path(game_dir));
    } else {
        write_journal(game_dir, &Journal { folder: j.folder, moved: left })?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(root: &Path, rel: &str, body: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    fn extra(rel: &str) -> Extra {
        Extra { kind: "files", rel: rel.into() }
    }

    #[test]
    fn extras_leave_for_the_session_and_come_back_exactly() {
        let d = tempfile::tempdir().unwrap();
        put(d.path(), "Data/Mod.esp", "plugin");
        put(d.path(), "Data/meshes/a/b.nif", "mesh");
        put(d.path(), "Data/Skyrim.esm", "base");
        let ex = [extra("Data/Mod.esp"), extra("Data/meshes/a/b.nif")];
        let moved = set_aside(d.path(), &ex, "t1").unwrap();
        assert_eq!((moved.files, moved.failed.len()), (2, 0));
        assert!(!d.path().join("Data/Mod.esp").exists() && !d.path().join("Data/meshes/a/b.nif").exists());
        assert!(d.path().join("Data/Skyrim.esm").exists(), "the game's own files stay");
        assert!(pending(d.path()));
        let back = restore(d.path()).unwrap();
        assert_eq!((back.files, back.kept.len()), (2, 0));
        assert_eq!(std::fs::read_to_string(d.path().join("Data/Mod.esp")).unwrap(), "plugin");
        assert_eq!(std::fs::read_to_string(d.path().join("Data/meshes/a/b.nif")).unwrap(), "mesh");
        assert!(!pending(d.path()));
        assert!(!aside_dir(d.path()).join("t1").exists());
    }

    #[test]
    fn a_new_file_at_the_old_place_is_never_overwritten() {
        let d = tempfile::tempdir().unwrap();
        put(d.path(), "Data/Mod.esp", "old");
        set_aside(d.path(), &[extra("Data/Mod.esp")], "t1").unwrap();
        put(d.path(), "Data/Mod.esp", "reinstalled");
        let back = restore(d.path()).unwrap();
        assert_eq!((back.files, back.kept.clone()), (0, vec!["Data/Mod.esp".to_string()]));
        assert_eq!(std::fs::read_to_string(d.path().join("Data/Mod.esp")).unwrap(), "reinstalled");
        assert_eq!(std::fs::read_to_string(aside_dir(d.path()).join("t1/Data/Mod.esp")).unwrap(), "old");
        assert!(pending(d.path()), "it stays on the list until the player sorts it out");
    }

    #[test]
    fn a_session_that_never_got_restored_is_put_back_before_the_next_one() {
        let d = tempfile::tempdir().unwrap();
        put(d.path(), "Data/A.esp", "a");
        set_aside(d.path(), &[extra("Data/A.esp")], "t1").unwrap();
        // The launcher died; the next Play starts a new set-aside.
        put(d.path(), "Data/B.esp", "b");
        let moved = set_aside(d.path(), &[extra("Data/B.esp")], "t2").unwrap();
        assert_eq!(moved.files, 1);
        assert!(d.path().join("Data/A.esp").exists(), "the earlier session's file is back");
        restore(d.path()).unwrap();
        assert!(d.path().join("Data/A.esp").exists() && d.path().join("Data/B.esp").exists());
    }

    #[test]
    fn a_file_that_cannot_move_is_reported_and_the_rest_still_go() {
        let d = tempfile::tempdir().unwrap();
        put(d.path(), "Data/A.esp", "a");
        let ex = [extra("Data/Missing.esp"), extra("Data/A.esp")];
        let moved = set_aside(d.path(), &ex, "t1").unwrap();
        assert_eq!(moved.files, 1);
        assert_eq!(moved.failed.len(), 1);
        assert!(moved.failed[0].starts_with("Data/Missing.esp"));
        restore(d.path()).unwrap();
        assert!(d.path().join("Data/A.esp").exists());
    }

    #[test]
    fn nothing_to_do_without_extras_or_a_journal() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(set_aside(d.path(), &[], "t1").unwrap(), Moved::default());
        assert_eq!(restore(d.path()).unwrap(), Restored::default());
        assert!(!pending(d.path()));
    }
}
