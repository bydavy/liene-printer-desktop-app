use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use futures::StreamExt;
use liene_protocol::{data_frames, methods, Frame, FrameReader, JobInfoParams, RpcResponse};
use serde_json::{json, Value};
use tokio::time::{sleep, timeout_at, Instant};
use waterkit_bluetooth::{
    BleConnection, BleScanner, ClassicBluetooth, ClassicDevice, DeviceId, GattService, ScanFilter,
    SppStream, Uuid,
};

/// Standard Serial Port Profile UUID.
pub const SPP_UUID: &str = "00001101-0000-1000-8000-00805f9b34fb";

/// How long to wait for the printer to report completion after the upload.
const PRINT_WATCH_TIMEOUT: Duration = Duration::from_secs(240);

pub async fn list_devices(
    name_prefix: &str,
    scan: bool,
    scan_secs: u64,
    all: bool,
) -> Result<Vec<ClassicDevice>> {
    let bt = ClassicBluetooth::new()
        .await
        .context("Bluetooth is unavailable or powered off")?;

    let mut devices = bt
        .paired_devices()
        .await
        .context("could not list paired Bluetooth devices")?;

    if scan {
        let found = discover(&bt, scan_secs).await?;
        for device in found {
            if !devices.iter().any(|d| d.device.id == device.device.id) {
                devices.push(device);
            }
        }
    }

    Ok(devices
        .into_iter()
        .filter(|device| all || name_matches(device, name_prefix))
        .collect())
}

async fn discover(bt: &ClassicBluetooth, scan_secs: u64) -> Result<Vec<ClassicDevice>> {
    let stream = bt
        .start_discovery()
        .await
        .context("could not start Bluetooth discovery")?;
    let mut stream = Box::pin(stream);

    let deadline = Instant::now() + Duration::from_secs(scan_secs);
    let mut found = Vec::new();
    while let Ok(Some(device)) = timeout_at(deadline, stream.next()).await {
        if !found
            .iter()
            .any(|d: &ClassicDevice| d.device.id == device.device.id)
        {
            found.push(device);
        }
    }
    drop(stream);
    bt.stop_discovery().await;
    Ok(found)
}

/// Scan BLE advertisements for devices whose name contains `name_prefix`.
///
/// This is a diagnostic aid: the N200 Pro speaks SPP for data, but it also
/// advertises over BLE, and some firmwares expose a Hannto GATT service.
pub async fn ble_scan(
    name_prefix: &str,
    scan_secs: u64,
) -> Result<Vec<waterkit_bluetooth::ScanResult>> {
    let scanner = BleScanner::new()
        .await
        .context("BLE is unavailable or powered off")?;
    let stream = scanner
        .start_scan(&ScanFilter {
            name_prefix: Some(name_prefix.to_owned()),
            ..ScanFilter::default()
        })
        .context("could not start BLE scan")?;
    let mut stream = Box::pin(stream);

    let deadline = Instant::now() + Duration::from_secs(scan_secs);
    let mut found = Vec::new();
    while let Ok(Some(result)) = timeout_at(deadline, stream.next()).await {
        let matches = result
            .device
            .name
            .as_deref()
            .is_some_and(|n| n.to_lowercase().contains(&name_prefix.to_lowercase()));
        if matches
            && !found
                .iter()
                .any(|r: &waterkit_bluetooth::ScanResult| r.device.id == result.device.id)
        {
            found.push(result);
        }
    }
    drop(stream);
    scanner.stop_scan();
    Ok(found)
}

/// Connect to a BLE device and enumerate GATT services.
pub async fn ble_services(device_id: &str) -> Result<(String, Vec<GattService>)> {
    let connection = BleConnection::connect(&DeviceId::new(device_id))
        .await
        .context("BLE connect failed")?;
    let services = connection
        .discover_services()
        .await
        .context("GATT service discovery failed")?;
    connection.disconnect().await;
    Ok((device_id.to_owned(), services))
}

/// Open the SPP channel, retrying because macOS may still be tearing down the
/// previous ACL link when the app was closed and reopened quickly.
async fn open_spp_with_retry(
    bt: &ClassicBluetooth,
    id: &DeviceId,
    name: &str,
) -> Result<SppStream> {
    let uuid = Uuid::new(SPP_UUID);
    let mut last_error: Option<anyhow::Error> = None;

    for attempt in 1..=3 {
        match tokio::time::timeout(Duration::from_secs(20), bt.connect_spp(id, &uuid)).await {
            Ok(Ok(stream)) => return Ok(stream),
            Ok(Err(err)) => last_error = Some(anyhow::Error::new(err)),
            Err(_) => last_error = Some(anyhow!("timed out opening the SPP channel")),
        }
        if attempt < 3 {
            sleep(Duration::from_secs(2)).await;
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow!("could not open the SPP channel"))).with_context(
        || {
            format!(
                "could not open the SPP channel to {name}. If the printer is not paired, \
                 pair it in your operating system's Bluetooth settings first."
            )
        },
    )
}

fn name_matches(device: &ClassicDevice, prefix: &str) -> bool {
    // Allow targeting an exact MAC address when discovery cannot resolve
    // names (IOBluetooth's inquiry does not request remote names by default).
    if prefix.contains(':') {
        return device.device.id.as_str().eq_ignore_ascii_case(prefix);
    }
    device
        .device
        .name
        .as_deref()
        .is_some_and(|name| name.to_lowercase().contains(&prefix.to_lowercase()))
}

pub struct Printer {
    // Keeps the classic-Bluetooth manager (and its inquiry state) alive.
    _bt: ClassicBluetooth,
    stream: SppStream,
    reader: FrameReader,
    next_id: u32,
    debug: bool,
    name: String,
    address: String,
}

/// Status properties the N200 Pro exposes (from the app's `pluginInfo`).
pub const STATUS_PROPS: [&str; 5] = [
    "printer-state",
    "printer-sub-state",
    "printer-state-alerts",
    "media-size",
    "battery",
];

/// Device information properties used by the app for protocol type 1.
pub const INFO_PROPS: [&str; 8] = [
    "model",
    "mac-address",
    "serial-number",
    "sn-pcba",
    "firmware-revision",
    "hardware-revision",
    "media-size",
    "auto-off-interval",
];

impl Printer {
    pub async fn connect(name_prefix: &str, scan_secs: u64, debug: bool) -> Result<Self> {
        let bt = ClassicBluetooth::new()
            .await
            .context("Bluetooth is unavailable or powered off")?;

        let mut candidates = bt.paired_devices().await.unwrap_or_default();
        let mut device = candidates
            .iter()
            .find(|d| name_matches(d, name_prefix))
            .cloned();

        if device.is_none() {
            eprintln!("# no paired printer found; scanning for {scan_secs}s...");
            let found = discover(&bt, scan_secs).await?;
            candidates.extend(found);
            device = candidates
                .into_iter()
                .find(|d| name_matches(d, name_prefix));
        }

        let device = device.ok_or_else(|| {
            anyhow!(
                "no device matching '{name_prefix}' found. Power the printer on, then pair it \
                 in your operating system's Bluetooth settings."
            )
        })?;

        let name = device.device.name.as_deref().unwrap_or("<unknown>");
        let id = device.device.id.clone();

        #[cfg(target_os = "macos")]
        if !device.is_paired {
            eprintln!("# {name} ({id}) is not paired; pairing now...");
            crate::pairing::pair_device(id.as_str())
                .await
                .context("pairing failed")?;
        }

        eprintln!("# connecting to {name} ({id}) over SPP");

        let stream = open_spp_with_retry(&bt, &id, name).await?;

        Ok(Self {
            _bt: bt,
            stream,
            reader: FrameReader::new(),
            next_id: 1,
            debug,
            name: name.to_owned(),
            address: id.as_str().to_owned(),
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn address(&self) -> &str {
        &self.address
    }

    pub async fn request(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<RpcResponse> {
        let id = self.next_id;
        self.next_id += 1;

        // `job-id-list` and friends take no params; the app omits them.
        let request = if params.is_null() {
            RawRequest::without_params(id, method)
        } else {
            RawRequest::with_params(id, method, params)
        };
        let frame = request.to_frame()?;

        if self.debug {
            eprintln!("# -> {method} id={id} {}", request.params_json());
        }
        self.write_frame(&frame).await?;

        let deadline = Instant::now() + timeout;
        loop {
            if let Some(item) = self.reader.next_frame() {
                match item {
                    Ok(frame) => {
                        if self.debug {
                            eprintln!("# <- {} bytes", frame.body.len());
                        }
                        if let Some(response) = parse_response(&frame, id)? {
                            return Ok(response);
                        }
                    }
                    Err(err) => {
                        if self.debug {
                            eprintln!("# ignoring bad frame: {err}");
                        }
                    }
                }
                continue;
            }

            if Instant::now() >= deadline {
                bail!("timed out waiting for a response to '{method}'");
            }
            self.read_more(deadline).await?;
        }
    }

    pub async fn request_value(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value> {
        let response = self.request(method, params, timeout).await?;
        if let Some(error) = response.error {
            bail!("printer returned an error: {error}");
        }
        Ok(response.result.unwrap_or(Value::Null))
    }

    /// Query the printer's live state properties.
    pub async fn status(&mut self, timeout: Duration) -> Result<Value> {
        self.request_value(methods::GET_PROP, json!(STATUS_PROPS), timeout)
            .await
    }

    /// Query model, serial number, firmware, and media information.
    pub async fn device_info(&mut self, timeout: Duration) -> Result<Value> {
        self.request_value(methods::GET_PROP, json!(INFO_PROPS), timeout)
            .await
    }

    /// Ask the printer to resume after a recoverable paper/ribbon error.
    pub async fn resume(&mut self, timeout: Duration) -> Result<Value> {
        self.request_value(methods::RESUME_PRINTER, json!([]), timeout)
            .await
    }

    /// Cancel a queued or active job.
    pub async fn cancel_job(&mut self, job_id: u32, timeout: Duration) -> Result<Value> {
        self.request_value(methods::CANCEL_JOB, json!([job_id]), timeout)
            .await
    }

    pub async fn job_info(&mut self, job_id: u32, timeout: Duration) -> Result<Value> {
        self.request_value(
            methods::GET_JOB_INFO,
            json!(JobInfoParams { job_id }),
            timeout,
        )
        .await
    }

    pub async fn upload(
        &mut self,
        job_id: u32,
        data: &[u8],
        frame_delay: Duration,
        mut progress: impl FnMut(usize, usize),
    ) -> Result<()> {
        let frames = data_frames(job_id, data)?;
        let total = frames.len();
        for (index, frame) in frames.iter().enumerate() {
            self.write_frame(frame)
                .await
                .with_context(|| format!("upload failed at frame {}/{total}", index + 1))?;
            progress(index + 1, total);

            // IOBluetooth's write completion means "accepted by the local
            // controller", not "consumed by the printer". The N200 Pro only
            // buffers ~47 frames (~42 KB), so pace the upload.
            if !frame_delay.is_zero() {
                sleep(frame_delay).await;
            } else {
                tokio::task::yield_now().await;
            }
        }
        Ok(())
    }

    /// Ask for the job state until it reaches a terminal state or the watch
    /// timeout expires. Poll failures are non-fatal: the printer is often
    /// busy while printing.
    ///
    /// `on_state` is called whenever the reported state changes; the return
    /// value is the final observed state (if any).
    pub async fn watch_job_with(
        &mut self,
        job_id: u32,
        request_timeout: Duration,
        mut on_state: impl FnMut(&Value),
    ) -> Result<Option<Value>> {
        let deadline = Instant::now() + PRINT_WATCH_TIMEOUT;
        // Let the printer start consuming the job before the first poll.
        sleep(Duration::from_secs(2)).await;
        let mut last = String::new();
        let mut final_state = None;
        loop {
            match self.job_info(job_id, request_timeout).await {
                Ok(value) => {
                    let rendered = serde_json::to_string(&value).unwrap_or_default();
                    if rendered != last {
                        on_state(&value);
                        last = rendered;
                    }
                    if is_terminal_job_state(&value) {
                        return Ok(Some(value));
                    }
                    final_state = Some(value);
                }
                Err(err) => {
                    if Instant::now() >= deadline {
                        if self.debug {
                            eprintln!("# stopped watching job {job_id}: {err:#}");
                        }
                        return Ok(final_state);
                    }
                    if self.debug {
                        eprintln!("# job poll failed (will retry): {err:#}");
                    }
                }
            }

            if Instant::now() >= deadline {
                if self.debug {
                    eprintln!("# timed out watching job {job_id}");
                }
                return Ok(final_state);
            }
            sleep(Duration::from_secs(2)).await;
        }
    }

    /// CLI-oriented wrapper around [`Self::watch_job_with`].
    pub async fn watch_job(&mut self, job_id: u32, request_timeout: Duration) -> Result<()> {
        self.watch_job_with(job_id, request_timeout, |value| {
            let rendered = serde_json::to_string(value).unwrap_or_default();
            eprintln!("# job {job_id}: {rendered}");
        })
        .await
        .map(|_| ())
    }

    pub async fn close(self) {
        self.stream.close().await;
    }

    async fn write_frame(&mut self, frame: &Frame) -> Result<()> {
        let bytes = frame.encode()?;
        if self.debug {
            eprintln!(
                "# tx {} bytes: {}",
                bytes.len(),
                hex(&bytes[..bytes.len().min(32)])
            );
        }

        let mut written = 0;
        while written < bytes.len() {
            let n = self
                .stream
                .write(&bytes[written..])
                .await
                .context("RFCOMM write failed")?;
            if n == 0 {
                bail!("RFCOMM write returned 0 bytes");
            }
            written += n;
        }
        Ok(())
    }

    async fn read_more(&mut self, deadline: Instant) -> Result<()> {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            bail!("timed out");
        }

        let mut buf = [0u8; 4096];
        let n = timeout_at(deadline, self.stream.read(&mut buf))
            .await
            .context("timed out waiting for data")?
            .context("RFCOMM read failed")?;

        if n == 0 {
            bail!("printer closed the connection");
        }
        if self.debug {
            eprintln!("# rx {} bytes: {}", n, hex(&buf[..n.min(32)]));
        }
        self.reader.push(&buf[..n]);
        Ok(())
    }
}

/// Minimal request type that can either include or omit `params`.
struct RawRequest {
    id: u32,
    method: String,
    params: Option<Value>,
}

impl RawRequest {
    fn with_params(id: u32, method: &str, params: Value) -> Self {
        Self {
            id,
            method: method.to_owned(),
            params: Some(params),
        }
    }

    fn without_params(id: u32, method: &str) -> Self {
        Self {
            id,
            method: method.to_owned(),
            params: None,
        }
    }

    fn params_json(&self) -> String {
        self.params
            .as_ref()
            .map(|p| p.to_string())
            .unwrap_or_default()
    }

    fn to_frame(&self) -> Result<Frame> {
        // Reuse the typed request for the wire shape, then drop `params` if
        // the caller asked for a no-argument method.
        let value = match &self.params {
            Some(params) => json!({
                "id": self.id,
                "method": self.method,
                "params": params,
                "spec-type": 1,
            }),
            None => json!({
                "id": self.id,
                "method": self.method,
                "spec-type": 1,
            }),
        };
        Ok(Frame::json(serde_json::to_vec(&value)?))
    }
}

fn parse_response(frame: &Frame, expected_id: u32) -> Result<Option<RpcResponse>> {
    let value: Value = match serde_json::from_slice(&frame.body) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };

    if let Some(method) = value.get("method").and_then(Value::as_str) {
        if method.starts_with("event.") {
            eprintln!("# event: {value}");
            return Ok(None);
        }
    }

    let Some(id) = value.get("id").and_then(Value::as_u64) else {
        return Ok(None);
    };
    if id as u32 != expected_id {
        return Ok(None);
    }

    let response: RpcResponse = serde_json::from_value(value)?;
    Ok(Some(response))
}

fn is_terminal_job_state(value: &Value) -> bool {
    // The printer returns numeric job states: 9 finished, 8 canceled,
    // 7 aborted (com.hannto.common.n.Q). Older/other responses may use
    // strings, so check both.
    fn numeric_terminal(object: &Value) -> bool {
        object
            .get("job-state")
            .and_then(Value::as_u64)
            .is_some_and(|state| matches!(state, 7..=9))
    }

    let numeric = match value {
        Value::Array(items) => items.iter().any(numeric_terminal),
        Value::Object(_) => numeric_terminal(value),
        _ => false,
    };
    if numeric {
        return true;
    }

    let text = value.to_string().to_lowercase();
    const TERMINAL: [&str; 6] = [
        "finished",
        "completed",
        "canceled",
        "cancelled",
        "aborted",
        "idle",
    ];
    TERMINAL.iter().any(|state| text.contains(state))
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}
