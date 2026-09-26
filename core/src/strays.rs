//! Finds plugins that aren't part of the server's client files: leftover SKSE
//! plugins and SkyrimPlatform scripts from older setups or other mods. They
//! load into the game alongside SkyMP and can crash it before the main menu,
//! so the launcher offers to move them aside (never deletes them).

use std::path::{Path, PathBuf};

use crate::manifest::Manifest;
use crate::Result;

const SKSE_PLUGINS: &str = "Data/SKSE/Plugins";
const PLATFORM_PLUGINS: &str = "Data/Platform/Plugins";
/// Loose menus here replace the game's own (RaceMenu, map and HUD mods).
/// Vanilla Skyrim keeps its interface in BSAs, so nothing loose belongs here.
const INTERFACE: &str = "Data/Interface";
/// SKSE crash loggers the launcher leaves in place so crash reports name the
/// module that failed.
pub const CRASH_LOGGERS: [&str; 3] = ["CrashLogger.dll", "TrainwreckSKSE.dll", "NetScriptFramework.Runtime.dll"];
const VORTEX_MARKER: &str = "__folder_managed_by_vortex";
/// Where moved files go, inside the game folder, keeping their paths.
pub const DISABLED_DIR: &str = ".aetherial-dawn/disabled";

fn listed(m: &Manifest, rel: &str) -> bool {
    m.files.iter().any(|f| f.path.eq_ignore_ascii_case(rel))
}

/// Game-relative paths ("Data/SKSE/Plugins/x.dll") of plugins the server
/// didn't ship: every .dll in SKSE's plugin folder, and every file in
/// SkyrimPlatform's plugin folder except the settings file the launcher writes.
pub fn find(game_dir: &Path, m: &Manifest) -> Vec<String> {
    // Files of mods on the server's list (however they were installed) stay.
    let keep = crate::allowlist::keep_set(game_dir);
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
            // Crash loggers only write a log when the game dies, and required
            // mods (Skyrim Souls RE and its dependencies) must stay; keep them.
            if CRASH_LOGGERS.iter().chain(crate::requirements::CRASH_LOGGER_FILES.iter()).chain(crate::requirements::SOULS_FILES.iter()).chain(crate::requirements::ENGINE_FIXES_FILES.iter()).chain([crate::requirements::MENU_FRAMEWORK_DLL].iter()).any(|c| c.eq_ignore_ascii_case(&name)) {
                continue;
            }
            if rel.eq_ignore_ascii_case(crate::settings::SETTINGS_PATH) || listed(m, &rel) || keep.contains(&rel.to_ascii_lowercase()) {
                continue;
            }
            out.push(rel);
        }
    }
    let mut loose = Vec::new();
    walk(&game_dir.join(INTERFACE), INTERFACE, &mut loose);
    // Required mods' menus (Skyrim Souls RE) and ImGui Icons' fonts stay.
    let icons = format!("{INTERFACE}/{}/", crate::requirements::IMGUI_ICONS_DIR).to_ascii_lowercase();
    let required = |rel: &str| {
        crate::requirements::SOULS_FILES.iter().any(|f| rel.eq_ignore_ascii_case(&format!("{INTERFACE}/{f}"))) || rel.to_ascii_lowercase().starts_with(&icons)
    };
    out.extend(loose.into_iter().filter(|rel| !listed(m, rel) && !rel.ends_with(VORTEX_MARKER) && !required(rel) && !keep.contains(&rel.to_ascii_lowercase())));
    out.sort();
    out
}

fn walk(dir: &Path, rel: &str, out: &mut Vec<String>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let r = format!("{rel}/{name}");
        if e.path().is_dir() {
            walk(&e.path(), &r, out);
        } else {
            out.push(r);
        }
    }
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
            "Data/Interface/racesex_menu.swf",
            "Data/Interface/racemenu/buttonart.swf",
            "Data/Interface/__folder_managed_by_vortex",
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
        assert_eq!(
            s,
            [
                "Data/Interface/racemenu/buttonart.swf",
                "Data/Interface/racesex_menu.swf",
                "Data/Platform/Plugins/rp-portrait.js",
                "Data/SKSE/Plugins/OldProbe.dll"
            ]
        );
        let dest = move_aside(&tmp, &s, "t1").unwrap();
        assert!(dest.join("Data/SKSE/Plugins/OldProbe.dll").is_file());
        assert!(find(&tmp, &m).is_empty());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
