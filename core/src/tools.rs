//! Tools a listed mod ships that normally need a person to press a button:
//! BodySlide fitting armor to the bodies (TOOLS-AND-BODIES.md). The launcher
//! runs it itself, minimized, after the mods install and before the game
//! starts, and only again when something it reads changes (Timothy's
//! one-button rule). Pandora is not supported: it is not on the allow-list.
//!
//! The program must be a file of that same mod, installed by the launcher or
//! named in the mod's own checks (installed through Vortex), so a list can
//! only start a tool that came from Nexus with the mod, and only one on the
//! launcher's own allow-list (BodySlide) under Data.
//!
//! A mod can list several runs (BodySlide once with the women's preset and
//! once with the men's). A run can build into its own folder first; its
//! files are then moved into Data, replacing what's there rather than
//! writing through Vortex's hard links into its staging folder.

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
/// The only programs a list can start, by file name, and only from under
/// Data (quality check 0.1.73: never SkyrimSE.exe, the SKSE loader or a
/// Vortex tool, whatever the mod's checks name).
const ALLOWED: [&str; 2] = ["BodySlide x64.exe", "BodySlide.exe"];

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
    /// Settings written right before each run (BodySlide's Config.xml
    /// SelectedPreset, which 5.8.x reads instead of its -p option).
    #[serde(default)]
    pub before: Vec<crate::presets::Setting>,
    /// A folder under ".aetherial-dawn/tools/" ("{out}" in args) the tool
    /// builds into; after a clean run its files move into Data.
    #[serde(default)]
    pub output: Option<String>,
    /// BodySlide outfit files (.osp under Data/CalienteTools/BodySlide/
    /// SliderSets/) for refits that ship no group of their own: the launcher
    /// reads the SliderSet names from the installed files and writes them as
    /// one group, SliderGroups/AetherialDawn-<id>.xml, which "{group}" in
    /// args names. A missing file is logged and left out, never guessed.
    #[serde(default)]
    pub group_from: Vec<String>,
}

const SLIDER_SETS: &str = "data/calientetools/bodyslide/slidersets/";
const SLIDER_GROUPS: &str = "Data/CalienteTools/BodySlide/SliderGroups";

/// The group a run writes, and what went into it.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub name: String,
    pub members: Vec<String>,
    /// Listed .osp files that aren't installed or can't be read.
    pub missing: Vec<String>,
}

/// The group name of a run with `group_from` ("AetherialDawn-<id>").
/// Only [A-Za-z0-9_-] from the mod id, so the name is always one plain
/// file name (quality check 0.1.78: an id with "/", "..", "\\" or ":" must
/// never reach outside SliderGroups).
pub fn group_name(m: &ModEntry, i: usize) -> String {
    let id: String = key(m, i).chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '-' }).collect();
    format!("AetherialDawn-{id}")
}

fn group_paths(run: &ToolRun) -> Result<Vec<PathBuf>> {
    run.group_from
        .iter()
        .map(|g| {
            crate::modlist::safe_rel(g)
                .filter(|r| {
                    let l = r.to_string_lossy().replace('\\', "/").to_ascii_lowercase();
                    l.starts_with(SLIDER_SETS) && l.ends_with(".osp")
                })
                .ok_or_else(|| Error::UnsafePath(g.clone()))
        })
        .collect()
}

/// Reads the SliderSet names from a run's `group_from` files and writes
/// them as its group (only when that changes the file). Nothing is written
/// when no listed file is installed.
pub fn write_group(game_dir: &Path, m: &ModEntry, i: usize) -> Result<Group> {
    let Some(run) = m.run.get(i) else { return Err(Error::Game(format!("{} has no run {i}", m.name))) };
    let name = group_name(m, i);
    let mut members: Vec<String> = Vec::new();
    let mut missing = Vec::new();
    for (rel, listed) in group_paths(run)?.iter().zip(&run.group_from) {
        let path = crate::presets::find(game_dir, &rel.to_string_lossy().replace('\\', "/"));
        let names = std::fs::read(&path).ok().and_then(|b| {
            let text = String::from_utf8_lossy(b.strip_prefix(b"\xEF\xBB\xBF".as_slice()).unwrap_or(&b)).into_owned();
            let doc = roxmltree::Document::parse(&text).ok()?;
            let n: Vec<String> = doc.descendants().filter(|e| e.has_tag_name("SliderSet")).filter_map(|e| e.attribute("name").map(str::to_string)).collect();
            (!n.is_empty()).then_some(n)
        });
        match names {
            Some(n) => {
                for x in n {
                    if !members.iter().any(|y| y.eq_ignore_ascii_case(&x)) {
                        members.push(x);
                    }
                }
            }
            None => missing.push(listed.clone()),
        }
    }
    if !members.is_empty() {
        let esc = |t: &str| t.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;");
        let mut xml = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<SliderGroups>\n    <Group name=\"{}\">\n", esc(&name));
        for x in &members {
            xml.push_str(&format!("        <Member name=\"{}\"/>\n", esc(x)));
        }
        xml.push_str("    </Group>\n</SliderGroups>\n");
        let groups = crate::presets::find(game_dir, SLIDER_GROUPS);
        let file = groups.join(format!("{name}.xml"));
        if file.parent() != Some(groups.as_path()) || !groups.starts_with(game_dir) {
            return Err(Error::UnsafePath(file.display().to_string()));
        }
        if std::fs::read(&file).ok().as_deref() != Some(xml.as_bytes()) {
            crate::presets::replace(&file, xml.as_bytes())?;
        }
    }
    Ok(Group { name, members, missing })
}

/// `run` in mods.json: one run or a list of them.
pub fn one_or_many<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Vec<ToolRun>, D::Error> {
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(Box<ToolRun>),
        Many(Vec<ToolRun>),
    }
    Ok(match <Option<OneOrMany> as serde::Deserialize>::deserialize(d)? {
        None => Vec::new(),
        Some(OneOrMany::One(r)) => vec![*r],
        Some(OneOrMany::Many(v)) => v,
    })
}


/// A program under Data whose file name is on the allow-list.
fn allowed(rel: &str) -> bool {
    let l = rel.to_ascii_lowercase();
    let name = l.rsplit('/').next().unwrap_or("");
    l.starts_with("data/") && ALLOWED.iter().any(|a| a.eq_ignore_ascii_case(name))
}

/// The record key of a mod's run: its id for the first, "<id>#<n>" after.
fn key(m: &ModEntry, i: usize) -> String {
    if i == 0 { m.id.clone() } else { format!("{}#{i}", m.id) }
}

/// The folder a run builds into, when it's a safe one.
fn output_dir(game_dir: &Path, run: &ToolRun) -> Result<Option<PathBuf>> {
    let Some(o) = &run.output else { return Ok(None) };
    let rel = crate::modlist::safe_rel(o).filter(|r| r.to_string_lossy().replace('\\', "/").to_ascii_lowercase().starts_with(".aetherial-dawn/tools/")).ok_or_else(|| Error::UnsafePath(o.clone()))?;
    Ok(Some(game_dir.join(rel)))
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
    crate::atomicfile::write(&p, &serde_json::to_vec_pretty(r)?)?;
    Ok(())
}

fn args_for(run: &ToolRun, game_dir: &Path, group: &str) -> Vec<String> {
    let game = game_dir.to_string_lossy();
    let data = game_dir.join("Data").to_string_lossy().into_owned();
    let out = run.output.as_ref().and_then(|o| crate::modlist::safe_rel(o)).map(|r| game_dir.join(r).to_string_lossy().into_owned()).unwrap_or_default();
    run.args.iter().map(|a| a.replace("{game}", &game).replace("{data}", &data).replace("{out}", &out).replace("{group}", group)).collect()
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
    // The settings it's given count too (a changed preset builds again).
    h.update(serde_json::to_vec(&run.before).unwrap_or_default());
    h.update([0]);
    h.update(run.output.as_deref().unwrap_or("").as_bytes());
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    // The outfit files a group is read from count as inputs.
    for i in std::iter::once(&run.exe).chain(&run.inputs).chain(&run.group_from) {
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
pub fn due(game_dir: &Path, m: &ModEntry, i: usize) -> Result<Option<(PathBuf, Vec<String>, String)>> {
    let Some(run) = m.run.get(i) else { return Ok(None) };
    if !m.installed(game_dir) {
        return Ok(None);
    }
    let rel = crate::modlist::safe_rel(&run.exe).ok_or_else(|| Error::UnsafePath(run.exe.clone()))?;
    let rel_s = rel.to_string_lossy().replace('\\', "/");
    let installed = crate::modlist::load_installed(game_dir);
    // The launcher's own record, or the mod's checks (Vortex installed it:
    // installed() above already found every checked file).
    let ours = installed.mods.get(&m.id).map(|r| r.files.iter().chain(&r.skipped).any(|f| f.eq_ignore_ascii_case(&rel_s))).unwrap_or(false)
        || m.check.iter().any(|c| c.eq_ignore_ascii_case(&rel_s));
    if !ours || !rel_s.to_ascii_lowercase().ends_with(".exe") {
        return Err(Error::Game(format!("{} isn't a program {} installed", run.exe, m.name)));
    }
    if !allowed(&rel_s) {
        return Err(Error::Game(format!("{} isn't a tool the launcher runs", run.exe)));
    }
    let exe = game_dir.join(&rel);
    if !exe.is_file() {
        return Err(Error::Game(format!("{} is missing", run.exe)));
    }
    output_dir(game_dir, run)?;
    group_paths(run)?;
    let s = stamp(game_dir, run);
    if load(game_dir).get(&key(m, i)).is_some_and(|d| d.stamp == s || (d.failed_stamp == s && d.failures >= MAX_FAILURES)) {
        return Ok(None);
    }
    Ok(Some((exe, args_for(run, game_dir, &group_name(m, i)), s)))
}

/// Runs a mod's tool when it's due. Returns its exit code and how long it
/// took, or None when nothing needed running. Only a clean exit (0) is
/// recorded, so a failed run is tried again on the next Play.
pub fn run(game_dir: &Path, m: &ModEntry, i: usize) -> Result<Option<(i32, Duration)>> {
    let Some((exe, args, s)) = due(game_dir, m, i)? else { return Ok(None) };
    let spec = &m.run[i];
    let timeout = Duration::from_secs(spec.timeout.unwrap_or(DEFAULT_TIMEOUT));
    // The settings files as they were, put back after the run (BodySlide
    // opens on the player's own preset again).
    let kept: Vec<(PathBuf, Option<Vec<u8>>)> = spec
        .before
        .iter()
        .map(|b| {
            let p = crate::presets::find(game_dir, &b.file);
            let bytes = std::fs::read(&p).ok();
            (p, bytes)
        })
        .collect();
    let put_back = || {
        for (p, bytes) in kept.iter().rev() {
            let _ = match bytes {
                Some(b) => crate::presets::replace(p, b),
                None => std::fs::remove_file(p),
            };
        }
    };
    for b in &spec.before {
        let left = match crate::presets::write_now(game_dir, b) {
            Ok(l) => l,
            Err(e) => {
                put_back();
                return Err(e.into());
            }
        };
        if !left.is_empty() {
            put_back();
            return Err(Error::Game(format!("couldn't set {} before running it", left.join(", "))));
        }
    }
    let out = output_dir(game_dir, spec)?;
    if let Some(o) = &out {
        let _ = std::fs::remove_dir_all(o);
        std::fs::create_dir_all(o)?;
    }
    let start = Instant::now();
    SKIP.store(false, std::sync::atomic::Ordering::SeqCst);
    let got = spawn_minimized_and_wait(&exe, &args, timeout);
    let took = start.elapsed();
    put_back();
    if matches!(&got, Err(Error::Game(e)) if e == SKIPPED) {
        return got.map(|c| Some((c, took)));
    }
    // A build that couldn't move into Data counts as a failed run, so it
    // stops after MAX_FAILURES like any other.
    let got = match (got, &out) {
        (Ok(0), Some(o)) => move_into_data(o, &game_dir.join("Data")).map(|_| 0),
        (g, _) => g,
    };
    let mut r = load(game_dir);
    let d = r.entry(key(m, i)).or_default();
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

/// Moves everything a tool built into Data at the same paths. An existing
/// file is removed first, so a Vortex hard link is replaced and its staging
/// copy stays as it was.
fn move_into_data(out: &Path, data: &Path) -> Result<usize> {
    let mut n = 0;
    let mut stack = vec![out.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d)?.flatten() {
            let p = e.path();
            let ft = e.file_type()?;
            if ft.is_symlink() {
                continue;
            }
            if ft.is_dir() {
                stack.push(p);
                continue;
            }
            let rel = p.strip_prefix(out).map_err(|_| Error::UnsafePath(p.display().to_string()))?;
            let dest = data.join(rel);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            if dest.exists() {
                std::fs::remove_file(&dest)?;
            }
            if std::fs::rename(&p, &dest).is_err() {
                std::fs::copy(&p, &dest)?;
                std::fs::remove_file(&p)?;
            }
            n += 1;
        }
    }
    let _ = std::fs::remove_dir_all(out);
    Ok(n)
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
        assert!(due(g, &m, 0).unwrap().is_none());
        let rec = json!({"mods": {"bodyslide": {"name": "BodySlide", "files": [exe_rel], "when": 1}}});
        std::fs::create_dir_all(g.join(".aetherial-dawn/mods")).unwrap();
        std::fs::write(g.join(".aetherial-dawn/mods/installed.json"), rec.to_string()).unwrap();
        let (code, _) = run(g, &m, 0).unwrap().unwrap();
        assert_eq!(code, 0);
        let ran = std::fs::read_to_string(exe.parent().unwrap().join("ran.txt")).unwrap();
        assert_eq!(ran.trim(), format!("--groupbuild 3BA -t {}", g.join("Data").display()));
        assert!(run(g, &m, 0).unwrap().is_none(), "nothing changed");
        std::fs::write(g.join("Data/CalienteTools/BodySlide/SliderPresets/b.xml"), "2").unwrap();
        assert!(run(g, &m, 0).unwrap().is_some(), "a new preset runs it again");

        // Two failures with the same inputs: no more tries until they change.
        std::fs::write(&exe, "#!/bin/sh\nexit 3\n").unwrap();
        assert_eq!(run(g, &m, 0).unwrap().unwrap().0, 3);
        assert_eq!(run(g, &m, 0).unwrap().unwrap().0, 3);
        assert!(run(g, &m, 0).unwrap().is_none(), "gave up on these inputs");
        std::fs::write(g.join("Data/CalienteTools/BodySlide/SliderPresets/c.xml"), "3").unwrap();
        assert!(run(g, &m, 0).unwrap().is_some(), "new inputs get another try");

        // A linked folder isn't followed.
        std::os::unix::fs::symlink(g, g.join("Data/CalienteTools/BodySlide/SliderPresets/loop")).unwrap();
        let _ = stamp(g, &m.run[0]);

        // A program the mod didn't install is refused.
        let other = ModEntry { run: vec![ToolRun { exe: "Data/other.exe".into(), ..m.run[0].clone() }], ..m.clone() };
        assert!(due(g, &other, 0).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn bodyslide_twice_into_a_side_folder() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let g = t.path();
        let bs = g.join("Data/CalienteTools/BodySlide");
        std::fs::create_dir_all(bs.join("SliderPresets")).unwrap();
        std::fs::write(bs.join("Config.xml"), "<Config>\n    <SelectedPreset>CBBE</SelectedPreset>\n</Config>\n").unwrap();
        // A stand-in BodySlide: builds a mesh named after the selected preset into -t.
        let exe = bs.join("BodySlide.exe");
        std::fs::write(&exe, "#!/bin/sh\np=$(sed -n 's/.*<SelectedPreset>\\(.*\\)<.*/\\1/p' Config.xml)\nmkdir -p \"$4/meshes/armor\"\necho \"$p\" > \"$4/meshes/armor/$2.nif\"\n").unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        // Vortex's deployed mesh, hard-linked from its staging folder.
        std::fs::create_dir_all(g.join("staging/meshes/armor")).unwrap();
        std::fs::write(g.join("staging/meshes/armor/3BA.nif"), "vortex").unwrap();
        std::fs::create_dir_all(g.join("Data/meshes/armor")).unwrap();
        std::fs::hard_link(g.join("staging/meshes/armor/3BA.nif"), g.join("Data/meshes/armor/3BA.nif")).unwrap();
        let spec = |group: &str, preset: &str| {
            json!({"label": "Fitting armor to bodies", "exe": "Data/CalienteTools/BodySlide/BodySlide.exe",
                "args": ["--groupbuild", group, "--targetdir", "{out}", "-p", preset, "--trimorphs"],
                "inputs": ["Data/CalienteTools/BodySlide/SliderPresets"], "output": ".aetherial-dawn/tools/bodyslide",
                "before": [{"file": "Data/CalienteTools/BodySlide/Config.xml", "format": "xml", "set": {"Config.SelectedPreset": preset}}]})
        };
        // Installed through Vortex: no launcher record, the program is in the checks.
        let m: ModEntry = serde_json::from_value(json!({"id": "bodyslide", "name": "BodySlide", "check": ["Data/CalienteTools/BodySlide/BodySlide.exe"],
            "run": [spec("3BA", "3BA Natural"), spec("HIMBO", "HIMBO Default")]})).unwrap();
        assert_eq!(m.run.len(), 2);
        assert_eq!(run(g, &m, 0).unwrap().unwrap().0, 0);
        assert_eq!(run(g, &m, 1).unwrap().unwrap().0, 0);
        assert_eq!(std::fs::read_to_string(g.join("Data/meshes/armor/3BA.nif")).unwrap().trim(), "3BA Natural");
        assert_eq!(std::fs::read_to_string(g.join("Data/meshes/armor/HIMBO.nif")).unwrap().trim(), "HIMBO Default");
        assert_eq!(std::fs::read_to_string(g.join("staging/meshes/armor/3BA.nif")).unwrap(), "vortex", "Vortex's staging copy is untouched");
        assert!(!g.join(".aetherial-dawn/tools/bodyslide").exists());
        assert!(std::fs::read_to_string(bs.join("Config.xml")).unwrap().contains("<SelectedPreset>CBBE</SelectedPreset>"), "the player's own preset is back");
        // Both recorded: nothing runs again until something changes.
        assert!(due(g, &m, 0).unwrap().is_none() && due(g, &m, 1).unwrap().is_none());
        // A new preset in the list builds that run again.
        let m2: ModEntry = serde_json::from_value(json!({"id": "bodyslide", "name": "BodySlide", "check": ["Data/CalienteTools/BodySlide/BodySlide.exe"],
            "run": [spec("3BA", "3BA Curvy"), spec("HIMBO", "HIMBO Default")]})).unwrap();
        assert!(due(g, &m2, 0).unwrap().is_some() && due(g, &m2, 1).unwrap().is_none());
        // One run as an object still reads.
        let one: ModEntry = serde_json::from_value(json!({"id": "x", "name": "X", "run": spec("3BA", "a")})).unwrap();
        assert_eq!(one.run.len(), 1);
        // An output folder outside the launcher's tools folder is refused.
        let bad: ModEntry = serde_json::from_value(json!({"id": "y", "name": "Y", "check": ["Data/CalienteTools/BodySlide/BodySlide.exe"],
            "run": {"label": "x", "exe": "Data/CalienteTools/BodySlide/BodySlide.exe", "output": "Data/meshes"}})).unwrap();
        assert!(due(g, &bad, 0).is_err());
    }

    #[test]
    fn a_group_is_read_from_the_installed_outfit_files() {
        let t = tempfile::tempdir().unwrap();
        let g = t.path();
        let sets = g.join("Data/CalienteTools/BodySlide/SliderSets");
        std::fs::create_dir_all(&sets).unwrap();
        std::fs::write(sets.join("Refit A.osp"), "\u{feff}<?xml version=\"1.0\"?>\n<SliderSetInfo version=\"1\"><SliderSet name=\"Armor A 3BA\"/><SliderSet name=\"Armor A Gauntlets &amp; Boots\"/></SliderSetInfo>").unwrap();
        std::fs::write(sets.join("refit b.osp"), "<SliderSetInfo><SliderSet name=\"Robe B\"/><SliderSet name=\"armor a 3ba\"/></SliderSetInfo>").unwrap();
        std::fs::write(sets.join("broken.osp"), "not xml").unwrap();
        let m: ModEntry = serde_json::from_value(json!({"id": "bodyslide", "name": "BodySlide", "run": [{"label": "y", "exe": "Data/CalienteTools/BodySlide/BodySlide x64.exe"}, {"label": "x", "exe": "Data/CalienteTools/BodySlide/BodySlide x64.exe",
            "args": ["--groupbuild", "{group}"],
            "group_from": ["Data/CalienteTools/BodySlide/SliderSets/Refit A.osp", "Data/CalienteTools/BodySlide/SliderSets/Refit B.osp", "Data/CalienteTools/BodySlide/SliderSets/Missing.osp", "Data/CalienteTools/BodySlide/SliderSets/broken.osp"]}]})).unwrap();
        let grp = write_group(g, &m, 1).unwrap();
        assert_eq!(grp.name, "AetherialDawn-bodyslide-1");
        assert_eq!(grp.members, vec!["Armor A 3BA", "Armor A Gauntlets & Boots", "Robe B"]);
        assert_eq!(grp.missing, vec!["Data/CalienteTools/BodySlide/SliderSets/Missing.osp", "Data/CalienteTools/BodySlide/SliderSets/broken.osp"]);
        let file = g.join("Data/CalienteTools/BodySlide/SliderGroups/AetherialDawn-bodyslide-1.xml");
        let xml = std::fs::read_to_string(&file).unwrap();
        assert!(xml.contains("<Group name=\"AetherialDawn-bodyslide-1\">") && xml.contains("<Member name=\"Armor A Gauntlets &amp; Boots\"/>"), "{xml}");
        // BodySlide's own reader accepts it as XML.
        roxmltree::Document::parse(&xml).unwrap();
        // Unchanged content isn't rewritten (it would count as a changed input).
        let before = std::fs::metadata(&file).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        write_group(g, &m, 1).unwrap();
        assert_eq!(std::fs::metadata(&file).unwrap().modified().unwrap(), before);
        assert_eq!(args_for(&m.run[1], g, &group_name(&m, 1)), vec!["--groupbuild", "AetherialDawn-bodyslide-1"]);
        // Nothing installed: nothing written.
        let none: ModEntry = serde_json::from_value(json!({"id": "z", "name": "Z", "run": {"label": "x", "exe": "Data/CalienteTools/BodySlide/BodySlide x64.exe", "group_from": ["Data/CalienteTools/BodySlide/SliderSets/Gone.osp"]}})).unwrap();
        assert!(write_group(g, &none, 0).unwrap().members.is_empty());
        assert!(!g.join("Data/CalienteTools/BodySlide/SliderGroups/AetherialDawn-z.xml").exists());
        // Whatever the mod id holds, the group stays one file in SliderGroups.
        for id in ["x/../../../Users/Public/foo", "a\\..\\..\\b", "C:evil", "..", "we ird#1"] {
            let m: ModEntry = serde_json::from_value(json!({"id": id, "name": "Z", "run": {"label": "x", "exe": "Data/CalienteTools/BodySlide/BodySlide x64.exe", "group_from": ["Data/CalienteTools/BodySlide/SliderSets/Refit A.osp"]}})).unwrap();
            let grp = write_group(g, &m, 0).unwrap();
            assert!(grp.name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'), "{id}: {}", grp.name);
            assert!(g.join("Data/CalienteTools/BodySlide/SliderGroups").join(format!("{}.xml", grp.name)).is_file(), "{id}");
        }
        assert!(!g.join("Users").exists() && !t.path().parent().unwrap().join("Users").exists());
        // Only .osp files under SliderSets.
        for bad in ["Data/SKSE/Plugins/x.osp", "Data/CalienteTools/BodySlide/SliderSets/../../x.osp", "Data/CalienteTools/BodySlide/SliderSets/a.xml"] {
            let m: ModEntry = serde_json::from_value(json!({"id": "z", "name": "Z", "run": {"label": "x", "exe": "Data/CalienteTools/BodySlide/BodySlide x64.exe", "group_from": [bad]}})).unwrap();
            assert!(write_group(g, &m, 0).is_err(), "{bad}");
        }
    }

    #[test]
    fn only_bodyslide_under_data_runs() {
        let t = tempfile::tempdir().unwrap();
        let g = t.path();
        for rel in ["SkyrimSE.exe", "skse64_loader.exe", "Data/skse64_loader.exe", "Data/CalienteTools/BodySlide/BodySlide x64.exe", "BodySlide x64.exe"] {
            let p = g.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, "x").unwrap();
        }
        let with = |exe: &str| -> ModEntry { serde_json::from_value(json!({"id": "t", "name": "T", "check": [exe], "run": {"label": "x", "exe": exe}})).unwrap() };
        for bad in ["SkyrimSE.exe", "skse64_loader.exe", "Data/skse64_loader.exe", "BodySlide x64.exe"] {
            let e = due(g, &with(bad), 0).unwrap_err().to_string();
            assert!(e.contains("isn't a tool"), "{bad}: {e}");
        }
        assert!(due(g, &with("Data/CalienteTools/BodySlide/BodySlide x64.exe"), 0).unwrap().is_some());
    }

    #[cfg(unix)]
    #[test]
    fn a_build_that_cant_move_in_counts_as_a_failure() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let g = t.path();
        let exe = g.join("Data/CalienteTools/BodySlide/BodySlide.exe");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, "#!/bin/sh\nmkdir -p \"$2/meshes\"\necho x > \"$2/meshes/a.nif\"\n").unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        // A folder where the built file goes: the move fails.
        std::fs::create_dir_all(g.join("Data/meshes/a.nif/inside")).unwrap();
        let m: ModEntry = serde_json::from_value(json!({"id": "bodyslide", "name": "BodySlide", "check": ["Data/CalienteTools/BodySlide/BodySlide.exe"],
            "run": {"label": "x", "exe": "Data/CalienteTools/BodySlide/BodySlide.exe", "args": ["-t", "{out}"], "output": ".aetherial-dawn/tools/bodyslide"}})).unwrap();
        assert!(run(g, &m, 0).is_err());
        assert!(run(g, &m, 0).is_err());
        assert!(run(g, &m, 0).unwrap().is_none(), "stops after two failed moves");
    }
}
