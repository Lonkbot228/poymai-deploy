//! Child processes without console windows, with optional line streaming.

use anyhow::{bail, Result};
use std::path::Path;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;

pub fn command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.stdin(Stdio::null());
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    cmd
}

/// GUI apps on macOS start with a minimal PATH; make Homebrew git/ssh visible.
pub fn fix_path() {
    #[cfg(target_os = "macos")]
    {
        let path = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", format!("/opt/homebrew/bin:/usr/local/bin:{path}"));
    }
}

pub struct Out {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Out {
    pub fn ok(&self) -> bool {
        self.code == 0
    }
    pub fn text(&self) -> String {
        let mut s = self.stdout.trim().to_string();
        if !self.stderr.trim().is_empty() {
            if !s.is_empty() {
                s.push('\n');
            }
            s.push_str(self.stderr.trim());
        }
        s
    }
}

pub async fn run(program: &str, args: &[&str], cwd: Option<&Path>) -> Result<Out> {
    let mut cmd = command(program);
    cmd.args(args);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let out = cmd.output().await.map_err(|e| anyhow::anyhow!("Не удалось запустить {program}: {e}"))?;
    Ok(Out {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}

pub async fn run_ok(program: &str, args: &[&str], cwd: Option<&Path>) -> Result<String> {
    let out = run(program, args, cwd).await?;
    if !out.ok() {
        bail!("{} {}: {}", program, args.join(" "), out.text());
    }
    Ok(out.stdout.trim().to_string())
}

/// Runs a command and hands every stdout/stderr line to `on_line`. Returns the exit code.
pub async fn stream<F: FnMut(String) + Send>(mut cmd: Command, mut on_line: F) -> Result<i32> {
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    pump(child.stdout.take().unwrap(), tx.clone());
    pump(child.stderr.take().unwrap(), tx);
    while let Some(line) = rx.recv().await {
        on_line(strip_ansi(&line));
    }
    let status = child.wait().await?;
    Ok(status.code().unwrap_or(-1))
}

fn pump<R: AsyncRead + Unpin + Send + 'static>(reader: R, tx: tokio::sync::mpsc::UnboundedSender<String>) {
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let _ = tx.send(line);
        }
    });
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            continue;
        }
        if c != '\r' && c != '\0' {
            out.push(c);
        }
    }
    out
}
