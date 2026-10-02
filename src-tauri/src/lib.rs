mod config;
mod git;
mod pipeline;
mod proc;
mod ssh;
mod watcher;

use config::Config;
use pipeline::{DeployOptions, Event, Outcome};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::Duration;
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};
use tauri_plugin_notification::NotificationExt;
use watcher::ProfileStatus;

#[derive(Default)]
struct AppState {
    config: Mutex<Config>,
    statuses: Mutex<HashMap<String, ProfileStatus>>,
    busy: Mutex<Option<String>>,
    last_failed: Mutex<bool>,
    /// GitHub commits we already told the user about.
    notified: Mutex<HashSet<String>>,
    tray: Mutex<Option<TrayIcon>>,
}

const ICON_IDLE: &[u8] = include_bytes!("../icons/tray-idle.png");
const ICON_BUSY: &[u8] = include_bytes!("../icons/tray-busy.png");
const ICON_ERROR: &[u8] = include_bytes!("../icons/tray-error.png");
const ICON_UPDATE: &[u8] = include_bytes!("../icons/tray-update.png");

fn refresh_tray(app: &AppHandle) {
    let state = app.state::<AppState>();
    let busy = state.busy.lock().unwrap().clone();
    let failed = *state.last_failed.lock().unwrap();
    let statuses = state.statuses.lock().unwrap();
    let pending = statuses.values().any(|s| s.incoming > 0);
    let out_of_sync = statuses.values().any(|s| {
        s.local.as_ref().is_some_and(|l| !l.head.is_empty() && !s.production.is_empty() && l.head != s.production)
    });
    let (bytes, tip) = if let Some(p) = &busy {
        (ICON_BUSY, format!("Poymai Deploy — деплой {p}…"))
    } else if failed {
        (ICON_ERROR, "Poymai Deploy — последний деплой не удался".to_string())
    } else if pending {
        (ICON_UPDATE, "Poymai Deploy — есть обновления с другого компьютера".to_string())
    } else if out_of_sync {
        (ICON_UPDATE, "Poymai Deploy — есть незадеплоенные изменения".to_string())
    } else {
        (ICON_IDLE, "Poymai Deploy — всё синхронизировано".to_string())
    };
    drop(statuses);
    let tray_guard = state.tray.lock().unwrap();
    if let Some(tray) = tray_guard.as_ref() {
        if let Ok(img) = Image::from_bytes(bytes) {
            let _ = tray.set_icon(Some(img));
            #[cfg(target_os = "macos")]
            let _ = tray.set_icon_as_template(bytes == ICON_IDLE);
        }
        let _ = tray.set_tooltip(Some(&tip));
    }
}

fn notify(app: &AppHandle, title: &str, body: &str) {
    let _ = app.notification().builder().title(title).body(body).show();
}

fn show_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

// ── Commands ─────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct Snapshot {
    config: Config,
    statuses: Vec<ProfileStatus>,
    busy: Option<String>,
    machine: String,
    steps: Vec<(String, u8, String)>,
}

#[tauri::command]
fn snapshot(state: State<AppState>) -> Snapshot {
    let config = state.config.lock().unwrap().clone();
    let statuses = state.statuses.lock().unwrap();
    Snapshot {
        statuses: config.profiles.iter().filter_map(|p| statuses.get(&p.id).cloned()).collect(),
        config,
        busy: state.busy.lock().unwrap().clone(),
        machine: pipeline::machine_name(),
        steps: pipeline::STEPS.iter().map(|(a, b, c)| (a.to_string(), *b, c.to_string())).collect(),
    }
}

#[tauri::command]
async fn refresh(app: AppHandle, profile: Option<String>) -> Result<(), String> {
    poll_once(&app, profile.as_deref()).await;
    Ok(())
}

#[tauri::command]
async fn server_status(state: State<'_, AppState>, profile: String) -> Result<serde_json::Value, String> {
    let cfg = state.config.lock().unwrap().clone();
    let p = cfg.profile(&profile).map_err(|e| e.to_string())?;
    watcher::server_status(&cfg, &p).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn releases(state: State<'_, AppState>, profile: String) -> Result<Vec<serde_json::Value>, String> {
    let cfg = state.config.lock().unwrap().clone();
    let p = cfg.profile(&profile).map_err(|e| e.to_string())?;
    watcher::releases(&cfg, &p).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn pull_updates(app: AppHandle, profile: String) -> Result<String, String> {
    let cfg = app.state::<AppState>().config.lock().unwrap().clone();
    let p = cfg.profile(&profile).map_err(|e| e.to_string())?;
    let text = watcher::pull_with_local_changes(&p).await.map_err(|e| e.to_string())?;
    poll_once(&app, Some(&profile)).await;
    Ok(text)
}

#[tauri::command]
async fn deploy(app: AppHandle, profile: String, options: DeployOptions) -> Result<Outcome, String> {
    let state = app.state::<AppState>();
    {
        let mut busy = state.busy.lock().unwrap();
        if let Some(other) = busy.as_ref() {
            return Err(format!("Уже идёт деплой: {other}"));
        }
        *busy = Some(profile.clone());
    }
    refresh_tray(&app);
    let cfg = state.config.lock().unwrap().clone();
    let result = match cfg.profile(&profile) {
        Ok(p) => {
            let emitter = app.clone();
            let emit = move |e: Event| {
                let _ = emitter.emit("deploy-event", e);
            };
            pipeline::deploy(&cfg, &p, options, &emit).await
        }
        Err(e) => Err(e),
    };
    *state.busy.lock().unwrap() = None;
    let ok = matches!(&result, Ok(o) if o.ok);
    *state.last_failed.lock().unwrap() = !ok;
    let name = cfg.profile(&profile).map(|p| p.name).unwrap_or(profile.clone());
    match &result {
        Ok(o) if o.ok => notify(&app, &format!("{name}: деплой готов"), &o.text),
        Ok(o) => notify(&app, &format!("{name}: деплой не удался"), &o.text),
        Err(e) => notify(&app, &format!("{name}: деплой не удался"), &e.to_string()),
    }
    poll_once(&app, Some(&profile)).await;
    result.map_err(|e| e.to_string())
}

#[tauri::command]
fn save_config(app: AppHandle, config: Config) -> Result<(), String> {
    config::save(&config).map_err(|e| e.to_string())?;
    *app.state::<AppState>().config.lock().unwrap() = config;
    Ok(())
}

#[tauri::command]
fn dismiss_error(app: AppHandle) {
    *app.state::<AppState>().last_failed.lock().unwrap() = false;
    refresh_tray(&app);
}

#[tauri::command]
fn open_path(path: String) -> Result<(), String> {
    tauri_plugin_opener::open_path(path, None::<&str>).map_err(|e| e.to_string())
}

// ── Background sync ──────────────────────────────────────────────────────────

async fn poll_once(app: &AppHandle, only: Option<&str>) {
    let state = app.state::<AppState>();
    let cfg = state.config.lock().unwrap().clone();
    for p in cfg.profiles.iter().filter(|p| only.is_none_or(|id| id == p.id)) {
        let mut st = watcher::status(p).await;
        let busy_here = state.busy.lock().unwrap().as_deref() == Some(p.id.as_str());
        if !busy_here && st.incoming > 0 {
            let key = format!("{}:{}", p.id, st.github);
            let fresh = state.notified.lock().unwrap().insert(key);
            if let Some(notice) = watcher::auto_pull(p, &st).await {
                let pulled = notice.title.ends_with("обновлено");
                if pulled || fresh {
                    notify(app, &notice.title, &notice.body);
                }
                if pulled {
                    st = watcher::status(p).await;
                }
            }
        }
        state.statuses.lock().unwrap().insert(p.id.clone(), st);
    }
    refresh_tray(app);
    let _ = app.emit("status-changed", ());
}

fn start_watcher(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            poll_once(&app, None).await;
            let secs = app.state::<AppState>().config.lock().unwrap().poll_seconds.max(15);
            tokio::time::sleep(Duration::from_secs(secs)).await;
        }
    });
}

fn build_tray(app: &AppHandle) -> tauri::Result<TrayIcon> {
    let cfg = app.state::<AppState>().config.lock().unwrap().clone();
    let open = MenuItem::with_id(app, "open", "Открыть Poymai Deploy", true, None::<&str>)?;
    let mut items: Vec<MenuItem<tauri::Wry>> = vec![];
    for p in &cfg.profiles {
        items.push(MenuItem::with_id(app, format!("deploy:{}", p.id), format!("Деплой {}", p.name), true, None::<&str>)?);
    }
    let sync = MenuItem::with_id(app, "sync", "Проверить обновления", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Выход", true, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let mut refs: Vec<&dyn tauri::menu::IsMenuItem<tauri::Wry>> = vec![&open, &sep1];
    for i in &items {
        refs.push(i);
    }
    refs.extend([&sync as &dyn tauri::menu::IsMenuItem<tauri::Wry>, &sep2, &quit]);
    let menu = Menu::with_items(app, &refs)?;

    TrayIconBuilder::with_id("main")
        .icon(Image::from_bytes(ICON_IDLE)?)
        .icon_as_template(cfg!(target_os = "macos"))
        .tooltip("Poymai Deploy")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            let id = event.id.as_ref().to_string();
            match id.as_str() {
                "open" => show_window(app),
                "quit" => app.exit(0),
                "sync" => {
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move { poll_once(&app, None).await });
                }
                _ => {
                    if let Some(profile) = id.strip_prefix("deploy:") {
                        show_window(app);
                        let _ = app.emit("tray-deploy", profile.to_string());
                    }
                }
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                let app = tray.app_handle();
                match app.get_webview_window("main") {
                    Some(w) if w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false) => {
                        let _ = w.hide();
                    }
                    _ => show_window(app),
                }
            }
        })
        .build(app)
}

pub fn run_app() {
    proc::fix_path();
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_window(app)))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--hidden"]),
        ))
        .manage(AppState { config: Mutex::new(config::load()), ..Default::default() })
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            let tray = build_tray(app.handle())?;
            *app.state::<AppState>().tray.lock().unwrap() = Some(tray);
            {
                use tauri_plugin_autostart::ManagerExt;
                let _ = app.autolaunch().enable();
            }
            if !std::env::args().any(|a| a == "--hidden") {
                show_window(app.handle());
            }
            start_watcher(app.handle().clone());
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            snapshot, refresh, server_status, releases, pull_updates, deploy, save_config, dismiss_error, open_path
        ])
        .run(tauri::generate_context!())
        .expect("error while running Poymai Deploy");
}

/// Headless mode: `poymai-deploy deploy <profile> [-m "message"] [--dry-run] [--sha <sha>]`.
pub fn run_cli(args: &[String]) -> i32 {
    proc::fix_path();
    let profile_id = args.get(2).cloned().unwrap_or_else(|| "main".into());
    let mut opts = DeployOptions::default();
    let mut it = args.iter().skip(3);
    while let Some(a) = it.next() {
        match a.as_str() {
            "-m" | "--message" => opts.message = it.next().cloned(),
            "--sha" => opts.sha = it.next().cloned(),
            "--dry-run" => opts.dry_run = true,
            "--accept-drift" => opts.accept_drift = true,
            "--backup-db" => opts.backup_db = true,
            _ => {}
        }
    }
    let cfg = config::load();
    let rt = tokio::runtime::Runtime::new().expect("tokio");
    rt.block_on(async {
        let profile = match cfg.profile(&profile_id) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("{e}");
                return 2;
            }
        };
        let emit = |e: Event| match e.kind.as_str() {
            "step" => println!("[{:>3}%] ==> {}", e.progress, e.text),
            "warn" => println!("  ! {}", e.text),
            "error" => println!("  ✗ {}", e.text),
            "done" => println!("  ✓ {}", e.text),
            _ => println!("    {}", e.text),
        };
        match pipeline::deploy(&cfg, &profile, opts, &emit).await {
            Ok(o) if o.ok => 0,
            Ok(o) => o.code.max(1),
            Err(_) => 1,
        }
    })
}
