//! Finds plugins that aren't part of the server's client files: leftover SKSE
//! plugins and SkyrimPlatform scripts from older setups or other mods. They
//! load into the game alongside SkyMP and can crash it before the main menu,
//! so the launcher offers to move them aside (never deletes them).

use std::path::{Path, PathBuf};

use crate::manifest::Manifest;
use crate::Result;

const SKSE_PLUGINS: &str = "Data/SKSE/Plugins";
const PLATFORM_PLUGINS: &str = "Data/Platform/Plugins";
/// Where moved files go, inside the game folder, keeping their paths.
pub const DISABLED_DIR: &str = ".aetherial-dawn/disabled";

fn listed(m: &Manifest, rel: &str) -> bool {
    m.files.iter().any(|f| f.path.eq_ignore_ascii_case(rel))
}

/// Game-relative paths ("Data/SKSE/Plugins/x.dll") of plugins the server
/// didn't ship: every .dll in SKSE's plugin folder, and every file in
/// SkyrimPlatform's plugin folder except the settings file the launcher writes.
pub fn find(game_dir: &Path, m: &Manifest) -> Vec<String> {
    let mut out = Vec::new();
    for (folder, only_dll) in [(SKSE_PLUGINS, true), (PLATFORM_PLUGINS, false)] {
        let Ok(rd) = std::fs::read_dir(game_dir.join(folder)) else { continue };
        for e in rd.flatten() {
            if !e.path().is_file() {
                continue;
            }
            let name = e.file_name().to_string_lossy().into_owned();
            let rel = format!("{folder}/{name}");
            if only_dll && !name.to_ascii_lowercase().ends_with(".dll") {
                continue;
            }
            if rel.eq_ignore_ascii_case(crate::settings::SETTINGS_PATH) || listed(m, &rel) {
                continue;
            }
            out.push(rel);
        }
    }
    out.sort();
    out
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{FileEntry, Server};

    #[test]
    fn finds_and_moves_strays() {
        let tmp = std::env::temp_dir().join(format!("ad-strays-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        for f in [
            "Data/SKSE/Plugins/MpClientPlugin.dll",
            "Data/SKSE/Plugins/OldProbe.dll",
            "Data/SKSE/Plugins/OldProbe.ini",
            "Data/Platform/Plugins/skymp5-client.js",
            "Data/Platform/Plugins/skymp5-client-settings.txt",
            "Data/Platform/Plugins/rp-portrait.js",
        ] {
            let p = tmp.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b"x").unwrap();
        }
        let entry = |p: &str| FileEntry { path: p.into(), size: 1, sha256: String::new() };
        let m = Manifest {
            schema: 1,
            build: "b".into(),
            server: Server { name: "s".into(), ip: "1.2.3.4".into(), port: 7777 },
            master: String::new(),
            files: vec![entry("Data/SKSE/Plugins/MpClientPlugin.dll"), entry("Data/Platform/Plugins/skymp5-client.js")],
            remove: vec![],
            game: None,
        };
        let s = find(&tmp, &m);
        assert_eq!(s, ["Data/Platform/Plugins/rp-portrait.js", "Data/SKSE/Plugins/OldProbe.dll"]);
        let dest = move_aside(&tmp, &s, "t1").unwrap();
        assert!(dest.join("Data/SKSE/Plugins/OldProbe.dll").is_file());
        assert!(find(&tmp, &m).is_empty());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
