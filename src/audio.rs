use anyhow::{Context, Result};
use std::collections::VecDeque;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};
use tokio::sync::Notify;

use crate::config;

/// Recorder that writes 16 kHz mono s16le PCM to stdout until SIGTERM.
#[cfg(target_os = "linux")]
#[rustfmt::skip]
const RECORDER: &[&str] = &["pw-record", "--rate", "16000", "--channels", "1", "--format", "s16", "-"];
#[cfg(target_os = "macos")]
#[rustfmt::skip]
const RECORDER: &[&str] = &["rec", "-q", "-t", "raw", "-r", "16000", "-c", "1", "-b", "16", "-e", "signed-integer", "-L", "-"];

struct Inner {
    chunks: VecDeque<Vec<u8>>,
    bytes: usize,
    eof: bool,
    dropped: bool,
}

struct AudioQ {
    inner: Mutex<Inner>,
    notify: Notify,
}

impl AudioQ {
    fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                chunks: VecDeque::new(),
                bytes: 0,
                eof: false,
                dropped: false,
            }),
            notify: Notify::new(),
        }
    }

    fn push(&self, chunk: Vec<u8>) {
        let mut g = self.inner.lock().unwrap();
        g.bytes += chunk.len();
        g.chunks.push_back(chunk);
        while g.bytes > config::AUDIO_CAP_BYTES {
            if let Some(old) = g.chunks.pop_front() {
                g.bytes -= old.len();
                if !g.dropped {
                    g.dropped = true;
                    eprintln!("tinydict: audio buffer full, dropping oldest");
                }
            }
        }
        self.notify.notify_one();
    }

    fn close(&self) {
        self.inner.lock().unwrap().eof = true;
        self.notify.notify_one();
    }

    fn pop(&self) -> Option<Vec<u8>> {
        let mut g = self.inner.lock().unwrap();
        if let Some(c) = g.chunks.pop_front() {
            g.bytes = g.bytes.saturating_sub(c.len());
            return Some(c);
        }
        None
    }

    fn is_eof(&self) -> bool {
        let g = self.inner.lock().unwrap();
        g.eof && g.chunks.is_empty()
    }

    async fn recv(&self) -> Option<Vec<u8>> {
        loop {
            if let Some(c) = self.pop() {
                return Some(c);
            }
            if self.is_eof() {
                return None;
            }
            self.notify.notified().await;
        }
    }
}

pub struct Capture {
    child: Child,
    q: Arc<AudioQ>,
    drain: Option<tokio::task::JoinHandle<()>>,
    stopped: bool,
}

impl Capture {
    pub async fn start() -> Result<Self> {
        let mut child = Command::new(RECORDER[0])
            .args(&RECORDER[1..])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("spawning {}", RECORDER[0]))?;

        let mut stdout = child.stdout.take().context("recorder stdout")?;
        let q = Arc::new(AudioQ::new());
        let q2 = q.clone();
        let drain = tokio::spawn(async move {
            let mut buf = vec![0u8; config::AUDIO_CHUNK];
            loop {
                match stdout.read(&mut buf).await {
                    Ok(0) => break,
                    Ok(n) => q2.push(buf[..n].to_vec()),
                    Err(err) => {
                        eprintln!("tinydict: recorder read: {err}");
                        break;
                    }
                }
            }
            q2.close();
        });

        Ok(Self {
            child,
            q,
            drain: Some(drain),
            stopped: false,
        })
    }

    pub async fn recv(&self) -> Option<Vec<u8>> {
        self.q.recv().await
    }

    pub async fn stop(&mut self) {
        if self.stopped {
            return;
        }
        self.stopped = true;
        if let Some(pid) = self.child.id() {
            unsafe {
                libc::kill(pid as libc::pid_t, libc::SIGTERM);
            }
        }
        let wait = tokio::time::timeout(std::time::Duration::from_secs(2), self.child.wait());
        if wait.await.is_err() {
            // Wedged recorder (e.g. stalled device): stdout never hits EOF otherwise.
            let _ = self.child.kill().await;
        }
        if let Some(drain) = self.drain.take() {
            let _ = drain.await;
        }
    }
}
