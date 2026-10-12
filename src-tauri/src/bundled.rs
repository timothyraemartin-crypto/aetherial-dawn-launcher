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
const SETTINGS: &[(&str, &[u8])] = &[("Data/SKSE/Plugins/AetherialQuietPickups.ini", include_bytes!("../bundled/Data/SKSE/Plugins/AetherialQuietPickups.ini")), ("Data/SKSE/Plugins/AetherialDawn-NoPause.ini", include_bytes!("../bundled/Data/SKSE/Plugins/AetherialDawn-NoPause.ini"))];

/// Our own mods that are switched off for now: the launcher takes its own copies back out.
const RETIRED: &[&str] = &["Data/SKSE/Plugins/AetherialFreezeMenus.dll"];

/// Sample skill data that 0.1.120-0.1.136 wrote as a settings file. The skill tree reads it when the
/// server state is missing, so players would see skills they never learned. Removed again, but only
/// while it is still byte-for-byte what we wrote. Kill switch: an empty `AetherialSkillTree.test.keep`
/// file next to it keeps the file.
const SAMPLE_SKILLS: (&str, &[u8]) = (
    "Data/SKSE/Plugins/AetherialSkillTree.test.json",
    b"{\"v\":1,\"skills\":{\"blacksmithing\":{\"r\":2,\"x\":60,\"n\":100,\"pts\":1,\"p\":[\"smith.iron\",\"smith.steel\",\"smith.fletcher\"]},\"archery\":{\"r\":1,\"x\":10,\"n\":100,\"pts\":0,\"p\":[]},\"fishing\":{\"r\":3,\"x\":300,\"n\":400,\"pts\":2,\"p\":[]}}}\n",
);

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
    retire_sample_skills(game_dir);
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

fn retire_sample_skills(game_dir: &Path) {
    let (rel, bytes) = SAMPLE_SKILLS;
    let path = game_dir.join(rel);
    if game_dir.join("Data/SKSE/Plugins/AetherialSkillTree.test.keep").exists() || !same(&path, bytes) {
        return;
    }
    match std::fs::remove_file(&path) {
        Ok(()) => crate::log::line("Removed the sample skill data the skill tree would show instead of your real skills"),
        Err(e) => crate::log::line(&format!("Couldn't remove the sample skill data {rel}: {e}")),
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
        let np = std::fs::read_to_string(d.path().join("Data/SKSE/Plugins/AetherialDawn-NoPause.ini")).unwrap();
        assert!(np.contains("Book=0"));
        assert_eq!(std::fs::read(&ini).unwrap(), b"edited");
    }

    #[test]
    fn removes_only_the_sample_skill_data_we_wrote() {
        let d = tempfile::tempdir().unwrap();
        let plugins = d.path().join("Data/SKSE/Plugins");
        std::fs::create_dir_all(&plugins).unwrap();
        let f = plugins.join("AetherialSkillTree.test.json");
        std::fs::write(&f, super::SAMPLE_SKILLS.1).unwrap();
        super::deploy(d.path());
        assert!(!f.exists(), "our sample is removed");
        super::deploy(d.path());
        assert!(!f.exists(), "and not written back");
        std::fs::write(&f, b"{\"v\":1,\"skills\":{}}").unwrap();
        super::deploy(d.path());
        assert!(f.exists(), "an edited file stays");
        std::fs::write(&f, super::SAMPLE_SKILLS.1).unwrap();
        std::fs::write(plugins.join("AetherialSkillTree.test.keep"), b"").unwrap();
        super::deploy(d.path());
        assert!(f.exists(), "the keep file switches the removal off");
    }
}
