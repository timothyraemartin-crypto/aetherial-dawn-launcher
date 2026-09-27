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
const LIST_EVERY: Duration = Duration::from_secs(5);
const LIST_MAX: Duration = Duration::from_secs(60);
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
    // (the file's modified time, when) for the one retry a 409 or 429 gets.
    let mut retry: Option<(Option<SystemTime>, Instant)> = None;
    let mut every = LIST_EVERY;
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
                        break;
                    }
                }
            }
        }
        // Everyone else's faces.
        if Instant::now() >= next_list {
            match list(&http, &api, &token).await {
                Ok(names) => {
                    every = LIST_EVERY;
                    let have = faces::saved(&dir);
                    for name in names.iter().filter(|n| !have.contains(*n)).take(PER_LIST) {
                        if stop.load(Ordering::SeqCst) {
                            break;
                        }
                        fetch(&http, &api, &token, &dir, name).await;
                    }
                    let set: BTreeSet<String> = names.into_iter().collect();
                    faces::remember(&game_dir, &set);
                    last = Some(set);
                }
                Err(List::Slower) => every = (every * 2).min(LIST_MAX),
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
    let dir = game_dir.join(faces::FOLDER);
    if dir.is_dir() {
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
        429 => Upload::RetryAfter(Duration::from_secs(10)),
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
    Slower,
    SignedOut,
    Failed(String),
}

async fn list(http: &reqwest::Client, api: &str, token: &str) -> Result<Vec<String>, List> {
    let r = http.get(format!("{api}/list")).header("authorization", token).timeout(Duration::from_secs(10)).send().await.map_err(|e| List::Failed(launcher_core::scrub(&e.to_string())))?;
    match r.status().as_u16() {
        200 => {
            let body = capped(r, faces::MAX_LIST).await.ok_or_else(|| List::Failed("the list was too long".into()))?;
            faces::list_names(&body).map_err(|e| List::Failed(e.to_string()))
        }
        429 => Err(List::Slower),
        401 => Err(List::SignedOut),
        s => Err(List::Failed(format!("answer {s}"))),
    }
}

async fn fetch(http: &reqwest::Client, api: &str, token: &str, dir: &Path, name: &str) {
    let r = match http.get(format!("{api}/f/{name}.jslot")).header("authorization", token).timeout(Duration::from_secs(15)).send().await {
        Ok(r) => r,
        Err(e) => {
            log::line(&format!("faces: couldn't fetch {name}: {}", launcher_core::scrub(&e.to_string())));
            return;
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
        s => log::line(&format!("faces: fetching {name} got {s}")),
    }
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
