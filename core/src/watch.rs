//! Watches Skyrim after Play, so a crash leaves a report behind: how long the
//! game ran, how it ended, and the logs SKSE, SkyrimPlatform and crash loggers
//! wrote during that session.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

pub const GAME_PROCESS: &str = "SkyrimSE.exe";

/// SKSE's loader, which starts SkyrimSE.exe and then exits.
pub const LOADER_PROCESS: &str = "skse64_loader.exe";
/// How long Play waits for SkyrimSE.exe once the loader has gone.
pub const START_WAIT: Duration = Duration::from_secs(90);
/// The most Play waits while the loader is still running (Steam starting or
/// updating first can hold it for minutes).
pub const START_WAIT_MAX: Duration = Duration::from_secs(10 * 60);

/// Waits for the game to appear after Play and returns its process id, or
/// None when it never did. The 90 seconds count from when SKSE's loader was
/// last seen, not from Play, so a slow start (Steam signing in or updating)
/// isn't taken for a crash; `max` bounds the whole wait. `find` looks up a
/// process by name (`find_process`); one check per `tick`.
pub async fn wait_for_game(find: impl Fn(&str) -> Option<u32>, tick: Duration, wait: Duration, max: Duration) -> Option<u32> {
    let start = tokio::time::Instant::now();
    let mut quiet_since = start;
    loop {
        if let Some(pid) = find(GAME_PROCESS) {
            return Some(pid);
        }
        let now = tokio::time::Instant::now();
        if find(LOADER_PROCESS).is_some() {
            quiet_since = now;
        }
        if now.duration_since(quiet_since) >= wait || now.duration_since(start) >= max {
            return None;
        }
        tokio::time::sleep(tick).await;
    }
}

/// Process id of a running process with this file name.
pub fn find_process(name: &str) -> Option<u32> {
    find_process_checked(name).ok().flatten()
}

/// Process id of a running process, or an error if the process list could not
/// be read. Callers that must prove a game is absent before changing the
/// launcher should use this rather than treating an enumeration error as None.
#[cfg(windows)]
pub fn find_process_checked(name: &str) -> std::io::Result<Option<u32>> {
    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_NO_MORE_FILES, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
    };
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return Err(std::io::Error::last_os_error());
        }
        let result = (|| {
            let mut e: PROCESSENTRY32W = std::mem::zeroed();
            e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            if Process32FirstW(snap, &mut e) == 0 {
                let error = std::io::Error::last_os_error();
                return if error.raw_os_error() == Some(ERROR_NO_MORE_FILES as i32) { Ok(None) } else { Err(error) };
            }
            loop {
                let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
                if String::from_utf16_lossy(&e.szExeFile[..len]).eq_ignore_ascii_case(name) {
                    return Ok(Some(e.th32ProcessID));
                }
                if Process32NextW(snap, &mut e) == 0 {
                    let error = std::io::Error::last_os_error();
                    return if error.raw_os_error() == Some(ERROR_NO_MORE_FILES as i32) { Ok(None) } else { Err(error) };
                }
            }
        })();
        CloseHandle(snap);
        result
    }
}

#[cfg(not(windows))]
pub fn find_process_checked(name: &str) -> std::io::Result<Option<u32>> {
    let want = name.trim_end_matches(".exe");
    let entries = std::fs::read_dir("/proc")?;
    for entry in entries {
        let e = entry?;
        let file_name = e.file_name();
        let Some(pid) = file_name.to_str().and_then(|s| s.parse::<u32>().ok()) else { continue };
        let Ok(comm) = std::fs::read_to_string(e.path().join("comm")) else { continue };
        if comm.trim() == want || comm.trim() == name {
            return Ok(Some(pid));
        }
    }
    Ok(None)
}

/// File names of every running process.
#[cfg(windows)]
pub fn process_names() -> Vec<String> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
    };
    let mut out = Vec::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return out;
        }
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(snap, &mut e) != 0;
        while ok {
            let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
            out.push(String::from_utf16_lossy(&e.szExeFile[..len]));
            ok = Process32NextW(snap, &mut e) != 0;
        }
        CloseHandle(snap);
    }
    out
}

#[cfg(not(windows))]
pub fn process_names() -> Vec<String> {
    std::fs::read_dir("/proc")
        .map(|r| r.flatten().filter_map(|e| std::fs::read_to_string(e.path().join("comm")).ok()).map(|c| c.trim().to_string()).collect())
        .unwrap_or_default()
}

/// Blocks until the process ends. Returns its exit code when Windows gives it.
#[cfg(windows)]
pub fn wait_exit(pid: u32) -> Option<u32> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, WaitForSingleObject, INFINITE, PROCESS_QUERY_LIMITED_INFORMATION,
        PROCESS_SYNCHRONIZE,
    };
    unsafe {
        let h = OpenProcess(PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        WaitForSingleObject(h, INFINITE);
        let mut code = 0u32;
        let ok = GetExitCodeProcess(h, &mut code) != 0;
        CloseHandle(h);
        ok.then_some(code)
    }
}

#[cfg(not(windows))]
pub fn wait_exit(pid: u32) -> Option<u32> {
    while Path::new(&format!("/proc/{pid}")).exists() {
        std::thread::sleep(Duration::from_millis(500));
    }
    None
}

/// Skyrim Platform's browser helper; it can keep running after Skyrim's window
/// is gone.
pub const BROWSER_HELPERS: [&str; 2] = ["SkyrimPlatformCEF.exe", "SkyrimPlatformCEF.exe.hidden"];

/// Whether the process has a visible top-level window (a minimized window
/// counts as visible).
#[cfg(windows)]
pub fn has_visible_window(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowThreadProcessId, IsWindowVisible};
    struct Find {
        pid: u32,
        found: bool,
    }
    unsafe extern "system" fn each(hwnd: HWND, lp: LPARAM) -> BOOL {
        let f = &mut *(lp as *mut Find);
        let mut owner = 0u32;
        GetWindowThreadProcessId(hwnd, &mut owner);
        if owner == f.pid && IsWindowVisible(hwnd) != 0 {
            f.found = true;
            return 0;
        }
        1
    }
    let mut f = Find { pid, found: false };
    unsafe {
        EnumWindows(Some(each), &mut f as *mut Find as LPARAM);
    }
    f.found
}

#[cfg(not(windows))]
pub fn has_visible_window(_pid: u32) -> bool {
    true
}

/// Ends every running process with this file name; returns their ids.
pub fn end_all(name: &str) -> Vec<u32> {
    let mut out = Vec::new();
    // find_process returns the first match; enough for our helpers, which are
    // ended one at a time until none are left.
    for _ in 0..16 {
        match find_process(name) {
            Some(p) if !out.contains(&p) => {
                out.push(p);
                if !terminate(p) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            _ => break,
        }
    }
    out
}

/// Ends a process. Returns whether Windows accepted.
#[cfg(windows)]
pub fn terminate(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};
    unsafe {
        let h = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if h.is_null() {
            return false;
        }
        let ok = TerminateProcess(h, 0) != 0;
        CloseHandle(h);
        ok
    }
}

#[cfg(not(windows))]
pub fn terminate(_pid: u32) -> bool {
    false
}

/// Plain-words meaning of common Windows exit codes.
pub fn describe(code: Option<u32>) -> String {
    match code {
        None => "exit code unknown".into(),
        Some(0) => "closed normally (code 0)".into(),
        Some(0xC000_0005) => "crashed: access violation (0xC0000005)".into(),
        Some(0xC000_0409) => "crashed: stack buffer overrun (0xC0000409)".into(),
        Some(0xC000_00FD) => "crashed: stack overflow (0xC00000FD)".into(),
        Some(0xC000_0135) => "couldn't start: a DLL is missing (0xC0000135)".into(),
        Some(0xC000_007B) => "couldn't start: a DLL is the wrong type (0xC000007B)".into(),
        Some(0xE06D_7363) => "crashed: unhandled C++ exception (0xE06D7363)".into(),
        Some(c) => format!("ended with code 0x{c:08X}"),
    }
}

/// Whether the session looks like a crash rather than the player quitting.
pub fn crashed(code: Option<u32>, ran: Duration) -> bool {
    match code {
        Some(0) => false,
        // Windows exception codes (0xC..., 0xE06D7363) mean a real crash.
        Some(c) if c >= 0xC000_0000 => true,
        // Other codes, or none: Skyrim sometimes quits untidily, so only a
        // very short session counts as a crash.
        _ => ran < Duration::from_secs(90),
    }
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>, depth: u8) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() && depth > 0 {
            walk(&p, out, depth - 1);
        } else if p.is_file() {
            out.push(p);
        }
    }
}

fn tail(path: &Path, lines: usize) -> String {
    let bytes = std::fs::read(path).unwrap_or_default();
    let text = String::from_utf8_lossy(&bytes);
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

/// The logs written since `since`: everything under the SKSE log folder
/// (skse64.log, skyrim-platform.log, crash logger output) and SkyrimPlatform's
/// browser log in the temp folder. Crash logs are included in full-ish.
pub fn collect(skse_log_dir: &Path, temp_dir: &Path, since: SystemTime) -> String {
    let fresh = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).map(|t| t + Duration::from_secs(2) >= since).unwrap_or(false);
    let mut files = Vec::new();
    walk(skse_log_dir, &mut files, 2);
    let platform_tmp = temp_dir.join("Skyrim Platform");
    walk(&platform_tmp, &mut files, 2);
    files.retain(|p| {
        let n = p.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        (n.ends_with(".log") || n.ends_with(".txt")) && fresh(p)
    });
    files.sort();
    let mut o = String::new();
    if files.is_empty() {
        o.push_str(&format!("(no logs written this session under {} or {})\n", skse_log_dir.display(), platform_tmp.display()));
    }
    for f in files {
        let n = f.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        if n.starts_with("crash-") {
            o.push_str(&format!("\n===== {} (exception, call stack and plugin lists) =====\n{}\n", f.display(), crash_log_parts(&f)));
        } else {
            o.push_str(&format!("\n===== {} (last 120 lines) =====\n{}\n", f.display(), tail(&f, 120)));
        }
    }
    o
}

/// A crash logger's log without the long register and stack dumps: its first
/// lines (the exception and call stack) and the module and plugin lists at
/// the end. The last lines alone were all stack dump (2026-09-26).
fn crash_log_parts(path: &Path) -> String {
    let bytes = std::fs::read(path).unwrap_or_default();
    crash_log_text_parts(&String::from_utf8_lossy(&bytes))
}

fn crash_log_text_parts(text: &str) -> String {
    let all: Vec<&str> = text.lines().collect();
    let stack_end = all.iter().position(|l| {
        let u = l.trim().to_ascii_uppercase();
        u.starts_with("REGISTERS") || u.starts_with("STACK:")
    });
    let head_end = stack_end.unwrap_or(0).clamp(120, 250).min(all.len());
    let mut out: Vec<&str> = all[..head_end].to_vec();
    if let Some(p) = all.iter().position(|l| l.trim().to_ascii_uppercase().starts_with("SKSE PLUGINS")) {
        if p > head_end {
            out.push("...");
            out.extend(&all[p..]);
        }
    }
    out.join("\n")
}

/// The short version of a crash logger's log written since `since`: the
/// exception line and the top of the call stack. Works with CrashLogger SSE
/// ("Unhandled exception ...", "PROBABLE CALL STACK:") and Trainwreck
/// ("Exception ...", "CALL STACK").
pub fn crash_logger_summary(skse_log_dir: &Path, since: SystemTime) -> Option<String> {
    let newest = newest_crash_log(skse_log_dir, since)?;
    let text = String::from_utf8_lossy(&std::fs::read(&newest).ok()?).into_owned();
    let lines: Vec<&str> = text.lines().collect();
    let exception = lines.iter().find(|l| {
        let l = l.to_ascii_lowercase();
        l.contains("unhandled exception") || l.trim_start().starts_with("exception")
    })?;
    let mut out = vec![exception.trim().to_string()];
    if let Some(i) = lines.iter().position(|l| l.to_ascii_uppercase().contains("CALL STACK")) {
        out.extend(lines[i + 1..].iter().map(|l| l.trim()).filter(|l| !l.is_empty()).take(5).map(str::to_string));
    }
    Some(out.join("\n"))
}

/// Longest crash log sent to staff (the bot's crashLog field, CONTRACT.md).
pub const CRASH_LOG_MAX: usize = 30_000;

/// The newest crash logger log written since `since`, for staff: the whole
/// file when it fits, else its header and plugin lists; never over
/// `CRASH_LOG_MAX` characters.
pub fn crash_log_for_staff(skse_log_dir: &Path, since: SystemTime, private: &[&Path]) -> Option<String> {
    let path = newest_crash_log(skse_log_dir, since)?;
    let whole = redact_paths(&String::from_utf8_lossy(&std::fs::read(&path).ok()?), private);
    // Whole when it fits, else cut from the middle only, so the registers,
    // stack and objects near the top and the lists at the end both stay.
    Some(cut_chars(&whole, CRASH_LOG_MAX))
}

/// Hides the player's own folders in text for staff: each of `private` (the
/// home and Documents folders) becomes %USERPROFILE% or <private>, matched
/// without regard to case or slash kind and only at a path boundary, and the
/// folder name after any "\Users\" (8.3 short names too) becomes <user>.
pub fn redact_paths(text: &str, private: &[&Path]) -> String {
    let mut out = text.to_string();
    let norm = |s: &str| s.to_ascii_lowercase().replace('/', "\\");
    for (n, p) in private.iter().enumerate() {
        let want = norm(&p.display().to_string());
        let want = want.trim_end_matches('\\');
        if want.len() < 4 {
            continue;
        }
        let label = if n == 0 { "%USERPROFILE%" } else { "<private>" };
        let mut i = 0;
        loop {
            let hay = norm(&out);
            let Some(at) = hay[i..].find(want).map(|a| a + i) else { break };
            let end = at + want.len();
            let boundary = hay[end..].chars().next().is_none_or(|c| !c.is_alphanumeric() && c != '_' && c != '~');
            if boundary && out.is_char_boundary(at) && out.is_char_boundary(end) {
                out.replace_range(at..end, label);
                i = at + label.len();
            } else {
                i = end;
            }
        }
    }
    // Any other user folder, e.g. a short name or another drive.
    let mut res = String::with_capacity(out.len());
    let hay = norm(&out);
    let mut last = 0;
    let mut i = 0;
    while let Some(at) = hay[i..].find("\\users\\").map(|a| a + i) {
        let start = at + "\\users\\".len();
        // A name may hold spaces ("Tim Martin"): it runs to the next
        // backslash on the same line; with none, to the first space.
        let line_end = hay[start..].find(['"', '\'', '\n', '\r', ')']).map(|e| e + start).unwrap_or(hay.len());
        let end = match hay[start..line_end].find('\\') {
            Some(e) => e + start,
            None => hay[start..line_end].find(char::is_whitespace).map(|e| e + start).unwrap_or(line_end),
        };
        let name = &hay[start..end];
        if !name.is_empty() && !matches!(name, "public" | "default" | "all users" | "<user>") && out.is_char_boundary(start) && out.is_char_boundary(end) {
            res.push_str(&out[last..start]);
            res.push_str("<user>");
            last = end;
        }
        i = end.max(start);
    }
    res.push_str(&out[last..]);
    res
}

/// At most `max` characters, cut from the middle so both the top (the
/// exception, call stack and objects) and the end (module and plugin lists)
/// stay (quality check L3).
pub fn cut_chars(text: &str, max: usize) -> String {
    let n = text.chars().count();
    if n <= max {
        return text.to_string();
    }
    let mark = "\n…(middle cut to fit)…\n";
    let room = max.saturating_sub(mark.chars().count());
    let head = room * 2 / 5;
    let tail = room - head;
    let mut out: String = text.chars().take(head).collect();
    out.push_str(mark);
    out.extend(text.chars().skip(n - tail));
    out
}

fn newest_crash_log(skse_log_dir: &Path, since: SystemTime) -> Option<PathBuf> {
    let mut files = Vec::new();
    walk(skse_log_dir, &mut files, 2);
    files
        .into_iter()
        .filter(|p| {
            let n = p.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
            n.contains("crash") && n.ends_with(".log")
        })
        .filter_map(|p| Some((std::fs::metadata(&p).ok()?.modified().ok()?, p)))
        .filter(|(t, _)| *t + Duration::from_secs(2) >= since)
        .max_by_key(|(t, _)| *t)
        .map(|(_, p)| p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_crash_log_header() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("crash-1.log");
        let mut text = String::from("Unhandled exception at X.dll+1\nCALL STACK:\n[ 0] X.dll+1\nREGISTERS:\n");
        for i in 0..1000 {
            text.push_str(&format!("[RSP+{i}] 0x0\n"));
        }
        text.push_str("SKSE PLUGINS:\n\tX.dll\nPLUGINS:\n\t[ 0] Skyrim.esm\n");
        std::fs::write(&p, text).unwrap();
        let got = crash_log_parts(&p);
        assert!(got.starts_with("Unhandled exception"));
        assert!(got.contains("[ 0] X.dll+1") && got.contains("[ 0] Skyrim.esm"));
        assert!(got.lines().count() < 200);
    }

    #[test]
    fn sends_the_crash_log_whole_or_trimmed() {
        let t = tempfile::tempdir().unwrap();
        let since = SystemTime::now() - Duration::from_secs(60);
        assert_eq!(crash_log_for_staff(t.path(), since, &[]), None);
        std::fs::write(t.path().join("crash-1.log"), "Unhandled exception\nREGISTERS:\nRAX 0").unwrap();
        assert_eq!(crash_log_for_staff(t.path(), since, &[]).unwrap(), "Unhandled exception\nREGISTERS:\nRAX 0");
        let mut big = String::from("Unhandled exception at X.dll+1\nREGISTERS:\n");
        for i in 0..5000 {
            big.push_str(&format!("[RSP+{i}] 0x0000000000000000 (size_t) [0]\n"));
        }
        big.push_str("SKSE PLUGINS:\n\tX.dll\n");
        std::fs::write(t.path().join("crash-1.log"), big).unwrap();
        let got = crash_log_for_staff(t.path(), since, &[]).unwrap();
        assert!(got.chars().count() <= CRASH_LOG_MAX && got.contains("X.dll+1") && got.contains("SKSE PLUGINS"));
        assert_eq!(cut_chars("abcdef", 100), "abcdef");
        assert!(cut_chars(&"é".repeat(50), 40).chars().count() <= 40);
        let cut = cut_chars(&format!("TOP{}END", "x".repeat(1000)), 100);
        assert!(cut.starts_with("TOP") && cut.ends_with("END") && cut.chars().count() <= 100);
        std::fs::write(t.path().join("crash-1.log"), "at C:\\Users\\Tim\\Documents\\x").unwrap();
        let got = crash_log_for_staff(t.path(), since, &[Path::new("C:\\Users\\Tim")]).unwrap();
        assert!(got.contains("%USERPROFILE%") && !got.contains("Tim"));
        let home = Path::new("C:\\Users\\Tim");
        let docs = Path::new("D:\\OneDrive\\Documents");
        let r = redact_paths("c:/users/tim/x C:\\Users\\Timothy\\y C:\\Users\\TIMOTH~1\\z d:\\onedrive\\documents\\My Games C:\\Users\\Public\\a", &[home, docs]);
        assert_eq!(r, "%USERPROFILE%/x C:\\Users\\<user>\\y C:\\Users\\<user>\\z <private>\\My Games C:\\Users\\Public\\a");
        let r = redact_paths("at D:\\Users\\Tim Martin\\AppData\\x.dll and C:\\Users\\Ann is done", &[]);
        assert_eq!(r, "at D:\\Users\\<user>\\AppData\\x.dll and C:\\Users\\<user> is done");
    }

    #[test]
    fn describes_and_judges() {
        assert!(describe(Some(0xC0000005)).contains("access violation"));
        assert!(!crashed(Some(0), Duration::from_secs(5)));
        assert!(crashed(Some(0xC0000005), Duration::from_secs(3600)));
        assert!(crashed(None, Duration::from_secs(6)));
        assert!(!crashed(Some(1), Duration::from_secs(3600)));
        assert!(!crashed(None, Duration::from_secs(600)));
    }

    #[test]
    fn collects_fresh_logs_only() {
        let tmp = std::env::temp_dir().join(format!("ad-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let skse = tmp.join("SKSE");
        std::fs::create_dir_all(skse.join("Crashlogs")).unwrap();
        let old = skse.join("old.log");
        std::fs::write(&old, "old").unwrap();
        std::fs::File::options().write(true).open(&old).unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(SystemTime::now() - Duration::from_secs(3600))).unwrap();
        let since = SystemTime::now() - Duration::from_secs(60);
        std::fs::write(skse.join("skse64.log"), "a\nb\nlast line").unwrap();
        std::fs::write(skse.join("Crashlogs/crash-1.log"), "Unhandled exception").unwrap();
        let r = collect(&skse, &tmp.join("temp"), since);
        assert!(r.contains("last line") && r.contains("Unhandled exception"));
        assert!(!r.contains("old.log"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn summarises_crash_logger() {
        let tmp = std::env::temp_dir().join(format!("ad-cl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("SKSE")).unwrap();
        std::fs::write(
            tmp.join("SKSE/crash-2026-09-26-17-03-13.log"),
            "Skyrim SSE v1.6.1170\nCrashLoggerSSE v1-15\n\nUnhandled exception \"EXCEPTION_ACCESS_VIOLATION\" at 0x7FF6A1B2C3D4 SkyrimSE.exe+0123456\n\nPROBABLE CALL STACK:\n\t[0] 0x7FF6A1B2C3D4 SkyrimSE.exe+0123456\n\t[1] 0x7FF9 SkyrimPlatformImpl.dll+0000ABC\n",
        )
        .unwrap();
        let s = crash_logger_summary(&tmp.join("SKSE"), SystemTime::now() - Duration::from_secs(60)).unwrap();
        assert!(s.starts_with("Unhandled exception \"EXCEPTION_ACCESS_VIOLATION\""));
        assert!(s.contains("[1] 0x7FF9 SkyrimPlatformImpl.dll+0000ABC"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn finds_own_process() {
        #[cfg(target_os = "linux")]
        {
            let me = std::fs::read_to_string("/proc/self/comm").unwrap();
            assert!(find_process(me.trim()).is_some());
        }
    }

    #[test]
    #[cfg(any(windows, target_os = "linux"))]
    fn checked_lookup_distinguishes_present_and_absent() {
        #[cfg(windows)]
        let name = std::env::current_exe().unwrap().file_name().unwrap().to_string_lossy().into_owned();
        #[cfg(target_os = "linux")]
        let name = std::fs::read_to_string("/proc/self/comm").unwrap().trim().to_owned();

        assert!(find_process_checked(&name).unwrap().is_some());
        // Longer than any process image name in the Windows snapshot or Linux comm.
        assert_eq!(find_process_checked(&"a".repeat(300)).unwrap(), None);
    }

    /// A fake process list: `loader_until` and `game_from` are in ticks.
    fn fake(loader_until: u32, game_from: Option<u32>) -> (impl Fn(&str) -> Option<u32>, std::rc::Rc<std::cell::Cell<u32>>) {
        let t = std::rc::Rc::new(std::cell::Cell::new(0u32));
        let t2 = t.clone();
        let f = move |name: &str| {
            let now = t2.get();
            if name == GAME_PROCESS {
                // Each game lookup is one tick of time.
                t2.set(now + 1);
                return game_from.filter(|&g| now >= g).map(|_| 42);
            }
            (name == LOADER_PROCESS && now < loader_until).then_some(7)
        };
        (f, t)
    }

    #[tokio::test(start_paused = true)]
    async fn a_slow_start_is_waited_for_while_the_loader_runs() {
        let tick = Duration::from_secs(1);
        // Old rule: 90 s from Play. Here the loader holds for 200 s (Steam
        // updating) and the game appears at 250 s: found, not a crash.
        let (f, _) = fake(200, Some(250));
        assert_eq!(wait_for_game(f, tick, START_WAIT, START_WAIT_MAX).await, Some(42));
        // Loader gone at 5 s and no game: gives up 90 s later.
        let (f, t) = fake(5, None);
        assert_eq!(wait_for_game(f, tick, START_WAIT, START_WAIT_MAX).await, None);
        assert!((94..=97).contains(&t.get()), "{}", t.get());
        // A loader that never exits doesn't hold Play's watch forever.
        let (f, t) = fake(u32::MAX, None);
        assert_eq!(wait_for_game(f, tick, START_WAIT, START_WAIT_MAX).await, None);
        assert!((600..=602).contains(&t.get()), "{}", t.get());
        // A normal start is found at once.
        let (f, _) = fake(3, Some(2));
        assert_eq!(wait_for_game(f, tick, START_WAIT, START_WAIT_MAX).await, Some(42));
    }
}
