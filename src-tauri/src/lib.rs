mod changes;
mod config;
mod git;
mod pipeline;
mod proc;
mod schedule;
mod site;
mod ssh;
mod watcher;

use config::Config;
use pipeline::{DeployOptions, Event, Outcome};
use schedule::{Action, Job};
use serde::{Deserialize, Serialize};
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
    jobs: Mutex<Vec<Job>>,
    /// Last answer of each site about a deploy window.
    site: Mutex<HashMap<String, SiteState>>,
}

#[derive(Clone, Debug, Default, Serialize)]
struct SiteState {
    data: Option<serde_json::Value>,
    error: String,
    /// RFC 3339 time of the last successful or failed check.
    checked_at: String,
    #[serde(skip)]
    checked: Option<std::time::Instant>,
}

impl SiteState {
    fn level(&self) -> Option<&str> {
        self.data.as_ref().map(site::verdict_level).filter(|l| !l.is_empty())
    }
    fn fresh_within(&self, secs: u64) -> bool {
        self.checked.is_some_and(|t| t.elapsed() < Duration::from_secs(secs))
    }
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
    // Same rule as the project dot in the window: anything to commit, push or deploy.
    let out_of_sync = statuses.values().any(|s| {
        s.local.as_ref().is_some_and(|l| !l.changes.is_empty() || l.ahead > 0)
            || (!s.github.is_empty() && !s.production.is_empty() && s.github != s.production)
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
    let mut tip = tip;
    if let Some(level) = state.site.lock().unwrap().get("main").and_then(|s| s.level().map(String::from)) {
        tip.push_str(match level.as_str() {
            "ok" => "
Сайт: можно деплоить",
            "warn" => "
Сайт: можно, но кто-то заметит",
            _ => "
Сайт: лучше подождать",
        });
    }
    if let Some(job) = state.jobs.lock().unwrap().iter().filter(|j| j.is_active()).min_by(|a, b| a.run_at.cmp(&b.run_at)) {
        tip.push_str(&format!("
Запланирован деплой: {}", job_when(job)));
    }
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
    jobs: Vec<Job>,
    site: HashMap<String, SiteState>,
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
        jobs: state.jobs.lock().unwrap().clone(),
        site: state.site.lock().unwrap().clone(),
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
async fn pending(app: AppHandle, profile: String) -> Result<changes::Pending, String> {
    let state = app.state::<AppState>();
    let cfg = state.config.lock().unwrap().clone();
    let p = cfg.profile(&profile).map_err(|e| e.to_string())?;
    let production = state.statuses.lock().unwrap().get(&profile).map(|s| s.production.clone()).unwrap_or_default();
    changes::pending(&p, &production).await.map_err(|e| e.to_string())
}

/// The site's deploy window; `fresh` forces a new SSH probe instead of the cached answer.
#[tauri::command]
async fn site_window(app: AppHandle, profile: String, fresh: bool) -> Result<SiteState, String> {
    if !fresh {
        if let Some(s) = app.state::<AppState>().site.lock().unwrap().get(&profile) {
            if s.fresh_within(60) {
                return Ok(s.clone());
            }
        }
    }
    Ok(refresh_site(&app, &profile).await)
}

#[tauri::command]
async fn announce(app: AppHandle, profile: String, minutes: u32, text: String) -> Result<(), String> {
    let cfg = app.state::<AppState>().config.lock().unwrap().clone();
    let p = cfg.profile(&profile).map_err(|e| e.to_string())?;
    let result = if minutes == 0 && text.is_empty() {
        site::clear_notice(&cfg, &p).await
    } else {
        site::announce(&cfg, &p, minutes, &text).await
    };
    result.map_err(|e| e.to_string())?;
    refresh_site(&app, &profile).await;
    Ok(())
}

#[tauri::command]
async fn deploy(app: AppHandle, profile: String, options: DeployOptions) -> Result<Outcome, String> {
    run_deploy(&app, &profile, options).await
}

#[derive(Debug, Deserialize)]
struct Plan {
    /// "at" | "quiet"
    mode: String,
    run_at: Option<String>,
    deadline: Option<String>,
    #[serde(default)]
    allow_warn: bool,
    #[serde(default)]
    force_at_deadline: bool,
    announce_minutes: Option<u32>,
    #[serde(default)]
    options: DeployOptions,
}

/// Commits and pushes local work now, then queues the deploy of exactly that commit.
#[tauri::command]
async fn schedule_deploy(app: AppHandle, profile: String, plan: Plan) -> Result<Job, String> {
    let state = app.state::<AppState>();
    let cfg = state.config.lock().unwrap().clone();
    let p = cfg.profile(&profile).map_err(|e| e.to_string())?;
    if plan.mode == "at" && schedule::parse(&plan.run_at).is_none() {
        return Err("Укажите время деплоя".into());
    }
    if plan.mode == "quiet" && !site::enabled(&p) {
        return Err(format!("Для «{}» нет связи с сайтом — выберите деплой ко времени", p.name));
    }
    {
        let mut busy = state.busy.lock().unwrap();
        if let Some(other) = busy.as_ref() {
            return Err(format!("Сейчас идёт деплой: {other}"));
        }
        *busy = Some(profile.clone());
    }
    let emitter = app.clone();
    let emit = move |e: Event| {
        let _ = emitter.emit("publish-event", e);
    };
    let published = pipeline::publish_only(&p, plan.options.clone(), &emit).await;
    *state.busy.lock().unwrap() = None;
    let sha = published.map_err(|e| e.to_string())?;
    let message = git::git(std::path::Path::new(&p.repo_path), &["log", "-1", "--format=%s", &sha]).await.unwrap_or_default();
    let now = chrono::Utc::now();
    let job = Job {
        id: format!("{}", now.timestamp_millis()),
        profile: profile.clone(),
        mode: plan.mode,
        run_at: plan.run_at,
        deadline: plan.deadline,
        allow_warn: plan.allow_warn,
        force_at_deadline: plan.force_at_deadline,
        announce_minutes: if site::enabled(&p) { plan.announce_minutes.unwrap_or(cfg.announce_minutes) } else { 0 },
        sha,
        message,
        backup_db: plan.options.backup_db,
        created_at: now.to_rfc3339(),
        created_on: pipeline::machine_name(),
        status: "waiting".into(),
        ..Default::default()
    };
    {
        let mut jobs = state.jobs.lock().unwrap();
        jobs.push(job.clone());
        let _ = schedule::save(&jobs);
    }
    notify(&app, &format!("{}: деплой запланирован", p.name), &format!("{} — {}", job_when(&job), job.message));
    poll_once(&app, Some(&profile)).await;
    Ok(job)
}

#[tauri::command]
async fn cancel_job(app: AppHandle, id: String) -> Result<(), String> {
    let job = update_job(&app, &id, |j| {
        if j.status == "running" {
            return;
        }
        j.status = "cancelled".into();
        j.result = "Отменено".into();
        j.finished_at = Some(chrono::Utc::now().to_rfc3339());
    })
    .ok_or("Задание не найдено")?;
    if job.announced_for.is_some() {
        clear_job_notice(&app, &job).await;
    }
    refresh_tray(&app);
    let _ = app.emit("status-changed", ());
    Ok(())
}

#[tauri::command]
async fn run_job_now(app: AppHandle, id: String) -> Result<(), String> {
    let job = app.state::<AppState>().jobs.lock().unwrap().iter().find(|j| j.id == id && j.is_active() && j.status != "running").cloned()
        .ok_or("Задание уже не активно")?;
    execute_job(&app, job).await;
    Ok(())
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

#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    if !url.starts_with("https://") {
        return Err("Можно открыть только https-ссылку".into());
    }
    tauri_plugin_opener::open_url(url, None::<&str>).map_err(|e| e.to_string())
}

// ── Deploy runs (manual and scheduled) ───────────────────────────────────────

async fn run_deploy(app: &AppHandle, profile: &str, options: DeployOptions) -> Result<Outcome, String> {
    let state = app.state::<AppState>();
    {
        let mut busy = state.busy.lock().unwrap();
        if let Some(other) = busy.as_ref() {
            return Err(format!("Уже идёт деплой: {other}"));
        }
        *busy = Some(profile.to_string());
    }
    refresh_tray(app);
    let _ = app.emit("status-changed", ());
    let cfg = state.config.lock().unwrap().clone();
    let result = match cfg.profile(profile) {
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
    let name = cfg.profile(profile).map(|p| p.name).unwrap_or(profile.to_string());
    match &result {
        Ok(o) if o.ok => notify(app, &format!("{name}: деплой готов"), &o.text),
        Ok(o) => notify(app, &format!("{name}: деплой не удался"), &o.text),
        Err(e) => notify(app, &format!("{name}: деплой не удался"), &e.to_string()),
    }
    poll_once(app, Some(profile)).await;
    if ok {
        // The restarted site has new numbers (uptime, last deploy).
        let _ = refresh_site(app, profile).await;
    }
    result.map_err(|e| e.to_string())
}

fn update_job(app: &AppHandle, id: &str, change: impl FnOnce(&mut Job)) -> Option<Job> {
    let state = app.state::<AppState>();
    let mut jobs = state.jobs.lock().unwrap();
    let job = jobs.iter_mut().find(|j| j.id == id)?;
    change(job);
    let copy = job.clone();
    let _ = schedule::save(&jobs);
    Some(copy)
}

fn job_when(job: &Job) -> String {
    let local = |ts: &Option<String>| {
        schedule::parse(ts).map(|d| d.with_timezone(&chrono::Local).format("%d.%m %H:%M").to_string())
    };
    if let Some(at) = local(&job.announced_for) {
        return format!("в {at}");
    }
    match job.mode.as_str() {
        "at" => local(&job.run_at).map(|t| format!("в {t}")).unwrap_or_default(),
        _ => match local(&job.deadline) {
            Some(d) => format!("в тихое окно до {d}"),
            None => "в ближайшее тихое окно".into(),
        },
    }
}

async fn clear_job_notice(app: &AppHandle, job: &Job) {
    let cfg = app.state::<AppState>().config.lock().unwrap().clone();
    if let Ok(p) = cfg.profile(&job.profile) {
        let _ = site::clear_notice(&cfg, &p).await;
    }
}

async fn execute_job(app: &AppHandle, job: Job) {
    let Some(job) = update_job(app, &job.id, |j| j.status = "running".into()) else { return };
    refresh_tray(app);
    let _ = app.emit("status-changed", ());
    let options = DeployOptions { sha: Some(job.sha.clone()), backup_db: job.backup_db, ..Default::default() };
    let result = run_deploy(app, &job.profile, options).await;
    let (status, text) = match &result {
        Ok(o) if o.ok => ("done", o.text.clone()),
        Ok(o) => ("failed", o.text.clone()),
        Err(e) => ("failed", e.clone()),
    };
    if job.announced_for.is_some() {
        clear_job_notice(app, &job).await;
    }
    update_job(app, &job.id, |j| {
        j.status = status.into();
        j.result = text;
        j.finished_at = Some(chrono::Utc::now().to_rfc3339());
    });
    refresh_tray(app);
    let _ = app.emit("status-changed", ());
}

// ── Site link ────────────────────────────────────────────────────────────────

async fn refresh_site(app: &AppHandle, profile: &str) -> SiteState {
    let state = app.state::<AppState>();
    let cfg = state.config.lock().unwrap().clone();
    let mut next = state.site.lock().unwrap().get(profile).cloned().unwrap_or_default();
    match cfg.profile(profile) {
        Ok(p) if site::enabled(&p) => match site::window(&cfg, &p).await {
            Ok(v) => {
                next.data = Some(v);
                next.error.clear();
            }
            Err(e) => next.error = e.to_string(),
        },
        Ok(_) => next.error = "Связь с сайтом выключена в настройках".into(),
        Err(e) => next.error = e.to_string(),
    }
    next.checked = Some(std::time::Instant::now());
    next.checked_at = chrono::Utc::now().to_rfc3339();
    state.site.lock().unwrap().insert(profile.to_string(), next.clone());
    refresh_tray(app);
    let _ = app.emit("site-changed", profile);
    next
}

/// Every 20 s: keep site answers fresh and move scheduled jobs forward.
fn start_scheduler(app: AppHandle) {
    *app.state::<AppState>().jobs.lock().unwrap() = schedule::load();
    tauri::async_runtime::spawn(async move {
        loop {
            scheduler_tick(&app).await;
            tokio::time::sleep(Duration::from_secs(20)).await;
        }
    });
}

async fn scheduler_tick(app: &AppHandle) {
    let state = app.state::<AppState>();
    let cfg = state.config.lock().unwrap().clone();
    let active: Vec<Job> = state.jobs.lock().unwrap().iter()
        .filter(|j| matches!(j.status.as_str(), "waiting" | "announced")).cloned().collect();
    for p in cfg.profiles.iter().filter(|p| site::enabled(p)) {
        let waiting_here = active.iter().any(|j| j.profile == p.id && (j.mode == "quiet" || j.status == "announced"));
        let max_age = if waiting_here { 60 } else { cfg.site_poll_seconds.max(30) };
        let stale = !state.site.lock().unwrap().get(&p.id).is_some_and(|s| s.fresh_within(max_age));
        if stale {
            refresh_site(app, &p.id).await;
        }
    }
    if state.busy.lock().unwrap().is_some() {
        return;
    }
    let now = chrono::Utc::now();
    for job in active {
        let verdict = state.site.lock().unwrap().get(&job.profile).and_then(|s| s.level().map(String::from));
        match schedule::decide(&job, now, verdict.as_deref()) {
            Action::Wait => {}
            Action::Announce { minutes, start } => {
                let Ok(p) = cfg.profile(&job.profile) else { continue };
                let msk = chrono::FixedOffset::east_opt(3 * 3600).expect("offset");
                let text = format!(
                    "Плановое обновление PoymAI в {} МСК. Связь может прерваться на минуту — ничего не потеряется.",
                    start.with_timezone(&msk).format("%H:%M")
                );
                let warning = site::announce(&cfg, &p, minutes, &text).await.err();
                update_job(app, &job.id, |j| {
                    j.status = "announced".into();
                    j.announced_for = Some(start.to_rfc3339());
                    // No notice is no reason to miss the slot.
                    j.result = warning.map(|e| format!("Не удалось предупредить пользователей: {e}")).unwrap_or_default();
                });
                notify(app, &format!("{}: деплой через {minutes} мин", p.name), "Пользователи предупреждены баннером в панели");
                refresh_tray(app);
                let _ = app.emit("status-changed", ());
            }
            Action::Unannounce => {
                clear_job_notice(app, &job).await;
                update_job(app, &job.id, |j| {
                    j.status = "waiting".into();
                    j.announced_for = None;
                    j.result = "На сайте снова работают люди — жду следующего тихого окна".into();
                });
                let _ = app.emit("status-changed", ());
            }
            Action::Run => {
                execute_job(app, job).await;
                return; // one deploy per tick; the rest wait for the next one
            }
            Action::Expire(reason) => {
                if job.announced_for.is_some() {
                    clear_job_notice(app, &job).await;
                }
                update_job(app, &job.id, |j| {
                    j.status = "expired".into();
                    j.result = reason.clone();
                    j.finished_at = Some(chrono::Utc::now().to_rfc3339());
                });
                let name = cfg.profile(&job.profile).map(|p| p.name).unwrap_or_default();
                notify(app, &format!("{name}: запланированный деплой не выполнен"), &reason);
                refresh_tray(app);
                let _ = app.emit("status-changed", ());
            }
        }
    }
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
        if site::enabled(p) {
            items.push(MenuItem::with_id(app, format!("quiet:{}", p.id), format!("Деплой {} в тихое окно…", p.name), true, None::<&str>)?);
        }
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
                    } else if let Some(profile) = id.strip_prefix("quiet:") {
                        show_window(app);
                        let _ = app.emit("tray-quiet", profile.to_string());
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
            start_scheduler(app.handle().clone());
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            snapshot, refresh, server_status, releases, pull_updates, pending, site_window, announce, deploy,
            schedule_deploy, cancel_job, run_job_now, save_config, dismiss_error, open_path, open_url
        ])
        .run(tauri::generate_context!())
        .expect("error while running Poymai Deploy");
}

/// Headless mode: `poymai-deploy deploy <profile> [-m "message"] [--dry-run] [--sha <sha>]`.
pub fn run_cli(args: &[String]) -> i32 {
    proc::fix_path();
    if args.get(1).map(String::as_str) == Some("window") {
        return cli_window(args.get(2).cloned().unwrap_or_else(|| "main".into()));
    }
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

/// `poymai-deploy window [profile]`: the site's verdict, for scripts and a quick look.
fn cli_window(profile_id: String) -> i32 {
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
        match site::window(&cfg, &profile).await {
            Ok(v) => {
                println!("{}", v["verdict"]["title"].as_str().unwrap_or("?"));
                for r in v["verdict"]["reasons"].as_array().into_iter().flatten() {
                    println!("  - {}", r["title"].as_str().unwrap_or_default());
                }
                if let Some(w) = v["windows"].as_array().and_then(|w| w.first()) {
                    println!("Лучшее окно: {} — {}", w["start"].as_str().unwrap_or_default(), w["end"].as_str().unwrap_or_default());
                }
                match site::verdict_level(&v) { "ok" => 0, "warn" => 3, _ => 4 }
            }
            Err(e) => {
                eprintln!("{e}");
                1
            }
        }
    })
}
