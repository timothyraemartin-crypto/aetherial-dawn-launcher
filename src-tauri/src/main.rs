// Hides the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use launcher_core::{game, manifest::Manifest, settings, sync, Error};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::Mutex;

/// Where the server publishes the launcher files. Set AD_BASE_URL when
/// building to point a release at the real server.
const DEFAULT_BASE_URL: &str = match option_env!("AD_BASE_URL") {
    Some(u) => u,
    None => "https://vps-d38c928e.vps.ovh.us/launcher",
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Config {
    game_dir: Option<PathBuf>,
    base_url: String,
    /// SkyMP offline-mode profile. A placeholder until Discord sign-in exists:
    /// the server trusts whatever number the client sends.
    profile_id: i64,
    close_on_launch: bool,
    background_updates: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            game_dir: None,
            base_url: DEFAULT_BASE_URL.into(),
            profile_id: rand::random_range(1..i32::MAX as i64),
            close_on_launch: true,
            background_updates: true,
        }
    }
}

struct AppState {
    config: Mutex<Config>,
    manifest: Mutex<Option<Manifest>>,
    http: reqwest::Client,
}

type CmdResult<T> = Result<T, String>;

fn err(e: Error) -> String {
    e.to_string()
}

fn config_path(app: &AppHandle) -> CmdResult<PathBuf> {
    app.path().app_config_dir().map(|d| d.join("config.json")).map_err(|e| e.to_string())
}

fn load_config(app: &AppHandle) -> Config {
    config_path(app)
        .ok()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice::<Config>(&b).ok())
        .map(|mut c| {
            // The server address is baked into each build, so an older saved
            // config (or one from a test build) never points somewhere stale.
            c.base_url = DEFAULT_BASE_URL.into();
            c
        })
        .unwrap_or_default()
}

fn save_config(app: &AppHandle, c: &Config) -> CmdResult<()> {
    let path = config_path(app)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, serde_json::to_vec_pretty(c).unwrap()).map_err(|e| e.to_string())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    config: Config,
    game: Option<game::GameInfo>,
    game_error: Option<String>,
    launcher_version: String,
}

/// Everything the UI needs on startup. Finds the game if no folder is saved.
#[tauri::command]
async fn get_state(app: AppHandle, state: State<'_, AppState>) -> CmdResult<Snapshot> {
    let mut config = state.config.lock().await;
    if config.game_dir.is_none() {
        if let Some(found) = game::detect() {
            config.game_dir = Some(found.dir);
            save_config(&app, &config)?;
        }
    }
    let (game, game_error) = match &config.game_dir {
        Some(dir) => match game::inspect(dir) {
            Ok(g) => (Some(g), None),
            Err(e) => (None, Some(e.to_string())),
        },
        None => (None, Some("Skyrim Special Edition wasn't found. Pick its folder.".into())),
    };
    Ok(Snapshot { config: config.clone(), game, game_error, launcher_version: app.package_info().version.to_string() })
}

#[tauri::command]
async fn set_game_dir(app: AppHandle, state: State<'_, AppState>, dir: PathBuf) -> CmdResult<game::GameInfo> {
    let info = game::inspect(&dir).map_err(err)?;
    let mut config = state.config.lock().await;
    config.game_dir = Some(info.dir.clone());
    save_config(&app, &config)?;
    Ok(info)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Prefs {
    close_on_launch: bool,
    background_updates: bool,
}

#[tauri::command]
async fn set_prefs(app: AppHandle, state: State<'_, AppState>, prefs: Prefs) -> CmdResult<()> {
    let mut config = state.config.lock().await;
    config.close_on_launch = prefs.close_on_launch;
    config.background_updates = prefs.background_updates;
    save_config(&app, &config)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CheckResult {
    build: String,
    server: launcher_core::manifest::Server,
    files: usize,
    remove: usize,
    bytes: u64,
}

async fn game_dir(state: &AppState) -> CmdResult<PathBuf> {
    state.config.lock().await.game_dir.clone().ok_or_else(|| "Pick your Skyrim folder first.".to_string())
}

/// Downloads the server's file list and works out what needs updating.
#[tauri::command]
async fn check(state: State<'_, AppState>, verify_all: bool) -> CmdResult<CheckResult> {
    let dir = game_dir(&state).await?;
    let base = state.config.lock().await.base_url.clone();
    let m = Manifest::fetch(&state.http, &base).await.map_err(err)?;
    let plan = sync::plan(&dir, &m, verify_all).await.map_err(err)?;
    let result = CheckResult {
        build: m.build.clone(),
        server: m.server.clone(),
        files: plan.download.len(),
        remove: plan.remove.len(),
        bytes: plan.download_bytes,
    };
    *state.manifest.lock().await = Some(m);
    Ok(result)
}

/// Brings the game folder in line with the file list, sending `sync-progress`
/// events to the UI as it goes.
#[tauri::command]
async fn update(app: AppHandle, state: State<'_, AppState>, verify_all: bool) -> CmdResult<String> {
    let dir = game_dir(&state).await?;
    let base = state.config.lock().await.base_url.clone();
    let m = match state.manifest.lock().await.clone() {
        Some(m) => m,
        None => Manifest::fetch(&state.http, &base).await.map_err(err)?,
    };
    let plan = sync::plan(&dir, &m, verify_all).await.map_err(err)?;
    sync::apply(&state.http, &base, &dir, &plan, |p| {
        let _ = app.emit("sync-progress", p);
    })
    .await
    .map_err(err)?;
    Ok(m.build)
}

/// Writes the SkyMP client settings and starts Skyrim through SKSE.
#[tauri::command]
async fn play(app: AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    let config = state.config.lock().await.clone();
    let dir = config.game_dir.clone().ok_or("Pick your Skyrim folder first.")?;
    let m = state.manifest.lock().await.clone().ok_or("Check for updates before playing.")?;
    settings::write(
        &dir,
        &settings::ClientSettings {
            server_ip: &m.server.ip,
            server_port: m.server.port,
            master: &m.master,
            profile_id: config.profile_id,
        },
    )
    .map_err(err)?;
    game::launch(&dir).map_err(err)?;
    if config.close_on_launch {
        app.exit(0);
    }
    Ok(())
}

/// The client files in the current server build, for the Game files page.
#[tauri::command]
async fn files(state: State<'_, AppState>) -> CmdResult<Vec<launcher_core::manifest::FileEntry>> {
    let m = state.manifest.lock().await;
    m.as_ref().map(|m| m.files.clone()).ok_or_else(|| "The file list hasn't loaded yet.".to_string())
}

#[tauri::command]
async fn open_game_folder(state: State<'_, AppState>) -> CmdResult<()> {
    let dir = game_dir(&state).await?;
    let opener = if cfg!(windows) { "explorer" } else { "xdg-open" };
    std::process::Command::new(opener).arg(dir).spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// Optional `status.json` for the side panel. Missing or broken is fine.
#[tauri::command]
async fn server_status(state: State<'_, AppState>) -> CmdResult<Option<serde_json::Value>> {
    let base = state.config.lock().await.base_url.clone();
    let url = format!("{}/status.json", base.trim_end_matches('/'));
    let Ok(resp) = state.http.get(url).send().await.and_then(|r| r.error_for_status()) else {
        return Ok(None);
    };
    Ok(resp.json().await.ok())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            let config = load_config(app.handle());
            save_config(app.handle(), &config)?;
            let http = reqwest::Client::builder()
                .user_agent(concat!("AetherialDawnLauncher/", env!("CARGO_PKG_VERSION")))
                .connect_timeout(std::time::Duration::from_secs(10))
                .build()?;
            app.manage(AppState { config: Mutex::new(config), manifest: Mutex::new(None), http });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![get_state, set_game_dir, set_prefs, check, update, play, files, open_game_folder, server_status])
        .run(tauri::generate_context!())
        .expect("error while running the launcher");
}
