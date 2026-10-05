//! `listsign <dir>`: signs each server list found in `<dir>` (as the server
//! serves it: mods.json, client/manifest.json, ...) and writes
//! `<list>.minisig` beside it. The key comes from LIST_SIGNING_KEY and
//! LIST_SIGNING_KEY_PASSWORD (the updater's secrets, base64 as Tauri keeps
//! it); it is never written anywhere. Each signature is checked against the
//! updater's public key in tauri.conf.json before it is kept.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use base64::Engine;

/// Must match launcher_core::listsig.
const PREFIX: &str = "aetherial-dawn-list:";
const LISTS: [&str; 6] = ["mods.json", "client/manifest.json", "aetherial-collection.json", "masters.json", "server-lane.json", "patches/index.json"];

fn b64(s: &str) -> Result<String, String> {
    let raw = base64::engine::general_purpose::STANDARD.decode(s.trim()).map_err(|e| format!("not base64: {e}"))?;
    String::from_utf8(raw).map_err(|e| e.to_string())
}

fn run(dir: &Path, conf: &Path) -> Result<usize, String> {
    let conf: serde_json::Value = serde_json::from_slice(&std::fs::read(conf).map_err(|e| format!("{}: {e}", conf.display()))?).map_err(|e| e.to_string())?;
    let pk_text = b64(conf.pointer("/plugins/updater/pubkey").and_then(|v| v.as_str()).ok_or("tauri.conf.json has no updater pubkey")?)?;
    let pk = minisign::PublicKeyBox::from_string(&pk_text).and_then(|b| b.into_public_key()).map_err(|e| format!("updater pubkey: {e}"))?;
    let key = std::env::var("LIST_SIGNING_KEY").map_err(|_| "LIST_SIGNING_KEY is not set")?;
    let password = std::env::var("LIST_SIGNING_KEY_PASSWORD").unwrap_or_default();
    let sk = minisign::SecretKeyBox::from_string(&b64(&key).map_err(|e| format!("LIST_SIGNING_KEY: {e}"))?)
        .and_then(|b| b.into_secret_key(Some(password)))
        .map_err(|e| format!("the signing key couldn't be opened: {e}"))?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|e| e.to_string())?.as_secs();
    let mut signed = 0;
    for name in LISTS {
        let path = dir.join(name);
        let Ok(body) = std::fs::read(&path) else {
            println!("skip {name}: not in {}", dir.display());
            continue;
        };
        serde_json::from_slice::<serde_json::Value>(&body).map_err(|e| format!("{name} isn't valid JSON: {e}"))?;
        let comment = format!("{PREFIX}{name}:{now}");
        let sig = minisign::sign(Some(&pk), &sk, &body[..], Some(&comment), Some("aetherial dawn server list")).map_err(|e| format!("{name}: {e}"))?;
        // sign() with the public key checks the result; a wrong key fails here.
        let out = PathBuf::from(format!("{}.minisig", path.display()));
        std::fs::write(&out, sig.to_string()).map_err(|e| format!("{}: {e}", out.display()))?;
        println!("signed {name} ({} bytes) at {now}", body.len());
        signed += 1;
    }
    Ok(signed)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(dir), conf) = (args.first(), args.get(1).map(String::as_str).unwrap_or("src-tauri/tauri.conf.json")) else {
        eprintln!("usage: listsign <lists dir> [tauri.conf.json]");
        return ExitCode::from(2);
    };
    match run(Path::new(dir), Path::new(conf)) {
        Ok(0) => {
            eprintln!("no server lists found in {dir}");
            ExitCode::FAILURE
        }
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("listsign: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signs_every_list_found_with_a_tauri_style_key() {
        let dir = std::env::temp_dir().join(format!("listsign-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("client")).unwrap();
        std::fs::write(dir.join("mods.json"), br#"{"mods":[]}"#).unwrap();
        std::fs::write(dir.join("client/manifest.json"), br#"{"files":[]}"#).unwrap();
        // A throwaway key in the form Tauri's signer writes (password-locked, base64).
        let kp = minisign::KeyPair::generate_encrypted_keypair(Some("pw".into())).unwrap();
        let enc = |s: String| base64::engine::general_purpose::STANDARD.encode(s);
        let conf = dir.join("tauri.conf.json");
        std::fs::write(&conf, serde_json::json!({"plugins": {"updater": {"pubkey": enc(kp.pk.to_box().unwrap().to_string())}}}).to_string()).unwrap();
        std::env::set_var("LIST_SIGNING_KEY", enc(kp.sk.to_box(None).unwrap().to_string()));
        std::env::set_var("LIST_SIGNING_KEY_PASSWORD", "pw");
        assert_eq!(run(&dir, &conf), Ok(2));
        for name in ["mods.json", "client/manifest.json"] {
            let body = std::fs::read(dir.join(name)).unwrap();
            let sig = minisign::SignatureBox::from_string(&std::fs::read_to_string(dir.join(format!("{name}.minisig"))).unwrap()).unwrap();
            assert!(sig.trusted_comment().unwrap().starts_with(&format!("{PREFIX}{name}:")));
            minisign::verify(&kp.pk, &sig, std::io::Cursor::new(&body), true, false, false).unwrap();
        }
        // Another key than the updater's is refused before anything is written.
        let other = minisign::KeyPair::generate_encrypted_keypair(Some("pw".into())).unwrap();
        std::env::set_var("LIST_SIGNING_KEY", enc(other.sk.to_box(None).unwrap().to_string()));
        assert!(run(&dir, &conf).is_err());
        // A wrong password never signs.
        std::env::set_var("LIST_SIGNING_KEY", enc(kp.sk.to_box(None).unwrap().to_string()));
        std::env::set_var("LIST_SIGNING_KEY_PASSWORD", "nope");
        assert!(run(&dir, &conf).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
