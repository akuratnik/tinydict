use crate::config::{self, Config};
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::time::Duration;

fn build_prompt(cfg: &Config) -> String {
    let mut p = cfg.cleanup.prompt.trim().to_string();
    if p.is_empty() {
        p = config::default_prompt();
    }
    let words: Vec<&str> = cfg.vocab.words.iter().map(|w| w.content()).collect();
    if !words.is_empty() {
        p.push_str("\n\nPreferred spellings: ");
        p.push_str(&words.join(", "));
        p.push('.');
    }
    p
}

pub async fn run(cfg: &Config, transcript: &str) -> Result<String> {
    let key = config::cleanup_api_key(&cfg.cleanup)?;
    let url = format!(
        "{}/chat/completions",
        cfg.cleanup.api_base.trim_end_matches('/')
    );
    let body = request_body(cfg, transcript)?;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(cfg.cleanup.timeout_secs.max(1)))
        .build()
        .context("http client")?;

    let resp = client
        .post(&url)
        .bearer_auth(&key)
        .header("HTTP-Referer", "https://github.com/akuratnik/tinydict")
        .header("X-Title", "tinydict")
        .json(&body)
        .send()
        .await
        .context("cleanup request")?;

    let status = resp.status();
    let text = resp.text().await.context("cleanup response body")?;
    if !status.is_success() {
        bail!("cleanup HTTP {status}: {}", cap(&text, 200));
    }

    let parsed: ChatResponse =
        serde_json::from_str(&text).context("parsing chat completions response")?;
    let cleaned = parsed
        .choices
        .first()
        .map(|c| c.message.content.as_str())
        .unwrap_or("")
        .to_string();
    validate(transcript, &cleaned).context("cleanup output rejected")
}

fn request_body(cfg: &Config, transcript: &str) -> Result<serde_json::Value> {
    let mut body = serde_json::json!({
        "model": cfg.cleanup.model,
        "temperature": 0.0,
        "messages": [
            { "role": "system", "content": build_prompt(cfg) },
            { "role": "user", "content": transcript },
        ],
    });
    let extra = serde_json::to_value(&cfg.cleanup.extra_body).context("cleanup.extra_body")?;
    if let (Some(body), serde_json::Value::Object(extra)) = (body.as_object_mut(), extra) {
        body.extend(extra);
    }
    Ok(body)
}

#[derive(Deserialize)]
struct ChatResponse {
    #[serde(default)]
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: ChoiceMessage,
}

#[derive(Deserialize)]
struct ChoiceMessage {
    #[serde(default)]
    content: String,
}

fn cap(s: &str, max: usize) -> String {
    let t = s.trim();
    if t.chars().count() <= max {
        t.to_string()
    } else {
        format!("{}…", t.chars().take(max).collect::<String>())
    }
}

pub fn validate(raw: &str, cleaned: &str) -> Result<String> {
    let stripped = strip_fences(cleaned.trim());
    if stripped.is_empty() {
        bail!("empty");
    }
    let in_len = raw.chars().count().max(1) as f32;
    let out_len = stripped.chars().count() as f32;
    let ratio = out_len / in_len;
    if ratio < 0.25 || ratio > 4.0 {
        bail!("length ratio {ratio:.2} outside 0.25–4.0");
    }
    Ok(stripped)
}

fn strip_fences(s: &str) -> String {
    let t = s.trim();
    if let Some(rest) = t.strip_prefix("```") {
        let rest = rest
            .strip_prefix("text")
            .or_else(|| rest.strip_prefix("markdown"))
            .unwrap_or(rest);
        let rest = rest.trim_start_matches('\n');
        if let Some(inner) = rest.strip_suffix("```") {
            return inner.trim().to_string();
        }
    }
    t.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_fences_and_ratio() {
        let raw = "hello world this is a test transcript";
        let out = validate(raw, "```\nHello world. This is a test transcript.\n```").unwrap();
        assert_eq!(out, "Hello world. This is a test transcript.");
        assert!(validate(raw, "").is_err());
        assert!(validate(raw, "x").is_err());
    }

    #[test]
    fn extra_body_merges_and_overrides() {
        let mut cfg = Config::default();
        cfg.cleanup.extra_body =
            toml::from_str("reasoning_effort = \"none\"\ntemperature = 0.7").unwrap();
        let body = request_body(&cfg, "hi").unwrap();
        assert_eq!(body["reasoning_effort"], "none");
        assert_eq!(body["temperature"], 0.7);
        assert_eq!(body["messages"][1]["content"], "hi");
    }
}
