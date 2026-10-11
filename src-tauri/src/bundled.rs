//! Our own mods ship inside the launcher. On every Play they are written into
//! the Skyrim folder (when missing or different), and the paths come back so
//! the no-mods gate treats exactly these files as the launcher's own.

use sha2::{Digest, Sha256};
use std::path::Path;

/// Path relative to the game folder, and the file's bytes.
/// The recipe plugin: ours, loaded last. Listed in the server order locally (see add_plugin_entry) until the server lists it itself.
pub const ESP_NAME: &str = "AetherialDawn.esp";
const ESP: &[u8] = include_bytes!("../bundled/Data/AetherialDawn.esp");

const FILES: &[(&str, &[u8])] = &[
    ("Data/AetherialDawn.esp", ESP),
    ("Data/SKSE/Plugins/AetherialSkillTree.dll", include_bytes!("../bundled/Data/SKSE/Plugins/AetherialSkillTree.dll")),
    ("Data/SKSE/Plugins/AetherialDawn-NoPause.dll", include_bytes!("../bundled/Data/SKSE/Plugins/AetherialDawn-NoPause.dll")),
    ("Data/SKSE/Plugins/AetherialQuietPickups.dll", include_bytes!("../bundled/Data/SKSE/Plugins/AetherialQuietPickups.dll")),
];

/// Settings files: written once, then the player's (or staff's) edits stay.
const SETTINGS: &[(&str, &[u8])] = &[("Data/SKSE/Plugins/AetherialQuietPickups.ini", include_bytes!("../bundled/Data/SKSE/Plugins/AetherialQuietPickups.ini"))];

/// Our own mods that are switched off for now: the launcher takes its own copies back out.
const RETIRED: &[&str] = &["Data/SKSE/Plugins/AetherialFreezeMenus.dll"];

fn same(path: &Path, bytes: &[u8]) -> bool {
    std::fs::read(path).is_ok_and(|b| b == bytes)
}

/// Writes our files into the game folder; returns every path that is ours.
/// A file that cannot be written is logged and still counted as ours.
pub fn deploy(game_dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for rel in RETIRED {
        let _ = std::fs::remove_file(game_dir.join(rel));
    }
    for (rel, bytes) in FILES.iter().chain(SETTINGS.iter()) {
        out.push((*rel).to_string());
        let path = game_dir.join(rel);
        let is_setting = SETTINGS.iter().any(|(r, _)| r == rel);
        if (is_setting && path.exists()) || same(&path, bytes) {
            continue;
        }
        let r = path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| std::fs::write(&path, bytes));
        if let Err(e) = r {
            crate::log::line(&format!("Couldn't write our file {rel}: {e}"));
        }
    }
    out
}

/// Adds AetherialDawn.esp, last, to the server's masters list when the server does not list it yet.
pub fn add_plugin_entry(masters: &mut serde_json::Value) {
    let key = if masters.get("masters").is_some() { "masters" } else if masters.get("files").is_some() { "files" } else { return };
    let Some(list) = masters.get_mut(key).and_then(|l| l.as_array_mut()) else { return };
    let listed = list.iter().any(|e| e.get("name").and_then(|n| n.as_str()).is_some_and(|n| n.eq_ignore_ascii_case(ESP_NAME)));
    if !listed {
        let sha: String = Sha256::digest(ESP).iter().map(|b| format!("{b:02x}")).collect();
        list.push(serde_json::json!({ "name": ESP_NAME, "size": ESP.len(), "sha256": sha }));
    }
}

/// One line for the session report: is our plugin there and switched on.
pub fn report_line(game_dir: &Path, plugins_txt: Option<&Path>) -> String {
    let size = std::fs::metadata(game_dir.join("Data").join(ESP_NAME)).map(|m| m.len()).ok();
    let on = plugins_txt
        .and_then(|p| std::fs::read_to_string(p).ok())
        .is_some_and(|t| t.lines().any(|l| l.trim().trim_start_matches('*').eq_ignore_ascii_case(ESP_NAME)));
    format!("{ESP_NAME}: {} in Data, {} in plugins.txt
", size.map_or("missing".to_string(), |s| format!("{s} bytes")), if on { "switched on" } else { "not switched on" })
}

#[cfg(test)]
mod tests {
    #[test]
    fn deploys_every_file_and_keeps_edited_settings() {
        let d = tempfile::tempdir().unwrap();
        let ours = super::deploy(d.path());
        assert_eq!(ours.len(), super::FILES.len() + super::SETTINGS.len());
        let dll = d.path().join("Data/SKSE/Plugins/AetherialQuietPickups.dll");
        let ini = d.path().join("Data/SKSE/Plugins/AetherialQuietPickups.ini");
        std::fs::write(&dll, b"old").unwrap();
        std::fs::write(&ini, b"edited").unwrap();
        super::deploy(d.path());
        assert_ne!(std::fs::read(&dll).unwrap(), b"old");
        assert_eq!(std::fs::read(&ini).unwrap(), b"edited");
    }

    #[test]
    fn the_plugin_joins_the_server_list_once_and_last() {
        let mut m = serde_json::json!({ "masters": [{ "name": "Skyrim.esm" }] });
        super::add_plugin_entry(&mut m);
        super::add_plugin_entry(&mut m);
        let l = m["masters"].as_array().unwrap();
        assert_eq!(l.len(), 2);
        assert_eq!(l[1]["name"], "AetherialDawn.esp");
        assert_eq!(l[1]["size"], 11808);
        assert!(l[1]["sha256"].as_str().unwrap().starts_with("2e04a9b3"));
    }
}
