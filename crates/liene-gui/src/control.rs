//! Authenticated loopback control channel for the desktop UI.
//!
//! The GUI only starts this server with `--control`. A discovery file contains
//! the ephemeral port and a per-process token; the `liene-control` CLI uses it
//! to drive the same actions as the UI and to take screenshots.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Discovery {
    pub port: u16,
    pub token: String,
    pub pid: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub token: String,
    pub action: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub ok: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub state: Option<Value>,
}

impl Response {
    pub fn ok(message: impl Into<String>) -> Self {
        Self {
            ok: true,
            error: None,
            message: Some(message.into()),
            state: None,
        }
    }

    pub fn with_state(message: impl Into<String>, state: Value) -> Self {
        Self {
            ok: true,
            error: None,
            message: Some(message.into()),
            state: Some(state),
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            error: Some(message.into()),
            message: None,
            state: None,
        }
    }
}

pub struct ControlJob {
    pub request: Request,
    pub reply: Sender<Response>,
}

pub fn default_discovery_path() -> PathBuf {
    if let Some(path) = std::env::var_os("LIENE_CONTROL_FILE") {
        return PathBuf::from(path);
    }
    std::env::temp_dir().join("liene-desktop-control.json")
}

pub fn start_server(path: &Path) -> Result<(Discovery, Receiver<ControlJob>)> {
    let listener =
        TcpListener::bind(("127.0.0.1", 0)).context("could not bind the UI control channel")?;
    let port = listener
        .local_addr()
        .context("could not read the UI control port")?
        .port();
    let discovery = Discovery {
        port,
        token: make_token(),
        pid: std::process::id(),
    };
    fs::write(path, serde_json::to_vec_pretty(&discovery)?)
        .with_context(|| format!("could not write {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }

    let (tx, rx) = mpsc::channel();
    let token = discovery.token.clone();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { break };
            let _ = handle_connection(stream, &token, &tx);
        }
    });
    Ok((discovery, rx))
}

fn handle_connection(mut stream: TcpStream, token: &str, jobs: &Sender<ControlJob>) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut line = String::new();
    {
        let mut reader = BufReader::new(stream.try_clone()?);
        reader.read_line(&mut line)?;
    }
    let request: Request = match serde_json::from_str(&line) {
        Ok(request) => request,
        Err(err) => {
            let _ = writeln!(
                stream,
                "{}",
                serde_json::to_string(&Response::error(format!("invalid request: {err}")))?,
            );
            return Ok(());
        }
    };
    if request.token != token {
        let _ = writeln!(
            stream,
            "{}",
            serde_json::to_string(&Response::error("invalid control token"))?,
        );
        return Ok(());
    }

    let (reply_tx, reply_rx) = mpsc::channel();
    if jobs
        .send(ControlJob {
            request,
            reply: reply_tx,
        })
        .is_err()
    {
        let _ = writeln!(
            stream,
            "{}",
            serde_json::to_string(&Response::error("the desktop app is not running"))?,
        );
        return Ok(());
    }
    let response = reply_rx
        .recv_timeout(Duration::from_secs(30))
        .unwrap_or_else(|_| Response::error("timed out waiting for the UI"));
    writeln!(stream, "{}", serde_json::to_string(&response)?)?;
    Ok(())
}

pub fn remove_discovery(path: &Path) {
    let _ = fs::remove_file(path);
}

pub fn send(path: &Path, action: &str, params: Value) -> Result<Response> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("no control discovery file at {}", path.display()))?;
    let discovery: Discovery = serde_json::from_str(&text)?;
    let mut stream = TcpStream::connect(("127.0.0.1", discovery.port))
        .context("could not reach the desktop app's control channel")?;
    stream.set_read_timeout(Some(Duration::from_secs(35)))?;
    let request = Request {
        token: discovery.token,
        action: action.to_owned(),
        params,
    };
    writeln!(stream, "{}", serde_json::to_string(&request)?)?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line)?;
    if line.trim().is_empty() {
        bail!("the desktop app returned no response");
    }
    let response: Response = serde_json::from_str(&line)?;
    if !response.ok {
        bail!(
            "{}",
            response
                .error
                .unwrap_or_else(|| "UI action failed".to_owned())
        );
    }
    Ok(response)
}

fn make_token() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let pid = u128::from(std::process::id());
    format!("{:032x}", nanos ^ (pid << 64) ^ 0x9e37_79b9_7f4a_7c15)
}
