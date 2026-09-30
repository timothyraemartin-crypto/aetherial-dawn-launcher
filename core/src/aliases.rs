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

/// The canonical form of a plugin for the server and every PC: the name it
/// runs under, and its bytes with renamed masters (None when they're
/// unchanged). The launcher makes the same bytes on each PC, so the server's
/// copy and the players' copies are identical.
pub fn canonical(data: &Path, plugin: &str, bytes: &[u8]) -> (String, Option<Vec<u8>>) {
    canonical_full(data, plugin, bytes, &[])
}

/// `canonical`, with the .esl files in `full` run as full plugins.
pub fn canonical_full(data: &Path, plugin: &str, bytes: &[u8], full: &[String]) -> (String, Option<Vec<u8>>) {
    let new = canonical_bytes_full(data, plugin, bytes, full);
    (run_name_full(plugin, new.is_some(), full), new)
}

/// One plugin as `canonicalize_dir` wrote it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CanonicalPlugin {
    /// The name in the mod's download ("JK's Skyrim.esp").
    pub original: String,
    /// The name the server and every PC load ("JKs-Skyrim.esp").
    pub name: String,
    /// Whether the file was changed: masters renamed, or the ESL flag cleared.
    pub rewritten: bool,
    /// sha256 of the written plugin.
    pub sha256: String,
    /// Archives, ini and string files written under the new name
    /// (paths relative to Data).
    pub companions: Vec<String>,
}

/// The server's side of the canonical rename (SERVER-PLUGINS.md): every
/// plugin at the top of `data` is written to `out` under the name it runs
/// under on players' PCs, with the same bytes the launcher makes there, and
/// its archives, ini and string files follow its name. Returns what was
/// written, in name order.
pub fn canonicalize_dir(data: &Path, out: &Path) -> std::io::Result<Vec<CanonicalPlugin>> {
    canonicalize_dir_full(data, out, &[])
}

/// `canonicalize_dir`, with the .esl files in `full` run as full plugins
/// ("<stem>.esm"). A different "<stem>.esm" already there is refused.
pub fn canonicalize_dir_full(data: &Path, out: &Path, full: &[String]) -> std::io::Result<Vec<CanonicalPlugin>> {
    use sha2::{Digest, Sha256};
    std::fs::create_dir_all(out)?;
    let mut names: Vec<String> = std::fs::read_dir(data)?
        .flatten()
        .filter(|e| e.path().is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| is_plugin(n))
        .collect();
    names.sort_by_key(|n| n.to_ascii_lowercase());
    // Two downloads that map to one name would overwrite each other.
    let mut seen: Vec<(String, String)> = Vec::new();
    for n in &names {
        let bytes = head(&data.join(n)).unwrap_or_default();
        let name = run_name_full(n, canonical_bytes_full(data, n, &bytes, full).is_some(), full).to_ascii_lowercase();
        if let Some((other, _)) = seen.iter().find(|(_, c)| c == &name) {
            return Err(std::io::Error::other(format!("{other} and {n} would both load as {name}")));
        }
        seen.push((n.clone(), name));
    }
    let mut done = Vec::new();
    for n in names {
        let bytes = std::fs::read(data.join(&n))?;
        let (name, new) = canonical_full(data, &n, &bytes, full);
        let body = new.as_deref().unwrap_or(&bytes);
        std::fs::write(out.join(&name), body)?;
        let mut companions_out = Vec::new();
        for (from, to) in companions(data, &n, &name) {
            let dest = out.join(&to);
            if let Some(p) = dest.parent() {
                std::fs::create_dir_all(p)?;
            }
            std::fs::copy(data.join(&from), dest)?;
            companions_out.push(to);
        }
        done.push(CanonicalPlugin { original: n, name, rewritten: new.is_some(), sha256: hex::encode(Sha256::digest(body)), companions: companions_out });
    }
    Ok(done)
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
    ensure_full(game_dir, plugins_txt, &[])
}

/// `ensure`, with the .esl files in `full` (from `light_as_full`) run as
/// full plugins under "<stem>.esm". A "<stem>.esm" in Data that isn't the
/// launcher's own copy stops it: two files would load under one name.
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
            if is_plugin(&n) && e.path().is_file() && !rec.links.iter().any(|l| l.to.eq_ignore_ascii_case(&format!("Data/{n}"))) {
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
    fn jks_plugins_with_apostrophes_run_as_dashed_names() {
        let t = tempfile::tempdir().unwrap();
        let g = t.path();
        let data = g.join("Data");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("Skyrim.esm"), plugin_with_masters(&[])).unwrap();
        std::fs::write(data.join("JK's Skyrim.esp"), plugin_with_masters(&["Skyrim.esm"])).unwrap();
        std::fs::write(data.join("JK's Skyrim.bsa"), b"bsa").unwrap();
        std::fs::write(data.join("JK's Whiterun Outskirts.esp"), plugin_with_masters(&["Skyrim.esm", "JK's Skyrim.esp"])).unwrap();
        // A patch with a plain name whose master has an apostrophe.
        std::fs::write(data.join("RSChildren_JKsSkyrim_Patch.esp"), plugin_with_masters(&["Skyrim.esm", "JK's Skyrim.esp"])).unwrap();
        let txt = g.join("plugins.txt");
        std::fs::write(&txt, "*JK's Skyrim.esp\r\n*JK's Whiterun Outskirts.esp\r\n*RSChildren_JKsSkyrim_Patch.esp\r\n").unwrap();
        ensure(g, Some(&txt)).unwrap();
        assert_eq!(std::fs::read_to_string(&txt).unwrap(), "*JKs-Skyrim.esp\r\n*JKs-Whiterun-Outskirts.esp\r\n*RSChildren_JKsSkyrim_Patch-AD.esp\r\n");
        let m = |n: &str| crate::loadorder::masters(&data.join(n)).unwrap();
        assert_eq!(m("JKs-Whiterun-Outskirts.esp"), ["Skyrim.esm", "JKs-Skyrim.esp"]);
        assert_eq!(m("RSChildren_JKsSkyrim_Patch-AD.esp"), ["Skyrim.esm", "JKs-Skyrim.esp"]);
        assert!(data.join("JKs-Skyrim.bsa").is_file());
        assert_eq!(m("JK's Whiterun Outskirts.esp"), ["Skyrim.esm", "JK's Skyrim.esp"], "the original stays as it was");
        assert_eq!(run_as(g, "JK's Skyrim.esp"), "JKs-Skyrim.esp");
        assert!(crate::loadorder::client_can_load_name(&run_as(g, "JK's Whiterun Outskirts.esp")));
        // Again: nothing changes.
        ensure(g, Some(&txt)).unwrap();
        assert_eq!(std::fs::read_to_string(&txt).unwrap(), "*JKs-Skyrim.esp\r\n*JKs-Whiterun-Outskirts.esp\r\n*RSChildren_JKsSkyrim_Patch-AD.esp\r\n");
    }

    #[test]
    fn the_server_gets_the_same_bytes_as_every_pc() {
        let t = tempfile::tempdir().unwrap();
        let (g, out) = (t.path().join("game"), t.path().join("server"));
        let data = g.join("Data");
        std::fs::create_dir_all(data.join("Strings")).unwrap();
        std::fs::write(data.join("Skyrim.esm"), plugin_with_masters(&[])).unwrap();
        std::fs::write(data.join("JKs Skyrim.esp"), plugin_with_masters(&["Skyrim.esm"])).unwrap();
        std::fs::write(data.join("JKs Skyrim.bsa"), b"bsa").unwrap();
        std::fs::write(data.join("CWE - JK - United.esp"), plugin_with_masters(&["Skyrim.esm", "JKs Skyrim.esp"])).unwrap();
        std::fs::write(data.join("Cloaks&Capes.esp"), plugin_with_masters(&["Skyrim.esm"])).unwrap();
        std::fs::write(data.join("Strings/Cloaks&Capes_english.strings"), b"s").unwrap();
        std::fs::write(data.join("RSChildren_JKsSkyrim_Patch.esp"), plugin_with_masters(&["Skyrim.esm", "JKs Skyrim.esp"])).unwrap();
        let got = canonicalize_dir(&data, &out).unwrap();
        let names: Vec<(&str, &str, bool)> = got.iter().map(|c| (c.original.as_str(), c.name.as_str(), c.rewritten)).collect();
        assert_eq!(
            names,
            [
                ("Cloaks&Capes.esp", "CloaksCapes.esp", false),
                ("CWE - JK - United.esp", "CWE-JK-United.esp", true),
                ("JKs Skyrim.esp", "JKs-Skyrim.esp", false),
                ("RSChildren_JKsSkyrim_Patch.esp", "RSChildren_JKsSkyrim_Patch-AD.esp", true),
                ("Skyrim.esm", "Skyrim.esm", false),
            ]
        );
        assert_eq!(got[2].companions, ["JKs-Skyrim.bsa"]);
        assert_eq!(got[0].companions, ["Strings/CloaksCapes_english.strings"]);
        // A player's launcher makes byte-identical files under the same names.
        let txt = g.join("plugins.txt");
        std::fs::write(&txt, "*JKs Skyrim.esp\n*CWE - JK - United.esp\n*Cloaks&Capes.esp\n*RSChildren_JKsSkyrim_Patch.esp\n").unwrap();
        ensure(&g, Some(&txt)).unwrap();
        for c in &got {
            assert_eq!(std::fs::read(data.join(&c.name)).unwrap(), std::fs::read(out.join(&c.name)).unwrap(), "{}", c.name);
            assert_eq!(run_as(&g, &c.original), c.name);
        }
    }

    #[test]
    fn links_under_the_old_rule_move_to_the_new_name() {
        let t = tempfile::tempdir().unwrap();
        let g = t.path();
        let data = g.join("Data");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("Skyrim.esm"), plugin_with_masters(&[])).unwrap();
        std::fs::write(data.join("JK's Skyrim.esp"), plugin_with_masters(&["Skyrim.esm"])).unwrap();
        std::fs::write(data.join("JK's Skyrim.bsa"), b"bsa").unwrap();
        std::fs::write(data.join("RSChildren_JKsSkyrim_Patch.esp"), plugin_with_masters(&["Skyrim.esm", "JK's Skyrim.esp"])).unwrap();
        // What 0.1.69/0.1.70 left: links under "JK-s-", a rewritten patch
        // naming "JK-s-Skyrim.esp", and the load order using them.
        std::fs::hard_link(data.join("JK's Skyrim.esp"), data.join("JK-s-Skyrim.esp")).unwrap();
        std::fs::hard_link(data.join("JK's Skyrim.bsa"), data.join("JK-s-Skyrim.bsa")).unwrap();
        std::fs::write(data.join("RSChildren_JKsSkyrim_Patch-AD.esp"), plugin_with_masters(&["Skyrim.esm", "JK-s-Skyrim.esp"])).unwrap();
        // Same time as its original, as rewrite() leaves it.
        let t0 = std::fs::metadata(data.join("RSChildren_JKsSkyrim_Patch.esp")).unwrap().modified().unwrap();
        std::fs::File::options().write(true).open(data.join("RSChildren_JKsSkyrim_Patch-AD.esp")).unwrap().set_modified(t0).unwrap();
        let rec = Record {
            links: vec![
                Alias { from: "Data/JK's Skyrim.esp".into(), to: "Data/JK-s-Skyrim.esp".into() },
                Alias { from: "Data/JK's Skyrim.bsa".into(), to: "Data/JK-s-Skyrim.bsa".into() },
                Alias { from: "Data/RSChildren_JKsSkyrim_Patch.esp".into(), to: "Data/RSChildren_JKsSkyrim_Patch-AD.esp".into() },
            ],
        };
        save(g, &rec);
        let txt = g.join("plugins.txt");
        std::fs::write(&txt, "*JK-s-Skyrim.esp\r\n*RSChildren_JKsSkyrim_Patch-AD.esp\r\n").unwrap();
        std::fs::write(g.join("loadorder.txt"), "Skyrim.esm\r\nJK-s-Skyrim.esp\r\nRSChildren_JKsSkyrim_Patch-AD.esp\r\n").unwrap();
        assert_eq!(crate::loadorder::masters(&data.join("RSChildren_JKsSkyrim_Patch-AD.esp")).unwrap(), ["Skyrim.esm", "JK-s-Skyrim.esp"]);

        ensure(g, Some(&txt)).unwrap();
        assert_eq!(std::fs::read_to_string(&txt).unwrap(), "*JKs-Skyrim.esp\r\n*RSChildren_JKsSkyrim_Patch-AD.esp\r\n");
        assert_eq!(std::fs::read_to_string(g.join("loadorder.txt")).unwrap(), "Skyrim.esm\r\nJKs-Skyrim.esp\r\nRSChildren_JKsSkyrim_Patch-AD.esp\r\n");
        assert!(!data.join("JK-s-Skyrim.esp").exists() && !data.join("JK-s-Skyrim.bsa").exists());
        assert!(data.join("JKs-Skyrim.esp").is_file() && data.join("JKs-Skyrim.bsa").is_file());
        assert_eq!(crate::loadorder::masters(&data.join("RSChildren_JKsSkyrim_Patch-AD.esp")).unwrap(), ["Skyrim.esm", "JKs-Skyrim.esp"], "the patch copy follows the new master name");
        assert!(links(g).iter().all(|l| !l.to.contains("JK-s-")));
        assert_eq!(std::fs::read(data.join("JK's Skyrim.bsa")).unwrap(), b"bsa", "the original is untouched");
    }

    #[test]
    fn esl_flagged_esps_load_as_full_plugins_on_the_server_and_every_pc() {
        // The ESL flag is bit 0x200 of the TES4 flags (byte 9).
        let esl = |ms: &[&str]| {
            let mut b = plugin_with_masters(ms);
            b[9] |= 0x02;
            b
        };
        let t = tempfile::tempdir().unwrap();
        let (g, out) = (t.path().join("game"), t.path().join("server"));
        let data = g.join("Data");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("Skyrim.esm"), plugin_with_masters(&[])).unwrap();
        let robes = esl(&["Skyrim.esm"]);
        std::fs::write(data.join("Kad_MoonMonkRobes.esp"), &robes).unwrap();
        std::fs::write(data.join("Kad_MoonMonkRobes.bsa"), b"bsa").unwrap();
        std::fs::write(data.join("Common Clothing Expanded.esp"), esl(&["Skyrim.esm"])).unwrap();
        std::fs::write(data.join("Robes Patch.esp"), plugin_with_masters(&["Skyrim.esm", "Kad_MoonMonkRobes.esp"])).unwrap();
        let hud = esl(&["Skyrim.esm"]);
        std::fs::write(data.join("TrueHUD.esl"), &hud).unwrap();
        let got = canonicalize_dir(&data, &out).unwrap();
        let names: Vec<(&str, &str, bool)> = got.iter().map(|c| (c.original.as_str(), c.name.as_str(), c.rewritten)).collect();
        assert_eq!(
            names,
            [
                ("Common Clothing Expanded.esp", "Common-Clothing-Expanded.esp", true),
                ("Kad_MoonMonkRobes.esp", "Kad_MoonMonkRobes-AD.esp", true),
                ("Robes Patch.esp", "Robes-Patch.esp", true),
                ("Skyrim.esm", "Skyrim.esm", false),
                ("TrueHUD.esl", "TrueHUD.esl", false),
            ]
        );
        // Exactly the original with the flag cleared; the same bytes on
        // every build, since nothing else changes.
        let mut want = robes.clone();
        want[9] &= !0x02;
        assert_eq!(std::fs::read(out.join("Kad_MoonMonkRobes-AD.esp")).unwrap(), want);
        assert_eq!(got[1].companions, ["Kad_MoonMonkRobes-AD.bsa"]);
        assert_eq!(crate::loadorder::masters(&out.join("Robes-Patch.esp")).unwrap(), ["Skyrim.esm", "Kad_MoonMonkRobes-AD.esp"]);
        // An .esl is light by its extension; it's left as it is.
        assert_eq!(std::fs::read(out.join("TrueHUD.esl")).unwrap(), hud);
        // A player's launcher makes byte-identical files under the same names,
        // and the original download is untouched.
        let txt = g.join("plugins.txt");
        std::fs::write(&txt, "*Kad_MoonMonkRobes.esp\n*Common Clothing Expanded.esp\n*Robes Patch.esp\n").unwrap();
        ensure(&g, Some(&txt)).unwrap();
        for c in &got {
            assert_eq!(std::fs::read(data.join(&c.name)).unwrap(), std::fs::read(out.join(&c.name)).unwrap(), "{}", c.name);
            assert_eq!(run_as(&g, &c.original), c.name);
        }
        assert_eq!(std::fs::read(data.join("Kad_MoonMonkRobes.esp")).unwrap(), robes);
        assert_eq!(std::fs::read_to_string(&txt).unwrap(), "*Kad_MoonMonkRobes-AD.esp\n*Common-Clothing-Expanded.esp\n*Robes-Patch.esp\n");
        // A second Play changes nothing.
        ensure(&g, Some(&txt)).unwrap();
        assert_eq!(std::fs::read(data.join("Kad_MoonMonkRobes-AD.esp")).unwrap(), want);
    }

    #[test]
    fn the_games_own_light_plugins_and_cut_headers_are_left_alone() {
        let esl = |ms: &[&str]| {
            let mut b = plugin_with_masters(ms);
            b[9] |= 0x02;
            b
        };
        let t = tempfile::tempdir().unwrap();
        let (g, out) = (t.path().join("game"), t.path().join("server"));
        let data = g.join("Data");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("Skyrim.esm"), plugin_with_masters(&[])).unwrap();
        let game_own = ["Dawnguard.esm", "ccBGSSSE001-Fish.esm", "ccBGSSSE025-AdvDSGS.esm", "ccKRTSSE001_Altar.esp"];
        for n in game_own {
            std::fs::write(data.join(n), esl(&["Skyrim.esm"])).unwrap();
        }
        std::fs::write(data.join("Other.esm"), esl(&["Skyrim.esm"])).unwrap();
        // A mod's own file named like Creation Club is still a mod.
        std::fs::write(data.join("ccBGSSSE001-Fish - Patch.esp"), esl(&["Skyrim.esm"])).unwrap();
        // The header says it's longer than the file: not read as flagged anywhere.
        let mut cut = esl(&["Skyrim.esm"]);
        cut.truncate(30);
        std::fs::write(data.join("Cut.esp"), &cut).unwrap();
        let got = canonicalize_dir(&data, &out).unwrap();
        let names: Vec<(&str, &str, bool)> = got.iter().map(|c| (c.original.as_str(), c.name.as_str(), c.rewritten)).collect();
        assert_eq!(
            names,
            [
                ("ccBGSSSE001-Fish - Patch.esp", "ccBGSSSE001-Fish-Patch.esp", true),
                ("ccBGSSSE001-Fish.esm", "ccBGSSSE001-Fish.esm", false),
                ("ccBGSSSE025-AdvDSGS.esm", "ccBGSSSE025-AdvDSGS.esm", false),
                ("ccKRTSSE001_Altar.esp", "ccKRTSSE001_Altar.esp", false),
                ("Cut.esp", "Cut.esp", false),
                ("Dawnguard.esm", "Dawnguard.esm", false),
                ("Other.esm", "Other-AD.esm", true),
                ("Skyrim.esm", "Skyrim.esm", false),
            ]
        );
        ensure(&g, None).unwrap();
        let mut made: Vec<String> = std::fs::read_dir(&data).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| !got.iter().any(|c| &c.original == n)).collect();
        made.sort();
        assert_eq!(made, ["Other-AD.esm", "ccBGSSSE001-Fish-Patch.esp"], "the PC makes the same copies as the server, and no others");
        for n in game_own {
            assert_eq!(run_as(&g, n), n);
        }
        assert_eq!(run_as(&g, "Cut.esp"), "Cut.esp");
    }

    #[test]
    fn listed_light_plugins_run_as_full_masters_on_the_server_and_every_pc() {
        // desync/esl-on-server.md: USSEP masters _ResourcePack.esl, COTN
        // needs Survival Mode; the server gives every plugin a full index.
        let flags = |mut b: Vec<u8>, f: u32| {
            b[8..12].copy_from_slice(&f.to_le_bytes());
            b
        };
        let flag = |p: &Path| {
            let b = std::fs::read(p).unwrap();
            u32::from_le_bytes([b[8], b[9], b[10], b[11]])
        };
        let t = tempfile::tempdir().unwrap();
        let (g, out) = (t.path().join("game"), t.path().join("server"));
        let data = g.join("Data");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("Skyrim.esm"), flags(plugin_with_masters(&[]), 0x1)).unwrap();
        std::fs::write(data.join("_ResourcePack.esl"), flags(plugin_with_masters(&["Skyrim.esm"]), 0x201)).unwrap();
        std::fs::write(data.join("_ResourcePack.bsa"), b"bsa").unwrap();
        std::fs::write(data.join("ccQDRSSE001-SurvivalMode.esl"), flags(plugin_with_masters(&["Skyrim.esm"]), 0x201)).unwrap();
        std::fs::write(data.join("Unofficial Skyrim Special Edition Patch.esp"), flags(plugin_with_masters(&["Skyrim.esm", "_ResourcePack.esl"]), 0x1)).unwrap();
        std::fs::write(data.join("COTN Patch.esp"), plugin_with_masters(&["Skyrim.esm", "ccQDRSSE001-SurvivalMode.esl"])).unwrap();
        // An .esl the list doesn't name stays light, as before.
        std::fs::write(data.join("Other.esl"), flags(plugin_with_masters(&["Skyrim.esm"]), 0x200)).unwrap();
        let server: Vec<String> = ["Skyrim.esm", "_ResourcePack.esm", "ccQDRSSE001-SurvivalMode.esm", "Unofficial-Skyrim-Special-Edition-Patch.esp", "COTN Patch-AD.esp"].iter().map(|s| s.to_string()).collect();
        let full = light_as_full(&g, &server);
        assert_eq!(full, ["_ResourcePack.esl", "ccQDRSSE001-SurvivalMode.esl"]);

        let got = canonicalize_dir_full(&data, &out, &full).unwrap();
        let names: Vec<(&str, &str, bool)> = got.iter().map(|c| (c.original.as_str(), c.name.as_str(), c.rewritten)).collect();
        assert_eq!(
            names,
            [
                ("_ResourcePack.esl", "_ResourcePack.esm", true),
                ("ccQDRSSE001-SurvivalMode.esl", "ccQDRSSE001-SurvivalMode.esm", true),
                ("COTN Patch.esp", "COTN-Patch.esp", true),
                ("Other.esl", "Other.esl", false),
                ("Skyrim.esm", "Skyrim.esm", false),
                ("Unofficial Skyrim Special Edition Patch.esp", "Unofficial-Skyrim-Special-Edition-Patch.esp", true),
            ]
        );
        // Light cleared, master set; the archive keeps its name.
        assert_eq!(flag(&out.join("_ResourcePack.esm")), 0x1);
        assert_eq!(flag(&out.join("ccQDRSSE001-SurvivalMode.esm")), 0x1);
        assert_eq!(flag(&out.join("Other.esl")), 0x200);
        assert_eq!(got[0].companions, ["_ResourcePack.bsa"]);
        let m = |d: &Path, n: &str| crate::loadorder::masters(&d.join(n)).unwrap();
        assert_eq!(m(&out, "Unofficial-Skyrim-Special-Edition-Patch.esp"), ["Skyrim.esm", "_ResourcePack.esm"]);
        assert_eq!(m(&out, "COTN-Patch.esp"), ["Skyrim.esm", "ccQDRSSE001-SurvivalMode.esm"]);

        // A PC makes the same bytes under the same names, and plugins.txt
        // names the copies; the .esl originals are left untouched.
        let txt = g.join("plugins.txt");
        std::fs::write(&txt, "*_ResourcePack.esl\n*Unofficial Skyrim Special Edition Patch.esp\n*COTN Patch.esp\n").unwrap();
        let before = std::fs::read(data.join("_ResourcePack.esl")).unwrap();
        ensure_full(&g, Some(&txt), &full).unwrap();
        for c in &got {
            assert_eq!(std::fs::read(data.join(&c.name)).unwrap(), std::fs::read(out.join(&c.name)).unwrap(), "{}", c.name);
            assert_eq!(run_as(&g, &c.original), c.name);
        }
        assert_eq!(std::fs::read(data.join("_ResourcePack.esl")).unwrap(), before);
        assert_eq!(std::fs::read_to_string(&txt).unwrap(), "*_ResourcePack.esm\n*Unofficial-Skyrim-Special-Edition-Patch.esp\n*COTN-Patch.esp\n");
        // Again: the copies are the launcher's own, so the list still maps,
        // and nothing changes.
        assert_eq!(light_as_full(&g, &server), full);
        ensure_full(&g, Some(&txt), &full).unwrap();
        assert!(!data.join("_ResourcePack-AD.esm").exists());
        assert_eq!(std::fs::read_to_string(&txt).unwrap(), "*_ResourcePack.esm\n*Unofficial-Skyrim-Special-Edition-Patch.esp\n*COTN-Patch.esp\n");
        // With no list, nothing is converted (the old behaviour).
        assert_eq!(canonical(&data, "_ResourcePack.esl", &before).0, "_ResourcePack.esl");
    }

    #[test]
    fn a_different_esm_under_the_run_name_is_refused() {
        let t = tempfile::tempdir().unwrap();
        let g = t.path();
        let data = g.join("Data");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("Curios.esl"), plugin_with_masters(&["Skyrim.esm"])).unwrap();
        std::fs::write(data.join("Curios.esm"), plugin_with_masters(&["Skyrim.esm", "Other.esm"])).unwrap();
        // The list can't pick it up (Data has a real Curios.esm)...
        assert!(light_as_full(g, &["Curios.esm".to_string()]).is_empty());
        // ...and naming it anyway stops both sides.
        let full = vec!["Curios.esl".to_string()];
        assert!(ensure_full(g, None, &full).unwrap_err().to_string().contains("a different Curios.esm"));
        assert!(canonicalize_dir_full(&data, &t.path().join("out"), &full).unwrap_err().to_string().contains("would both load as curios.esm"));
    }

    #[test]
    fn creation_club_names_are_told_apart_by_shape() {
        for n in ["ccBGSSSE001-Fish.esm", "ccKRTSSE001_Altar.esl", "CCQDRSSE001-SurvivalMode.ESL", "ccBGSSSE064-Some_Name.esp", "_ResourcePack.esl", "Skyrim.esm"] {
            assert!(shipped_with_game(n), "{n}");
        }
        for n in ["ccBGSSSE001-Fish - Patch.esp", "ccBGSSSE001-Fish.bsa", "ccBGSSSE001-.esp", "ccBGSSE001-Fish.esp", "ccBGSSSE01-Fish.esp", "ccBGSSSE001Fish.esp", "cc.esp", "Other.esp"] {
            assert!(!shipped_with_game(n), "{n}");
        }
    }

    #[test]
    fn two_downloads_with_one_canonical_name_are_refused() {
        let t = tempfile::tempdir().unwrap();
        let data = t.path().join("Data");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("JK's Skyrim.esp"), plugin_with_masters(&[])).unwrap();
        std::fs::write(data.join("JKs Skyrim.esp"), plugin_with_masters(&[])).unwrap();
        let e = canonicalize_dir(&data, &t.path().join("out")).unwrap_err().to_string();
        assert!(e.contains("jks-skyrim.esp"), "{e}");
    }

    #[test]
    fn a_patch_of_a_rewritten_patch_names_its_ad_copy() {
        let t = tempfile::tempdir().unwrap();
        let (g, out) = (t.path().join("game"), t.path().join("server"));
        let data = g.join("Data");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("Skyrim.esm"), plugin_with_masters(&[])).unwrap();
        // A (spaced) <- B (plain name, rewritten as B-AD) <- C (plain name).
        std::fs::write(data.join("JKs Skyrim.esp"), plugin_with_masters(&["Skyrim.esm"])).unwrap();
        std::fs::write(data.join("CWE_JK_United.esp"), plugin_with_masters(&["Skyrim.esm", "JKs Skyrim.esp"])).unwrap();
        std::fs::write(data.join("CWE_JK_Relighting.esp"), plugin_with_masters(&["Skyrim.esm", "CWE_JK_United.esp"])).unwrap();
        let txt = g.join("plugins.txt");
        std::fs::write(&txt, "*JKs Skyrim.esp\n*CWE_JK_United.esp\n*CWE_JK_Relighting.esp\n").unwrap();
        ensure(&g, Some(&txt)).unwrap();
        assert_eq!(std::fs::read_to_string(&txt).unwrap(), "*JKs-Skyrim.esp\n*CWE_JK_United-AD.esp\n*CWE_JK_Relighting-AD.esp\n");
        let m = |n: &str| crate::loadorder::masters(&data.join(n)).unwrap();
        assert_eq!(m("CWE_JK_United-AD.esp"), ["Skyrim.esm", "JKs-Skyrim.esp"]);
        assert_eq!(m("CWE_JK_Relighting-AD.esp"), ["Skyrim.esm", "CWE_JK_United-AD.esp"]);
        assert_eq!(m("CWE_JK_Relighting.esp"), ["Skyrim.esm", "CWE_JK_United.esp"], "the original stays as it was");
        // Again: nothing changes.
        ensure(&g, Some(&txt)).unwrap();
        assert!(!data.join("CWE_JK_Relighting-AD-AD.esp").exists());
        // The server's tool, run on the downloads, makes the same files.
        let src = t.path().join("downloads");
        std::fs::create_dir_all(&src).unwrap();
        for n in ["Skyrim.esm", "JKs Skyrim.esp", "CWE_JK_United.esp", "CWE_JK_Relighting.esp"] {
            std::fs::copy(data.join(n), src.join(n)).unwrap();
        }
        let got = canonicalize_dir(&src, &out).unwrap();
        for c in &got {
            assert_eq!(std::fs::read(data.join(&c.name)).unwrap(), std::fs::read(out.join(&c.name)).unwrap(), "{}", c.name);
        }
        assert!(got.iter().any(|c| c.original == "CWE_JK_Relighting.esp" && c.name == "CWE_JK_Relighting-AD.esp" && c.rewritten));
    }

    #[test]
    fn names() {
        assert_eq!(alias_name("Unofficial Skyrim Special Edition Patch.esp"), "Unofficial-Skyrim-Special-Edition-Patch.esp");
        assert_eq!(alias_name("A  +  B .esp"), "A-B.esp");
        assert!(client_can_load_name(&alias_name("Élan's Mod (v2).esm")));
        // The names SERVER-PLUGINS.md lists.
        assert_eq!(alias_name("The Great City of Morthal.esp"), "The-Great-City-of-Morthal.esp");
        assert_eq!(alias_name("Cloaks&Capes.esp"), "CloaksCapes.esp");
        assert_eq!(alias_name("COTN - Morthal.esp"), "COTN-Morthal.esp");
        assert_eq!(alias_name("OCW_Obscure's_CollegeofWinterhold.esp"), "OCW_Obscures_CollegeofWinterhold.esp");
        assert_eq!(alias_name("The Great Cities of JK's North - Patch.esp"), "The-Great-Cities-of-JKs-North-Patch.esp");
        assert_eq!(alias_name("Riften Expansion - JK’s Skyrim Patch.esp"), "Riften-Expansion-JKs-Skyrim-Patch.esp");
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
