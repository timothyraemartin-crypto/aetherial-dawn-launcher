//! Character portrait for the launcher's card (core/src/portrait.rs has the
//! rules and checks). While the game runs, a finished RaceMenu (the game
//! rewrites ad/self.jslot, the file faces.rs sends) triggers one grab of the
//! game window, kept as a small WebP and sent to the server. The character
//! record and every character's portrait come back from the server, so the
//! card and the in-game Character window read the same data. Every failure is
//! logged and the game carries on.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use launcher_core::portrait::{self, Character};
use launcher_core::faces;

use crate::log;

const TICK: Duration = Duration::from_secs(1);
/// After the preset file changes, wait for the race menu to be gone and the
/// character to be back in view before grabbing.
const SETTLE: Duration = Duration::from_secs(8);
const CACHE: &str = ".aetherial-dawn/portrait/characters.json";
const CHARS: &str = ".aetherial-dawn/portrait/chars";

fn cache_dir(game_dir: &Path) -> PathBuf {
    game_dir.join(CHARS)
}

/// What the card shows.
#[derive(serde::Serialize, Debug, PartialEq)]
pub struct Card {
    pub name: String,
    pub race: String,
    pub sex: String,
    pub hold: String,
    #[serde(rename = "playtimeMin")]
    pub playtime_min: u64,
    pub bio: String,
    /// `data:image/webp;base64,...`, or None until the character has a portrait.
    pub portrait: Option<String>,
}

/// The card for the character seen most recently, from what the last refresh
/// saved. None when nothing has been fetched yet.
pub fn card(game_dir: &Path) -> Option<Card> {
    use base64::Engine;
    let body = std::fs::read(game_dir.join(CACHE)).ok()?;
    let list = portrait::characters_answer(&body).ok()?;
    let c = list.into_iter().max_by_key(|c| c.last_seen)?;
    let portrait = c.portrait.as_ref().and_then(|_| std::fs::read(cache_dir(game_dir).join(format!("{}.webp", c.actor))).ok()).filter(|b| portrait::check_webp(b).is_ok()).map(|b| format!("data:image/webp;base64,{}", base64::engine::general_purpose::STANDARD.encode(b)));
    Some(Card { name: c.name, race: c.race, sex: c.sex, hold: c.hold, playtime_min: c.playtime_min, bio: c.bio, portrait })
}

pub enum Upload {
    Done,
    SignedOut,
}

/// Sends the saved portrait.
pub async fn upload(http: &reqwest::Client, base: &str, token: &str, body: Vec<u8>) -> Upload {
    let url = format!("{}/portrait/self", base.trim_end_matches('/'));
    match http.put(url).header("authorization", token).header("content-type", "image/webp").body(body).timeout(Duration::from_secs(20)).send().await {
        Ok(r) => match r.status().as_u16() {
            200 | 204 => log::line("portrait: sent your portrait"),
            401 => return Upload::SignedOut,
            s => log::line(&format!("portrait: sending your portrait got {s}")),
        },
        Err(e) => log::line(&format!("portrait: couldn't send your portrait: {}", launcher_core::scrub(&e.to_string()))),
    }
    Upload::Done
}

/// The signed-in account's characters.
pub async fn characters(http: &reqwest::Client, base: &str, token: &str) -> Result<(Vec<Character>, Vec<u8>), String> {
    let url = format!("{}/characters/mine", base.trim_end_matches('/'));
    let r = http.get(url).header("authorization", token).timeout(Duration::from_secs(10)).send().await.map_err(|e| launcher_core::scrub(&e.to_string()))?;
    match r.status().as_u16() {
        200 => {
            let body = capped(r, portrait::MAX_ANSWER).await.ok_or("the answer was too long")?;
            let list = portrait::characters_answer(&body).map_err(|e| e.to_string())?;
            Ok((list, body))
        }
        401 => Err("signed out".into()),
        s => Err(format!("answer {s}")),
    }
}

async fn portrait_file(http: &reqwest::Client, base: &str, token: &str, actor: u32) -> Option<Vec<u8>> {
    let url = format!("{}/portrait/{actor}.webp", base.trim_end_matches('/'));
    let r = http.get(url).header("authorization", token).timeout(Duration::from_secs(15)).send().await.ok()?;
    if r.status().as_u16() != 200 {
        return None;
    }
    let body = capped(r, portrait::MAX_BYTES).await?;
    portrait::check_webp(&body).ok()?;
    Some(body)
}

/// Fetches the records and any portrait that is new or changed, then saves
/// the records. False when the account is signed out.
pub async fn refresh(http: &reqwest::Client, base: &str, token: &str, game_dir: &Path) -> bool {
    let (list, body) = match characters(http, base, token).await {
        Ok(x) => x,
        Err(e) if e == "signed out" => return false,
        Err(e) => {
            log::line(&format!("portrait: couldn't get your characters: {e}"));
            return true;
        }
    };
    let old: Vec<Character> = std::fs::read(game_dir.join(CACHE)).ok().and_then(|b| portrait::characters_answer(&b).ok()).unwrap_or_default();
    let dir = cache_dir(game_dir);
    let _ = std::fs::create_dir_all(&dir);
    for c in &list {
        let file = dir.join(format!("{}.webp", c.actor));
        let Some(p) = &c.portrait else {
            let _ = std::fs::remove_file(&file);
            continue;
        };
        let have = old.iter().find(|o| o.actor == c.actor).and_then(|o| o.portrait.as_ref()).is_some_and(|o| o.version == p.version) && file.exists();
        if have {
            continue;
        }
        match portrait_file(http, base, token, c.actor).await {
            Some(b) => {
                if let Err(e) = launcher_core::atomicfile::write(&file, &b) {
                    log::line(&format!("portrait: didn't save the portrait of {}: {e}", c.actor));
                    // Keep the old version number so the next refresh tries again.
                    return true;
                }
            }
            None => return true,
        }
    }
    // Portraits of characters no longer on the account.
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            let keep = e.file_name().to_str().and_then(|n| n.strip_suffix(".webp")).and_then(|n| n.parse::<u32>().ok()).is_some_and(|a| list.iter().any(|c| c.actor == a));
            if !keep {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    if let Err(e) = launcher_core::atomicfile::write(&game_dir.join(CACHE), &body) {
        log::line(&format!("portrait: didn't save your characters: {e}"));
    }
    true
}

#[cfg(windows)]
fn grab() -> Result<Vec<u8>, String> {
    portrait::grab::portrait().map_err(|e| e.to_string())
}

#[cfg(not(windows))]
fn grab() -> Result<Vec<u8>, String> {
    Err("the portrait grab only works on Windows".into())
}

fn modified(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

fn now() -> u64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Runs until `stop` is set.
pub async fn run(http: reqwest::Client, base: String, token: String, game_dir: PathBuf, stop: Arc<AtomicBool>) {
    let preset = game_dir.join(faces::FOLDER).join(faces::SELF_FILE);
    let mut seen = modified(&preset);
    let mut changed_at: Option<Instant> = None;
    let mut last_grab: Option<u64> = None;
    if !refresh(&http, &base, &token, &game_dir).await {
        log::line("portrait: the login service says signed out; stopping for this session");
        return;
    }
    while !stop.load(Ordering::SeqCst) {
        let m = modified(&preset);
        if m != seen {
            seen = m;
            changed_at = Some(Instant::now());
        }
        if changed_at.is_some_and(|t| t.elapsed() >= SETTLE) {
            changed_at = None;
            if portrait::due(last_grab, now()) {
                last_grab = Some(now());
                match tokio::task::spawn_blocking(grab).await {
                    Ok(Ok(body)) => {
                        if let Err(e) = portrait::save_self(&game_dir, &body) {
                            log::line(&format!("portrait: didn't keep your portrait: {e}"));
                        } else if let Upload::SignedOut = upload(&http, &base, &token, body).await {
                            log::line("portrait: the login service says signed out; stopping for this session");
                            return;
                        } else if !refresh(&http, &base, &token, &game_dir).await {
                            return;
                        }
                    }
                    Ok(Err(e)) => log::line(&format!("portrait: no portrait this time: {e}")),
                    Err(_) => {}
                }
            }
        }
        tokio::time::sleep(TICK).await;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    type Seen = Arc<Mutex<Vec<(String, String, Vec<u8>)>>>;
    type Reply = (u16, Vec<u8>);

    /// A local HTTP server answering from `reply("METHOD /path")`.
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
                    let (status, body) = reply(&key);
                    let out = format!("HTTP/1.1 {status} X\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", body.len());
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

    fn webp(seed: u8) -> Vec<u8> {
        let rgba: Vec<u8> = (0..240 * 320 * 4).map(|i| (i as u32 % 251) as u8 ^ seed).collect();
        portrait::encode_webp(&rgba, (240, 320)).unwrap()
    }

    fn answer(version: u32, last_seen: u64) -> Vec<u8> {
        format!(
            r#"{{"characters":[{{"actor":4101,"name":"Ysolda","race":"Nord","sex":"female","hold":"Whiterun","playtimeMin":60,"lastSeen":{last_seen},"bio":"Hi","portrait":{{"version":{version},"url":"/ad/portrait/4101.webp","takenAt":5}}}},{{"actor":7,"name":"Old Hand","race":"Orc","sex":"male","hold":"Rift","playtimeMin":5,"lastSeen":1,"bio":"","portrait":null}}]}}"#
        )
        .into_bytes()
    }

    #[tokio::test]
    async fn upload_sends_the_portrait_with_the_token() {
        let body = webp(1);
        for (status, signed_out) in [(200, false), (204, false), (500, false), (401, true)] {
            let (base, seen) = server(move |_| (status, b"{}".to_vec())).await;
            let r = upload(&client(), &base, "tok", body.clone()).await;
            assert_eq!(matches!(r, Upload::SignedOut), signed_out, "status {status}");
            let seen = seen.lock().unwrap();
            assert_eq!(seen[0].0, "PUT /portrait/self");
            assert_eq!(seen[0].1, "tok");
            assert_eq!(seen[0].2, body);
        }
        // A server that isn't there is a log line, not a failure.
        assert!(matches!(upload(&client(), "http://127.0.0.1:1", "t", body).await, Upload::Done));
    }

    #[tokio::test]
    async fn refresh_saves_records_and_only_new_portraits() {
        let dir = tempfile::tempdir().unwrap();
        let (v7, v8) = (webp(7), webp(8));
        let state = Arc::new(Mutex::new((7u32, v7.clone())));
        let st = state.clone();
        let (base, seen) = server(move |key| {
            let (version, pic) = st.lock().unwrap().clone();
            match key {
                "GET /characters/mine" => (200, answer(version, 100)),
                "GET /portrait/4101.webp" => (200, pic),
                _ => (404, vec![]),
            }
        })
        .await;
        assert!(refresh(&client(), &base, "tok", dir.path()).await);
        let card1 = card(dir.path()).unwrap();
        assert_eq!(card1.name, "Ysolda");
        assert_eq!(card1.playtime_min, 60);
        let url = card1.portrait.unwrap();
        assert!(url.starts_with("data:image/webp;base64,"));
        let fetches = |seen: &Seen| seen.lock().unwrap().iter().filter(|s| s.0 == "GET /portrait/4101.webp").count();
        assert_eq!(fetches(&seen), 1);
        // Same version again: the picture isn't fetched again.
        assert!(refresh(&client(), &base, "tok", dir.path()).await);
        assert_eq!(fetches(&seen), 1);
        // A new version is.
        *state.lock().unwrap() = (8, v8.clone());
        assert!(refresh(&client(), &base, "tok", dir.path()).await);
        assert_eq!(fetches(&seen), 2);
        assert_eq!(std::fs::read(dir.path().join(CHARS).join("4101.webp")).unwrap(), v8);
        // The character with no portrait has none saved.
        assert!(!dir.path().join(CHARS).join("7.webp").exists());
    }

    #[tokio::test]
    async fn refresh_keeps_nothing_from_a_bad_picture_and_reports_sign_out() {
        let dir = tempfile::tempdir().unwrap();
        let (base, _) = server(|key| if key == "GET /characters/mine" { (200, answer(1, 1)) } else { (200, b"not a picture".to_vec()) }).await;
        assert!(refresh(&client(), &base, "tok", dir.path()).await);
        assert!(card(dir.path()).is_none(), "records are saved only after their pictures");
        let (base, _) = server(|_| (401, b"{}".to_vec())).await;
        assert!(!refresh(&client(), &base, "tok", dir.path()).await);
    }

    #[test]
    fn the_card_is_the_character_seen_last() {
        let dir = tempfile::tempdir().unwrap();
        assert!(card(dir.path()).is_none());
        let body = answer(1, 100);
        std::fs::create_dir_all(dir.path().join(CACHE).parent().unwrap()).unwrap();
        std::fs::write(dir.path().join(CACHE), &body).unwrap();
        let c = card(dir.path()).unwrap();
        assert_eq!(c.name, "Ysolda");
        assert!(c.portrait.is_none(), "no picture file yet");
    }
}
