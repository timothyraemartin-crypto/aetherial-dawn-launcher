//! Finds plugins switched on in the player's load order (plugins.txt) that
//! aren't the base game, Creation Club or Aetherial Dawn's own, and broken
//! plugin files. In the first live test a 59-byte SkyUI_SE.esp stub left by an
//! old setup was the only active plugin, and Skyrim crashed while loading data.

use std::path::Path;

use crate::manifest::Manifest;
use crate::Result;

const BASE: [&str; 6] = ["skyrim.esm", "update.esm", "dawnguard.esm", "hearthfires.esm", "dragonborn.esm", "_resourcepack.esl"];

#[derive(Debug, Clone, PartialEq)]
pub struct Extra {
    pub name: String,
    /// Why the file itself is unusable, when it is.
    pub broken: Option<String>,
}

impl Extra {
    pub fn describe(&self) -> String {
        match &self.broken {
            Some(why) => format!("{} (switched on in your load order; the file is broken: {why})", self.name),
            None => format!("{} (switched on in your load order)", self.name),
        }
    }
}

/// Checks a plugin file's header. None means it looks like a real plugin.
pub fn broken(path: &Path) -> Option<String> {
    use std::io::Read;
    let Ok(mut f) = std::fs::File::open(path) else { return Some("missing from Data".into()) };
    let len = f.metadata().map(|m| m.len() as usize).unwrap_or(0);
    // Only the header: masters can be hundreds of MB.
    let mut bytes = vec![0u8; 64.min(len)];
    if f.read_exact(&mut bytes).is_err() || bytes.len() < 24 || &bytes[..4] != b"TES4" {
        return Some("not a Skyrim plugin".into());
    }
    let size = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
    let hedr = &bytes[24..];
    if hedr.len() < 10 || &hedr[..4] != b"HEDR" {
        return Some("header has no HEDR".into());
    }
    let version = f32::from_le_bytes(hedr[6..10].try_into().unwrap());
    if !(0.9..=2.0).contains(&version) {
        return Some(format!("header version {version:.2}"));
    }
    if len <= 24 + size {
        return Some(format!("only {len} bytes, no records"));
    }
    None
}

/// The HEDR version and the TES4 record's form version of a plugin.
pub fn header(path: &Path) -> Option<(f32, u16)> {
    use std::io::Read;
    let mut bytes = [0u8; 34];
    std::fs::File::open(path).ok()?.read_exact(&mut bytes).ok()?;
    if &bytes[..4] != b"TES4" || &bytes[24..28] != b"HEDR" {
        return None;
    }
    let form = u16::from_le_bytes([bytes[20], bytes[21]]);
    Some((f32::from_le_bytes(bytes[30..34].try_into().ok()?), form))
}

/// Plugins in Data made for a newer Skyrim than the game's own masters: their
/// header or form version is higher than any of the base masters'. Steam
/// updates Creation Club downloads separately from the game, so after a
/// downgrade they can be left on the newer build, and the older game can't
/// load them. Returns the plugin names with their versions.
pub fn too_new(game_dir: &Path) -> Vec<(String, String)> {
    let data = game_dir.join("Data");
    let base: Vec<(f32, u16)> = BASE[..5].iter().filter_map(|n| find(&data, n)).filter_map(|p| header(&p)).collect();
    if base.len() < 5 {
        return Vec::new();
    }
    let max_v = base.iter().map(|b| b.0).fold(0.0f32, f32::max);
    let max_f = base.iter().map(|b| b.1).max().unwrap_or(0);
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(&data) else { return out };
    for e in rd.flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        let l = n.to_ascii_lowercase();
        if !(l.ends_with(".esm") || l.ends_with(".esl") || l.ends_with(".esp")) || BASE[..5].contains(&l.as_str()) {
            continue;
        }
        if let Some((v, f)) = header(&e.path()) {
            if v > max_v + 0.001 || f > max_f {
                out.push((n, format!("header {v:.2}, form {f}; the game's masters are {max_v:.2}, form {max_f}")));
            }
        }
    }
    out.sort();
    out
}

/// A file in `dir` by name, ignoring case.
fn find(dir: &Path, name: &str) -> Option<std::path::PathBuf> {
    std::fs::read_dir(dir).ok()?.flatten().find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(name)).map(|e| e.path())
}

/// A plugin and the archives that belong to it (`<name>.bsa`,
/// `<name> - Textures.bsa`), as paths relative to the game folder.
pub fn with_archives(game_dir: &Path, plugin: &str) -> Vec<String> {
    let stem = plugin.rsplit_once('.').map(|(s, _)| s).unwrap_or(plugin).to_ascii_lowercase();
    let mut out = vec![format!("Data/{plugin}")];
    if let Ok(rd) = std::fs::read_dir(game_dir.join("Data")) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            let l = n.to_ascii_lowercase();
            if l == format!("{stem}.bsa") || l == format!("{stem} - textures.bsa") {
                out.push(format!("Data/{n}"));
            }
        }
    }
    out
}

fn allowed(game_dir: &Path, manifest: &Manifest) -> Vec<String> {
    let mut ok: Vec<String> = BASE.iter().map(|s| s.to_string()).collect();
    // A required mod (Skyrim Souls RE's dependency), from Nexus Mods.
    ok.push(crate::requirements::USSEP_PLUGIN.to_ascii_lowercase());
    // Plugins of other mods on the server's list.
    ok.extend(crate::allowlist::kept_plugins(&crate::allowlist::keep_set(game_dir)));
    for ccc in [game_dir.join("Data").join("Skyrim.ccc"), game_dir.join("Skyrim.ccc")] {
        if let Ok(t) = std::fs::read_to_string(ccc) {
            ok.extend(t.lines().map(|l| l.trim().to_ascii_lowercase()).filter(|l| !l.is_empty()));
        }
    }
    for f in &manifest.files {
        if let Some(n) = f.path.strip_prefix("Data/") {
            if !n.contains('/') {
                ok.push(n.to_ascii_lowercase());
            }
        }
    }
    ok
}

/// Active plugins that shouldn't load with Aetherial Dawn.
pub fn extras(game_dir: &Path, plugins_txt: &Path, manifest: &Manifest) -> Vec<Extra> {
    let Ok(text) = std::fs::read_to_string(plugins_txt) else { return Vec::new() };
    let ok = allowed(game_dir, manifest);
    text.lines()
        .filter_map(|l| l.trim().strip_prefix('*'))
        .map(str::trim)
        .filter(|n| !n.is_empty() && !ok.contains(&n.to_ascii_lowercase()))
        .map(|n| Extra { name: n.to_string(), broken: broken(&game_dir.join("Data").join(n)) })
        .collect()
}

/// Switches plugins off in plugins.txt (drops the `*`), keeping the file and
/// its place in the list. A copy of the old file is kept next to it.
pub fn switch_off(plugins_txt: &Path, names: &[String]) -> Result<()> {
    let text = std::fs::read_to_string(plugins_txt)?;
    std::fs::write(plugins_txt.with_extension("txt.aetherial-dawn-backup"), &text)?;
    let lower: Vec<String> = names.iter().map(|n| n.to_ascii_lowercase()).collect();
    let mut out: Vec<String> = text
        .lines()
        .map(|l| match l.trim().strip_prefix('*') {
            Some(n) if lower.contains(&n.trim().to_ascii_lowercase()) => n.trim().to_string(),
            _ => l.to_string(),
        })
        .collect();
    if text.ends_with('\n') {
        out.push(String::new());
    }
    std::fs::write(plugins_txt, out.join(if text.contains("\r\n") { "\r\n" } else { "\n" }))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plugin(version: f32, records: bool) -> Vec<u8> {
        plugin_form(version, 44, records)
    }

    fn plugin_form(version: f32, form: u16, records: bool) -> Vec<u8> {
        let mut sub = b"HEDR".to_vec();
        sub.extend(12u16.to_le_bytes());
        sub.extend(version.to_le_bytes());
        sub.extend([0u8; 8]);
        let mut b = b"TES4".to_vec();
        b.extend((sub.len() as u32).to_le_bytes());
        b.extend([0u8; 12]);
        b.extend(form.to_le_bytes());
        b.extend([0u8; 2]);
        b.extend(sub);
        if records {
            b.extend(b"GRUP");
            b.extend([0u8; 20]);
        }
        b
    }

    #[test]
    fn finds_and_switches_off_extras() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("Data");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("SkyUI_SE.esp"), plugin(0.0, false)).unwrap();
        std::fs::write(data.join("Good.esp"), plugin(1.71, true)).unwrap();
        std::fs::write(data.join("Skyrim.ccc"), "ccBGSSSE001-Fish.esm\n").unwrap();
        let txt = tmp.path().join("plugins.txt");
        std::fs::write(&txt, "# Vortex\r\n*SkyUI_SE.esp\r\n*ccBGSSSE001-Fish.esm\r\n*Good.esp\r\nOff.esp\r\n").unwrap();
        let m: Manifest = serde_json::from_value(serde_json::json!({
            "schema": 1, "build": "b", "server": {"name": "t", "ip": "1.2.3.4", "port": 7777}, "files": []
        }))
        .unwrap();
        let ex = extras(tmp.path(), &txt, &m);
        assert_eq!(ex.len(), 2);
        assert_eq!(ex[0].name, "SkyUI_SE.esp");
        assert!(ex[0].broken.as_deref().unwrap().contains("version 0.00"));
        assert!(ex[1].broken.is_none());
        switch_off(&txt, &ex.iter().map(|e| e.name.clone()).collect::<Vec<_>>()).unwrap();
        let after = std::fs::read_to_string(&txt).unwrap();
        assert_eq!(after, "# Vortex\r\nSkyUI_SE.esp\r\n*ccBGSSSE001-Fish.esm\r\nGood.esp\r\nOff.esp\r\n");
        assert!(extras(tmp.path(), &txt, &m).is_empty());
    }

    #[test]
    fn finds_plugins_newer_than_the_game() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("Data");
        std::fs::create_dir_all(&data).unwrap();
        for m in ["Skyrim.esm", "Update.esm", "Dawnguard.esm", "HearthFires.esm", "Dragonborn.esm"] {
            std::fs::write(data.join(m), plugin_form(1.71, 44, true)).unwrap();
        }
        std::fs::write(data.join("ccOld.esl"), plugin_form(1.71, 44, true)).unwrap();
        std::fs::write(data.join("ccNew.esl"), plugin_form(1.72, 44, true)).unwrap();
        std::fs::write(data.join("ccNewForm.esm"), plugin_form(1.71, 45, true)).unwrap();
        std::fs::write(data.join("ccNew.bsa"), b"x").unwrap();
        std::fs::write(data.join("ccNew - Textures.bsa"), b"x").unwrap();
        let n: Vec<String> = too_new(tmp.path()).into_iter().map(|(n, _)| n).collect();
        assert_eq!(n, ["ccNew.esl", "ccNewForm.esm"]);
        let mut a = with_archives(tmp.path(), "ccNew.esl");
        a.sort();
        assert_eq!(a, ["Data/ccNew - Textures.bsa", "Data/ccNew.bsa", "Data/ccNew.esl"]);
    }
}
