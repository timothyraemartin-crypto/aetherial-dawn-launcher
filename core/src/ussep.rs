//! Which Unofficial Skyrim Special Edition Patch a player has. USSEP 4.3.9
//! and later need Skyrim 1.7.99; the server runs 1.6.1170, whose last patch
//! is 4.3.8a. 4.3.9c on 1.6.1170 crashed the game 17-19 seconds in while
//! drawing land it changes (SkyrimSE.exe+02AD242, Timothy 2026-09-26 22:39
//! and 23:50), so a newer patch is set aside and 4.3.8a installed instead.

use std::path::{Path, PathBuf};

use crate::requirements::USSEP_PLUGIN;

/// The newest patch made for Skyrim 1.6.1170.
pub const NEWEST_FOR_1170: (u32, u32, u32) = (4, 3, 8);
/// Picks the 1.6.1170 file on the Nexus page (matched against the version).
pub const NEXUS_PICK: &str = "4.3.8";
pub const USSEP_ARCHIVE: &str = "Unofficial Skyrim Special Edition Patch.bsa";

/// The first "4.3.9"-style version in a text ("4-3-9c" too), with no digit
/// right before it.
pub fn parse_version(text: &str) -> Option<(u32, u32, u32)> {
    let b = text.as_bytes();
    let num = |i: usize| -> Option<(u32, usize)> {
        let end = b[i..].iter().position(|c| !c.is_ascii_digit()).map(|e| i + e).unwrap_or(b.len());
        (end > i && end - i <= 4).then(|| (text[i..end].parse().ok(), end)).and_then(|(n, e)| n.map(|n| (n, e)))
    };
    for i in 0..b.len() {
        if !b[i].is_ascii_digit() || (i > 0 && b[i - 1].is_ascii_digit()) {
            continue;
        }
        let Some((a, e1)) = num(i) else { continue };
        if a != 4 || e1 >= b.len() || !matches!(b[e1], b'.' | b'-') {
            continue;
        }
        let Some((m, e2)) = num(e1 + 1) else { continue };
        if e2 >= b.len() || !matches!(b[e2], b'.' | b'-') {
            continue;
        }
        if let Some((p, _)) = num(e2 + 1) {
            return Some((a, m, p));
        }
    }
    None
}

/// The version in a plugin's header description (SNAM), if it names one.
pub fn header_version(plugin: &Path) -> Option<(u32, u32, u32)> {
    use std::io::Read;
    let mut f = std::fs::File::open(plugin).ok()?;
    let mut head = [0u8; 24];
    f.read_exact(&mut head).ok()?;
    if &head[..4] != b"TES4" {
        return None;
    }
    let size = u32::from_le_bytes(head[4..8].try_into().ok()?) as usize;
    let mut body = vec![0u8; size.min(1 << 20)];
    f.read_exact(&mut body).ok()?;
    let mut at = 0;
    while at + 6 <= body.len() {
        let kind = &body[at..at + 4];
        let len = u16::from_le_bytes([body[at + 4], body[at + 5]]) as usize;
        let data = body.get(at + 6..at + 6 + len)?;
        if kind == b"SNAM" {
            return parse_version(&String::from_utf8_lossy(data));
        }
        at += 6 + len;
    }
    None
}

/// The patch's version from its Vortex mod folder name
/// ("Unofficial Skyrim Special Edition Patch-266-4-3-9c-1757000000").
fn vortex_version(game_dir: &Path) -> Option<(u32, u32, u32)> {
    let want = format!("data/{}", USSEP_PLUGIN.to_ascii_lowercase());
    let f = crate::allowlist::vortex_files(game_dir).into_iter().find(|f| f.rel.to_ascii_lowercase() == want)?;
    parse_version(&f.source)
}

/// The installed patch's version: its header, then Vortex's folder name,
/// then the launcher's own install record.
pub fn installed_version(game_dir: &Path) -> Option<(u32, u32, u32)> {
    let esp = game_dir.join("Data").join(USSEP_PLUGIN);
    if !esp.is_file() {
        return None;
    }
    header_version(&esp)
        .or_else(|| vortex_version(game_dir))
        .or_else(|| crate::modlist::load_installed(game_dir).mods.get("ussep").and_then(|m| m.version.as_deref().and_then(parse_version)))
}

fn show(v: (u32, u32, u32)) -> String {
    format!("{}.{}.{}", v.0, v.1, v.2)
}

/// Why the installed patch can't be used on 1.6.1170, when it can't.
pub fn too_new(game_dir: &Path) -> Option<String> {
    let v = installed_version(game_dir)?;
    (v > NEWEST_FOR_1170).then(|| format!("Unofficial Patch {} needs Skyrim 1.7.99; Skyrim 1.6.1170 needs {}", show(v), "4.3.8a"))
}

/// A downloaded patch plugin that is too new for 1.6.1170 (by its header).
pub fn plugin_too_new(plugin: &Path) -> Option<String> {
    header_version(plugin).filter(|v| *v > NEWEST_FOR_1170).map(show)
}

/// The patch's files to set aside: the plugin, its archive, and the rest of
/// its Vortex mod (so the mod goes whole).
fn files(game_dir: &Path) -> Vec<String> {
    let mut out = vec![format!("Data/{USSEP_PLUGIN}"), format!("Data/{USSEP_ARCHIVE}")];
    let want = format!("data/{}", USSEP_PLUGIN.to_ascii_lowercase());
    let vf = crate::allowlist::vortex_files(game_dir);
    if let Some(src) = vf.iter().find(|f| f.rel.to_ascii_lowercase() == want).map(|f| f.source.clone()) {
        for f in vf.iter().filter(|f| f.source == src) {
            if !out.iter().any(|o| o.eq_ignore_ascii_case(&f.rel)) {
                out.push(f.rel.clone());
            }
        }
    }
    out.retain(|r| game_dir.join(r).is_file());
    out
}

/// Moves a patch too new for 1.6.1170 to `.aetherial-dawn/disabled/<time>-too-new-ussep/`
/// with the reason in why.txt. Returns where, or None when there was nothing to do.
pub fn set_aside_if_too_new(game_dir: &Path) -> crate::Result<Option<(PathBuf, String)>> {
    let Some(why) = too_new(game_dir) else { return Ok(None) };
    let list = files(game_dir);
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let dest = crate::strays::move_aside(game_dir, &list, &format!("{secs}-too-new-ussep"))?;
    std::fs::create_dir_all(&dest)?;
    std::fs::write(dest.join("why.txt"), format!("{why}. Moved: {}\n", list.join(", ")))?;
    Ok(Some((dest, why)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A plugin header with a description.
    pub fn plugin_with(desc: &str) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend(b"HEDR");
        body.extend(12u16.to_le_bytes());
        body.extend(1.71f32.to_le_bytes());
        body.extend([0u8; 8]);
        body.extend(b"SNAM");
        body.extend(((desc.len() + 1) as u16).to_le_bytes());
        body.extend(desc.as_bytes());
        body.push(0);
        let mut out = Vec::new();
        out.extend(b"TES4");
        out.extend((body.len() as u32).to_le_bytes());
        out.extend([0u8; 16]);
        out.extend(body);
        out.extend(b"GRUP");
        out.extend([0u8; 20]);
        out
    }

    #[test]
    fn reads_versions() {
        assert_eq!(parse_version("Unofficial Skyrim Special Edition Patch-266-4-3-9c-1757000000"), Some((4, 3, 9)));
        assert_eq!(parse_version("Unofficial Skyrim Special Edition Patch 266 4.3.8a 1720000000"), Some((4, 3, 8)));
        assert_eq!(parse_version("version 14.3.9"), None);
        assert_eq!(parse_version("no version"), None);
    }

    #[test]
    fn sets_aside_a_patch_made_for_1_7() {
        let t = tempfile::tempdir().unwrap();
        let g = t.path();
        std::fs::create_dir_all(g.join("Data")).unwrap();
        std::fs::write(g.join("Data").join(USSEP_PLUGIN), plugin_with("Unofficial Skyrim Special Edition Patch 4.3.9c")).unwrap();
        std::fs::write(g.join("Data").join(USSEP_ARCHIVE), b"BSA").unwrap();
        assert!(too_new(g).unwrap().contains("4.3.9"));
        let (dest, _) = set_aside_if_too_new(g).unwrap().unwrap();
        assert!(dest.join("Data").join(USSEP_PLUGIN).is_file() && dest.join("why.txt").is_file());
        assert!(!g.join("Data").join(USSEP_ARCHIVE).exists());
        // 4.3.8a is left alone.
        std::fs::write(g.join("Data").join(USSEP_PLUGIN), plugin_with("USSEP 4.3.8a")).unwrap();
        assert_eq!(too_new(g), None);
        assert_eq!(set_aside_if_too_new(g).unwrap(), None);
    }

    #[test]
    fn falls_back_to_the_vortex_folder_name() {
        let t = tempfile::tempdir().unwrap();
        let g = t.path();
        std::fs::create_dir_all(g.join("Data")).unwrap();
        std::fs::write(g.join("Data").join(USSEP_PLUGIN), plugin_with("")).unwrap();
        std::fs::write(
            g.join("Data/vortex.deployment.json"),
            r#"{"files":[{"relPath":"Unofficial Skyrim Special Edition Patch.esp","source":"Unofficial Skyrim Special Edition Patch-266-4-3-9c-1757000000"}]}"#,
        )
        .unwrap();
        assert_eq!(installed_version(g), Some((4, 3, 9)));
    }
}
