//! Keeps the server's Skyrim build on hand after a good downgrade, so the
//! launcher can put it back by itself when Steam updates or repairs the game,
//! with no Steam sign-in and nothing for the player to do.
//!
//! The copies are hard links in `.aetherial-dawn/pristine/`, on the same drive
//! as the game, so they take no extra space until Steam replaces a file.
//! Steam writes an update as a new file and swaps it in, which leaves our link
//! pointing at the old build. If a file is ever changed in place instead, its
//! size or date no longer matches what was recorded, and the launcher falls
//! back to downloading.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::manifest::GameSpec;
use crate::Result;

pub const DIR: &str = ".aetherial-dawn/pristine";
const INDEX: &str = "index.json";

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct Entry {
    path: String,
    size: u64,
    mtime: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct Index {
    version: String,
    depots: Vec<(u32, String)>,
    files: Vec<Entry>,
}

fn stamp(p: &Path) -> Option<(u64, u64)> {
    let md = std::fs::metadata(p).ok()?;
    Some((md.len(), md.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_secs()))
}

/// Top-level files Steam's Skyrim depots put in place.
const TOP: [&str; 4] = ["skyrimse.exe", "skyrimselauncher.exe", "steam_api64.dll", "bink2w64.dll"];
const BASE_MASTERS: [&str; 5] = ["skyrim.esm", "update.esm", "dawnguard.esm", "hearthfires.esm", "dragonborn.esm"];

/// Steam's own files only: the game exe and its libraries, the base masters,
/// Bethesda's archives, the Creation Club content and list. Mod files are
/// never kept, so something the player removes is never brought back.
fn is_steam_data(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    if BASE_MASTERS.contains(&l.as_str()) || l == "skyrim.ccc" {
        return true;
    }
    let archive_or_plugin = l.ends_with(".bsa") || l.ends_with(".esm") || l.ends_with(".esl");
    archive_or_plugin && (l.starts_with("skyrim - ") || l.starts_with("_resourcepack.") || l.starts_with("cc"))
}

fn steam_files(game_dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(game_dir) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if e.path().is_file() && TOP.contains(&n.to_ascii_lowercase().as_str()) {
                out.push(n);
            }
        }
    }
    if let Ok(rd) = std::fs::read_dir(game_dir.join("Data")) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if e.path().is_file() && is_steam_data(&n) {
                out.push(format!("Data/{n}"));
            }
        }
    }
    out.sort();
    out
}

fn link_or_copy(from: &Path, to: &Path) -> std::io::Result<()> {
    if let Some(p) = to.parent() {
        std::fs::create_dir_all(p)?;
    }
    let _ = std::fs::remove_file(to);
    std::fs::hard_link(from, to).or_else(|_| std::fs::copy(from, to).map(|_| ()))
}

fn spec_key(spec: &GameSpec) -> (String, Vec<(u32, String)>) {
    (spec.version.clone().unwrap_or_default(), spec.depots.iter().map(|d| (d.depot, d.manifest.clone())).collect())
}

/// Records the game folder, just confirmed to be the server's build.
/// Returns the number of files kept.
pub fn save(game_dir: &Path, spec: &GameSpec) -> Result<usize> {
    let root = game_dir.join(DIR);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root)?;
    let mut files = Vec::new();
    for rel in steam_files(game_dir) {
        let src = game_dir.join(&rel);
        let dst = root.join(&rel);
        // Links only: a full copy of the game would take gigabytes, so a
        // drive that can't link (FAT32/exFAT) simply gets no saved copy.
        if let Some(p) = dst.parent() {
            std::fs::create_dir_all(p)?;
        }
        std::fs::hard_link(&src, &dst).map_err(|e| {
            let _ = std::fs::remove_dir_all(&root);
            crate::Error::Game(format!("this drive can't keep a linked copy of {rel} ({e})"))
        })?;
        let (size, mtime) = stamp(&dst).ok_or_else(|| crate::Error::Game(format!("couldn't read {rel}")))?;
        files.push(Entry { path: rel, size, mtime });
    }
    let (version, depots) = spec_key(spec);
    let n = files.len();
    std::fs::write(root.join(INDEX), serde_json::to_vec_pretty(&Index { version, depots, files })?)?;
    Ok(n)
}

/// Whether a saved copy of this exact build exists and is intact.
pub fn available(game_dir: &Path, spec: &GameSpec) -> bool {
    load(game_dir, spec).is_some()
}

fn load(game_dir: &Path, spec: &GameSpec) -> Option<Index> {
    let root = game_dir.join(DIR);
    let idx: Index = serde_json::from_slice(&std::fs::read(root.join(INDEX)).ok()?).ok()?;
    let (version, depots) = spec_key(spec);
    if idx.version != version || idx.depots != depots || idx.files.is_empty() {
        return None;
    }
    let intact = idx.files.iter().all(|e| stamp(&root.join(&e.path)) == Some((e.size, e.mtime)));
    intact.then_some(idx)
}

/// Puts the saved build back over whatever Steam changed. Returns the files
/// replaced, or None when there is no intact copy of this build.
pub fn restore(game_dir: &Path, spec: &GameSpec) -> Result<Option<Vec<String>>> {
    let Some(idx) = load(game_dir, spec) else { return Ok(None) };
    let root: PathBuf = game_dir.join(DIR);
    let mut replaced = Vec::new();
    for e in &idx.files {
        let saved = root.join(&e.path);
        let live = game_dir.join(&e.path);
        if stamp(&live) == Some((e.size, e.mtime)) && same_file(&saved, &live) {
            continue;
        }
        link_or_copy(&saved, &live).map_err(|err| crate::Error::Game(format!("couldn't put back {} ({err}). Close Skyrim and Steam, then try again.", e.path)))?;
        replaced.push(e.path.clone());
    }
    Ok(Some(replaced))
}

#[cfg(unix)]
fn same_file(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    matches!((std::fs::metadata(a), std::fs::metadata(b)), (Ok(x), Ok(y)) if x.ino() == y.ino() && x.dev() == y.dev())
}

#[cfg(not(unix))]
fn same_file(a: &Path, b: &Path) -> bool {
    // Size and date already match; compare the first and last bytes too.
    use std::io::{Read, Seek, SeekFrom};
    let head = |p: &Path| -> Option<Vec<u8>> {
        let mut f = std::fs::File::open(p).ok()?;
        let mut buf = vec![0u8; 4096];
        let n = f.read(&mut buf).ok()?;
        buf.truncate(n);
        let len = f.metadata().ok()?.len();
        if len > 8192 {
            f.seek(SeekFrom::Start(len - 4096)).ok()?;
            let mut tail = vec![0u8; 4096];
            f.read_exact(&mut tail).ok()?;
            buf.extend(tail);
        }
        Some(buf)
    };
    head(a).is_some() && head(a) == head(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Depot;

    fn spec() -> GameSpec {
        GameSpec { version: Some("1.6.1170.0".into()), skse_version: None, app: 489830, depots: vec![Depot { depot: 489831, manifest: "1".into() }], tool: None }
    }

    #[test]
    fn saves_and_puts_back_after_steam_replaces_files() {
        let tmp = tempfile::tempdir().unwrap();
        let g = tmp.path();
        std::fs::create_dir_all(g.join("Data")).unwrap();
        std::fs::write(g.join("SkyrimSE.exe"), b"exe 1.6").unwrap();
        std::fs::write(g.join("Data/Skyrim.esm"), b"esm 1.6").unwrap();
        std::fs::write(g.join("Data/Mod.esp"), b"not steam").unwrap();
        std::fs::write(g.join("Data/SomeMod.bsa"), b"not steam").unwrap();
        std::fs::write(g.join("dinput8.dll"), b"not steam").unwrap();
        assert_eq!(save(g, &spec()).unwrap(), 2);
        assert!(available(g, &spec()));
        // Nothing changed: nothing to put back.
        assert_eq!(restore(g, &spec()).unwrap().unwrap(), Vec::<String>::new());
        // Steam swaps in a new file (new inode), leaving our link on the old one.
        std::fs::remove_file(g.join("Data/Skyrim.esm")).unwrap();
        std::fs::write(g.join("Data/Skyrim.esm"), b"esm 1.7 longer").unwrap();
        assert_eq!(restore(g, &spec()).unwrap().unwrap(), ["Data/Skyrim.esm"]);
        assert_eq!(std::fs::read(g.join("Data/Skyrim.esm")).unwrap(), b"esm 1.6");
        // A different build on the server: the saved copy doesn't count.
        let mut other = spec();
        other.version = Some("1.6.1179.0".into());
        assert!(restore(g, &other).unwrap().is_none());
    }

    #[test]
    fn in_place_change_disables_the_copy() {
        let tmp = tempfile::tempdir().unwrap();
        let g = tmp.path();
        std::fs::create_dir_all(g.join("Data")).unwrap();
        std::fs::write(g.join("SkyrimSE.exe"), b"exe").unwrap();
        save(g, &spec()).unwrap();
        // Writing through the shared link changes the saved copy too.
        std::fs::write(g.join("SkyrimSE.exe"), b"patched in place").unwrap();
        assert!(!available(g, &spec()));
    }
}
