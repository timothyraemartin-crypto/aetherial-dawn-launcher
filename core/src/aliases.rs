//! Plugins whose file names the SkyMP client can't load (Timothy,
//! 2026-09-26: "we need to just correct it"). The client's load-order check
//! calls Skyrim Platform's getFileInfo for every plugin, which rejects names
//! with spaces, so "Unofficial Skyrim Special Edition Patch.esp" left a black
//! screen. Before Play, each such plugin gets a hard link (a copy when linking
//! fails) under a dash-joined name, "Unofficial-Skyrim-Special-Edition-
//! Patch.esp", with its archives, string files and ini, and the load order
//! names the copy where the original was. Nothing Vortex deployed is renamed
//! or moved; links whose original is gone are removed.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::loadorder::client_can_load_name;

/// "Unofficial Skyrim Special Edition Patch.esp" ->
/// "Unofficial-Skyrim-Special-Edition-Patch.esp": every character the client
/// rejects becomes a dash, with runs of dashes joined.
pub fn alias_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        let c = if c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.') { c } else { '-' };
        if c == '-' && out.ends_with('-') {
            continue;
        }
        out.push(c);
    }
    // No dash right before the extension or at the start.
    out = out.replace("-.", ".");
    out.trim_start_matches('-').to_string()
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub struct Alias {
    /// Game-relative original, "Data/Unofficial Skyrim Special Edition Patch.esp".
    pub from: String,
    /// Game-relative link, "Data/Unofficial-Skyrim-Special-Edition-Patch.esp".
    pub to: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Record {
    #[serde(default)]
    links: Vec<Alias>,
}

fn record_path(game_dir: &Path) -> PathBuf {
    game_dir.join(crate::modlist::MODS_DIR).join("aliases.json")
}

fn load(game_dir: &Path) -> Record {
    std::fs::read(record_path(game_dir)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save(game_dir: &Path, r: &Record) {
    let p = record_path(game_dir);
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if let Ok(b) = serde_json::to_vec_pretty(r) {
        let _ = std::fs::write(p, b);
    }
}

/// The name a plugin in Data runs under at Play: the launcher's link or
/// rewritten copy when it made one, else its dashed alias when the client
/// can't load the name, else the name itself.
pub fn run_as(game_dir: &Path, plugin: &str) -> String {
    let from = format!("Data/{plugin}");
    if let Some(l) = load(game_dir).links.iter().find(|l| l.from.eq_ignore_ascii_case(&from) && l.to.strip_prefix("Data/").is_some_and(is_plugin)) {
        return l.to.trim_start_matches("Data/").to_string();
    }
    if client_can_load_name(plugin) { plugin.to_string() } else { alias_name(plugin) }
}

/// The links the launcher made, for tidying (never set aside on their own).
pub fn links(game_dir: &Path) -> Vec<Alias> {
    load(game_dir).links
}

fn is_plugin(n: &str) -> bool {
    let l = n.to_ascii_lowercase();
    l.ends_with(".esp") || l.ends_with(".esm") || l.ends_with(".esl")
}

fn stem(n: &str) -> &str {
    n.rsplit_once('.').map(|(s, _)| s).unwrap_or(n)
}

/// Same file? Hard links share size and modified time; a copy is refreshed
/// when either differs.
fn in_step(a: &Path, b: &Path) -> bool {
    match (std::fs::metadata(a), std::fs::metadata(b)) {
        (Ok(x), Ok(y)) => x.len() == y.len() && x.modified().ok() == y.modified().ok(),
        _ => false,
    }
}

fn link(from: &Path, to: &Path) -> std::io::Result<()> {
    if to.exists() {
        if in_step(from, to) {
            return Ok(());
        }
        std::fs::remove_file(to)?;
    }
    if std::fs::hard_link(from, to).is_ok() {
        return Ok(());
    }
    std::fs::copy(from, to)?;
    // Keep the time so in_step matches next time.
    if let Ok(t) = std::fs::metadata(from).and_then(|m| m.modified()) {
        let _ = std::fs::File::options().write(true).open(to).and_then(|f| f.set_modified(t));
    }
    Ok(())
}

/// The plugin with each master the client can't load renamed to its dashed
/// alias (Obsidian CS.esp masters "Obsidian Weathers.esp", which only runs
/// as Obsidian-Weathers.esp: a plain link would load with a master that
/// isn't active, and the game crashes at start). None when no master needs
/// it or the header can't be read.
pub fn with_aliased_masters(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.len() < 24 || &bytes[..4] != b"TES4" {
        return None;
    }
    let size = u32::from_le_bytes(bytes[4..8].try_into().ok()?) as usize;
    let body = bytes.get(24..24 + size)?;
    let mut out_body = Vec::with_capacity(body.len() + 64);
    let mut changed = false;
    let mut at = 0;
    while at + 6 <= body.len() {
        let kind = &body[at..at + 4];
        if kind == b"XXXX" {
            // An oversized subrecord follows: keep the rest as it is.
            out_body.extend_from_slice(&body[at..]);
            at = body.len();
            break;
        }
        let len = u16::from_le_bytes([body[at + 4], body[at + 5]]) as usize;
        let data = body.get(at + 6..at + 6 + len)?;
        let name = String::from_utf8_lossy(data).trim_end_matches('\0').to_string();
        if kind == b"MAST" && !client_can_load_name(&name) {
            let mut new = alias_name(&name).into_bytes();
            new.push(0);
            out_body.extend_from_slice(b"MAST");
            out_body.extend_from_slice(&u16::try_from(new.len()).ok()?.to_le_bytes());
            out_body.extend_from_slice(&new);
            changed = true;
        } else {
            out_body.extend_from_slice(&body[at..at + 6 + len]);
        }
        at += 6 + len;
    }
    if !changed || at != body.len() {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() + 64);
    out.extend_from_slice(&bytes[..4]);
    out.extend_from_slice(&u32::try_from(out_body.len()).ok()?.to_le_bytes());
    out.extend_from_slice(&bytes[8..24]);
    out.extend_from_slice(&out_body);
    out.extend_from_slice(&bytes[24 + size..]);
    Some(out)
}

/// The plugin's header, enough to read its masters.
fn head(path: &Path) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut h = [0u8; 24];
    f.read_exact(&mut h).ok()?;
    if &h[..4] != b"TES4" {
        return None;
    }
    let size = u32::from_le_bytes(h[4..8].try_into().ok()?) as usize;
    let mut b = h.to_vec();
    b.resize(24 + size.min(1 << 20), 0);
    f.read_exact(&mut b[24..]).ok()?;
    Some(b)
}

/// Whether a plugin names a master the client can't load.
fn needs_rewrite(path: &Path) -> bool {
    head(path).is_some_and(|h| with_aliased_masters(&h).is_some())
}

/// Writes the rewritten copy, keeping the original's modified time so it's
/// only rewritten when the original changes.
fn rewrite(from: &Path, to: &Path) -> std::io::Result<()> {
    let t = std::fs::metadata(from)?.modified()?;
    // Up to date only when it's already a rewritten copy: a dashed alias
    // made by 0.1.49-0.1.67 is a hard link with the same time and the old
    // master names.
    if std::fs::metadata(to).and_then(|m| m.modified()).ok() == Some(t) && !needs_rewrite(to) {
        return Ok(());
    }
    let bytes = std::fs::read(from)?;
    let new = with_aliased_masters(&bytes).ok_or_else(|| std::io::Error::other("the plugin's header can't be rewritten"))?;
    // Remove the old name first and write a new file: writing through a
    // hard link would change the original (Vortex's copy) too.
    if to.exists() {
        std::fs::remove_file(to)?;
    }
    std::fs::write(to, new)?;
    std::fs::File::options().write(true).open(to)?.set_modified(t)?;
    Ok(())
}

/// The name a plugin runs under: its dashed alias, or "<name>-AD.esp" for
/// a loadable name whose masters need rewriting.
fn run_name(plugin: &str, rewritten: bool) -> String {
    if !client_can_load_name(plugin) {
        return alias_name(plugin);
    }
    if rewritten {
        let (s, x) = plugin.rsplit_once('.').unwrap_or((plugin, "esp"));
        return format!("{s}-AD.{x}");
    }
    plugin.to_string()
}

/// Files keyed to a plugin's name: its archives, its ini, its string files
/// and its translations. (original relative to Data, alias relative to Data)
fn companions(data: &Path, plugin: &str, alias: &str) -> Vec<(String, String)> {
    let s = stem(plugin);
    let a = stem(alias).to_string();
    let sl = s.to_ascii_lowercase();
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(data) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            let l = n.to_ascii_lowercase();
            for suffix in [".bsa", " - textures.bsa", ".ini"] {
                if l == format!("{sl}{suffix}") {
                    out.push((n.clone(), format!("{a}{}", &n[s.len()..])));
                }
            }
        }
    }
    for dir in ["Strings", "Interface/Translations"] {
        if let Ok(rd) = std::fs::read_dir(data.join(dir)) {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().into_owned();
                if n.to_ascii_lowercase().starts_with(&format!("{sl}_")) {
                    out.push((format!("{dir}/{n}"), format!("{dir}/{a}{}", &n[s.len()..])));
                }
            }
        }
    }
    out
}

/// Replaces `from` with `to` in a load-order file, keeping its `*` and its
/// place; drops `from` when `to` is already listed. Returns whether it
/// changed the file. The old file is kept next to it.
fn rename_in(txt: &Path, from: &str, to: &str) -> std::io::Result<bool> {
    let Ok(text) = std::fs::read_to_string(txt) else { return Ok(false) };
    let name = |l: &str| l.trim().trim_start_matches('*').trim().to_ascii_lowercase();
    let (f, t) = (from.to_ascii_lowercase(), to.to_ascii_lowercase());
    if !text.lines().any(|l| name(l) == f) {
        return Ok(false);
    }
    let has_to = text.lines().any(|l| name(l) == t);
    let mut out: Vec<String> = Vec::new();
    for l in text.lines() {
        if name(l) == f {
            if !has_to {
                let on = l.trim().starts_with('*');
                out.push(format!("{}{to}", if on { "*" } else { "" }));
            }
        } else {
            out.push(l.to_string());
        }
    }
    out.push(String::new());
    std::fs::write(txt.with_extension("txt.aetherial-dawn-backup"), &text)?;
    std::fs::write(txt, out.join(if text.contains("\r\n") { "\r\n" } else { "\n" }))?;
    Ok(true)
}

/// Makes and refreshes the dash-named links, points plugins.txt and
/// loadorder.txt at them, and removes links whose original is gone. Returns
/// the plugins now running under another name, (original, alias).
pub fn ensure(game_dir: &Path, plugins_txt: Option<&Path>) -> std::io::Result<Vec<(String, String)>> {
    let data = game_dir.join("Data");
    let mut rec = load(game_dir);
    // Links whose original is gone go too, so a mod removed in Vortex
    // doesn't linger under its other name.
    rec.links.retain(|l| {
        if game_dir.join(&l.from).is_file() {
            return true;
        }
        let _ = std::fs::remove_file(game_dir.join(&l.to));
        false
    });
    let mut plugins = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&data) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if is_plugin(&n) && e.path().is_file() && !rec.links.iter().any(|l| l.to.eq_ignore_ascii_case(&format!("Data/{n}"))) {
                let rewritten = needs_rewrite(&e.path());
                if !client_can_load_name(&n) || rewritten {
                    plugins.push((n, rewritten));
                }
            }
        }
    }
    plugins.sort();
    let mut out = Vec::new();
    for (p, rewritten) in plugins {
        let alias = run_name(&p, rewritten);
        if rewritten {
            rewrite(&data.join(&p), &data.join(&alias))?;
        }
        let mut pairs = if rewritten { Vec::new() } else { vec![(p.clone(), alias.clone())] };
        pairs.extend(companions(&data, &p, &alias));
        let a = Alias { from: format!("Data/{p}"), to: format!("Data/{alias}") };
        if rewritten && !rec.links.contains(&a) {
            rec.links.push(a);
        }
        for (from, to) in pairs {
            link(&data.join(&from), &data.join(&to))?;
            let a = Alias { from: format!("Data/{from}"), to: format!("Data/{to}") };
            if !rec.links.contains(&a) {
                rec.links.push(a);
            }
        }
        if let Some(txt) = plugins_txt {
            rename_in(txt, &p, &alias)?;
            rename_in(&txt.with_file_name("loadorder.txt"), &p, &alias)?;
        }
        out.push((p, alias));
    }
    save(game_dir, &rec);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plugin_with_masters(ms: &[&str]) -> Vec<u8> {
        let mut sub = b"HEDR".to_vec();
        sub.extend(12u16.to_le_bytes());
        sub.extend(1.7f32.to_le_bytes());
        sub.extend([0u8; 8]);
        for m in ms {
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
        b.extend(b"GRUPrest-of-file");
        b
    }

    #[test]
    fn patches_of_renamed_plugins_get_their_masters_renamed() {
        let t = tempfile::tempdir().unwrap();
        let g = t.path();
        let data = g.join("Data");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("Obsidian Weathers.esp"), plugin_with_masters(&["Skyrim.esm"])).unwrap();
        std::fs::write(data.join("Obsidian CS.esp"), plugin_with_masters(&["Skyrim.esm", "Obsidian Weathers.esp"])).unwrap();
        std::fs::write(data.join("Audio Overhaul Skyrim.esp"), plugin_with_masters(&["Skyrim.esm"])).unwrap();
        std::fs::write(data.join("AOS_ISC_Integration.esp"), plugin_with_masters(&["Skyrim.esm", "Audio Overhaul Skyrim.esp"])).unwrap();
        std::fs::write(data.join("AOS_ISC_Integration.bsa"), b"bsa").unwrap();
        std::fs::write(data.join("Skyrim.esm"), plugin_with_masters(&[])).unwrap();
        let txt = g.join("plugins.txt");
        std::fs::write(&txt, "*Obsidian Weathers.esp\n*Obsidian CS.esp\n*Audio Overhaul Skyrim.esp\n*AOS_ISC_Integration.esp\n").unwrap();
        ensure(g, Some(&txt)).unwrap();
        assert_eq!(std::fs::read_to_string(&txt).unwrap(), "*Obsidian-Weathers.esp\n*Obsidian-CS.esp\n*Audio-Overhaul-Skyrim.esp\n*AOS_ISC_Integration-AD.esp\n");
        let m = |n: &str| crate::loadorder::masters(&data.join(n)).unwrap();
        assert_eq!(m("Obsidian-CS.esp"), ["Skyrim.esm", "Obsidian-Weathers.esp"]);
        assert_eq!(m("AOS_ISC_Integration-AD.esp"), ["Skyrim.esm", "Audio-Overhaul-Skyrim.esp"]);
        // The rest of the file is kept after the rewritten header.
        assert!(std::fs::read(data.join("AOS_ISC_Integration-AD.esp")).unwrap().ends_with(b"GRUPrest-of-file"));
        assert!(data.join("AOS_ISC_Integration-AD.bsa").is_file());
        assert_eq!(m("Obsidian Weathers.esp"), ["Skyrim.esm"]);
        assert_eq!(run_as(g, "AOS_ISC_Integration.esp"), "AOS_ISC_Integration-AD.esp");
        assert_eq!(run_as(g, "Obsidian CS.esp"), "Obsidian-CS.esp");
        assert_eq!(run_as(g, "Skyrim.esm"), "Skyrim.esm");
        // Again: nothing changes, and no copy of a copy.
        ensure(g, Some(&txt)).unwrap();
        assert!(!data.join("AOS_ISC_Integration-AD-AD.esp").exists());
        assert_eq!(std::fs::read_to_string(&txt).unwrap(), "*Obsidian-Weathers.esp\n*Obsidian-CS.esp\n*Audio-Overhaul-Skyrim.esp\n*AOS_ISC_Integration-AD.esp\n");
        // A hard-linked alias left by an older launcher (same bytes, same
        // time) is replaced by a rewritten copy; the original is untouched.
        let before = std::fs::read(data.join("Obsidian CS.esp")).unwrap();
        std::fs::remove_file(data.join("Obsidian-CS.esp")).unwrap();
        std::fs::hard_link(data.join("Obsidian CS.esp"), data.join("Obsidian-CS.esp")).unwrap();
        ensure(g, Some(&txt)).unwrap();
        assert_eq!(m("Obsidian-CS.esp"), ["Skyrim.esm", "Obsidian-Weathers.esp"]);
        assert_eq!(std::fs::read(data.join("Obsidian CS.esp")).unwrap(), before);
        assert_eq!(m("Obsidian CS.esp"), ["Skyrim.esm", "Obsidian Weathers.esp"]);
        // A plugin with no such master isn't rewritten.
        assert!(with_aliased_masters(&plugin_with_masters(&["Skyrim.esm"])).is_none());
    }

    #[test]
    fn names() {
        assert_eq!(alias_name("Unofficial Skyrim Special Edition Patch.esp"), "Unofficial-Skyrim-Special-Edition-Patch.esp");
        assert_eq!(alias_name("A  +  B .esp"), "A-B.esp");
        assert!(client_can_load_name(&alias_name("Élan's Mod (v2).esm")));
    }

    #[test]
    fn links_the_patch_and_points_the_load_order_at_it() {
        let t = tempfile::tempdir().unwrap();
        let g = t.path();
        let data = g.join("Data");
        std::fs::create_dir_all(data.join("Interface/Translations")).unwrap();
        std::fs::write(data.join("Unofficial Skyrim Special Edition Patch.esp"), b"esp").unwrap();
        std::fs::write(data.join("Unofficial Skyrim Special Edition Patch.bsa"), b"bsa").unwrap();
        std::fs::write(data.join("Interface/Translations/Unofficial Skyrim Special Edition Patch_english.txt"), b"t").unwrap();
        std::fs::write(data.join("SkyUI_SE.esp"), b"x").unwrap();
        let txt = g.join("plugins.txt");
        std::fs::write(&txt, "# Vortex\r\n*unofficial skyrim special edition patch.esp\r\n*SkyUI_SE.esp\r\n").unwrap();
        std::fs::write(g.join("loadorder.txt"), "Skyrim.esm\r\nunofficial skyrim special edition patch.esp\r\nSkyUI_SE.esp\r\n").unwrap();
        let got = ensure(g, Some(&txt)).unwrap();
        assert_eq!(got, [("Unofficial Skyrim Special Edition Patch.esp".to_string(), "Unofficial-Skyrim-Special-Edition-Patch.esp".to_string())]);
        assert_eq!(std::fs::read(data.join("Unofficial-Skyrim-Special-Edition-Patch.esp")).unwrap(), b"esp");
        assert_eq!(std::fs::read(data.join("Unofficial-Skyrim-Special-Edition-Patch.bsa")).unwrap(), b"bsa");
        assert!(data.join("Interface/Translations/Unofficial-Skyrim-Special-Edition-Patch_english.txt").is_file());
        assert!(data.join("Unofficial Skyrim Special Edition Patch.esp").is_file());
        assert_eq!(std::fs::read_to_string(&txt).unwrap(), "# Vortex\r\n*Unofficial-Skyrim-Special-Edition-Patch.esp\r\n*SkyUI_SE.esp\r\n");
        assert_eq!(std::fs::read_to_string(g.join("loadorder.txt")).unwrap(), "Skyrim.esm\r\nUnofficial-Skyrim-Special-Edition-Patch.esp\r\nSkyUI_SE.esp\r\n");
        // Again: nothing changes.
        ensure(g, Some(&txt)).unwrap();
        assert_eq!(std::fs::read_to_string(&txt).unwrap(), "# Vortex\r\n*Unofficial-Skyrim-Special-Edition-Patch.esp\r\n*SkyUI_SE.esp\r\n");
        // Switched off in Vortex (Vortex writes its own name back): stays off.
        std::fs::write(&txt, "*SkyUI_SE.esp\r\nunofficial skyrim special edition patch.esp\r\n").unwrap();
        ensure(g, Some(&txt)).unwrap();
        assert_eq!(std::fs::read_to_string(&txt).unwrap(), "*SkyUI_SE.esp\r\nUnofficial-Skyrim-Special-Edition-Patch.esp\r\n");
        assert_eq!(links(g).len(), 3);
        // Original removed: its links go.
        std::fs::remove_file(data.join("Unofficial Skyrim Special Edition Patch.esp")).unwrap();
        ensure(g, Some(&txt)).unwrap();
        assert!(!data.join("Unofficial-Skyrim-Special-Edition-Patch.esp").exists());
    }
}
