//! Tools a listed mod ships that normally need a person to press a button:
//! BodySlide fitting armor to the bodies, Pandora building animation
//! behaviors (TOOLS-AND-BODIES.md). The launcher runs them itself, minimized,
//! after the mods install and before the game starts, and only again when
//! something they read changes (Timothy's one-button rule).
//!
//! The program must be a file the launcher installed for that same mod, so a
//! list can only start a tool that came from Nexus with the mod.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use crate::modlist::ModEntry;
use crate::{Error, Result};

const RECORD: &str = ".aetherial-dawn/mods/tools.json";
const DEFAULT_TIMEOUT: u64 = 30 * 60;

/// A mods.json entry's `run`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ToolRun {
    /// Shown while it runs ("Fitting armor to bodies…").
    pub label: String,
    /// The program, relative to the game folder ("Data/CalienteTools/BodySlide/BodySlide.exe").
    pub exe: String,
    /// Its arguments; "{game}" and "{data}" become the game and Data folders.
    #[serde(default)]
    pub args: Vec<String>,
    /// Files or folders (relative to the game folder) it reads: it runs
    /// again when any of them changes.
    #[serde(default)]
    pub inputs: Vec<String>,
    /// Seconds before it is stopped (default 30 minutes).
    #[serde(default)]
    pub timeout: Option<u64>,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct Done {
    stamp: String,
    when: u64,
}

fn load(game_dir: &Path) -> BTreeMap<String, Done> {
    std::fs::read(game_dir.join(RECORD)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save(game_dir: &Path, r: &BTreeMap<String, Done>) -> Result<()> {
    let p = game_dir.join(RECORD);
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(p, serde_json::to_vec_pretty(r)?)?;
    Ok(())
}

fn args_for(run: &ToolRun, game_dir: &Path) -> Vec<String> {
    let game = game_dir.to_string_lossy();
    let data = game_dir.join("Data").to_string_lossy().into_owned();
    run.args.iter().map(|a| a.replace("{game}", &game).replace("{data}", &data)).collect()
}

/// Every file under the inputs (path, size, modified time), hashed with the
/// program and its arguments.
fn stamp(game_dir: &Path, run: &ToolRun) -> String {
    let mut h = Sha256::new();
    h.update(run.exe.to_ascii_lowercase().as_bytes());
    for a in &run.args {
        h.update([0]);
        h.update(a.as_bytes());
    }
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    for i in std::iter::once(&run.exe).chain(&run.inputs) {
        let Some(rel) = crate::modlist::safe_rel(i) else { continue };
        let root = game_dir.join(&rel);
        let mut stack = vec![root];
        while let Some(p) = stack.pop() {
            if p.is_dir() {
                if let Ok(rd) = std::fs::read_dir(&p) {
                    stack.extend(rd.flatten().map(|e| e.path()));
                }
            } else if p.is_file() {
                let r = p.strip_prefix(game_dir).unwrap_or(&p).to_string_lossy().replace('\\', "/").to_ascii_lowercase();
                files.push((r, p));
            }
        }
    }
    files.sort();
    files.dedup_by(|a, b| a.0 == b.0);
    for (r, p) in files {
        let md = std::fs::metadata(&p).ok();
        let size = md.as_ref().map(|m| m.len()).unwrap_or(0);
        let mtime = md.and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
        h.update(format!("\n{r}|{size}|{mtime}").as_bytes());
    }
    hex::encode(h.finalize())
}

/// The program to run for a mod, when its `run` is due: the mod is
/// installed, the program is one of its installed files, and its inputs
/// changed since the last good run.
pub fn due(game_dir: &Path, m: &ModEntry) -> Result<Option<(PathBuf, Vec<String>, String)>> {
    let Some(run) = &m.run else { return Ok(None) };
    if !m.installed(game_dir) {
        return Ok(None);
    }
    let rel = crate::modlist::safe_rel(&run.exe).ok_or_else(|| Error::UnsafePath(run.exe.clone()))?;
    let rel_s = rel.to_string_lossy().replace('\\', "/");
    let installed = crate::modlist::load_installed(game_dir);
    let ours = installed.mods.get(&m.id).map(|r| r.files.iter().chain(&r.skipped).any(|f| f.eq_ignore_ascii_case(&rel_s))).unwrap_or(false);
    if !ours || !rel_s.to_ascii_lowercase().ends_with(".exe") {
        return Err(Error::Game(format!("{} isn't a program {} installed", run.exe, m.name)));
    }
    let exe = game_dir.join(&rel);
    if !exe.is_file() {
        return Err(Error::Game(format!("{} is missing", run.exe)));
    }
    let s = stamp(game_dir, run);
    if load(game_dir).get(&m.id).is_some_and(|d| d.stamp == s) {
        return Ok(None);
    }
    Ok(Some((exe, args_for(run, game_dir), s)))
}

/// Runs a mod's tool when it's due. Returns its exit code and how long it
/// took, or None when nothing needed running. Only a clean exit (0) is
/// recorded, so a failed run is tried again on the next Play.
pub fn run(game_dir: &Path, m: &ModEntry) -> Result<Option<(i32, Duration)>> {
    let Some((exe, args, s)) = due(game_dir, m)? else { return Ok(None) };
    let timeout = Duration::from_secs(m.run.as_ref().and_then(|r| r.timeout).unwrap_or(DEFAULT_TIMEOUT));
    let start = Instant::now();
    let code = spawn_minimized_and_wait(&exe, &args, timeout)?;
    let took = start.elapsed();
    if code == 0 {
        let mut r = load(game_dir);
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        r.insert(m.id.clone(), Done { stamp: s, when: now });
        save(game_dir, &r)?;
    }
    Ok(Some((code, took)))
}

/// Quotes one argument the way Windows programs split their command line.
pub fn quote(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
        return arg.to_string();
    }
    let mut out = String::from('"');
    let mut slashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => slashes += 1,
            '"' => {
                out.extend(std::iter::repeat_n('\\', slashes * 2 + 1));
                out.push('"');
                slashes = 0;
            }
            _ => {
                out.extend(std::iter::repeat_n('\\', slashes));
                out.push(c);
                slashes = 0;
            }
        }
    }
    out.extend(std::iter::repeat_n('\\', slashes * 2));
    out.push('"');
    out
}

#[cfg(windows)]
fn spawn_minimized_and_wait(exe: &Path, args: &[String], timeout: Duration) -> Result<i32> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_TIMEOUT};
    use windows_sys::Win32::System::Threading::{
        CreateProcessW, GetExitCodeProcess, TerminateProcess, WaitForSingleObject, CREATE_UNICODE_ENVIRONMENT, PROCESS_INFORMATION, STARTF_USESHOWWINDOW, STARTUPINFOW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWMINNOACTIVE;

    let wide = |s: &std::ffi::OsStr| s.encode_wide().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let mut line = quote(&exe.to_string_lossy());
    for a in args {
        line.push(' ');
        line.push_str(&quote(a));
    }
    let app = wide(exe.as_os_str());
    let mut cmd = wide(std::ffi::OsStr::new(&line));
    let dir = wide(exe.parent().unwrap_or(Path::new(".")).as_os_str());
    // SAFETY: plain Win32 calls with zeroed structs and NUL-terminated
    // UTF-16 strings that outlive the calls; both handles are closed.
    unsafe {
        let mut si: STARTUPINFOW = std::mem::zeroed();
        si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        si.dwFlags = STARTF_USESHOWWINDOW;
        si.wShowWindow = SW_SHOWMINNOACTIVE as u16;
        let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
        if CreateProcessW(app.as_ptr(), cmd.as_mut_ptr(), std::ptr::null(), std::ptr::null(), 0, CREATE_UNICODE_ENVIRONMENT, std::ptr::null(), dir.as_ptr(), &si, &mut pi) == 0 {
            return Err(Error::Game(format!("couldn't start {} ({})", exe.display(), std::io::Error::last_os_error())));
        }
        let ms = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX - 1);
        let waited = WaitForSingleObject(pi.hProcess, ms);
        let mut code: u32 = 1;
        if waited == WAIT_TIMEOUT {
            TerminateProcess(pi.hProcess, 1);
            WaitForSingleObject(pi.hProcess, 5000);
        } else {
            GetExitCodeProcess(pi.hProcess, &mut code);
        }
        CloseHandle(pi.hThread);
        CloseHandle(pi.hProcess);
        if waited == WAIT_TIMEOUT {
            return Err(Error::Game(format!("{} was still running after {} minutes and was stopped", exe.display(), timeout.as_secs() / 60)));
        }
        Ok(code as i32)
    }
}

#[cfg(not(windows))]
fn spawn_minimized_and_wait(exe: &Path, args: &[String], timeout: Duration) -> Result<i32> {
    let mut child = std::process::Command::new(exe).args(args).current_dir(exe.parent().unwrap_or(Path::new("."))).spawn()?;
    let start = Instant::now();
    loop {
        if let Some(st) = child.try_wait()? {
            return Ok(st.code().unwrap_or(1));
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::Game(format!("{} was still running after {} minutes and was stopped", exe.display(), timeout.as_secs() / 60)));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn quotes_like_windows() {
        assert_eq!(quote("plain"), "plain");
        assert_eq!(quote("A:\\steam\\Skyrim Special Edition"), "\"A:\\steam\\Skyrim Special Edition\"");
        assert_eq!(quote("--tesv:A:\\my game\\"), "\"--tesv:A:\\my game\\\\\"");
        assert_eq!(quote("say \"hi\""), "\"say \\\"hi\\\"\"");
        assert_eq!(quote(""), "\"\"");
    }

    #[cfg(unix)]
    #[test]
    fn runs_once_until_an_input_changes() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let g = t.path();
        let exe_rel = "Data/CalienteTools/BodySlide/BodySlide.exe";
        let exe = g.join(exe_rel);
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, "#!/bin/sh\necho \"$@\" >> ran.txt\nexit 0\n").unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::create_dir_all(g.join("Data/CalienteTools/BodySlide/SliderPresets")).unwrap();
        std::fs::write(g.join("Data/CalienteTools/BodySlide/SliderPresets/a.xml"), "1").unwrap();
        let m: ModEntry = serde_json::from_value(json!({"id": "bodyslide", "name": "BodySlide", "run": {
            "label": "Fitting armor to bodies", "exe": exe_rel, "args": ["--groupbuild", "3BA", "-t", "{data}"],
            "inputs": ["Data/CalienteTools/BodySlide/SliderPresets"]}})).unwrap();
        // Not installed by the launcher: never run.
        assert!(due(g, &m).unwrap().is_none());
        let rec = json!({"mods": {"bodyslide": {"name": "BodySlide", "files": [exe_rel], "when": 1}}});
        std::fs::create_dir_all(g.join(".aetherial-dawn/mods")).unwrap();
        std::fs::write(g.join(".aetherial-dawn/mods/installed.json"), rec.to_string()).unwrap();
        let (code, _) = run(g, &m).unwrap().unwrap();
        assert_eq!(code, 0);
        let ran = std::fs::read_to_string(exe.parent().unwrap().join("ran.txt")).unwrap();
        assert_eq!(ran.trim(), format!("--groupbuild 3BA -t {}", g.join("Data").display()));
        assert!(run(g, &m).unwrap().is_none(), "nothing changed");
        std::fs::write(g.join("Data/CalienteTools/BodySlide/SliderPresets/b.xml"), "2").unwrap();
        assert!(run(g, &m).unwrap().is_some(), "a new preset runs it again");

        // A program the mod didn't install is refused.
        let other = ModEntry { run: Some(ToolRun { exe: "Data/other.exe".into(), ..m.run.clone().unwrap() }), ..m.clone() };
        assert!(due(g, &other).is_err());
    }
}
