use crate::config::{self, Speechmatics, VocabEntry};
use anyhow::{bail, Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

pub struct Session {
    sink: futures_util::stream::SplitSink<Ws, Message>,
    seq: u64,
    started: mpsc::Receiver<()>,
    done: mpsc::Receiver<std::result::Result<(), String>>,
    text: Arc<Mutex<String>>,
    reader: tokio::task::JoinHandle<()>,
}

#[derive(Deserialize)]
struct ServerMsg {
    message: String,
    #[serde(default)]
    metadata: Option<Meta>,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Deserialize)]
struct Meta {
    #[serde(default)]
    transcript: Option<String>,
}

pub async fn connect(sm: &Speechmatics, vocab: &[VocabEntry]) -> Result<Session> {
    let key = config::api_key(sm)?;
    let url = config::ws_url(sm);
    let mut req = url
        .as_str()
        .into_client_request()
        .with_context(|| format!("invalid speechmatics url {url}"))?;
    req.headers_mut().insert(
        "Authorization",
        format!("Bearer {key}")
            .parse()
            .context("api key is not a valid header value")?,
    );

    let (ws, _) = tokio_tungstenite::connect_async(req)
        .await
        .context("connecting to Speechmatics")?;
    let (mut sink, mut stream) = ws.split();

    let start = json!({
        "message": "StartRecognition",
        "audio_format": {
            "type": "raw",
            "encoding": "pcm_s16le",
            "sample_rate": config::SAMPLE_RATE,
        },
        "transcription_config": {
            "language": sm.language,
            "model": sm.model,
            "max_delay": sm.max_delay,
            "additional_vocab": vocab_json(vocab),
        }
    });
    sink.send(Message::text(start.to_string()))
        .await
        .context("sending StartRecognition")?;

    let (started_tx, started) = mpsc::channel(1);
    let (done_tx, done) = mpsc::channel(1);
    let text = Arc::new(Mutex::new(String::new()));
    let text_r = text.clone();

    let reader = tokio::spawn(async move {
        while let Some(msg) = stream.next().await {
            let Ok(msg) = msg else {
                let _ = done_tx
                    .send(Err("speechmatics websocket error".into()))
                    .await;
                return;
            };
            let Message::Text(body) = msg else { continue };
            let Ok(parsed) = serde_json::from_str::<ServerMsg>(body.as_ref()) else {
                continue;
            };
            match parsed.message.as_str() {
                "RecognitionStarted" => {
                    let _ = started_tx.send(()).await;
                }
                "AddTranscript" => {
                    if let Some(t) = parsed.metadata.and_then(|m| m.transcript) {
                        text_r.lock().unwrap().push_str(&t);
                    }
                }
                "EndOfTranscript" => {
                    let _ = done_tx.send(Ok(())).await;
                    return;
                }
                "Error" => {
                    let reason = parsed.reason.unwrap_or_else(|| "speechmatics error".into());
                    let _ = done_tx.send(Err(reason)).await;
                    return;
                }
                "Warning" => {
                    if let Some(reason) = parsed.reason {
                        eprintln!("tinydict: speechmatics warning: {reason}");
                    }
                }
                _ => {}
            }
        }
        let _ = done_tx
            .send(Err("speechmatics connection closed".into()))
            .await;
    });

    let mut session = Session {
        sink,
        seq: 0,
        started,
        done,
        text,
        reader,
    };
    session
        .wait_started(Duration::from_secs(config::ARMING_TIMEOUT_SECS))
        .await?;
    Ok(session)
}

impl Session {
    async fn wait_started(&mut self, timeout: Duration) -> Result<()> {
        match tokio::time::timeout(timeout, self.started.recv()).await {
            Ok(Some(())) => Ok(()),
            Ok(None) => self.fail_if_done(),
            Err(_) => bail!(
                "timeout waiting for RecognitionStarted ({}s)",
                timeout.as_secs()
            ),
        }
    }

    pub async fn send_audio(&mut self, bytes: Vec<u8>) -> Result<()> {
        self.check()?;
        self.seq += 1;
        self.sink
            .send(Message::binary(bytes))
            .await
            .context("sending audio")?;
        Ok(())
    }

    pub async fn finish(mut self, timeout: Duration) -> Result<String> {
        let eos = json!({ "message": "EndOfStream", "last_seq_no": self.seq });
        self.sink
            .send(Message::text(eos.to_string()))
            .await
            .context("sending EndOfStream")?;
        match tokio::time::timeout(timeout, self.done.recv()).await {
            Ok(Some(Ok(()))) => {}
            Ok(Some(Err(e))) => bail!(e),
            Ok(None) => eprintln!("tinydict: speechmatics reader ended before EndOfTranscript"),
            Err(_) => eprintln!("tinydict: EndOfTranscript timeout, keeping finals received"),
        }
        let _ = self.sink.close().await;
        Ok(self.transcript())
    }

    fn check(&mut self) -> Result<()> {
        match self.done.try_recv() {
            Ok(Err(e)) => bail!(e),
            Ok(Ok(())) | Err(_) => Ok(()),
        }
    }

    pub fn transcript(&self) -> String {
        self.text.lock().unwrap().clone()
    }

    fn fail_if_done(&mut self) -> Result<()> {
        match self.done.try_recv() {
            Ok(Err(e)) => bail!(e),
            _ => bail!("websocket closed before RecognitionStarted"),
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

fn vocab_json(words: &[VocabEntry]) -> Value {
    Value::Array(
        words
            .iter()
            .map(|w| match w {
                VocabEntry::Word(s) => json!(s),
                VocabEntry::Detailed {
                    content,
                    sounds_like,
                } => {
                    if sounds_like.is_empty() {
                        json!({ "content": content })
                    } else {
                        json!({ "content": content, "sounds_like": sounds_like })
                    }
                }
            })
            .collect(),
    )
}
