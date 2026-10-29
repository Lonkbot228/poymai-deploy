//! Per-machine settings: where each project lives on this computer and how to reach its server.
//! Stored in ~/.poymai-deploy/config.json so Windows and Mac can have different repo paths.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Route {
    pub name: String,
    pub user: String,
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    /// `user@host[:port]` used as ssh -J (e.g. reach the Telegram box through the main server).
    #[serde(default)]
    pub jump: Option<String>,
}

fn default_port() -> u16 {
    22
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub repo_path: String,
    #[serde(default = "default_branch")]
    pub branch: String,
    pub remote_dir: String,
    /// Tried in order; the first reachable one wins.
    pub routes: Vec<Route>,
    /// Commands run in the repo before committing (empty = skip).
    #[serde(default)]
    pub checks: Vec<String>,
    #[serde(default = "yes")]
    pub auto_pull: bool,
    /// Remote command (run in `remote_dir`) that prints the site's deploy-window JSON.
    /// `None` = not set yet (filled with the default for PoymAI), "" = switched off.
    #[serde(default)]
    pub site_probe: Option<String>,
}

pub const POYMAI_PROBE: &str = "if docker compose version >/dev/null 2>&1; then c='docker compose'; else c=docker-compose; fi; $c exec -T backend python3 -m app.deploy_window";

fn default_branch() -> String {
    "main".into()
}
fn yes() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    pub profiles: Vec<Profile>,
    #[serde(default = "default_poll")]
    pub poll_seconds: u64,
    /// Private key for the servers; empty = ssh defaults / agent.
    #[serde(default)]
    pub ssh_key: String,
    /// Before a scheduled deploy, show "Обновление через N мин" in the dashboard (0 = never).
    #[serde(default = "default_announce")]
    pub announce_minutes: u32,
    /// Ask before a manual deploy when the site says people would be interrupted.
    #[serde(default = "yes")]
    pub guard_manual: bool,
    /// How often the site is asked about a deploy window, seconds.
    #[serde(default = "default_site_poll")]
    pub site_poll_seconds: u64,
}

fn default_announce() -> u32 {
    5
}
fn default_site_poll() -> u64 {
    120
}

fn default_poll() -> u64 {
    60
}

pub fn config_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join(".poymai-deploy")
}

fn config_path() -> PathBuf {
    config_dir().join("config.json")
}

fn default_code_dir() -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(r"C:\code")
    } else {
        dirs::home_dir().unwrap_or_default().join("code")
    }
}

impl Default for Config {
    fn default() -> Self {
        let code = default_code_dir();
        let key = dirs::home_dir()
            .unwrap_or_default()
            .join(".ssh")
            .join("id_ed25519_poymai");
        let main_lan = Route {
            name: "LAN".into(),
            user: "poymai".into(),
            host: "192.168.3.209".into(),
            port: 22,
            jump: None,
        };
        let main_wan = Route {
            name: "Внешний".into(),
            user: "poymai".into(),
            host: "185.33.228.250".into(),
            port: 22,
            jump: None,
        };
        Config {
            poll_seconds: 60,
            announce_minutes: default_announce(),
            guard_manual: true,
            site_poll_seconds: default_site_poll(),
            ssh_key: key.to_string_lossy().into_owned(),
            profiles: vec![
                Profile {
                    id: "main".into(),
                    name: "PoymAI".into(),
                    repo_path: code.join("poymai").to_string_lossy().into_owned(),
                    branch: "main".into(),
                    remote_dir: "/home/poymai/poymai-backend".into(),
                    routes: vec![main_lan, main_wan],
                    checks: vec![],
                    auto_pull: true,
                    site_probe: Some(POYMAI_PROBE.into()),
                },
                Profile {
                    id: "telegram".into(),
                    name: "Telegram-шлюз".into(),
                    repo_path: code.join("poymaitelegram").to_string_lossy().into_owned(),
                    branch: "main".into(),
                    remote_dir: "/opt/telegram-bridge".into(),
                    routes: vec![
                        Route {
                            name: "LAN".into(),
                            user: "root".into(),
                            host: "192.168.3.99".into(),
                            port: 22,
                            jump: None,
                        },
                        Route {
                            name: "Через PoymAI".into(),
                            user: "root".into(),
                            host: "192.168.3.99".into(),
                            port: 22,
                            jump: Some("poymai@185.33.228.250".into()),
                        },
                    ],
                    checks: vec![],
                    auto_pull: true,
                    site_probe: Some(String::new()),
                },
            ],
        }
    }
}

pub fn load() -> Config {
    match std::fs::read_to_string(config_path()) {
        Ok(text) => {
            let mut cfg: Config = serde_json::from_str(&text).unwrap_or_default();
            // Configs written before the site link existed: PoymAI gets the probe.
            for p in cfg.profiles.iter_mut().filter(|p| p.site_probe.is_none()) {
                p.site_probe = Some(if p.id == "main" { POYMAI_PROBE.into() } else { String::new() });
            }
            cfg
        }
        Err(_) => {
            let cfg = Config::default();
            let _ = save(&cfg);
            cfg
        }
    }
}

pub fn save(cfg: &Config) -> anyhow::Result<()> {
    std::fs::create_dir_all(config_dir())?;
    std::fs::write(config_path(), serde_json::to_string_pretty(cfg)?)?;
    Ok(())
}

impl Config {
    pub fn profile(&self, id: &str) -> anyhow::Result<Profile> {
        self.profiles
            .iter()
            .find(|p| p.id == id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Профиль {id} не найден"))
    }
}
