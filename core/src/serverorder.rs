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

/// The server's order from its masters.json (list order is load order).
pub fn server_order(masters: &serde_json::Value) -> Vec<ServerPlugin> {
    let list = masters.get("masters").or_else(|| masters.get("files")).unwrap_or(masters);
    let Some(a) = list.as_array() else {
        return crate::health::parse_masters(masters).into_iter().map(|(name, size, sha256)| ServerPlugin { name, size, sha256, crc32: None }).collect();
    };
    a.iter()
        .filter_map(|e| {
            let n = e.get("name").or_else(|| e.get("file")).or_else(|| e.get("path")).and_then(|n| n.as_str())?;
            let crc32 = match e.get("crc32") {
                Some(serde_json::Value::Number(n)) => n.as_u64().and_then(|v| u32::try_from(v).ok()),
                Some(serde_json::Value::String(s)) => u32::from_str_radix(s.trim_start_matches("0x"), 16).ok(),
                _ => None,
            };
            Some(ServerPlugin {
                name: n.rsplit(['/', '\\']).next().unwrap_or(n).to_string(),
                size: e.get("size").and_then(|s| s.as_u64()),
                sha256: e.get("sha256").and_then(|s| s.as_str()).map(|s| s.to_ascii_lowercase()),
                crc32,
            })
        })
        .collect()
}

/// Whether the server loads more than the five base masters: only then do
/// Creation Club plugins and plugins.txt have to follow it exactly.
pub fn beyond_base(order: &[ServerPlugin]) -> bool {
    order.len() > BASE.len()
}

fn ccc_paths(game_dir: &Path) -> Vec<PathBuf> {
    [game_dir.join("Skyrim.ccc"), game_dir.join("Data").join("Skyrim.ccc")].into_iter().filter(|p| p.is_file()).collect()
}

/// Empties Skyrim.ccc for a session, keeping the original beside it (a
/// backup already there from an unfinished session is kept, never
/// overwritten). Returns the files emptied.
pub fn hide_ccc(game_dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for p in ccc_paths(game_dir) {
        let backup = p.with_file_name(CCC_BACKUP);
        if !backup.exists() {
            std::fs::copy(&p, &backup)?;
        }
        if std::fs::metadata(&p)?.len() > 0 {
            // A new file, never a write through a link.
            std::fs::remove_file(&p)?;
            std::fs::write(&p, b"")?;
            out.push(p);
        }
    }
    Ok(out)
}

/// Puts back the Skyrim.ccc a session emptied. Returns whether it did.
pub fn restore_ccc(game_dir: &Path) -> std::io::Result<bool> {
    let mut any = false;
    for dir in [game_dir.to_path_buf(), game_dir.join("Data")] {
        let backup = dir.join(CCC_BACKUP);
        if backup.is_file() {
            std::fs::rename(&backup, dir.join("Skyrim.ccc"))?;
            any = true;
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
        for n in std::fs::read_to_string(ccc).unwrap_or_default().lines().map(str::trim).filter(|l| !l.is_empty()) {
            if !has(&out, n) && full(n).is_some() {
                out.push(n.to_string());
            }
        }
    }
    let active: Vec<String> = std::fs::read_to_string(plugins_txt)
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
    let mut table = [0u32; 256];
    for (i, t) in table.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
        *t = c;
    }
    let mut f = std::fs::File::open(path).ok()?;
    let mut buf = vec![0u8; 1 << 20];
    let mut crc = 0xFFFF_FFFFu32;
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        for b in &buf[..n] {
            crc = table[((crc ^ *b as u32) & 0xFF) as usize] ^ (crc >> 8);
        }
    }
    Some(!crc)
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
/// (Vortex's spaced originals among them; the files stay in Data). The old
/// file is kept beside it. Returns whether it changed.
pub fn set_exact(plugins_txt: &Path, server: &[ServerPlugin]) -> std::io::Result<bool> {
    let text = std::fs::read_to_string(plugins_txt).unwrap_or_default();
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
        let on = l.trim().starts_with('*') && client_can_load_name(&n);
        out.push(if on { format!("*{n}") } else { n });
    }
    let mut new = out.join(nl);
    new.push_str(nl);
    if new == text {
        return Ok(false);
    }
    if !text.is_empty() {
        std::fs::write(plugins_txt.with_extension("txt.aetherial-dawn-backup"), &text)?;
    }
    std::fs::write(plugins_txt, new)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plugin(flags: u32) -> Vec<u8> {
        let mut b = b"TES4".to_vec();
        b.extend(0u32.to_le_bytes());
        b.extend(flags.to_le_bytes());
        b.extend([0u8; 12]);
        b
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
        std::fs::write(g.join("Skyrim.ccc"), "ccASVSSE001-ALMSIVI.esm\r\nccQDRSSE001-SurvivalMode.esl\r\n").unwrap();
        let txt = g.join("plugins.txt");
        std::fs::write(&txt, "# Vortex\r\n*RaceMenu.esp\r\n*Unofficial Skyrim Special Edition Patch.esp\r\n*JKs-Skyrim.esp\r\n*TrueHUD.esl\r\n*Unofficial-Skyrim-Special-Edition-Patch.esp\r\nOld.esp\r\n").unwrap();
        let mut server: Vec<ServerPlugin> = BASE.iter().map(|b| sp(b)).collect();
        server.push(sp("Unofficial-Skyrim-Special-Edition-Patch.esp"));
        server.push(sp("JKs-Skyrim.esp"));

        // Creation Club and Vortex's order put the wrong plugins at 5 and 6.
        assert_eq!(game_order(g, &txt)[5], "ccASVSSE001-ALMSIVI.esm");
        assert!(!mismatches(g, &txt, &server).is_empty());

        hide_ccc(g).unwrap();
        assert!(set_exact(&txt, &server).unwrap());
        assert_eq!(
            std::fs::read_to_string(&txt).unwrap(),
            "# Vortex\r\n*Unofficial-Skyrim-Special-Edition-Patch.esp\r\n*JKs-Skyrim.esp\r\n*RaceMenu.esp\r\nUnofficial Skyrim Special Edition Patch.esp\r\n*TrueHUD.esl\r\nOld.esp\r\n"
        );
        assert_eq!(game_order(g, &txt), [&BASE[..], &["Unofficial-Skyrim-Special-Edition-Patch.esp", "JKs-Skyrim.esp", "RaceMenu.esp"]].concat());
        assert_eq!(mismatches(g, &txt, &server), Vec::<String>::new());
        assert!(!set_exact(&txt, &server).unwrap(), "already exact");

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
    }
}
