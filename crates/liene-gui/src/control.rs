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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn test_path(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "liene-control-test-{}-{name}.json",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        path
    }

    fn discovery_after_start(path: &Path) -> Discovery {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if let Ok(text) = fs::read_to_string(path) {
                if let Ok(discovery) = serde_json::from_str(&text) {
                    return discovery;
                }
            }
            assert!(
                std::time::Instant::now() < deadline,
                "control discovery file was not written"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn raw_request(port: u16, token: &str, action: &str) -> Response {
        let mut stream =
            TcpStream::connect(("127.0.0.1", port)).expect("connect to control server");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("read timeout");
        let request = Request {
            token: token.to_owned(),
            action: action.to_owned(),
            params: Value::Null,
        };
        writeln!(stream, "{}", serde_json::to_string(&request).unwrap()).unwrap();
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }

    #[test]
    fn rejects_requests_with_the_wrong_token() {
        let path = test_path("wrong-token");
        let (_discovery, _jobs) = start_server(&path).expect("start control server");
        let discovery = discovery_after_start(&path);

        let response = raw_request(discovery.port, "not-the-token", "state");
        assert!(!response.ok);
        assert_eq!(response.error.as_deref(), Some("invalid control token"));

        remove_discovery(&path);
    }

    #[test]
    fn forwards_authenticated_requests_to_the_ui() {
        let path = test_path("round-trip");
        let (_discovery, jobs) = start_server(&path).expect("start control server");
        let _ = discovery_after_start(&path);

        let ui = thread::spawn(move || {
            let job = jobs
                .recv_timeout(Duration::from_secs(5))
                .expect("UI should receive the request");
            assert_eq!(job.request.action, "state");
            job.reply
                .send(Response::with_state("ok", json!({ "photos": 3 })))
                .expect("reply to the control client");
        });

        let response = send(&path, "state", Value::Null).expect("send request");
        assert!(response.ok);
        assert_eq!(response.state.unwrap()["photos"], 3);
        ui.join().expect("UI thread");

        remove_discovery(&path);
        assert!(!path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn discovery_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let path = test_path("permissions");
        let (_discovery, _jobs) = start_server(&path).expect("start control server");
        let _ = discovery_after_start(&path);

        let mode = fs::metadata(&path)
            .expect("discovery metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "discovery file mode was {mode:o}");

        remove_discovery(&path);
    }
}
