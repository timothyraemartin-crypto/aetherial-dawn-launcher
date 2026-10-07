//! What the launcher shows and does about mods: the server's mods.json,
//! the Requirements rows, the Vortex pairing and opening a mod's Nexus page.
//! Mods are installed through Vortex; the launcher no longer downloads them.

use std::path::{Path, PathBuf};

use launcher_core::{auth, modlist, modlist::ModEntry, nexus};
use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::{log, AppState, CmdResult};

#[derive(Default)]
pub struct ModsState {
    /// The server's mods.json, once fetched.
    server_list: tokio::sync::Mutex<Option<modlist::ModList>>,
    /// Exactly which mods.json that was (sha256 of the bytes, revision).
    receipt: tokio::sync::Mutex<Option<launcher_core::inventory::FeedReceipt>>,
}

fn key_path(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join("nexus.bin"))
}

pub(crate) fn nexus_key(app: &AppHandle) -> Option<String> {
    key_path(app).and_then(|p| auth::load_token(&p))
}

/// Where the nxm:// command that was there before is kept while the launcher
/// holds it, so it can be put back even after a crash.
fn previous_handler_path(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join("nxm-previous.txt"))
}

/// Gives nxm:// back to whoever had it, if a run was cut short.
pub fn restore_left_handler(app: &AppHandle) {
    let Some(p) = previous_handler_path(app) else { return };
    if let Ok(prev) = std::fs::read_to_string(&p) {
        let prev = prev.trim();
        let _ = nexus::restore_nxm_handler(if prev.is_empty() { None } else { Some(prev) });
        let _ = std::fs::remove_file(&p);
        log::line("mods: gave the nxm:// links back to the program that had them");
    }
}

/// A second launch (Windows starts the launcher again for an nxm:// link):
/// bring the running launcher forward.
pub fn on_second_launch(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// The server's mods.json as already fetched; never fetches.
pub async fn fetched_server_list(state: &AppState) -> Option<modlist::ModList> {
    state.mods.server_list.lock().await.clone()
}

pub async fn server_list(state: &AppState) -> Option<modlist::ModList> {
    if let Some(l) = state.mods.server_list.lock().await.clone() {
        return Some(l);
    }
    fetch_server_list(state).await
}

async fn fetch_server_list(state: &AppState) -> Option<modlist::ModList> {
    let base = state.config.lock().await.base_url.clone();
    let url = format!("{}/mods.json", base.trim_end_matches('/'));
    let bytes = match state.http.get(&url).timeout(std::time::Duration::from_secs(8)).send().await {
        Ok(r) if r.status().is_success() => r.bytes().await.ok(),
        _ => None,
    };
    let got = bytes.as_ref().and_then(|b| serde_json::from_slice::<modlist::ModList>(b).ok().map(|l| (l, b)));
    let (l, b) = got?;
    let receipt = launcher_core::inventory::FeedReceipt::of(b, &l);
    if state.mods.receipt.lock().await.as_ref() != Some(&receipt) {
        log::line(&format!("mods: fetched {url}: {}", receipt.describe()));
    }
    *state.mods.receipt.lock().await = Some(receipt);
    *state.mods.server_list.lock().await = Some(l.clone());
    Some(l)
}

/// Play fetches mods.json again, so a list changed while the launcher was
/// open is the one used. Returns false when the current feed cannot be
/// confirmed; Play then stops rather than trusting the old cache.
pub async fn refresh_server_list(state: &AppState) -> bool {
    let before = state.mods.server_list.lock().await.as_ref().map(|l| l.mods.len());
    let got = fetch_server_list(state).await;
    // Saved now, so Play's tidying (removed mods) goes by this list.
    if let (Some(l), Some(dir)) = (&got, state.config.lock().await.game_dir.clone()) {
        launcher_core::allowlist::save_server_list(&dir, l);
    }
    let current = got.is_some();
    match got {
        Some(l) if before.is_some_and(|b| b != l.mods.len()) => log::line(&format!("mods: the server's list changed while the launcher was open ({} entries, was {})", l.mods.len(), before.unwrap_or(0))),
        Some(_) => {}
        None => log::line("mods: couldn't fetch the server's list again; Play waits for a current list"),
    }
    current
}

/// The served client set (`aetherial-collection.json`, design 6.1), when the
/// server publishes one. During manual Vortex setup Play always checks the
/// current mods.json pins, with this optional record adding collection pinning.
pub async fn served_client_set(state: &AppState) -> Option<launcher_core::vortex::ClientSet> {
    let base = state.config.lock().await.base_url.clone();
    let url = format!("{}/aetherial-collection.json", base.trim_end_matches('/'));
    match state.http.get(&url).timeout(std::time::Duration::from_secs(8)).send().await {
        Ok(r) if r.status().is_success() => r.json().await.ok(),
        _ => None,
    }
}

/// Read the active profile from the signed, read-only Vortex extension.
pub async fn vortex_status(app: &AppHandle, state: &AppState) -> Option<launcher_core::vortex::Status> {
    use launcher_core::vortex;
    let home = vortex::home(&app.path().app_local_data_dir().ok()?);
    let token = vortex::token(&home).ok()?;
    match vortex::call(&state.http, &home, &token, "status", &serde_json::json!({}), "").await {
        Ok(v) => {
            let status = serde_json::from_value::<vortex::Status>(v).ok()?;
            if !vortex::loaded_is_current(&status, vortex::EXTENSION) {
                // Vortex is still running an older helper: nothing it says
                // counts until Vortex is restarted with this one.
                log::line(&format!("mods: Vortex runs helper {:?}, not {:?}; restart Vortex", status.extension_version, vortex::bundled_version(vortex::EXTENSION)));
                return None;
            }
            Some(status)
        }
        Err(e) => {
            if !matches!(e, vortex::JobError::NotRunning) {
                log::line(&format!("mods: the Vortex extension didn't answer: {e}"));
            }
            None
        }
    }
}

/// "Connect Vortex": keeps (or, with `fresh`, replaces) the pairing token
/// and puts the read-only Aetherial Dawn extension into Vortex's plugins
/// folder. Changes nothing else in Vortex. Says what the player does next.
#[tauri::command]
pub async fn vortex_connect(app: AppHandle, state: State<'_, AppState>, fresh: Option<bool>) -> CmdResult<String> {
    use launcher_core::vortex;
    let home = vortex::home(&app.path().app_local_data_dir().map_err(|e| e.to_string())?);
    let token = if fresh.unwrap_or(false) { vortex::rotate(&home) } else { vortex::pair(&home) }.map_err(|e| e.to_string())?;
    let roaming = app.path().data_dir().map_err(|e| e.to_string())?;
    let plugins = vortex::plugins_dir(&roaming);
    if vortex::needs_write(&plugins, vortex::EXTENSION) {
        // The helper's folder is swapped only while Vortex is closed, so
        // Vortex never holds it open or loads it mid-swap. If Windows can't
        // list processes, it is treated as running.
        match launcher_core::watch::find_process_checked("Vortex.exe") {
            Ok(None) => {}
            Ok(Some(_)) => return Err("Close Vortex first, then press Connect Vortex again. The helper is only put in while Vortex is closed.".into()),
            Err(e) => {
                log::line(&format!("vortex: couldn't check whether Vortex is running: {e}"));
                return Err("The launcher couldn't check whether Vortex is running. Close Vortex, then press Connect Vortex again.".into());
            }
        }
    }
    let done = vortex::install_extension(&plugins, vortex::EXTENSION).map_err(|e| e.to_string())?;
    log::line(&format!("vortex: extension {done:?}{}", if fresh.unwrap_or(false) { ", new pairing" } else { "" }));
    if vortex::extension_state(&plugins, vortex::EXTENSION) == vortex::ExtensionState::Mixed {
        log::line("vortex: extension folder is not one whole version; not pairing");
        return Err("The Aetherial Dawn helper in Vortex is incomplete. Close Vortex, then press Connect Vortex again.".into());
    }
    let answers = vortex::call(&state.http, &home, &token, "status", &serde_json::json!({}), "").await.ok()
        .and_then(|v| serde_json::from_value::<vortex::Status>(v).ok())
        .is_some_and(|s| vortex::loaded_is_current(&s, vortex::EXTENSION));
    Ok(match (&done, answers) {
        (_, true) if !done.needs_restart() => "Vortex is connected.".into(),
        (vortex::Installed::NewerKept { installed }, false) => format!("A newer Aetherial Dawn helper ({installed}) is already in Vortex. Start or restart Vortex and it connects."),
        (d, _) if d.needs_restart() => "The Aetherial Dawn helper is now in Vortex. Close Vortex and open it again once, then press Check again.".into(),
        _ => "The Aetherial Dawn helper is in Vortex. Start or restart Vortex, then press Check again.".into(),
    })
}

pub async fn full_list(state: &AppState) -> Vec<ModEntry> {
    let version = state.manifest.lock().await.as_ref().and_then(|m| m.game.as_ref()).and_then(|g| g.version.clone());
    let server = server_list(state).await;
    if let (Some(l), Some(dir)) = (&server, state.config.lock().await.game_dir.clone()) {
        launcher_core::allowlist::save_server_list(&dir, l);
    }
    modlist::play_required(modlist::merged(version.as_deref(), server.as_ref()))
}

#[derive(Serialize)]
pub struct Row {
    id: String,
    name: String,
    page: Option<String>,
    hint: Option<String>,
    looks_for: String,
    /// Its files are in the game folder (the game-files inventory).
    installed: bool,
    /// Vortex deployed it (Vortex's deployment record); None without one.
    in_vortex: Option<bool>,
    /// Vortex's three answers, each on its own; None is unknown.
    vortex_installed: Option<bool>,
    vortex_enabled: Option<bool>,
    vortex_deployed: Option<bool>,
    from: &'static str,
}

pub fn row(m: &ModEntry, game_dir: &Path) -> Row {
    let files = launcher_core::allowlist::vortex_files(game_dir);
    row_with(m, game_dir, launcher_core::inventory::standing(m, game_dir, &files, launcher_core::inventory::has_vortex_record(game_dir)))
}

fn row_with(m: &ModEntry, _game_dir: &Path, st: launcher_core::inventory::Standing) -> Row {
    Row {
        id: m.id.clone(),
        name: m.name.clone(),
        page: m.download_page(),
        hint: m.hint.clone(),
        looks_for: m.check.join(", "),
        installed: st.game_files,
        in_vortex: st.vortex_deployed,
        vortex_installed: None,
        vortex_enabled: None,
        vortex_deployed: None,
        from: if m.nexus.is_some() { "nexus" } else { "direct" },
    }
}

#[derive(Serialize)]
pub struct ModsView {
    mods: Vec<Row>,
    vortex: bool,
    /// Both inventories, each naming what it measures.
    counts: launcher_core::inventory::Counts,
    /// "Game files: 36 of 37 present · Vortex: 0 of 37 deployed · …"
    counts_text: String,
    /// Which mods.json the counts are for; None when the server's list
    /// couldn't be fetched (the launcher's own list only).
    feed: Option<String>,
    /// The Aetherial Dawn profile line from Vortex's own state, when the
    /// launcher is paired with the extension.
    vortex_line: Option<String>,
    vortex_ready: Option<bool>,
    /// The server has switched the Vortex gate on (vortexRequired). Off,
    /// PLAY needs only every listed mod present.
    vortex_required: bool,
    /// The launcher has a Vortex pairing token.
    vortex_paired: bool,
    /// Mods the launcher put in the game folder, by who else holds their
    /// files (read-only; Codex 5910357069).
    ownership_text: String,
}

/// Use the same profile, exact Vortex deployment source, and physical file
/// checks that Play requires. The per-mod answers also drive the Requirements
/// rows, so an old deployment record cannot make a wrong package look ready.
struct VortexReadout {
    line: String,
    ready: bool,
    exact: Vec<Option<bool>>,
    confirmed: usize,
    required: usize,
}

fn vortex_readout(
    set: &launcher_core::vortex::ClientSet,
    status: Option<&launcher_core::vortex::Status>,
    files: &[launcher_core::allowlist::VortexFile],
    dir: &Path,
    current_issue: Option<&str>,
) -> VortexReadout {
    use launcher_core::vortex;

    let required = set.mods.iter().filter(|m| vortex::vortex_ref(m).is_some()).count();
    if let Some(issue) = current_issue {
        return VortexReadout {
            line: issue.into(),
            ready: false,
            exact: vec![None; set.mods.len()],
            confirmed: 0,
            required,
        };
    }
    let exact: Vec<Option<bool>> = set.mods.iter().map(|m| {
        vortex::vortex_ref(m)?;
        let status = status?;
        Some(vortex::deployment_ready_for(&set.mods, m, status, files, dir)
            && (m.check.is_empty() || m.game_files_present(dir)))
    }).collect();
    let confirmed = exact.iter().filter(|v| **v == Some(true)).count();
    let step = vortex::step(set, status);
    let (line, ready) = if !step.ok() {
        (step.describe(), false)
    } else {
        let missing: Vec<&str> = set.mods.iter().zip(&exact)
            .filter_map(|(m, ok)| (ok == &Some(false)).then_some(m.name.as_str()))
            .collect();
        if missing.is_empty() {
            (format!("{} · deployment and game files confirmed", step.describe()), true)
        } else {
            (format!("Vortex: {} required mod{} need deployment or game files in Skyrim: {}. Deploy in Vortex, then Check again.",
                missing.len(), if missing.len() == 1 { "" } else { "s" }, missing.join(", ")), false)
        }
    };
    let line = if status.is_some_and(|s| vortex::female_face_alternative_installed(&set.mods, s)) {
        format!("{line} · Male Face Overlays selected; Female Face Overlays installed as an alternative")
    } else { line };
    let line = match status.and_then(|s| vortex::staging_on_other_drive(s, dir)) {
        Some(p) => format!("{line} · Vortex keeps its mods on another drive than Skyrim ({p}), so deploying can fail. In Vortex, Settings > Mods, move the Mod Staging Folder to Skyrim's drive"),
        None => line,
    };
    VortexReadout { line, ready, exact, confirmed, required }
}

#[tauri::command]
pub async fn mods_state(app: AppHandle, state: State<'_, AppState>) -> CmdResult<ModsView> {
    let dir = state.config.lock().await.game_dir.clone().ok_or("Pick your Skyrim folder first.")?;
    let base = state.config.lock().await.base_url.clone();
    let (feed_current, current_manifest) = tokio::join!(
        refresh_server_list(&state),
        launcher_core::manifest::Manifest::fetch(&state.http, &base),
    );
    let manifest_current = current_manifest.is_ok();
    let version = match current_manifest {
        Ok(m) => m.game.and_then(|g| g.version),
        Err(_) => state.manifest.lock().await.as_ref()
            .and_then(|m| m.game.as_ref()).and_then(|g| g.version.clone()),
    };
    let server = fetched_server_list(&state).await;
    let list = modlist::play_required(modlist::merged(version.as_deref(), server.as_ref()));
    let (mut st, mut counts) = launcher_core::inventory::count(&list, &dir);
    let feed = state.mods.receipt.lock().await.as_ref().map(|r| r.describe());
    // The served collection can add a collection pin, but its own mod array
    // never replaces the current merged mods.json list used by Play.
    let served = served_client_set(&state).await;
    let vortex_required = launcher_core::vortex::gate_on(served.as_ref());
    let collection = served.and_then(|s| s.collection);
    let set = launcher_core::vortex::ClientSet { collection, mods: list.clone(), vortex_required };
    let home = app.path().app_local_data_dir().ok().map(|d| launcher_core::vortex::home(&d));
    let paired = home.as_ref().is_some_and(|h| launcher_core::vortex::token(h).is_ok());
    let status = if paired { vortex_status(&app, &state).await } else { None };
    let deployed = launcher_core::allowlist::vortex_files(&dir);
    let current_issue = if !manifest_current {
        Some("Vortex: the server's current game requirements are unavailable. Try Check again when the server responds.")
    } else if !feed_current {
        Some("Vortex: the server's current mod list is unavailable. Try Check again when the server responds.")
    } else { None };
    let readout = vortex_readout(&set, status.as_ref(), &deployed, &dir, current_issue);
    // Each row's "present" answer matches what Play will check
    // (launcher_core::vortex::present_for_play). With the gate on, a
    // checkless Nexus mod is present once Vortex's exact deployment is.
    for ((m, standing), exact) in list.iter().zip(&mut st).zip(&readout.exact) {
        if !vortex_required {
            standing.game_files = launcher_core::vortex::present_for_play(m, &dir, false);
        } else if launcher_core::vortex::vortex_ref(m).is_some() {
            standing.game_files = if m.check.is_empty() { *exact == Some(true) } else { m.game_files_present(&dir) };
        }
    }
    counts.game_files_present = st.iter().filter(|s| s.game_files).count();
    let rows = list.iter().zip(st).zip(&readout.exact).map(|((m, s), exact)| {
        let mut row = row_with(m, &dir, s);
        row.in_vortex = *exact;
        // Unknown while the server's current lists can't be read, like the gate.
        if current_issue.is_none() {
            let v = launcher_core::vortex::package_states(&set.mods, m, status.as_ref(), &deployed, &dir);
            (row.vortex_installed, row.vortex_enabled, row.vortex_deployed) = (v.installed, v.enabled, v.deployed);
        }
        row
    }).collect();
    let counts_text = if current_issue.is_some() {
        format!("Vortex requirements: current server data unavailable · Game files: {} of {} present",
            counts.game_files_present, counts.listed)
    } else if status.is_none() {
        format!("Vortex profile: not checked yet · Game files: {} of {} present",
            counts.game_files_present, counts.listed)
    } else {
        format!("Vortex: {} of {} required Nexus mods confirmed · Game files: {} of {} present",
            readout.confirmed, readout.required, counts.game_files_present, counts.listed)
    };
    let ownership = launcher_core::inventory::ownership(&dir);
    let ownership_text = launcher_core::inventory::describe_ownership(&ownership, launcher_core::inventory::has_vortex_record(&dir));
    static LOGGED: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());
    if let Ok(mut last) = LOGGED.lock() {
        if *last != ownership_text {
            log::line(&format!("mods: {ownership_text}"));
            for o in ownership.iter().filter(|o| o.both > 0) {
                log::line(&format!("mods: {} ({}): {} files also deployed by Vortex from {}", o.name, o.id, o.both, o.vortex_sources.join(", ")));
            }
            *last = ownership_text.clone();
        }
    }
    Ok(ModsView {
        mods: rows,
        vortex: modlist::vortex_manages(&dir),
        counts_text,
        counts,
        feed,
        vortex_line: paired.then_some(readout.line),
        vortex_ready: paired.then_some(readout.ready),
        vortex_required,
        vortex_paired: paired,
        ownership_text,
    })
}

fn open_url(app: &AppHandle, url: &str) -> CmdResult<()> {
    use tauri_plugin_opener::OpenerExt;
    app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
}

/// Opens a Nexus Mods page for a required mod (only nexusmods.com pages).
#[tauri::command]
pub fn open_mod_page(app: AppHandle, url: String) -> CmdResult<()> {
    if !url.starts_with("https://www.nexusmods.com/") {
        return Err("That isn't a Nexus Mods page.".into());
    }
    open_url(&app, &url)
}

#[cfg(test)]
mod tests {
    #[test]
    fn requirements_readout_needs_current_feed_exact_deployment_and_game_files() {
        use launcher_core::allowlist::VortexFile;
        use launcher_core::modlist::{ModEntry, NexusRef};
        use launcher_core::vortex::{ClientSet, Profile, Status, VortexMod};

        let dir = std::env::temp_dir().join(format!("ad-mods-readout-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(dir.join("Data")).unwrap();
        let set = ClientSet { vortex_required: false, collection: None, mods: vec![ModEntry {
            id: "test-mod".into(), name: "Test Mod".into(),
            nexus: Some(NexusRef { mod_id: 42, file: Some(73), pick: None }),
            check: vec!["Data/Test.txt".into(), "test-loader.exe".into()],
            ..Default::default()
        }] };
        let status = Status {
            profile: Some(Profile { id: "p1".into(), name: "Aetherial Dawn".into(), active: true }),
            aetherial_profiles: 1,
            mods: vec![VortexMod {
                id: "test-package".into(), installation_path: Some("test-package-folder".into()),
                state: Some("installed".into()), nexus_mod_id: Some(42),
                nexus_file_id: Some(73), enabled: true,
            }],
            ..Default::default()
        };
        let files = vec![VortexFile { rel: "Data/Test.txt".into(), source: "test-package-folder".into() }];
        std::fs::write(dir.join("Data/Test.txt"), b"deployed").unwrap();

        let unavailable = super::vortex_readout(&set, Some(&status), &files, &dir,
            Some("Vortex: current game requirements unavailable"));
        assert!(!unavailable.ready);
        assert_eq!(unavailable.exact, vec![None]);

        let missing_game_file = super::vortex_readout(&set, Some(&status), &files, &dir, None);
        assert!(!missing_game_file.ready);
        assert_eq!(missing_game_file.exact, vec![Some(false)]);
        assert!(missing_game_file.line.contains("1 required mod"));
        assert!(missing_game_file.line.contains("Test Mod"));

        std::fs::write(dir.join("test-loader.exe"), b"loader").unwrap();
        let wrong_source = [VortexFile { rel: "Data/Test.txt".into(), source: "old-package-folder".into() }];
        assert!(!super::vortex_readout(&set, Some(&status), &wrong_source, &dir, None).ready);

        let ready = super::vortex_readout(&set, Some(&status), &files, &dir, None);
        assert!(ready.ready, "{} {:?}", ready.line, ready.exact);
        assert_eq!(ready.exact, vec![Some(true)]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn requirements_names_installed_female_alternative_without_counting_it_as_deployed() {
        use launcher_core::allowlist::VortexFile;
        use launcher_core::modlist::{self, ModEntry, NexusRef};
        use launcher_core::vortex::{ClientSet, Profile, Status, VortexMod};
        let dir = std::env::temp_dir().join(format!("ad-face-readout-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let face = |id: &str, file| ModEntry { id: id.into(), name: id.into(),
            nexus: Some(NexusRef { mod_id: 22487, file: Some(file), pick: None }), ..Default::default() };
        let required = modlist::play_required(vec![face("community-overlays-1-female-face", 104828),
            face("community-overlays-1-male-face", 104868)]);
        let set = ClientSet { vortex_required: false, collection: None, mods: required };
        let checks = &set.mods[0].check;
        assert_eq!(checks.len(), 25);
        for rel in checks {
            let target = dir.join(rel);
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::write(target, b"selected male texture").unwrap();
        }
        let package = |id: &str, file| VortexMod { id: id.into(), installation_path: Some(format!("{id}-folder")),
            state: Some("installed".into()), nexus_mod_id: Some(22487), nexus_file_id: Some(file), enabled: true };
        let status = Status { profile: Some(Profile { id: "p1".into(), name: "Aetherial Dawn".into(), active: true }),
            aetherial_profiles: 1, mods: vec![package("female", 104828), package("male", 104868)], ..Default::default() };
        let files: Vec<VortexFile> = checks.iter().map(|rel| VortexFile { rel: rel.clone(), source: "male-folder".into() }).collect();
        let readout = super::vortex_readout(&set, Some(&status), &files, &dir, None);
        assert!(readout.ready, "{}", readout.line);
        assert_eq!((readout.confirmed, readout.required), (1, 1));
        assert!(readout.line.contains("Female Face Overlays installed as an alternative"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
