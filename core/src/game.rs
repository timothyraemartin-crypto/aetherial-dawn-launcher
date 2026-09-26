//! Finding Skyrim Special Edition (which includes the Anniversary Edition
//! upgrade), checking for SKSE, and starting the game through it.

use serde::Serialize;
use std::path::{Path, PathBuf};

use crate::{Error, Result};

pub const GAME_EXE: &str = "SkyrimSE.exe";
pub const SKSE_LOADER: &str = "skse64_loader.exe";
const STEAM_FOLDER: &str = "steamapps/common/Skyrim Special Edition";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameInfo {
    pub dir: PathBuf,
    pub has_skse: bool,
}

pub fn inspect(dir: &Path) -> Result<GameInfo> {
    if !dir.join(GAME_EXE).is_file() {
        return Err(Error::Game(format!("{GAME_EXE} isn't in {}. Pick your Skyrim Special Edition folder.", dir.display())));
    }
    Ok(GameInfo { dir: dir.to_path_buf(), has_skse: dir.join(SKSE_LOADER).is_file() })
}

/// Looks in every Steam library for Skyrim SE. Returns the first folder that
/// has the game in it.
pub fn detect() -> Option<GameInfo> {
    steam_roots()
        .into_iter()
        .flat_map(|root| {
            let vdf = std::fs::read_to_string(root.join("steamapps/libraryfolders.vdf")).unwrap_or_default();
            let mut libs = library_paths(&vdf);
            libs.insert(0, root);
            libs
        })
        .map(|lib| lib.join(STEAM_FOLDER))
        .find_map(|dir| inspect(&dir).ok())
}

/// Pulls the `"path"` values out of Steam's libraryfolders.vdf.
pub fn library_paths(vdf: &str) -> Vec<PathBuf> {
    vdf.lines()
        .filter_map(|line| {
            let mut quoted = line.split('"').skip(1).step_by(2);
            match (quoted.next(), quoted.next()) {
                (Some("path"), Some(v)) => Some(PathBuf::from(v.replace("\\\\", "\\"))),
                _ => None,
            }
        })
        .collect()
}

#[cfg(windows)]
pub fn steam_roots() -> Vec<PathBuf> {
    use winreg::{enums::*, RegKey};
    let mut roots = Vec::new();
    let keys = [
        (HKEY_CURRENT_USER, r"Software\Valve\Steam", "SteamPath"),
        (HKEY_LOCAL_MACHINE, r"SOFTWARE\WOW6432Node\Valve\Steam", "InstallPath"),
        (HKEY_LOCAL_MACHINE, r"SOFTWARE\Valve\Steam", "InstallPath"),
    ];
    for (hive, path, value) in keys {
        if let Ok(v) = RegKey::predef(hive).open_subkey(path).and_then(|k| k.get_value::<String, _>(value)) {
            let p = PathBuf::from(v.replace('/', "\\"));
            if !roots.contains(&p) {
                roots.push(p);
            }
        }
    }
    roots.push(PathBuf::from(r"C:\Program Files (x86)\Steam"));
    roots
}

#[cfg(not(windows))]
pub fn steam_roots() -> Vec<PathBuf> {
    std::env::var_os("HOME")
        .map(|h| {
            let h = PathBuf::from(h);
            vec![h.join(".steam/steam"), h.join(".local/share/Steam")]
        })
        .unwrap_or_default()
}

pub fn launch(dir: &Path) -> Result<std::process::Child> {
    let info = inspect(dir)?;
    if !info.has_skse {
        return Err(Error::Game("SKSE isn't installed. Install it from skse.silverlock.org, then try again.".into()));
    }
    Ok(std::process::Command::new(dir.join(SKSE_LOADER)).current_dir(dir).spawn()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_library_folders() {
        let vdf = r#"
"libraryfolders"
{
	"0"
	{
		"path"		"C:\\Program Files (x86)\\Steam"
		"label"		""
	}
	"1"
	{
		"path"		"D:\\SteamLibrary"
		"apps" { "489830"		"13071174321" }
	}
}"#;
        assert_eq!(
            library_paths(vdf),
            [PathBuf::from(r"C:\Program Files (x86)\Steam"), PathBuf::from(r"D:\SteamLibrary")]
        );
    }

    #[test]
    fn inspect_needs_game_exe() {
        let dir = tempfile::tempdir().unwrap();
        assert!(inspect(dir.path()).is_err());
        std::fs::write(dir.path().join(GAME_EXE), b"").unwrap();
        assert!(!inspect(dir.path()).unwrap().has_skse);
        std::fs::write(dir.path().join(SKSE_LOADER), b"").unwrap();
        assert!(inspect(dir.path()).unwrap().has_skse);
    }
}
