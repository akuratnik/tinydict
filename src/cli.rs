use crate::config;
use anyhow::{bail, Context, Result};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

pub fn send(cmd: &str) -> Result<()> {
    let line = request(cmd)?;
    print!("{line}");
    if line.starts_with("error") {
        std::process::exit(1);
    }
    Ok(())
}

/// Sends one command to the daemon and returns its reply line.
pub fn request(cmd: &str) -> Result<String> {
    let path = config::socket_path();
    let stream = UnixStream::connect(&path).with_context(|| {
        format!(
            "connecting to {} (run `tinydict setup` if the socket is missing)",
            path.display()
        )
    })?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let mut stream = stream;
    writeln!(stream, "{cmd}").context("sending command")?;
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .context("reading reply")?;
    if line.is_empty() {
        bail!("empty reply from daemon");
    }
    Ok(line)
}
