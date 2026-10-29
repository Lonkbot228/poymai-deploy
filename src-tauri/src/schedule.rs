//! Deploys that run later: at a set time, or as soon as the site is quiet.
//!
//! The commit is fixed when the job is created (local work is committed and pushed right away),
//! so a scheduled deploy installs exactly what was reviewed. Jobs live in
//! ~/.poymai-deploy/schedule.json and survive restarts; `decide` is pure and unit-tested.

use crate::config::config_dir;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

/// A time-based job that could not start within this long (computer asleep, app closed) expires.
pub const MISSED_GRACE_MINUTES: i64 = 30;
const KEEP_FINISHED: usize = 40;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Job {
    pub id: String,
    pub profile: String,
    /// "at" = at `run_at`; "quiet" = first moment the site allows, between `run_at` and `deadline`.
    pub mode: String,
    #[serde(default)]
    pub run_at: Option<String>,
    #[serde(default)]
    pub deadline: Option<String>,
    /// Quiet mode: "можно, но кое-кто заметит" is good enough.
    #[serde(default)]
    pub allow_warn: bool,
    /// Quiet mode: deploy at the deadline even if it never got quiet.
    #[serde(default)]
    pub force_at_deadline: bool,
    #[serde(default)]
    pub announce_minutes: u32,
    pub sha: String,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub backup_db: bool,
    pub created_at: String,
    #[serde(default)]
    pub created_on: String,
    /// waiting | announced | running | done | failed | cancelled | expired
    pub status: String,
    /// When the announced deploy starts.
    #[serde(default)]
    pub announced_for: Option<String>,
    #[serde(default)]
    pub result: String,
    #[serde(default)]
    pub finished_at: Option<String>,
}

#[derive(Debug, PartialEq)]
pub enum Action {
    Wait,
    /// Post the dashboard notice and start in `minutes`.
    Announce { minutes: u32, start: DateTime<Utc> },
    /// The site got busy after the notice: take it back and keep waiting.
    Unannounce,
    Run,
    Expire(String),
}

pub fn parse(ts: &Option<String>) -> Option<DateTime<Utc>> {
    ts.as_deref().and_then(|s| DateTime::parse_from_rfc3339(s).ok()).map(|d| d.with_timezone(&Utc))
}

impl Job {
    pub fn is_active(&self) -> bool {
        matches!(self.status.as_str(), "waiting" | "announced" | "running")
    }
}

fn minutes_until(now: DateTime<Utc>, at: DateTime<Utc>) -> u32 {
    let secs = (at - now).num_seconds().max(0);
    ((secs + 59) / 60) as u32
}

/// What a waiting/announced job should do now. `verdict` is the site's level ("ok" | "warn" |
/// "stop"), `None` when the site could not be asked.
pub fn decide(job: &Job, now: DateTime<Utc>, verdict: Option<&str>) -> Action {
    if !matches!(job.status.as_str(), "waiting" | "announced") {
        return Action::Wait;
    }
    let announced = job.status == "announced";
    let announced_for = parse(&job.announced_for);
    match job.mode.as_str() {
        "at" => {
            let Some(target) = parse(&job.run_at) else { return Action::Expire("Не задано время".into()) };
            if now >= target + Duration::minutes(MISSED_GRACE_MINUTES) {
                return Action::Expire("Время прошло, пока приложение не работало".into());
            }
            if now >= target {
                return Action::Run;
            }
            if !announced && job.announce_minutes > 0 && now >= target - Duration::minutes(job.announce_minutes as i64) {
                return Action::Announce { minutes: minutes_until(now, target), start: target };
            }
            Action::Wait
        }
        "quiet" => {
            if let Some(start) = parse(&job.run_at) {
                if now < start {
                    return Action::Wait;
                }
            }
            let past_deadline = parse(&job.deadline).is_some_and(|d| now >= d);
            if announced {
                if verdict == Some("stop") && !(past_deadline && job.force_at_deadline) {
                    return Action::Unannounce;
                }
                return match announced_for {
                    Some(at) if now < at => Action::Wait,
                    _ => Action::Run,
                };
            }
            let acceptable = verdict == Some("ok") || (job.allow_warn && verdict == Some("warn"));
            if !acceptable && past_deadline {
                if !job.force_at_deadline {
                    return Action::Expire("Тихого окна до крайнего срока не нашлось".into());
                }
            } else if !acceptable {
                return Action::Wait;
            }
            if job.announce_minutes > 0 {
                Action::Announce { minutes: job.announce_minutes, start: now + Duration::minutes(job.announce_minutes as i64) }
            } else {
                Action::Run
            }
        }
        other => Action::Expire(format!("Неизвестный режим {other}")),
    }
}

fn path() -> std::path::PathBuf {
    config_dir().join("schedule.json")
}

pub fn load() -> Vec<Job> {
    let mut jobs: Vec<Job> = std::fs::read_to_string(path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    // A deploy cannot still be running after a restart of this app.
    for job in jobs.iter_mut().filter(|j| j.status == "running") {
        job.status = "failed".into();
        job.result = "Приложение закрылось во время деплоя — проверьте историю на сервере".into();
    }
    jobs
}

pub fn save(jobs: &[Job]) -> anyhow::Result<()> {
    let mut keep: Vec<Job> = jobs.iter().filter(|j| j.is_active()).cloned().collect();
    let mut finished: Vec<Job> = jobs.iter().filter(|j| !j.is_active()).cloned().collect();
    finished.sort_by(|a, b| b.finished_at.cmp(&a.finished_at));
    keep.extend(finished.into_iter().take(KEEP_FINISHED));
    std::fs::create_dir_all(config_dir())?;
    std::fs::write(path(), serde_json::to_string_pretty(&keep)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(min: i64) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-23T22:00:00Z").unwrap().with_timezone(&Utc) + Duration::minutes(min)
    }
    fn job(mode: &str) -> Job {
        Job { mode: mode.into(), status: "waiting".into(), announce_minutes: 5, ..Default::default() }
    }

    #[test]
    fn at_job_announces_then_runs_then_expires_when_missed() {
        let mut j = job("at");
        j.run_at = Some(t(0).to_rfc3339());
        assert_eq!(decide(&j, t(-10), None), Action::Wait);
        assert_eq!(decide(&j, t(-5), None), Action::Announce { minutes: 5, start: t(0) });
        j.status = "announced".into();
        assert_eq!(decide(&j, t(-1), Some("stop")), Action::Wait);
        assert_eq!(decide(&j, t(0), Some("stop")), Action::Run);
        assert!(matches!(decide(&j, t(MISSED_GRACE_MINUTES), None), Action::Expire(_)));
    }

    #[test]
    fn at_job_without_notice_just_runs() {
        let mut j = job("at");
        j.announce_minutes = 0;
        j.run_at = Some(t(0).to_rfc3339());
        assert_eq!(decide(&j, t(-1), None), Action::Wait);
        assert_eq!(decide(&j, t(1), None), Action::Run);
    }

    #[test]
    fn quiet_job_waits_for_ok_and_backs_off_when_busy_again() {
        let mut j = job("quiet");
        j.deadline = Some(t(120).to_rfc3339());
        assert_eq!(decide(&j, t(0), Some("stop")), Action::Wait);
        assert_eq!(decide(&j, t(0), Some("warn")), Action::Wait);
        assert_eq!(decide(&j, t(0), None), Action::Wait);
        assert_eq!(decide(&j, t(0), Some("ok")), Action::Announce { minutes: 5, start: t(5) });
        j.status = "announced".into();
        j.announced_for = Some(t(5).to_rfc3339());
        assert_eq!(decide(&j, t(3), Some("ok")), Action::Wait);
        assert_eq!(decide(&j, t(3), Some("stop")), Action::Unannounce);
        assert_eq!(decide(&j, t(5), Some("warn")), Action::Run);
    }

    #[test]
    fn quiet_job_accepts_warn_when_allowed_and_respects_start() {
        let mut j = job("quiet");
        j.allow_warn = true;
        j.announce_minutes = 0;
        j.run_at = Some(t(60).to_rfc3339());
        assert_eq!(decide(&j, t(0), Some("ok")), Action::Wait);
        assert_eq!(decide(&j, t(60), Some("warn")), Action::Run);
    }

    #[test]
    fn quiet_job_deadline_expires_or_forces() {
        let mut j = job("quiet");
        j.deadline = Some(t(0).to_rfc3339());
        assert!(matches!(decide(&j, t(1), Some("stop")), Action::Expire(_)));
        j.force_at_deadline = true;
        assert_eq!(decide(&j, t(1), Some("stop")), Action::Announce { minutes: 5, start: t(6) });
        j.status = "announced".into();
        j.announced_for = Some(t(6).to_rfc3339());
        assert_eq!(decide(&j, t(6), Some("stop")), Action::Run);
    }

    #[test]
    fn finished_jobs_do_nothing() {
        let mut j = job("at");
        j.status = "done".into();
        assert_eq!(decide(&j, t(0), None), Action::Wait);
    }
}
