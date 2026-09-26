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

/// Files keyed to a plugin's name: its archives, its ini, its string files
/// and its translations. (original relative to Data, alias relative to Data)
fn companions(data: &Path, plugin: &str) -> Vec<(String, String)> {
    let s = stem(plugin);
    let a = stem(&alias_name(plugin)).to_string();
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
            if is_plugin(&n) && !client_can_load_name(&n) && e.path().is_file() {
                plugins.push(n);
            }
        }
    }
    plugins.sort();
    let mut out = Vec::new();
    for p in plugins {
        let alias = alias_name(&p);
        let mut pairs = vec![(p.clone(), alias.clone())];
        pairs.extend(companions(&data, &p));
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
