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
//! quality check 2026-09-27). So, PKCE style (RFC 7636, S256): the launcher
//! keeps a secret verifier, sends only its SHA-256 as `code_challenge` with
//! the state, and shows the verifier on the status call in the
//! `x-code-verifier` header. A service that doesn't know about challenges
//! ignores both.

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

pub fn login_url(base: &str, state: &str, challenge: &str) -> String {
    format!("{}/api/users/login-discord?state={state}&code_challenge={challenge}&code_challenge_method=S256", base.trim_end_matches('/'))
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

/// One poll of the browser sign-in.
pub async fn poll(client: &reqwest::Client, base: &str, state: &str, verifier: &str) -> Answer<SignedIn> {
    let url = format!("{}/api/users/login-discord/status?state={state}", base.trim_end_matches('/'));
    classify(client.get(url).header("x-code-verifier", verifier).send().await, true).await
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
    std::fs::write(path, protect(token.as_bytes(), true)?)?;
    Ok(())
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
        assert!(login_url("https://x/ad/", "s", "c").ends_with("/ad/api/users/login-discord?state=s&code_challenge=c&code_challenge_method=S256"));
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
