use crate::config;
use anyhow::{bail, Context, Result};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as platform;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as platform;

pub fn run(binding: Option<&str>) -> Result<()> {
    platform::check_deps()?;
    install_self()?;
    config::mkdir_0700(&config::config_dir())?;
    config::mkdir_0700(&config::data_dir())?;

    let cfg = config::config_path();
    if cfg.exists() {
        println!("config exists {}", cfg.display());
    } else {
        config::write_0600(&cfg, config::CONFIG_TEMPLATE)?;
        println!("wrote {}", cfg.display());
    }

    platform::install(binding)?;
    println!(
        "setup complete. fill in api_key fields in {}",
        cfg.display()
    );
    Ok(())
}

fn install_self() -> Result<()> {
    let dest = config::bin_path();
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    let src = std::env::current_exe().context("current_exe")?;
    if src.canonicalize().ok() == dest.canonicalize().ok() {
        return Ok(());
    }
    let _ = fs::remove_file(&dest);
    fs::copy(&src, &dest).with_context(|| format!("copy to {}", dest.display()))?;
    let _ = fs::set_permissions(&dest, fs::Permissions::from_mode(0o755));
    println!("installed {}", dest.display());
    Ok(())
}

fn run_cmd(args: &[&str]) -> Result<()> {
    let status = Command::new(args[0])
        .args(&args[1..])
        .status()
        .with_context(|| format!("running {}", args[0]))?;
    if !status.success() {
        bail!("{} failed", args.join(" "));
    }
    Ok(())
}
