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
    let mut cmd = std::process::Command::new(dir.join(SKSE_LOADER));
    cmd.current_dir(dir);
    for name in GOOGLE_ENV {
        cmd.env_remove(name);
    }
    Ok(cmd.spawn()?)
}

/// Skyrim Platform watches every folder named by PluginFolders in
/// Data/SKSE/Plugins/SkyrimPlatform.ini (by default Data/Platform/Plugins and
/// Data/Platform/PluginsDev). A folder that doesn't exist makes it throw
/// "DirectoryMonitor(...) failed with code 2", and the SkyMP client then
/// never shows its login or connects (seen 2026-09-26). Makes any missing
/// folder, empty. Returns the folders made.
pub fn ensure_platform_folders(dir: &Path) -> Result<Vec<String>> {
    let ini = std::fs::read_to_string(dir.join("Data/SKSE/Plugins/SkyrimPlatform.ini")).unwrap_or_default();
    let mut folders = vec!["Data/Platform/Plugins".to_string(), "Data/Platform/PluginsDev".to_string()];
    for line in ini.lines() {
        let line = line.trim();
        if let Some((k, v)) = line.split_once('=') {
            if k.trim().eq_ignore_ascii_case("PluginFolders") {
                folders.extend(v.split(';').map(|f| f.trim().replace('\\', "/")).filter(|f| !f.is_empty()));
            }
        }
    }
    let mut made = Vec::new();
    for f in folders {
        // Only folders inside the game folder, never absolute paths or "..".
        if f.contains(':') || f.starts_with('/') || f.split('/').any(|c| c == "..") {
            continue;
        }
        let p = dir.join(&f);
        if !p.is_dir() {
            std::fs::create_dir_all(&p)?;
            made.push(f);
        }
    }
    made.sort();
    made.dedup();
    Ok(made)
}

/// Google sign-in settings some Chromium guides and tools put in the Windows
/// environment. Skyrim Platform's browser (CEF 108) reads them, turns on
/// Google sign-in code it doesn't support, and crashes about 5 seconds in
/// (seen 2026-09-26: a null read in libcef.dll while reading sign-in
/// preferences). The game never needs them, so they're left out of its
/// environment. The player's own settings aren't changed.
pub const GOOGLE_ENV: [&str; 9] = [
    "GOOGLE_API_KEY",
    "GOOGLE_DEFAULT_CLIENT_ID",
    "GOOGLE_DEFAULT_CLIENT_SECRET",
    "GOOGLE_CLIENT_ID_MAIN",
    "GOOGLE_CLIENT_SECRET_MAIN",
    "GOOGLE_CLIENT_ID_REMOTING",
    "GOOGLE_CLIENT_SECRET_REMOTING",
    "GOOGLE_CLIENT_ID_REMOTING_HOST",
    "GOOGLE_CLIENT_SECRET_REMOTING_HOST",
];

/// Which of those are set on this PC (names only, never values).
pub fn google_env_present() -> Vec<&'static str> {
    GOOGLE_ENV.iter().copied().filter(|n| std::env::var_os(n).is_some()).collect()
}

/// Asks Windows to run Skyrim on the high-performance graphics card (Settings,
/// Display, Graphics), the same as choosing it by hand. PCs with a built-in
/// Intel chip next to the gaming card can otherwise start Skyrim Platform's
/// browser on the wrong adapter. A choice the player already made is kept.
/// Returns true when the setting was added.
#[cfg(windows)]
pub fn prefer_fast_gpu(game_dir: &Path) -> std::io::Result<bool> {
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(r"Software\Microsoft\DirectX\UserGpuPreferences")?;
    let exe = gpu_pref_path(game_dir);
    if key.get_value::<String, _>(&exe).is_ok() {
        return Ok(false);
    }
    key.set_value(&exe, &"GpuPreference=2;")?;
    Ok(true)
}

#[cfg(not(windows))]
pub fn prefer_fast_gpu(_game_dir: &Path) -> std::io::Result<bool> {
    Ok(false)
}

/// The exe path the way Windows writes it in that setting.
pub fn gpu_pref_path(game_dir: &Path) -> String {
    let mut p = game_dir.join(GAME_EXE).to_string_lossy().replace('/', "\\");
    while p.contains("\\\\") {
        p = p.replace("\\\\", "\\");
    }
    if p.as_bytes().get(1) == Some(&b':') {
        p = p[..1].to_ascii_uppercase() + &p[1..];
    }
    p
}

#[cfg(test)]
mod tests {
    #[test]
    fn makes_missing_platform_folders() {
        let tmp = tempfile::tempdir().unwrap();
        let g = tmp.path();
        std::fs::create_dir_all(g.join("Data/SKSE/Plugins")).unwrap();
        std::fs::write(g.join("Data/SKSE/Plugins/SkyrimPlatform.ini"), "[Main]\nPluginFolders = Data/Platform/Plugins;Data/Platform/PluginsDev;C:/evil;../up\n").unwrap();
        let made = super::ensure_platform_folders(g).unwrap();
        assert_eq!(made, ["Data/Platform/Plugins", "Data/Platform/PluginsDev"]);
        assert!(g.join("Data/Platform/PluginsDev").is_dir());
        assert!(super::ensure_platform_folders(g).unwrap().is_empty());
    }
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

    #[test]
    fn writes_the_exe_path_like_windows() {
        let p = gpu_pref_path(Path::new("a:\\steam\\steamapps/common/Skyrim Special Edition"));
        if cfg!(windows) {
            assert_eq!(p, "A:\\steam\\steamapps\\common\\Skyrim Special Edition\\SkyrimSE.exe");
        } else {
            assert!(p.starts_with("A:\\steam\\steamapps\\common\\Skyrim Special Edition"));
            assert!(p.ends_with("SkyrimSE.exe"));
        }
    }
}
