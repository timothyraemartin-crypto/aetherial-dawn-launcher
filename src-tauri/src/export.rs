//! The server-mods export (staging runbook A1): when the server publishes
//! server-lane.json naming this PC's Discord account, the launcher fetches
//! those pinned Nexus files with the signed-in Premium account, in the
//! background, into %LOCALAPPDATA%\gg.aetherialdawn.launcher\server-lane
//! (never the Skyrim folder), keeps their plugins and zips them to
//! server-lane.zip next to a sha256 line. It uploads nothing: the zip stays
//! on this PC.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use std::time::Duration;

use launcher_core::{modlist, nexus, serverlane, watch};
use tauri::{AppHandle, Manager};

use crate::AppState;

static RUNNING: AtomicBool = AtomicBool::new(false);
/// Longest wait for a request to answer, or for the next piece of a download.
const STALL: Duration = Duration::from_secs(60);
/// Why a mod stopped when the game started: it's fetched again after.
const GAME_RUNNING: &str = "the game started";

/// Every export line goes through here: download addresses (Nexus CDN links
/// carry md5, expires and user_id) are hidden, since Copy diagnostics puts
/// the log on the clipboard.
fn say(msg: &str) {
    crate::log::line(&launcher_core::scrub(msg));
}

fn game_running() -> bool {
    watch::find_process(watch::GAME_PROCESS).is_some()
}

/// Waits while Skyrim runs, so the export never competes with the game.
async fn wait_for_game_to_close() {
    if game_running() {
        say("export: paused while the game runs");
        while game_running() {
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
        say("export: the game closed; carrying on");
    }
}

async fn in_time<T, E: std::fmt::Display>(what: &str, f: impl std::future::Future<Output = Result<T, E>>) -> Result<T, String> {
    match tokio::time::timeout(STALL, f).await {
        Ok(r) => r.map_err(|e| e.to_string()),
        Err(_) => Err(format!("{what} didn't answer within {} seconds", STALL.as_secs())),
    }
}

/// Whether an export is running now.
pub fn running() -> bool {
    RUNNING.load(Ordering::SeqCst)
}

/// Called as the launcher closes: an export cut off here leaves its finished
/// downloads, and the next start carries on from them.
pub fn on_exit() {
    if running() {
        say("export: the launcher closed before the export finished; finished downloads are kept and it carries on the next time the launcher opens");
    }
}

/// Starts an export in the background when one is due. Only one runs at a time.
pub fn start(app: &AppHandle) {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = run(&app).await {
            say(&format!("export: stopped: {e}"));
        }
        RUNNING.store(false, Ordering::SeqCst);
    });
}

async fn run(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let (base, me, game_dir) = {
        let c = state.config.lock().await;
        (c.base_url.clone(), c.account.as_ref().and_then(|a| a.discord_id.clone()), c.game_dir.clone())
    };
    let lane_root = serverlane::lane_dir(&app.path().app_local_data_dir().map_err(|e| e.to_string())?);
    // A local override (test runs only) replaces the served list and keeps
    // its own state; a broken one stops the export, never falling back.
    let local = {
        let r = lane_root.clone();
        tokio::task::spawn_blocking(move || serverlane::local_override(&r)).await.map_err(|e| e.to_string())?
    }
    .map_err(|e| format!("local override: {e}"))?;
    let (lane, source, root) = match local {
        Some((lane, sha)) => (lane, serverlane::Source::local(&sha), lane_root.join(serverlane::OVERRIDE_RUN)),
        None => {
            let url = format!("{}/server-lane.json", base.trim_end_matches('/'));
            let bytes = match in_time("the server", state.http.get(&url).send()).await {
                Ok(r) if r.status().is_success() => in_time("the server", r.bytes()).await.map_err(|e| format!("server-lane.json: {e}"))?,
                // Not published: nothing to export (every other player's case).
                _ => return Ok(()),
            };
            crate::list_signed(&state.http, &base, launcher_core::listsig::SERVER_LANE, &bytes).await?;
            let lane: serverlane::ServerLane = serde_json::from_slice(&bytes).map_err(|e| format!("server-lane.json: {e}"))?;
            (lane, serverlane::Source::served(), lane_root)
        }
    };
    if me.as_deref() != Some(lane.for_discord_id.as_str()) {
        if source.override_sha256.is_some() {
            say("export: the local override names another Discord account; nothing exported");
        }
        return Ok(());
    }
    serverlane::check(&lane).map_err(|e| e.to_string())?;
    let hash = serverlane::list_hash(&lane);
    if serverlane::done(&root, &hash) || serverlane::gave_up(&root, &hash) {
        return Ok(());
    }
    let Some(key) = crate::mods::nexus_key(app) else {
        say("export: the server lane waits for a Nexus sign-in");
        return Ok(());
    };
    let version = app.package_info().version.to_string();
    let api = nexus::Client { http: &state.http, key: &key, app_version: &version };
    if !in_time("Nexus", api.validate()).await?.is_premium {
        say("export: the server lane needs a Premium Nexus account; nothing downloaded");
        return Ok(());
    }
    say(&format!(
        "export: server lane of {} mods into {} (list {}; source: {})",
        lane.mods.len(),
        root.display(),
        hash,
        match &source.override_sha256 {
            Some(sha) => format!("local override, {} sha256 {sha}", serverlane::OVERRIDE_LIST),
            None => "served server-lane.json".into(),
        }
    ));
    {
        let root = root.clone();
        tokio::task::spawn_blocking(move || serverlane::start_over(&root)).await.map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;
    }
    // The server's masters past the base five (Creation Club content) are
    // checked against this PC's own game files; their bytes never go in the
    // zip, only their names and converted hashes.
    let game_data = game_dir.as_ref().map(|d| d.join("Data"));
    if lane.masters().len() > launcher_core::health::MASTERS.len() && game_data.is_none() {
        return Err("the server's list has Creation Club masters to check, and no Skyrim folder is picked".into());
    }
    let mut plugins = std::collections::BTreeMap::new();
    let mut failed = Vec::new();
    let mut outcomes = Vec::new();
    for m in &lane.mods {
        wait_for_game_to_close().await;
        let mut got = one(&api, &root, m).await;
        // The game started mid-download: wait, then fetch it again once.
        if got.as_ref().is_err_and(|e| e == GAME_RUNNING) {
            wait_for_game_to_close().await;
            got = one(&api, &root, m).await;
        }
        // Playing isn't a failure: stop without counting a try, and carry
        // on (finished downloads kept) when the game next closes.
        if got.as_ref().is_err_and(|e| e == GAME_RUNNING) {
            say("export: stopped for the game; it carries on after the game closes");
            return Ok(());
        }
        let mut out = serverlane::ModOutcome { id: m.entry.id.clone(), name: m.entry.name.clone(), declared: m.plugins.clone(), ..Default::default() };
        match got {
            Ok(picked) => {
                let names: Vec<&str> = picked.iter().map(|p| p.1.as_str()).collect();
                say(&format!("export: {} gave {}", m.entry.name, names.join(", ")));
                out.gave = names.iter().map(|n| n.to_string()).collect();
                if let Err(e) = serverlane::collect(&root, &m.entry.id, &picked, &mut plugins) {
                    failed.push(format!("{}: {e}", m.entry.name));
                    out.error = Some(e.to_string());
                }
            }
            Err(e) => {
                failed.push(format!("{}: {e}", m.entry.name));
                out.error = Some(e);
            }
        }
        outcomes.push(out);
    }
    // What each mod gave or why it didn't, whether or not the run finished.
    match serverlane::report_with(&root, &lane, &hash, &plugins, outcomes, game_data.as_deref()) {
        Ok(r) => {
            // Masters the server couldn't load, named before the zip is refused.
            for p in &r.master_problems {
                say(&format!("export: masters: {p}"));
            }
            say(&format!(
                "export: {} of {} declared plugins collected{}{}",
                r.collected,
                r.declared,
                r.first_failure.as_ref().map(|f| format!("; first failure: {f}")).unwrap_or_default(),
                if r.missing.is_empty() { String::new() } else { format!("; missing: {}", r.missing.iter().map(|(m, p)| format!("{p} ({m})")).collect::<Vec<_>>().join(", ")) }
            ))
        }
        Err(e) => say(&format!("export: couldn't write the report: {e}")),
    }
    if !failed.is_empty() {
        for f in &failed {
            say(&format!("export: failed {f}"));
        }
        let tries = serverlane::failed(&root, &hash).map_err(|e| e.to_string())?;
        let next = if tries >= serverlane::MAX_FAILURES { "it stops trying until the server's list changes" } else { "it tries again the next time the launcher starts or the game closes" };
        return Err(format!("{} of {} mods didn't export (try {tries}); {next}", failed.len(), lane.mods.len()));
    }
    let rec = {
        let root = root.clone();
        let lane = lane.clone();
        let game_data = game_data.clone();
        tokio::task::spawn_blocking(move || serverlane::finish_with(&root, &lane, &hash, plugins, source, game_data.as_deref())).await.map_err(|e| e.to_string())?.map_err(|e| e.to_string())?
    };
    let _ = std::fs::remove_dir_all(root.join("downloads"));
    let _ = std::fs::remove_dir_all(root.join("unpacked"));
    say(&format!(
        "export: {} ready: {} plugins, {} bytes, sha256 {}",
        root.join(serverlane::ZIP_NAME).display(),
        rec.plugins.len(),
        rec.zip_bytes,
        rec.zip_sha256
    ));
    Ok(())
}

/// Downloads (or reuses) one pinned file, unpacks it and picks its plugins.
async fn one(api: &nexus::Client<'_>, root: &Path, m: &serverlane::LaneMod) -> Result<Vec<(std::path::PathBuf, String)>, String> {
    let n = m.entry.nexus.as_ref().ok_or("not a Nexus mod")?;
    let file = n.file.ok_or("not pinned to one file")?;
    let downloads = root.join("downloads");
    let prefix = format!("{}-{file}.", m.entry.id);
    let have = std::fs::read_dir(&downloads).ok().and_then(|rd| rd.flatten().map(|e| e.path()).find(|p| p.file_name().is_some_and(|f| f.to_string_lossy().starts_with(&prefix) && !f.to_string_lossy().ends_with(".part"))));
    let archive = match have.filter(|p| serverlane::verify(m, p).is_ok()) {
        Some(p) => {
            say(&format!("export: {} already downloaded; using it", m.entry.name));
            p
        }
        None => {
            let url = in_time("Nexus", api.download_link(modlist::NEXUS_GAME, n.mod_id, file, None)).await?;
            let name = url.split('?').next().unwrap_or("").rsplit('/').next().unwrap_or("");
            let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).filter(|e| e.len() <= 4 && e.bytes().all(|b| b.is_ascii_alphanumeric())).unwrap_or_else(|| "bin".into());
            let path = downloads.join(format!("{prefix}{ext}"));
            say(&format!("export: downloading {} (mod {}, file {file})", m.entry.name, n.mod_id));
            let tmp = fetch(api.http, &url, &path).await?;
            if let Some(want) = m.archive.as_ref().and_then(|a| a.size_bytes) {
                let got = std::fs::metadata(&tmp).map(|md| md.len()).unwrap_or(0);
                if got != want {
                    say(&format!("export: {} came as {got} bytes; the list expected {want} ({})", m.entry.name, m.archive.as_ref().and_then(|a| a.version.as_deref()).unwrap_or("?")));
                }
            }
            // Checked while still a .part file, so a bad download is never
            // taken for a finished one.
            if let Err(e) = serverlane::verify(m, &tmp) {
                let _ = std::fs::remove_file(&tmp);
                return Err(e.to_string());
            }
            std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
            path
        }
    };
    let (m, root) = (m.clone(), root.to_path_buf());
    tokio::task::spawn_blocking(move || -> Result<_, String> {
        let work = root.join("unpacked").join(&m.entry.id);
        let _ = std::fs::remove_dir_all(&work);
        modlist::extract(&archive, &work).map_err(|e| e.to_string())?;
        if let Some(r) = modlist::fomod_report(&m.entry, &work) {
            say(&format!("export: {} installer options (picks {:?}): {}", m.entry.name, m.entry.fomod, r.join(" || ")));
        }
        // Listed with no plugins: every plugin in the download is named, so
        // the list can pin them next time (whether or not the pick works).
        if m.plugins.is_empty() {
            let all = serverlane::plugins_in(&work);
            say(&format!("export: {} holds {} plugin(s) in its download: {}", m.entry.name, all.len(), if all.is_empty() { "none".to_string() } else { all.join(", ") }));
        }
        let picked = serverlane::pick(&m, &work).map_err(|e| e.to_string());
        // The picked files are copied out before the folder goes.
        let out = picked.and_then(|p| {
            let keep = root.join("unpacked").join(format!("{}.keep", m.entry.id));
            let _ = std::fs::remove_dir_all(&keep);
            std::fs::create_dir_all(&keep).map_err(|e| e.to_string())?;
            let copied = p
                .into_iter()
                .map(|(from, name)| {
                    let to = keep.join(&name);
                    std::fs::copy(&from, &to).map_err(|e| e.to_string())?;
                    Ok((to, name))
                })
                .collect::<Result<Vec<_>, String>>();
            // A copy that fails leaves no part-filled .keep folder behind.
            if copied.is_err() {
                let _ = std::fs::remove_dir_all(&keep);
            }
            copied
        });
        let _ = std::fs::remove_dir_all(&work);
        out
    })
    .await
    .map_err(|e| e.to_string())?
}

/// A plain download into `path`'s .part file, which it returns for the
/// caller to check and rename. Only https. Stops when the connection stalls,
/// runs too slowly, or the game starts.
async fn fetch(http: &reqwest::Client, url: &str, path: &Path) -> Result<std::path::PathBuf, String> {
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;
    if !url.starts_with("https://") {
        return Err("the download address isn't secure".into());
    }
    let resp = in_time("the download", async { http.get(url).send().await.and_then(|r| r.error_for_status()) }).await?;
    if let Some(p) = path.parent() {
        tokio::fs::create_dir_all(p).await.map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("part");
    let mut f = tokio::fs::File::create(&tmp).await.map_err(|e| e.to_string())?;
    let mut stream = resp.bytes_stream();
    let mut checked = std::time::Instant::now();
    let started = std::time::Instant::now();
    let mut got = 0u64;
    loop {
        let chunk = match tokio::time::timeout(STALL, stream.next()).await {
            Ok(Some(c)) => c.map_err(|e| e.to_string())?,
            Ok(None) => break,
            Err(_) => return Err(format!("the download stalled for {} seconds", STALL.as_secs())),
        };
        f.write_all(&chunk).await.map_err(|e| e.to_string())?;
        got += chunk.len() as u64;
        if serverlane::too_slow(got, started.elapsed()) {
            drop(f);
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(format!("the download ran slower than {} KB/s", serverlane::MIN_RATE / 1024));
        }
        if checked.elapsed() > Duration::from_secs(5) {
            checked = std::time::Instant::now();
            if game_running() {
                drop(f);
                let _ = tokio::fs::remove_file(&tmp).await;
                return Err(GAME_RUNNING.into());
            }
        }
    }
    f.flush().await.map_err(|e| e.to_string())?;
    drop(f);
    Ok(tmp)
}
