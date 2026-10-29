//! "Что уедет": commits and files between what runs in production and this checkout,
//! grouped by area, with the things worth knowing before a restart (migrations, rebuilds).

use crate::config::Profile;
use crate::git::{self, git};
use anyhow::Result;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize)]
pub struct Commit {
    pub sha: String,
    pub subject: String,
    pub author: String,
    pub date: String,
    pub machine: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Area {
    pub name: String,
    pub count: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct Flag {
    /// info | warn
    pub level: String,
    pub text: String,
}

#[derive(Clone, Debug, Serialize, Default)]
pub struct Pending {
    pub base: String,
    pub commits: Vec<Commit>,
    pub local_files: usize,
    pub files: Vec<String>,
    pub areas: Vec<Area>,
    pub migrations: Vec<String>,
    pub flags: Vec<Flag>,
}

/// Area label for a repository path (PoymAI layout; anything else falls into the generic ones).
pub fn area_of(path: &str) -> &'static str {
    let p = path.to_ascii_lowercase();
    if p.contains("/migrations/") || p.starts_with("migrations/") {
        "Миграции БД"
    } else if p.starts_with("frontend/") && (p.ends_with("package.json") || p.ends_with("package-lock.json")) {
        "Зависимости JS"
    } else if p.ends_with("requirements.txt") || p.ends_with("pyproject.toml") {
        "Зависимости Python"
    } else if p.contains("dockerfile") || p.starts_with("docker-compose") {
        "Docker"
    } else if p.starts_with("nginx/") {
        "Nginx"
    } else if p.starts_with("scripts/deploy/") {
        "Скрипт деплоя"
    } else if p.starts_with("frontend/") {
        "Фронтенд"
    } else if p.starts_with("tests/") || p.contains("/test") || p.contains(".test.") {
        "Тесты"
    } else if p.ends_with(".md") || p.starts_with("docs/") {
        "Документация"
    } else if p.starts_with("app/") || p.ends_with(".py") || p.ends_with(".js") || p.ends_with(".ts") {
        "Бэкенд"
    } else {
        "Прочее"
    }
}

pub fn flags_for(areas: &[Area], migrations: &[String]) -> Vec<Flag> {
    let has = |name: &str| areas.iter().any(|a| a.name == name);
    let mut flags = vec![];
    if !migrations.is_empty() {
        flags.push(Flag {
            level: "warn".into(),
            text: format!("Миграции БД ({}) — перед деплоем сервер сделает бэкап базы. Откатить код можно, схему — нет.", migrations.len()),
        });
    }
    if has("Docker") || has("Зависимости Python") || has("Зависимости JS") {
        flags.push(Flag { level: "info".into(), text: "Меняются зависимости или Docker — сборка займёт дольше обычного".into() });
    }
    if has("Nginx") {
        flags.push(Flag { level: "info".into(), text: "Меняется nginx — контейнер прокси пересоздастся".into() });
    }
    if has("Скрипт деплоя") {
        flags.push(Flag { level: "warn".into(), text: "Меняется сам скрипт деплоя — этот релиз поставится уже новым скриптом".into() });
    }
    if !areas.is_empty() && areas.iter().all(|a| a.name == "Тесты" || a.name == "Документация") {
        flags.push(Flag { level: "info".into(), text: "Только тесты и документация — для пользователей ничего не меняется".into() });
    }
    flags
}

pub async fn pending(profile: &Profile, production: &str) -> Result<Pending> {
    let repo = PathBuf::from(&profile.repo_path);
    let local = git::local_state(&repo, &profile.branch).await?;
    let mut base = production.to_string();
    if !base.is_empty() && git(&repo, &["cat-file", "-e", &format!("{base}^{{commit}}")]).await.is_err() {
        let _ = git(&repo, &["fetch", "-q", "origin", "production"]).await;
    }
    if base.is_empty() || git(&repo, &["cat-file", "-e", &format!("{base}^{{commit}}")]).await.is_err() {
        base = String::new();
    }

    let mut commits = vec![];
    let mut files: Vec<String> = vec![];
    if !base.is_empty() {
        let range = format!("{base}..HEAD");
        let log = git(&repo, &["log", "-100", "--format=%H%x1f%s%x1f%an%x1f%aI%x1f%(trailers:key=Deployed-from,valueonly,separator=)", &range])
            .await
            .unwrap_or_default();
        for line in log.lines() {
            let parts: Vec<&str> = line.split('\u{1f}').collect();
            if parts.len() >= 4 {
                commits.push(Commit {
                    sha: parts[0].into(),
                    subject: parts[1].into(),
                    author: parts[2].into(),
                    date: parts[3].into(),
                    machine: parts.get(4).map(|s| s.trim().to_string()).unwrap_or_default(),
                });
            }
        }
        files = git(&repo, &["diff", "--name-only", &base, "HEAD"]).await.unwrap_or_default()
            .lines().map(String::from).collect();
    }
    for change in &local.changes {
        if !files.contains(&change.path) {
            files.push(change.path.clone());
        }
    }

    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for f in &files {
        *counts.entry(area_of(f)).or_default() += 1;
    }
    let mut areas: Vec<Area> = counts.into_iter().map(|(name, count)| Area { name: name.into(), count }).collect();
    areas.sort_by(|a, b| b.count.cmp(&a.count));
    let migrations: Vec<String> = files.iter().filter(|f| area_of(f) == "Миграции БД")
        .map(|f| f.rsplit('/').next().unwrap_or(f).to_string()).collect();
    let flags = flags_for(&areas, &migrations);
    files.truncate(300);
    Ok(Pending { base, commits, local_files: local.changes.len(), files, areas, migrations, flags })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn areas_follow_the_poymai_layout() {
        assert_eq!(area_of("app/migrations/20261034_x.sql"), "Миграции БД");
        assert_eq!(area_of("app/server.py"), "Бэкенд");
        assert_eq!(area_of("frontend/src/app/page.tsx"), "Фронтенд");
        assert_eq!(area_of("frontend/package-lock.json"), "Зависимости JS");
        assert_eq!(area_of("requirements.txt"), "Зависимости Python");
        assert_eq!(area_of("nginx/nginx.conf"), "Nginx");
        assert_eq!(area_of("tests/test_x.py"), "Тесты");
        assert_eq!(area_of("docker-compose.yml"), "Docker");
    }

    #[test]
    fn flags_warn_about_migrations_and_calm_test_only_releases() {
        let areas = vec![Area { name: "Миграции БД".into(), count: 1 }];
        let flags = flags_for(&areas, &["x.sql".into()]);
        assert_eq!(flags[0].level, "warn");
        let calm = flags_for(&[Area { name: "Тесты".into(), count: 3 }], &[]);
        assert!(calm[0].text.starts_with("Только тесты"));
    }
}
