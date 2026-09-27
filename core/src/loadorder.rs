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

/// Plugins of required SKSE mods, switched on when they're in Data
/// (Timothy, 2026-09-26: SmoothCam, True Directional Movement, TrueHUD and
/// MCM Helper, which they need).
// TrueHUD ships TrueHUD.esl (seen on Timothy's PC); .esp kept for older builds.
pub const COMPANION_PLUGINS: [&str; 5] = ["MCMHelper.esp", "SmoothCam.esp", "TrueDirectionalMovement.esp", "TrueHUD.esl", "TrueHUD.esp"];

fn allowed(game_dir: &Path, manifest: &Manifest) -> Vec<String> {
    let mut ok: Vec<String> = BASE.iter().map(|s| s.to_string()).collect();
    ok.extend(COMPANION_PLUGINS.iter().map(|p| p.to_ascii_lowercase()));
    // A required mod, from Nexus Mods.
    ok.push(crate::requirements::USSEP_PLUGIN.to_ascii_lowercase());
    ok.push(crate::requirements::SKYUI_PLUGIN.to_ascii_lowercase());
    // Plugins of other mods on the server's list.
    ok.extend(crate::allowlist::kept_plugins(&crate::allowlist::keep_set(game_dir)));
    // Plugins whose names the client can't load run under a dash-named
    // copy (aliases.rs); that copy is allowed wherever its original is.
    let also: Vec<String> = ok.iter().filter(|n| !client_can_load_name(n)).map(|n| crate::aliases::alias_name(n).to_ascii_lowercase()).collect();
    ok.extend(also);
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
        .filter(|n| !n.is_empty())
        .map(|n| {
            let why = if client_can_load_name(n) { broken(&game_dir.join("Data").join(n)) } else { Some(BAD_NAME.to_string()) };
            Extra { name: n.to_string(), broken: why }
        })
        // Allowed plugins stay on unless they're broken, like the SkyUI stub
        // from the first live test, or have a name the client can't load.
        .filter(|e| !ok.contains(&e.name.to_ascii_lowercase()) || !client_can_load_name(&e.name) || (e.broken.is_some() && game_dir.join("Data").join(&e.name).is_file()))
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

/// Plugins of required and listed mods that are in Data and sound, which
/// must be switched on for the mod to work (SkyUI's menus live in its
/// archive, which only loads with its plugin).
pub fn wanted(game_dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = vec![crate::requirements::USSEP_PLUGIN.into(), crate::requirements::SKYUI_PLUGIN.into()];
    names.extend(COMPANION_PLUGINS.iter().map(|p| p.to_string()));
    let rec = crate::modlist::load_installed(game_dir);
    for m in crate::allowlist::listed(game_dir) {
        // Plugins the launcher installed for a listed mod whose checks name
        // no plugin (FOMOD installers pick them), when their masters are here.
        if let Some(r) = rec.mods.get(&m.id) {
            names.extend(crate::modlist::top_plugins(&r.files).into_iter().filter(|n| masters_present(game_dir, n)));
        }
        for c in &m.check {
            if let Some(n) = c.replace('\\', "/").strip_prefix("Data/") {
                let l = n.to_ascii_lowercase();
                if !n.contains('/') && (l.ends_with(".esp") || l.ends_with(".esm") || l.ends_with(".esl")) {
                    names.push(n.to_string());
                }
            }
        }
    }
    let mut seen = std::collections::HashSet::new();
    names
        .into_iter()
        .filter(|n| seen.insert(n.to_ascii_lowercase()))
        // A name the client can't load runs as its dash-named copy.
        .map(|n| if client_can_load_name(&n) { n } else { crate::aliases::alias_name(&n) })
        .filter(|n| {
            let p = game_dir.join("Data").join(n);
            p.is_file() && broken(&p).is_none()
        })
        .collect()
}

/// Wanted plugins (SkyUI, SmoothCam, ...) that plugins.txt lists switched
/// off, which the launcher leaves off. SmoothCam's settings page only shows
/// with SmoothCam.esp on (Timothy, 2026-09-26).
pub fn wanted_but_off(game_dir: &Path, plugins_txt: &Path) -> Vec<String> {
    let text = std::fs::read_to_string(plugins_txt).unwrap_or_default();
    let off: std::collections::HashSet<String> = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('*') && !l.starts_with('#')).map(|l| l.to_ascii_lowercase()).collect();
    wanted(game_dir).into_iter().filter(|n| off.contains(&n.to_ascii_lowercase())).collect()
}

/// Switches required plugins on in plugins.txt, also when a line lists them
/// switched off: they're the server's required mods (their menus, like
/// SmoothCam's settings page, only show with the plugin on; Vortex had
/// SmoothCam, TDM and TrueHUD off on Timothy's PC). Returns the ones changed;
/// the old file is kept next to it.
pub fn force_on(plugins_txt: &Path, names: &[String]) -> Result<Vec<String>> {
    let text = std::fs::read_to_string(plugins_txt).unwrap_or_default();
    let nl = if text.contains("\r\n") || text.is_empty() { "\r\n" } else { "\n" };
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let mut changed = Vec::new();
    for n in names {
        let l = n.to_ascii_lowercase();
        match lines.iter_mut().find(|x| x.trim().trim_start_matches('*').trim().to_ascii_lowercase() == l) {
            Some(x) if x.trim_start().starts_with('*') => {}
            Some(x) => {
                *x = format!("*{}", x.trim());
                changed.push(n.clone());
            }
            None => {
                lines.push(format!("*{n}"));
                changed.push(n.clone());
            }
        }
    }
    if changed.is_empty() {
        return Ok(changed);
    }
    if !text.is_empty() {
        std::fs::write(plugins_txt.with_extension("txt.aetherial-dawn-backup"), &text)?;
    } else if let Some(d) = plugins_txt.parent() {
        std::fs::create_dir_all(d)?;
    }
    lines.retain(|l| !l.is_empty());
    lines.push(String::new());
    std::fs::write(plugins_txt, lines.join(nl))?;
    Ok(changed)
}

/// Switches plugins on in plugins.txt by adding the line when it's missing
/// (the launcher's own installs). A plugin listed there switched off stays
/// off: the player or Vortex chose that. Returns the ones it added; the old
/// file is kept next to it.
pub fn switch_on(plugins_txt: &Path, names: &[String]) -> Result<Vec<String>> {
    let text = std::fs::read_to_string(plugins_txt).unwrap_or_default();
    let nl = if text.contains("\r\n") || text.is_empty() { "\r\n" } else { "\n" };
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let mut changed = Vec::new();
    for n in names {
        let l = n.to_ascii_lowercase();
        match lines.iter_mut().find(|x| x.trim().trim_start_matches('*').trim().to_ascii_lowercase() == l) {
            Some(_) => {}
            None => {
                lines.push(format!("*{n}"));
                changed.push(n.clone());
            }
        }
    }
    if changed.is_empty() {
        return Ok(changed);
    }
    if !text.is_empty() {
        std::fs::write(plugins_txt.with_extension("txt.aetherial-dawn-backup"), &text)?;
    } else if let Some(d) = plugins_txt.parent() {
        std::fs::create_dir_all(d)?;
    }
    lines.push(String::new());
    std::fs::write(plugins_txt, lines.join(nl))?;
    Ok(changed)
}

/// Puts the five base masters first in loadorder.txt, in their own order,
/// keeping everything else as it was (Vortex had the Unofficial Patch ahead
/// of Skyrim.esm on 2026-09-26). Returns whether it changed the file.
pub fn fix_order(loadorder_txt: &Path) -> Result<bool> {
    let Ok(text) = std::fs::read_to_string(loadorder_txt) else { return Ok(false) };
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
    std::fs::write(loadorder_txt.with_extension("txt.aetherial-dawn-backup"), &text)?;
    let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut out: Vec<&str> = comments;
    out.extend(want);
    out.push("");
    std::fs::write(loadorder_txt, out.join(nl))?;
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
    fn spots_a_wanted_plugin_switched_off() {
        let t = tempfile::tempdir().unwrap();
        let g = t.path();
        std::fs::create_dir_all(g.join("Data")).unwrap();
        std::fs::write(g.join("Data/SmoothCam.esp"), plugin(1.71, true)).unwrap();
        std::fs::write(g.join("Data/TrueHUD.esp"), plugin(1.71, true)).unwrap();
        let txt = g.join("plugins.txt");
        std::fs::write(&txt, "*TrueHUD.esp\r\nSmoothCam.esp\r\n").unwrap();
        assert_eq!(wanted_but_off(g, &txt), vec!["SmoothCam.esp".to_string()]);
        assert_eq!(force_on(&txt, &wanted(g)).unwrap(), vec!["SmoothCam.esp".to_string()]);
        assert_eq!(std::fs::read_to_string(&txt).unwrap(), "*TrueHUD.esp\r\n*SmoothCam.esp\r\n");
        assert!(wanted_but_off(g, &txt).is_empty());
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
    fn switches_on_skyui() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("Data");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("SkyUI_SE.esp"), plugin(1.7, true)).unwrap();
        assert_eq!(wanted(tmp.path()), ["SkyUI_SE.esp"]);
        let txt = tmp.path().join("plugins.txt");
        // Switched off in Vortex: stays off.
        std::fs::write(&txt, "# Vortex\r\nSkyUI_SE.esp\r\n*Good.esp\r\n").unwrap();
        assert!(switch_on(&txt, &wanted(tmp.path())).unwrap().is_empty());
        // Not listed at all (the launcher installed it): added, switched on.
        std::fs::write(&txt, "# Vortex\r\n*Good.esp\r\n").unwrap();
        assert_eq!(switch_on(&txt, &wanted(tmp.path())).unwrap(), ["SkyUI_SE.esp"]);
        assert_eq!(std::fs::read_to_string(&txt).unwrap(), "# Vortex\r\n*Good.esp\r\n*SkyUI_SE.esp\r\n");
        assert!(switch_on(&txt, &wanted(tmp.path())).unwrap().is_empty());
        std::fs::remove_file(&txt).unwrap();
        switch_on(&txt, &["SkyUI_SE.esp".into()]).unwrap();
        assert_eq!(std::fs::read_to_string(&txt).unwrap(), "*SkyUI_SE.esp\r\n");
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
