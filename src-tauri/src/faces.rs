//! Face sharing while the game runs (core/src/faces.rs has the rules and
//! checks). Started when SkyrimSE.exe appears, stopped when it closes; every
//! failure is logged and the game carries on.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use launcher_core::faces;

use crate::log;

const TICK: Duration = Duration::from_secs(1);
/// The list's pace: the server's `listEvery`, 5 s when it says nothing
/// (core/src/faces.rs), never slower than this after failures.
const LIST_MAX: Duration = faces::LIST_EVERY_MAX;
/// Faces fetched per list, so one list can't hold the loop for long.
const PER_LIST: usize = 20;

/// Runs until `stop` is set; returns the last list seen (for tidying).
pub async fn run(http: reqwest::Client, base: String, token: String, game_dir: PathBuf, stop: Arc<AtomicBool>) -> Option<BTreeSet<String>> {
    let dir = match faces::folder(&game_dir) {
        Ok(d) => d,
        Err(e) => {
            log::line(&format!("faces: off for this session: {e}"));
            return None;
        }
    };
    let api = format!("{}/faces", base.trim_end_matches('/'));
    let own = dir.join(faces::SELF_FILE);
    let mut seen = modified(&own);
    let mut changed_at: Option<Instant> = None;
    // (the file's modified time, when) for the one retry a 409, 429 or 503
    // gets.
    let mut retry: Option<(Option<SystemTime>, Instant)> = None;
    // The server's pace, from its last list answer.
    let mut pace = faces::LIST_EVERY;
    let mut every = pace;
    let mut next_list = Instant::now();
    let mut last: Option<BTreeSet<String>> = None;
    log::line("faces: sharing faces while the game runs");
    while !stop.load(Ordering::SeqCst) {
        // Own face: sent once, 2 seconds after the game last wrote it.
        let now_m = modified(&own);
        if now_m != seen {
            seen = now_m;
            changed_at = Some(Instant::now());
            retry = None;
        }
        if changed_at.is_some_and(|t| t.elapsed() >= Duration::from_secs(2)) {
            changed_at = None;
            match upload(&http, &api, &token, &own).await {
                Upload::Done => {}
                Upload::RetryAfter(d) => retry = Some((seen, Instant::now() + d)),
                Upload::SignedOut => {
                    log::line("faces: the login service says signed out; stopping for this session");
                    break;
                }
            }
        } else if let Some((m, at)) = retry {
            if Instant::now() >= at {
                retry = None;
                if m == seen {
                    // The second and last try for this file.
                    if let Upload::SignedOut = upload(&http, &api, &token, &own).await {
                        log::line("faces: the login service says signed out; stopping for this session");
                        break;
                    }
                }
            }
        }
        // Everyone else's faces.
        if Instant::now() >= next_list {
            match list(&http, &api, &token).await {
                Ok(answer) => {
                    if answer.every != pace {
                        log::line(&format!("faces: the server asks for the list every {:.1} s", answer.every.as_secs_f64()));
                        pace = answer.every;
                    }
                    every = pace;
                    let names = answer.names;
                    let have = faces::saved(&dir);
                    for name in names.iter().filter(|n| !have.contains(*n)).take(PER_LIST) {
                        if stop.load(Ordering::SeqCst) {
                            break;
                        }
                        // Busy: the rest wait for the next list.
                        if !fetch(&http, &api, &token, &dir, name).await {
                            every = (every * 2).min(LIST_MAX);
                            break;
                        }
                    }
                    let set: BTreeSet<String> = names.into_iter().collect();
                    // Written only when the list changes.
                    if last.as_ref() != Some(&set) {
                        faces::remember(&game_dir, &set);
                    }
                    last = Some(set);
                }
                // The server's Retry-After when it sends one, else twice as slow.
                Err(List::Slower(Some(wait))) => every = wait.max(pace),
                Err(List::Slower(None)) => every = (every * 2).min(LIST_MAX),
                Err(List::SignedOut) => {
                    log::line("faces: the login service says signed out; stopping for this session");
                    break;
                }
                Err(List::Failed(e)) => {
                    every = (every * 2).min(LIST_MAX);
                    log::line(&format!("faces: couldn't get the list: {e}"));
                }
            }
            next_list = Instant::now() + every;
        }
        tokio::time::sleep(TICK).await;
    }
    last
}

/// Deletes faces not on the last list (at game close) or the remembered one
/// (at launcher start).
pub fn tidy(game_dir: &Path, keep: Option<BTreeSet<String>>) {
    let keep = keep.unwrap_or_else(|| faces::remembered(game_dir));
    // A link in the ad folder's place is left alone (never followed).
    if let Some(dir) = faces::existing_folder(game_dir) {
        let n = faces::tidy(&dir, &keep);
        if n > 0 {
            log::line(&format!("faces: removed {n} face(s) of characters no longer online"));
        }
    }
}

fn modified(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

enum Upload {
    Done,
    RetryAfter(Duration),
    SignedOut,
}

async fn upload(http: &reqwest::Client, api: &str, token: &str, own: &Path) -> Upload {
    let size = std::fs::metadata(own).map(|m| m.len()).unwrap_or(0);
    if size == 0 {
        return Upload::Done;
    }
    if size > faces::MAX_UPLOAD {
        log::line(&format!("faces: your face file is {size} bytes, over the limit; not sent"));
        return Upload::Done;
    }
    let Ok(body) = std::fs::read(own) else { return Upload::Done };
    let res = http.put(format!("{api}/self")).header("authorization", token).body(body).timeout(Duration::from_secs(20)).send().await;
    let r = match res {
        Ok(r) => r,
        Err(e) => {
            log::line(&format!("faces: couldn't send your face: {}", launcher_core::scrub(&e.to_string())));
            return Upload::Done;
        }
    };
    let status = r.status().as_u16();
    let wait = retry_after(&r);
    let text = capped_text(r, 4096).await;
    match status {
        200 => {
            log::line(&format!("faces: sent your face ({})", if text.contains("\"unchanged\":true") { "unchanged" } else { "saved" }));
            Upload::Done
        }
        409 => {
            log::line(&format!("faces: the server wasn't ready for your face ({}); trying once more in 15 s", short(&text)));
            Upload::RetryAfter(Duration::from_secs(15))
        }
        // Too many sends, or the face service busy: wait, then once more.
        429 => Upload::RetryAfter(wait.unwrap_or(Duration::from_secs(10))),
        503 => {
            log::line("faces: the face service was busy; trying once more shortly");
            Upload::RetryAfter(wait.unwrap_or(Duration::from_secs(5)))
        }
        401 => Upload::SignedOut,
        400 | 413 => {
            log::line(&format!("faces: the server refused your face: {}", short(&text)));
            Upload::Done
        }
        s => {
            log::line(&format!("faces: sending your face got {s}"));
            Upload::Done
        }
    }
}

enum List {
    /// Busy or too soon, with the server's Retry-After if it sent one.
    Slower(Option<Duration>),
    SignedOut,
    Failed(String),
}

async fn list(http: &reqwest::Client, api: &str, token: &str) -> Result<faces::FaceList, List> {
    let r = http.get(format!("{api}/list")).header("authorization", token).timeout(Duration::from_secs(10)).send().await.map_err(|e| List::Failed(launcher_core::scrub(&e.to_string())))?;
    match r.status().as_u16() {
        200 => {
            let body = capped(r, faces::MAX_LIST).await.ok_or_else(|| List::Failed("the list was too long".into()))?;
            faces::list_answer(&body).map_err(|e| List::Failed(e.to_string()))
        }
        429 | 503 => Err(List::Slower(retry_after(&r))),
        401 => Err(List::SignedOut),
        s => Err(List::Failed(format!("answer {s}"))),
    }
}

/// Fetches and saves one face; false when the service is busy and the rest
/// should wait.
async fn fetch(http: &reqwest::Client, api: &str, token: &str, dir: &Path, name: &str) -> bool {
    let r = match http.get(format!("{api}/f/{name}.jslot")).header("authorization", token).timeout(Duration::from_secs(15)).send().await {
        Ok(r) => r,
        Err(e) => {
            log::line(&format!("faces: couldn't fetch {name}: {}", launcher_core::scrub(&e.to_string())));
            return true;
        }
    };
    match r.status().as_u16() {
        200 => match capped(r, faces::MAX_FACE).await {
            Some(body) => {
                if let Err(e) = faces::save(dir, name, &body) {
                    log::line(&format!("faces: didn't save {name}: {e}"));
                }
            }
            None => log::line(&format!("faces: didn't save {name}: over the size limit")),
        },
        // Replaced since the list: the next list names the new one.
        404 => {}
        429 | 503 => return false,
        s => log::line(&format!("faces: fetching {name} got {s}")),
    }
    true
}

/// The server's Retry-After in seconds, kept between 1 and 60.
fn retry_after(r: &reqwest::Response) -> Option<Duration> {
    let s: u64 = r.headers().get(reqwest::header::RETRY_AFTER)?.to_str().ok()?.trim().parse().ok()?;
    Some(Duration::from_secs(s.clamp(1, 60)))
}

/// The body, or None when it's longer than `cap`.
async fn capped(r: reqwest::Response, cap: usize) -> Option<Vec<u8>> {
    use futures_util::StreamExt;
    if r.content_length().is_some_and(|l| l as usize > cap) {
        return None;
    }
    let mut out = Vec::new();
    let mut s = r.bytes_stream();
    while let Some(c) = s.next().await {
        out.extend_from_slice(&c.ok()?);
        if out.len() > cap {
            return None;
        }
    }
    Some(out)
}

async fn capped_text(r: reqwest::Response, cap: usize) -> String {
    capped(r, cap).await.map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default()
}

/// The server's reason, from {"error":..,"message":..}, kept short.
fn short(text: &str) -> String {
    let v: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
    let s = v["message"].as_str().or(v["error"].as_str()).unwrap_or(text);
    s.chars().take(200).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// What the stand-in server saw: "METHOD /path", the authorization header, the body.
    type Seen = Arc<Mutex<Vec<(String, String, Vec<u8>)>>>;
    type Reply = (u16, Vec<(&'static str, String)>, Vec<u8>);

    /// A local HTTP server answering from `reply("METHOD /path")`. Returns its
    /// base URL and what it saw.
    async fn server(reply: impl Fn(&str) -> Reply + Send + Sync + 'static) -> (String, Seen) {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        let seen: Seen = Arc::default();
        let (seen2, reply) = (seen.clone(), Arc::new(reply));
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = l.accept().await else { return };
                let (seen, reply) = (seen2.clone(), reply.clone());
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut chunk = [0u8; 8192];
                    let head_end = loop {
                        let n = sock.read(&mut chunk).await.unwrap_or(0);
                        if n == 0 {
                            return;
                        }
                        buf.extend_from_slice(&chunk[..n]);
                        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            break i + 4;
                        }
                    };
                    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
                    let mut lines = head.lines();
                    let mut first = lines.next().unwrap_or("").split(' ');
                    let key = format!("{} {}", first.next().unwrap_or(""), first.next().unwrap_or(""));
                    let header = |name: &str| lines.clone().find_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.trim().to_string())).unwrap_or_default();
                    let len: usize = header("content-length").parse().unwrap_or(0);
                    let auth = header("authorization");
                    while buf.len() < head_end + len {
                        let n = sock.read(&mut chunk).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        buf.extend_from_slice(&chunk[..n]);
                    }
                    seen.lock().unwrap().push((key.clone(), auth, buf[head_end..].to_vec()));
                    let (status, headers, body) = reply(&key);
                    let mut out = format!("HTTP/1.1 {status} X\r\ncontent-length: {}\r\nconnection: close\r\n", body.len());
                    for (k, v) in headers {
                        out += &format!("{k}: {v}\r\n");
                    }
                    out += "\r\n";
                    let _ = sock.write_all(out.as_bytes()).await;
                    let _ = sock.write_all(&body).await;
                    let _ = sock.shutdown().await;
                });
            }
        });
        (base, seen)
    }

    fn client() -> reqwest::Client {
        reqwest::Client::builder().no_proxy().build().unwrap()
    }

    fn named(body: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        format!("aff000003-{}", &hex::encode(Sha256::digest(body))[..16])
    }

    fn own_file(len: usize) -> (tempfile::TempDir, PathBuf) {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join(faces::SELF_FILE);
        std::fs::write(&p, vec![b'x'; len]).unwrap();
        (t, p)
    }

    #[tokio::test]
    async fn upload_sends_the_face_and_reads_the_answer() {
        let (_t, own) = own_file(10);
        let retry = |s: &str| vec![("retry-after", s.to_string())];
        for (status, headers, want) in [
            (200, vec![], "done"),
            (400, vec![], "done"),
            (413, vec![], "done"),
            (500, vec![], "done"),
            (401, vec![], "signed out"),
            (409, vec![], "retry 15"),
            (429, retry("3"), "retry 3"),
            (429, vec![], "retry 10"),
            (503, retry("500"), "retry 60"),
            (503, vec![], "retry 5"),
        ] {
            let (base, seen) = server(move |_| (status, headers.clone(), br#"{"error":"x"}"#.to_vec())).await;
            let got = match upload(&client(), &format!("{base}/faces"), "tok123", &own).await {
                Upload::Done => "done".to_string(),
                Upload::SignedOut => "signed out".to_string(),
                Upload::RetryAfter(d) => format!("retry {}", d.as_secs()),
            };
            assert_eq!(got, want, "status {status}");
            let seen = seen.lock().unwrap();
            assert_eq!(seen.len(), 1);
            assert_eq!(seen[0].0, "PUT /faces/self");
            assert_eq!(seen[0].1, "tok123");
            assert_eq!(seen[0].2, vec![b'x'; 10]);
        }
    }

    #[tokio::test]
    async fn upload_skips_an_empty_missing_or_oversized_file() {
        let (base, seen) = server(|_| (200, vec![], b"{}".to_vec())).await;
        let api = format!("{base}/faces");
        let (_t, empty) = own_file(0);
        assert!(matches!(upload(&client(), &api, "t", &empty).await, Upload::Done));
        assert!(matches!(upload(&client(), &api, "t", &empty.with_file_name("missing.jslot")).await, Upload::Done));
        let (_t2, big) = own_file(faces::MAX_UPLOAD as usize + 1);
        assert!(matches!(upload(&client(), &api, "t", &big).await, Upload::Done));
        assert!(seen.lock().unwrap().is_empty(), "nothing was sent");
        // A server that isn't there is a log line, not a failure.
        let (_t3, small) = own_file(5);
        assert!(matches!(upload(&client(), "http://127.0.0.1:1/faces", "t", &small).await, Upload::Done));
    }

    #[tokio::test]
    async fn list_reads_answers_and_failures() {
        let body = br#"{"faces":[{"name":"aff000003-0123456789abcdef"},{"name":"../evil"}],"listEvery":2}"#.to_vec();
        let (base, seen) = server(move |_| (200, vec![], body.clone())).await;
        let Ok(answer) = list(&client(), &format!("{base}/faces"), "tok").await else { panic!("200 should read") };
        assert_eq!(answer.names, vec!["aff000003-0123456789abcdef"]);
        assert_eq!(answer.every, Duration::from_secs(2));
        assert_eq!(seen.lock().unwrap()[0], ("GET /faces/list".to_string(), "tok".to_string(), vec![]));

        let retry = |s: &str| vec![("retry-after", s.to_string())];
        for (status, headers, want) in [(401, vec![], "signed out"), (429, retry("7"), "slower 7"), (503, vec![], "slower"), (500, vec![], "failed answer 500")] {
            let (base, _) = server(move |_| (status, headers.clone(), vec![])).await;
            let got = match list(&client(), &format!("{base}/faces"), "t").await {
                Ok(_) => "ok".to_string(),
                Err(List::SignedOut) => "signed out".to_string(),
                Err(List::Slower(Some(d))) => format!("slower {}", d.as_secs()),
                Err(List::Slower(None)) => "slower".to_string(),
                Err(List::Failed(e)) => format!("failed {e}"),
            };
            assert_eq!(got, want, "status {status}");
        }
        // Not JSON, and too long.
        let (base, _) = server(|_| (200, vec![], b"nope".to_vec())).await;
        assert!(matches!(list(&client(), &format!("{base}/faces"), "t").await, Err(List::Failed(_))));
        let (base, _) = server(|_| (200, vec![], vec![b' '; faces::MAX_LIST + 1])).await;
        assert!(matches!(list(&client(), &format!("{base}/faces"), "t").await, Err(List::Failed(_))));
    }

    #[tokio::test]
    async fn fetch_saves_only_a_face_that_checks_out() {
        let t = tempfile::tempdir().unwrap();
        let dir = faces::folder(t.path()).unwrap();
        let good = br#"{"headParts":[]}"#.to_vec();
        let name = named(&good);
        // One slider too many in the vanilla numbers: the hash matches the
        // contents, the structure doesn't.
        let long = format!(r#"{{"morphs":{{"default":{{"presets":[0,0,0,0,0],"morphs":[{}]}}}}}}"#, vec!["0"; 19].join(",")).into_bytes();
        let long_name = named(&long);
        let (g, n, l, ln) = (good.clone(), name.clone(), long.clone(), long_name.clone());
        let (base, seen) = server(move |key| match key {
            k if k == format!("GET /faces/f/{n}.jslot") => (200, vec![], g.clone()),
            k if k == format!("GET /faces/f/{ln}.jslot") => (200, vec![], l.clone()),
            "GET /faces/f/aff000004-0123456789abcdef.jslot" => (200, vec![], b"{}".to_vec()),
            "GET /faces/f/aff000005-0123456789abcdef.jslot" => (200, vec![], vec![b' '; faces::MAX_FACE + 1]),
            "GET /faces/f/aff000006-0123456789abcdef.jslot" => (404, vec![], vec![]),
            "GET /faces/f/aff000007-0123456789abcdef.jslot" => (429, vec![], vec![]),
            _ => (500, vec![], vec![]),
        })
        .await;
        let api = format!("{base}/faces");
        assert!(fetch(&client(), &api, "tok", &dir, &name).await);
        assert_eq!(std::fs::read(dir.join(format!("{name}.jslot"))).unwrap(), good);
        // Wrong hash, bad structure, too big, gone, a server error: nothing saved and the loop goes on.
        for other in [&long_name, "aff000004-0123456789abcdef", "aff000005-0123456789abcdef", "aff000006-0123456789abcdef", "aff000008-0123456789abcdef"] {
            assert!(fetch(&client(), &api, "tok", &dir, other).await, "{other}");
        }
        // Busy: the caller stops fetching.
        assert!(!fetch(&client(), &api, "tok", &dir, "aff000007-0123456789abcdef").await);
        assert_eq!(faces::saved(&dir), BTreeSet::from([name]));
        assert!(std::fs::read_dir(&dir).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().ends_with(".tmp")));
        assert_eq!(seen.lock().unwrap()[0].1, "tok");
    }

    #[test]
    fn retry_after_is_kept_between_one_and_sixty_seconds() {
        let secs = |v: &str| {
            let r = reqwest::Response::from(http::Response::builder().status(429).header("retry-after", v).body(Vec::<u8>::new()).unwrap());
            retry_after(&r).map(|d| d.as_secs())
        };
        assert_eq!(secs("0"), Some(1));
        assert_eq!(secs(" 12 "), Some(12));
        assert_eq!(secs("9999"), Some(60));
        assert_eq!(secs("soon"), None);
        assert_eq!(secs("-1"), None);
    }

    #[test]
    fn short_keeps_the_servers_reason_brief() {
        assert_eq!(short(r#"{"error":"slot","message":"not ready"}"#), "not ready");
        assert_eq!(short(r#"{"error":"slot"}"#), "slot");
        assert_eq!(short("plain text"), "plain text");
        assert_eq!(short(&"y".repeat(500)).len(), 200);
    }

    #[tokio::test]
    async fn run_saves_faces_then_hands_back_the_list_for_tidying() {
        let t = tempfile::tempdir().unwrap();
        let good = br#"{"headParts":[]}"#.to_vec();
        let name = named(&good);
        let (g, n) = (good.clone(), name.clone());
        let list_body = format!(r#"{{"faces":[{{"name":"{name}"}}]}}"#).into_bytes();
        let (base, _) = server(move |key| match key {
            "GET /faces/list" => (200, vec![], list_body.clone()),
            k if k == format!("GET /faces/f/{n}.jslot") => (200, vec![], g.clone()),
            _ => (404, vec![], vec![]),
        })
        .await;
        let stop = Arc::new(AtomicBool::new(false));
        let game = t.path().to_path_buf();
        let task = tokio::spawn(run(client(), base, "tok".into(), game.clone(), stop.clone()));
        let saved = game.join(faces::FOLDER).join(format!("{name}.jslot"));
        for _ in 0..50 {
            if saved.exists() && faces::remembered(&game).contains(&name) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        stop.store(true, Ordering::SeqCst);
        let last = task.await.unwrap();
        assert_eq!(std::fs::read(&saved).unwrap(), good);
        assert_eq!(last, Some(BTreeSet::from([name.clone()])));
        // Closing the game tidies away faces that left the list, never self.jslot.
        let ad = game.join(faces::FOLDER);
        std::fs::write(ad.join(faces::SELF_FILE), "{}").unwrap();
        tidy(&game, Some(BTreeSet::new()));
        assert!(!saved.exists());
        assert!(ad.join(faces::SELF_FILE).exists());
    }

    #[tokio::test]
    async fn run_stops_on_sign_out() {
        let t = tempfile::tempdir().unwrap();
        let (base, seen) = server(|_| (401, vec![], vec![])).await;
        let stop = Arc::new(AtomicBool::new(false));
        let last = tokio::time::timeout(Duration::from_secs(10), run(client(), base, "tok".into(), t.path().to_path_buf(), stop)).await.expect("run should stop by itself on a 401");
        assert_eq!(last, None);
        assert_eq!(seen.lock().unwrap().len(), 1, "one list asked, then stopped");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn run_and_tidy_stay_away_from_a_linked_folder() {
        let t = tempfile::tempdir().unwrap();
        let elsewhere = t.path().join("elsewhere");
        let ad = elsewhere.join("SKSE/Plugins/CharGen/Presets/ad");
        std::fs::create_dir_all(&ad).unwrap();
        let game = t.path().join("game");
        std::fs::create_dir_all(&game).unwrap();
        std::os::unix::fs::symlink(&elsewhere, game.join("Data")).unwrap();
        let (base, seen) = server(|_| (200, vec![], b"{\"faces\":[]}".to_vec())).await;
        let stop = Arc::new(AtomicBool::new(false));
        assert_eq!(run(client(), base, "tok".into(), game.clone(), stop).await, None);
        assert!(seen.lock().unwrap().is_empty(), "off for the session: no requests");
        let stale = ad.join("aff000003-0123456789abcdef.jslot");
        std::fs::write(&stale, "{}").unwrap();
        tidy(&game, Some(BTreeSet::new()));
        assert!(stale.exists(), "tidying never follows the link");
    }
}
