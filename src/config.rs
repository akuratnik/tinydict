use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;

pub const APP: &str = "tinydict";
#[cfg(target_os = "linux")]
pub const DEFAULT_BINDING: &str = "<Control><Super>x";
pub const HISTORY_KEEP: usize = 500;
pub const ARMING_TIMEOUT_SECS: u64 = 20;
pub const FINALIZE_TIMEOUT_SECS: u64 = 15;
pub const AUDIO_CAP_BYTES: usize = 10_000_000;
pub const AUDIO_CHUNK: usize = 4096;
pub const SAMPLE_RATE: u32 = 16_000;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub speechmatics: Speechmatics,
    #[serde(default)]
    pub vocab: Vocab,
    #[serde(default)]
    pub replacements: BTreeMap<String, String>,
    #[serde(default)]
    pub cleanup: Cleanup,
    #[serde(default)]
    pub daemon: Daemon,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Speechmatics {
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub key_file: Option<PathBuf>,
    #[serde(default = "default_url")]
    pub url: String,
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default = "default_max_delay")]
    pub max_delay: f32,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Vocab {
    #[serde(default)]
    pub words: Vec<VocabEntry>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum VocabEntry {
    Word(String),
    Detailed {
        content: String,
        #[serde(default)]
        sounds_like: Vec<String>,
    },
}

#[derive(Debug, Clone, Deserialize)]
pub struct Cleanup {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_cleanup_model")]
    pub model: String,
    #[serde(default = "default_api_base")]
    pub api_base: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub key_file: Option<PathBuf>,
    #[serde(default)]
    pub provider_only: Vec<String>,
    #[serde(default)]
    pub quantizations: Vec<String>,
    #[serde(default = "default_true")]
    pub allow_fallbacks: bool,
    /// `false` sends extras to disable model thinking. Omit for plain OpenAI-compatible APIs.
    #[serde(default)]
    pub thinking: Option<bool>,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    #[serde(default = "default_prompt")]
    pub prompt: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Daemon {
    #[serde(default = "default_idle")]
    pub idle_exit_secs: u64,
    #[serde(default = "default_true")]
    pub history: bool,
    #[serde(default = "default_true")]
    pub notification_preview: bool,
    #[serde(default)]
    pub wrap_tag: String,
}

impl Default for Speechmatics {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            key_file: None,
            url: default_url(),
            language: default_language(),
            model: default_model(),
            max_delay: default_max_delay(),
        }
    }
}

impl Default for Cleanup {
    fn default() -> Self {
        Self {
            enabled: false,
            model: default_cleanup_model(),
            api_base: default_api_base(),
            api_key: String::new(),
            key_file: None,
            provider_only: Vec::new(),
            quantizations: Vec::new(),
            allow_fallbacks: true,
            thinking: None,
            timeout_secs: default_timeout(),
            prompt: default_prompt(),
        }
    }
}

impl Default for Daemon {
    fn default() -> Self {
        Self {
            idle_exit_secs: default_idle(),
            history: true,
            notification_preview: true,
            wrap_tag: String::new(),
        }
    }
}

fn default_url() -> String {
    "wss://eu.rt.speechmatics.com/v2/".into()
}
fn default_language() -> String {
    "en".into()
}
fn default_model() -> String {
    "enhanced".into()
}
fn default_max_delay() -> f32 {
    1.5
}
fn default_true() -> bool {
    true
}
fn default_cleanup_model() -> String {
    "gpt-4o-mini".into()
}
fn default_api_base() -> String {
    "https://api.openai.com/v1".into()
}
fn default_timeout() -> u64 {
    30
}
pub fn default_prompt() -> String {
    "You are cleaning up a speech-to-text transcript. \
Fix punctuation, capitalization, and obvious recognition errors. \
Do not add information that was not spoken. \
Do not wrap the result in quotes or code fences. \
Output only the cleaned transcript."
        .into()
}
fn default_idle() -> u64 {
    90
}

impl VocabEntry {
    pub fn content(&self) -> &str {
        match self {
            VocabEntry::Word(w) => w,
            VocabEntry::Detailed { content, .. } => content,
        }
    }
}

pub fn load() -> Result<Config> {
    let path = config_path();
    if !path.exists() {
        bail!("no config at {} — run `tinydict setup`", path.display());
    }
    let text = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}

pub fn api_key(cfg: &Speechmatics) -> Result<String> {
    resolve_secret(&cfg.api_key, &cfg.key_file, "speechmatics.api_key")
}

pub fn cleanup_api_key(cfg: &Cleanup) -> Result<String> {
    resolve_secret(&cfg.api_key, &cfg.key_file, "cleanup.api_key")
}

fn resolve_secret(inline: &str, key_file: &Option<PathBuf>, field: &str) -> Result<String> {
    if let Some(path) = key_file {
        let mut s = String::new();
        fs::File::open(path)
            .with_context(|| format!("opening key_file {}", path.display()))?
            .read_to_string(&mut s)?;
        let key = s.trim().to_string();
        if key.is_empty() {
            bail!("key_file {} is empty", path.display());
        }
        return Ok(key);
    }
    let inline = inline.trim();
    if !inline.is_empty() {
        return Ok(inline.to_string());
    }
    bail!("set {field} in {}", config_path().display())
}

pub fn ws_url(cfg: &Speechmatics) -> String {
    let base = cfg.url.trim_end_matches('/');
    if base.ends_with(&format!("/{}", cfg.language)) {
        base.to_string()
    } else {
        format!("{base}/{}", cfg.language)
    }
}

pub fn apply_replacements(text: &str, map: &BTreeMap<String, String>) -> String {
    let mut items: Vec<_> = map.iter().collect();
    items.sort_by_key(|(k, _)| std::cmp::Reverse(k.len()));
    let mut out = text.to_string();
    for (from, to) in items {
        out = out.replace(from, to);
    }
    out
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .expect("HOME is required")
}

pub fn config_dir() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config").join(APP)
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

pub fn open_settings() -> Result<()> {
    let path = config_path();
    if !path.exists() {
        write_0600(&path, CONFIG_TEMPLATE)?;
    }
    let (opener, args) = OPENER;
    let status = Command::new(opener)
        .args(args)
        .arg(&path)
        .status()
        .with_context(|| format!("running {opener}"))?;
    if !status.success() {
        bail!("{opener} failed for {}", path.display());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
const OPENER: (&str, &[&str]) = ("xdg-open", &[]);
// `open` alone has no .toml association; -t uses the default text editor.
#[cfg(target_os = "macos")]
const OPENER: (&str, &[&str]) = ("open", &["-t"]);

pub fn data_dir() -> PathBuf {
    xdg("XDG_DATA_HOME", ".local/share").join(APP)
}

#[cfg(target_os = "linux")]
pub fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp").join(format!("runtime-{}", uid())))
}

// macOS has no per-user runtime dir, and launchd needs a fixed socket path in
// the plist, so the socket lives next to the history file.
#[cfg(target_os = "macos")]
pub fn runtime_dir() -> PathBuf {
    data_dir()
}

pub fn socket_path() -> PathBuf {
    runtime_dir().join(format!("{APP}.sock"))
}

#[cfg(target_os = "linux")]
pub fn clip_path() -> PathBuf {
    runtime_dir().join(format!("{APP}-clip.txt"))
}

pub fn history_path() -> PathBuf {
    data_dir().join("history.jsonl")
}

pub fn bin_path() -> PathBuf {
    home().join(".local/bin").join(APP)
}

#[cfg(target_os = "linux")]
pub fn systemd_user_dir() -> PathBuf {
    config_dir()
        .parent()
        .unwrap_or_else(|| Path::new("/"))
        .join("systemd/user")
}

fn xdg(var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(var)
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(fallback))
}

pub fn uid() -> u32 {
    unsafe { libc::getuid() }
}

pub fn mkdir_0700(path: &Path) -> Result<()> {
    if path.exists() {
        return Ok(());
    }
    fs::DirBuilder::new()
        .mode(0o700)
        .recursive(true)
        .create(path)
        .with_context(|| format!("creating {}", path.display()))
}

pub fn write_0600(path: &Path, contents: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        mkdir_0700(parent)?;
    }
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("writing {}", path.display()))?;
    f.write_all(contents.as_bytes())?;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    Ok(())
}

pub const CONFIG_TEMPLATE: &str = r#"# tinydict
[speechmatics]
api_key = ""
url = "wss://eu.rt.speechmatics.com/v2/"
language = "en"
model = "enhanced"
max_delay = 1.5

[vocab]
words = [ "Speechmatics", { content = "ksni", sounds_like = ["kay snee"] } ]

[replacements]
# "gnome" = "GNOME"

[cleanup]
enabled = false
api_key = ""
timeout_secs = 30
# Recommended (OpenRouter):
# api_base = "https://openrouter.ai/api/v1"
# model = "google/gemma-4-31b-it:nitro"
# provider_only = ["cerebras"]
# quantizations = ["fp16"]
# allow_fallbacks = false
# thinking = false
prompt = """
You are cleaning up a speech-to-text transcript. Fix punctuation, capitalization, and obvious recognition errors. Do not add information that was not spoken. Do not wrap the result in quotes or code fences. Output only the cleaned transcript.
"""

[daemon]
idle_exit_secs = 90
history = true
notification_preview = true
# wrap_tag = "voice_transcript"
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacements_longer_first() {
        let mut map = BTreeMap::new();
        map.insert("gnome".into(), "GNOME".into());
        map.insert("gnome shell".into(), "GNOME Shell".into());
        assert_eq!(
            apply_replacements("I use gnome shell on gnome", &map),
            "I use GNOME Shell on GNOME"
        );
    }

    #[test]
    fn ws_url_appends_language() {
        let sm = Speechmatics::default();
        assert_eq!(ws_url(&sm), "wss://eu.rt.speechmatics.com/v2/en");
        let mut pinned = sm.clone();
        pinned.url = "wss://eu.rt.speechmatics.com/v2/en".into();
        assert_eq!(ws_url(&pinned), "wss://eu.rt.speechmatics.com/v2/en");
    }

    #[test]
    fn template_parses_as_config() {
        let cfg: Config = toml::from_str(CONFIG_TEMPLATE).unwrap();
        assert_eq!(cfg.speechmatics.model, "enhanced");
        assert_eq!(cfg.speechmatics.api_key, "");
        assert_eq!(cfg.cleanup.api_key, "");
        assert!(!cfg.cleanup.enabled);
        assert_eq!(cfg.cleanup.model, "gpt-4o-mini");
        assert_eq!(cfg.cleanup.api_base, "https://api.openai.com/v1");
        assert!(cfg.cleanup.provider_only.is_empty());
        assert!(cfg.cleanup.quantizations.is_empty());
        assert_eq!(cfg.cleanup.thinking, None);
        assert_eq!(cfg.cleanup.prompt.trim(), default_prompt());
        assert_eq!(cfg.daemon.idle_exit_secs, default_idle());
        assert!(cfg.daemon.wrap_tag.is_empty());
    }
}
