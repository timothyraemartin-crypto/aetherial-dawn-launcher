//! Discord sign-in through the Aetherial Dawn login service ("ad-gate").
//! Contract: aetherial-dawn-discord/CONTRACT.md in the project files.
//!
//! The launcher opens the browser at the service with a random `state`, then
//! polls until the player has finished on Discord. It gets back an opaque token
//! (never shown or logged) that it trades for a game session on every Play.
//! Being banned from, or leaving, the Discord makes the service refuse the
//! token, which signs the player out.
//!
//! The state alone must not be enough to collect the token (a crafted link
//! with a state someone else chose could hand them the player's sign-in;
//! quality check 2026-09-27). So sign-in uses a loopback redirect (RFC 8252)
//! plus a verifier (RFC 7636, S256): the service sends the browser back to
//! 127.0.0.1 on a port only this launcher listens on, with a one-time code,
//! and the launcher trades state + code + its secret verifier for the token
//! (CONTRACT.md "Launcher flow", bot commit aac8e71). A crafted link can't
//! reach an attacker's launcher, and the code is useless without the
//! verifier.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Result;

/// The server's key in the SkyMP master API.
pub const SERVER_KEY: &str = "aetherial-dawn";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    #[serde(default)]
    pub master_api_id: Option<i64>,
    #[serde(default)]
    pub discord_id: Option<String>,
    #[serde(default)]
    pub discord_username: Option<String>,
    #[serde(default)]
    pub discord_discriminator: Option<String>,
    #[serde(default)]
    pub discord_avatar: Option<String>,
}

/// What the service said about a sign-in or a token.
#[derive(Debug, Clone, PartialEq)]
pub enum Answer<T> {
    Ok(T),
    /// Still waiting for the browser (sign-in only).
    Pending,
    /// Token expired or unknown, or the sign-in link expired: sign in again.
    SignedOut(String),
    /// Banned, not in the Discord, or revoked by an admin.
    Refused { error: String, message: String },
    /// Couldn't reach the service, or Discord was unavailable.
    Offline(String),
}

pub fn login_url(base: &str, state: &str, challenge: &str, port: u16) -> String {
    format!("{}/api/users/login-discord?state={state}&challenge={challenge}&port={port}", base.trim_end_matches('/'))
}

/// The path the service sends the browser back to.
pub const CALLBACK_PATH: &str = "/aetherial-login";
/// The page the browser shows once it's back: whose sign-in it was (so a
/// player notices a sign-in they didn't start), or why it didn't go through.
pub fn close_page(line: &str) -> String {
    let esc: String = line.chars().map(|c| match c {
        '<' => "&lt;".to_string(),
        '>' => "&gt;".to_string(),
        '&' => "&amp;".to_string(),
        '"' => "&quot;".to_string(),
        c => c.to_string(),
    }).collect();
    format!("<!doctype html><meta charset=utf-8><title>Aetherial Dawn</title><body style=\"font-family:sans-serif;background:#111;color:#eee;text-align:center;padding-top:20vh\"><h2>{esc}</h2><p>You can close this tab and go back to the launcher.</p>")
}

/// The browser's return, held open until the launcher knows what to say.
pub struct Return {
    pub code: String,
    sock: tokio::net::TcpStream,
}

impl Return {
    /// Answers the browser with `close_page(line)` and closes.
    pub async fn finish(mut self, line: &str) {
        let _ = respond(&mut self.sock, "200 OK", &close_page(line)).await;
    }
}

async fn respond(sock: &mut tokio::net::TcpStream, status: &str, body: &str) -> std::io::Result<()> {
    use tokio::io::AsyncWriteExt;
    let resp = format!("HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n{body}", body.len());
    sock.write_all(resp.as_bytes()).await?;
    sock.shutdown().await
}

/// The browser's return request line ("GET /aetherial-login?state=..&code=.. HTTP/1.1"):
/// its state and code.
pub fn parse_callback(request_line: &str) -> Option<(String, String)> {
    let mut parts = request_line.split_whitespace();
    if parts.next()? != "GET" {
        return None;
    }
    let (path, query) = parts.next()?.split_once('?')?;
    if path != CALLBACK_PATH {
        return None;
    }
    let (mut state, mut code) = (None, None);
    for kv in query.split('&') {
        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
        match k {
            "state" => state = Some(unescape(v)),
            "code" => code = Some(unescape(v)),
            _ => {}
        }
    }
    Some((state?, code.filter(|c| !c.is_empty())?))
}

fn unescape(v: &str) -> String {
    let b = v.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => match std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                Some(x) => {
                    out.push(x);
                    i += 3;
                    continue;
                }
                None => out.push(b'%'),
            },
            b'+' => out.push(b' '),
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A listener on 127.0.0.1 at a free port, for the browser's return.
pub async fn listen() -> Result<(tokio::net::TcpListener, u16)> {
    let l = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
    let port = l.local_addr()?.port();
    Ok((l, port))
}

/// Waits for the browser's return with our state and returns it, still
/// open, with the code. Other requests get a 404 and are ignored; each
/// connection gets 5 seconds in all to send its request line.
pub async fn wait_for_code(listener: tokio::net::TcpListener, state: &str) -> Option<Return> {
    use tokio::io::AsyncReadExt;
    loop {
        let (mut sock, _) = listener.accept().await.ok()?;
        let mut buf = vec![0u8; 8192];
        let mut n = 0;
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while n < buf.len() {
                match sock.read(&mut buf[n..]).await {
                    Ok(0) | Err(_) => break,
                    Ok(k) => n += k,
                }
                if buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
        })
        .await;
        let text = String::from_utf8_lossy(&buf[..n]).into_owned();
        match text.lines().next().and_then(parse_callback).filter(|(s, _)| s == state) {
            Some((_, code)) => return Some(Return { code, sock }),
            None => {
                let _ = respond(&mut sock, "404 Not Found", "").await;
            }
        }
    }
}

/// Trades the code, with the verifier, for the launcher token. One try per
/// sign-in.
pub async fn exchange(client: &reqwest::Client, base: &str, state: &str, code: &str, verifier: &str) -> Answer<SignedIn> {
    let url = format!("{}/api/users/login-discord/token", base.trim_end_matches('/'));
    let resp = match client.post(url).json(&serde_json::json!({ "state": state, "code": code, "verifier": verifier })).timeout(std::time::Duration::from_secs(20)).send().await {
        Ok(r) => r,
        Err(e) => return Answer::Offline(format!("couldn't reach the login service ({e})")),
    };
    let status = resp.status().as_u16();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    match status {
        200 => match serde_json::from_value(body) {
            Ok(v) => Answer::Ok(v),
            Err(e) => Answer::Offline(format!("the login service sent something unexpected ({e})")),
        },
        403 if body["error"] == "bad_verifier" => Answer::SignedOut("That sign-in didn't go through. Try again.".into()),
        403 => Answer::Refused {
            error: body["error"].as_str().unwrap_or("refused").to_string(),
            message: message(&body, "Your Discord account can't use Aetherial Dawn right now."),
        },
        404 => Answer::SignedOut("The sign-in link expired. Try again.".into()),
        // Discord or the membership check didn't answer: stop waiting and say so.
        503 => Answer::Refused {
            error: "unavailable".into(),
            message: message(&body, "Discord isn't answering right now. Try again in a minute."),
        },
        _ => Answer::Offline(message(&body, &format!("the login service answered {status}"))),
    }
}

pub fn new_state() -> String {
    use rand::RngCore;
    let mut b = [0u8; 32];
    rand::rng().fill_bytes(&mut b);
    hex::encode(b)
}

/// A PKCE verifier: 64 hex characters (within RFC 7636's 43-128 unreserved).
pub fn new_verifier() -> String {
    new_state()
}

/// RFC 7636 S256: base64url (no padding) of the SHA-256 of the verifier.
pub fn challenge(verifier: &str) -> String {
    use sha2::{Digest, Sha256};
    base64url(&Sha256::digest(verifier.as_bytes()))
}

fn base64url(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..=c.len() {
            out.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    out
}

fn message(v: &Value, fallback: &str) -> String {
    v["message"].as_str().filter(|s| !s.is_empty()).unwrap_or(fallback).to_string()
}

async fn classify<T: serde::de::DeserializeOwned>(resp: reqwest::Result<reqwest::Response>, pending_on_401: bool) -> Answer<T> {
    let resp = match resp {
        Ok(r) => r,
        Err(e) => return Answer::Offline(format!("couldn't reach the login service ({e})")),
    };
    let status = resp.status().as_u16();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    match status {
        200 => match serde_json::from_value(body) {
            Ok(v) => Answer::Ok(v),
            Err(e) => Answer::Offline(format!("the login service sent something unexpected ({e})")),
        },
        401 if pending_on_401 => Answer::Pending,
        401 => Answer::SignedOut(message(&body, "Please sign in with Discord again.")),
        404 if pending_on_401 => Answer::SignedOut("The sign-in link expired. Try again.".into()),
        403 => Answer::Refused {
            error: body["error"].as_str().unwrap_or("refused").to_string(),
            message: message(&body, "Your Discord account can't use Aetherial Dawn right now."),
        },
        _ => Answer::Offline(message(&body, &format!("the login service answered {status}"))),
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct SignedIn {
    pub token: String,
    #[serde(flatten)]
    pub profile: Profile,
}

/// Checks the token and re-reads the player's profile.
pub async fn me(client: &reqwest::Client, base: &str, token: &str) -> Answer<Profile> {
    let url = format!("{}/api/users/me", base.trim_end_matches('/'));
    classify(client.get(url).header("authorization", token).send().await, false).await
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlaySession {
    pub session: String,
}

/// A fresh game session for this Play.
pub async fn play(client: &reqwest::Client, base: &str, token: &str) -> Answer<PlaySession> {
    let url = format!("{}/api/users/me/play/{SERVER_KEY}", base.trim_end_matches('/'));
    classify(client.post(url).header("authorization", token).json(&serde_json::json!({})).send().await, false).await
}

// ---------- token storage ----------
// On Windows the token is encrypted with DPAPI for the current Windows user,
// so copying the file to another account or PC doesn't carry the sign-in.

#[cfg(windows)]
fn protect(data: &[u8], encrypt: bool) -> Result<Vec<u8>> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB};
    let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
    let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
    let ok = unsafe {
        if encrypt {
            CryptProtectData(&input, std::ptr::null(), std::ptr::null(), std::ptr::null(), std::ptr::null(), 0, &mut out)
        } else {
            CryptUnprotectData(&input, std::ptr::null_mut(), std::ptr::null(), std::ptr::null(), std::ptr::null(), 0, &mut out)
        }
    };
    if ok == 0 {
        return Err(crate::Error::Game("couldn't read the saved sign-in".into()));
    }
    let bytes = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec() };
    unsafe { LocalFree(out.pbData as _) };
    Ok(bytes)
}

#[cfg(not(windows))]
fn protect(data: &[u8], _encrypt: bool) -> Result<Vec<u8>> {
    Ok(data.to_vec())
}

pub fn save_token(path: &std::path::Path, token: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Written beside it and moved over it, so a failed or cut-short save
    // leaves the sign-in that was there before, never a half-written one.
    let part = path.with_extension("part");
    let written = (|| {
        use std::io::Write;
        let mut f = std::fs::File::create(&part)?;
        f.write_all(&protect(token.as_bytes(), true)?)?;
        f.sync_all()?;
        std::fs::rename(&part, path)?;
        Ok(())
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    written
}

pub fn load_token(path: &std::path::Path) -> Option<String> {
    let raw = std::fs::read(path).ok()?;
    String::from_utf8(protect(&raw, false).ok()?).ok().filter(|t| !t.is_empty())
}

pub fn forget_token(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_saved_sign_in_replaces_the_old_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth").join("token.bin");
        save_token(&path, "first").unwrap();
        save_token(&path, "second").unwrap();
        assert_eq!(load_token(&path).as_deref(), Some("second"));
        assert!(!path.with_extension("part").exists());
    }

    #[test]
    fn a_failed_save_keeps_the_sign_in_that_was_there() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("token.bin");
        save_token(&path, "kept").unwrap();
        // Something in the way of the temporary file: the save fails.
        std::fs::create_dir(path.with_extension("part")).unwrap();
        assert!(save_token(&path, "new").is_err());
        assert_eq!(load_token(&path).as_deref(), Some("kept"));
    }

    #[test]
    fn state_is_64_hex() {
        let s = new_state();
        assert_eq!(s.len(), 64);
        assert!(s.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_ne!(s, new_state());
    }

    #[test]
    fn challenge_is_rfc7636_s256() {
        // RFC 7636 appendix B.
        assert_eq!(challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        let v = new_verifier();
        assert_eq!(challenge(&v).len(), 43);
        assert!(login_url("https://x/ad/", "s", "c", 5000).ends_with("/ad/api/users/login-discord?state=s&challenge=c&port=5000"));
    }

    #[test]
    fn reads_the_browser_return() {
        assert_eq!(parse_callback("GET /aetherial-login?state=ab&code=x%2By HTTP/1.1"), Some(("ab".into(), "x+y".into())));
        assert_eq!(parse_callback("GET /aetherial-login?code=1&state=ab HTTP/1.1"), Some(("ab".into(), "1".into())));
        assert_eq!(parse_callback("GET /other?state=ab&code=1 HTTP/1.1"), None);
        assert_eq!(parse_callback("GET /aetherial-login?state=ab HTTP/1.1"), None);
        assert_eq!(parse_callback("POST /aetherial-login?state=ab&code=1 HTTP/1.1"), None);
        assert_eq!(unescape("a%2"), "a%2");
    }

    #[tokio::test]
    async fn loopback_takes_only_our_state() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (l, port) = listen().await.unwrap();
        let wait = tokio::spawn(async move { wait_for_code(l, "mine").await });
        let hit = |path: &'static str| async move {
            let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            s.write_all(format!("GET {path} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes()).await.unwrap();
            let mut out = String::new();
            s.read_to_string(&mut out).await.unwrap();
            out
        };
        assert!(hit("/aetherial-login?state=theirs&code=1").await.starts_with("HTTP/1.1 404"));
        let page = tokio::spawn(hit("/aetherial-login?state=mine&code=good"));
        let ret = wait.await.unwrap().unwrap();
        assert_eq!(ret.code, "good");
        ret.finish("Signed in as <Tim>").await;
        let page = page.await.unwrap();
        assert!(page.contains("Signed in as &lt;Tim&gt;") && page.contains("close this tab"));
    }

    #[test]
    fn token_round_trip() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a/session.bin");
        assert_eq!(load_token(&p), None);
        save_token(&p, "tok").unwrap();
        assert_eq!(load_token(&p).as_deref(), Some("tok"));
        forget_token(&p);
        assert_eq!(load_token(&p), None);
    }

    #[test]
    fn profile_reads_service_body() {
        let v: SignedIn = serde_json::from_str(r#"{"token":"t","masterApiId":7,"discordId":"1","discordUsername":"Lydia","discordDiscriminator":null,"discordAvatar":null}"#).unwrap();
        assert_eq!(v.token, "t");
        assert_eq!(v.profile.master_api_id, Some(7));
        assert_eq!(v.profile.discord_username.as_deref(), Some("Lydia"));
    }
}
