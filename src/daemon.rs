use crate::{audio, cleanup, config, output, speechmatics, tray};
use anyhow::{bail, Context, Result};
use futures_util::FutureExt;
use std::fs;
use std::panic::AssertUnwindSafe;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::signal::unix::{signal, SignalKind};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::{sleep, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Arming,
    Recording,
    Finalizing,
    Cleaning,
    Publishing,
}

impl Phase {
    fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Arming => "arming",
            Self::Recording => "recording",
            Self::Finalizing => "finalizing",
            Self::Cleaning => "cleaning",
            Self::Publishing => "publishing",
        }
    }

    fn busy(self) -> bool {
        matches!(self, Self::Finalizing | Self::Cleaning | Self::Publishing)
    }

    fn live(self) -> bool {
        matches!(self, Self::Arming | Self::Recording)
    }
}

enum SessionCmd {
    Stop,
    Cancel,
}

#[derive(Debug)]
struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("cancelled")
    }
}

impl std::error::Error for Cancelled {}

#[tokio::main(flavor = "current_thread")]
pub async fn run() -> Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .ok();
    let listener = bind().await?;
    let (phase_tx, mut phase_rx) = watch::channel(Phase::Idle);
    let idle_for = Duration::from_secs(
        config::load()
            .unwrap_or_default()
            .daemon
            .idle_exit_secs
            .max(1),
    );
    let mut sigterm = signal(SignalKind::terminate())?;
    let mut sigint = signal(SignalKind::interrupt())?;
    let mut session: Option<JoinHandle<()>> = None;
    let mut session_tx: Option<mpsc::Sender<SessionCmd>> = None;
    let mut idle_deadline = std::pin::pin!(sleep(idle_for));

    loop {
        tokio::select! {
            _ = &mut idle_deadline, if *phase_rx.borrow() == Phase::Idle => {
                eprintln!("tinydict: idle exit");
                return Ok(());
            }
            _ = phase_rx.changed() => {
                if *phase_rx.borrow() == Phase::Idle {
                    idle_deadline.as_mut().reset(Instant::now() + idle_for);
                }
            }
            joined = async { session.as_mut().unwrap().await }, if session.is_some() => {
                session = None;
                session_tx = None;
                if let Err(err) = joined {
                    if err.is_panic() {
                        eprintln!("tinydict: session panicked: {err}");
                        output::notify_error("internal error (session crashed)");
                    }
                }
                let _ = phase_tx.send(Phase::Idle);
                idle_deadline.as_mut().reset(Instant::now() + idle_for);
            }
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, _)) => {
                        handle_client(stream, &phase_tx, &mut session, &mut session_tx).await;
                    }
                    Err(e) => eprintln!("tinydict: accept: {e}"),
                }
            }
            _ = sigterm.recv() => {
                shutdown_session(&mut session, &mut session_tx).await;
                return Ok(());
            }
            _ = sigint.recv() => {
                shutdown_session(&mut session, &mut session_tx).await;
                return Ok(());
            }
        }
    }
}

async fn shutdown_session(
    session: &mut Option<JoinHandle<()>>,
    session_tx: &mut Option<mpsc::Sender<SessionCmd>>,
) {
    if let Some(tx) = session_tx.take() {
        let _ = tx.send(SessionCmd::Cancel).await;
    }
    if let Some(h) = session.take() {
        let _ = tokio::time::timeout(Duration::from_secs(2), h).await;
    }
}

async fn bind() -> Result<UnixListener> {
    if let Some(l) = activated_listener()? {
        l.set_nonblocking(true)?;
        return Ok(UnixListener::from_std(l)?);
    }
    let path = config::socket_path();
    if path.exists() {
        match std::os::unix::net::UnixStream::connect(&path) {
            Ok(_) => bail!("already running at {}", path.display()),
            Err(_) => {
                let _ = fs::remove_file(&path);
            }
        }
    }
    UnixListener::bind(&path).with_context(|| format!("binding {}", path.display()))
}

#[cfg(target_os = "linux")]
fn activated_listener() -> Result<Option<std::os::unix::net::UnixListener>> {
    listenfd::ListenFd::from_env()
        .take_unix_listener(0)
        .context("LISTEN_FDS")
}

/// Name of the `Sockets` entry in the launchd plist.
#[cfg(target_os = "macos")]
pub const LAUNCHD_SOCKET: &str = "Listener";

#[cfg(target_os = "macos")]
fn activated_listener() -> Result<Option<std::os::unix::net::UnixListener>> {
    use std::os::fd::{FromRawFd, OwnedFd};

    // <launch.h>, part of libSystem.
    extern "C" {
        fn launch_activate_socket(
            name: *const libc::c_char,
            fds: *mut *mut libc::c_int,
            cnt: *mut libc::size_t,
        ) -> libc::c_int;
    }

    let name = std::ffi::CString::new(LAUNCHD_SOCKET)?;
    let mut fds = std::ptr::null_mut();
    let mut cnt = 0;
    let err = unsafe { launch_activate_socket(name.as_ptr(), &mut fds, &mut cnt) };
    match err {
        0 => {}
        // Not started by launchd, or no such socket: bind it ourselves.
        libc::ESRCH | libc::ENOENT => return Ok(None),
        e => return Err(std::io::Error::from_raw_os_error(e)).context("launch_activate_socket"),
    }
    if fds.is_null() {
        return Ok(None);
    }
    // We own every fd and the malloc'd array; extra fds close on drop.
    let owned: Vec<OwnedFd> = unsafe {
        let v = std::slice::from_raw_parts(fds, cnt)
            .iter()
            .map(|&fd| OwnedFd::from_raw_fd(fd))
            .collect();
        libc::free(fds.cast());
        v
    };
    Ok(owned.into_iter().next().map(Into::into))
}

async fn handle_client(
    stream: UnixStream,
    phase_tx: &watch::Sender<Phase>,
    session: &mut Option<JoinHandle<()>>,
    session_tx: &mut Option<mpsc::Sender<SessionCmd>>,
) {
    if let Err(err) = handle_client_inner(stream, phase_tx, session, session_tx).await {
        eprintln!("tinydict: client: {err:#}");
    }
}

async fn handle_client_inner(
    stream: UnixStream,
    phase_tx: &watch::Sender<Phase>,
    session: &mut Option<JoinHandle<()>>,
    session_tx: &mut Option<mpsc::Sender<SessionCmd>>,
) -> Result<()> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    match tokio::time::timeout(Duration::from_secs(2), reader.read_line(&mut line)).await {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => return Err(e.into()),
        Err(_) => bail!("client timed out"),
    }
    let cmd = line.trim().to_ascii_lowercase();
    let reply = match cmd.as_str() {
        "status" => phase_tx.borrow().as_str().to_string(),
        "toggle" => toggle(phase_tx, session, session_tx).await,
        "cancel" => cancel(phase_tx, session_tx).await,
        _ => "error unknown command".into(),
    };
    let mut stream = reader.into_inner();
    stream.write_all(reply.as_bytes()).await?;
    stream.write_all(b"\n").await?;
    Ok(())
}

async fn toggle(
    phase_tx: &watch::Sender<Phase>,
    session: &mut Option<JoinHandle<()>>,
    session_tx: &mut Option<mpsc::Sender<SessionCmd>>,
) -> String {
    let phase = *phase_tx.borrow();
    if phase.busy() {
        return format!("busy {}", phase.as_str());
    }
    if session.is_some() || phase.live() {
        if let Some(tx) = session_tx {
            let _ = tx.send(SessionCmd::Stop).await;
        }
        return "ok".into();
    }
    let cfg = match config::load() {
        Ok(c) => c,
        Err(e) => return format!("error {e}"),
    };
    if let Err(e) = config::api_key(&cfg.speechmatics) {
        return format!("error {e}");
    }
    let (tx, rx) = mpsc::channel(4);
    *session_tx = Some(tx);
    let _ = phase_tx.send(Phase::Arming);
    let phase = phase_tx.clone();
    *session = Some(tokio::spawn(async move {
        run_session(cfg, rx, phase).await;
    }));
    "ok arming".into()
}

async fn cancel(
    phase_tx: &watch::Sender<Phase>,
    session_tx: &mut Option<mpsc::Sender<SessionCmd>>,
) -> String {
    if *phase_tx.borrow() == Phase::Idle {
        return "idle".into();
    }
    if let Some(tx) = session_tx {
        let _ = tx.send(SessionCmd::Cancel).await;
    }
    "cancelled".into()
}

async fn run_session(
    cfg: config::Config,
    mut cmds: mpsc::Receiver<SessionCmd>,
    phase: watch::Sender<Phase>,
) {
    let mut tray = tray::start(tray::Color::Red).await;
    let caught = AssertUnwindSafe(session_inner(&cfg, &mut cmds, &phase, tray.as_ref()))
        .catch_unwind()
        .await;
    if let Some(t) = tray.take() {
        t.shutdown().await;
    }
    let _ = phase.send(Phase::Idle);
    match caught {
        Ok(Ok(())) => {}
        Ok(Err(e)) if e.is::<Cancelled>() => output::notify_cancelled(),
        Ok(Err(e)) => {
            eprintln!("tinydict: {e:#}");
            output::notify_error(&short_err(&e));
        }
        Err(_) => {
            eprintln!("tinydict: session panicked");
            output::notify_error("internal error (session crashed)");
        }
    }
}

fn short_err(e: &anyhow::Error) -> String {
    let s = format!("{e}");
    if s.len() > 200 {
        format!("{}…", s.chars().take(200).collect::<String>())
    } else {
        s
    }
}

async fn race_cancel<T>(
    cmds: &mut mpsc::Receiver<SessionCmd>,
    fut: impl std::future::Future<Output = Result<T>>,
) -> Result<T> {
    tokio::pin!(fut);
    loop {
        tokio::select! {
            cmd = cmds.recv() => match cmd {
                Some(SessionCmd::Cancel) | None => bail!(Cancelled),
                Some(SessionCmd::Stop) => {}
            },
            res = &mut fut => return res,
        }
    }
}

async fn session_inner(
    cfg: &config::Config,
    cmds: &mut mpsc::Receiver<SessionCmd>,
    phase: &watch::Sender<Phase>,
    tray: Option<&tray::Handle>,
) -> Result<()> {
    let mut capture = audio::Capture::start().await?;
    let connect = speechmatics::connect(&cfg.speechmatics, &cfg.vocab.words);
    tokio::pin!(connect);

    let mut stopped = false;
    let mut stt = loop {
        tokio::select! {
            cmd = cmds.recv() => match cmd {
                Some(SessionCmd::Cancel) | None => {
                    capture.stop().await;
                    bail!(Cancelled);
                }
                Some(SessionCmd::Stop) => {
                    capture.stop().await;
                    stopped = true;
                }
            },
            res = &mut connect => break res?,
        }
    };

    if !stopped {
        let _ = phase.send(Phase::Recording);
    }

    loop {
        tokio::select! {
            cmd = cmds.recv() => match cmd {
                Some(SessionCmd::Cancel) | None => {
                    capture.stop().await;
                    bail!(Cancelled);
                }
                Some(SessionCmd::Stop) => {
                    capture.stop().await;
                }
            },
            chunk = capture.recv() => match chunk {
                Some(c) => stt.send_audio(c).await?,
                None => break,
            }
        }
    }

    let _ = phase.send(Phase::Finalizing);
    if let Some(t) = tray {
        t.set(tray::Color::Amber).await;
    }
    capture.stop().await;

    let raw = race_cancel(
        cmds,
        stt.finish(Duration::from_secs(config::FINALIZE_TIMEOUT_SECS)),
    )
    .await?;
    let raw = raw.trim().to_string();
    if raw.is_empty() {
        output::notify_plain("no speech detected");
        return Ok(());
    }

    let replaced = config::apply_replacements(&raw, &cfg.replacements);

    let _ = phase.send(Phase::Cleaning);
    let (text, note) = if cfg.cleanup.enabled {
        match race_cancel(cmds, cleanup::run(cfg, &replaced)).await {
            Ok(t) => (t, None),
            Err(e) if e.is::<Cancelled>() => return Err(e),
            Err(e) => fallback_cleanup(&replaced, e),
        }
    } else {
        (replaced, None)
    };

    let _ = phase.send(Phase::Publishing);
    output::publish(cfg, &raw, &text, note.as_deref()).await?;
    Ok(())
}

fn fallback_cleanup(replaced: &str, e: anyhow::Error) -> (String, Option<String>) {
    eprintln!("tinydict: cleanup failed: {e:#}");
    (
        replaced.to_string(),
        Some(format!("cleanup skipped: {}", short_err(&e))),
    )
}
