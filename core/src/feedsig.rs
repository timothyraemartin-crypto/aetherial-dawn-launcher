//! Signatures on the server files the launcher acts on: `mods.json` (what to
//! download and install) and `client/manifest.json` (which files to put in
//! the game folder or remove). Both are plain files on a web server, so
//! whoever can edit them could otherwise push code to every player.
//!
//! Each file has a detached Ed25519 signature next to it (`<file>.sig`, 128
//! hex characters) over the exact bytes served, so there is nothing to
//! canonicalize. The message is prefixed with the feed's name so a signature
//! for one file is never accepted for another. The public keys are pinned
//! here; the private key stays on the server and is never in a repo
//! (docs/signing-feeds.md, `sign-feed`).
//!
//! Rollout: until a launcher has seen one valid signature for a feed, an
//! unsigned feed is still accepted, so a launcher released before the server
//! signs keeps working. From the first valid signature on, that launcher
//! refuses the feed whenever the signature is missing, so removing the `.sig`
//! file is not a way around the check. `REQUIRE_SIGNED` makes every launcher
//! refuse unsigned feeds from the start (flip it in a later release).

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use crate::{Error, Result};

/// Raw Ed25519 public keys (hex) whose signatures count. The first is the
/// server key already in `settings::SERVER_PUBLIC_KEYS`; a second, offline
/// key can be added here for rotation.
pub const PUBLIC_KEYS: [&str; 1] = ["bff32f8ada1767e212926e3506db4d64455fd5b730ef16ead8d04012d467d9c6"];

/// When true, a feed without a valid signature is refused even by a launcher
/// that never saw a signed one.
pub const REQUIRE_SIGNED: bool = false;

const SEEN_FILE: &str = "signed-feeds.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Feed {
    Mods,
    Manifest,
}

impl Feed {
    /// Where the feed sits under the server's base address.
    pub fn path(self) -> &'static str {
        match self {
            Feed::Mods => "mods.json",
            Feed::Manifest => "client/manifest.json",
        }
    }
}

/// What gets signed: the feed's name, then its exact bytes.
pub fn message(feed: Feed, bytes: &[u8]) -> Vec<u8> {
    let mut m = format!("aetherial-dawn feed v1\n{}\n", feed.path()).into_bytes();
    m.extend_from_slice(bytes);
    m
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// A pinned key signed exactly these bytes.
    Signed,
    /// No signature is published yet and this launcher never saw one.
    Unsigned,
}

pub fn pinned_keys() -> Vec<VerifyingKey> {
    PUBLIC_KEYS
        .iter()
        .filter_map(|h| hex::decode(h).ok())
        .filter_map(|b| <[u8; 32]>::try_from(b).ok())
        .filter_map(|b| VerifyingKey::from_bytes(&b).ok())
        .collect()
}

/// The contents of `<feed>.sig` for these bytes (the server side, `sign-feed`).
pub fn sign(key: &SigningKey, feed: Feed, bytes: &[u8]) -> String {
    format!("{}\n", hex::encode(key.sign(&message(feed, bytes)).to_bytes()))
}

/// 128 hex characters, surrounding whitespace allowed.
fn parse_signature(text: &[u8]) -> Option<Signature> {
    let text = std::str::from_utf8(text).ok()?.trim();
    let bytes: [u8; 64] = hex::decode(text).ok()?.try_into().ok()?;
    Some(Signature::from_bytes(&bytes))
}

/// `https`, or plain http only to this computer (tests, a local server).
pub fn address_ok(base_url: &str) -> bool {
    let l = base_url.trim().to_ascii_lowercase();
    l.starts_with("https://") || ["http://127.0.0.1", "http://localhost", "http://[::1]"].iter().any(|p| l.strip_prefix(p).is_some_and(|r| r.is_empty() || r.starts_with(['/', ':'])))
}

pub struct Trust {
    keys: Vec<VerifyingKey>,
    require: bool,
    /// Feeds this launcher has seen a valid signature on.
    seen: Mutex<HashSet<String>>,
    dir: Option<PathBuf>,
    /// The file and its `.sig` are two files, so a launcher that asks while
    /// the server is replacing both can see a new one beside an old one; it
    /// asks once more after this long before giving up.
    retry_after: Duration,
}

impl Trust {
    pub fn new(keys: Vec<VerifyingKey>, require: bool, dir: Option<&Path>) -> Trust {
        let seen = dir
            .and_then(|d| std::fs::read(d.join(SEEN_FILE)).ok())
            .and_then(|b| serde_json::from_slice::<Vec<String>>(&b).ok())
            .unwrap_or_default();
        Trust { keys, require, seen: Mutex::new(seen.into_iter().collect()), dir: dir.map(Path::to_path_buf), retry_after: Duration::from_secs(3) }
    }

    /// Whether `bytes` may be used, given the signature file's contents
    /// (`None`: the server has none).
    pub fn check(&self, feed: Feed, bytes: &[u8], sig: Option<&[u8]>) -> Result<Verdict> {
        let name = feed.path();
        let seen = self.seen.lock().unwrap().contains(name);
        match sig.and_then(parse_signature) {
            Some(s) => {
                let msg = message(feed, bytes);
                if self.keys.iter().any(|k| k.verify(&msg, &s).is_ok()) {
                    self.remember(name);
                    Ok(Verdict::Signed)
                } else {
                    Err(Error::FeedSignature(format!("{name}: the signature doesn't match, so the file was changed or isn't from the server")))
                }
            }
            // A missing or unreadable signature (a web server answering
            // with a page for an unknown address) counts as none.
            None if seen || self.require => Err(Error::FeedSignature(format!("{name}: it has no valid signature, and this launcher only uses signed files"))),
            None => Ok(Verdict::Unsigned),
        }
    }

    fn remember(&self, name: &str) {
        let mut seen = self.seen.lock().unwrap();
        if !seen.insert(name.to_string()) {
            return;
        }
        if let Some(dir) = &self.dir {
            let mut all: Vec<_> = seen.iter().cloned().collect();
            all.sort();
            let _ = std::fs::create_dir_all(dir);
            let _ = std::fs::write(dir.join(SEEN_FILE), serde_json::to_vec(&all).unwrap_or_default());
        }
    }

    /// The feed's bytes, after its signature has been checked. A 404 for
    /// the feed itself is `NotPublished`; a 4xx for the signature means none
    /// is published; a server error or no answer for it is an error (the
    /// file can't be confirmed).
    pub async fn fetch(&self, client: &reqwest::Client, base_url: &str, feed: Feed, timeout: Option<Duration>) -> Result<Vec<u8>> {
        match self.fetch_once(client, base_url, feed, timeout).await {
            Err(Error::FeedSignature(_)) => {
                tokio::time::sleep(self.retry_after).await;
                self.fetch_once(client, base_url, feed, timeout).await
            }
            other => other,
        }
    }

    async fn fetch_once(&self, client: &reqwest::Client, base_url: &str, feed: Feed, timeout: Option<Duration>) -> Result<Vec<u8>> {
        if !address_ok(base_url) {
            return Err(Error::FeedSignature(format!("the server address {base_url} doesn't use https, so its files aren't used")));
        }
        let url = format!("{}/{}", base_url.trim_end_matches('/'), feed.path());
        let get = |u: String| {
            let r = client.get(u);
            async move { (if let Some(t) = timeout { r.timeout(t) } else { r }).send().await }
        };
        let resp = get(url.clone()).await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(Error::NotPublished);
        }
        let bytes = resp.error_for_status()?.bytes().await?.to_vec();
        let sig_resp = get(format!("{url}.sig")).await?;
        let sig = if sig_resp.status().is_success() {
            Some(sig_resp.bytes().await?.to_vec())
        } else if sig_resp.status().is_client_error() {
            None
        } else {
            sig_resp.error_for_status()?;
            None
        };
        self.check(feed, &bytes, sig.as_deref())?;
        Ok(bytes)
    }
}

static TRUST: OnceLock<Trust> = OnceLock::new();

/// Sets where the launcher remembers which feeds were signed. Call once at
/// startup, before any feed is fetched.
pub fn init(dir: &Path) {
    let _ = TRUST.set(Trust::new(pinned_keys(), REQUIRE_SIGNED, Some(dir)));
}

pub fn trust() -> &'static Trust {
    TRUST.get_or_init(|| Trust::new(pinned_keys(), REQUIRE_SIGNED, None))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;

    pub(crate) fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    pub(crate) fn trust_for(k: &SigningKey, require: bool, dir: Option<&Path>) -> Trust {
        let mut t = Trust::new(vec![k.verifying_key()], require, dir);
        t.retry_after = Duration::from_millis(50);
        t
    }

    pub(crate) fn sign(k: &SigningKey, feed: Feed, bytes: &[u8]) -> Vec<u8> {
        format!("{}\n", hex::encode(k.sign(&message(feed, bytes)).to_bytes())).into_bytes()
    }

    const BODY: &[u8] = br#"{"mods":[]}"#;

    #[test]
    fn a_good_signature_is_accepted() {
        let k = key(1);
        let t = trust_for(&k, true, None);
        assert_eq!(t.check(Feed::Mods, BODY, Some(&sign(&k, Feed::Mods, BODY))).unwrap(), Verdict::Signed);
    }

    #[test]
    fn a_changed_file_is_refused() {
        let k = key(1);
        let sig = sign(&k, Feed::Mods, BODY);
        for t in [trust_for(&k, false, None), trust_for(&k, true, None)] {
            let err = t.check(Feed::Mods, br#"{"mods":[{"id":"evil"}]}"#, Some(&sig)).unwrap_err();
            assert!(matches!(err, Error::FeedSignature(_)), "{err}");
        }
    }

    #[test]
    fn a_signature_for_another_feed_is_refused() {
        let k = key(1);
        let t = trust_for(&k, false, None);
        assert!(t.check(Feed::Manifest, BODY, Some(&sign(&k, Feed::Mods, BODY))).is_err());
    }

    #[test]
    fn another_key_is_refused() {
        let t = trust_for(&key(1), false, None);
        assert!(t.check(Feed::Mods, BODY, Some(&sign(&key(2), Feed::Mods, BODY))).is_err());
    }

    #[test]
    fn unsigned_is_accepted_only_until_a_signature_was_seen() {
        let k = key(1);
        let t = trust_for(&k, false, None);
        assert_eq!(t.check(Feed::Mods, BODY, None).unwrap(), Verdict::Unsigned);
        t.check(Feed::Mods, BODY, Some(&sign(&k, Feed::Mods, BODY))).unwrap();
        assert!(t.check(Feed::Mods, BODY, None).is_err(), "taking the .sig away must not work any more");
        assert_eq!(t.check(Feed::Manifest, BODY, None).unwrap(), Verdict::Unsigned, "each feed is tracked on its own");
    }

    #[test]
    fn unsigned_is_refused_when_signatures_are_required() {
        assert!(trust_for(&key(1), true, None).check(Feed::Mods, BODY, None).is_err());
    }

    #[test]
    fn a_page_in_place_of_a_signature_counts_as_none_but_a_wrong_signature_never_passes() {
        let k = key(1);
        let t = trust_for(&k, false, None);
        assert_eq!(t.check(Feed::Mods, BODY, Some(b"<html>not found</html>")).unwrap(), Verdict::Unsigned);
        let wrong = "ab".repeat(64);
        assert!(t.check(Feed::Mods, BODY, Some(wrong.as_bytes())).is_err());
        t.check(Feed::Mods, BODY, Some(&sign(&k, Feed::Mods, BODY))).unwrap();
        assert!(t.check(Feed::Mods, BODY, Some(b"<html>not found</html>")).is_err());
    }

    #[test]
    fn a_launcher_remembers_signed_feeds_after_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let k = key(1);
        trust_for(&k, false, Some(dir.path())).check(Feed::Manifest, BODY, Some(&sign(&k, Feed::Manifest, BODY))).unwrap();
        let again = trust_for(&k, false, Some(dir.path()));
        assert!(again.check(Feed::Manifest, BODY, None).is_err());
        assert!(again.check(Feed::Mods, BODY, None).is_ok());
    }

    /// The trust the launcher builds for real (pinned keys), as it would be with `REQUIRE_SIGNED` on.
    fn strict() -> Trust {
        Trust::new(pinned_keys(), true, None)
    }

    #[test]
    fn strict_trust_refuses_unsigned_foreign_signed_and_tampered_feeds_but_never_panics() {
        let t = strict();
        assert!(!pinned_keys().is_empty(), "no pinned key would refuse every feed");
        for feed in [Feed::Mods, Feed::Manifest] {
            // No .sig at all, an empty one, a page in its place, and a signature from another key.
            assert!(matches!(t.check(feed, BODY, None), Err(Error::FeedSignature(_))));
            assert!(matches!(t.check(feed, BODY, Some(b"")), Err(Error::FeedSignature(_))));
            assert!(matches!(t.check(feed, BODY, Some(b"<html>404</html>")), Err(Error::FeedSignature(_))));
            assert!(matches!(t.check(feed, BODY, Some(&sign(&key(9), feed, BODY))), Err(Error::FeedSignature(_))));
        }
    }

    #[test]
    fn a_feed_error_names_the_file_for_the_log_and_the_player_sees_words() {
        let Err(e) = strict().check(Feed::Mods, BODY, None) else { panic!("unsigned feed accepted") };
        let raw = e.to_string();
        assert!(raw.contains("mods.json"), "{raw}");
        assert!(crate::plain_ui(&raw).starts_with("The launcher couldn't confirm"), "{raw}");
    }

    #[test]
    fn the_pinned_key_is_the_servers_script_key() {
        use ed25519_dalek::pkcs8::DecodePublicKey;
        let pem = crate::settings::SERVER_PUBLIC_KEYS[0].1;
        let from_pem = VerifyingKey::from_public_key_pem(pem).unwrap();
        assert_eq!(pinned_keys(), vec![from_pem]);
    }

    #[test]
    fn only_https_or_this_computer() {
        assert!(address_ok("https://vps-d38c928e.vps.ovh.us/launcher"));
        assert!(address_ok("http://127.0.0.1:8080/launcher"));
        assert!(address_ok("http://localhost/launcher"));
        assert!(!address_ok("http://vps-d38c928e.vps.ovh.us/launcher"));
        assert!(!address_ok("http://localhost.evil.example/launcher"));
        assert!(!address_ok("ftp://x"));
    }

    /// A one-file-per-path HTTP server on this computer.
    pub(crate) async fn serve(files: Vec<(&'static str, u16, Vec<u8>)>) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let Ok((mut s, _)) = listener.accept().await else { return };
                let files = files.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 4096];
                    let n = s.read(&mut buf).await.unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]).into_owned();
                    let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
                    let (status, body) = files.iter().find(|(p, _, _)| *p == path).map(|(_, st, b)| (*st, b.clone())).unwrap_or((404, b"no".to_vec()));
                    let head = format!("HTTP/1.1 {status} X\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", body.len());
                    let _ = s.write_all(head.as_bytes()).await;
                    let _ = s.write_all(&body).await;
                });
            }
        });
        format!("http://127.0.0.1:{port}/launcher")
    }

    #[tokio::test]
    async fn fetch_returns_signed_bytes_and_refuses_tampered_ones() {
        let k = key(3);
        let t = trust_for(&k, false, None);
        let http = reqwest::Client::new();
        let base = serve(vec![
            ("/launcher/mods.json", 200, BODY.to_vec()),
            ("/launcher/mods.json.sig", 200, sign(&k, Feed::Mods, BODY)),
            ("/launcher/client/manifest.json", 200, b"{\"changed\":true}".to_vec()),
            ("/launcher/client/manifest.json.sig", 200, sign(&k, Feed::Manifest, b"{}")),
        ])
        .await;
        assert_eq!(t.fetch(&http, &base, Feed::Mods, None).await.unwrap(), BODY);
        assert!(matches!(t.fetch(&http, &base, Feed::Manifest, None).await, Err(Error::FeedSignature(_))));
    }

    #[tokio::test]
    async fn fetch_asks_again_once_when_the_file_and_signature_were_mid_replacement() {
        use std::sync::{Arc, Mutex};
        let k = key(3);
        let t = trust_for(&k, false, None);
        let http = reqwest::Client::new();
        // First round the .sig is the old one; the server finishes before the second.
        let stale = sign(&k, Feed::Mods, b"old");
        let fresh = sign(&k, Feed::Mods, BODY);
        let sig = Arc::new(Mutex::new(stale));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let sig2 = sig.clone();
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            loop {
                let Ok((mut s, _)) = listener.accept().await else { return };
                let mut buf = vec![0u8; 4096];
                let n = s.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).into_owned();
                let is_sig = req.split_whitespace().nth(1).is_some_and(|p| p.ends_with(".sig"));
                let body = if is_sig { let b = sig2.lock().unwrap().clone(); *sig2.lock().unwrap() = fresh.clone(); b } else { BODY.to_vec() };
                let _ = s.write_all(format!("HTTP/1.1 200 X\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", body.len()).as_bytes()).await;
                let _ = s.write_all(&body).await;
            }
        });
        assert_eq!(t.fetch(&http, &format!("http://127.0.0.1:{port}"), Feed::Mods, None).await.unwrap(), BODY);
    }

    #[tokio::test]
    async fn fetch_handles_unsigned_missing_and_broken_servers() {
        let k = key(3);
        let http = reqwest::Client::new();
        let base = serve(vec![("/launcher/mods.json", 200, BODY.to_vec()), ("/launcher/client/manifest.json", 200, b"{}".to_vec()), ("/launcher/client/manifest.json.sig", 500, vec![])]).await;
        // No .sig yet: fine until a signature has been seen.
        assert_eq!(trust_for(&k, false, None).fetch(&http, &base, Feed::Mods, None).await.unwrap(), BODY);
        assert!(trust_for(&k, true, None).fetch(&http, &base, Feed::Mods, None).await.is_err());
        // A server error for the .sig can't be told from tampering.
        assert!(trust_for(&k, false, None).fetch(&http, &base, Feed::Manifest, None).await.is_err());
        // The feed itself missing is "not published".
        let none = serve(vec![]).await;
        assert!(matches!(trust_for(&k, false, None).fetch(&http, &none, Feed::Mods, None).await, Err(Error::NotPublished)));
        // Plain http to another host is refused before any request.
        assert!(matches!(trust_for(&k, false, None).fetch(&http, "http://example.invalid/launcher", Feed::Mods, None).await, Err(Error::FeedSignature(_))));
    }
}
