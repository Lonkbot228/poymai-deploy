//! What the live site says about a restart: who works right now, background jobs, the quietest
//! hours ahead (PoymAI `app/deploy_window.py`). Read over SSH with the probe inside the backend
//! container, so no site password lives on this computer and nothing new is exposed publicly.

use crate::config::{Config, Profile};
use crate::ssh;
use anyhow::{bail, Result};
use serde_json::Value;

fn probe(profile: &Profile) -> Result<&str> {
    match profile.site_probe.as_deref().map(str::trim) {
        Some(cmd) if !cmd.is_empty() => Ok(cmd),
        _ => bail!("Для «{}» не настроена связь с сайтом", profile.name),
    }
}

async fn call(cfg: &Config, profile: &Profile, args: &str) -> Result<Value> {
    let cmd = probe(profile)?;
    let route = ssh::pick_route(profile).await?;
    let remote = format!("cd {} && {cmd} {args}", ssh::sh_quote(&profile.remote_dir));
    let out = match ssh::exec(cfg, &route, &remote).await {
        Ok(out) => out,
        Err(e) => {
            let text = e.to_string();
            if text.contains("No module named") || text.contains("app.deploy_window") {
                bail!("На сервере старая версия сайта без связи с Poymai Deploy — задеплойте PoymAI один раз вручную");
            }
            return Err(e);
        }
    };
    let Some(line) = out.lines().rev().find(|l| l.trim_start().starts_with('{')) else {
        // A site without the probe imports the module and prints nothing.
        bail!("На сервере старая версия сайта без связи с Poymai Deploy — задеплойте PoymAI");
    };
    let value: Value = serde_json::from_str(line)?;
    if let Some(err) = value.get("error").and_then(Value::as_str) {
        bail!("Сайт не ответил: {err}");
    }
    Ok(value)
}

pub fn enabled(profile: &Profile) -> bool {
    probe(profile).is_ok()
}

/// Full deploy-window payload: verdict, people, signals, timeline, windows, notice.
pub async fn window(cfg: &Config, profile: &Profile) -> Result<Value> {
    let value = call(cfg, profile, "status").await?;
    if value["verdict"]["level"].as_str().is_none() {
        bail!("Сайт ответил без оценки готовности — обновите PoymAI");
    }
    Ok(value)
}

/// Shows "Обновление через N мин" to everyone in the dashboard.
pub async fn announce(cfg: &Config, profile: &Profile, minutes: u32, text: &str) -> Result<()> {
    let args = if text.trim().is_empty() {
        format!("notice {minutes}")
    } else {
        format!("notice {minutes} {}", ssh::sh_quote(text.trim()))
    };
    call(cfg, profile, &args).await.map(|_| ())
}

pub async fn clear_notice(cfg: &Config, profile: &Profile) -> Result<()> {
    call(cfg, profile, "clear").await.map(|_| ())
}

/// "ok" | "warn" | "stop" | "" when unknown.
pub fn verdict_level(window: &Value) -> &str {
    window["verdict"]["level"].as_str().unwrap_or("")
}
