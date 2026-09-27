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
const DEFAULT_TIMEOUT: u64 = 10 * 60;
/// Failed runs with the same inputs before the launcher stops trying until
/// something changes.
const MAX_FAILURES: u32 = 2;

/// Set by the player's "Skip for now": the running tool is stopped and the
/// game starts; it runs again on the next Play.
static SKIP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn skip() {
    SKIP.store(true, std::sync::atomic::Ordering::SeqCst);
}

fn skipped() -> bool {
    SKIP.load(std::sync::atomic::Ordering::SeqCst)
}

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
    /// Seconds before it is stopped (default 10 minutes).
    #[serde(default)]
    pub timeout: Option<u64>,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct Done {
    #[serde(default)]
    stamp: String,
    #[serde(default)]
    when: u64,
    /// Failed runs with these inputs (a different stamp from the good one).
    #[serde(default)]
    failed_stamp: String,
    #[serde(default)]
    failures: u32,
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
            // Links and junctions aren't followed (a loop, or a whole drive).
            let Ok(md) = std::fs::symlink_metadata(&p) else { continue };
            if md.file_type().is_symlink() {
                continue;
            }
            if md.is_dir() {
                if let Ok(rd) = std::fs::read_dir(&p) {
                    stack.extend(rd.flatten().map(|e| e.path()));
                }
            } else if md.is_file() {
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
    if load(game_dir).get(&m.id).is_some_and(|d| d.stamp == s || (d.failed_stamp == s && d.failures >= MAX_FAILURES)) {
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
    SKIP.store(false, std::sync::atomic::Ordering::SeqCst);
    let got = spawn_minimized_and_wait(&exe, &args, timeout);
    let took = start.elapsed();
    if matches!(&got, Err(Error::Game(e)) if e == SKIPPED) {
        return got.map(|c| Some((c, took)));
    }
    let mut r = load(game_dir);
    let d = r.entry(m.id.clone()).or_default();
    match &got {
        Ok(0) => {
            d.stamp = s;
            d.when = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            d.failed_stamp.clear();
            d.failures = 0;
        }
        _ => {
            if d.failed_stamp != s {
                d.failed_stamp = s;
                d.failures = 0;
            }
            d.failures += 1;
        }
    }
    save(game_dir, &r)?;
    got.map(|c| Some((c, took)))
}

/// The error text when the player skipped the run.
pub const SKIPPED: &str = "skipped for now";

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
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows_sys::Win32::System::Threading::{
        CreateProcessW, GetExitCodeProcess, ResumeThread, TerminateProcess, WaitForSingleObject, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, PROCESS_INFORMATION, STARTF_USESHOWWINDOW, STARTUPINFOW,
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
    // UTF-16 strings that outlive the calls; every handle is closed.
    unsafe {
        // A job that ends the tool and anything it started when it's
        // stopped (or when the launcher closes).
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if !job.is_null() {
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(job, JobObjectExtendedLimitInformation, &info as *const _ as *const _, std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32);
        }
        let mut si: STARTUPINFOW = std::mem::zeroed();
        si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        si.dwFlags = STARTF_USESHOWWINDOW;
        si.wShowWindow = SW_SHOWMINNOACTIVE as u16;
        let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
        if CreateProcessW(app.as_ptr(), cmd.as_mut_ptr(), std::ptr::null(), std::ptr::null(), 0, CREATE_UNICODE_ENVIRONMENT | CREATE_SUSPENDED, std::ptr::null(), dir.as_ptr(), &si, &mut pi) == 0 {
            let e = std::io::Error::last_os_error();
            if !job.is_null() {
                CloseHandle(job);
            }
            return Err(Error::Game(format!("couldn't start {} ({e})", exe.display())));
        }
        if !job.is_null() {
            AssignProcessToJobObject(job, pi.hProcess);
        }
        ResumeThread(pi.hThread);
        let start = Instant::now();
        let mut stop: Option<String> = None;
        loop {
            if WaitForSingleObject(pi.hProcess, 250) != WAIT_TIMEOUT {
                break;
            }
            if skipped() {
                stop = Some(SKIPPED.to_string());
            } else if start.elapsed() > timeout {
                stop = Some(format!("{} was still running after {} minutes and was stopped", exe.display(), timeout.as_secs() / 60));
            }
            if stop.is_some() {
                if job.is_null() {
                    TerminateProcess(pi.hProcess, 1);
                } else {
                    TerminateJobObject(job, 1);
                }
                WaitForSingleObject(pi.hProcess, 5000);
                break;
            }
        }
        let mut code: u32 = 1;
        GetExitCodeProcess(pi.hProcess, &mut code);
        CloseHandle(pi.hThread);
        CloseHandle(pi.hProcess);
        if !job.is_null() {
            // Anything the tool left running ends with the job.
            CloseHandle(job);
        }
        match stop {
            Some(e) => Err(Error::Game(e)),
            None => Ok(code as i32),
        }
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
        if skipped() || start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            if skipped() {
                return Err(Error::Game(SKIPPED.into()));
            }
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

        // Two failures with the same inputs: no more tries until they change.
        std::fs::write(&exe, "#!/bin/sh\nexit 3\n").unwrap();
        assert_eq!(run(g, &m).unwrap().unwrap().0, 3);
        assert_eq!(run(g, &m).unwrap().unwrap().0, 3);
        assert!(run(g, &m).unwrap().is_none(), "gave up on these inputs");
        std::fs::write(g.join("Data/CalienteTools/BodySlide/SliderPresets/c.xml"), "3").unwrap();
        assert!(run(g, &m).unwrap().is_some(), "new inputs get another try");

        // A linked folder isn't followed.
        std::os::unix::fs::symlink(g, g.join("Data/CalienteTools/BodySlide/SliderPresets/loop")).unwrap();
        let _ = stamp(g, m.run.as_ref().unwrap());

        // A program the mod didn't install is refused.
        let other = ModEntry { run: Some(ToolRun { exe: "Data/other.exe".into(), ..m.run.clone().unwrap() }), ..m.clone() };
        assert!(due(g, &other).is_err());
    }
}
