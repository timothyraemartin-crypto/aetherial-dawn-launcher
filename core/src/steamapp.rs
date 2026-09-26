//! Downgrading through the Steam app the player is already signed into.
//! Steam won't let another program start a depot download, but its console
//! (steam://open/console) takes `download_depot <app> <depot> <manifest>` from
//! the player. Steam then downloads with the player's own login into
//! `<steam>/steamapps/content/app_<app>/depot_<depot>/`; this module watches
//! those folders and copies the files into the game folder when the player
//! says Steam has finished.

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::manifest::GameSpec;
use crate::{Error, Result};

/// A folder must be this long without changes before its files are copied.
pub const SETTLE_SECS: u64 = 10;

/// The Steam install folder (not a library folder): where `steamapps/content` lives.
pub fn steam_root() -> Option<PathBuf> {
    crate::game::steam_roots().into_iter().find(|r| r.join("steamapps").is_dir())
}

/// Whether the Steam app is open and signed in on this PC.
#[cfg(windows)]
pub fn steam_running() -> bool {
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};
    let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey(r"Software\Valve\Steam\ActiveProcess");
    let pid = key.as_ref().ok().and_then(|k| k.get_value::<u32, _>("pid").ok()).unwrap_or(0);
    let user = key.as_ref().ok().and_then(|k| k.get_value::<u32, _>("ActiveUser").ok()).unwrap_or(1);
    pid != 0 && user != 0
}

#[cfg(not(windows))]
pub fn steam_running() -> bool {
    steam_root().is_some()
}

/// The lines the player pastes into Steam's console, one per depot.
pub fn commands(spec: &GameSpec) -> Vec<String> {
    spec.depots.iter().map(|d| format!("download_depot {} {} {}", spec.app, d.depot, d.manifest)).collect()
}

pub fn content_dir(root: &Path, app: u32, depot: u32) -> PathBuf {
    root.join("steamapps").join("content").join(format!("app_{app}")).join(format!("depot_{depot}"))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DepotState {
    pub depot: u32,
    pub command: String,
    pub present: bool,
    pub files: usize,
    pub bytes: u64,
    /// Seconds since anything in the folder last changed.
    pub quiet_secs: u64,
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else {
            out.push(p);
        }
    }
}

pub fn state(root: &Path, spec: &GameSpec) -> Vec<DepotState> {
    let cmds = commands(spec);
    spec.depots
        .iter()
        .zip(cmds)
        .map(|(d, command)| {
            let dir = content_dir(root, spec.app, d.depot);
            let mut files = Vec::new();
            walk(&dir, &mut files);
            let mut bytes = 0;
            let mut newest: Option<SystemTime> = None;
            for f in &files {
                if let Ok(m) = std::fs::metadata(f) {
                    bytes += m.len();
                    if let Ok(t) = m.modified() {
                        newest = Some(newest.map_or(t, |n: SystemTime| n.max(t)));
                    }
                }
            }
            let quiet_secs = newest.and_then(|t| SystemTime::now().duration_since(t).ok()).map(|d| d.as_secs()).unwrap_or(0);
            DepotState { depot: d.depot, command, present: !files.is_empty(), files: files.len(), bytes, quiet_secs }
        })
        .collect()
}

/// Removes leftovers from an earlier download so old files can't be mistaken
/// for the new ones.
pub fn clear(root: &Path, spec: &GameSpec) {
    for d in &spec.depots {
        let _ = std::fs::remove_dir_all(content_dir(root, spec.app, d.depot));
    }
}

/// Copies every downloaded depot into the game folder, overwriting what's
/// there, then deletes the downloaded copy to give the space back.
/// Returns the number of files copied.
pub fn install(root: &Path, spec: &GameSpec, game_dir: &Path) -> Result<usize> {
    let st = state(root, spec);
    if let Some(d) = st.iter().find(|d| !d.present) {
        return Err(Error::Game(format!(
            "Steam hasn't downloaded depot {} yet. Paste its line into Steam's console and wait for \"Depot download complete\".",
            d.depot
        )));
    }
    if let Some(d) = st.iter().find(|d| d.quiet_secs < SETTLE_SECS) {
        return Err(Error::Game(format!(
            "Steam is still writing depot {}. Wait until the console says \"Depot download complete\" for all {} lines, then try again.",
            d.depot,
            st.len()
        )));
    }
    let mut copied = 0;
    for d in &spec.depots {
        let src = content_dir(root, spec.app, d.depot);
        let mut files = Vec::new();
        walk(&src, &mut files);
        for f in files {
            let rel = f.strip_prefix(&src).map_err(|e| Error::Game(e.to_string()))?;
            let to = game_dir.join(rel);
            if let Some(p) = to.parent() {
                std::fs::create_dir_all(p)?;
            }
            std::fs::copy(&f, &to).map_err(|e| Error::Game(format!("couldn't copy {} into Skyrim: {e}. Close Skyrim and try again.", rel.display())))?;
            copied += 1;
        }
    }
    clear(root, spec);
    Ok(copied)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> GameSpec {
        serde_json::from_value(serde_json::json!({
            "version": "1.6.1170.0",
            "depots": [{"depot": 489831, "manifest": "111"}, {"depot": 489833, "manifest": "333"}]
        }))
        .unwrap()
    }

    #[test]
    fn console_lines() {
        assert_eq!(commands(&spec()), ["download_depot 489830 489831 111", "download_depot 489830 489833 333"]);
    }

    #[test]
    fn waits_then_copies() {
        let tmp = std::env::temp_dir().join(format!("ad-steamapp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let root = tmp.join("steam");
        let game = tmp.join("game");
        std::fs::create_dir_all(&game).unwrap();
        let s = spec();
        assert!(install(&root, &s, &game).unwrap_err().to_string().contains("489831"));
        let d1 = content_dir(&root, 489830, 489831);
        let d3 = content_dir(&root, 489830, 489833);
        std::fs::create_dir_all(d1.join("Data")).unwrap();
        std::fs::create_dir_all(&d3).unwrap();
        std::fs::write(d1.join("Data/Skyrim.esm"), b"esm").unwrap();
        std::fs::write(d3.join("SkyrimSE.exe"), b"exe").unwrap();
        // Just written, so it isn't settled yet.
        assert!(install(&root, &s, &game).unwrap_err().to_string().contains("still writing"));
        let old = std::fs::FileTimes::new().set_modified(SystemTime::now() - std::time::Duration::from_secs(60));
        for p in [d1.join("Data/Skyrim.esm"), d3.join("SkyrimSE.exe")] {
            std::fs::File::options().write(true).open(&p).unwrap().set_times(old).unwrap();
        }
        let st = state(&root, &s);
        assert!(st.iter().all(|d| d.present && d.quiet_secs >= SETTLE_SECS), "{st:?}");
        assert_eq!(install(&root, &s, &game).unwrap(), 2);
        assert_eq!(std::fs::read(game.join("Data/Skyrim.esm")).unwrap(), b"esm");
        assert!(!d1.exists());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
