//! Keeps this computer in step with GitHub: polls `ls-remote`, fast-forwards clean checkouts,
//! asks before touching a checkout with local edits.

use crate::config::{Config, Profile};
use crate::git::{self, git, LocalState};
use crate::ssh;
use anyhow::{bail, Result};
use serde::Serialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, Serialize)]
pub struct ProfileStatus {
    pub id: String,
    pub name: String,
    pub local: Option<LocalState>,
    pub github: String,
    pub production: String,
    pub error: String,
    pub checked_at: String,
    /// Commits on GitHub this checkout has not got yet.
    pub incoming: u32,
    /// Files edited both locally and in the incoming commits.
    pub overlap: Vec<String>,
}

pub struct SyncNotice {
    pub title: String,
    pub body: String,
}

async fn incoming_overlap(repo: &Path, branch: &str, local: &LocalState) -> Vec<String> {
    let range = format!("HEAD...origin/{branch}");
    let theirs = git(repo, &["diff", "--name-only", &range]).await.unwrap_or_default();
    let theirs: HashSet<&str> = theirs.lines().collect();
    local
        .changes
        .iter()
        .filter(|c| theirs.contains(c.path.as_str()))
        .map(|c| c.path.clone())
        .collect()
}

pub async fn status(profile: &Profile) -> ProfileStatus {
    let mut st = ProfileStatus {
        id: profile.id.clone(),
        name: profile.name.clone(),
        checked_at: chrono::Local::now().format("%H:%M:%S").to_string(),
        ..Default::default()
    };
    let repo = PathBuf::from(&profile.repo_path);
    match git::remote_heads(&repo, &profile.branch).await {
        Ok((main, prod)) => {
            st.github = main;
            st.production = prod;
        }
        Err(e) => st.error = e.to_string(),
    }
    match git::local_state(&repo, &profile.branch).await {
        Ok(mut local) => {
            if !st.github.is_empty() && st.github != local.upstream {
                // GitHub moved: bring the objects in so ahead/behind are real.
                if git(&repo, &["fetch", "-q", "origin", "--prune"]).await.is_ok() {
                    if let Ok(fresh) = git::local_state(&repo, &profile.branch).await {
                        local = fresh;
                    }
                }
            }
            st.incoming = local.behind;
            if local.behind > 0 {
                st.overlap = incoming_overlap(&repo, &profile.branch, &local).await;
            }
            st.local = Some(local);
        }
        Err(e) => st.error = e.to_string(),
    }
    st
}

fn lockfile_hint(changed: &str) -> Option<String> {
    let mut hints = vec![];
    if changed.lines().any(|l| l.ends_with("package-lock.json")) {
        hints.push("npm ci");
    }
    if changed.lines().any(|l| l.ends_with("requirements.txt")) {
        hints.push("pip install -r requirements.txt");
    }
    (!hints.is_empty()).then(|| format!("Изменились зависимости — выполните: {}", hints.join(", ")))
}

async fn describe_incoming(repo: &Path, from: &str, to: &str) -> String {
    let range = format!("{from}..{to}");
    let log = git(repo, &["log", "--format=%s%x1f%(trailers:key=Deployed-from,valueonly,separator=)", &range])
        .await
        .unwrap_or_default();
    let mut lines = log.lines();
    let first = lines.next().unwrap_or_default();
    let (subject, origin) = first.split_once('\u{1f}').unwrap_or((first, ""));
    let count = log.lines().count();
    let origin = if origin.trim().is_empty() { String::new() } else { format!(" с {}", origin.trim()) };
    if count > 1 {
        format!("{subject} (+{} ещё){origin}", count - 1)
    } else {
        format!("{subject}{origin}")
    }
}

/// Clean checkout behind GitHub → fast-forward silently. Returns a notification text when something happened.
pub async fn auto_pull(profile: &Profile, st: &ProfileStatus) -> Option<SyncNotice> {
    let local = st.local.as_ref()?;
    if !profile.auto_pull || local.behind == 0 || local.branch != profile.branch {
        return None;
    }
    let repo = PathBuf::from(&profile.repo_path);
    let upstream = format!("origin/{}", profile.branch);
    if !local.changes.is_empty() || local.ahead > 0 {
        let what = describe_incoming(&repo, &local.head, &upstream).await;
        return Some(SyncNotice {
            title: format!("{}: есть обновление", profile.name),
            body: format!("{what}. У вас локальные правки — откройте Poymai Deploy и нажмите «Обновить»."),
        });
    }
    let before = local.head.clone();
    if git(&repo, &["merge", "-q", "--ff-only", &upstream]).await.is_err() {
        return None;
    }
    let what = describe_incoming(&repo, &before, "HEAD").await;
    let changed = git(&repo, &["diff", "--name-only", &before, "HEAD"]).await.unwrap_or_default();
    let mut body = what;
    if let Some(hint) = lockfile_hint(&changed) {
        body.push_str(". ");
        body.push_str(&hint);
    }
    Some(SyncNotice { title: format!("{}: обновлено", profile.name), body })
}

/// User asked to update a checkout that has local edits: stash → rebase onto GitHub → unstash.
pub async fn pull_with_local_changes(profile: &Profile) -> Result<String> {
    let repo = PathBuf::from(&profile.repo_path);
    git(&repo, &["fetch", "-q", "origin", "--prune"]).await?;
    let local = git::local_state(&repo, &profile.branch).await?;
    if local.branch != profile.branch {
        bail!("Открыта ветка «{}», а не «{}»", local.branch, profile.branch);
    }
    let overlap = incoming_overlap(&repo, &profile.branch, &local).await;
    if !overlap.is_empty() {
        bail!(
            "Эти файлы изменены и у вас, и в обновлении — закоммитьте или разрулите вручную:\n{}",
            overlap.join("\n")
        );
    }
    let upstream = format!("origin/{}", profile.branch);
    let before = local.head.clone();
    let stashed = !local.changes.is_empty();
    if stashed {
        git(&repo, &["stash", "push", "-u", "-q", "-m", "poymai-deploy: auto-stash before update"]).await?;
    }
    let pulled = if local.ahead > 0 {
        git(&repo, &["rebase", "-q", &upstream]).await
    } else {
        git(&repo, &["merge", "-q", "--ff-only", &upstream]).await
    };
    if let Err(e) = pulled {
        let _ = git(&repo, &["rebase", "--abort"]).await;
        if stashed {
            let _ = git(&repo, &["stash", "pop", "-q"]).await;
        }
        bail!("Не удалось обновиться, всё возвращено как было: {e}");
    }
    if stashed {
        if let Err(e) = git(&repo, &["stash", "pop", "-q"]).await {
            bail!("Обновление получено, но ваши правки не вернулись автоматически — они в `git stash list`: {e}");
        }
    }
    let changed = git(&repo, &["diff", "--name-only", &before, "HEAD"]).await.unwrap_or_default();
    let mut text = describe_incoming(&repo, &before, "HEAD").await;
    if let Some(hint) = lockfile_hint(&changed) {
        text.push_str(". ");
        text.push_str(&hint);
    }
    Ok(text)
}

pub async fn server_status(cfg: &Config, profile: &Profile) -> Result<serde_json::Value> {
    let route = ssh::pick_route(profile).await?;
    let out = ssh::exec(cfg, &route, &ssh::server_script(&profile.remote_dir, "status")).await?;
    let mut v: serde_json::Value = serde_json::from_str(out.lines().last().unwrap_or("{}"))?;
    v["route"] = serde_json::Value::String(route.name);
    Ok(v)
}

pub async fn releases(cfg: &Config, profile: &Profile) -> Result<Vec<serde_json::Value>> {
    let route = ssh::pick_route(profile).await?;
    let out = ssh::exec(cfg, &route, &ssh::server_script(&profile.remote_dir, "releases 40")).await?;
    let mut list: Vec<serde_json::Value> = out.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
    list.reverse();
    Ok(list)
}
