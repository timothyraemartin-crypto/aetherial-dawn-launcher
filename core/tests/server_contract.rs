//! The launcher's sign-in code against the login service contract (contract/launcher-server.json).
//! For each status the contract says the launcher handles, a stand-in server answers with the contract's
//! example body and the launcher's own `auth` code must read it the way the table below says. A changed
//! status, a renamed field or a new status in the contract fails here before a player sees it.
//! (The calls themselves are checked by .github/scripts/contract-check.js.)

use launcher_core::auth::{self, Answer, PlaySession, Profile, SignedIn};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn contract() -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../contract/launcher-server.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("contract/launcher-server.json")).expect("contract is JSON")
}

fn endpoint<'a>(c: &'a Value, id: &str) -> &'a Value {
    c["endpoints"].as_array().unwrap().iter().find(|e| e["id"] == id).unwrap_or_else(|| panic!("contract has no endpoint {id}"))
}

/// Answers every request on a fresh port with `status` and `body`, then closes the connection.
async fn serve(status: u16, body: Value) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else { return };
            let body = body.to_string();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                let mut n = 0;
                loop {
                    match sock.read(&mut buf[n..]).await {
                        Ok(0) | Err(_) => break,
                        Ok(k) => n += k,
                    }
                    if buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                let reply = format!("HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
                let _ = sock.write_all(reply.as_bytes()).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    base
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Want {
    Ok,
    SignedOut,
    Refused,
    Offline,
}

fn kind<T>(a: &Answer<T>) -> Want {
    match a {
        Answer::Ok(_) => Want::Ok,
        Answer::SignedOut(_) => Want::SignedOut,
        Answer::Refused { .. } => Want::Refused,
        Answer::Offline(_) => Want::Offline,
        Answer::Pending => panic!("a finished call can't be pending"),
    }
}

/// What the launcher should do with each answer the contract lists, per endpoint. A key is the status,
/// or "status/error" when the contract gives a separate example for one error code.
const SIGN_IN: &[(&str, Want)] = &[("200", Want::Ok), ("403", Want::Refused), ("403/bad_verifier", Want::SignedOut), ("404", Want::SignedOut), ("503", Want::Refused)];
const ME: &[(&str, Want)] = &[("200", Want::Ok), ("401", Want::SignedOut), ("403", Want::Refused)];
// 503 is the staff-only (maintenance) answer: the player stays signed in and sees the server's message.
const PLAY: &[(&str, Want)] = &[("200", Want::Ok), ("401", Want::SignedOut), ("403", Want::Refused), ("503", Want::Offline)];

fn status_of(key: &str) -> u16 {
    key.split('/').next().unwrap().parse().unwrap()
}

/// The example body for `key` (a 2xx with no listed body gets `{}`).
fn example(e: &Value, key: &str) -> Value {
    match e["responses"][key].get("example") {
        Some(body) => body.clone(),
        None => {
            assert!(status_of(key) < 300, "{}: contract has no example body for {key}", e["id"]);
            serde_json::json!({})
        }
    }
}

/// The contract and this table must list the same statuses, so a new one can't slip past unread.
fn check_table(e: &Value, table: &[(&str, Want)]) {
    let mut listed: Vec<u16> = e["launcherHandles"].as_array().unwrap().iter().map(|s| s.as_u64().unwrap() as u16).collect();
    let mut known: Vec<u16> = table.iter().map(|(k, _)| status_of(k)).collect();
    listed.sort();
    listed.dedup();
    known.sort();
    known.dedup();
    assert_eq!(listed, known, "{}: the contract's launcherHandles and this test's table differ; decide what the launcher does with the change", e["id"]);
}

#[tokio::test]
async fn sign_in_exchange_reads_the_contract_answers() {
    let c = contract();
    let e = endpoint(&c, "login-token");
    check_table(e, SIGN_IN);
    let client = reqwest::Client::new();
    for &(key, want) in SIGN_IN {
        let base = serve(status_of(key), example(e, key)).await;
        let got = auth::exchange(&client, &base, "state", "code", "verifier").await;
        assert_eq!(kind(&got), want, "login-token {key}: {got:?}");
        if let Answer::Ok(SignedIn { token, profile }) = got {
            assert_eq!(token, e["responses"]["200"]["example"]["token"].as_str().unwrap());
            assert!(profile.discord_id.is_some() && profile.discord_username.is_some() && profile.master_api_id.is_some());
        }
    }
}

#[tokio::test]
async fn me_reads_the_contract_answers() {
    let c = contract();
    let e = endpoint(&c, "me");
    check_table(e, ME);
    let client = reqwest::Client::new();
    for &(key, want) in ME {
        let base = serve(status_of(key), example(e, key)).await;
        let got = auth::me(&client, &base, "token").await;
        assert_eq!(kind(&got), want, "me {key}: {got:?}");
        if let Answer::Ok(p) = got {
            let ex = &e["responses"]["200"]["example"];
            assert_eq!(p, Profile {
                master_api_id: ex["masterApiId"].as_i64(),
                discord_id: ex["discordId"].as_str().map(str::to_string),
                discord_username: ex["discordUsername"].as_str().map(str::to_string),
                discord_discriminator: ex["discordDiscriminator"].as_str().map(str::to_string),
                discord_avatar: ex["discordAvatar"].as_str().map(str::to_string),
            });
        }
    }
}

#[tokio::test]
async fn play_reads_the_contract_answers() {
    let c = contract();
    let e = endpoint(&c, "play");
    check_table(e, PLAY);
    let client = reqwest::Client::new();
    for &(key, want) in PLAY {
        let body = example(e, key);
        let base = serve(status_of(key), body.clone()).await;
        let got = auth::play(&client, &base, "token").await;
        assert_eq!(kind(&got), want, "play {key}: {got:?}");
        match got {
            Answer::Ok(PlaySession { session }) => assert_eq!(session, body["session"].as_str().unwrap()),
            // The player is told what the server said.
            Answer::Offline(m) => assert_eq!(m, body["message"].as_str().unwrap()),
            _ => {}
        }
    }
}

#[test]
fn the_contract_names_the_launchers_server_key() {
    assert_eq!(contract()["defaultServerKey"], auth::SERVER_KEY);
}
