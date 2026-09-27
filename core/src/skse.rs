//! Reads an SKSE plugin DLL's version data and applies SKSE 2.2.6's own
//! rules for Skyrim 1.6.1170, so a DLL built for another Skyrim is caught
//! before the SKSE Plugin Loader stops the game with "disabled, only
//! compatible with versions earlier than 1.6.629" (True Directional
//! Movement's old build, 2026-09-26).

use std::path::Path;

/// Skyrim 1.6.1170 as SKSE packs it: (major << 24) | (minor << 16) | (build << 4).
pub const RUNTIME_1_6_1170: u32 = (1 << 24) | (6 << 16) | (1170 << 4);
/// SKSE 2.2.6, packed the same way.
pub const SKSE_2_2_6: u32 = (2 << 24) | (2 << 16) | (6 << 4);

// SKSEPluginVersionData flags (skse64 PluginAPI.h).
const ADDRESS_LIBRARY_POST_AE: u32 = 1 << 0;
const SIGNATURES: u32 = 1 << 1;
const STRUCTS_POST_629: u32 = 1 << 2;
const NO_STRUCT_USE: u32 = 1 << 0;

/// What SKSE would make of a plugin DLL.
#[derive(Debug, Clone, PartialEq)]
pub enum Build {
    /// SKSE loads it on 1.6.1170.
    Fits,
    /// SKSE refuses it; the text says why, in plain words.
    Wrong(String),
    /// Not a readable SKSE plugin (a helper DLL, or a layout this doesn't
    /// know); left alone.
    Unknown,
}

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}

/// The exports of a 64-bit DLL by name, with the file offset of what each
/// points at (None when it points past the file's data, like zeroed memory).
fn exports(b: &[u8]) -> Option<Vec<(String, Option<usize>)>> {
    if b.get(0..2)? != b"MZ" {
        return None;
    }
    let pe = u32_at(b, 0x3c)? as usize;
    if b.get(pe..pe + 4)? != b"PE\0\0" {
        return None;
    }
    let coff = pe + 4;
    let sections = u16_at(b, coff + 2)? as usize;
    let opt_size = u16_at(b, coff + 16)? as usize;
    let opt = coff + 20;
    let dirs = match u16_at(b, opt)? {
        0x20b => opt + 112,
        0x10b => opt + 96,
        _ => return None,
    };
    let export_rva = u32_at(b, dirs)? as usize;
    if export_rva == 0 {
        return Some(Vec::new());
    }
    let table = opt + opt_size;
    let secs: Vec<(usize, usize, usize, usize)> = (0..sections)
        .filter_map(|i| {
            let s = table + i * 40;
            Some((u32_at(b, s + 12)? as usize, u32_at(b, s + 8)? as usize, u32_at(b, s + 20)? as usize, u32_at(b, s + 16)? as usize))
        })
        .collect();
    // File offset of an RVA, only when that much is really in the file.
    let off = |rva: usize, len: usize| -> Option<usize> {
        secs.iter().find_map(|&(va, vsize, raw, rawsize)| {
            (rva >= va && rva < va + vsize.max(rawsize)).then_some(())?;
            let d = rva - va;
            (d + len <= rawsize && raw + d + len <= b.len()).then_some(raw + d)
        })
    };
    let ed = off(export_rva, 40)?;
    let names = u32_at(b, ed + 24)? as usize;
    let funcs = off(u32_at(b, ed + 28)? as usize, 0)?;
    let name_ptrs = off(u32_at(b, ed + 32)? as usize, names * 4)?;
    let ords = off(u32_at(b, ed + 36)? as usize, names * 2)?;
    let mut out = Vec::new();
    for i in 0..names.min(4096) {
        let Some(n) = off(u32_at(b, name_ptrs + i * 4)? as usize, 1) else { continue };
        let end = b[n..].iter().take(256).position(|&c| c == 0).map(|e| n + e)?;
        let name = String::from_utf8_lossy(&b[n..end]).into_owned();
        let ord = u16_at(b, ords + i * 2)? as usize;
        let rva = u32_at(b, funcs + ord * 4)? as usize;
        out.push((name, off(rva, 848)));
    }
    Some(out)
}

/// Applies SKSE 2.2.6's plugin checks for Skyrim 1.6.1170 to a DLL's bytes.
pub fn build_of_bytes(b: &[u8]) -> Build {
    let Some(ex) = exports(b) else { return Build::Unknown };
    let version = ex.iter().find(|(n, _)| n == "SKSEPlugin_Version");
    let Some((_, at)) = version else {
        return if ex.iter().any(|(n, _)| n == "SKSEPlugin_Query") {
            Build::Wrong("it's the build for Skyrim before 1.6.629".into())
        } else {
            Build::Unknown
        };
    };
    let Some(v) = at.map(|o| &b[o..o + 848]) else { return Build::Unknown };
    let data_version = u32_at(v, 0).unwrap_or(0);
    if data_version == 0 || data_version > 1 || v[8] == 0 {
        return Build::Unknown;
    }
    let ex_flags = u32_at(v, 772).unwrap_or(0);
    let flags = u32_at(v, 776).unwrap_or(0);
    let compatible: Vec<u32> = (0..16).filter_map(|i| u32_at(v, 780 + i * 4)).take_while(|&x| x != 0).collect();
    let se_required = u32_at(v, 844).unwrap_or(0);
    if flags & STRUCTS_POST_629 == 0 && ex_flags & NO_STRUCT_USE == 0 {
        return Build::Wrong("it's the build for Skyrim before 1.6.629".into());
    }
    if flags & (ADDRESS_LIBRARY_POST_AE | SIGNATURES) == 0 && !compatible.contains(&RUNTIME_1_6_1170) {
        return Build::Wrong("it's built for a different Skyrim version than 1.6.1170".into());
    }
    if se_required > SKSE_2_2_6 {
        return Build::Wrong("it needs a newer SKSE than 2.2.6".into());
    }
    Build::Fits
}

/// What the launcher read from a DLL's SKSE data, for the log when it
/// refuses one: its SKSEPlugin_* exports, flags and listed Skyrim versions.
pub fn describe(p: &Path) -> String {
    let Ok(b) = std::fs::read(p) else { return "unreadable".into() };
    let Some(ex) = exports(&b) else { return format!("not a 64-bit DLL ({} bytes)", b.len()) };
    let names: Vec<&str> = ex.iter().map(|(n, _)| n.as_str()).filter(|n| n.starts_with("SKSEPlugin")).collect();
    let mut out = format!("{} bytes, {} exports, SKSE exports [{}]", b.len(), ex.len(), names.join(", "));
    if let Some(v) = ex.iter().find(|(n, _)| n == "SKSEPlugin_Version").and_then(|(_, at)| at.map(|o| &b[o..o + 848])) {
        let ver = |x: u32| format!("{}.{}.{}", x >> 24, (x >> 16) & 0xff, (x >> 4) & 0xfff);
        let compatible: Vec<String> = (0..16).filter_map(|i| u32_at(v, 780 + i * 4)).take_while(|&x| x != 0).map(ver).collect();
        out.push_str(&format!(
            "; data version {}, flags {:#x}, ex flags {:#x}, games [{}], needs SKSE {}",
            u32_at(v, 0).unwrap_or(0),
            u32_at(v, 776).unwrap_or(0),
            u32_at(v, 772).unwrap_or(0),
            compatible.join(", "),
            ver(u32_at(v, 844).unwrap_or(0))
        ));
    }
    out
}

/// What SKSE would make of the DLL at `p` (Unknown when it can't be read).
pub fn build_of(p: &Path) -> Build {
    match std::fs::read(p) {
        Ok(b) => build_of_bytes(&b),
        Err(_) => Build::Unknown,
    }
}

/// Whether `rel` (relative to the game folder, / or \) is a DLL SKSE loads.
pub fn is_skse_plugin(rel: &str) -> bool {
    let l = rel.replace('\\', "/").to_ascii_lowercase();
    let l = l.trim_start_matches("./");
    l.starts_with("data/skse/plugins/") && l.ends_with(".dll") && l.matches('/').count() == 3
}

/// Why a plugin DLL won't load, when it definitely won't.
pub fn wrong_build(p: &Path) -> Option<String> {
    match build_of(p) {
        Build::Wrong(w) => Some(w),
        _ => None,
    }
}

/// The DLLs in Data/SKSE/Plugins that SKSE would refuse, with the reason.
pub fn wrong_builds(game_dir: &Path) -> Vec<(String, String)> {
    let dir = game_dir.join("Data").join("SKSE").join("Plugins");
    let mut out: Vec<(String, String)> = std::fs::read_dir(dir)
        .map(|r| {
            r.flatten()
                .filter(|e| e.path().is_file() && e.file_name().to_string_lossy().to_ascii_lowercase().ends_with(".dll"))
                .filter_map(|e| wrong_build(&e.path()).map(|w| (format!("Data/SKSE/Plugins/{}", e.file_name().to_string_lossy()), w)))
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

#[cfg(test)]
pub mod tests {
    use super::*;

    #[test]
    fn describes_what_it_read() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("old.dll");
        std::fs::write(&p, dll(&["SKSEPlugin_Query", "SKSEPlugin_Load"], &[0u8; 4])).unwrap();
        let d = describe(&p);
        assert!(d.contains("SKSE exports [SKSEPlugin_Query, SKSEPlugin_Load]"), "{d}");
        assert_eq!(describe(&t.path().join("missing.dll")), "unreadable");
    }

    /// A minimal 64-bit DLL exporting `names`, the first pointing at `data`.
    pub fn dll(names: &[&str], data: &[u8]) -> Vec<u8> {
        let mut b = vec![0u8; 0x400];
        b[0..2].copy_from_slice(b"MZ");
        b[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        b[0x80..0x84].copy_from_slice(b"PE\0\0");
        let coff = 0x84;
        b[coff + 2..coff + 4].copy_from_slice(&1u16.to_le_bytes());
        b[coff + 16..coff + 18].copy_from_slice(&240u16.to_le_bytes());
        let opt = coff + 20;
        b[opt..opt + 2].copy_from_slice(&0x20bu16.to_le_bytes());
        // One section: RVA 0x1000 maps to file offset 0x400.
        let sec = opt + 240;
        let body_len = 0x200 + data.len() + 0x100;
        b[sec + 8..sec + 12].copy_from_slice(&(body_len as u32).to_le_bytes());
        b[sec + 12..sec + 16].copy_from_slice(&0x1000u32.to_le_bytes());
        b[sec + 16..sec + 20].copy_from_slice(&(body_len as u32).to_le_bytes());
        b[sec + 20..sec + 24].copy_from_slice(&0x400u32.to_le_bytes());
        let mut body = vec![0u8; body_len];
        let rva = |o: usize| (0x1000 + o) as u32;
        // Export directory at body 0, tables at 0x40.., names at 0x100.., data at 0x200.
        let n = names.len();
        body[24..28].copy_from_slice(&(n as u32).to_le_bytes());
        body[28..32].copy_from_slice(&rva(0x40).to_le_bytes());
        body[32..36].copy_from_slice(&rva(0x60).to_le_bytes());
        body[36..40].copy_from_slice(&rva(0x80).to_le_bytes());
        let mut at = 0x100;
        for (i, name) in names.iter().enumerate() {
            body[0x40 + i * 4..0x44 + i * 4].copy_from_slice(&rva(0x200).to_le_bytes());
            body[0x60 + i * 4..0x64 + i * 4].copy_from_slice(&rva(at).to_le_bytes());
            body[0x80 + i * 2..0x82 + i * 2].copy_from_slice(&(i as u16).to_le_bytes());
            body[at..at + name.len()].copy_from_slice(name.as_bytes());
            at += name.len() + 1;
        }
        body[0x200..0x200 + data.len()].copy_from_slice(data);
        b[opt + 112..opt + 116].copy_from_slice(&rva(0).to_le_bytes());
        b[opt + 116..opt + 120].copy_from_slice(&40u32.to_le_bytes());
        b.extend(body);
        b
    }

    /// SKSEPluginVersionData with the given flags and compatible versions.
    pub fn version_data(flags: u32, ex: u32, compatible: &[u32]) -> Vec<u8> {
        let mut v = vec![0u8; 848 + 520];
        v[0..4].copy_from_slice(&1u32.to_le_bytes());
        v[8..12].copy_from_slice(b"Test");
        v[772..776].copy_from_slice(&ex.to_le_bytes());
        v[776..780].copy_from_slice(&flags.to_le_bytes());
        for (i, c) in compatible.iter().enumerate() {
            v[780 + i * 4..784 + i * 4].copy_from_slice(&c.to_le_bytes());
        }
        v
    }

    /// An AE plugin DLL that SKSE loads on 1.6.1170.
    pub fn good_dll() -> Vec<u8> {
        dll(&["SKSEPlugin_Version"], &version_data(ADDRESS_LIBRARY_POST_AE | STRUCTS_POST_629, 0, &[]))
    }

    /// The pre-1.6.629 build of a plugin, like the TDM file on Timothy's PC.
    pub fn old_dll() -> Vec<u8> {
        dll(&["SKSEPlugin_Version"], &version_data(ADDRESS_LIBRARY_POST_AE, 0, &[]))
    }

    #[test]
    fn reads_what_skse_would_decide() {
        assert_eq!(build_of_bytes(&good_dll()), Build::Fits);
        assert!(matches!(build_of_bytes(&old_dll()), Build::Wrong(w) if w.contains("1.6.629")));
        // Only 1.5.97 (SE) support: no Version export at all.
        assert!(matches!(build_of_bytes(&dll(&["SKSEPlugin_Query"], &[0; 8])), Build::Wrong(_)));
        // A plugin with no structs that lists 1.6.1170 by hand is fine; one listing only 1.6.640 isn't.
        assert_eq!(build_of_bytes(&dll(&["SKSEPlugin_Version"], &version_data(0, NO_STRUCT_USE, &[RUNTIME_1_6_1170]))), Build::Fits);
        let v640 = (1 << 24) | (6 << 16) | (640 << 4);
        assert!(matches!(build_of_bytes(&dll(&["SKSEPlugin_Version"], &version_data(STRUCTS_POST_629, 0, &[v640]))), Build::Wrong(w) if w.contains("different")));
        // Not an SKSE plugin, or not a DLL: left alone.
        assert_eq!(build_of_bytes(&dll(&["Helper"], &[0; 8])), Build::Unknown);
        assert_eq!(build_of_bytes(b"not a dll"), Build::Unknown);
        assert!(is_skse_plugin("Data/SKSE/Plugins/TrueHUD.dll"));
        assert!(!is_skse_plugin("Data/SKSE/Plugins/x/TrueHUD.dll"));
    }

    #[test]
    fn real_engine_fixes_builds() {
        let dir = match std::env::var("AD_SKSE_DLLS") {
            Ok(d) => std::path::PathBuf::from(d),
            Err(_) => return,
        };
        let ae = build_of(&dir.join("EngineFixes FOMOD Installer/AE/SKSE/Plugins/EngineFixes.dll"));
        let se = build_of(&dir.join("EngineFixes FOMOD Installer/SE/SKSE/Plugins/EngineFixes.dll"));
        assert_eq!(ae, Build::Fits);
        assert!(matches!(se, Build::Wrong(_)), "{se:?}");
        assert_eq!(build_of(&dir.join("SKSE/Plugins/SkyrimSoulsRE.dll")), Build::Fits);
    }
}
