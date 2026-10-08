//! Signs the files the launcher only trusts when signed (feedsig.rs). Runs on
//! the server after `mods.json` or `client/manifest.json` changes:
//!
//!     sign-feed sign <private-key.pem> <web root>   writes <file>.sig next to each
//!     sign-feed verify <web root>                    checks them against the key in the launcher
//!     sign-feed pubkey <private-key.pem>             prints the public key (hex) to pin
//!
//! `<web root>` is the folder served at the launcher's base address. The
//! private key is an Ed25519 key in PKCS#8 PEM (`openssl genpkey -algorithm
//! ed25519`), kept on the server and never in a repo.

use ed25519_dalek::pkcs8::DecodePrivateKey;
use ed25519_dalek::SigningKey;
use launcher_core::feedsig::{self, Feed};
use std::path::Path;

const FEEDS: [Feed; 2] = [Feed::Mods, Feed::Manifest];

fn die(msg: String) -> ! {
    eprintln!("sign-feed: {msg}");
    std::process::exit(1);
}

fn load_key(path: &str) -> SigningKey {
    let pem = std::fs::read_to_string(path).unwrap_or_else(|e| die(format!("can't read {path}: {e}")));
    SigningKey::from_pkcs8_pem(&pem).unwrap_or_else(|e| die(format!("{path} isn't an Ed25519 PKCS#8 PEM private key: {e}")))
}

fn present(root: &Path) -> Vec<(Feed, std::path::PathBuf)> {
    FEEDS.iter().map(|f| (*f, root.join(f.path()))).filter(|(_, p)| p.is_file()).collect()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["pubkey", key] => println!("{}", hex::encode(load_key(key).verifying_key().to_bytes())),
        ["sign", key, root] => {
            let key = load_key(key);
            let pinned = feedsig::pinned_keys();
            if !pinned.contains(&key.verifying_key()) {
                die(format!(
                    "this key ({}) isn't one the launcher trusts; launchers would refuse what it signs. Pin it in feedsig::PUBLIC_KEYS first.",
                    hex::encode(key.verifying_key().to_bytes())
                ));
            }
            let files = present(Path::new(root));
            if files.is_empty() {
                die(format!("no mods.json or client/manifest.json under {root}"));
            }
            for (feed, path) in files {
                let bytes = std::fs::read(&path).unwrap_or_else(|e| die(format!("can't read {}: {e}", path.display())));
                let sig_path = format!("{}.sig", path.display());
                // Replace the signature in one step so a launcher never reads half of it.
                let tmp = format!("{sig_path}.tmp");
                std::fs::write(&tmp, feedsig::sign(&key, feed, &bytes)).and_then(|_| std::fs::rename(&tmp, &sig_path)).unwrap_or_else(|e| die(format!("can't write {sig_path}: {e}")));
                println!("signed {}", path.display());
            }
        }
        ["verify", root] => {
            let files = present(Path::new(root));
            if files.is_empty() {
                die(format!("no mods.json or client/manifest.json under {root}"));
            }
            let trust = feedsig::Trust::new(feedsig::pinned_keys(), true, None);
            let mut bad = false;
            for (feed, path) in files {
                let bytes = std::fs::read(&path).unwrap_or_else(|e| die(format!("can't read {}: {e}", path.display())));
                let sig = std::fs::read(format!("{}.sig", path.display())).ok();
                match trust.check(feed, &bytes, sig.as_deref()) {
                    Ok(_) => println!("ok      {}", path.display()),
                    Err(e) => {
                        bad = true;
                        println!("BAD     {}: {e}", path.display());
                    }
                }
            }
            if bad {
                std::process::exit(1);
            }
        }
        _ => {
            eprintln!("usage: sign-feed sign <private-key.pem> <web root> | verify <web root> | pubkey <private-key.pem>");
            std::process::exit(2);
        }
    }
}
