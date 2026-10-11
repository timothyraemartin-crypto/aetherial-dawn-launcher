//! Our own mods ship inside the launcher. On every Play they are written into
//! the Skyrim folder (when missing or different), and the paths come back so
//! the no-mods gate treats exactly these files as the launcher's own.

use std::path::Path;

/// Path relative to the game folder, and the file's bytes.
const FILES: &[(&str, &[u8])] = &[
    ("Data/SKSE/Plugins/AetherialDawn-NoPause.dll", include_bytes!("../bundled/Data/SKSE/Plugins/AetherialDawn-NoPause.dll")),
    ("Data/SKSE/Plugins/AetherialQuietPickups.dll", include_bytes!("../bundled/Data/SKSE/Plugins/AetherialQuietPickups.dll")),
    ("Data/SKSE/Plugins/AetherialFreezeMenus.dll", include_bytes!("../bundled/Data/SKSE/Plugins/AetherialFreezeMenus.dll")),
];

/// Settings files: written once, then the player's (or staff's) edits stay.
const SETTINGS: &[(&str, &[u8])] = &[("Data/SKSE/Plugins/AetherialQuietPickups.ini", include_bytes!("../bundled/Data/SKSE/Plugins/AetherialQuietPickups.ini"))];

fn same(path: &Path, bytes: &[u8]) -> bool {
    std::fs::read(path).is_ok_and(|b| b == bytes)
}

/// Writes our files into the game folder; returns every path that is ours.
/// A file that cannot be written is logged and still counted as ours.
pub fn deploy(game_dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
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

#[cfg(test)]
mod tests {
    #[test]
    fn deploys_every_file_and_keeps_edited_settings() {
        let d = tempfile::tempdir().unwrap();
        let ours = super::deploy(d.path());
        assert_eq!(ours.len(), super::FILES.len() + super::SETTINGS.len());
        let dll = d.path().join("Data/SKSE/Plugins/AetherialDawn-NoPause.dll");
        let ini = d.path().join("Data/SKSE/Plugins/AetherialQuietPickups.ini");
        std::fs::write(&dll, b"old").unwrap();
        std::fs::write(&ini, b"edited").unwrap();
        super::deploy(d.path());
        assert_ne!(std::fs::read(&dll).unwrap(), b"old");
        assert_eq!(std::fs::read(&ini).unwrap(), b"edited");
    }
}
