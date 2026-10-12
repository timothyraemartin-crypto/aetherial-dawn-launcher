//! Skyrim Platform's in-game browser draws SkyMP's login box. When it never
//! starts the player sits at the main menu with no way to connect (Lambcut,
//! 2026-10-12: Platform log ends at "SetVisible false" with no browser
//! backend lines, helper process not running). This module checks what the
//! launcher can check and fix before Play, and writes what staff need when
//! it still fails:
//!
//! - `SkyrimPlatform.ini` [Browser] BackendName off or nirnlab (no
//!   NirnLabUIPlatform ships with the server) is put back to auto, which
//!   falls back to the built-in browser;
//! - the temp folder the browser keeps its profile in is writable;
//! - the browser's files (SkyrimPlatformCEF.exe, libcef.dll) are listed, so a
//!   report shows at once whether they are on the PC.

use std::path::{Path, PathBuf};

use crate::health::{Check, Status};

const INI: &str = "Data/SKSE/Plugins/SkyrimPlatform.ini";

pub fn ini_path(game_dir: &Path) -> PathBuf {
    game_dir.join(INI)
}

/// The [Browser] BackendName value, lower-cased (None when the ini has none).
pub fn backend(ini: &str) -> Option<String> {
    let mut in_browser = false;
    for line in ini.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            in_browser = l.trim_end().eq_ignore_ascii_case("[Browser]");
        } else if in_browser {
            if let Some((k, v)) = l.split_once('=') {
                if k.trim().eq_ignore_ascii_case("BackendName") {
                    let v = v.split([';', '#']).next().unwrap_or("").trim();
                    return Some(v.to_ascii_lowercase());
                }
            }
        }
    }
    None
}

/// The ini with a browser that cannot work switched back to auto; None when
/// it is fine (auto, tilted, or not set).
pub fn fixed_ini(ini: &str) -> Option<String> {
    if !matches!(backend(ini).as_deref(), Some("off" | "nirnlab")) {
        return None;
    }
    let mut in_browser = false;
    let mut out = Vec::new();
    for line in ini.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            in_browser = l.eq_ignore_ascii_case("[Browser]");
        }
        let is_backend = in_browser && l.split_once('=').is_some_and(|(k, _)| k.trim().eq_ignore_ascii_case("BackendName"));
        out.push(if is_backend { "BackendName=auto".to_string() } else { line.to_string() });
    }
    let mut text = out.join("\r\n");
    text.push_str("\r\n");
    Some(text)
}

/// Fixes the ini on disk. Returns (old value, new value) when it changed it.
pub fn apply_ini_fix(game_dir: &Path) -> std::io::Result<Option<(String, String)>> {
    let p = ini_path(game_dir);
    let Ok(text) = std::fs::read_to_string(&p) else { return Ok(None) };
    let Some(new) = fixed_ini(&text) else { return Ok(None) };
    let old = backend(&text).unwrap_or_default();
    crate::atomicfile::write(&p, new.as_bytes())?;
    Ok(Some((old, "auto".into())))
}

/// Whether `dir` can be created and written to (the browser's profile folder).
pub fn temp_probe(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("can't create {}: {e}", dir.display()))?;
    let f = dir.join(".ad-write-test");
    std::fs::write(&f, b"x").map_err(|e| format!("can't write in {}: {e}", dir.display()))?;
    let _ = std::fs::remove_file(&f);
    Ok(())
}

/// The browser's files found under Data/Platform and Data/SKSE/Plugins: name
/// (relative to the game folder) and size.
pub fn inventory(game_dir: &Path) -> Vec<(String, u64)> {
    fn walk(root: &Path, dir: &Path, depth: u32, out: &mut Vec<(String, u64)>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                if depth < 5 {
                    walk(root, &p, depth + 1, out);
                }
                continue;
            }
            let n = e.file_name().to_string_lossy().to_ascii_lowercase();
            if n.contains("cef") || n.contains("nirnlab") || n == "icudtl.dat" || n == "resources.pak" {
                let rel = p.strip_prefix(root).map(|r| r.to_string_lossy().replace('\\', "/")).unwrap_or_default();
                out.push((rel, e.metadata().map(|m| m.len()).unwrap_or(0)));
            }
        }
    }
    let mut out = Vec::new();
    for d in ["Data/Platform", "Data/SKSE/Plugins"] {
        walk(game_dir, &game_dir.join(d), 0, &mut out);
    }
    out.sort();
    out.dedup();
    out
}

fn has(inv: &[(String, u64)], name: &str) -> bool {
    inv.iter().any(|(rel, size)| *size > 0 && rel.rsplit('/').next().is_some_and(|f| f.eq_ignore_ascii_case(name)))
}

/// The health check: warns when the browser cannot start.
pub fn check(inv: &[(String, u64)], backend: Option<&str>, temp: &Result<(), String>, fixed: Option<&(String, String)>) -> Check {
    let mut items = Vec::new();
    if !has(inv, "SkyrimPlatformCEF.exe") {
        items.push("SkyrimPlatformCEF.exe (the browser's helper) is missing or empty".to_string());
    }
    if !has(inv, "libcef.dll") {
        items.push("libcef.dll (the browser engine) is missing or empty".to_string());
    }
    if let Err(e) = temp {
        items.push(format!("the browser's temp folder is not usable: {e}"));
    }
    if let Some(b) = backend.filter(|b| matches!(*b, "off" | "nirnlab")) {
        items.push(format!("SkyrimPlatform.ini had the browser set to \"{b}\""));
    }
    let (status, detail) = if items.is_empty() {
        (Status::Ok, match fixed {
            Some((old, new)) => format!("SkyrimPlatform.ini had the browser set to \"{old}\"; the launcher put it back to \"{new}\"."),
            None => "The in-game browser's files are there and its temp folder is writable.".to_string(),
        })
    } else {
        (Status::Warn, "The in-game browser that shows the login box may not start.".to_string())
    };
    Check { id: "browser", title: "In-game browser", status, detail, items }
}

/// For a staff report: the ini, then the browser's files with sizes.
pub fn report_text(game_dir: &Path) -> String {
    let ini = std::fs::read_to_string(ini_path(game_dir)).unwrap_or_else(|_| "(SkyrimPlatform.ini not found)".into());
    let inv = inventory(game_dir);
    let files = if inv.is_empty() { "  (none found under Data/Platform or Data/SKSE/Plugins)".to_string() } else { inv.iter().map(|(n, s)| format!("  {n}  {s} bytes")).collect::<Vec<_>>().join("\n") };
    format!("===== SkyrimPlatform.ini =====\n{}\n===== browser files =====\n{files}\n", ini.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_browser_backend_is_read_from_its_section_only() {
        assert_eq!(backend("[Debug]\nBackendName=off\n[Browser]\nBackendName = Auto ; fallback\n"), Some("auto".into()));
        assert_eq!(backend("[Debug]\nBackendName=off\n"), None);
        assert_eq!(backend("[browser]\nbackendname=NirnLab\n"), Some("nirnlab".into()));
    }

    #[test]
    fn only_an_off_or_nirnlab_browser_is_fixed_and_the_rest_of_the_file_is_kept() {
        assert_eq!(fixed_ini("[Browser]\nBackendName=auto\n"), None);
        assert_eq!(fixed_ini("[Browser]\nBackendName=tilted\n"), None);
        assert_eq!(fixed_ini("[Main]\nPluginFolders=Data/Platform/Plugins\n"), None);
        let fixed = fixed_ini("[Main]\nPluginFolders=Data/Platform/Plugins\n[Browser]\nBackendName=off\n[Debug]\nBackendName=off\n").unwrap();
        assert_eq!(backend(&fixed), Some("auto".into()));
        assert!(fixed.contains("PluginFolders=Data/Platform/Plugins"));
        assert!(fixed.contains("[Debug]\r\nBackendName=off"), "other sections are left alone");
    }

    #[test]
    fn the_inventory_finds_the_browser_files_and_the_check_names_what_is_missing() {
        let g = std::env::temp_dir().join(format!("ad-browser-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&g);
        let d = g.join("Data/Platform/Distribution/RuntimeDependencies");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("libcef.dll"), b"x").unwrap();
        std::fs::write(d.join("SkyrimPlatformCEF.exe"), b"").unwrap();
        let inv = inventory(&g);
        assert_eq!(inv.len(), 2);
        let c = check(&inv, Some("off"), &Err("denied".into()), None);
        assert_eq!(c.status, Status::Warn);
        assert_eq!(c.items.len(), 3, "empty helper, temp, backend: {:?}", c.items);
        std::fs::write(d.join("SkyrimPlatformCEF.exe"), b"x").unwrap();
        let ok = check(&inventory(&g), None, &Ok(()), None);
        assert_eq!(ok.status, Status::Ok);
        assert!(temp_probe(&g.join("tmp")).is_ok());
        let _ = std::fs::remove_dir_all(&g);
    }
}
