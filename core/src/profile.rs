//! The launcher as a profile: every file in the Skyrim folder that is not the
//! game, SKSE or SkyMP (core/src/modguard.rs) is moved into
//! `.aetherial-dawn/aside/<time>/` before the game starts and moved back when it
//! closes. A player keeps their mods; the game only ever sees the allowed
//! files. Moves stay on the same drive, so they are instant renames.
//!
//! The list of what was moved, including unresolved earlier sessions, is written to `.aetherial-dawn/aside/journal.json`
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

// Keep the original single-session format when possible. Multiple unresolved
// sessions need one durable list so resolving one cannot forget the others.
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum JournalFile {
    Single(Journal),
    Multiple { sessions: Vec<Journal> },
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
    /// Aside copies dropped because an identical file already sat at their place.
    pub duplicates: usize,
    /// Files left in the aside folder because something new sits at their place.
    pub kept: Vec<String>,
}

fn aside_dir(game_dir: &Path) -> PathBuf {
    game_dir.join(DIR)
}

fn journal_path(game_dir: &Path) -> PathBuf {
    aside_dir(game_dir).join(JOURNAL)
}

fn validate_path(value: &str, folder: bool) -> std::io::Result<()> {
    let invalid = || std::io::Error::new(std::io::ErrorKind::InvalidData, format!("Unsafe mod recovery path: {value}"));
    crate::manifest::safe_relative(value).map_err(|_| invalid())?;
    if value.split('/').any(|part| part.is_empty() || part == "." || part == ".." || part.ends_with(['.', ' ']))
        || (folder && value.contains('/'))
        || (!folder && value.split('/').next().is_some_and(|p| p.eq_ignore_ascii_case(".aetherial-dawn")))
    {
        return Err(invalid());
    }
    Ok(())
}

// Resolve existing ancestors before moving or pruning anything. A junction or
// symlink below the selected game folder must not redirect recovery elsewhere.
fn validate_local(game_dir: &Path, rel: &str) -> std::io::Result<()> {
    let root = std::fs::canonicalize(game_dir)?;
    let mut current = game_dir.to_path_buf();
    for part in Path::new(rel).components() {
        current.push(part);
        match std::fs::symlink_metadata(&current) {
            Ok(meta) => {
                if meta.file_type().is_symlink() || !std::fs::canonicalize(&current)?.starts_with(&root) {
                    return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "Mod recovery cannot follow a link outside its file list"));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

fn validate_sessions(game_dir: &Path, sessions: &[Journal]) -> std::io::Result<()> {
    validate_local(game_dir, &format!("{DIR}/{JOURNAL}"))?;
    for j in sessions {
        validate_path(&j.folder, true)?;
        validate_local(game_dir, &format!("{DIR}/{}", j.folder))?;
        for rel in &j.moved {
            validate_path(rel, false)?;
            validate_local(game_dir, rel)?;
            validate_local(game_dir, &format!("{DIR}/{}/{rel}", j.folder))?;
        }
    }
    Ok(())
}

fn read_journals(game_dir: &Path) -> Option<Vec<Journal>> {
    let bytes = std::fs::read(journal_path(game_dir)).ok()?;
    match serde_json::from_slice(&bytes).ok()? {
        JournalFile::Single(j) => Some(vec![j]),
        JournalFile::Multiple { sessions } => Some(sessions),
    }
}

/// Whether the journal file is there but can't be used (cut short or zeroed by
/// a power cut, edited by hand). The files it listed are still in the aside
/// folder, so `restore` rebuilds the list from what is there. Switch:
/// AD_ASIDE_REBUILD=0 gives the old behaviour (an unreadable journal means
/// nothing is waiting).
fn journal_unreadable(game_dir: &Path) -> bool {
    if std::env::var_os("AD_ASIDE_REBUILD").is_some_and(|v| v == "0") {
        return false;
    }
    journal_path(game_dir).is_file() && read_journals(game_dir).is_none()
}

/// Every file under each session folder of the aside folder, as (folder, paths
/// relative to the game folder).
fn rebuilt_journals(game_dir: &Path) -> Vec<Journal> {
    fn walk(dir: &Path, base: &Path, out: &mut Vec<String>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                walk(&p, base, out);
            } else if let Ok(r) = p.strip_prefix(base) {
                out.push(r.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(aside_dir(game_dir)) else { return out };
    let mut folders: Vec<String> = rd.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_dir())).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    folders.sort();
    for folder in folders {
        let base = aside_dir(game_dir).join(&folder);
        let mut moved = Vec::new();
        walk(&base, &base, &mut moved);
        moved.sort();
        if !moved.is_empty() {
            out.push(Journal { folder, moved });
        }
    }
    out
}

fn write_journals(game_dir: &Path, sessions: Vec<Journal>) -> std::io::Result<()> {
    let file = match sessions.as_slice() {
        [] => {
            return match std::fs::remove_file(journal_path(game_dir)) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                result => result,
            };
        }
        [j] => JournalFile::Single(j.clone()),
        _ => JournalFile::Multiple { sessions },
    };
    crate::atomicfile::write(&journal_path(game_dir), &serde_json::to_vec_pretty(&file).map_err(std::io::Error::other)?)
}

/// Whether a set-aside is waiting to be put back.
pub fn pending(game_dir: &Path) -> bool {
    if validate_local(game_dir, &format!("{DIR}/{JOURNAL}")).is_err() {
        return true;
    }
    if journal_unreadable(game_dir) {
        return !rebuilt_journals(game_dir).is_empty();
    }
    read_journals(game_dir).is_some_and(|sessions| validate_sessions(game_dir, &sessions).is_err() || sessions.iter().any(|j| !j.moved.is_empty()))
}

/// Moves the extras aside. The journal is saved first, so whatever happens
/// next, `restore` knows where everything went. A file that cannot be moved is
/// reported and left where it is (the caller then refuses to start the game).
pub fn set_aside(game_dir: &Path, extras: &[Extra], stamp: &str) -> std::io::Result<Moved> {
    // Validate the entire request before even restoring an earlier session.
    validate_path(stamp, true)?;
    for e in extras {
        validate_path(&e.rel, false)?;
    }
    validate_sessions(game_dir, &[Journal { folder: stamp.to_string(), moved: extras.iter().map(|e| e.rel.clone()).collect() }])?;
    // Restore what we can; retain earlier conflicts alongside the new session.
    restore(game_dir)?;
    if extras.is_empty() {
        return Ok(Moved::default());
    }
    let folder = stamp.to_string();
    let dest = aside_dir(game_dir).join(&folder);
    std::fs::create_dir_all(aside_dir(game_dir))?;
    // A repeated timestamp must never overwrite an earlier session's copies.
    std::fs::create_dir(&dest)?;
    let mut sessions = read_journals(game_dir).unwrap_or_default();
    let all: Vec<String> = extras.iter().map(|e| e.rel.clone()).collect();
    sessions.push(Journal { folder: folder.clone(), moved: all });
    write_journals(game_dir, sessions.clone())?;
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
    sessions.pop();
    if !kept_moved.is_empty() {
        sessions.push(Journal { folder, moved: kept_moved });
    } else {
        let _ = std::fs::remove_dir(&dest);
    }
    write_journals(game_dir, sessions)?;
    Ok(done)
}

/// Same size and same bytes.
fn same_file(a: &Path, b: &Path) -> bool {
    use std::io::Read;
    let (Ok(ma), Ok(mb)) = (std::fs::metadata(a), std::fs::metadata(b)) else { return false };
    if ma.len() != mb.len() {
        return false;
    }
    let (Ok(mut fa), Ok(mut fb)) = (std::fs::File::open(a), std::fs::File::open(b)) else { return false };
    let (mut x, mut y) = (vec![0u8; 1 << 16], vec![0u8; 1 << 16]);
    loop {
        let (Ok(n), Ok(m)) = (fa.read(&mut x), fb.read(&mut y)) else { return false };
        if n != m || x[..n] != y[..m] {
            return false;
        }
        if n == 0 {
            return true;
        }
    }
}

/// Puts back everything the journal lists. A file whose place has something
/// new in it stays in the aside folder (and is reported); nothing is ever
/// overwritten.
pub fn restore(game_dir: &Path) -> std::io::Result<Restored> {
    validate_local(game_dir, &format!("{DIR}/{JOURNAL}"))?;
    let sessions = if journal_unreadable(game_dir) {
        let found = rebuilt_journals(game_dir);
        eprintln!("profile: journal.json is unreadable; putting back the {} session folder(s) found in the aside folder", found.len());
        found
    } else {
        let Some(sessions) = read_journals(game_dir) else { return Ok(Restored::default()) };
        sessions
    };
    // One bad entry must not allow earlier valid entries to move first.
    validate_sessions(game_dir, &sessions)?;
    let mut out = Restored::default();
    let mut keep = Vec::new();
    for j in sessions {
        let (r, left) = restore_folder(game_dir, &j);
        out.files += r.files;
        out.duplicates += r.duplicates;
        out.kept.extend(r.kept);
        if !left.is_empty() {
            keep.push(Journal { folder: j.folder, moved: left });
        }
    }
    write_journals(game_dir, keep)?;
    Ok(out)
}

/// Puts back one session folder's files, never overwriting. Returns the tally
/// and the paths still left in the folder; an emptied folder is removed.
fn restore_folder(game_dir: &Path, j: &Journal) -> (Restored, Vec<String>) {
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
            // A mod manager put the same file back while the game ran: the aside copy
            // is a duplicate, so drop it instead of leaving it behind for ever.
            if same_file(&from, &to) && std::fs::remove_file(&from).is_ok() {
                out.duplicates += 1;
                continue;
            }
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
        prune_empty_dirs(&src);
    }
    (out, left)
}

// Never recursively delete a session's contents: a journal can be incomplete,
// and unrelated files may have been placed there. Do not follow directory links.
fn prune_empty_dirs(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        if entry.file_type().is_ok_and(|kind| kind.is_dir() && !kind.is_symlink()) {
            prune_empty_dirs(&entry.path());
        }
    }
    let _ = std::fs::remove_dir(dir);
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
    fn an_identical_file_already_at_the_place_drops_the_aside_copy_a_different_one_stays() {
        let d = tempfile::tempdir().unwrap();
        let g = d.path();
        put(g, "Data/a.esp", "same");
        put(g, "Data/b.esp", "old");
        set_aside(g, &[extra("Data/a.esp"), extra("Data/b.esp")], "1").unwrap();
        put(g, "Data/a.esp", "same");
        put(g, "Data/b.esp", "new");
        let r = restore(g).unwrap();
        assert_eq!((r.files, r.duplicates, r.kept.len()), (0, 1, 1));
        assert!(!g.join(".aetherial-dawn/aside/1/Data/a.esp").exists());
        assert!(g.join(".aetherial-dawn/aside/1/Data/b.esp").exists());
    }

    #[test]
    fn an_unreadable_journal_does_not_strand_the_mods() {
        let d = tempfile::tempdir().unwrap();
        let g = d.path();
        put(g, "Data/Mod.esp", "plugin");
        put(g, "Data/meshes/a/b.nif", "mesh");
        set_aside(g, &[extra("Data/Mod.esp"), extra("Data/meshes/a/b.nif")], "t1").unwrap();
        // A power cut after the rename but before the data reached the disk leaves a file of zero bytes.
        std::fs::write(journal_path(g), [0u8; 16]).unwrap();
        assert!(pending(g), "the aside folder still holds the player's files");
        let back = restore(g).unwrap();
        assert_eq!((back.files, back.kept.len()), (2, 0));
        assert_eq!(std::fs::read_to_string(g.join("Data/Mod.esp")).unwrap(), "plugin");
        assert_eq!(std::fs::read_to_string(g.join("Data/meshes/a/b.nif")).unwrap(), "mesh");
        assert!(!pending(g) && !journal_path(g).exists() && !aside_dir(g).join("t1").exists());
    }

    #[test]
    fn the_next_set_aside_puts_back_what_an_unreadable_journal_listed() {
        let d = tempfile::tempdir().unwrap();
        let g = d.path();
        put(g, "Data/A.esp", "a");
        set_aside(g, &[extra("Data/A.esp")], "t1").unwrap();
        std::fs::write(journal_path(g), b"{\"folder\": \"t1\", \"mo").unwrap();
        put(g, "Data/B.esp", "b");
        set_aside(g, &[extra("Data/B.esp")], "t2").unwrap();
        assert!(g.join("Data/A.esp").exists(), "the earlier session's file is back");
        assert!(!aside_dir(g).join("t1").exists());
    }

    #[test]
    fn a_rebuilt_restore_never_overwrites() {
        let d = tempfile::tempdir().unwrap();
        let g = d.path();
        put(g, "Data/Mod.esp", "old");
        set_aside(g, &[extra("Data/Mod.esp")], "t1").unwrap();
        put(g, "Data/Mod.esp", "reinstalled");
        std::fs::write(journal_path(g), b"garbage").unwrap();
        let back = restore(g).unwrap();
        assert_eq!((back.files, back.kept.clone()), (0, vec!["Data/Mod.esp".to_string()]));
        assert_eq!(std::fs::read_to_string(g.join("Data/Mod.esp")).unwrap(), "reinstalled");
        assert_eq!(std::fs::read_to_string(aside_dir(g).join("t1/Data/Mod.esp")).unwrap(), "old");
        assert!(pending(g), "it stays listed until the player sorts it out");
        assert_eq!(read_journals(g).unwrap()[0].moved, vec!["Data/Mod.esp".to_string()]);
    }

    #[test]
    fn a_new_session_keeps_earlier_conflicts_recoverable() {
        let d = tempfile::tempdir().unwrap();
        let g = d.path();
        put(g, "Data/A.esp", "original a");
        set_aside(g, &[extra("Data/A.esp")], "t1").unwrap();
        put(g, "Data/A.esp", "replacement a");
        put(g, "Data/B.esp", "original b");
        set_aside(g, &[extra("Data/B.esp")], "t2").unwrap();
        restore(g).unwrap();
        assert!(pending(g), "the earlier conflict must remain tracked after the new session ends");
        assert_eq!(std::fs::read_to_string(g.join("Data/A.esp")).unwrap(), "replacement a");
        std::fs::remove_file(g.join("Data/A.esp")).unwrap();
        assert_eq!(restore(g).unwrap().files, 1);
        assert_eq!(std::fs::read_to_string(g.join("Data/A.esp")).unwrap(), "original a");
        assert_eq!(std::fs::read_to_string(g.join("Data/B.esp")).unwrap(), "original b");
        assert!(!pending(g));
    }

    #[test]
    fn rebuilt_conflicts_from_multiple_sessions_all_remain_recoverable() {
        let d = tempfile::tempdir().unwrap();
        let g = d.path();
        put(g, ".aetherial-dawn/aside/t1/Data/A.esp", "original a");
        put(g, ".aetherial-dawn/aside/t2/Data/B.esp", "original b");
        put(g, "Data/A.esp", "replacement a");
        put(g, "Data/B.esp", "replacement b");
        put(g, ".aetherial-dawn/aside/journal.json", "broken");
        assert_eq!(restore(g).unwrap().kept.len(), 2);
        std::fs::remove_file(g.join("Data/A.esp")).unwrap();
        assert_eq!(restore(g).unwrap().files, 1);
        assert!(pending(g), "the second session must stay tracked after the first is restored");
        std::fs::remove_file(g.join("Data/B.esp")).unwrap();
        assert_eq!(restore(g).unwrap().files, 1);
        assert_eq!(std::fs::read_to_string(g.join("Data/B.esp")).unwrap(), "original b");
        assert!(!pending(g));
    }

    #[test]
    fn a_reused_session_folder_cannot_replace_an_unrestored_copy() {
        let d = tempfile::tempdir().unwrap();
        let g = d.path();
        put(g, "Data/A.esp", "original a");
        set_aside(g, &[extra("Data/A.esp")], "t1").unwrap();
        put(g, "Data/A.esp", "replacement a");
        let err = set_aside(g, &[extra("Data/A.esp")], "t1").unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read_to_string(aside_dir(g).join("t1/Data/A.esp")).unwrap(), "original a");
        assert_eq!(std::fs::read_to_string(g.join("Data/A.esp")).unwrap(), "replacement a");
        assert!(pending(g));
    }

    #[test]
    fn an_escaping_journal_is_rejected_before_any_restore_or_cleanup() {
        let d = tempfile::tempdir().unwrap();
        let g = d.path();
        // Every target, including the deliberately escaping path, is inside this test's temp root.
        put(g, "victim/keep.txt", "keep");
        put(g, ".aetherial-dawn/aside/journal.json", r#"{"folder":"../../victim","moved":[]}"#);
        assert!(restore(g).is_err());
        assert_eq!(std::fs::read_to_string(g.join("victim/keep.txt")).unwrap(), "keep");
        assert!(pending(g), "an unsafe journal must not be treated as no pending recovery");
    }

    #[test]
    fn invalid_set_aside_paths_are_rejected_before_any_files_move() {
        let d = tempfile::tempdir().unwrap();
        let g = d.path().join("game");
        put(&g, "Data/A.esp", "a");
        put(d.path(), "outside.esp", "outside");
        assert!(set_aside(&g, &[extra("Data/A.esp"), extra("../outside.esp")], "t1").is_err());
        assert_eq!(std::fs::read_to_string(g.join("Data/A.esp")).unwrap(), "a");
        assert_eq!(std::fs::read_to_string(d.path().join("outside.esp")).unwrap(), "outside");
        assert!(set_aside(&g, &[extra("Data/A.esp")], "../escape").is_err());
    }

    #[test]
    fn restoring_a_session_does_not_delete_unlisted_files_in_its_folder() {
        let d = tempfile::tempdir().unwrap();
        let g = d.path();
        put(g, "Data/A.esp", "a");
        set_aside(g, &[extra("Data/A.esp")], "t1").unwrap();
        put(g, ".aetherial-dawn/aside/t1/Data/unlisted.esp", "keep");
        restore(g).unwrap();
        assert_eq!(std::fs::read_to_string(aside_dir(g).join("t1/Data/unlisted.esp")).unwrap(), "keep");
    }

    #[test]
    fn every_journal_entry_is_validated_before_a_valid_entry_moves() {
        let d = tempfile::tempdir().unwrap();
        let g = d.path();
        put(g, ".aetherial-dawn/aside/t1/Data/A.esp", "a");
        let sessions = vec![Journal { folder: "t1".into(), moved: vec!["Data/A.esp".into()] },
            Journal { folder: "t2".into(), moved: vec!["../outside.esp".into()] }];
        write_journals(g, sessions).unwrap();
        assert!(restore(g).is_err());
        assert!(!g.join("Data/A.esp").exists());
        assert_eq!(std::fs::read_to_string(aside_dir(g).join("t1/Data/A.esp")).unwrap(), "a");
    }

    #[test]
    fn recovery_paths_reject_windows_aliases_and_metadata() {
        for rel in ["", ".", "..", "Data/../a", "Data/./a", "Data/a.", "Data/a ", "Data//a", "C:/a", "Data\\a", ".aetherial-dawn/aside/journal.json"] {
            assert!(validate_path(rel, false).is_err(), "{rel}");
        }
        assert!(validate_path("Data/Jörgen.esp", false).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn a_directory_link_cannot_redirect_restoration_or_set_aside() {
        let d = tempfile::tempdir().unwrap();
        let g = d.path().join("game");
        put(&g, ".aetherial-dawn/aside/t1/Data/A.esp", "original");
        put(d.path(), "outside/keep.esp", "keep");
        std::os::unix::fs::symlink(d.path().join("outside"), g.join("Data")).unwrap();
        write_journals(&g, vec![Journal { folder: "t1".into(), moved: vec!["Data/A.esp".into()] }]).unwrap();
        assert!(restore(&g).is_err());
        assert!(!d.path().join("outside/A.esp").exists());
        assert!(set_aside(&g, &[extra("Data/keep.esp")], "t2").is_err());
        assert_eq!(std::fs::read_to_string(d.path().join("outside/keep.esp")).unwrap(), "keep");
    }

    #[test]
    fn nothing_to_do_without_extras_or_a_journal() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(set_aside(d.path(), &[], "t1").unwrap(), Moved::default());
        assert_eq!(restore(d.path()).unwrap(), Restored::default());
        assert!(!pending(d.path()));
    }
}
