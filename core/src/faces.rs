//! Face sharing (Sync's contract, desync/face-files-launcher-contract.md,
//! 2026-09-27; Timothy's OK 19:09): while the game runs, the launcher sends
//! the player's own RaceMenu preset (ad/self.jslot, written by the game) to
//! the server and saves other online characters' presets next to it, so each
//! game can load the faces of the people around it. Only these files, only
//! in Data/SKSE/Plugins/CharGen/Presets/ad/, and only after checking them.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::{Error, Result};

pub const FOLDER: &str = "Data/SKSE/Plugins/CharGen/Presets/ad";
pub const SELF_FILE: &str = "self.jslot";
/// Largest own preset the launcher sends.
pub const MAX_UPLOAD: u64 = 1_500_000;
/// Largest face file it saves (the server's cleaned limit).
pub const MAX_FACE: usize = 400 * 1024;
/// Largest list answer it reads (300 names).
pub const MAX_LIST: usize = 256 * 1024;
const LIST_RECORD: &str = ".aetherial-dawn/faces-list.json";

/// The ad/ folder, created when missing. The ad folder itself being a link
/// (or a junction) is refused; the folders above it are the game's own and
/// aren't checked.
pub fn folder(game_dir: &Path) -> Result<PathBuf> {
    let dir = game_dir.join(FOLDER);
    std::fs::create_dir_all(&dir)?;
    if std::fs::symlink_metadata(&dir)?.file_type().is_symlink() {
        return Err(Error::Game(format!("{FOLDER} is a link; faces aren't saved there")));
    }
    Ok(dir)
}

/// The ad/ folder when it's already there as a real folder (not a link),
/// for tidying; never created.
pub fn existing_folder(game_dir: &Path) -> Option<PathBuf> {
    let dir = game_dir.join(FOLDER);
    std::fs::symlink_metadata(&dir).ok().filter(|m| m.is_dir()).map(|_| dir)
}

/// A face name as the server gives it: "a<1-8 hex>-<16 hex>".
pub fn valid_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix('a') else { return false };
    let Some((id, hash)) = rest.split_once('-') else { return false };
    let hex = |s: &str| s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    (1..=8).contains(&id.len()) && hex(id) && hash.len() == 16 && hex(hash)
}

/// Checks a downloaded face before it's saved: size, JSON, and that its
/// SHA-256 starts with the name's hash part.
pub fn check_face(name: &str, body: &[u8]) -> Result<()> {
    if !valid_name(name) {
        return Err(Error::Game(format!("{name:?} isn't a face name")));
    }
    if body.len() > MAX_FACE {
        return Err(Error::Game(format!("{name} is {} bytes, over the limit", body.len())));
    }
    serde_json::from_slice::<serde_json::Value>(body).map_err(|_| Error::Game(format!("{name} isn't a preset file")))?;
    let hash = hex::encode(Sha256::digest(body));
    if !name.ends_with(&hash[..16]) {
        return Err(Error::Game(format!("{name} doesn't match its contents")));
    }
    Ok(())
}

/// Saves a checked face as ad/<name>.jslot (through a .tmp file).
pub fn save(dir: &Path, name: &str, body: &[u8]) -> Result<()> {
    check_face(name, body)?;
    let tmp = dir.join(format!("{name}.jslot.tmp"));
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, dir.join(format!("{name}.jslot")))?;
    Ok(())
}

/// Faces already saved, by name.
pub fn saved(dir: &Path) -> BTreeSet<String> {
    std::fs::read_dir(dir)
        .map(|rd| rd.flatten().filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_suffix(".jslot")).filter(|n| valid_name(n)).map(str::to_string)).collect())
        .unwrap_or_default()
}

/// Deletes the saved faces not in `keep` (and leftover .tmp files).
/// self.jslot and anything not named like a face stay.
pub fn tidy(dir: &Path, keep: &BTreeSet<String>) -> usize {
    let mut n = 0;
    let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
    for e in rd.flatten() {
        let Some(file) = e.file_name().to_str().map(str::to_string) else { continue };
        let stale = match (file.strip_suffix(".jslot"), file.strip_suffix(".jslot.tmp")) {
            (Some(name), _) => valid_name(name) && !keep.contains(name),
            (_, Some(name)) => valid_name(name),
            _ => false,
        };
        if stale && e.file_type().map(|t| t.is_file()).unwrap_or(false) && std::fs::remove_file(e.path()).is_ok() {
            n += 1;
        }
    }
    n
}

/// How often the list is asked for when the server doesn't say: safe with
/// faces 1.1.6, which refuses a second list within 2 s.
pub const LIST_EVERY: std::time::Duration = std::time::Duration::from_secs(5);
/// The fastest and slowest a server may ask for.
pub const LIST_EVERY_MIN: std::time::Duration = std::time::Duration::from_secs(1);
pub const LIST_EVERY_MAX: std::time::Duration = std::time::Duration::from_secs(60);

/// The list answer: the names (invalid ones dropped) and how soon to ask
/// again, from `"listEvery"` in seconds when the server sends it (kept
/// between 1 and 60 s), otherwise `LIST_EVERY`.
#[derive(Debug, PartialEq)]
pub struct FaceList {
    pub names: Vec<String>,
    pub every: std::time::Duration,
}

pub fn list_answer(body: &[u8]) -> Result<FaceList> {
    #[derive(serde::Deserialize)]
    struct Face {
        name: String,
    }
    #[derive(serde::Deserialize)]
    struct List {
        faces: Vec<Face>,
        #[serde(default, rename = "listEvery")]
        list_every: Option<serde_json::Value>,
    }
    if body.len() > MAX_LIST {
        return Err(Error::Game("the face list is too long".into()));
    }
    let l: List = serde_json::from_slice(body)?;
    let every = l
        .list_every
        .and_then(|v| v.as_f64())
        .filter(|s| s.is_finite() && *s > 0.0)
        .map(|s| std::time::Duration::from_secs_f64(s.min(3600.0)).clamp(LIST_EVERY_MIN, LIST_EVERY_MAX))
        .unwrap_or(LIST_EVERY);
    Ok(FaceList { names: l.faces.into_iter().map(|f| f.name).filter(|n| valid_name(n)).take(300).collect(), every })
}

/// The list answer's names (invalid ones dropped).
pub fn list_names(body: &[u8]) -> Result<Vec<String>> {
    list_answer(body).map(|l| l.names)
}

/// Remembers the latest list, for tidying at the next launcher start.
pub fn remember(game_dir: &Path, names: &BTreeSet<String>) {
    let p = game_dir.join(LIST_RECORD);
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(p, serde_json::to_vec(names).unwrap_or_default());
}

/// The list remembered by the last session (empty when none).
pub fn remembered(game_dir: &Path) -> BTreeSet<String> {
    std::fs::read(game_dir.join(LIST_RECORD)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(body: &[u8]) -> String {
        format!("aff000003-{}", &hex::encode(Sha256::digest(body))[..16])
    }

    #[test]
    fn names_follow_the_servers_shape() {
        assert!(valid_name("aff000003-0123456789abcdef"));
        assert!(valid_name("a1-0123456789abcdef"));
        for bad in ["self", "a-0123456789abcdef", "a123456789-0123456789abcdef", "aff-0123456789ABCDEF", "aff-0123", "bff-0123456789abcdef", "aff-0123456789abcdef/..", "../aff-0123456789abcdef"] {
            assert!(!valid_name(bad), "{bad}");
        }
    }

    #[test]
    fn a_face_is_saved_only_when_it_checks_out() {
        let t = tempfile::tempdir().unwrap();
        let dir = folder(t.path()).unwrap();
        let body = br#"{"Headparts":[1,2]}"#;
        let name = named(body);
        save(&dir, &name, body).unwrap();
        assert_eq!(std::fs::read(dir.join(format!("{name}.jslot"))).unwrap(), body);
        assert!(saved(&dir).contains(&name));
        // Wrong hash, not JSON, too big, bad name: not saved.
        assert!(save(&dir, "aff000003-0000000000000000", body).is_err());
        let not_json = b"not json";
        assert!(save(&dir, &named(not_json), not_json).is_err());
        let big = format!("{{\"x\":\"{}\"}}", "a".repeat(MAX_FACE));
        assert!(save(&dir, &named(big.as_bytes()), big.as_bytes()).is_err());
        assert!(save(&dir, "self", body).is_err());
        assert_eq!(saved(&dir).len(), 1);
    }

    #[test]
    fn tidy_keeps_the_list_and_the_players_own_face() {
        let t = tempfile::tempdir().unwrap();
        let dir = folder(t.path()).unwrap();
        for f in ["self.jslot", "aff000003-0123456789abcdef.jslot", "aff000004-0123456789abcdef.jslot", "aff000005-0123456789abcdef.jslot.tmp", "notes.txt", "a-bad.jslot"] {
            std::fs::write(dir.join(f), "{}").unwrap();
        }
        let keep: BTreeSet<String> = ["aff000003-0123456789abcdef".to_string()].into();
        assert_eq!(tidy(&dir, &keep), 2);
        let mut left: Vec<String> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left, vec!["a-bad.jslot", "aff000003-0123456789abcdef.jslot", "notes.txt", "self.jslot"]);
        remember(t.path(), &keep);
        assert_eq!(remembered(t.path()), keep);
    }

    #[test]
    fn reads_the_list_answer() {
        let body = br#"{"v":1,"folder":"ad","faces":[{"name":"aff000003-0123456789abcdef","bytes":12},{"name":"../x","bytes":1}]}"#;
        assert_eq!(list_names(body).unwrap(), vec!["aff000003-0123456789abcdef"]);
        assert!(list_names(b"nope").is_err());
    }

    #[test]
    fn the_list_interval_comes_from_the_server_with_a_safe_fallback() {
        use std::time::Duration;
        let with = |v: &str| list_answer(format!(r#"{{"faces":[],"listEvery":{v}}}"#).as_bytes()).unwrap().every;
        // faces 1.1.6 says nothing: 5 s, clear of its 2 s gap.
        assert_eq!(list_answer(br#"{"faces":[]}"#).unwrap().every, Duration::from_secs(5));
        assert_eq!(with("1"), Duration::from_secs(1));
        assert_eq!(with("2.5"), Duration::from_millis(2500));
        // Kept between 1 and 60 s; nonsense falls back.
        assert_eq!(with("0.2"), Duration::from_secs(1));
        assert_eq!(with("9999"), Duration::from_secs(60));
        assert_eq!(with("0"), Duration::from_secs(5));
        assert_eq!(with("-3"), Duration::from_secs(5));
        assert_eq!(with("\"soon\""), Duration::from_secs(5));
        assert_eq!(with("null"), Duration::from_secs(5));
    }

    #[cfg(unix)]
    #[test]
    fn a_linked_folder_is_refused() {
        let t = tempfile::tempdir().unwrap();
        let elsewhere = t.path().join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::create_dir_all(t.path().join("Data/SKSE/Plugins/CharGen/Presets")).unwrap();
        std::os::unix::fs::symlink(&elsewhere, t.path().join(FOLDER)).unwrap();
        assert!(folder(t.path()).is_err());
        assert!(existing_folder(t.path()).is_none());
    }

    #[test]
    fn existing_folder_is_never_created() {
        let t = tempfile::tempdir().unwrap();
        assert!(existing_folder(t.path()).is_none());
        assert!(!t.path().join(FOLDER).exists());
        folder(t.path()).unwrap();
        assert_eq!(existing_folder(t.path()), Some(t.path().join(FOLDER)));
    }
}
