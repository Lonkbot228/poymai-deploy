//! Deploy = commit → push to GitHub → server installs exactly that commit → health check → `production` ref.

use crate::config::{Config, Profile};
use crate::git::{self, git};
use crate::proc::{self, run};
use crate::ssh;
use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Debug, Serialize)]
pub struct Event {
    pub profile: String,
    /// step | log | warn | error | done
    pub kind: String,
    pub step: String,
    pub text: String,
    pub progress: u8,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct DeployOptions {
    pub message: Option<String>,
    /// Deploy this existing GitHub commit (rollback / redeploy) instead of committing local work.
    pub sha: Option<String>,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub accept_drift: bool,
    #[serde(default)]
    pub backup_db: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Outcome {
    pub ok: bool,
    pub sha: String,
    pub status: String,
    pub text: String,
    pub code: i32,
}

pub const STEPS: &[(&str, u8, &str)] = &[
    ("preflight", 3, "Проверка репозитория"),
    ("checks", 6, "Локальные проверки"),
    ("commit", 10, "Коммит изменений"),
    ("push", 16, "Отправка в GitHub"),
    ("connect", 20, "Подключение к серверу"),
    ("lock", 23, "Блокировка деплоя"),
    ("fetch", 27, "Получение коммита с GitHub"),
    ("drift", 31, "Проверка ручных правок на сервере"),
    ("backup", 36, "Резервная копия базы"),
    ("checkout", 40, "Переключение версии"),
    ("validate", 44, "Проверка кода"),
    ("build", 50, "Сборка Docker-образов"),
    ("up", 82, "Перезапуск контейнеров"),
    ("health", 88, "Проверка здоровья"),
    ("rollback", 90, "Откат"),
    ("done", 96, "Готово на сервере"),
    ("finalize", 98, "Отметка релиза в GitHub"),
];

fn progress_of(step: &str) -> u8 {
    STEPS.iter().find(|s| s.0 == step).map(|s| s.1).unwrap_or(0)
}

pub fn machine_name() -> String {
    let host = hostname::get().map(|h| h.to_string_lossy().into_owned()).unwrap_or_default();
    let os = if cfg!(windows) { "Windows" } else if cfg!(target_os = "macos") { "Mac" } else { "Linux" };
    format!("{os} · {host}")
}

struct Ctx<'a, F: Fn(Event) + Send + Sync> {
    profile: &'a Profile,
    emit: &'a F,
    progress: AtomicU8,
}

impl<F: Fn(Event) + Send + Sync> Ctx<'_, F> {
    fn send(&self, kind: &str, step: &str, text: impl Into<String>) {
        if kind == "step" {
            self.progress.fetch_max(progress_of(step), Ordering::Relaxed);
        }
        (self.emit)(Event {
            profile: self.profile.id.clone(),
            kind: kind.into(),
            step: step.into(),
            text: text.into(),
            progress: self.progress.load(Ordering::Relaxed),
        });
    }
    fn step(&self, id: &str) {
        let label = STEPS.iter().find(|s| s.0 == id).map(|s| s.2).unwrap_or(id);
        self.send("step", id, label);
    }
    fn log(&self, text: impl Into<String>) {
        self.send("log", "", text);
    }
}

async fn run_check(repo: &Path, line: &str) -> Result<proc::Out> {
    if cfg!(windows) {
        run("cmd", &["/d", "/s", "/c", line], Some(repo)).await
    } else {
        run("sh", &["-lc", line], Some(repo)).await
    }
}

/// Commits local work (if any), rebases onto GitHub and pushes. Returns the commit to deploy.
async fn publish<F: Fn(Event) + Send + Sync>(ctx: &Ctx<'_, F>, repo: &Path, opts: &DeployOptions) -> Result<String> {
    let p = ctx.profile;
    ctx.step("preflight");
    let state = git::local_state(repo, &p.branch).await?;
    if state.branch != p.branch {
        bail!("Сейчас открыта ветка «{}». Деплой делается только из «{}».", state.branch, p.branch);
    }
    if Path::new(repo).join(".git").join("rebase-merge").exists() || repo.join(".git").join("MERGE_HEAD").exists() {
        bail!("В репозитории незавершённый merge/rebase — завершите его вручную.");
    }
    git(repo, &["fetch", "-q", "origin", "--prune"]).await
        .map_err(|e| anyhow!("Не удалось связаться с GitHub: {e}"))?;

    if !p.checks.is_empty() {
        ctx.step("checks");
        for check in &p.checks {
            ctx.log(format!("$ {check}"));
            let out = run_check(repo, check).await?;
            if !out.ok() {
                for line in out.text().lines().rev().take(40).collect::<Vec<_>>().into_iter().rev() {
                    ctx.log(line);
                }
                bail!("Проверка «{check}» не прошла");
            }
        }
    }

    ctx.step("commit");
    if state.changes.is_empty() {
        ctx.log("Нет незакоммиченных изменений");
    } else {
        git::check_secrets(repo, &state.changes)?;
        let message = opts
            .message
            .clone()
            .filter(|m| !m.trim().is_empty())
            .unwrap_or_else(|| git::auto_message(&state.changes));
        git(repo, &["add", "-A"]).await?;
        let trailer = format!("Deployed-from: {}", machine_name());
        git(repo, &["commit", "-q", "-m", message.trim(), "-m", &trailer]).await?;
        ctx.log(format!("Коммит: {} ({} файлов)", message.trim(), state.changes.len()));
    }

    ctx.step("push");
    let upstream = format!("origin/{}", p.branch);
    let refspec = format!("HEAD:refs/heads/{}", p.branch);
    for attempt in 0..3 {
        let behind: u32 = git(repo, &["rev-list", "--count", &format!("HEAD..{upstream}")]).await?.parse().unwrap_or(0);
        if behind > 0 {
            ctx.log(format!("На GitHub {behind} новых коммитов с другого компьютера — перебазирую"));
            let out = run("git", &["rebase", "-q", &upstream], Some(repo)).await?;
            if !out.ok() {
                let _ = run("git", &["rebase", "--abort"], Some(repo)).await;
                bail!("Конфликт с изменениями с другого компьютера. Сделайте `git pull --rebase` и решите конфликт вручную.\n{}", out.text());
            }
        }
        let out = run("git", &["push", "-q", "origin", &refspec], Some(repo)).await?;
        if out.ok() {
            break;
        }
        if attempt == 2 {
            bail!("git push не удался: {}", out.text());
        }
        ctx.log("GitHub отклонил push (кто-то запушил раньше) — повторяю");
        git(repo, &["fetch", "-q", "origin"]).await?;
    }
    let sha = git(repo, &["rev-parse", "HEAD"]).await?;
    ctx.log(format!("GitHub {} = {}", p.branch, &sha[..8]));
    Ok(sha)
}

pub async fn deploy<F: Fn(Event) + Send + Sync>(cfg: &Config, profile: &Profile, opts: DeployOptions, emit: &F) -> Result<Outcome> {
    let ctx = Ctx { profile, emit, progress: AtomicU8::new(0) };
    let repo = PathBuf::from(&profile.repo_path);
    let result = deploy_inner(&ctx, cfg, &repo, &opts).await;
    match &result {
        Ok(o) if o.ok => {
            ctx.progress.store(100, Ordering::Relaxed);
            ctx.send("done", "done", o.text.clone());
        }
        Ok(o) => ctx.send("error", "", o.text.clone()),
        Err(e) => ctx.send("error", "", e.to_string()),
    }
    result
}

async fn deploy_inner<F: Fn(Event) + Send + Sync>(ctx: &Ctx<'_, F>, cfg: &Config, repo: &Path, opts: &DeployOptions) -> Result<Outcome> {
    let p = ctx.profile;
    let sha = match &opts.sha {
        Some(sha) => {
            ctx.step("preflight");
            git(repo, &["fetch", "-q", "origin", "--prune"]).await?;
            git(repo, &["rev-parse", &format!("{sha}^{{commit}}")]).await
                .map_err(|_| anyhow!("Коммит {sha} не найден"))?
        }
        None => publish(ctx, repo, opts).await?,
    };

    ctx.step("connect");
    let route = ssh::pick_route(p).await?;
    ctx.log(format!("Маршрут: {} ({}@{}{})", route.name, route.user, route.host,
        route.jump.as_ref().map(|j| format!(" через {j}")).unwrap_or_default()));

    let mut args = format!("deploy {sha}");
    if opts.dry_run { args.push_str(" --dry-run"); }
    if opts.accept_drift { args.push_str(" --accept-drift"); }
    if opts.backup_db { args.push_str(" --backup-db"); }
    let actor = git(repo, &["config", "user.name"]).await.unwrap_or_else(|_| "unknown".into());
    let remote = ssh::release_script(&p.remote_dir, &sha, &args, &actor, &machine_name());

    let mut error: Option<(i32, String)> = None;
    let mut result_status = String::new();
    let code = proc::stream(ssh::ssh_command(cfg, &route, &remote), |line| {
        if let Some(rest) = line.strip_prefix("::step ") {
            let (id, _text) = rest.split_once('|').unwrap_or((rest, ""));
            ctx.step(id);
        } else if let Some(rest) = line.strip_prefix("::warn ") {
            ctx.send("warn", "", rest);
        } else if let Some(rest) = line.strip_prefix("::error ") {
            let (c, text) = rest.split_once('|').unwrap_or(("1", rest));
            error = Some((c.parse().unwrap_or(1), text.to_string()));
        } else if let Some(rest) = line.strip_prefix("::result ") {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(rest) {
                result_status = v["status"].as_str().unwrap_or_default().to_string();
            }
        } else if !line.trim().is_empty() {
            ctx.log(line);
        }
    })
    .await?;

    if code != 0 {
        let (c, text) = error.unwrap_or((code, format!("Сервер вернул код {code}")));
        let text = if code == 255 { format!("SSH-соединение прервалось ({}). {text}", route.host) } else { text };
        return Ok(Outcome { ok: false, sha, status: if result_status.is_empty() { "failed".into() } else { result_status }, text, code: c });
    }

    if opts.dry_run {
        return Ok(Outcome { ok: true, sha: sha.clone(), status: "dry_run".into(), text: "Проверка пройдена, сервер не тронут".into(), code: 0 });
    }

    ctx.step("finalize");
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let prod = format!("{sha}:refs/heads/production");
    let tag = format!("{sha}:refs/tags/deploy/{stamp}");
    if let Err(e) = git(repo, &["push", "-q", "--force", "origin", &prod, &tag]).await {
        ctx.send("warn", "", format!("Сервер обновлён, но отметить релиз в GitHub не удалось: {e}"));
    }
    Ok(Outcome { ok: true, sha: sha.clone(), status: "success".into(), text: format!("Задеплоено {}", &sha[..8]), code: 0 })
}
