use super::run_cmd;
use crate::config;
use anyhow::{bail, Context, Result};
use std::fs;
use std::process::{Command, Stdio};

const SOCKET_UNIT: &str = r#"[Unit]
Description=tinydict transcription socket

[Socket]
ListenStream=%t/tinydict.sock
SocketMode=0600

[Install]
WantedBy=sockets.target
"#;

const SERVICE_UNIT: &str = r#"[Unit]
Description=tinydict transcription daemon
Requires=tinydict.socket
After=tinydict.socket

[Service]
Type=simple
ExecStart=%h/.local/bin/tinydict daemon
UMask=0077
"#;

const KEY_SCHEMA: &str = "org.gnome.settings-daemon.plugins.media-keys";
const ITEM_SCHEMA: &str = "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding";
const ITEM_PATH: &str =
    "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/tinydict/";

pub fn install(binding: Option<&str>) -> Result<()> {
    write_units()?;
    enable_socket()?;
    bind_session_hotkey(binding)
}

pub fn check_deps() -> Result<()> {
    let required = ["pw-record", "notify-send", "systemd-run", "systemctl"];
    for bin in required {
        if !have(bin) {
            bail!("missing {bin}");
        }
    }
    if !have("wl-copy") {
        println!("warning: wl-copy not found — install with: sudo apt install wl-clipboard");
    }
    if !sni_watcher() {
        println!("warning: StatusNotifierWatcher not on the bus — tray icon will be skipped");
    }
    Ok(())
}

fn have(bin: &str) -> bool {
    Command::new("sh")
        .args(["-c", "command -v \"$1\" >/dev/null", "--", bin])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn sni_watcher() -> bool {
    Command::new("busctl")
        .args(["--user", "status", "org.kde.StatusNotifierWatcher"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn write_units() -> Result<()> {
    let dir = config::systemd_user_dir();
    fs::create_dir_all(&dir)?;
    fs::write(dir.join("tinydict.socket"), SOCKET_UNIT)?;
    fs::write(dir.join("tinydict.service"), SERVICE_UNIT)?;
    println!("wrote systemd user units in {}", dir.display());
    Ok(())
}

fn enable_socket() -> Result<()> {
    run_cmd(&["systemctl", "--user", "daemon-reload"])?;
    run_cmd(&["systemctl", "--user", "enable", "--now", "tinydict.socket"])?;
    println!("enabled tinydict.socket");
    Ok(())
}

fn bind_session_hotkey(binding: Option<&str>) -> Result<()> {
    let cmd = format!("{} toggle", config::bin_path().display());
    if hyprland() {
        bind_hyprland(&cmd)?;
        return Ok(());
    }
    if have("gsettings") {
        bind_gnome(binding.unwrap_or(config::DEFAULT_BINDING), &cmd)?;
        return Ok(());
    }
    println!("bind a hotkey to: {cmd}");
    println!("default chord is Ctrl+Super+X");
    Ok(())
}

fn hyprland() -> bool {
    std::env::var("HYPRLAND_INSTANCE_SIGNATURE").is_ok()
        || desktop().to_lowercase().contains("hyprland")
}

fn desktop() -> String {
    std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default()
}

fn hypr_dir() -> std::path::PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| config::home().join(".config"))
        .join("hypr")
}

fn bind_hyprland(cmd: &str) -> Result<()> {
    let lua = hypr_dir().join("bindings.lua");
    let conf = hypr_dir().join("hyprland.conf");
    if lua.exists() {
        let block = format!(
            "\n-- tinydict (takes Voxtype's Super+Ctrl+X if that was bound)\nhl.unbind(\"SUPER + CTRL + X\")\no.bind(\"SUPER + CTRL + X\", \"tinydict\", \"{cmd}\")\n"
        );
        append_once(&lua, "-- tinydict", &block)?;
        println!("hotkey Super+Ctrl+X → {cmd}");
        println!("wrote {}", lua.display());
        println!("Omarchy: this is Voxtype's dictation chord. Reload Hyprland. F9 PTT stays with Voxtype if installed.");
        return Ok(());
    }
    if conf.exists() {
        let block = format!("\n# tinydict\nbind = SUPER CTRL, X, exec, {cmd}\n");
        append_once(&conf, "# tinydict", &block)?;
        println!("hotkey Super+Ctrl+X → {cmd}");
        println!("wrote {}", conf.display());
        println!("reload Hyprland to apply");
        return Ok(());
    }
    println!("bind a hotkey to: {cmd}");
    println!("Hyprland:  bind = SUPER CTRL, X, exec, {cmd}");
    println!("Omarchy (~/.config/hypr/bindings.lua):");
    println!("  hl.unbind(\"SUPER + CTRL + X\")");
    println!("  o.bind(\"SUPER + CTRL + X\", \"tinydict\", \"{cmd}\")");
    Ok(())
}

fn append_once(path: &std::path::Path, marker: &str, block: &str) -> Result<()> {
    let existing = fs::read_to_string(path).unwrap_or_default();
    if existing.contains(marker) {
        println!("hotkey already in {}", path.display());
        return Ok(());
    }
    let mut f = fs::OpenOptions::new()
        .append(true)
        .open(path)
        .with_context(|| format!("appending {}", path.display()))?;
    use std::io::Write;
    f.write_all(block.as_bytes())?;
    Ok(())
}

fn bind_gnome(binding: &str, command: &str) -> Result<()> {
    gset_item("name", "tinydict")?;
    gset_item("command", &command)?;
    gset_item("binding", binding)?;

    let current = gget(&[KEY_SCHEMA, "custom-keybindings"]);
    let mut paths = parse_gsettings_list(&current);
    if !paths.iter().any(|p| p == ITEM_PATH) {
        paths.push(ITEM_PATH.to_string());
        let encoded = format!(
            "[{}]",
            paths
                .iter()
                .map(|p| format!("'{p}'"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        run_cmd(&[
            "gsettings",
            "set",
            KEY_SCHEMA,
            "custom-keybindings",
            &encoded,
        ])?;
    }
    println!("hotkey {binding} → {command}");
    Ok(())
}

fn gset_item(key: &str, value: &str) -> Result<()> {
    run_cmd(&[
        "gsettings",
        "set",
        &format!("{ITEM_SCHEMA}:{ITEM_PATH}"),
        key,
        value,
    ])
}

fn gget(args: &[&str]) -> String {
    let out = Command::new("gsettings")
        .arg("get")
        .args(args)
        .output()
        .ok();
    out.and_then(|o| {
        if o.status.success() {
            Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
        } else {
            None
        }
    })
    .unwrap_or_default()
}

fn parse_gsettings_list(s: &str) -> Vec<String> {
    s.split('\'')
        .skip(1)
        .step_by(2)
        .filter(|p| !p.is_empty())
        .map(|p| p.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_empty_and_list() {
        assert!(parse_gsettings_list("@as []").is_empty());
        assert_eq!(parse_gsettings_list("['/a/', '/b/']"), vec!["/a/", "/b/"]);
    }
}
