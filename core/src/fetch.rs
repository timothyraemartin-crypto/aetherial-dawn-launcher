//! Big downloads that survive a dropped connection or a closed launcher.
//!
//! A download goes to `<file>.part`, with `<file>.part.json` saying which
//! file it is (`key`) and how big it is. After a dropped connection it goes
//! on from where it stopped (an HTTP Range request); after the launcher is
//! closed, the next run does the same, even with a new download link, as
//! long as the key and the size match. A finished archive that wasn't
//! installed yet is used again without downloading it.

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::io::AsyncWriteExt;

/// Tries after a dropped connection, with waits of 2, 4, 8, 16, 30, 30 s.
pub const RETRIES: u32 = 6;

/// A connection that sends nothing for this long counts as dropped, so a
/// stalled download picks up again instead of waiting for ever.
pub const STALL: Duration = Duration::from_secs(30);

enum Waited<T> {
    Got(T),
    Cancelled,
    Stalled,
}

/// Waits for `fut`, checking `cancel` four times a second, for at most `stall`.
async fn watch<T>(fut: impl std::future::Future<Output = T>, cancel: &AtomicBool, stall: Duration) -> Waited<T> {
    tokio::pin!(fut);
    let until = tokio::time::Instant::now() + stall;
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Waited::Cancelled;
        }
        let left = until.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            return Waited::Stalled;
        }
        if let Ok(v) = tokio::time::timeout(left.min(Duration::from_millis(250)), &mut fut).await {
            return Waited::Got(v);
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
struct Note {
    key: String,
    total: u64,
    #[serde(default)]
    complete: bool,
}

fn note_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".part.json");
    PathBuf::from(s)
}

fn part_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

fn read_note(path: &Path) -> Option<Note> {
    serde_json::from_slice(&std::fs::read(note_path(path)).ok()?).ok()
}

fn write_note(path: &Path, note: &Note) {
    let _ = std::fs::write(note_path(path), serde_json::to_vec(note).unwrap_or_default());
}

/// Removes a download and what goes with it (after it is installed).
pub fn forget(path: &Path) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(part_path(path));
    let _ = std::fs::remove_file(note_path(path));
}

/// Bytes already on disk towards `key` at `path` (a finished archive counts
/// in full), for progress and time left before anything is fetched.
pub fn have(path: &Path, key: &str) -> u64 {
    match read_note(path) {
        Some(n) if n.key == key && n.complete => std::fs::metadata(path).map(|m| m.len()).unwrap_or(0),
        Some(n) if n.key == key => std::fs::metadata(part_path(path)).map(|m| m.len()).unwrap_or(0),
        _ => 0,
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Fetched {
    /// Bytes this call took over the network.
    pub fetched: u64,
    /// Bytes that were already there (a part or a finished archive).
    pub reused: u64,
    pub total: u64,
    /// Connections that dropped and were picked up again.
    pub resumed: u32,
}

/// Downloads `url` to `path`. `key` names the exact file (not the link, which
/// changes), so only the same file is ever continued. `progress(done, total)`
/// is called at most every 250 ms. Only https, except to 127.0.0.1 in tests.
pub async fn fetch(http: &reqwest::Client, url: &str, path: &Path, key: &str, cancel: &AtomicBool, progress: impl FnMut(u64, u64)) -> Result<Fetched, String> {
    fetch_within(http, url, path, key, cancel, STALL, progress).await
}

/// `fetch`, with the silence that counts as a dropped connection.
async fn fetch_within(
    http: &reqwest::Client,
    url: &str,
    path: &Path,
    key: &str,
    cancel: &AtomicBool,
    stall: Duration,
    mut progress: impl FnMut(u64, u64),
) -> Result<Fetched, String> {
    if !(url.starts_with("https://") || cfg!(test) && url.starts_with("http://127.0.0.1:")) {
        return Err("the download address isn't secure".into());
    }
    if let Some(p) = path.parent() {
        tokio::fs::create_dir_all(p).await.map_err(|e| e.to_string())?;
    }
    let part = part_path(path);
    let mut out = Fetched::default();
    // Finished last time but not installed: nothing to fetch.
    if let Some(n) = read_note(path) {
        let len = std::fs::metadata(path).map(|m| m.len()).ok();
        if n.key == key && n.complete && len == Some(n.total) {
            out.reused = n.total;
            out.total = n.total;
            progress(n.total, n.total);
            return Ok(out);
        }
        if n.key != key {
            // Another file under this name: start again.
            forget(path);
        }
    }
    let mut failures = 0u32;
    let mut first = true;
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err("cancelled".into());
        }
        let have = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
        let known = read_note(path).filter(|n| n.key == key).map(|n| n.total);
        let mut req = http.get(url);
        if have > 0 && known.is_some() {
            req = req.header(reqwest::header::RANGE, format!("bytes={have}-"));
        }
        let attempt = async {
            let resp = match watch(req.send(), cancel, stall).await {
                Waited::Got(r) => r.map_err(|e| (true, e.to_string()))?,
                Waited::Cancelled => return Err((false, "cancelled".to_string())),
                Waited::Stalled => return Err((true, "the server stopped answering".to_string())),
            };
            let status = resp.status();
            if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE && known == Some(have) {
                return Ok::<_, (bool, String)>(None);
            }
            let resp = resp.error_for_status().map_err(|e| (status.is_server_error() || status.as_u16() == 429, e.to_string()))?;
            Ok(Some(resp))
        };
        let resp = match attempt.await {
            Ok(r) => r,
            Err((retry, e)) => {
                if !retry || failures >= RETRIES {
                    return Err(e);
                }
                failures += 1;
                wait(failures, cancel).await?;
                continue;
            }
        };
        let (mut file, got, total) = match resp {
            // Everything was already there.
            None => (None, have, have),
            Some(resp) => {
                let partial = resp.status() == reqwest::StatusCode::PARTIAL_CONTENT;
                let range_total = resp
                    .headers()
                    .get(reqwest::header::CONTENT_RANGE)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.rsplit('/').next())
                    .and_then(|v| v.parse::<u64>().ok());
                let (start, total) = match (partial, range_total) {
                    (true, Some(t)) if Some(t) == known => (have, t),
                    // A server that ignores Range, or a changed file: start again.
                    _ => (0, resp.content_length().unwrap_or(0)),
                };
                if start > 0 && !first {
                    out.resumed += 1;
                } else if start > 0 {
                    out.reused = start;
                }
                write_note(path, &Note { key: key.into(), total, complete: false });
                let f = if start > 0 {
                    tokio::fs::OpenOptions::new().append(true).open(&part).await
                } else {
                    tokio::fs::File::create(&part).await
                }
                .map_err(|e| e.to_string())?;
                let mut got = start;
                let mut f = Some(f);
                let mut stream = resp.bytes_stream();
                let mut last = std::time::Instant::now();
                let mut dropped = None;
                loop {
                    let next = watch(stream.next(), cancel, stall).await;
                    if matches!(next, Waited::Cancelled) || cancel.load(Ordering::SeqCst) {
                        // The part stays for next time.
                        if let Some(mut f) = f.take() {
                            let _ = f.flush().await;
                        }
                        return Err("cancelled".into());
                    }
                    let chunk = match next {
                        Waited::Got(Some(c)) => c,
                        Waited::Got(None) => break,
                        _ => {
                            dropped = Some(format!("no data for {} s", stall.as_secs()));
                            break;
                        }
                    };
                    match chunk {
                        Ok(c) => {
                            f.as_mut().unwrap().write_all(&c).await.map_err(|e| e.to_string())?;
                            got += c.len() as u64;
                            out.fetched += c.len() as u64;
                            if last.elapsed() >= Duration::from_millis(250) {
                                progress(got, total);
                                last = std::time::Instant::now();
                            }
                        }
                        Err(e) => {
                            dropped = Some(e.to_string());
                            break;
                        }
                    }
                }
                if let Some(f) = f.as_mut() {
                    f.flush().await.map_err(|e| e.to_string())?;
                }
                if dropped.is_some() || (total > 0 && got < total) {
                    first = false;
                    if failures >= RETRIES {
                        return Err(dropped.unwrap_or_else(|| "the download stopped early".into()));
                    }
                    failures += 1;
                    progress(got, total);
                    wait(failures, cancel).await?;
                    continue;
                }
                (f, got, total.max(got))
            }
        };
        if let Some(f) = file.take() {
            drop(f);
        }
        tokio::fs::rename(&part, path).await.map_err(|e| e.to_string())?;
        write_note(path, &Note { key: key.into(), total, complete: true });
        progress(got, total);
        out.total = total;
        return Ok(out);
    }
}

async fn wait(failures: u32, cancel: &AtomicBool) -> Result<(), String> {
    let secs = if failures >= 5 { 30 } else { (1u64 << failures.min(4)).clamp(2, 30) };
    for _ in 0..secs * 4 {
        if cancel.load(Ordering::SeqCst) {
            return Err("cancelled".into());
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Ok(())
}

/// The time left for what's still to download, from how fast archives have
/// actually been getting done (downloaded and installed) over the last few
/// minutes of wall-clock time, so installs and clicks count too.
#[derive(Debug, Default)]
pub struct Pace {
    /// (seconds since start, bytes done) samples.
    points: Vec<(f64, u64)>,
}

impl Pace {
    /// How much history the rate is taken from.
    pub const WINDOW: f64 = 180.0;
    /// No estimate until this much has been measured.
    pub const WARMUP: f64 = 10.0;

    pub fn add(&mut self, at: f64, done: u64) {
        self.points.push((at, done));
        let cut = at - Self::WINDOW;
        // Keep one point older than the window, so the window is full.
        while self.points.len() > 2 && self.points[1].0 <= cut {
            self.points.remove(0);
        }
    }

    /// Bytes per second over the window.
    pub fn rate(&self) -> Option<f64> {
        let (a, b) = (self.points.first()?, self.points.last()?);
        let dt = b.0 - a.0;
        (dt >= Self::WARMUP && b.1 > a.1).then(|| (b.1 - a.1) as f64 / dt)
    }

    /// Seconds left for `left` bytes.
    pub fn eta(&self, left: u64) -> Option<u64> {
        self.rate().map(|r| (left as f64 / r).round() as u64)
    }
}

/// Free space the run needs on the Skyrim drive, mod by mod in install
/// order: what is installed so far, plus the mod in flight (what's left of
/// its archive, and its unpacked files, twice over when files are copied
/// rather than moved into Data). The archive goes once the mod is in.
/// `mods` is (archive bytes, unpacked bytes, archive bytes already here),
/// in install order; bytes already here are on the disk now, so not needed.
pub fn space_needed(mods: &[(u64, u64, u64)], moved: bool) -> u64 {
    let mut installed = 0u64;
    let mut peak = 0u64;
    for &(archive, unpacked, here) in mods {
        let flight = archive.saturating_sub(here) + unpacked + if moved { 0 } else { unpacked };
        peak = peak.max(installed + flight);
        installed += unpacked;
    }
    peak.max(installed)
}

/// Free bytes on the drive holding `dir`, where Windows can say.
pub fn free_space(dir: &Path) -> Option<u64> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let wide: Vec<u16> = dir.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        let mut free = 0u64;
        let ok = unsafe { windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, std::ptr::null_mut(), std::ptr::null_mut()) };
        (ok != 0).then_some(free)
    }
    #[cfg(not(windows))]
    {
        let _ = dir;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Arc;

    /// A tiny HTTP server for one file: honours Range, sends `rate` bytes a
    /// second, and drops the first `drops` connections after `drop_at` bytes.
    struct Server {
        url: String,
        requests: Arc<AtomicUsize>,
        ranged: Arc<AtomicUsize>,
    }

    fn serve(body: Arc<Vec<u8>>, rate: usize, drop_at: usize, drops: usize, honour_range: bool) -> Server {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://127.0.0.1:{}/file.7z", l.local_addr().unwrap().port());
        let requests = Arc::new(AtomicUsize::new(0));
        let ranged = Arc::new(AtomicUsize::new(0));
        let (rq, rg) = (requests.clone(), ranged.clone());
        std::thread::spawn(move || {
            for s in l.incoming() {
                let Ok(mut s) = s else { continue };
                let n = rq.fetch_add(1, Ordering::SeqCst);
                let mut buf = [0u8; 4096];
                let len = s.read(&mut buf).unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..len]).to_ascii_lowercase();
                let start = head
                    .lines()
                    .find_map(|l| l.strip_prefix("range: bytes="))
                    .and_then(|r| r.trim_end_matches('-').trim().trim_end_matches('-').parse::<usize>().ok())
                    .filter(|_| honour_range);
                let start = start.unwrap_or(0);
                if start > 0 {
                    rg.fetch_add(1, Ordering::SeqCst);
                }
                let total = body.len();
                let hdr = if start > 0 {
                    format!("HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {start}-{}/{total}\r\nConnection: close\r\n\r\n", total - start, total - 1)
                } else {
                    format!("HTTP/1.1 200 OK\r\nContent-Length: {total}\r\nConnection: close\r\n\r\n")
                };
                let _ = s.write_all(hdr.as_bytes());
                let chunk = (rate / 20).max(1);
                let mut at = start;
                while at < total {
                    if n < drops && at >= drop_at {
                        break;
                    }
                    let end = (at + chunk).min(total);
                    if s.write_all(&body[at..end]).is_err() {
                        break;
                    }
                    at = end;
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        });
        Server { url, requests, ranged }
    }

    fn body(n: usize) -> Arc<Vec<u8>> {
        Arc::new((0..n).map(|i| (i * 31 % 251) as u8).collect())
    }

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
    }

    #[test]
    fn a_dropped_connection_carries_on_from_where_it_stopped() {
        let data = body(400_000);
        let srv = serve(data.clone(), 4_000_000, 150_000, 1, true);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.7z");
        let http = reqwest::Client::new();
        let got = rt().block_on(fetch(&http, &srv.url, &path, "nexus-1-2", &AtomicBool::new(false), |_, _| {})).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), *data);
        assert_eq!(got.resumed, 1);
        assert_eq!(srv.ranged.load(Ordering::SeqCst), 1);
        // Nothing fetched twice (the drop comes at a chunk boundary at or past 150 000).
        assert_eq!(got.fetched, 400_000);
    }

    #[test]
    fn a_closed_launcher_carries_on_next_time_even_with_a_new_link() {
        let data = body(300_000);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.7z");
        let http = reqwest::Client::new();
        let stop = AtomicBool::new(false);
        let srv = serve(data.clone(), 400_000, usize::MAX, 0, true);
        // Stopped part-way (the player pressed Stop or closed the launcher).
        let r = rt().block_on(fetch(&http, &srv.url, &path, "nexus-1-2", &stop, |done, _| {
            if done >= 100_000 {
                stop.store(true, Ordering::SeqCst);
            }
        }));
        assert_eq!(r, Err("cancelled".into()));
        let kept = have(&path, "nexus-1-2");
        assert!((100_000..300_000).contains(&kept), "{kept}");
        // Next run: another server address stands in for a new Nexus link.
        let srv2 = serve(data.clone(), 2_000_000, usize::MAX, 0, true);
        let got = rt().block_on(fetch(&http, &srv2.url, &path, "nexus-1-2", &AtomicBool::new(false), |_, _| {})).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), *data);
        assert_eq!(got.reused, kept);
        assert_eq!(got.fetched, 300_000 - kept);
    }

    #[test]
    fn another_file_under_the_same_name_starts_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.7z");
        std::fs::write(part_path(&path), vec![7u8; 50_000]).unwrap();
        write_note(&path, &Note { key: "nexus-1-OLD".into(), total: 300_000, complete: false });
        let data = body(120_000);
        let srv = serve(data.clone(), 4_000_000, usize::MAX, 0, true);
        let got = rt().block_on(fetch(&reqwest::Client::new(), &srv.url, &path, "nexus-1-NEW", &AtomicBool::new(false), |_, _| {})).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), *data);
        assert_eq!((got.reused, got.fetched), (0, 120_000));
    }

    #[test]
    fn a_server_that_ignores_range_starts_again_cleanly() {
        let data = body(200_000);
        let srv = serve(data.clone(), 400_000, 80_000, 1, false);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.7z");
        rt().block_on(fetch(&reqwest::Client::new(), &srv.url, &path, "k", &AtomicBool::new(false), |_, _| {})).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), *data);
        assert_eq!(srv.requests.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_finished_archive_is_used_again_without_downloading() {
        let data = body(90_000);
        let srv = serve(data.clone(), 4_000_000, usize::MAX, 0, true);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.7z");
        let http = reqwest::Client::new();
        rt().block_on(fetch(&http, &srv.url, &path, "k", &AtomicBool::new(false), |_, _| {})).unwrap();
        let again = rt().block_on(fetch(&http, &srv.url, &path, "k", &AtomicBool::new(false), |_, _| {})).unwrap();
        assert_eq!((again.fetched, again.reused), (0, 90_000));
        assert_eq!(srv.requests.load(Ordering::SeqCst), 1);
        forget(&path);
        assert!(!path.exists() && !note_path(&path).exists());
    }

    /// The 0.1.96 download, for the before numbers: no Range, a new file
    /// each try, and the part deleted on any stop.
    async fn old_fetch(http: &reqwest::Client, url: &str, path: &Path) -> Result<u64, String> {
        let resp = http.get(url).send().await.map_err(|e| e.to_string())?;
        let mut f = tokio::fs::File::create(path).await.map_err(|e| e.to_string())?;
        let mut stream = resp.bytes_stream();
        let mut got = 0u64;
        while let Some(c) = stream.next().await {
            let c = c.map_err(|e| e.to_string())?;
            f.write_all(&c).await.map_err(|e| e.to_string())?;
            got += c.len() as u64;
        }
        Ok(got)
    }

    /// Throttled-link measurement (run by hand, prints numbers):
    ///   cargo test -p launcher-core measure_throttled -- --ignored --nocapture
    #[test]
    #[ignore]
    fn measure_throttled() {
        const RATE: usize = 4_000_000; // bytes a second, a 32 Mbit/s line
        let sizes = [30_000_000usize, 5_000_000, 12_000_000, 2_000_000, 40_000_000, 8_000_000, 1_000_000, 22_000_000];
        let total: usize = sizes.iter().sum();
        let rt = rt();
        let http = reqwest::Client::new();
        let dir = tempfile::tempdir().unwrap();

        // 1. A connection that drops at 60 % of the biggest file.
        let big = body(40_000_000);
        let before = {
            let srv = serve(big.clone(), RATE, 24_000_000, 1, false);
            let p = dir.path().join("old.7z");
            let t = std::time::Instant::now();
            let first = rt.block_on(old_fetch(&http, &srv.url, &p));
            // The 0.1.96 run: one quiet retry, from the start.
            let second = rt.block_on(old_fetch(&http, &srv.url, &p)).unwrap();
            let fetched = std::fs::metadata(&p).unwrap().len() + 24_000_000;
            assert!(first.is_err() || first.unwrap() < 40_000_000);
            (second, fetched, t.elapsed().as_secs_f64())
        };
        let after = {
            let srv = serve(big.clone(), RATE, 24_000_000, 1, true);
            let p = dir.path().join("new.7z");
            let t = std::time::Instant::now();
            let got = rt.block_on(fetch(&http, &srv.url, &p, "k", &AtomicBool::new(false), |_, _| {})).unwrap();
            assert_eq!(std::fs::read(&p).unwrap(), *big);
            (got.fetched, got.resumed, t.elapsed().as_secs_f64())
        };
        println!("DROP at 60% of a 40 MB file at 4 MB/s:");
        println!("  before: {:.1} MB fetched, {:.1} s (started again from 0)", before.1 as f64 / 1e6, before.2);
        println!("  after:  {:.1} MB fetched, {:.1} s ({} drop picked up; the wait before it is 2 s)", after.0 as f64 / 1e6, after.2, after.1);

        // 2. The launcher closed at 50 % of the same file, then opened again.
        let closed = {
            let srv = serve(big.clone(), RATE, usize::MAX, 0, true);
            let p = dir.path().join("closed.7z");
            let stop = AtomicBool::new(false);
            let _ = rt.block_on(fetch(&http, &srv.url, &p, "k2", &stop, |d, _| {
                if d >= 20_000_000 {
                    stop.store(true, Ordering::SeqCst);
                }
            }));
            let kept = have(&p, "k2");
            let srv2 = serve(big.clone(), RATE, usize::MAX, 0, true);
            let got = rt.block_on(fetch(&http, &srv2.url, &p, "k2", &AtomicBool::new(false), |_, _| {})).unwrap();
            (kept, got.fetched)
        };
        println!("CLOSED at 50%: before, 40.0 MB fetched again (the part was deleted); after, {:.1} MB kept and {:.1} MB fetched", closed.0 as f64 / 1e6, closed.1 as f64 / 1e6);

        // 3. Time left over a whole run of 8 mods (120 MB) with a 1 s install
        // for every 10 MB, sampled at each quarter against the real time left.
        let srvs: Vec<_> = sizes.iter().map(|n| (serve(body(*n), RATE, usize::MAX, 0, true), *n)).collect();
        let start = std::time::Instant::now();
        let pace = std::sync::Mutex::new(Pace::default());
        let samples = std::sync::Mutex::new(Vec::<(f64, u64, Option<u64>)>::new());
        let mut base = 0u64;
        for (i, (srv, n)) in srvs.iter().enumerate() {
            let p = dir.path().join(format!("m{i}.7z"));
            rt.block_on(fetch(&http, &srv.url, &p, &format!("m{i}"), &AtomicBool::new(false), |d, _| {
                let done = base + d;
                let at = start.elapsed().as_secs_f64();
                let mut pc = pace.lock().unwrap();
                pc.add(at, done);
                samples.lock().unwrap().push((at, done, pc.eta(total as u64 - done)));
            }))
            .unwrap();
            base += *n as u64;
            // Unpacking and moving into Data.
            std::thread::sleep(Duration::from_millis((*n / 10_000) as u64));
            pace.lock().unwrap().add(start.elapsed().as_secs_f64(), base);
        }
        let end = start.elapsed().as_secs_f64();
        println!("TIME LEFT over 8 mods, 120 MB at 4 MB/s plus 1 s of install per 10 MB (real run {:.1} s):", end);
        for q in [0.1, 0.25, 0.5, 0.75, 0.9] {
            let s = samples.lock().unwrap();
            if let Some((at, _, eta)) = s.iter().find(|x| x.1 as f64 >= q * total as f64) {
                let real = end - at;
                match eta {
                    Some(e) => println!("  at {:>3.0}%: said {:>3} s, real {:>5.1} s ({:+.0}%)", q * 100.0, e, real, (*e as f64 - real) / real * 100.0),
                    None => println!("  at {:>3.0}%: working it out, real {:.1} s", q * 100.0, real),
                }
            }
        }
    }

    #[test]
    fn time_left_follows_the_real_pace() {
        let mut p = Pace::default();
        assert_eq!(p.eta(1000), None);
        for s in 0..=20 {
            p.add(s as f64, s as u64 * 1_000_000);
        }
        assert_eq!(p.eta(60_000_000), Some(60));
        // It slows to half: within the window the estimate moves towards it.
        for s in 21..=400 {
            p.add(s as f64, 20_000_000 + (s as u64 - 20) * 500_000);
        }
        assert_eq!(p.eta(60_000_000), Some(120));
    }

    #[test]
    fn space_needed_follows_the_install_order() {
        // (archive, unpacked, archive already here)
        let mods = [(7_000, 10_400, 0), (2_700, 4_600, 0), (100, 300, 0)];
        // Copied: the first mod's archive, unpacked files and copy at once.
        assert_eq!(space_needed(&mods, false), 27_800);
        // Moved: the second mod on top of the first one's files is the peak.
        assert_eq!(space_needed(&mods, true), 17_700);
        // An archive already downloaded is on the disk already.
        let mods = [(7_000, 10_400, 7_000), (2_700, 4_600, 0)];
        assert_eq!(space_needed(&mods, true), 17_700);
        assert_eq!(space_needed(&[], true), 0);
    }

    /// Sends the headers and `first` bytes, then goes silent on the first
    /// connection (holding it open); later connections get the rest.
    fn serve_stalling(body: Arc<Vec<u8>>, first: usize) -> String {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/file", l.local_addr().unwrap());
        std::thread::spawn(move || {
            let mut held = Vec::new();
            for (n, s) in l.incoming().enumerate() {
                let Ok(mut s) = s else { continue };
                let mut buf = [0u8; 4096];
                let len = s.read(&mut buf).unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..len]).to_ascii_lowercase();
                let start = head.lines().find_map(|l| l.strip_prefix("range: bytes=")).and_then(|r| r.trim_end_matches('-').trim().trim_end_matches('-').parse::<usize>().ok()).unwrap_or(0);
                let total = body.len();
                let hdr = if start > 0 {
                    format!("HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {start}-{}/{total}\r\nConnection: close\r\n\r\n", total - start, total - 1)
                } else {
                    format!("HTTP/1.1 200 OK\r\nContent-Length: {total}\r\nConnection: close\r\n\r\n")
                };
                let _ = s.write_all(hdr.as_bytes());
                if n == 0 {
                    let _ = s.write_all(&body[..first]);
                    let _ = s.flush();
                    held.push(s);
                } else {
                    let _ = s.write_all(&body[start..]);
                }
            }
        });
        url
    }

    #[test]
    fn a_stalled_connection_counts_as_dropped_and_carries_on() {
        let data = body(200_000);
        let url = serve_stalling(data.clone(), 50_000);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.7z");
        let t = std::time::Instant::now();
        let got = rt().block_on(fetch_within(&reqwest::Client::new(), &url, &path, "k", &AtomicBool::new(false), Duration::from_secs(1), |_, _| {})).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), *data);
        assert_eq!(got.resumed, 1);
        // One second of silence, the 2 s wait, then the rest.
        assert!(t.elapsed() < Duration::from_secs(10), "{:?}", t.elapsed());
    }

    #[test]
    fn stop_works_while_the_connection_is_silent() {
        let url = serve_stalling(body(200_000), 50_000);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.7z");
        let stop = Arc::new(AtomicBool::new(false));
        let s2 = stop.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(500));
            s2.store(true, Ordering::SeqCst);
        });
        let t = std::time::Instant::now();
        let r = rt().block_on(fetch_within(&reqwest::Client::new(), &url, &path, "k", &stop, Duration::from_secs(60), |_, _| {}));
        assert_eq!(r, Err("cancelled".into()));
        assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
        // What came before the silence stays for next time.
        assert_eq!(have(&path, "k"), 50_000);
    }
}
