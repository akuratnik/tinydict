use crate::config::{self, Config};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
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
    let body = ChatRequest {
        model: cfg.cleanup.model.clone(),
        temperature: 0.0,
        messages: vec![
            ChatMessage {
                role: "system",
                content: build_prompt(cfg),
            },
            ChatMessage {
                role: "user",
                content: transcript.to_string(),
            },
        ],
        provider: provider_prefs(&cfg.cleanup),
        reasoning: reasoning_prefs(&cfg.cleanup),
        chat_template_kwargs: thinking_kwargs(&cfg.cleanup),
    };

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(cfg.cleanup.timeout_secs.max(1)))
        .build()
        .context("http client")?;

    let resp = client
        .post(&url)
        .bearer_auth(&key)
        .header(
            "HTTP-Referer",
            "https://github.com/akuratnik/tinydict",
        )
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

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    temperature: f32,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider: Option<ProviderPrefs>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning: Option<ReasoningPrefs>,
    #[serde(skip_serializing_if = "Option::is_none")]
    chat_template_kwargs: Option<ChatTemplateKwargs>,
}

#[derive(Serialize)]
struct ProviderPrefs {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    only: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    quantizations: Vec<String>,
    allow_fallbacks: bool,
}

fn provider_prefs(cfg: &config::Cleanup) -> Option<ProviderPrefs> {
    if cfg.provider_only.is_empty() && cfg.quantizations.is_empty() {
        return None;
    }
    Some(ProviderPrefs {
        only: cfg.provider_only.clone(),
        quantizations: cfg.quantizations.clone(),
        allow_fallbacks: cfg.allow_fallbacks,
    })
}

fn reasoning_prefs(cfg: &config::Cleanup) -> Option<ReasoningPrefs> {
    match cfg.thinking {
        Some(false) => Some(ReasoningPrefs { enabled: false }),
        _ => None,
    }
}

fn thinking_kwargs(cfg: &config::Cleanup) -> Option<ChatTemplateKwargs> {
    match cfg.thinking {
        Some(false) => Some(ChatTemplateKwargs {
            enable_thinking: false,
        }),
        _ => None,
    }
}

#[derive(Serialize)]
struct ReasoningPrefs {
    enabled: bool,
}

#[derive(Serialize)]
struct ChatTemplateKwargs {
    enable_thinking: bool,
}

#[derive(Serialize)]
struct ChatMessage {
    role: &'static str,
    content: String,
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
}
