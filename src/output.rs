use crate::config::{self, Config};
use anyhow::{Context, Result};
use serde_json::json;
use std::fs;
use std::io::{BufRead, BufReader};
use std::os::unix::fs::OpenOptionsExt;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

pub async fn publish(cfg: &Config, raw: &str, text: &str, note: Option<&str>) -> Result<()> {
    if cfg.daemon.history {
        if let Err(e) = append_history(raw, text) {
            eprintln!("tinydict: history: {e:#}");
        }
    }
    let mut note = note.map(str::to_string);
    let paste = wrap_for_paste(cfg, text);
    if let Err(e) = clipboard(&paste).await {
        eprintln!("tinydict: clipboard: {e:#}");
        let extra = format!("clipboard failed: {e}");
        note = Some(match note {
            Some(n) => format!("{n}; {extra}"),
            None => extra,
        });
    }
    notify(cfg, text, note.as_deref());
    Ok(())
}

pub fn notify_error(msg: &str) {
    notify_send("tinydict", msg);
}

pub fn notify_cancelled() {
    notify_send("tinydict", "cancelled");
}

pub fn notify_plain(msg: &str) {
    notify_send("tinydict", msg);
}

fn wrap_for_paste(cfg: &Config, text: &str) -> String {
    let tag = cfg.daemon.wrap_tag.trim();
    if tag.is_empty() {
        text.to_string()
    } else {
        format!("<{tag}>{text}</{tag}>")
    }
}

async fn clipboard(text: &str) -> Result<()> {
    let path = config::clip_path();

    let _ = Command::new("systemctl")
        .args(["--user", "stop", "tinydict-clip.service"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;

    config::write_0600(&path, text)?;

    let spec = format!("StandardInput=file:{}", path.display());
    let status = Command::new("systemd-run")
        .args([
            "--user",
            "--collect",
            "--quiet",
            "--unit=tinydict-clip",
            "-p",
            &spec,
            "wl-copy",
            "--foreground",
        ])
        .status()
        .await
        .context("systemd-run wl-copy")?;
    if !status.success() {
        anyhow::bail!("systemd-run wl-copy failed");
    }

    // systemd-run returns before it opens StandardInput. Keep the file until
    // the next copy; wait until the unit is actually running.
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(25)).await;
        if systemctl_ok(["--user", "is-active", "--quiet", "tinydict-clip.service"]).await {
            return Ok(());
        }
        if systemctl_ok(["--user", "is-failed", "--quiet", "tinydict-clip.service"]).await {
            break;
        }
    }
    anyhow::bail!("wl-copy did not stay running (see: journalctl --user -u tinydict-clip)")
}

async fn systemctl_ok<const N: usize>(args: [&str; N]) -> bool {
    Command::new("systemctl")
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

fn notify(cfg: &Config, text: &str, note: Option<&str>) {
    let body = if cfg.daemon.notification_preview {
        preview(text)
    } else {
        let n = text.split_whitespace().count();
        format!("{n} words")
    };
    let body = match note {
        Some(n) => format!("{body}\n{}", cap(n, 160)),
        None => body,
    };
    notify_send("tinydict", &body);
}

fn cap(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max).collect::<String>())
    }
}

fn preview(text: &str) -> String {
    cap(text.trim(), 180)
}

fn notify_send(summary: &str, body: &str) {
    let _ = std::process::Command::new("notify-send")
        .args(["--app-name=tinydict", "--", summary, body])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn append_history(raw: &str, clean: &str) -> Result<()> {
    let path = config::history_path();
    config::mkdir_0700(&config::data_dir())?;
    let rec = json!({
        "ts": timestamp(),
        "raw": raw,
        "clean": clean,
    });
    let mut lines: Vec<String> = if path.exists() {
        let f = fs::File::open(&path)?;
        BufReader::new(f)
            .lines()
            .filter_map(|l| l.ok())
            .filter(|l| !l.is_empty())
            .collect()
    } else {
        Vec::new()
    };
    lines.push(rec.to_string());
    if lines.len() > config::HISTORY_KEEP {
        let skip = lines.len() - config::HISTORY_KEEP;
        lines.drain(..skip);
    }
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&path)
        .with_context(|| format!("writing {}", path.display()))?;
    use std::io::Write;
    for line in lines {
        writeln!(f, "{line}")?;
    }
    Ok(())
}

fn timestamp() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_else(|_| "0".into())
}
