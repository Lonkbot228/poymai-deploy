//! System OpenSSH (built into Windows 10+ and macOS) with automatic LAN / external route choice.

use crate::config::{Config, Profile, Route};
use crate::proc::command;
use anyhow::{bail, Result};
use std::net::ToSocketAddrs;
use std::path::Path;
use std::time::Duration;
use tokio::process::Command;

fn reachable(host: &str, port: u16, timeout: Duration) -> bool {
    let Ok(addrs) = (host, port).to_socket_addrs() else { return false };
    addrs
        .into_iter()
        .any(|addr| std::net::TcpStream::connect_timeout(&addr, timeout).is_ok())
}

fn split_jump(jump: &str) -> (String, u16) {
    let host = jump.rsplit('@').next().unwrap_or(jump);
    match host.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse().unwrap_or(22)),
        None => (host.to_string(), 22),
    }
}

/// The first route whose entry point answers on TCP within 1.5 s.
pub async fn pick_route(profile: &Profile) -> Result<Route> {
    let routes = profile.routes.clone();
    tokio::task::spawn_blocking(move || {
        for route in &routes {
            let (host, port) = match &route.jump {
                Some(j) => split_jump(j),
                None => (route.host.clone(), route.port),
            };
            if reachable(&host, port, Duration::from_millis(1500)) {
                return Ok(route.clone());
            }
        }
        bail!("Сервер недоступен ни по одному адресу: {}",
            routes.iter().map(|r| format!("{} ({})", r.name, r.jump.clone().unwrap_or_else(|| r.host.clone()))).collect::<Vec<_>>().join(", "))
    })
    .await?
}

pub fn ssh_command(cfg: &Config, route: &Route, remote: &str) -> Command {
    let mut cmd = command("ssh");
    cmd.args([
        "-o", "BatchMode=yes",
        "-o", "ConnectTimeout=8",
        "-o", "ServerAliveInterval=15",
        "-o", "ServerAliveCountMax=4",
        "-o", "StrictHostKeyChecking=accept-new",
    ]);
    if !cfg.ssh_key.is_empty() && Path::new(&cfg.ssh_key).exists() {
        cmd.args(["-i", &cfg.ssh_key, "-o", "IdentitiesOnly=yes"]);
    }
    if let Some(jump) = &route.jump {
        cmd.args(["-J", jump]);
    }
    cmd.args(["-p", &route.port.to_string()]);
    cmd.arg(format!("{}@{}", route.user, route.host));
    cmd.arg(remote);
    cmd
}

pub async fn exec(cfg: &Config, route: &Route, remote: &str) -> Result<String> {
    let out = ssh_command(cfg, route, remote).output().await?;
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        bail!("ssh {}: {}", route.host, if stderr.is_empty() { &stdout } else { &stderr });
    }
    Ok(stdout)
}

/// Single-quote for a POSIX shell.
pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Remote shell snippet that runs scripts/deploy/remote-deploy.sh *from the given commit*.
pub fn release_script(remote_dir: &str, sha: &str, args: &str, actor: &str, machine: &str) -> String {
    format!(
        "cd {dir} && git fetch -q origin --prune && f=$(mktemp) && \
         {{ git show {sha}:scripts/deploy/remote-deploy.sh > \"$f\" 2>/dev/null || \
         {{ echo '::error 11|Коммит {short} не найден на GitHub или в нём нет scripts/deploy/remote-deploy.sh'; rm -f \"$f\"; exit 11; }}; }} && \
         DEPLOY_ACTOR={actor} DEPLOY_MACHINE={machine} bash \"$f\" {args}; rc=$?; rm -f \"$f\"; exit $rc",
        dir = sh_quote(remote_dir),
        sha = sha,
        short = &sha[..sha.len().min(8)],
        actor = sh_quote(actor),
        machine = sh_quote(machine),
        args = args,
    )
}

/// Reads the server's own copy of the script (status / releases need no particular commit).
pub fn server_script(remote_dir: &str, args: &str) -> String {
    format!(
        "cd {dir} && if [ -f scripts/deploy/remote-deploy.sh ]; then bash scripts/deploy/remote-deploy.sh {args}; \
         else echo '{{\"head\":\"'$(git rev-parse HEAD)'\",\"drift\":'$(git status --porcelain | wc -l)',\"busy\":\"\",\"current\":null}}'; fi",
        dir = sh_quote(remote_dir),
        args = args
    )
}
