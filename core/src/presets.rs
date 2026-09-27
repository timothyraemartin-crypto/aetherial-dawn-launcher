//! Each listed mod's settings, the server's preset (PRESETS.md, Timothy
//! 02:58 2026-09-27: "set the preferences for all of them"). A mods.json
//! entry's `settings` names keys in a file under Data; the launcher writes
//! each key once, the first time the mod is in, and records what it wrote in
//! `.aetherial-dawn/mods/presets.json`. After that the player's own changes
//! stay; a key is written again only when the list changes its value.
//!
//! MCM Helper files (`Data/MCM/Settings/<Mod>.ini`) only get keys the mod's
//! own `Data/MCM/Config/<Mod>/settings.ini` defines; anything else is
//! logged and skipped, never guessed. Every file is backed up before the
//! first change.

use std::collections::BTreeMap;
use std::path::{Component, Path};

use serde_json::Value;

use crate::modlist::ModEntry;

const RECORD: &str = ".aetherial-dawn/mods/presets.json";

/// One settings file of a mod and the keys to set in it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Setting {
    /// Path under the game folder, starting with "Data/".
    pub file: String,
    /// "ini", "json" or "xml".
    pub format: String,
    /// "<section>.<key>" (ini) or a dotted path (json) to its value.
    pub set: BTreeMap<String, Value>,
}

/// mod id -> "<file>|<key>" -> the value last written.
type Record = BTreeMap<String, BTreeMap<String, Value>>;

fn load(game_dir: &Path) -> Record {
    std::fs::read(game_dir.join(RECORD)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save(game_dir: &Path, r: &Record) -> std::io::Result<()> {
    let p = game_dir.join(RECORD);
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(p, serde_json::to_vec_pretty(r)?)
}

/// A served path that stays inside Data and names a settings file.
/// SkyPatcher's inis are never written: they change records on this
/// client only, which desyncs it from the server.
fn safe(file: &str) -> bool {
    let p = Path::new(file);
    let l = file.to_ascii_lowercase();
    l.starts_with("data/")
        && !l.starts_with("data/skse/plugins/skypatcher/")
        && [".ini", ".json", ".xml"].iter().any(|x| l.ends_with(x))
        && p.components().all(|c| matches!(c, Component::Normal(_)))
}

/// The MCM Helper config that defines a settings file's keys, when the file
/// is one (`Data/MCM/Settings/<Mod>.ini` -> `Data/MCM/Config/<Mod>/settings.ini`).
fn mcm_config(file: &str) -> Option<String> {
    let l = file.to_ascii_lowercase();
    let name = l.strip_prefix("data/mcm/settings/")?.strip_suffix(".ini")?;
    // Same length lower-cased (ASCII), so the original spelling is kept.
    let name = &file[file.len() - 4 - name.len()..file.len() - 4];
    (!name.is_empty() && !name.contains('/')).then(|| format!("Data/MCM/Config/{name}/settings.ini"))
}

/// The file on disk, matching its path case-insensitively (Windows does;
/// the tests run on Linux).
fn find(game_dir: &Path, rel: &str) -> std::path::PathBuf {
    let mut at = game_dir.to_path_buf();
    for part in rel.split('/') {
        let exact = at.join(part);
        at = if exact.exists() {
            exact
        } else {
            std::fs::read_dir(&at)
                .ok()
                .and_then(|rd| rd.flatten().find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(part)).map(|e| e.path()))
                .unwrap_or(exact)
        };
    }
    at
}

fn split_key(key: &str) -> (&str, &str) {
    key.split_once('.').unwrap_or(("", key))
}

/// Sections and their keys, lower-cased.
pub fn ini_keys(text: &str) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut section = String::new();
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') && t.ends_with(']') {
            section = t[1..t.len() - 1].trim().to_string();
            out.entry(section.clone()).or_default();
        } else if !t.starts_with(';') && !t.starts_with('#') {
            if let Some((k, _)) = t.split_once('=') {
                out.entry(section.clone()).or_default().push(k.trim().to_string());
            }
        }
    }
    out
}

fn has_key(keys: &BTreeMap<String, Vec<String>>, key: &str) -> bool {
    let (s, k) = split_key(key);
    keys.iter().any(|(sec, ks)| sec.eq_ignore_ascii_case(s) && ks.iter().any(|x| x.eq_ignore_ascii_case(k)))
}

fn ini_value(v: &Value) -> String {
    match v {
        Value::Bool(b) => if *b { "1" } else { "0" }.into(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Sets "<section>.<key>" in ini text, keeping every other line as it is.
pub fn ini_set(text: &str, key: &str, value: &Value) -> String {
    let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let (s, k) = split_key(key);
    let v = ini_value(value);
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let mut section = String::new();
    let mut last_in_section: Option<usize> = None;
    let mut seen_section = s.is_empty();
    for (i, line) in lines.iter_mut().enumerate() {
        let t = line.trim();
        if t.starts_with('[') && t.ends_with(']') {
            section = t[1..t.len() - 1].trim().to_string();
            if section.eq_ignore_ascii_case(s) {
                seen_section = true;
            }
            continue;
        }
        if !section.eq_ignore_ascii_case(s) {
            continue;
        }
        if !t.is_empty() {
            last_in_section = Some(i);
        }
        if t.starts_with(';') || t.starts_with('#') {
            continue;
        }
        if let Some((lk, _)) = t.split_once('=') {
            if lk.trim().eq_ignore_ascii_case(k) {
                let indent = &line[..line.len() - line.trim_start().len()];
                *line = format!("{indent}{} = {v}", lk.trim());
                return lines.join(nl) + nl;
            }
        }
    }
    let entry = format!("{k} = {v}");
    match (seen_section, last_in_section) {
        (true, Some(i)) => lines.insert(i + 1, entry),
        (true, None) if s.is_empty() => lines.insert(0, entry),
        (true, None) => {
            let at = lines.iter().position(|l| {
                let t = l.trim();
                t.starts_with('[') && t[1..t.len() - 1].trim().eq_ignore_ascii_case(s)
            });
            lines.insert(at.map(|i| i + 1).unwrap_or(lines.len()), entry);
        }
        (false, _) => {
            if lines.last().map(|l| !l.trim().is_empty()).unwrap_or(false) {
                lines.push(String::new());
            }
            lines.push(format!("[{s}]"));
            lines.push(entry);
        }
    }
    lines.join(nl) + nl
}

/// Sets a dotted path in a JSON object, making the objects on the way.
pub fn json_set(doc: &mut Value, key: &str, value: &Value) -> bool {
    let mut at = doc;
    let parts: Vec<&str> = key.split('.').collect();
    for p in &parts[..parts.len() - 1] {
        let Some(o) = at.as_object_mut() else { return false };
        at = o.entry(p.to_string()).or_insert_with(|| Value::Object(Default::default()));
    }
    match at.as_object_mut() {
        Some(o) => {
            o.insert(parts[parts.len() - 1].to_string(), value.clone());
            true
        }
        None => false,
    }
}

/// Finds `<name>` or `<name .../>` in text[from..to]: (start of the tag,
/// end of the opening tag, and for a non-empty element the start and end of
/// its closing tag).
type XmlSpan = (usize, usize, Option<(usize, usize)>);

fn xml_find(text: &str, name: &str, from: usize, to: usize) -> Option<XmlSpan> {
    let mut at = from;
    while let Some(i) = text[at..to].find(&format!("<{name}")) {
        let start = at + i;
        let after = text[start + 1 + name.len()..].chars().next()?;
        if after == '>' || after == '/' || after.is_whitespace() {
            let open_end = start + text[start..to].find('>')? + 1;
            if text[..open_end].ends_with("/>") {
                return Some((start, open_end, None));
            }
            let close = format!("</{name}>");
            let c = open_end + text[open_end..to].find(&close)?;
            return Some((start, open_end, Some((c, c + close.len()))));
        }
        at = start + 1;
    }
    None
}

fn xml_escape(v: &str) -> String {
    v.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Sets the text of the element at a dotted path ("Config.SelectedPreset"),
/// adding the last element to its parent when it's missing. None when the
/// parent isn't there.
pub fn xml_set(text: &str, key: &str, value: &Value) -> Option<String> {
    let v = xml_escape(&ini_value(value));
    let parts: Vec<&str> = key.split('.').collect();
    let (mut from, mut to) = (0, text.len());
    for p in &parts[..parts.len() - 1] {
        let (_, open_end, close) = xml_find(text, p, from, to)?;
        let (c, _) = close?;
        (from, to) = (open_end, c);
    }
    let last = parts[parts.len() - 1];
    Some(match xml_find(text, last, from, to) {
        Some((_, open_end, Some((c, _)))) => format!("{}{v}{}", &text[..open_end], &text[c..]),
        Some((start, open_end, None)) => format!("{}<{last}>{v}</{last}>{}", &text[..start], &text[open_end..]),
        None => {
            let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
            let line_start = text[..to].rfind('\n').map(|i| i + 1).unwrap_or(to);
            let indent: String = text[line_start..to].chars().take_while(|c| c.is_whitespace()).collect();
            format!("{}{indent}    <{last}>{v}</{last}>{nl}{}", &text[..line_start], &text[line_start..])
        }
    })
}

/// Writes a new file beside the old one and renames it over, so a hard link
/// (Vortex deploys by hard link from its staging folder) is replaced, never
/// written through.
fn replace(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".aetherial-part");
    let tmp = std::path::PathBuf::from(tmp);
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

fn back_up(game_dir: &Path, rel: &str, stamp: &str) -> std::io::Result<()> {
    let from = find(game_dir, rel);
    if !from.is_file() {
        return Ok(());
    }
    let to = game_dir.join(crate::strays::DISABLED_DIR).join(stamp).join(rel);
    if to.exists() {
        return Ok(());
    }
    if let Some(p) = to.parent() {
        std::fs::create_dir_all(p)?;
    }
    std::fs::copy(from, to).map(|_| ())
}

/// Writes the preset keys that aren't written yet (or whose value the list
/// changed) for every installed mod. Returns lines for the log.
pub fn apply_all(game_dir: &Path, list: &[ModEntry]) -> Vec<String> {
    let mut rec = load(game_dir);
    let mut log = Vec::new();
    let mut changed = false;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let stamp = format!("{now}-presets");
    for m in list.iter().filter(|m| !m.settings.is_empty()) {
        if !m.installed(game_dir) {
            continue;
        }
        for s in &m.settings {
            if !safe(&s.file) {
                log.push(format!("{}: skipped settings for {} (not a settings file under Data)", m.name, s.file));
                continue;
            }
            let done = rec.get(&m.id);
            let todo: Vec<(&String, &Value)> = s.set.iter().filter(|(k, v)| done.and_then(|d| d.get(&format!("{}|{k}", s.file))) != Some(*v)).collect();
            if todo.is_empty() {
                continue;
            }
            match write_one(game_dir, s, &todo, &stamp) {
                Ok((wrote, skipped)) => {
                    if !wrote.is_empty() {
                        let r = rec.entry(m.id.clone()).or_default();
                        for k in &wrote {
                            r.insert(format!("{}|{k}", s.file), s.set[k].clone());
                        }
                        changed = true;
                        log.push(format!("{}: set {} in {} (old file backed up)", m.name, wrote.join(", "), s.file));
                    }
                    if !skipped.is_empty() {
                        log.push(format!("{}: left {} in {}: {}", m.name, skipped.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>().join(", "), s.file, skipped[0].1));
                    }
                }
                Err(e) => log.push(format!("{}: couldn't write {}: {e}", m.name, s.file)),
            }
        }
    }
    if changed {
        if let Err(e) = save(game_dir, &rec) {
            log.push(format!("couldn't record the presets written: {e}"));
        }
    }
    log
}

type Written = (Vec<String>, Vec<(String, &'static str)>);

fn write_one(game_dir: &Path, s: &Setting, todo: &[(&String, &Value)], stamp: &str) -> std::io::Result<Written> {
    let path = find(game_dir, &s.file);
    let mut wrote = Vec::new();
    let mut skipped = Vec::new();
    match s.format.to_ascii_lowercase().as_str() {
        "ini" => {
            let allowed = match mcm_config(&s.file) {
                Some(cfg) => match std::fs::read_to_string(find(game_dir, &cfg)) {
                    Ok(t) => Some(ini_keys(&t)),
                    Err(_) => {
                        skipped.extend(todo.iter().map(|(k, _)| ((*k).clone(), "its MCM config isn't installed yet")));
                        return Ok((wrote, skipped));
                    }
                },
                None => None,
            };
            let mut text = std::fs::read_to_string(&path).unwrap_or_default();
            for (k, v) in todo {
                if v.as_str().is_some_and(|t| t.contains(['\n', '\r'])) || k.contains(['\n', '\r', '=', '[', ']']) {
                    skipped.push(((*k).clone(), "a value or key can't span lines"));
                    continue;
                }
                if allowed.as_ref().map(|a| !has_key(a, k)).unwrap_or(false) {
                    skipped.push(((*k).clone(), "the mod's MCM config has no such key"));
                    continue;
                }
                text = ini_set(&text, k, v);
                wrote.push((*k).clone());
            }
            if !wrote.is_empty() {
                back_up(game_dir, &s.file, stamp)?;
                replace(&path, text.as_bytes())?;
            }
        }
        "json" => {
            // The mod makes the full file on its first start; a file with
            // only our keys could leave the rest unset.
            let Ok(bytes) = std::fs::read(&path) else {
                skipped.extend(todo.iter().map(|(k, _)| ((*k).clone(), "waits until the game has made the file")));
                return Ok((wrote, skipped));
            };
            let mut doc: Value = serde_json::from_slice(&bytes)?;
            for (k, v) in todo {
                if json_set(&mut doc, k, v) {
                    wrote.push((*k).clone());
                } else {
                    skipped.push(((*k).clone(), "that path isn't an object in the file"));
                }
            }
            if !wrote.is_empty() {
                back_up(game_dir, &s.file, stamp)?;
                replace(&path, serde_json::to_string_pretty(&doc)?.as_bytes())?;
            }
        }
        "xml" => {
            let Ok(mut text) = std::fs::read_to_string(&path) else {
                skipped.extend(todo.iter().map(|(k, _)| ((*k).clone(), "waits until the mod has installed the file")));
                return Ok((wrote, skipped));
            };
            for (k, v) in todo {
                if v.as_str().is_some_and(|t| t.contains(['\n', '\r'])) || !k.split('.').all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')) {
                    skipped.push(((*k).clone(), "not a one-line value or a plain element name"));
                    continue;
                }
                match xml_set(&text, k, v) {
                    Some(t) => {
                        text = t;
                        wrote.push((*k).clone());
                    }
                    None => skipped.push(((*k).clone(), "that element's parent isn't in the file")),
                }
            }
            if !wrote.is_empty() {
                back_up(game_dir, &s.file, stamp)?;
                replace(&path, text.as_bytes())?;
            }
        }
        _ => skipped.extend(todo.iter().map(|(k, _)| ((*k).clone(), "this launcher can't write that format"))),
    }
    Ok((wrote, skipped))
}

/// For the log after an install: the sections and keys of every MCM config
/// the mod put in, so the preset can name them exactly.
pub fn mcm_keys(game_dir: &Path, files: &[String]) -> Vec<String> {
    files
        .iter()
        .filter(|f| {
            let l = f.to_ascii_lowercase();
            l.starts_with("data/mcm/config/") && l.ends_with("/settings.ini")
        })
        .filter_map(|f| {
            let text = std::fs::read_to_string(game_dir.join(f)).ok()?;
            let keys = ini_keys(&text);
            let body = keys.iter().filter(|(_, ks)| !ks.is_empty()).map(|(s, ks)| format!("[{s}] {}", ks.join(", "))).collect::<Vec<_>>().join("; ");
            Some(format!("{f}: {body}"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry(settings: Vec<Setting>) -> ModEntry {
        let mut m: ModEntry = serde_json::from_value(json!({"id": "precision", "name": "Precision", "check": ["Data/SKSE/Plugins/Precision.dll"]})).unwrap();
        m.settings = settings;
        m
    }

    fn put(g: &Path, rel: &str, text: &str) {
        let p = g.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    #[test]
    fn ini_keeps_other_lines() {
        let t = "; top\n[General]\nbA = 0\nfB=2.0\n\n[Other]\nx = 1\n";
        let out = ini_set(t, "general.fb", &json!(0.5));
        assert_eq!(out, "; top\n[General]\nbA = 0\nfB = 0.5\n\n[Other]\nx = 1\n");
        let out = ini_set(&out, "General.bNew", &json!(true));
        assert!(out.contains("fB = 0.5\nbNew = 1\n\n[Other]"));
        let out = ini_set(&out, "Third.s", &json!("on"));
        assert!(out.ends_with("x = 1\n\n[Third]\ns = on\n"));
        assert_eq!(ini_set("a=1\r\n", ".a", &json!(2)), "a = 2\r\n");
    }

    #[test]
    fn json_sets_paths() {
        let mut d = json!({"SSS": {"Enabled": false}, "x": 1});
        assert!(json_set(&mut d, "SSS.Enabled", &json!(true)));
        assert!(json_set(&mut d, "Wetness.Enabled", &json!(true)));
        assert!(!json_set(&mut d, "x.y", &json!(1)));
        assert_eq!(d, json!({"SSS": {"Enabled": true}, "Wetness": {"Enabled": true}, "x": 1}));
    }

    #[test]
    fn writes_once_keeps_player_changes_and_reapplies_a_changed_value() {
        let t = tempfile::tempdir().unwrap();
        let g = t.path();
        let s = |v: f64| Setting {
            file: "Data/MCM/Settings/Precision.ini".into(),
            format: "ini".into(),
            set: [("General.fHitstop".to_string(), json!(v)), ("General.bGuess".to_string(), json!(true))].into_iter().collect(),
        };
        let list = vec![entry(vec![s(0.0)])];
        assert!(apply_all(g, &list).is_empty(), "nothing before the mod is installed");
        put(g, "Data/SKSE/Plugins/Precision.dll", "x");
        let log = apply_all(g, &list);
        assert!(log[0].contains("MCM config isn't installed"), "{log:?}");
        put(g, "Data/MCM/Config/Precision/settings.ini", "[General]\nfHitstop = 1.0\nfOther = 2\n");
        let log = apply_all(g, &list);
        assert!(log[0].contains("set General.fHitstop"), "{log:?}");
        assert!(log[1].contains("bGuess") && log[1].contains("no such key"), "{log:?}");
        let file = g.join("Data/MCM/Settings/Precision.ini");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "[General]\nfHitstop = 0.0\n");

        // The player's change stays.
        std::fs::write(&file, "[General]\nfHitstop = 0.3\n").unwrap();
        let log = apply_all(g, &list);
        assert!(log.iter().all(|l| !l.contains("set ")), "{log:?}");
        assert!(std::fs::read_to_string(&file).unwrap().contains("0.3"));

        // A new preset value goes in once, after a backup.
        let log = apply_all(g, &[entry(vec![s(0.1)])]);
        assert!(log[0].contains("set General.fHitstop"), "{log:?}");
        assert!(std::fs::read_to_string(&file).unwrap().contains("fHitstop = 0.1"));
        let aside = std::fs::read_dir(g.join(crate::strays::DISABLED_DIR)).unwrap().flatten().next().unwrap().path();
        assert!(std::fs::read_to_string(aside.join("Data/MCM/Settings/Precision.ini")).unwrap().contains("0.3"));
    }

    #[test]
    fn json_waits_for_the_file_and_unsafe_paths_are_refused() {
        let t = tempfile::tempdir().unwrap();
        let g = t.path();
        put(g, "Data/SKSE/Plugins/Precision.dll", "x");
        let cs = Setting {
            file: "Data/SKSE/Plugins/CommunityShaders/SettingsUser.json".into(),
            format: "json".into(),
            set: [("Wetness.Enabled".to_string(), json!(true))].into_iter().collect(),
        };
        let bad = Setting { file: "Data/../SkyrimSE.exe".into(), format: "ini".into(), set: [("a.b".to_string(), json!(1))].into_iter().collect() };
        let list = vec![entry(vec![cs, bad])];
        let log = apply_all(g, &list);
        assert!(log.iter().any(|l| l.contains("waits until")), "{log:?}");
        assert!(log.iter().any(|l| l.contains("not a settings file")), "{log:?}");
        put(g, "Data/SKSE/Plugins/CommunityShaders/SettingsUser.json", r#"{"Wetness":{"Enabled":false,"Rain":2}}"#);
        apply_all(g, &list);
        let v: Value = serde_json::from_slice(&std::fs::read(g.join("Data/SKSE/Plugins/CommunityShaders/SettingsUser.json")).unwrap()).unwrap();
        assert_eq!(v, json!({"Wetness": {"Enabled": true, "Rain": 2}}));
        assert!(!safe("Data/x.dll") && !safe("/etc/x.ini") && !safe("Skyrim.ini") && safe("Data/MCM/Settings/A.ini"));
        assert!(!safe("Data/SKSE/Plugins/SkyPatcher/npc/x.ini"));
    }

    #[test]
    fn never_writes_through_a_hard_link_or_adds_lines() {
        let t = tempfile::tempdir().unwrap();
        let g = t.path();
        put(g, "Data/SKSE/Plugins/Precision.dll", "x");
        put(g, "staging/Shipped.ini", "[A]\nb = 1\n");
        std::fs::create_dir_all(g.join("Data/SKSE/Plugins")).unwrap();
        std::fs::hard_link(g.join("staging/Shipped.ini"), g.join("Data/SKSE/Plugins/Shipped.ini")).unwrap();
        let s = Setting {
            file: "Data/SKSE/Plugins/Shipped.ini".into(),
            format: "ini".into(),
            set: [("A.b".to_string(), json!(2)), ("A.c".to_string(), json!("x\n[Evil]"))].into_iter().collect(),
        };
        let log = apply_all(g, &[entry(vec![s])]);
        assert!(log.iter().any(|l| l.contains("A.c") && l.contains("span lines")), "{log:?}");
        assert_eq!(std::fs::read_to_string(g.join("Data/SKSE/Plugins/Shipped.ini")).unwrap(), "[A]\nb = 2\n");
        assert_eq!(std::fs::read_to_string(g.join("staging/Shipped.ini")).unwrap(), "[A]\nb = 1\n", "Vortex's staging copy is untouched");
        assert!(!g.join("Data/SKSE/Plugins/Shipped.ini.aetherial-part").exists());
    }

    #[test]
    fn xml_sets_element_text() {
        let t = "<Config>\n    <ShapeDataPath>x</ShapeDataPath>\n    <SelectedPreset>CBBE</SelectedPreset>\n    <Empty/>\n</Config>\n";
        let out = xml_set(t, "Config.SelectedPreset", &json!("3BA & more")).unwrap();
        assert!(out.contains("<SelectedPreset>3BA &amp; more</SelectedPreset>") && out.contains("<ShapeDataPath>x</ShapeDataPath>"));
        let out = xml_set(&out, "Config.Empty", &json!(1)).unwrap();
        assert!(out.contains("<Empty>1</Empty>"));
        let out = xml_set(&out, "Config.New", &json!(true)).unwrap();
        assert!(out.ends_with("<Empty>1</Empty>\n    <New>1</New>\n</Config>\n"), "{out}");
        assert!(xml_set(t, "Other.X", &json!(1)).is_none());
        assert!(xml_set(t, "Config.SelectedPresetX", &json!(1)).unwrap().contains("<SelectedPreset>CBBE</SelectedPreset>"));
    }

    #[test]
    fn logs_mcm_keys() {
        let t = tempfile::tempdir().unwrap();
        let g = t.path();
        put(g, "Data/MCM/Config/Precision/settings.ini", "[General]\nfHitstop = 1\n; c\n[Trails]\nbOn=1\n");
        let l = mcm_keys(g, &["Data/MCM/Config/Precision/settings.ini".into(), "Data/x.esp".into()]);
        assert_eq!(l, vec!["Data/MCM/Config/Precision/settings.ini: [General] fHitstop; [Trails] bOn".to_string()]);
    }
}
