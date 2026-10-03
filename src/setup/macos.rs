use super::run_cmd;
use crate::{config, daemon};
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// launchd agents get only this PATH and none of the shell's environment.
const BASE_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

pub fn check_deps() -> Result<()> {
    rec_dir().map(drop)
}

pub fn install(binding: Option<&str>) -> Result<()> {
    if binding.is_some() {
        println!("note: --binding is ignored on macOS");
    }
    // Forward what the agents can't see but paths depend on, so setup, the
    // daemon and the hotkey agent agree on config and socket locations.
    let mut env = vec![("PATH", format!("{}:{BASE_PATH}", rec_dir()?.display()))];
    for key in ["XDG_CONFIG_HOME", "XDG_DATA_HOME"] {
        if let Ok(v) = std::env::var(key) {
            env.push((key, v));
        }
    }
    let env: String = env
        .iter()
        .map(|(k, v)| format!("    <key>{k}</key><string>{}</string>\n", xml(v)))
        .collect();
    let env = format!("  <key>EnvironmentVariables</key>\n  <dict>\n{env}  </dict>\n");

    let sock = xml(&config::socket_path().display().to_string());
    let daemon = format!(
        r#"  <key>Sockets</key>
  <dict>
    <key>{}</key>
    <dict>
      <key>SockPathName</key><string>{sock}</string>
      <key>SockPathMode</key><integer>384</integer><!-- 0600 -->
    </dict>
  </dict>
  <key>Umask</key><integer>63</integer><!-- 077 -->
  <!-- respawn right away even if idle_exit_secs is under launchd's 10 s default -->
  <key>ThrottleInterval</key><integer>1</integer>
"#,
        daemon::LAUNCHD_SOCKET
    );
    agent("daemon", &env, &daemon)?;
    agent(
        "hotkey",
        &env,
        "  <key>RunAtLoad</key><true/>\n  <key>KeepAlive</key><true/>\n",
    )?;
    println!("hotkey Ctrl+Cmd+X → tinydict toggle");
    println!("macOS asks for microphone access on the first recording");
    println!("notifications come from Script Editor; allow it in System Settings → Notifications");
    Ok(())
}

/// Directory of sox's `rec` on the user's PATH.
fn rec_dir() -> Result<PathBuf> {
    let out = Command::new("sh")
        .args(["-c", "command -v rec"])
        .output()
        .context("running sh")?;
    let path = String::from_utf8_lossy(&out.stdout);
    match Path::new(path.trim()).parent() {
        Some(dir) if out.status.success() => Ok(dir.to_path_buf()),
        _ => bail!("missing rec — install with: brew install sox"),
    }
}

/// Writes `~/Library/LaunchAgents/tinydict.<cmd>.plist` running `tinydict <cmd>`
/// and (re)loads it, so a reinstalled binary or changed plist takes effect.
fn agent(cmd: &str, env: &str, extra: &str) -> Result<()> {
    let label = format!("tinydict.{cmd}");
    let bin = xml(&config::bin_path().display().to_string());
    let log = xml(&config::data_dir()
        .join("tinydict.log")
        .display()
        .to_string());
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{label}</string>
  <key>ProgramArguments</key>
  <array><string>{bin}</string><string>{cmd}</string></array>
  <key>ProcessType</key><string>Interactive</string>
  <key>StandardErrorPath</key><string>{log}</string>
{env}{extra}</dict>
</plist>
"#
    );

    let dir = config::home().join("Library/LaunchAgents");
    fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{label}.plist"));
    fs::write(&path, plist).with_context(|| format!("writing {}", path.display()))?;
    let path = path.to_str().context("non-UTF-8 LaunchAgents path")?;
    let domain = format!("gui/{}", config::uid());

    let _ = Command::new("launchctl")
        .args(["bootout", &format!("{domain}/{label}")])
        .stderr(Stdio::null())
        .status();
    // bootout can return before the old job is gone; bootstrap fails (EIO) until it is.
    for _ in 0..20 {
        let ok = Command::new("launchctl")
            .args(["bootstrap", &domain, path])
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if ok {
            println!("loaded {path}");
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    run_cmd(&["launchctl", "bootstrap", &domain, path])
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
