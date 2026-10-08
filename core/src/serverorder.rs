//! The server's plugin order on this PC (SERVER-PLUGINS.md, World sync's
//! review 2026-09-27). SkyMP's client compares its plugin at each position
//! with the server's by name, size and crc32 (loadOrderVerificationService),
//! and items are raw form ids that include the plugin's position. So once
//! the server loads more than the five base masters, every PC must load
//! exactly the server's plugins, in its order, and nothing between them:
//! - Creation Club masters listed in Skyrim.ccc load right after the base
//!   masters, so for a session the launcher empties Skyrim.ccc (the original
//!   is kept beside it and put back when the game closes);
//! - plugins.txt lists the server's plugins first, in order, under the names
//!   they run as here, then client-only plugins (RaceMenu's), and nothing else
//!   switched on;
//! - before the game starts the launcher predicts the game's plugin list and
//!   compares it with the server's the way SkyMP will.

use std::path::{Path, PathBuf};

use crate::loadorder::client_can_load_name;

pub const BASE: [&str; 5] = ["Skyrim.esm", "Update.esm", "Dawnguard.esm", "HearthFires.esm", "Dragonborn.esm"];
const CCC_BACKUP: &str = "Skyrim.ccc.aetherial-dawn-backup";

/// One plugin in the server's order.
#[derive(Debug, Clone, PartialEq)]
pub struct ServerPlugin {
    pub name: String,
    pub size: Option<u64>,
    pub sha256: Option<String>,
    pub crc32: Option<u32>,
}

/// A plugin name a PC can be told to load: a bare file name ending in
/// .esp, .esm or .esl, with no path separators, control characters or a
/// leading "*" or "#" (a served name becomes a plugins.txt line).
pub fn plain_plugin_name(n: &str) -> bool {
    let l = n.to_ascii_lowercase();
    !n.is_empty()
        && n == n.trim()
        && !n.chars().any(|c| c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
        && !n.starts_with('#')
        && n != "." && n != ".."
        && [".esp", ".esm", ".esl"].iter().any(|x| l.ends_with(x) && l.len() > x.len())
}

/// The server's order from its masters.json (list order is load order).
/// Only a list says an order: the object form comes back sorted, so it
/// gives no order at all.
pub fn server_order(masters: &serde_json::Value) -> Vec<ServerPlugin> {
    let list = masters.get("masters").or_else(|| masters.get("files")).unwrap_or(masters);
    let Some(a) = list.as_array() else { return Vec::new() };
    let out: Vec<ServerPlugin> = a.iter()
        .filter_map(|e| {
            let n = e.get("name").or_else(|| e.get("file")).or_else(|| e.get("path")).and_then(|n| n.as_str())?;
            // A master every PC runs as a converted copy is compared by the
            // converted bytes (canonical_*); the plain fields are the game's
            // own file then, which no PC loads under this name.
            let get = |k: &str| {
                let camel = k.split('_').enumerate().map(|(i, w)| if i == 0 { w.to_string() } else { w[..1].to_ascii_uppercase() + &w[1..] }).collect::<String>();
                e.get(k).or_else(|| e.get(camel.as_str()))
            };
            let canon = get("canonical_sha256").and_then(|s| s.as_str()).filter(|s| !s.is_empty());
            let pick = |k: &str| if canon.is_some() { get(&format!("canonical_{k}")) } else { e.get(k) };
            let crc32 = match pick("crc32") {
                Some(serde_json::Value::Number(n)) => n.as_u64().and_then(|v| u32::try_from(v).ok()),
                Some(serde_json::Value::String(s)) => u32::from_str_radix(s.trim_start_matches("0x"), 16).ok(),
                _ => None,
            };
            Some(ServerPlugin {
                name: n.rsplit(['/', '\\']).next().unwrap_or(n).to_string(),
                size: pick("size").and_then(|s| s.as_u64()),
                sha256: canon.or_else(|| e.get("sha256").and_then(|s| s.as_str())).map(|s| s.to_ascii_lowercase()),
                crc32,
            })
        })
        .collect();
    // A name that isn't a plain plugin file name, or that appears twice,
    // makes the whole list unusable: Play then refuses, as for a list with
    // no base masters, rather than write it into plugins.txt.
    let mut seen = std::collections::HashSet::new();
    if out.len() != a.len() || !out.iter().all(|p| plain_plugin_name(&p.name) && seen.insert(p.name.to_ascii_lowercase())) {
        return Vec::new();
    }
    out
}

/// The served masters.json for an export: the five base masters as the
/// current masters.json has them (name, size, sha256), then every master
/// past them and every lane plugin from the exporter's `export.json`, in
/// load-order `index`, each under its `run_name` with the canonical values
/// every PC's copy must match (`canonical_sha256`, `canonical_size` from
/// `canonical_bytes`, `canonical_crc32`). The export's plain `sha256` is the
/// exporting PC's original file and is left out. Refuses an export whose
/// indexes don't run on from the base masters with no gap or repeat.
pub fn masters_json(base: &[serde_json::Value], export: &serde_json::Value) -> Result<serde_json::Value, String> {
    let head = server_order(&serde_json::json!({ "masters": base }));
    if head.len() != BASE.len() || !valid_base(&head) {
        return Err("the base masters must be the five, in order, each with a size and sha256".into());
    }
    let mut rest: Vec<(u64, &serde_json::Value)> = Vec::new();
    let masters = export.get("masters").and_then(|m| m.as_array()).map(|a| a.iter().collect::<Vec<_>>()).unwrap_or_default();
    let files = export.get("files").and_then(|f| f.as_object()).map(|o| o.values().collect::<Vec<_>>()).unwrap_or_default();
    for e in masters.into_iter().chain(files) {
        let index = e.get("index").and_then(|i| i.as_u64()).ok_or("an export entry has no index")?;
        rest.push((index, e));
    }
    rest.sort_by_key(|(i, _)| *i);
    let mut out: Vec<serde_json::Value> = base.to_vec();
    for (k, (index, e)) in rest.into_iter().enumerate() {
        let want = (BASE.len() + k) as u64;
        let name = e.get("run_name").and_then(|n| n.as_str()).filter(|n| !n.is_empty()).ok_or(format!("the export entry at index {index} has no run_name"))?;
        if !plain_plugin_name(name) || BASE.iter().any(|b| b.eq_ignore_ascii_case(name)) || out.iter().any(|o| o["name"].as_str().is_some_and(|x| x.eq_ignore_ascii_case(name))) {
            return Err(format!("{name:?} at index {index} isn't a plain, new plugin name"));
        }
        if index != want {
            return Err(format!("{name} is at index {index}; the next index is {want}"));
        }
        let sha = e.get("canonical_sha256").and_then(|s| s.as_str()).filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or(format!("{name} has no canonical_sha256"))?;
        let bytes = e.get("canonical_bytes").and_then(|b| b.as_u64()).filter(|b| *b > 0).ok_or(format!("{name} has no canonical_bytes"))?;
        let crc = e.get("canonical_crc32").and_then(|c| c.as_u64()).and_then(|c| u32::try_from(c).ok()).ok_or(format!("{name} has no canonical_crc32"))?;
        // The plain fields carry the same values, so a launcher from before
        // canonical_* checks the converted copy too, never the original.
        let sha = sha.to_ascii_lowercase();
        out.push(serde_json::json!({ "name": name, "size": bytes, "crc32": crc, "sha256": sha, "canonical_sha256": sha, "canonical_size": bytes, "canonical_crc32": crc }));
    }
    Ok(serde_json::json!({ "masters": out }))
}

/// Whether the server loads more than the five base masters: only then do
/// Creation Club plugins and plugins.txt have to follow it exactly.
pub fn beyond_base(order: &[ServerPlugin]) -> bool {
    order.len() > BASE.len()
}

/// A Play decision needs an ordered server list and fingerprints for all five
/// base masters. The server may publish more plugins after those masters.
pub fn valid_base(order: &[ServerPlugin]) -> bool {
    order.len() >= BASE.len() && BASE.iter().zip(order).all(|(base, plugin)| {
        plugin.name.eq_ignore_ascii_case(base)
            && plugin.size.is_some_and(|size| size > 0)
            && plugin.sha256.as_ref().is_some_and(|sha| sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_hexdigit()))
    })
}

fn ccc_paths(game_dir: &Path) -> Vec<PathBuf> {
    [game_dir.join("Skyrim.ccc"), game_dir.join("Data").join("Skyrim.ccc")].into_iter().filter(|p| p.is_file()).collect()
}

/// Empties Skyrim.ccc for a session, keeping the original beside it. A
/// non-empty Skyrim.ccc is the real one (Steam may have put it back), so it
/// replaces any older backup; an empty one keeps the backup an unfinished
/// session left. Returns the files emptied.
pub fn hide_ccc(game_dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for p in ccc_paths(game_dir) {
        let backup = p.with_file_name(CCC_BACKUP);
        let real = std::fs::metadata(&p)?.len() > 0;
        if real || !backup.exists() {
            std::fs::copy(&p, &backup)?;
        }
        if real {
            // A new file, never a write through a link.
            std::fs::remove_file(&p)?;
            std::fs::write(&p, b"")?;
            out.push(p);
        }
    }
    Ok(out)
}

/// Puts back the Skyrim.ccc a session emptied. Only an empty (or missing)
/// Skyrim.ccc is replaced; a non-empty one is newer than the backup (Steam
/// restored it), so the stale backup goes instead. Returns whether it put
/// one back.
pub fn restore_ccc(game_dir: &Path) -> std::io::Result<bool> {
    let mut any = false;
    for dir in [game_dir.to_path_buf(), game_dir.join("Data")] {
        let backup = dir.join(CCC_BACKUP);
        if !backup.is_file() {
            continue;
        }
        let ccc = dir.join("Skyrim.ccc");
        if std::fs::metadata(&ccc).map(|m| m.len() == 0).unwrap_or(true) {
            std::fs::rename(&backup, &ccc)?;
            any = true;
        } else {
            std::fs::remove_file(&backup)?;
        }
    }
    Ok(any)
}

/// The TES4 record flags: (master, light).
fn flags(path: &Path) -> Option<(bool, bool)> {
    use std::io::Read;
    let mut h = [0u8; 12];
    std::fs::File::open(path).ok()?.read_exact(&mut h).ok()?;
    if &h[..4] != b"TES4" {
        return None;
    }
    let f = u32::from_le_bytes(h[8..12].try_into().ok()?);
    Some((f & 0x1 != 0, f & 0x200 != 0))
}

fn find(data: &Path, name: &str) -> Option<PathBuf> {
    let exact = data.join(name);
    if exact.is_file() {
        return Some(exact);
    }
    std::fs::read_dir(data).ok()?.flatten().find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(name)).map(|e| e.path())
}

/// The full (not light) plugins the game will load, in order, as SkyMP
/// lists them: the base masters, Skyrim.ccc's masters that are in Data, then
/// plugins.txt's active plugins with masters before the rest.
pub fn game_order(game_dir: &Path, plugins_txt: &Path) -> Vec<String> {
    let data = game_dir.join("Data");
    let mut out: Vec<String> = Vec::new();
    let has = |out: &Vec<String>, n: &str| out.iter().any(|o| o.eq_ignore_ascii_case(n));
    for b in BASE {
        if find(&data, b).is_some() {
            out.push(b.to_string());
        }
    }
    let full = |n: &str| -> Option<(bool, PathBuf)> {
        let p = find(&data, n)?;
        let (master, light) = flags(&p)?;
        let l = n.to_ascii_lowercase();
        if light || l.ends_with(".esl") {
            return None;
        }
        Some((master || l.ends_with(".esm"), p))
    };
    if let Some(ccc) = ccc_paths(game_dir).first() {
        for n in crate::loadorder::read_text(ccc).unwrap_or_default().lines().map(str::trim).filter(|l| !l.is_empty()) {
            if !has(&out, n) && full(n).is_some() {
                out.push(n.to_string());
            }
        }
    }
    let active: Vec<String> = crate::loadorder::read_text(plugins_txt)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.trim().strip_prefix('*'))
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .collect();
    let mut rest = Vec::new();
    for n in active {
        if has(&out, &n) || has(&rest, &n) {
            continue;
        }
        match full(&n) {
            Some((true, _)) => out.push(n),
            Some((false, _)) => rest.push(n),
            None => {}
        }
    }
    out.extend(rest);
    out
}

/// Standard CRC-32 (the one SkyMP's client computes).
pub fn crc32(path: &Path) -> Option<u32> {
    use std::io::Read;
    let table = crc_table();
    let mut f = std::fs::File::open(path).ok()?;
    let mut buf = vec![0u8; 1 << 20];
    let mut crc = 0xFFFF_FFFFu32;
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        crc = crc_update(&table, crc, &buf[..n]);
    }
    Some(!crc)
}

/// `crc32` of bytes in memory (a converted plugin's canonical bytes).
pub fn crc32_bytes(b: &[u8]) -> u32 {
    !crc_update(&crc_table(), 0xFFFF_FFFF, b)
}

fn crc_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    for (i, t) in table.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
        *t = c;
    }
    table
}

fn crc_update(table: &[u32; 256], mut crc: u32, bytes: &[u8]) -> u32 {
    for b in bytes {
        crc = table[((crc ^ *b as u32) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc
}

/// Where the game's list differs from the server's, by position: name,
/// size, and crc32 when the server gives it (sha256 is checked by the
/// masters check). A PC with more plugins than the server is fine, as it is
/// for SkyMP.
pub fn mismatches(game_dir: &Path, plugins_txt: &Path, server: &[ServerPlugin]) -> Vec<String> {
    let data = game_dir.join("Data");
    let mine = game_order(game_dir, plugins_txt);
    let mut out = Vec::new();
    for (i, s) in server.iter().enumerate() {
        let Some(m) = mine.get(i) else {
            out.push(format!("position {i}: the server loads {}, this PC loads nothing there", s.name));
            continue;
        };
        if !m.eq_ignore_ascii_case(&s.name) {
            out.push(format!("position {i}: the server loads {}, this PC loads {m}", s.name));
            continue;
        }
        let Some(p) = find(&data, m) else { continue };
        if let Some(size) = s.size {
            let have = std::fs::metadata(&p).map(|md| md.len()).unwrap_or(0);
            if have != size {
                out.push(format!("{m}: {have} bytes, the server's is {size}"));
                continue;
            }
        }
        if let Some(c) = s.crc32 {
            if crc32(&p) != Some(c) {
                out.push(format!("{m}: same size, different contents (crc32)"));
            }
        }
    }
    out
}

/// Rewrites plugins.txt so the server's plugins after the base masters come
/// first, switched on, in the server's order, then the plugins already on
/// that the client may load (RaceMenu's), then everything else switched off
/// (Vortex's spaced originals among them; the files stay in Data). A full
/// master the server doesn't load (a Creation Club .esm, an ESM-flagged
/// .esp) is switched off too: the game loads masters first, so it would
/// sit between the server's plugins. The old file is kept beside it.
/// Returns whether it changed.
pub fn set_exact(game_dir: &Path, plugins_txt: &Path, server: &[ServerPlugin]) -> std::io::Result<bool> {
    let data = game_dir.join("Data");
    let full_master = |n: &str| {
        let l = n.to_ascii_lowercase();
        find(&data, n).and_then(|p| flags(&p)).is_some_and(|(master, light)| !light && !l.ends_with(".esl") && (master || l.ends_with(".esm")))
    };
    let file = crate::loadorder::ListFile::read(plugins_txt)?;
    let text = file.as_ref().map_or("", |f| f.text.as_str());
    let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let name = |l: &str| l.trim().trim_start_matches('*').trim().to_string();
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let header: Vec<&str> = lines.iter().copied().filter(|l| l.trim_start().starts_with('#')).collect();
    let entries: Vec<&str> = lines.iter().copied().filter(|l| !l.trim_start().starts_with('#')).collect();
    let is_server = |n: &str| server.iter().any(|s| s.name.eq_ignore_ascii_case(n)) || BASE.iter().any(|b| b.eq_ignore_ascii_case(n));
    let mut out: Vec<String> = header.iter().map(|h| h.to_string()).collect();
    for s in server.iter().filter(|s| !BASE.iter().any(|b| b.eq_ignore_ascii_case(&s.name))) {
        out.push(format!("*{}", s.name));
    }
    for l in &entries {
        let n = name(l);
        if is_server(&n) {
            continue;
        }
        let on = l.trim().starts_with('*') && client_can_load_name(&n) && !full_master(&n);
        out.push(if on { format!("*{n}") } else { n });
    }
    let mut new = out.join(nl);
    new.push_str(nl);
    if new == text {
        return Ok(false);
    }
    crate::loadorder::write_list(plugins_txt, file.as_ref(), &new)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_of_bytes_is_the_standard_one_and_matches_the_file_reading() {
        assert_eq!(crc32_bytes(b"123456789"), 0xCBF4_3926);
        let t = tempfile::tempdir().unwrap();
        let f = t.path().join("p.esp");
        std::fs::write(&f, b"123456789").unwrap();
        assert_eq!(crc32(&f), Some(0xCBF4_3926));
    }

    #[test]
    fn play_requires_ordered_fingerprinted_base_masters_but_allows_later_plugins() {
        let mut order: Vec<ServerPlugin> = BASE.iter().map(|name| ServerPlugin {
            name: (*name).into(), size: Some(1), sha256: Some("a".repeat(64)), crc32: None,
        }).collect();
        assert!(valid_base(&order));
        order.push(ServerPlugin { name: "ServerWorld.esp".into(), size: None, sha256: None, crc32: None });
        assert!(valid_base(&order));
        order[0].sha256 = None;
        assert!(!valid_base(&order));
        order[0].sha256 = Some("g".repeat(64));
        assert!(!valid_base(&order));
        order[0].sha256 = Some("a".repeat(64));
        order.swap(0, 1);
        assert!(!valid_base(&order));
        order.swap(0, 1);
        order.truncate(4);
        assert!(!valid_base(&order));
    }

    fn plugin(flags: u32) -> Vec<u8> {
        let mut b = b"TES4".to_vec();
        b.extend(0u32.to_le_bytes());
        b.extend(flags.to_le_bytes());
        b.extend([0u8; 12]);
        b
    }

    #[test]
    fn set_exact_keeps_an_ansi_plugins_txt_and_refuses_an_unreadable_one() {
        let t = tempfile::tempdir().unwrap();
        let g = t.path();
        std::fs::create_dir_all(g.join("Data")).unwrap();
        let txt = g.join("plugins.txt");
        std::fs::write(&txt, b"*Caf\xE9.esp\r\n*Mine.esp\r\n").unwrap();
        let server = vec![sp("Server.esp")];
        assert!(set_exact(g, &txt, &server).unwrap());
        let out = std::fs::read(&txt).unwrap();
        assert!(out.windows(4).any(|w| w == b"Caf\xE9"), "the accented name survives: {out:?}");
        assert_eq!(std::fs::read(g.join("plugins.txt.aetherial-dawn-backup")).unwrap(), b"*Caf\xE9.esp\r\n*Mine.esp\r\n");

        let bad = g.join("dir.txt");
        std::fs::create_dir(&bad).unwrap();
        assert!(set_exact(g, &bad, &server).is_err());
        assert!(bad.is_dir());
    }

    fn sp(n: &str) -> ServerPlugin {
        ServerPlugin { name: n.into(), size: None, sha256: None, crc32: None }
    }

    #[test]
    fn crc32_matches_the_standard() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("x");
        std::fs::write(&p, b"123456789").unwrap();
        assert_eq!(crc32(&p), Some(0xCBF4_3926));
    }

    #[test]
    fn creation_club_is_emptied_for_a_session_and_put_back() {
        let t = tempfile::tempdir().unwrap();
        let g = t.path();
        std::fs::write(g.join("Skyrim.ccc"), "ccASVSSE001-ALMSIVI.esm\r\nccBGSSSE001-Fish.esm\r\n").unwrap();
        assert_eq!(hide_ccc(g).unwrap().len(), 1);
        assert_eq!(std::fs::read(g.join("Skyrim.ccc")).unwrap(), b"");
        // A second Play before the game closed keeps the real backup.
        hide_ccc(g).unwrap();
        assert!(restore_ccc(g).unwrap());
        assert!(std::fs::read_to_string(g.join("Skyrim.ccc")).unwrap().contains("ALMSIVI"));
        assert!(!g.join(CCC_BACKUP).exists());
        assert!(!restore_ccc(g).unwrap());

        // Steam puts a newer list back mid-session: it wins over the backup.
        hide_ccc(g).unwrap();
        std::fs::write(g.join("Skyrim.ccc"), "ccBGSSSE001-Fish.esm\r\nccNEW.esm\r\n").unwrap();
        assert!(!restore_ccc(g).unwrap());
        assert!(std::fs::read_to_string(g.join("Skyrim.ccc")).unwrap().contains("ccNEW"));
        assert!(!g.join(CCC_BACKUP).exists(), "the stale backup goes");
        // And the next session backs up the newer list.
        std::fs::write(g.join(CCC_BACKUP), "old\r\n").unwrap();
        hide_ccc(g).unwrap();
        assert!(std::fs::read_to_string(g.join(CCC_BACKUP)).unwrap().contains("ccNEW"));
        restore_ccc(g).unwrap();
        assert!(std::fs::read_to_string(g.join("Skyrim.ccc")).unwrap().contains("ccNEW"));
    }

    #[test]
    fn predicts_the_order_and_compares_it_with_the_server() {
        let t = tempfile::tempdir().unwrap();
        let g = t.path();
        let data = g.join("Data");
        std::fs::create_dir_all(&data).unwrap();
        for b in BASE {
            std::fs::write(data.join(b), plugin(1)).unwrap();
        }
        std::fs::write(data.join("ccASVSSE001-ALMSIVI.esm"), plugin(1)).unwrap();
        std::fs::write(data.join("ccQDRSSE001-SurvivalMode.esl"), plugin(0x201)).unwrap();
        std::fs::write(data.join("Unofficial-Skyrim-Special-Edition-Patch.esp"), plugin(1)).unwrap();
        std::fs::write(data.join("Unofficial Skyrim Special Edition Patch.esp"), plugin(1)).unwrap();
        std::fs::write(data.join("JKs-Skyrim.esp"), plugin(0)).unwrap();
        std::fs::write(data.join("RaceMenu.esp"), plugin(0)).unwrap();
        std::fs::write(data.join("TrueHUD.esl"), plugin(0x200)).unwrap();
        std::fs::write(data.join("ccBGSSSE025-AdvDSGS.esm"), plugin(1)).unwrap();
        std::fs::write(g.join("Skyrim.ccc"), "ccASVSSE001-ALMSIVI.esm\r\nccQDRSSE001-SurvivalMode.esl\r\n").unwrap();
        let txt = g.join("plugins.txt");
        std::fs::write(&txt, "# Vortex\r\n*RaceMenu.esp\r\n*Unofficial Skyrim Special Edition Patch.esp\r\n*JKs-Skyrim.esp\r\n*TrueHUD.esl\r\n*ccBGSSSE025-AdvDSGS.esm\r\n*Unofficial-Skyrim-Special-Edition-Patch.esp\r\nOld.esp\r\n").unwrap();
        let mut server: Vec<ServerPlugin> = BASE.iter().map(|b| sp(b)).collect();
        server.push(sp("Unofficial-Skyrim-Special-Edition-Patch.esp"));
        server.push(sp("JKs-Skyrim.esp"));

        // Creation Club and Vortex's order put the wrong plugins at 5 and 6.
        assert_eq!(game_order(g, &txt)[5], "ccASVSSE001-ALMSIVI.esm");
        assert!(!mismatches(g, &txt, &server).is_empty());

        hide_ccc(g).unwrap();
        assert!(set_exact(g, &txt, &server).unwrap());
        assert_eq!(
            std::fs::read_to_string(&txt).unwrap(),
            "# Vortex\r\n*Unofficial-Skyrim-Special-Edition-Patch.esp\r\n*JKs-Skyrim.esp\r\n*RaceMenu.esp\r\nUnofficial Skyrim Special Edition Patch.esp\r\n*TrueHUD.esl\r\nccBGSSSE025-AdvDSGS.esm\r\nOld.esp\r\n"
        );
        assert_eq!(game_order(g, &txt), [&BASE[..], &["Unofficial-Skyrim-Special-Edition-Patch.esp", "JKs-Skyrim.esp", "RaceMenu.esp"]].concat());
        assert_eq!(mismatches(g, &txt, &server), Vec::<String>::new());
        assert!(!set_exact(g, &txt, &server).unwrap(), "already exact");

        // A different file under the right name is caught by size.
        server[6].size = Some(999);
        assert_eq!(mismatches(g, &txt, &server), ["JKs-Skyrim.esp: 24 bytes, the server's is 999"]);
    }

    #[test]
    fn reads_the_servers_order() {
        let v = serde_json::json!({"masters": [{"name": "Skyrim.esm", "size": 5, "sha256": "AB"}, {"name": "Data/JKs-Skyrim.esp", "crc32": "0x0000000A"}, {"name": "X.esp", "crc32": 11}]});
        let o = server_order(&v);
        assert_eq!(o[0], ServerPlugin { name: "Skyrim.esm".into(), size: Some(5), sha256: Some("ab".into()), crc32: None });
        assert_eq!((o[1].name.as_str(), o[1].crc32), ("JKs-Skyrim.esp", Some(10)));
        assert_eq!(o[2].crc32, Some(11));
        assert!(!beyond_base(&o));
        // The object form has no order.
        assert!(server_order(&serde_json::json!({"Skyrim.esm": {"size": 1}, "A.esp": {"size": 2}})).is_empty());
    }

    #[test]
    fn a_converted_master_is_compared_by_its_canonical_bytes() {
        let v = serde_json::json!({"masters": [
            {"name": "_ResourcePack.esm", "size": 9, "crc32": "0000000a", "sha256": "AA", "canonical_sha256": "BB", "canonical_size": 8, "canonical_crc32": "0000000b"},
            {"name": "ccBGSSSE001-Fish.esm", "size": 7, "crc32": 12, "sha256": "cc"},
            {"name": "ccQDRSSE001-SurvivalMode.esm", "size": 9, "crc32": 1, "sha256": "dd", "canonicalSha256": "ee"}]});
        let o = server_order(&v);
        assert_eq!((o[0].size, o[0].crc32, o[0].sha256.as_deref()), (Some(8), Some(0xb), Some("bb")));
        assert_eq!((o[1].size, o[1].crc32, o[1].sha256.as_deref()), (Some(7), Some(12), Some("cc")));
        // Canonical hash without a canonical size or crc: those aren't checked.
        assert_eq!((o[2].size, o[2].crc32, o[2].sha256.as_deref()), (None, None, Some("ee")));
        // A served name that could write a line of its own into plugins.txt,
        // a path, or a repeat makes the whole list unusable (Play refuses).
        let base: Vec<serde_json::Value> = BASE.iter().map(|n| serde_json::json!({"name": n, "size": 1, "sha256": "a".repeat(64)})).collect();
        let with = |extra: &[&str]| {
            let mut v = base.clone();
            v.extend(extra.iter().map(|n| serde_json::json!({"name": n, "size": 1})));
            server_order(&serde_json::json!({ "masters": v }))
        };
        assert_eq!(with(&["COTN-Dawnstar.esp"]).len(), 6);
        // A "Data/..." path is read as its file name, as before.
        assert_eq!(with(&["Data/COTN-Dawnstar.esp"])[5].name, "COTN-Dawnstar.esp");
        for bad in [&["../../evil\n*X.esp"][..], &["X.esp\r"], &["*X.esp"], &["A.esp", "a.ESP"], &["Skyrim.esm"]] {
            assert!(with(bad).is_empty(), "{bad:?}");
        }
    }

    #[test]
    fn masters_json_is_made_from_the_export_record() {
        let base: Vec<serde_json::Value> = BASE.iter().enumerate().map(|(i, n)| serde_json::json!({"name": n, "size": 10 + i, "sha256": format!("{i}").repeat(64)})).collect();
        let h = |c: char| c.to_string().repeat(64);
        let export = serde_json::json!({
            "masters": [
                {"file": "_ResourcePack.esl", "run_name": "_ResourcePack.esm", "index": 5, "sha256": h('a'), "canonical_sha256": h('B'), "canonical_bytes": 90, "canonical_crc32": 4000000000u32},
                {"file": "ccBGSSSE001-Fish.esm", "run_name": "ccBGSSSE001-Fish.esm", "index": 6, "sha256": h('c'), "canonical_sha256": h('c'), "canonical_bytes": 70, "canonical_crc32": 7}],
            "files": {
                "COTN Dawnstar.esp": {"sha256": h('d'), "bytes": 5, "index": 8, "run_name": "COTN-Dawnstar.esp", "canonical_sha256": h('e'), "canonical_bytes": 6, "canonical_crc32": 8},
                "Unofficial Skyrim Special Edition Patch.esp": {"sha256": h('f'), "bytes": 9, "index": 7, "run_name": "Unofficial-Skyrim-Special-Edition-Patch.esp", "canonical_sha256": h('f'), "canonical_bytes": 9, "canonical_crc32": 9}}});
        let m = masters_json(&base, &export).unwrap();
        let order = server_order(&m);
        let names: Vec<&str> = order.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names[5..], ["_ResourcePack.esm", "ccBGSSSE001-Fish.esm", "Unofficial-Skyrim-Special-Edition-Patch.esp", "COTN-Dawnstar.esp"]);
        assert!(valid_base(&order));
        assert_eq!((order[5].size, order[5].crc32, order[5].sha256.clone()), (Some(90), Some(4000000000), Some("b".repeat(64))));
        // Only canonical values, never the exporting PC's own file hash.
        assert!(!m.to_string().contains(&h('a')));
        // The health check reads the same values.
        assert_eq!(crate::health::parse_masters(&m)[5], ("_ResourcePack.esm".into(), Some(90), Some("b".repeat(64))));
        // A gap, a missing canonical value, or bad base masters are refused.
        let mut gap = export.clone();
        gap["files"]["COTN Dawnstar.esp"]["index"] = 9.into();
        assert!(masters_json(&base, &gap).unwrap_err().contains("next index is 8"));
        let mut no_crc = export.clone();
        no_crc["masters"][1].as_object_mut().unwrap().remove("canonical_crc32");
        assert!(masters_json(&base, &no_crc).unwrap_err().contains("canonical_crc32"));
        assert!(masters_json(&base[..4], &export).is_err());
        // Older launchers read the plain fields: they carry the canonical values.
        assert_eq!((m["masters"][5]["size"].as_u64(), m["masters"][5]["sha256"].as_str()), (Some(90), Some("b".repeat(64).as_str())));
        // A name that isn't a plain plugin file name, a base name, or a repeat is refused.
        for bad in ["../../evil\n*X.esp", "Data/X.esp", "*X.esp", "#X.esp", "X.txt", "Skyrim.esm", "_resourcepack.ESM"] {
            let mut e = export.clone();
            e["files"]["COTN Dawnstar.esp"]["run_name"] = bad.into();
            assert!(masters_json(&base, &e).is_err(), "{bad:?}");
        }
    }
}
