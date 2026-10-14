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
/// "Unofficial-Skyrim-Special-Edition-Patch.esp": the canonical name the
/// server and every PC share (SERVER-PLUGINS.md). Apostrophes and
/// ampersands are dropped ("JK's Skyrim.esp" -> "JKs-Skyrim.esp",
/// "Cloaks&Capes.esp" -> "CloaksCapes.esp"); every other character the client
/// rejects becomes a dash, with runs of dashes joined.
pub fn alias_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars().filter(|c| !matches!(c, '\'' | '\u{2019}' | '&')) {
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
    game_dir.join(".aetherial-dawn/mods").join("aliases.json")
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
        // tmp + rename: a power cut leaves the old record, not a cut one that reads as empty.
        if std::env::var_os("AD_ALIASES_ATOMIC_SAVE").is_some_and(|v| v == "0") {
            let _ = std::fs::write(p, b);
        } else {
            let _ = crate::atomicfile::write(&p, &b);
        }
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

fn link(from: &Path, to: &Path, ours: bool) -> std::io::Result<()> {
    if to.exists() {
        if in_step(from, to) {
            return Ok(());
        }
        // Only a file the launcher made may be replaced; a dash-named file
        // the player or a mod put there is theirs.
        if !ours {
            return Err(std::io::Error::other(format!("{} is already in Data and isn't the launcher's; not replacing it", to.display())));
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
    rename_masters(bytes, &|m: &str| (!client_can_load_name(m)).then(|| alias_name(m)))
}

/// The TES4 "light" (ESL) flag.
const LIGHT: u32 = 0x200;

/// A plugin the game itself ships: the five base masters, and Creation
/// Club content by its name. Their ESL flags are the game's own business.
/// Only the name counts, never Skyrim.ccc: the server has none, and the
/// launcher empties the PC's for a session, so both decide the same way.
pub(crate) fn shipped_with_game(plugin: &str) -> bool {
    if crate::health::MASTERS.iter().any(|m| m.eq_ignore_ascii_case(plugin)) || plugin.eq_ignore_ascii_case("_ResourcePack.esl") {
        return true;
    }
    // Names shaped like Creation Club files: cc + a 3-letter creator code +
    // "sse" + 3 digits, then "-" or "_" and a name of letters, digits and
    // "_", then .esm, .esp or .esl, as in ccBGSSSE025-AdvDSGS.esm or
    // ccKRTSSE001_Altar.esl. A mod's own file such as "ccBGSSSE001-Fish -
    // Patch.esp" has a space and doesn't match.
    let b = plugin.as_bytes();
    let Some(dot) = plugin.rfind('.') else { return false };
    let ext = &plugin[dot..];
    b.len() > 13
        && [".esm", ".esp", ".esl"].iter().any(|e| e.eq_ignore_ascii_case(ext))
        && b[..2].eq_ignore_ascii_case(b"cc")
        && b[2..5].iter().all(u8::is_ascii_alphabetic)
        && b[5..8].eq_ignore_ascii_case(b"sse")
        && b[8..11].iter().all(u8::is_ascii_digit)
        && matches!(b[11], b'-' | b'_')
        && dot > 12
        && b[12..dot].iter().all(|&c| c.is_ascii_alphanumeric() || c == b'_')
}

/// The whole TES4 header is there (at most 1 MiB, the most `head` reads),
/// so the server, which reads whole files, and PCs, which read headers,
/// decide the same way.
fn full_header(bytes: &[u8]) -> bool {
    bytes.len() >= 24 && &bytes[..4] == b"TES4" && {
        let size = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
        size <= 1 << 20 && bytes.len() >= 24 + size
    }
}

/// An .esp or .esm with the ESL flag set. The game loads it as a light
/// plugin on PCs, while the SkyMP server gives it a full load-order index,
/// so every plugin after it would disagree (Kad_MoonMonkRobes.esp,
/// 2026-09-27). Its canonical copy has the flag cleared. An .esl stays
/// light whatever its flag says, and the game's own plugins are left alone.
fn light_flagged(plugin: &str, bytes: &[u8]) -> bool {
    !plugin.to_ascii_lowercase().ends_with(".esl")
        && full_header(bytes)
        && u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) & LIGHT != 0
        && !shipped_with_game(plugin)
}

/// The TES4 "master" flag.
const MASTER: u32 = 0x1;

/// Whether an .esl is one the server's list runs as a full plugin
/// (desync/esl-on-server.md, 2026-09-30): the SkyMP server gives every
/// plugin a full index, so a light plugin the PC loads as FE would disagree
/// with it. Only the .esl files named in `full` are converted, Creation Club
/// ones included; any other .esl stays as it is.
fn full_esl(plugin: &str, full: &[String]) -> bool {
    plugin.to_ascii_lowercase().ends_with(".esl") && full.iter().any(|f| f.eq_ignore_ascii_case(plugin))
}

/// The .esl files a list runs as full plugins: every name in `server` ending
/// in .esm that Data holds only as "<stem>.esl" (or as the launcher's own
/// .esm copy of it). "_ResourcePack.esm" on the list and "_ResourcePack.esl"
/// in Data gives "_ResourcePack.esl".
pub fn light_as_full(game_dir: &Path, server: &[String]) -> Vec<String> {
    let data = game_dir.join("Data");
    let ours = load(game_dir).links;
    let mut out: Vec<String> = Vec::new();
    for n in server {
        if !n.to_ascii_lowercase().ends_with(".esm") {
            continue;
        }
        let esl = format!("{}.esl", stem(n));
        let Some(real) = std::fs::read_dir(&data).ok().and_then(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).find(|f| f.eq_ignore_ascii_case(&esl))) else { continue };
        let esm_there = data.join(n).exists();
        let esm_ours = ours.iter().any(|l| l.to.eq_ignore_ascii_case(&format!("Data/{n}")) && l.from.eq_ignore_ascii_case(&format!("Data/{real}")));
        if (!esm_there || esm_ours) && !out.iter().any(|o| o.eq_ignore_ascii_case(&real)) {
            out.push(real);
        }
    }
    out
}

/// The canonical bytes of a plugin in `data`: masters that run under
/// another name renamed, and the ESL flag cleared on an .esp or .esm. None
/// when neither is needed (or the header can't be read).
pub fn canonical_bytes(data: &Path, plugin: &str, bytes: &[u8]) -> Option<Vec<u8>> {
    canonical_bytes_full(data, plugin, bytes, &[])
}

/// `canonical_bytes`, with the .esl files in `full` run as full plugins:
/// their copy has the light flag cleared and the master flag set, so it
/// stays in the game's master block on PCs.
pub fn canonical_bytes_full(data: &Path, plugin: &str, bytes: &[u8], full: &[String]) -> Option<Vec<u8>> {
    let renamed = with_masters_in_full(data, bytes, full);
    let (clear, set) = if full_esl(plugin, full) && full_header(bytes) {
        (LIGHT, MASTER)
    } else if light_flagged(plugin, bytes) {
        (LIGHT, 0)
    } else {
        return renamed;
    };
    let mut out = renamed.unwrap_or_else(|| bytes.to_vec());
    let f = (u32::from_le_bytes([out[8], out[9], out[10], out[11]]) & !clear) | set;
    out[8..12].copy_from_slice(&f.to_le_bytes());
    Some(out)
}

/// Whether a plugin in Data runs as a rewritten copy: it's ESL-flagged, or
/// a master it names runs under another name (a dashed alias, or itself a
/// rewritten copy's "-AD" name, down a chain of patches).
fn rewritten_in(data: &Path, plugin: &str, depth: u8, full: &[String]) -> bool {
    if depth > 32 {
        return false;
    }
    if full_esl(plugin, full) {
        return true;
    }
    if head(&data.join(plugin)).is_some_and(|h| light_flagged(plugin, &h)) {
        return true;
    }
    crate::loadorder::masters(&data.join(plugin)).unwrap_or_default().iter().any(|m| !client_can_load_name(m) || rewritten_in(data, m, depth + 1, full))
}

/// The name a master runs under in Data, when it isn't its own.
fn master_run_name(data: &Path, m: &str, full: &[String]) -> Option<String> {
    if full_esl(m, full) {
        Some(run_name_full(m, true, full))
    } else if !client_can_load_name(m) {
        Some(alias_name(m))
    } else if rewritten_in(data, m, 0, full) {
        Some(run_name(m, true))
    } else {
        None
    }
}

/// `with_aliased_masters` for a plugin in `data`: masters that run as
/// rewritten copies ("Patch-AD.esp") are renamed too.
pub fn with_masters_in(data: &Path, bytes: &[u8]) -> Option<Vec<u8>> {
    with_masters_in_full(data, bytes, &[])
}

/// `with_masters_in`, with the .esl files in `full` named by their .esm
/// run names ("_ResourcePack.esl" -> "_ResourcePack.esm").
pub fn with_masters_in_full(data: &Path, bytes: &[u8], full: &[String]) -> Option<Vec<u8>> {
    rename_masters(bytes, &|m: &str| master_run_name(data, m, full))
}

fn rename_masters(bytes: &[u8], rename: &dyn Fn(&str) -> Option<String>) -> Option<Vec<u8>> {
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
        let renamed = if kind == b"MAST" { rename(&name) } else { None };
        if let Some(new) = renamed {
            let mut new = new.into_bytes();
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

/// Whether a plugin names a master the client can't load, or is ESL-flagged.
fn needs_rewrite(data: &Path, path: &Path, full: &[String]) -> bool {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    head(path).is_some_and(|h| canonical_bytes_full(data, &name, &h, full).is_some())
}

/// Writes the rewritten copy, keeping the original's modified time so it's
/// only rewritten when the original changes.
fn rewrite(from: &Path, to: &Path, full: &[String]) -> std::io::Result<()> {
    let t = std::fs::metadata(from)?.modified()?;
    // Up to date only when it's already a rewritten copy: a dashed alias
    // made by 0.1.49-0.1.67 is a hard link with the same time and the old
    // master names.
    // Up to date only when its header is exactly the rewrite of the
    // original's (the master names follow the current naming rule).
    let data = from.parent().unwrap_or(Path::new("."));
    let name = from.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let fresh = head(from).and_then(|h| canonical_bytes_full(data, &name, &h, full)).is_some_and(|want| head(to).as_deref() == Some(&want[..]));
    if std::fs::metadata(to).and_then(|m| m.modified()).ok() == Some(t) && fresh {
        return Ok(());
    }
    let bytes = std::fs::read(from)?;
    let new = canonical_bytes_full(data, &name, &bytes, full).ok_or_else(|| std::io::Error::other("the plugin's header can't be rewritten"))?;
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
/// a loadable name whose masters need rewriting or whose ESL flag is cleared
/// ("<name>-AD.esm" for an .esm).
pub fn run_name(plugin: &str, rewritten: bool) -> String {
    if !client_can_load_name(plugin) {
        return alias_name(plugin);
    }
    if rewritten {
        let (s, x) = plugin.rsplit_once('.').unwrap_or((plugin, "esp"));
        return format!("{s}-AD.{x}");
    }
    plugin.to_string()
}

/// `run_name`, where an .esl in `full` runs as "<same stem>.esm": the stem
/// is kept so its archives and the strings inside them are still found.
pub fn run_name_full(plugin: &str, rewritten: bool, full: &[String]) -> String {
    if rewritten && full_esl(plugin, full) {
        let base = if client_can_load_name(plugin) { plugin.to_string() } else { alias_name(plugin) };
        return format!("{}.esm", stem(&base));
    }
    run_name(plugin, rewritten)
}

/// Files keyed to a plugin's name: its archives, its ini, its string files
/// and its translations. (original relative to Data, alias relative to Data)
pub fn companions(data: &Path, plugin: &str, alias: &str) -> Vec<(String, String)> {
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
    let Some(file) = crate::loadorder::ListFile::read(txt)? else { return Ok(false) };
    let text = file.text.as_str();
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
    crate::loadorder::write_list(txt, Some(&file), &out.join(if text.contains("\r\n") { "\r\n" } else { "\n" }))?;
    Ok(true)
}

/// Points plugins.txt at the launcher's links again: a line naming an original it made a link for ("Unofficial Skyrim
/// Special Edition Patch.esp", which Vortex switches back on) becomes the link's name, keeping its `*` and place.
/// For use while the game runs, when something else has rewritten the file. Returns (original, link) for each change.
pub fn point_at_links(game_dir: &Path, plugins_txt: &Path) -> std::io::Result<Vec<(String, String)>> {
    let mut done = Vec::new();
    for l in load(game_dir).links {
        let (Some(from), Some(to)) = (l.from.strip_prefix("Data/"), l.to.strip_prefix("Data/")) else { continue };
        if is_plugin(to) && game_dir.join(&l.to).is_file() && rename_in(plugins_txt, from, to)? {
            done.push((from.to_string(), to.to_string()));
        }
    }
    Ok(done)
}

/// A plugin the SkyMP client can't load as it is: its name has spaces, or it names such a plugin as a master
/// (USSEP, and MoreCraftableEquipment_USSEP.esp which needs it). These get a link or a rewritten "-AD" copy too.
fn client_cannot_load(data: &Path, name: &str) -> bool {
    // Kill switch: AD_ALIAS_ONLY_SERVER=1 restores the old behaviour (only the server's .esl files).
    if std::env::var_os("AD_ALIAS_ONLY_SERVER").is_some_and(|v| v == "1") {
        return false;
    }
    !client_can_load_name(name) || crate::loadorder::masters(&data.join(name)).unwrap_or_default().iter().any(|m| !client_can_load_name(m))
}

/// Gives each .esl in `full` (from `light_as_full`: the game's own light
/// plugins that the server runs as full ones, _ResourcePack.esl) its
/// "<stem>.esm" copy with the light flag cleared, the same bytes as the
/// server's, and points plugins.txt and loadorder.txt at it. Links whose
/// original is gone are removed. A "<stem>.esm" in Data that isn't the
/// launcher's own copy stops it: two files would load under one name.
/// Returns the plugins now running under another name, (original, alias).
/// Other plugins the client can't load (a spaced name, or a spaced master) are linked or rewritten the same way
/// (`client_cannot_load`); the rest are not renamed or rewritten.
pub fn ensure_full(game_dir: &Path, plugins_txt: Option<&Path>, full: &[String]) -> std::io::Result<Vec<(String, String)>> {
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
            if is_plugin(&n) && (full_esl(&n, full) || client_cannot_load(&data, &n)) && e.path().is_file() && !rec.links.iter().any(|l| l.to.eq_ignore_ascii_case(&format!("Data/{n}"))) {
                let rewritten = needs_rewrite(&data, &e.path(), full);
                if !client_can_load_name(&n) || rewritten {
                    plugins.push((n, rewritten));
                }
            }
        }
    }
    plugins.sort();
    for (p, rewritten) in &plugins {
        let alias = run_name_full(p, *rewritten, full);
        let ours = rec.links.iter().any(|l| l.to.eq_ignore_ascii_case(&format!("Data/{alias}")) && l.from.eq_ignore_ascii_case(&format!("Data/{p}")));
        if full_esl(p, full) && *rewritten && data.join(&alias).exists() && !ours {
            return Err(std::io::Error::other(format!("{p} would run as {alias}, but a different {alias} is already in Data")));
        }
    }
    // What each original maps to now. A link made under an older naming rule
    // ("JK-s-Skyrim.esp" before apostrophes were dropped) goes, and the load
    // order moves to the new name.
    let mut now: Vec<(String, String)> = Vec::new();
    for (p, rewritten) in &plugins {
        let alias = run_name_full(p, *rewritten, full);
        now.push((format!("Data/{p}"), format!("Data/{alias}")));
        for (f, t) in companions(&data, p, &alias).into_iter().filter(|(f, t)| !f.eq_ignore_ascii_case(t)) {
            now.push((format!("Data/{f}"), format!("Data/{t}")));
        }
    }
    let mut stale = Vec::new();
    rec.links.retain(|l| {
        let Some((_, to)) = now.iter().find(|(f, _)| f.eq_ignore_ascii_case(&l.from)) else { return true };
        if to.eq_ignore_ascii_case(&l.to) {
            return true;
        }
        stale.push((l.clone(), to.clone()));
        false
    });
    for (l, to) in &stale {
        if !now.iter().any(|(_, t)| t.eq_ignore_ascii_case(&l.to)) {
            let _ = std::fs::remove_file(game_dir.join(&l.to));
        }
        if let (Some(old), Some(new), Some(txt)) = (l.to.strip_prefix("Data/"), to.strip_prefix("Data/"), plugins_txt) {
            if is_plugin(old) {
                rename_in(txt, old, new)?;
                rename_in(&txt.with_file_name("loadorder.txt"), old, new)?;
            }
        }
    }
    let mut out = Vec::new();
    let done = (|| -> std::io::Result<()> {
    for (p, rewritten) in plugins {
        let alias = run_name_full(&p, rewritten, full);
        if rewritten {
            rewrite(&data.join(&p), &data.join(&alias), full)?;
        }
        let mut pairs = if rewritten { Vec::new() } else { vec![(p.clone(), alias.clone())] };
        // An .esl run as "<stem>.esm" keeps its archives' names as they are.
        pairs.extend(companions(&data, &p, &alias).into_iter().filter(|(f, t)| !f.eq_ignore_ascii_case(t)));
        let a = Alias { from: format!("Data/{p}"), to: format!("Data/{alias}") };
        if rewritten && !rec.links.contains(&a) {
            rec.links.push(a);
        }
        for (from, to) in pairs {
            let a = Alias { from: format!("Data/{from}"), to: format!("Data/{to}") };
            link(&data.join(&from), &data.join(&to), rec.links.contains(&a))?;
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
    Ok(())
    })();
    // Save even when a later plugin failed: the copies and links made before it are the launcher's,
    // and without the record the next run calls them someone else's file and refuses to go on.
    // AD_ALIASES_SAVE_ON_ERROR=0 gives the old behaviour (record saved only after a clean run).
    match done {
        Ok(()) => save(game_dir, &rec),
        Err(e) => {
            if std::env::var_os("AD_ALIASES_SAVE_ON_ERROR").is_none_or(|v| v != "0") {
                eprintln!("aliases: stopped part-way ({e}); the {} link(s) made so far are kept in aliases.json", rec.links.len());
                save(game_dir, &rec);
            }
            return Err(e);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {

    #[test]
    fn point_at_links_turns_a_switched_on_original_back_into_the_link() {
        let d = std::env::temp_dir().join(format!("ad-pointlinks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("Data")).unwrap();
        std::fs::write(d.join("Data/Unofficial-Skyrim-Special-Edition-Patch.esp"), b"x").unwrap();
        save(&d, &Record { links: vec![Alias { from: "Data/Unofficial Skyrim Special Edition Patch.esp".into(), to: "Data/Unofficial-Skyrim-Special-Edition-Patch.esp".into() }] });
        let txt = d.join("plugins.txt");
        std::fs::write(&txt, "# x\r\n*AetherialDawn.esp\r\n*Unofficial Skyrim Special Edition Patch.esp\r\n*RaceMenu.esp\r\n").unwrap();
        let done = point_at_links(&d, &txt).unwrap();
        assert_eq!(done, vec![("Unofficial Skyrim Special Edition Patch.esp".to_string(), "Unofficial-Skyrim-Special-Edition-Patch.esp".to_string())]);
        assert_eq!(std::fs::read_to_string(&txt).unwrap(), "# x\r\n*AetherialDawn.esp\r\n*Unofficial-Skyrim-Special-Edition-Patch.esp\r\n*RaceMenu.esp\r\n");
        assert!(point_at_links(&d, &txt).unwrap().is_empty(), "already pointing at the link");
        let _ = std::fs::remove_dir_all(&d);
    }
    use super::*;

    #[test]
    fn a_spaced_plugin_and_the_plugin_that_needs_it_run_under_names_the_client_can_load() {
        let g = tempfile::tempdir().unwrap();
        let data = g.path().join("Data");
        std::fs::create_dir_all(&data).unwrap();
        let usse = "Unofficial Skyrim Special Edition Patch.esp";
        std::fs::write(data.join(usse), plugin_with_masters(&["Skyrim.esm"])).unwrap();
        std::fs::write(data.join("MoreCraftableEquipment_USSEP.esp"), plugin_with_masters(&["Skyrim.esm", usse])).unwrap();
        std::fs::write(data.join("SkyUI_SE.esp"), plugin_with_masters(&["Skyrim.esm"])).unwrap();
        let txt = g.path().join("plugins.txt");
        std::fs::write(&txt, format!("*{usse}\r\n*MoreCraftableEquipment_USSEP.esp\r\n*SkyUI_SE.esp\r\n")).unwrap();
        let got = ensure_full(g.path(), Some(&txt), &[]).unwrap();
        assert_eq!(got, vec![
            ("MoreCraftableEquipment_USSEP.esp".to_string(), "MoreCraftableEquipment_USSEP-AD.esp".to_string()),
            (usse.to_string(), "Unofficial-Skyrim-Special-Edition-Patch.esp".to_string()),
        ]);
        assert_eq!(std::fs::read_to_string(&txt).unwrap(), "*Unofficial-Skyrim-Special-Edition-Patch.esp\r\n*MoreCraftableEquipment_USSEP-AD.esp\r\n*SkyUI_SE.esp\r\n");
        let masters = crate::loadorder::masters(&data.join("MoreCraftableEquipment_USSEP-AD.esp")).unwrap();
        assert_eq!(masters, vec!["Skyrim.esm".to_string(), "Unofficial-Skyrim-Special-Edition-Patch.esp".to_string()]);
        assert!(!data.join("SkyUI_SE-AD.esp").exists());
        // the original (Vortex's) is untouched, and a second run changes nothing
        assert!(data.join(usse).is_file() && data.join("MoreCraftableEquipment_USSEP.esp").is_file());
        assert!(ensure_full(g.path(), Some(&txt), &[]).unwrap().len() == 2);
    }

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
    fn a_dash_named_file_the_launcher_did_not_make_is_not_deleted() {
        let t = tempfile::tempdir().unwrap();
        let (from, to) = (t.path().join("A B.esp"), t.path().join("A-B.esp"));
        std::fs::write(&from, b"original").unwrap();
        std::fs::write(&to, b"players own file").unwrap();
        assert!(link(&from, &to, false).is_err());
        assert_eq!(std::fs::read(&to).unwrap(), b"players own file");
        // One the launcher made is refreshed.
        link(&from, &to, true).unwrap();
        assert_eq!(std::fs::read(&to).unwrap(), b"original");
    }

    #[test]
    fn only_the_listed_light_plugins_run_as_full_masters() {
        let flags = |mut b: Vec<u8>, f: u32| {
            b[8..12].copy_from_slice(&f.to_le_bytes());
            b
        };
        let flag = |p: &Path| {
            let b = std::fs::read(p).unwrap();
            u32::from_le_bytes([b[8], b[9], b[10], b[11]])
        };
        let t = tempfile::tempdir().unwrap();
        let g = t.path().join("game");
        let data = g.join("Data");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("Skyrim.esm"), flags(plugin_with_masters(&[]), 0x1)).unwrap();
        std::fs::write(data.join("_ResourcePack.esl"), flags(plugin_with_masters(&["Skyrim.esm"]), 0x201)).unwrap();
        std::fs::write(data.join("_ResourcePack.bsa"), b"bsa").unwrap();
        // An .esl the list doesn't name stays light; a plugin with a name the client
        // can't load gets its dash-named link too (USSEP, 2026-10-14).
        std::fs::write(data.join("Other.esl"), flags(plugin_with_masters(&["Skyrim.esm"]), 0x200)).unwrap();
        std::fs::write(data.join("Some Mod.esp"), plugin_with_masters(&["Skyrim.esm"])).unwrap();
        let server: Vec<String> = ["Skyrim.esm", "_ResourcePack.esm"].iter().map(|s| s.to_string()).collect();
        let full = light_as_full(&g, &server);
        assert_eq!(full, ["_ResourcePack.esl"]);

        let txt = g.join("plugins.txt");
        std::fs::write(&txt, "*_ResourcePack.esl\n*Some Mod.esp\n").unwrap();
        let before = std::fs::read(data.join("_ResourcePack.esl")).unwrap();
        let got = ensure_full(&g, Some(&txt), &full).unwrap();
        assert_eq!(got, [("Some Mod.esp".to_string(), "Some-Mod.esp".to_string()), ("_ResourcePack.esl".to_string(), "_ResourcePack.esm".to_string())]);
        // Light cleared, master set; the original and archive are untouched.
        assert_eq!(flag(&data.join("_ResourcePack.esm")), 0x1);
        assert_eq!(flag(&data.join("Other.esl")), 0x200);
        assert_eq!(std::fs::read(data.join("_ResourcePack.esl")).unwrap(), before);
        assert_eq!(std::fs::read_to_string(&txt).unwrap(), "*_ResourcePack.esm\n*Some-Mod.esp\n");
        assert!(data.join("Some-Mod.esp").exists() && !data.join("Some Mod-AD.esp").exists(), "a spaced plugin is linked, not rewritten");
        assert_eq!(run_name_full("_ResourcePack.esl", true, &full), "_ResourcePack.esm");
        // Again: the copy is the launcher's own, so nothing changes.
        assert_eq!(light_as_full(&g, &server), full);
        ensure_full(&g, Some(&txt), &full).unwrap();
        assert_eq!(std::fs::read_to_string(&txt).unwrap(), "*_ResourcePack.esm\n*Some-Mod.esp\n");
        // With no list, nothing is converted.
        assert_eq!(ensure_full(&g, Some(&txt), &[]).unwrap(), [("Some Mod.esp".to_string(), "Some-Mod.esp".to_string())], "only the spaced plugin is linked again");
    }

    #[test]
    fn a_different_esm_under_the_run_name_is_refused() {
        let t = tempfile::tempdir().unwrap();
        let g = t.path();
        let data = g.join("Data");
        std::fs::create_dir_all(&data).unwrap();
        let mut esl = plugin_with_masters(&[]);
        esl[8..12].copy_from_slice(&0x200u32.to_le_bytes());
        std::fs::write(data.join("_ResourcePack.esl"), esl).unwrap();
        std::fs::write(data.join("_ResourcePack.esm"), b"someone else's").unwrap();
        let full = vec!["_ResourcePack.esl".to_string()];
        assert!(ensure_full(g, None, &full).is_err());
        assert_eq!(std::fs::read(data.join("_ResourcePack.esm")).unwrap(), b"someone else's");
    }

    #[test]
    fn a_failed_later_plugin_does_not_make_the_earlier_copy_look_foreign() {
        let t = tempfile::tempdir().unwrap();
        let g = t.path();
        let data = g.join("Data");
        std::fs::create_dir_all(&data).unwrap();
        let mut esl = plugin_with_masters(&[]);
        esl[8..12].copy_from_slice(&0x200u32.to_le_bytes());
        std::fs::write(data.join("_ResourcePack.esl"), esl).unwrap();
        std::fs::write(data.join("zed mod.esp"), plugin_with_masters(&["Skyrim.esm"])).unwrap();
        // The player's own dash-named file: zed mod is refused, after _ResourcePack was already converted.
        std::fs::write(data.join("zed-mod.esp"), b"players own file").unwrap();
        let full = vec!["_ResourcePack.esl".to_string()];
        assert!(ensure_full(g, None, &full).is_err());
        assert!(data.join("_ResourcePack.esm").exists());
        std::fs::remove_file(data.join("zed-mod.esp")).unwrap();
        // The earlier copy is the launcher's own: the next run must go through, not call it someone else's.
        let got = ensure_full(g, None, &full).expect("the launcher's own copy is still its own");
        assert_eq!(got.len(), 2);
    }

    #[test]
    fn creation_club_names_are_told_apart_by_shape() {
        for n in ["Skyrim.esm", "_ResourcePack.esl", "ccBGSSSE001-Fish.esm", "ccKRTSSE001_Altar.esp"] {
            assert!(shipped_with_game(n), "{n}");
        }
        for n in ["ccBGSSSE001-Fish - Patch.esp", "Other.esm"] {
            assert!(!shipped_with_game(n), "{n}");
        }
    }
}
