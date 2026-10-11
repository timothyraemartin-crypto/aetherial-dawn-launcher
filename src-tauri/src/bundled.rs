//! Our own mods ship inside the launcher. On every Play they are written into
//! the Skyrim folder (when missing or different), and the paths come back so
//! the no-mods gate treats exactly these files as the launcher's own.

use std::path::Path;

/// Path relative to the game folder, and the file's bytes.
const ESP_NAME: &str = "AetherialDawn.esp";

const FILES: &[(&str, &[u8])] = &[
    ("Data/SKSE/Plugins/AetherialSkillTree.dll", include_bytes!("../bundled/Data/SKSE/Plugins/AetherialSkillTree.dll")),
    ("Data/SKSE/Plugins/AetherialDawn-NoPause.dll", include_bytes!("../bundled/Data/SKSE/Plugins/AetherialDawn-NoPause.dll")),
    ("Data/SKSE/Plugins/AetherialQuietPickups.dll", include_bytes!("../bundled/Data/SKSE/Plugins/AetherialQuietPickups.dll")),
];

/// Settings files: written once, then the player's (or staff's) edits stay.
const SETTINGS: &[(&str, &[u8])] = &[("Data/SKSE/Plugins/AetherialSkillTree.test.json", include_bytes!("../bundled/Data/SKSE/Plugins/AetherialSkillTree.test.json")), ("Data/SKSE/Plugins/AetherialQuietPickups.ini", include_bytes!("../bundled/Data/SKSE/Plugins/AetherialQuietPickups.ini")), ("Data/SKSE/Plugins/AetherialDawn-NoPause.ini", include_bytes!("../bundled/Data/SKSE/Plugins/AetherialDawn-NoPause.ini"))];

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
        let np = std::fs::read_to_string(d.path().join("Data/SKSE/Plugins/AetherialDawn-NoPause.ini")).unwrap();
        assert!(np.contains("Book=0"));
        assert_eq!(std::fs::read(&ini).unwrap(), b"edited");
    }

}
