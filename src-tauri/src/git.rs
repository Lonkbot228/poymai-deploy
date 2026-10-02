//! Thin wrappers over the git CLI, so the user's own credentials (Credential Manager / Keychain) are used.

use crate::proc::{run, run_ok};
use anyhow::{bail, Result};
use serde::Serialize;
use std::path::Path;

pub async fn git(repo: &Path, args: &[&str]) -> Result<String> {
    run_ok("git", args, Some(repo)).await
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct ChangedFile {
    pub status: String,
    pub path: String,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct LocalState {
    pub branch: String,
    pub head: String,
    pub head_message: String,
    pub upstream: String,
    pub ahead: u32,
    pub behind: u32,
    pub changes: Vec<ChangedFile>,
}

pub async fn local_state(repo: &Path, branch: &str) -> Result<LocalState> {
    if !repo.join(".git").exists() {
        bail!("{} — не git-репозиторий", repo.display());
    }
    let cur = git(repo, &["rev-parse", "--abbrev-ref", "HEAD"]).await?;
    let head = git(repo, &["rev-parse", "HEAD"]).await.unwrap_or_default();
    let head_message = git(repo, &["log", "-1", "--format=%s"]).await.unwrap_or_default();
    let upstream_ref = format!("origin/{branch}");
    let upstream = git(repo, &["rev-parse", &upstream_ref]).await.unwrap_or_default();
    let (mut ahead, mut behind) = (0, 0);
    if !upstream.is_empty() {
        let range = format!("{upstream_ref}...HEAD");
        if let Ok(counts) = git(repo, &["rev-list", "--left-right", "--count", &range]).await {
            let mut it = counts.split_whitespace();
            behind = it.next().and_then(|v| v.parse().ok()).unwrap_or(0);
            ahead = it.next().and_then(|v| v.parse().ok()).unwrap_or(0);
        }
    }
    Ok(LocalState {
        branch: cur,
        head,
        head_message,
        upstream,
        ahead,
        behind,
        changes: changes(repo).await?,
    })
}

pub async fn changes(repo: &Path) -> Result<Vec<ChangedFile>> {
    let out = git(repo, &["status", "--porcelain=v1", "--untracked-files=all"]).await?;
    Ok(out
        .lines()
        .filter(|l| l.len() > 3)
        .map(|l| ChangedFile {
            status: l[..2].trim().to_string(),
            path: l[3..].trim_matches('"').to_string(),
        })
        .collect())
}

/// Remote branch heads without fetching objects: (main, production).
pub async fn remote_heads(repo: &Path, branch: &str) -> Result<(String, String)> {
    let main_ref = format!("refs/heads/{branch}");
    let out = run("git", &["ls-remote", "origin", &main_ref, "refs/heads/production"], Some(repo)).await?;
    if !out.ok() {
        bail!("GitHub недоступен: {}", out.text());
    }
    let mut main = String::new();
    let mut prod = String::new();
    for line in out.stdout.lines() {
        let mut parts = line.split_whitespace();
        let (Some(sha), Some(name)) = (parts.next(), parts.next()) else { continue };
        if name == main_ref {
            main = sha.into();
        } else if name == "refs/heads/production" {
            prod = sha.into();
        }
    }
    Ok((main, prod))
}

const BLOCKED_SUFFIXES: &[&str] = &[".pem", ".key", ".p12", ".pfx", ".session", ".sqlite", ".db"];
const MAX_FILE_BYTES: u64 = 20 * 1024 * 1024;

/// Refuses to commit anything that looks like a secret or a large binary.
pub fn check_secrets(repo: &Path, files: &[ChangedFile]) -> Result<()> {
    let mut problems = vec![];
    for f in files.iter().filter(|f| f.status != "D") {
        let name = f.path.rsplit('/').next().unwrap_or(&f.path).to_lowercase();
        let is_env = name.starts_with(".env") && !name.ends_with(".example");
        let is_deploy_env = name.starts_with(".deploy") && name.ends_with(".env");
        if is_env || is_deploy_env || BLOCKED_SUFFIXES.iter().any(|s| name.ends_with(s)) || name.starts_with("id_") {
            problems.push(format!("секрет: {}", f.path));
            continue;
        }
        if let Ok(meta) = std::fs::metadata(repo.join(&f.path)) {
            if meta.is_file() && meta.len() > MAX_FILE_BYTES {
                problems.push(format!("файл больше 20 МБ: {} ({} МБ)", f.path, meta.len() / 1024 / 1024));
            }
        }
    }
    if !problems.is_empty() {
        bail!(
            "Коммит остановлен — добавьте эти файлы в .gitignore:\n{}",
            problems.join("\n")
        );
    }
    Ok(())
}

pub fn auto_message(files: &[ChangedFile]) -> String {
    let mut areas: Vec<String> = vec![];
    for f in files {
        let top = f.path.split('/').take(if f.path.starts_with("frontend/src/") { 4 } else { 2 }).collect::<Vec<_>>();
        let area = if top.len() > 1 { top[..top.len() - 1].join("/") } else { "root".into() };
        if !areas.contains(&area) {
            areas.push(area);
        }
    }
    let shown = areas.iter().take(4).cloned().collect::<Vec<_>>().join(", ");
    let more = if areas.len() > 4 { format!(" +{}", areas.len() - 4) } else { String::new() };
    format!("Update {shown}{more} ({} files)", files.len())
}
