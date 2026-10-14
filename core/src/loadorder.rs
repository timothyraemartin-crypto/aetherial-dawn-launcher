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
            Some(why) if why == BAD_NAME => format!("{} (switched on in your load order; {why})", self.name),
            Some(why) => format!("{} (switched on in your load order; the file is broken: {why})", self.name),
            None => format!("{} (switched on in your load order)", self.name),
        }
    }
}

/// Whether the SkyMP client can handle this plugin name. Its load-order
/// check calls Skyrim Platform's getFileInfo for every plugin, which rejects
/// names with spaces ("'unofficial skyrim special edition patch.esp' is not
/// a valid argument for 'filename'", 2026-09-26) and stops the client's
/// update loop: a black screen after loading in. Base game and Creation Club
/// names only use letters, digits, '_', '-' and '.', so that's what passes.
pub fn client_can_load_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

pub const BAD_NAME: &str = "its name has spaces or other characters the SkyMP game client can't load, and the launcher couldn't make a copy under a name it accepts";

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
    // A header-only plugin is valid: MCM Helper's MCMHelper.esp is one, and
    // it was parked as broken on 2026-09-26. The SkyUI stub from the first
    // live test is still caught by its version-0 header above; a header cut
    // short is caught here.
    if len < 24 + size {
        return Some(format!("only {len} bytes, header cut short"));
    }
    None
}

/// The masters a plugin names in its header (MAST), or None when the file
/// isn't a readable plugin.
pub fn masters(path: &Path) -> Option<Vec<String>> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut head = [0u8; 24];
    f.read_exact(&mut head).ok()?;
    if &head[..4] != b"TES4" {
        return None;
    }
    let size = u32::from_le_bytes(head[4..8].try_into().ok()?) as usize;
    let mut body = vec![0u8; size.min(1 << 20)];
    f.read_exact(&mut body).ok()?;
    let mut out = Vec::new();
    let mut at = 0;
    while at + 6 <= body.len() {
        if &body[at..at + 4] == b"XXXX" {
            // An oversized subrecord (a big ONAM): the masters came first.
            break;
        }
        let len = u16::from_le_bytes([body[at + 4], body[at + 5]]) as usize;
        let data = body.get(at + 6..at + 6 + len)?;
        if &body[at..at + 4] == b"MAST" {
            out.push(String::from_utf8_lossy(data).trim_end_matches('\0').to_string());
        }
        at += 6 + len;
    }
    Some(out)
}

/// Whether every master a plugin in Data needs is in Data too. A patch for
/// a mod the player doesn't have must stay off, or the game won't start.
pub fn masters_present(game_dir: &Path, plugin: &str) -> bool {
    let data = game_dir.join("Data");
    match masters(&data.join(plugin)) {
        Some(ms) => ms.iter().all(|m| find(&data, m).is_some()),
        None => false,
    }
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

/// Plugin names the launcher leaves switched on: the base game, Creation
/// Club content and the plugins the server's file list ships.
fn allowed(game_dir: &Path, manifest: &Manifest) -> Vec<String> {
    let mut ok: Vec<String> = BASE.iter().map(|s| s.to_string()).collect();
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

/// Active plugins the SkyMP client can't load: a name with spaces (it stops
/// the client's load-order check), or a plugin file that is broken. Other
/// active plugins are the player's own business.
pub fn extras(game_dir: &Path, plugins_txt: &Path, manifest: &Manifest) -> Vec<Extra> {
    let Ok(text) = read_text(plugins_txt) else { return Vec::new() };
    let ok = allowed(game_dir, manifest);
    text.lines()
        .filter_map(|l| l.trim().strip_prefix('*'))
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(|n| {
            let why = if client_can_load_name(n) { broken(&game_dir.join("Data").join(n)) } else { Some(BAD_NAME.to_string()) };
            Extra { name: n.to_string(), broken: why }
        })
        // The server's own and base plugins stay on unless they are broken.
        .filter(|e| {
            let broken_file = e.broken.is_some() && game_dir.join("Data").join(&e.name).is_file();
            if ok.contains(&e.name.to_ascii_lowercase()) { broken_file } else { !client_can_load_name(&e.name) || broken_file }
        })
        .collect()
}

/// A list file (plugins.txt, loadorder.txt) as it is on disk. Skyrim writes
/// them in Windows-1252, so a name like "Café.esp" isn't UTF-8. Such a file
/// is held as Latin-1 text, one char per byte, and written back the same way,
/// so what the launcher doesn't change stays byte for byte.
pub struct ListFile {
    pub bytes: Vec<u8>,
    pub text: String,
    ansi: bool,
}

/// Decodes list-file bytes: UTF-8 when it is, else one char per byte.
fn decode(bytes: &[u8]) -> (String, bool) {
    match std::str::from_utf8(bytes) {
        Ok(t) => (t.to_string(), false),
        Err(_) => (bytes.iter().map(|&b| b as char).collect(), true),
    }
}

/// The text of a list file, for reading only. Unlike `read_to_string` it
/// doesn't fail on a non-UTF-8 name.
pub fn read_text(path: &Path) -> std::io::Result<String> {
    Ok(decode(&std::fs::read(path)?).0)
}

impl ListFile {
    /// Reads the file. `None` when it doesn't exist; any other failure (a lock,
    /// no permission) is an error, never an empty list.
    pub fn read(path: &Path) -> std::io::Result<Option<ListFile>> {
        match std::fs::read(path) {
            Ok(bytes) => {
                let (text, ansi) = decode(&bytes);
                Ok(Some(ListFile { bytes, text, ansi }))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn encode(&self, new: &str) -> std::io::Result<Vec<u8>> {
        if !self.ansi {
            return Ok(new.as_bytes().to_vec());
        }
        new.chars()
            .map(|c| u8::try_from(c as u32).map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("'{c}' can't be written to an ANSI list file"))))
            .collect()
    }
}

/// Writes `new` to a list file. The old contents are backed up first (see
/// `keep_backup`), and the file is replaced in one step, so a crash or a
/// failed backup never leaves a short or empty list.
pub fn write_list(path: &Path, old: Option<&ListFile>, new: &str) -> std::io::Result<()> {
    let bytes = match old {
        Some(f) => {
            keep_backup(path, &f.bytes)?;
            f.encode(new)?
        }
        None => new.as_bytes().to_vec(),
    };
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    atomic_write(path, &bytes)
}

/// Writes `bytes` to a temp file next to `path`, then renames it over `path`.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    crate::atomicfile::write(path, bytes)
}

/// Keeps `old`, the list as it was before the launcher changed it, beside it.
/// `<name>.txt.aetherial-dawn-backup` is the first one only (as the ini
/// backups are), so later changes never replace the player's own load order
/// with one the launcher wrote; an empty one, left by an older version that
/// had read a list as empty, doesn't count. `<name>.txt.aetherial-dawn-previous`
/// is the list just before the latest change. Nothing is kept for an empty
/// list: there is nothing to lose.
pub fn keep_backup(txt: &Path, old: &[u8]) -> std::io::Result<()> {
    if old.is_empty() {
        return Ok(());
    }
    let backup = txt.with_extension("txt.aetherial-dawn-backup");
    if std::fs::metadata(&backup).map_or(true, |m| m.len() == 0) {
        atomic_write(&backup, old)?;
    }
    atomic_write(&txt.with_extension("txt.aetherial-dawn-previous"), old)
}

/// Switches plugins off in plugins.txt (drops the `*`), keeping the file and
/// its place in the list. A copy of the old file is kept next to it.
pub fn switch_off(plugins_txt: &Path, names: &[String]) -> Result<()> {
    let Some(file) = ListFile::read(plugins_txt)? else { return Err(std::io::Error::from(std::io::ErrorKind::NotFound).into()) };
    let text = file.text.as_str();
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
    write_list(plugins_txt, Some(&file), &out.join(if text.contains("\r\n") { "\r\n" } else { "\n" }))?;
    Ok(())
}

/// Puts the five base masters first in loadorder.txt, in their own order,
/// keeping everything else as it was (Vortex had the Unofficial Patch ahead
/// of Skyrim.esm on 2026-09-26). Returns whether it changed the file.
pub fn fix_order(loadorder_txt: &Path) -> Result<bool> {
    let Some(file) = ListFile::read(loadorder_txt)? else { return Ok(false) };
    let text = file.text.as_str();
    let masters = &BASE[..5];
    let lines: Vec<&str> = text.lines().collect();
    let is_master = |l: &str| masters.contains(&l.trim().to_ascii_lowercase().as_str());
    let (comments, rest): (Vec<&str>, Vec<&str>) = lines.iter().partition(|l| l.trim_start().starts_with('#'));
    let mut first: Vec<&str> = rest.iter().copied().filter(|l| is_master(l)).collect();
    first.sort_by_key(|l| masters.iter().position(|m| *m == l.trim().to_ascii_lowercase()));
    let entries: Vec<&str> = rest.iter().copied().filter(|l| !l.trim().is_empty()).collect();
    let mut want: Vec<&str> = first.clone();
    want.extend(entries.iter().copied().filter(|l| !is_master(l)));
    if want == entries {
        return Ok(false);
    }
    let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut out: Vec<&str> = comments;
    out.extend(want);
    out.push("");
    write_list(loadorder_txt, Some(&file), &out.join(nl))?;
    Ok(true)
}

/// A minimal plugin file for tests elsewhere in the crate.
#[cfg(test)]
pub fn test_plugin(version: f32, records: bool) -> Vec<u8> {
    tests::plugin(version, records)
}

#[cfg(test)]
pub mod tests {
    use super::*;

    #[test]
    fn the_backup_keeps_the_players_own_list() {
        let t = tempfile::tempdir().unwrap();
        let txt = t.path().join("plugins.txt");
        std::fs::write(&txt, "*Mine.esp\n*Other.esp\n").unwrap();
        switch_off(&txt, &["Mine.esp".into()]).unwrap();
        switch_off(&txt, &["Other.esp".into()]).unwrap();
        switch_off(&txt, &["Mine.esp".into()]).unwrap();
        // Three changes later, the backup is still the list before the first.
        assert_eq!(std::fs::read_to_string(t.path().join("plugins.txt.aetherial-dawn-backup")).unwrap(), "*Mine.esp\n*Other.esp\n");
    }

    // "Café.esp" in Windows-1252, as Skyrim writes plugins.txt.
    const ANSI: &[u8] = b"# Vortex\r\n*Caf\xE9.esp\r\n*Other.esp\r\n";

    #[test]
    fn switch_off_keeps_an_ansi_plugins_txt() {
        let t = tempfile::tempdir().unwrap();
        let txt = t.path().join("plugins.txt");
        std::fs::write(&txt, ANSI).unwrap();
        switch_off(&txt, &["Other.esp".into()]).unwrap();
        assert_eq!(std::fs::read(&txt).unwrap(), b"# Vortex\r\n*Caf\xE9.esp\r\nOther.esp\r\n");
        assert_eq!(std::fs::read(t.path().join("plugins.txt.aetherial-dawn-backup")).unwrap(), ANSI);
    }

    #[test]
    fn an_unreadable_plugins_txt_is_an_error_and_is_left_alone() {
        let t = tempfile::tempdir().unwrap();
        // A directory stands in for a file that can't be read (locked).
        let txt = t.path().join("plugins.txt");
        std::fs::create_dir(&txt).unwrap();
        assert!(switch_off(&txt, &["A.esp".into()]).is_err());
        assert!(txt.is_dir());
        assert!(!t.path().join("plugins.txt.aetherial-dawn-backup").exists());
    }

    #[test]
    fn an_empty_first_backup_is_replaced_by_a_real_one() {
        let t = tempfile::tempdir().unwrap();
        let txt = t.path().join("plugins.txt");
        std::fs::write(&txt, "*Mine.esp\n").unwrap();
        std::fs::write(t.path().join("plugins.txt.aetherial-dawn-backup"), b"").unwrap();
        switch_off(&txt, &["Mine.esp".into()]).unwrap();
        assert_eq!(std::fs::read_to_string(t.path().join("plugins.txt.aetherial-dawn-backup")).unwrap(), "*Mine.esp\n");
    }

    #[test]
    fn every_rewrite_also_keeps_the_list_just_before_it() {
        let t = tempfile::tempdir().unwrap();
        let txt = t.path().join("plugins.txt");
        std::fs::write(&txt, "*Mine.esp\n").unwrap();
        switch_off(&txt, &["Mine.esp".into()]).unwrap();
        switch_off(&txt, &["Mine.esp".into()]).unwrap();
        assert_eq!(std::fs::read_to_string(t.path().join("plugins.txt.aetherial-dawn-previous")).unwrap(), "Mine.esp\n");
    }

    #[test]
    fn reads_masters_and_checks_they_are_here() {
        let mut sub = b"HEDR".to_vec();
        sub.extend(12u16.to_le_bytes());
        sub.extend(1.7f32.to_le_bytes());
        sub.extend([0u8; 8]);
        for m in ["Skyrim.esm", "True Storms.esp"] {
            sub.extend(b"MAST");
            sub.extend(((m.len() + 1) as u16).to_le_bytes());
            sub.extend(m.as_bytes());
            sub.push(0);
            sub.extend(b"DATA");
            sub.extend(8u16.to_le_bytes());
            sub.extend([0u8; 8]);
        }
        let mut b = b"TES4".to_vec();
        b.extend((sub.len() as u32).to_le_bytes());
        b.extend([0u8; 16]);
        b.extend(sub);
        let t = tempfile::tempdir().unwrap();
        let data = t.path().join("Data");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("patch.esp"), &b).unwrap();
        std::fs::write(data.join("skyrim.esm"), b"x").unwrap();
        assert_eq!(masters(&data.join("patch.esp")).unwrap(), ["Skyrim.esm", "True Storms.esp"]);
        assert!(!masters_present(t.path(), "patch.esp"));
        std::fs::write(data.join("True Storms.esp"), b"x").unwrap();
        assert!(masters_present(t.path(), "patch.esp"));
    }

    pub fn plugin(version: f32, records: bool) -> Vec<u8> {
        plugin_form(version, 44, records)
    }

    /// A header-only plugin (version 1.71) naming `masters`.
    pub fn plugin_with_masters(masters: &[&str]) -> Vec<u8> {
        let mut sub = b"HEDR".to_vec();
        sub.extend(12u16.to_le_bytes());
        sub.extend(1.71f32.to_le_bytes());
        sub.extend([0u8; 8]);
        for m in masters {
            sub.extend(b"MAST");
            sub.extend(((m.len() + 1) as u16).to_le_bytes());
            sub.extend(m.as_bytes());
            sub.push(0);
        }
        let mut b = b"TES4".to_vec();
        b.extend((sub.len() as u32).to_le_bytes());
        b.extend([0u8; 12]);
        b.extend(44u16.to_le_bytes());
        b.extend([0u8; 2]);
        b.extend(sub);
        b
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
        // Good.esp is the player's own, loadable plugin: left alone.
        assert_eq!(ex.len(), 1);
        assert_eq!(ex[0].name, "SkyUI_SE.esp");
        assert!(ex[0].broken.as_deref().unwrap().contains("version 0.00"));
        switch_off(&txt, &ex.iter().map(|e| e.name.clone()).collect::<Vec<_>>()).unwrap();
        let after = std::fs::read_to_string(&txt).unwrap();
        assert_eq!(after, "# Vortex\r\nSkyUI_SE.esp\r\n*ccBGSSSE001-Fish.esm\r\n*Good.esp\r\nOff.esp\r\n");
        assert!(extras(tmp.path(), &txt, &m).is_empty());
    }

    #[test]
    fn puts_the_masters_first() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("loadorder.txt");
        std::fs::write(&p, "# Vortex\r\nunofficial skyrim special edition patch.esp\r\nSkyrim.esm\r\nUpdate.esm\r\nDawnguard.esm\r\nHearthFires.esm\r\nDragonborn.esm\r\nSkyUI_SE.esp\r\n").unwrap();
        assert!(fix_order(&p).unwrap());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "# Vortex\r\nSkyrim.esm\r\nUpdate.esm\r\nDawnguard.esm\r\nHearthFires.esm\r\nDragonborn.esm\r\nunofficial skyrim special edition patch.esp\r\nSkyUI_SE.esp\r\n");
        assert!(!fix_order(&p).unwrap());
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
