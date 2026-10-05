//! Signed server lists (triple check B-launcher-3). The lists that decide
//! what the launcher installs are signed with the launcher's update key,
//! the same minisign key whose public half the updater already carries
//! (`tauri.conf.json`), and served with a `<list>.minisig` beside them.
//!
//! Each signature's trusted comment names the list and when it was signed:
//! `aetherial-dawn-list:<name>:<unix seconds>`. A signature for one list
//! never passes for another, and a list older than the newest one already
//! accepted from that server is refused, so an old signed list can't be
//! served again.
//!
//! Rollout is sticky: a list with no signature still loads until its server
//! has once served it signed; from then on an unsigned copy is refused.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use base64::Engine;
use minisign_verify::{PublicKey, Signature};

/// The lists that are signed, as served under the server's base URL.
pub const MODS: &str = "mods.json";
pub const MANIFEST: &str = "client/manifest.json";
pub const COLLECTION: &str = "aetherial-collection.json";
pub const MASTERS: &str = "masters.json";
pub const SERVER_LANE: &str = "server-lane.json";
pub const PATCH_INDEX: &str = crate::patcher::INDEX;
pub const LISTS: [&str; 6] = [MODS, MANIFEST, COLLECTION, MASTERS, SERVER_LANE, PATCH_INDEX];

/// The trusted comment's prefix. `listsign` writes the same.
pub const PREFIX: &str = "aetherial-dawn-list:";

const TAURI_CONF: &str = include_str!("../../src-tauri/tauri.conf.json");

/// The update key's public half, read from the updater's own setting so the
/// two can never differ.
pub fn update_key() -> Result<PublicKey, String> {
    let conf: serde_json::Value = serde_json::from_str(TAURI_CONF).map_err(|e| e.to_string())?;
    let b64 = conf.pointer("/plugins/updater/pubkey").and_then(|v| v.as_str()).ok_or("no updater pubkey")?;
    let text = base64::engine::general_purpose::STANDARD.decode(b64).map_err(|e| e.to_string())?;
    PublicKey::decode(&String::from_utf8_lossy(&text)).map_err(|e| e.to_string())
}

/// The URL of a list's signature.
pub fn sig_url(list_url: &str) -> String {
    format!("{list_url}.minisig")
}

/// A list's signature as the server serves it, or None when there is none
/// (or it can't be fetched; the sticky rule below decides what that means).
pub async fn fetch_sig(client: &reqwest::Client, list_url: &str) -> Option<String> {
    let r = client.get(sig_url(list_url)).timeout(std::time::Duration::from_secs(8)).send().await.ok()?;
    if !r.status().is_success() {
        return None;
    }
    r.text().await.ok()
}

/// Checks one signature: right key, untouched bytes, and a trusted comment
/// naming `name`. Returns the signing time. Takes the minisign text, or that
/// text base64-wrapped as `tauri signer sign` writes it.
pub fn verify(key: &PublicKey, body: &[u8], sig: &str, name: &str) -> Result<u64, String> {
    let sig = sig.trim();
    let text = if sig.starts_with("untrusted comment:") {
        sig.to_string()
    } else {
        let raw = base64::engine::general_purpose::STANDARD.decode(sig).map_err(|_| "the signature file is unreadable".to_string())?;
        String::from_utf8(raw).map_err(|_| "the signature file is unreadable".to_string())?
    };
    let sig = Signature::decode(&text).map_err(|_| "the signature file is unreadable".to_string())?;
    key.verify(body, &sig, false).map_err(|_| "the signature doesn't match the list".to_string())?;
    let (signed_name, time) = sig
        .trusted_comment()
        .strip_prefix(PREFIX)
        .and_then(|r| r.rsplit_once(':'))
        .ok_or_else(|| "the signature isn't for a server list".to_string())?;
    if signed_name != name {
        return Err(format!("the signature is for {signed_name}, not {name}"));
    }
    time.parse().map_err(|_| "the signature has no signing time".to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Signed by the update key at this time.
    Verified(u64),
    /// No signature, and this server has never signed this list.
    Unsigned,
}

/// Newest accepted signing time per "<server>|<list>".
type Seen = BTreeMap<String, u64>;

static STORE: OnceLock<PathBuf> = OnceLock::new();
static LOCK: Mutex<()> = Mutex::new(());

/// Where the newest accepted times are kept (the launcher's own data
/// folder). Without one, nothing is remembered between runs.
pub fn set_store(path: PathBuf) {
    let _ = STORE.set(path);
}

/// Decides whether a fetched list may be used. `base` is the server's base
/// URL; `sig` is what `fetch_sig` returned.
pub fn accept(base: &str, name: &str, body: &[u8], sig: Option<&str>) -> Result<Verdict, String> {
    let key = update_key()?;
    accept_in(STORE.get().map(PathBuf::as_path), &key, base, name, body, sig)
}

pub fn accept_in(store: Option<&Path>, key: &PublicKey, base: &str, name: &str, body: &[u8], sig: Option<&str>) -> Result<Verdict, String> {
    let _held = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let id = format!("{}|{name}", base.trim_end_matches('/').to_ascii_lowercase());
    let mut seen: Seen = store.and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let Some(sig) = sig else {
        return match seen.get(&id) {
            Some(_) => Err(format!("{name} came without its signature, but this server signs it")),
            None => Ok(Verdict::Unsigned),
        };
    };
    let time = verify(key, body, sig, name).map_err(|e| format!("{name}: {e}"))?;
    if let Some(&last) = seen.get(&id) {
        if time < last {
            return Err(format!("{name} is an older signed copy ({time}) than one already used ({last})"));
        }
        if time == last {
            return Ok(Verdict::Verified(time));
        }
    }
    seen.insert(id, time);
    if let Some(p) = store {
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let tmp = p.with_extension("tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&seen).unwrap_or_default())
            .and_then(|_| std::fs::rename(&tmp, p))
            .map_err(|e| format!("couldn't save {}: {e}", p.display()))?;
    }
    Ok(Verdict::Verified(time))
}

/// Fetches a list's signature and decides, in one step.
pub async fn check(client: &reqwest::Client, base: &str, name: &str, body: &[u8]) -> Result<Verdict, String> {
    let url = format!("{}/{name}", base.trim_end_matches('/'));
    let sig = fetch_sig(client, &url).await;
    accept(base, name, body, sig.as_deref())
}

/// What a player sees when a list is refused.
pub const REFUSED: &str = "The server's file list couldn't be verified. Staff need to sign it again; try again later.";

#[cfg(test)]
mod tests {
    use super::*;

    struct Keys {
        sk: minisign::SecretKey,
        pk: PublicKey,
    }

    fn keys() -> Keys {
        let kp = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        let pk = PublicKey::decode(&kp.pk.to_box().unwrap().to_string()).unwrap();
        Keys { sk: kp.sk, pk }
    }

    fn sign(k: &Keys, body: &[u8], comment: &str) -> String {
        minisign::sign(None, &k.sk, body, Some(comment), None).unwrap().to_string()
    }

    const BASE: &str = "https://example.test/launcher/";

    #[test]
    fn the_update_key_is_read_from_the_updater_setting() {
        assert!(update_key().is_ok());
    }

    #[test]
    fn a_signed_list_is_accepted_and_remembered() {
        let k = keys();
        let t = tempfile::tempdir().unwrap();
        let store = t.path().join("seen.json");
        let body = br#"{"mods":[]}"#;
        let sig = sign(&k, body, "aetherial-dawn-list:mods.json:100");
        assert_eq!(accept_in(Some(&store), &k.pk, BASE, MODS, body, Some(&sig)), Ok(Verdict::Verified(100)));
        assert_eq!(accept_in(Some(&store), &k.pk, BASE, MODS, body, Some(&sig)), Ok(Verdict::Verified(100)));
        // The same server's other lists are still on their own.
        assert_eq!(accept_in(Some(&store), &k.pk, BASE, MASTERS, b"{}", None), Ok(Verdict::Unsigned));
        // Once signed, an unsigned copy is refused (base URL case and slash don't matter).
        assert!(accept_in(Some(&store), &k.pk, "https://EXAMPLE.test/launcher", MODS, body, None).is_err());
    }

    #[test]
    fn the_tauri_base64_form_is_accepted() {
        let k = keys();
        let body = b"{}";
        let sig = base64::engine::general_purpose::STANDARD.encode(sign(&k, body, "aetherial-dawn-list:masters.json:5"));
        assert_eq!(verify(&k.pk, body, &sig, MASTERS), Ok(5));
    }

    #[test]
    fn bad_signatures_are_refused() {
        let k = keys();
        let other = keys();
        let body = br#"{"files":[]}"#;
        let good = sign(&k, body, "aetherial-dawn-list:client/manifest.json:7");
        assert_eq!(verify(&k.pk, body, &good, MANIFEST), Ok(7));
        // A changed byte.
        assert!(verify(&k.pk, br#"{"files":[1]}"#, &good, MANIFEST).is_err());
        // Another list's signature.
        assert!(verify(&k.pk, body, &good, MODS).unwrap_err().contains("client/manifest.json"));
        // Another key.
        assert!(verify(&other.pk, body, &good, MANIFEST).is_err());
        // A signature not made for a server list (an update's, say).
        assert!(verify(&k.pk, body, &sign(&k, body, "timestamp:7\tfile:x.exe"), MANIFEST).is_err());
        assert!(verify(&k.pk, body, &sign(&k, body, "aetherial-dawn-list:client/manifest.json:soon"), MANIFEST).is_err());
        assert!(verify(&k.pk, body, "not a signature", MANIFEST).is_err());
        assert!(verify(&k.pk, body, "", MANIFEST).is_err());
    }

    #[test]
    fn an_older_signed_copy_is_refused() {
        let k = keys();
        let t = tempfile::tempdir().unwrap();
        let store = t.path().join("seen.json");
        let new = sign(&k, b"new", "aetherial-dawn-list:mods.json:200");
        let old = sign(&k, b"old", "aetherial-dawn-list:mods.json:100");
        assert_eq!(accept_in(Some(&store), &k.pk, BASE, MODS, b"new", Some(&new)), Ok(Verdict::Verified(200)));
        assert!(accept_in(Some(&store), &k.pk, BASE, MODS, b"old", Some(&old)).unwrap_err().contains("older"));
        // A newer one moves the mark on.
        let newer = sign(&k, b"newer", "aetherial-dawn-list:mods.json:300");
        assert_eq!(accept_in(Some(&store), &k.pk, BASE, MODS, b"newer", Some(&newer)), Ok(Verdict::Verified(300)));
        assert!(accept_in(Some(&store), &k.pk, BASE, MODS, b"new", Some(&new)).is_err());
        // Another server keeps its own marks.
        assert_eq!(accept_in(Some(&store), &k.pk, "https://other.test", MODS, b"old", Some(&old)), Ok(Verdict::Verified(100)));
    }

    #[test]
    fn a_bad_signature_never_moves_the_mark() {
        let k = keys();
        let t = tempfile::tempdir().unwrap();
        let store = t.path().join("seen.json");
        let wrong = sign(&k, b"x", "aetherial-dawn-list:mods.json:500");
        assert!(accept_in(Some(&store), &k.pk, BASE, MODS, b"y", Some(&wrong)).is_err());
        assert!(!store.exists());
        assert_eq!(accept_in(Some(&store), &k.pk, BASE, MODS, b"y", None), Ok(Verdict::Unsigned));
    }
}
