//! JSON-line daemon used by the GUI to keep Bluetooth work on a process
//! whose main run loop actually services IOBluetooth callbacks.
//!
//! Commands arrive as one JSON object per line; events are emitted the same
//! way. The enums are shared by the CLI server and the GUI client.

use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::collage;
use crate::edit::PhotoEdit;
use crate::image_prep::{self, Fit};
use crate::printer::Printer;
use liene_protocol::{frame_count, methods, PrintJob, PrintJobParams, MEDIA_SIZE_2X3};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(12);
const FRAME_DELAY: Duration = Duration::from_millis(20);
const SCAN_SECS: u64 = 10;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    Connect { name: String, scan_secs: u64 },
    RefreshStatus,
    RefreshInfo,
    Print(PrintRequest),
    PrintCollage(CollageRequest),
    Resume,
    CancelJob { job_id: u32 },
    Pair { address: String },
    Scan,
    Disconnect,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollageRequest {
    pub paths: Vec<PathBuf>,
    pub layout: String,
    pub copies: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrintRequest {
    pub path: PathBuf,
    pub copies: u32,
    pub width: u32,
    pub height: u32,
    pub fit: Fit,
    #[serde(default)]
    pub edit: PhotoEdit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    Busy { busy: bool },
    Connecting { name: String },
    Connected { name: String, address: String },
    Disconnected,
    Status { value: Value },
    Info { value: Value },
    Devices { lines: Vec<String> },
    PrintPrepared { bytes: usize },
    JobCreated { job_id: u32, total_frames: usize },
    UploadProgress { sent: usize, total: usize },
    JobState { value: Value },
    PrintFinished { state: Option<Value> },
    Log { message: String },
    Error { message: String },
}

pub async fn run(cmd_rx: Receiver<Command>, mut emit: impl FnMut(Event)) {
    let mut printer: Option<Printer> = None;
    let mut name = "Liene".to_owned();
    let mut scan_secs = 12;

    while let Ok(command) = cmd_rx.recv() {
        match command {
            Command::Connect {
                name: new_name,
                scan_secs: new_scan,
            } => {
                name = new_name;
                scan_secs = new_scan;
                emit(Event::Busy { busy: true });
                if ensure_connected(&mut printer, &mut emit, &name, scan_secs).await {
                    let _ = refresh_status(&mut printer, &mut emit).await;
                    let _ = refresh_info(&mut printer, &mut emit).await;
                }
                emit(Event::Busy { busy: false });
            }

            Command::RefreshStatus => {
                if ensure_connected(&mut printer, &mut emit, &name, scan_secs).await {
                    let _ = refresh_status(&mut printer, &mut emit).await;
                }
            }

            Command::RefreshInfo => {
                if ensure_connected(&mut printer, &mut emit, &name, scan_secs).await {
                    let _ = refresh_info(&mut printer, &mut emit).await;
                }
            }

            Command::Print(request) => {
                emit(Event::Busy { busy: true });
                if let Err(err) = print(&mut printer, &mut emit, &name, scan_secs, request).await {
                    emit(Event::Error {
                        message: format!("{err:#}"),
                    });
                    printer = None;
                    emit(Event::Disconnected);
                }
                emit(Event::Busy { busy: false });
                if printer.is_some() {
                    let _ = refresh_status(&mut printer, &mut emit).await;
                }
            }

            Command::PrintCollage(request) => {
                emit(Event::Busy { busy: true });
                if let Err(err) =
                    print_collage(&mut printer, &mut emit, &name, scan_secs, request).await
                {
                    emit(Event::Error {
                        message: format!("{err:#}"),
                    });
                    printer = None;
                    emit(Event::Disconnected);
                }
                emit(Event::Busy { busy: false });
                if printer.is_some() {
                    let _ = refresh_status(&mut printer, &mut emit).await;
                }
            }

            Command::Resume => {
                if ensure_connected(&mut printer, &mut emit, &name, scan_secs).await {
                    let result = match printer.as_mut() {
                        Some(p) => p.resume(REQUEST_TIMEOUT).await,
                        None => Ok(Value::Null),
                    };
                    match result {
                        Ok(value) => {
                            emit(Event::Log {
                                message: format!("resume-printer: {value}"),
                            });
                            let _ = refresh_status(&mut printer, &mut emit).await;
                        }
                        Err(err) => {
                            emit(Event::Error {
                                message: format!("could not resume the printer: {err:#}"),
                            });
                            printer = None;
                            emit(Event::Disconnected);
                        }
                    }
                }
            }

            Command::CancelJob { job_id } => {
                if ensure_connected(&mut printer, &mut emit, &name, scan_secs).await {
                    let result = match printer.as_mut() {
                        Some(p) => p.cancel_job(job_id, REQUEST_TIMEOUT).await,
                        None => Ok(Value::Null),
                    };
                    match result {
                        Ok(value) => {
                            emit(Event::Log {
                                message: format!("cancel-job {job_id}: {value}"),
                            });
                            let _ = refresh_status(&mut printer, &mut emit).await;
                        }
                        Err(err) => {
                            emit(Event::Error {
                                message: format!("could not cancel job {job_id}: {err:#}"),
                            });
                            printer = None;
                            emit(Event::Disconnected);
                        }
                    }
                }
            }

            Command::Pair { address } => match crate::pairing::pair_device(&address).await {
                Ok(()) => {
                    emit(Event::Log {
                        message: format!("paired {address}"),
                    });
                    emit(Event::Busy { busy: false });
                }
                Err(err) => {
                    emit(Event::Error {
                        message: format!("pairing failed: {err:#}"),
                    });
                    emit(Event::Busy { busy: false });
                }
            },

            Command::Scan => {
                emit(Event::Busy { busy: true });
                match crate::list_devices(&name, true, SCAN_SECS, true).await {
                    Ok(devices) => {
                        let lines = devices
                            .into_iter()
                            .map(|d| {
                                format!(
                                    "{}\t{}\t{}",
                                    d.device.id,
                                    d.device.name.as_deref().unwrap_or("<unnamed>"),
                                    if d.is_paired { "paired" } else { "unpaired" }
                                )
                            })
                            .collect();
                        emit(Event::Devices { lines });
                    }
                    Err(err) => {
                        emit(Event::Error {
                            message: format!("scan failed: {err:#}"),
                        });
                    }
                }
                emit(Event::Busy { busy: false });
            }

            Command::Disconnect => {
                if let Some(printer) = printer.take() {
                    printer.close().await;
                }
                emit(Event::Disconnected);
                emit(Event::Busy { busy: false });
            }
        }
    }

    if let Some(printer) = printer.take() {
        printer.close().await;
    }
}

async fn ensure_connected(
    printer: &mut Option<Printer>,
    emit: &mut impl FnMut(Event),
    name: &str,
    scan_secs: u64,
) -> bool {
    if printer.is_some() {
        return true;
    }
    emit(Event::Connecting {
        name: name.to_owned(),
    });
    emit(Event::Log {
        message: format!("Connecting to {name}…"),
    });
    match Printer::connect(name, scan_secs, false).await {
        Ok(p) => {
            emit(Event::Connected {
                name: p.name().to_owned(),
                address: p.address().to_owned(),
            });
            *printer = Some(p);
            true
        }
        Err(err) => {
            emit(Event::Error {
                message: format!("{err:#}"),
            });
            false
        }
    }
}

async fn refresh_status(printer: &mut Option<Printer>, emit: &mut impl FnMut(Event)) -> Result<()> {
    let Some(p) = printer.as_mut() else {
        return Ok(());
    };
    match p.status(REQUEST_TIMEOUT).await {
        Ok(value) => {
            emit(Event::Status { value });
            Ok(())
        }
        Err(err) => {
            emit(Event::Error {
                message: format!("status query failed: {err:#}"),
            });
            *printer = None;
            emit(Event::Disconnected);
            Err(err)
        }
    }
}

async fn refresh_info(printer: &mut Option<Printer>, emit: &mut impl FnMut(Event)) -> Result<()> {
    let Some(p) = printer.as_mut() else {
        return Ok(());
    };
    match p.device_info(REQUEST_TIMEOUT).await {
        Ok(value) => {
            emit(Event::Info { value });
            Ok(())
        }
        Err(err) => {
            emit(Event::Log {
                message: format!("device info query failed: {err:#}"),
            });
            Ok(())
        }
    }
}

async fn print(
    printer: &mut Option<Printer>,
    emit: &mut impl FnMut(Event),
    name: &str,
    scan_secs: u64,
    request: PrintRequest,
) -> Result<()> {
    if !ensure_connected(printer, emit, name, scan_secs).await {
        anyhow::bail!("printer is not connected");
    }

    let data = image_prep::prepare_jpeg_with_edit(
        &request.path,
        request.width,
        request.height,
        request.fit,
        &request.edit,
    )
    .with_context(|| format!("could not prepare {}", request.path.display()))?;
    emit(Event::PrintPrepared { bytes: data.len() });

    let document_name = request
        .path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("liene-print.jpg");
    let document_name = format!(
        "{}.jpg",
        document_name
            .rsplit_once('.')
            .map(|(stem, _)| stem)
            .unwrap_or(document_name)
    );
    send_print_data(printer, emit, &data, request.copies, &document_name).await
}

async fn print_collage(
    printer: &mut Option<Printer>,
    emit: &mut impl FnMut(Event),
    name: &str,
    scan_secs: u64,
    request: CollageRequest,
) -> Result<()> {
    if !ensure_connected(printer, emit, name, scan_secs).await {
        anyhow::bail!("printer is not connected");
    }

    let data = collage::render_collage_jpeg(&request.paths, &request.layout, 1200, 1800)
        .context("could not prepare the collage")?;
    emit(Event::PrintPrepared { bytes: data.len() });

    send_print_data(printer, emit, &data, request.copies, "collage.jpg").await
}

async fn send_print_data(
    printer: &mut Option<Printer>,
    emit: &mut impl FnMut(Event),
    data: &[u8],
    copies: u32,
    document_name: &str,
) -> Result<()> {
    let job = PrintJob {
        copies,
        media_size: MEDIA_SIZE_2X3,
        ..PrintJob::default()
    };
    let params = PrintJobParams::photo(job, document_name, data.len() as u64);

    let p = printer.as_mut().context("printer disconnected")?;
    let response = p
        .request(methods::PRINT_JOB, json!(params), REQUEST_TIMEOUT)
        .await
        .context("print-job request failed")?;
    let job_id = response
        .job_id()
        .with_context(|| format!("printer did not return a job id: {response:?}"))?;

    let total_frames = frame_count(data.len());
    emit(Event::JobCreated {
        job_id,
        total_frames,
    });

    p.upload(job_id, data, FRAME_DELAY, |sent, total| {
        emit(Event::UploadProgress { sent, total });
    })
    .await
    .context("upload failed")?;

    tokio::time::sleep(Duration::from_secs(2)).await;

    let final_state = p
        .watch_job_with(job_id, REQUEST_TIMEOUT, |value| {
            emit(Event::JobState {
                value: value.clone(),
            });
        })
        .await
        .context("job tracking failed")?;

    emit(Event::PrintFinished { state: final_state });
    Ok(())
}
