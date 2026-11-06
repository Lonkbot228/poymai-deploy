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
    /// A commit message built from the changed paths, offered to the user
    /// for confirmation; never committed without them seeing it.
    pub suggested_message: String,
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
    let changes = changes(repo).await?;
    let suggested_message = if changes.is_empty() { String::new() } else { auto_message(&changes) };
    Ok(LocalState {
        branch: cur,
        head,
        head_message,
        upstream,
        ahead,
        behind,
        changes,
        suggested_message,
    })
}

/// Where the previous HEAD is kept when GitHub history was rewritten under it.
pub const BEFORE_REWRITE_REF: &str = "refs/poymai-deploy/before-rewrite";

/// How HEAD got onto `origin/<branch>`.
#[derive(Debug, PartialEq, Eq)]
pub enum Rebased {
    /// Histories are related: a plain `git rebase`.
    Plain,
    /// GitHub history was rewritten (force-push), so HEAD and upstream share no commit.
    /// `upstream_twin` is the GitHub commit holding exactly the tree of the newest local
    /// commit that was already there; only local commits after it were replayed.
    Rewritten { replayed: u32, upstream_twin: String },
}

/// Newest local commit whose tree some upstream commit also has, and that upstream
/// commit (the newest one with it). Both inputs are `git log --format='%H %T'` output,
/// newest first.
fn newest_shared_tree(local_log: &str, upstream_log: &str) -> Option<(String, String)> {
    let mut by_tree = std::collections::HashMap::new();
    for line in upstream_log.lines() {
        if let Some((hash, tree)) = line.split_once(' ') {
            by_tree.entry(tree).or_insert(hash);
        }
    }
    local_log.lines().find_map(|line| {
        let (hash, tree) = line.split_once(' ')?;
        by_tree.get(tree).map(|twin| (hash.to_string(), twin.to_string()))
    })
}

/// Puts HEAD on top of `upstream`. Also copes with a force-pushed upstream, where a plain
/// rebase would replay the whole history from the first commit and conflict everywhere.
/// On failure the rebase is aborted and the git output is returned as the error.
pub async fn rebase_onto_upstream(repo: &Path, upstream: &str) -> Result<Rebased> {
    let related = git(repo, &["merge-base", "HEAD", upstream]).await.is_ok();
    let (args, rebased) = if related {
        (vec!["rebase", "-q", upstream].into_iter().map(String::from).collect::<Vec<_>>(), Rebased::Plain)
    } else {
        let local_log = git(repo, &["log", "--format=%H %T", "HEAD"]).await?;
        let upstream_log = git(repo, &["log", "--format=%H %T", upstream]).await?;
        let Some((local_base, upstream_twin)) = newest_shared_tree(&local_log, &upstream_log) else {
            bail!(
                "История {upstream} переписана и не имеет общих снимков с локальной веткой — \
                 перенесите свои коммиты вручную (git rebase --onto)."
            );
        };
        let replayed: u32 = git(repo, &["rev-list", "--count", &format!("{local_base}..HEAD")])
            .await?
            .parse()
            .unwrap_or(0);
        git(repo, &["update-ref", BEFORE_REWRITE_REF, "HEAD"]).await?;
        let args = ["rebase", "-q", "--onto", upstream, &local_base].map(String::from).to_vec();
        (args, Rebased::Rewritten { replayed, upstream_twin })
    };
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = run("git", &args, Some(repo)).await?;
    if !out.ok() {
        let _ = run("git", &["rebase", "--abort"], Some(repo)).await;
        bail!("{}", out.text());
    }
    Ok(rebased)
}

pub async fn changes(repo: &Path) -> Result<Vec<ChangedFile>> {
    // `run`, not `git`/`run_ok`: those trim stdout, and the first entry of
    // " M app/x.py" starts with a space that belongs to the status column.
    let out = run("git", &["status", "--porcelain=v1", "-z", "--untracked-files=all"], Some(repo)).await?;
    if !out.ok() {
        bail!("git status: {}", out.text());
    }
    Ok(parse_porcelain_z(&out.stdout))
}

/// Parses `git status --porcelain=v1 -z`: `XY path\0`, where a rename or copy
/// is followed by one more `\0`-terminated field with the original path.
/// With `-z` paths are never quoted, so names with spaces or non-ASCII
/// characters come through as they are.
pub fn parse_porcelain_z(out: &str) -> Vec<ChangedFile> {
    let mut files = vec![];
    let mut fields = out.split('\0');
    while let Some(entry) = fields.next() {
        if entry.len() < 4 || !entry.is_char_boundary(2) || !entry.is_char_boundary(3) {
            continue;
        }
        let status = &entry[..2];
        if status.contains('R') || status.contains('C') {
            fields.next();
        }
        files.push(ChangedFile {
            status: status.trim().to_string(),
            path: entry[3..].to_string(),
        });
    }
    files
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

/// "Update app, frontend/src/app (8 files)": the areas the change touches.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn file(status: &str, path: &str) -> ChangedFile {
        ChangedFile { status: status.into(), path: path.into() }
    }

    #[test]
    fn first_entry_keeps_its_leading_status_space() {
        let files = parse_porcelain_z(" M app/x.py\0 M frontend/src/app/page.tsx\0?? tests/new.py\0");
        let paths: Vec<_> = files.iter().map(|f| (f.status.as_str(), f.path.as_str())).collect();
        assert_eq!(
            paths,
            vec![("M", "app/x.py"), ("M", "frontend/src/app/page.tsx"), ("??", "tests/new.py")]
        );
    }

    #[test]
    fn renames_skip_the_original_path_field() {
        let files = parse_porcelain_z("R  docs/new.md\0old.md\0 M app/x.py\0");
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].status, "R");
        assert_eq!(files[0].path, "docs/new.md");
        assert_eq!(files[1].path, "app/x.py");
    }

    #[test]
    fn spaces_and_cyrillic_are_not_quoted_or_cut() {
        let files = parse_porcelain_z("?? docs/план работ.md\0");
        assert_eq!(files[0].path, "docs/план работ.md");
    }

    #[test]
    fn a_modified_root_env_is_still_caught_as_a_secret() {
        let files = parse_porcelain_z(" M .env\0");
        assert_eq!(files[0].path, ".env");
        let err = check_secrets(Path::new("/nonexistent"), &files).unwrap_err().to_string();
        assert!(err.contains("секрет: .env"), "{err}");
    }

    #[test]
    fn rewritten_history_is_matched_by_tree() {
        // Local: L3 (new work) on top of L2, whose tree GitHub has as U2 after a rewrite.
        let local = "L3 t3\nL2 t2\nL1 t1\n";
        let upstream = "U4 t4\nU2b t2\nU2 t2\nU1 t1\n";
        assert_eq!(newest_shared_tree(local, upstream), Some(("L2".into(), "U2b".into())));
    }

    #[test]
    fn nothing_shared_means_no_base() {
        assert_eq!(newest_shared_tree("L1 a\n", "U1 b\n"), None);
    }

    #[test]
    fn auto_message_names_the_areas_correctly() {
        let files = vec![file("M", "app/server.py"), file("M", "app/auth.py"), file("M", "frontend/src/app/page.tsx")];
        assert_eq!(auto_message(&files), "Update app, frontend/src/app (3 files)");
    }
}
