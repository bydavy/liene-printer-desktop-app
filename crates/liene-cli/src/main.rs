use liene_client::{daemon, image_prep, pairing, printer, NormalizedRect, PhotoEdit};

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use liene_protocol::{methods, JobInfoParams, PrintJob, MEDIA_SIZE_2X3};
use serde_json::{json, Value};

use printer::Printer;

/// 2× the printer's native 300 dpi output.
const PRINT_WIDTH: u32 = 1200;
const PRINT_HEIGHT: u32 = 1800;

#[derive(Parser, Debug)]
#[command(
    name = "liene-n200",
    version,
    about = "Print photos on Liene/Hannto photo printers over Bluetooth"
)]
struct Cli {
    /// Device name prefix to look for (case-insensitive).
    #[arg(long, global = true, default_value = "Liene")]
    name: String,

    /// How long to scan for an unpaired device.
    #[arg(long, global = true, default_value_t = 12)]
    scan_secs: u64,

    /// Overall timeout for a request/response exchange.
    #[arg(long, global = true, default_value_t = 10)]
    timeout_secs: u64,

    /// Log every frame sent and received.
    #[arg(long, global = true)]
    debug: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// List paired classic Bluetooth devices, and optionally scan.
    Devices {
        /// Also perform a classic Bluetooth inquiry.
        #[arg(long)]
        scan: bool,

        /// Show every device, including names that do not match --name.
        #[arg(long)]
        all: bool,
    },

    /// Pair a device by Bluetooth address (e.g. 11:22:33:44:55:66).
    Pair { address: String },

    /// Scan BLE advertisements (diagnostic; data normally flows over SPP).
    Ble {
        /// Connect to the first match and enumerate GATT services.
        #[arg(long)]
        connect: bool,
    },

    /// Render the edited photo to a JPEG without printing (for previews
    /// and debugging).
    Prepare {
        image: PathBuf,

        /// Output JPEG path.
        #[arg(short, long, default_value = "prepared.jpg")]
        out: PathBuf,

        #[arg(long, default_value = "contain")]
        fit: String,

        #[arg(long, default_value_t = 0)]
        rotate: u16,

        #[arg(long)]
        crop: Option<String>,
    },

    /// Query printer state properties.
    Status,

    /// Query device information.
    Info,

    /// Print an image file (JPEG or PNG).
    Print {
        image: PathBuf,

        /// Number of copies (1-9).
        #[arg(short, long, default_value_t = 1)]
        copies: u32,

        /// Fit mode: contain (letterbox, default) or cover (crop to fill).
        #[arg(long, default_value = "contain")]
        fit: String,

        /// Delay between 918-byte upload frames, so the printer's RFCOMM
        /// receive buffer can drain.
        #[arg(long, default_value_t = 20)]
        chunk_delay_ms: u64,

        /// Rotate clockwise before printing (0, 90, 180, or 270 degrees).
        #[arg(long, default_value_t = 0)]
        rotate: u16,

        /// Crop after rotation, as normalized x,y,w,h (e.g. 0.1,0.1,0.8,0.8).
        #[arg(long)]
        crop: Option<String>,

        /// Do not wait for the print to finish.
        #[arg(long)]
        no_wait: bool,
    },

    /// List job ids known to the printer.
    Jobs,

    /// Query information for one job.
    Job { id: u32 },

    /// Resume the printer after a recoverable error (paper, ribbon, ...).
    Resume,

    /// Cancel a job.
    Cancel { id: u32 },

    /// Run the JSON-line daemon used by the GUI (one command per line on
    /// stdin, one event per line on stdout).
    #[command(hide = true)]
    Serve,

    /// Send an arbitrary method with JSON params.
    Raw {
        method: String,

        /// JSON value for "params" (default: []).
        #[arg(default_value = "[]")]
        params: String,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    platform::run(cli)
}

#[cfg(target_os = "macos")]
mod platform {
    use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
    use std::sync::Arc;

    use objc2_foundation::{NSDate, NSRunLoop};

    use super::Cli;

    pub fn run(cli: Cli) -> std::process::ExitCode {
        let done = Arc::new(AtomicBool::new(false));
        let code = Arc::new(AtomicI32::new(1));

        let worker_done = Arc::clone(&done);
        let worker_code = Arc::clone(&code);
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            let result = runtime.block_on(super::async_main(cli));
            match result {
                Ok(()) => worker_code.store(0, Ordering::SeqCst),
                Err(err) => {
                    eprintln!("error: {err:#}");
                    worker_code.store(1, Ordering::SeqCst);
                }
            }
            worker_done.store(true, Ordering::SeqCst);
        });

        // IOBluetooth dispatches callbacks onto the main queue, which is
        // serviced by the main run loop. Keep it pumping while the worker
        // thread drives the async protocol.
        while !done.load(Ordering::SeqCst) {
            let date = NSDate::dateWithTimeIntervalSinceNow(0.02);
            NSRunLoop::currentRunLoop().runUntilDate(&date);
        }

        std::process::ExitCode::from(code.load(Ordering::SeqCst) as u8)
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::Cli;

    pub fn run(cli: Cli) -> std::process::ExitCode {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(err) => {
                eprintln!("error: {err}");
                return std::process::ExitCode::FAILURE;
            }
        };
        match runtime.block_on(super::async_main(cli)) {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("error: {err:#}");
                std::process::ExitCode::FAILURE
            }
        }
    }
}

fn build_edit(rotate: u16, crop: Option<String>) -> Result<PhotoEdit> {
    if !matches!(rotate % 360, 0 | 90 | 180 | 270) {
        bail!("rotation must be 0, 90, 180, or 270");
    }
    let mut edit = PhotoEdit {
        rotation: rotate % 360,
        ..PhotoEdit::default()
    };
    if let Some(crop) = crop {
        edit.crop = parse_crop(&crop)?;
    }
    Ok(edit)
}

fn parse_crop(value: &str) -> Result<NormalizedRect> {
    let parts: Vec<f32> = value
        .split(',')
        .map(|part| {
            part.trim()
                .parse::<f32>()
                .with_context(|| format!("invalid crop component '{}'", part.trim()))
        })
        .collect::<Result<_>>()?;
    if parts.len() != 4 {
        bail!("crop must be four comma-separated numbers: x,y,w,h");
    }
    Ok(NormalizedRect {
        x: parts[0],
        y: parts[1],
        w: parts[2],
        h: parts[3],
    }
    .clamped())
}

async fn async_main(cli: Cli) -> Result<()> {
    let timeout = Duration::from_secs(cli.timeout_secs);

    match cli.command {
        Command::Devices { scan, all } => {
            let devices = printer::list_devices(&cli.name, scan, cli.scan_secs, all).await?;
            if devices.is_empty() {
                println!("no matching devices");
            }
            for device in devices {
                println!(
                    "{}\t{}\t{}",
                    device.device.id,
                    device.device.name.as_deref().unwrap_or("<unknown>"),
                    if device.is_paired {
                        "paired"
                    } else {
                        "unpaired"
                    }
                );
            }
        }

        Command::Pair { address } => {
            pairing::pair_device(&address).await?;
            println!("paired {address}");
        }

        Command::Ble { connect } => {
            let found = printer::ble_scan(&cli.name, cli.scan_secs).await?;
            if found.is_empty() {
                println!("no matching BLE devices");
            }
            for result in &found {
                println!(
                    "{}\t{}\trssi={:?}\tservices={:?}\tmanufacturer={:?}",
                    result.device.id,
                    result.device.name.as_deref().unwrap_or("<unknown>"),
                    result.device.rssi,
                    result.service_uuids,
                    result.manufacturer_data,
                );
            }

            if connect {
                if let Some(first) = found.first() {
                    let (id, services) = printer::ble_services(first.device.id.as_str()).await?;
                    println!("# GATT services for {id}");
                    for service in services {
                        println!("service {}", service.uuid);
                        for characteristic in service.characteristics {
                            println!(
                                "  characteristic {} read={} write={} write_no_rsp={} notify={} indicate={}",
                                characteristic.uuid,
                                characteristic.properties.read,
                                characteristic.properties.write,
                                characteristic.properties.write_without_response,
                                characteristic.properties.notify,
                                characteristic.properties.indicate,
                            );
                        }
                    }
                }
            }
        }

        Command::Prepare {
            image,
            out,
            fit,
            rotate,
            crop,
        } => {
            let fit = image_prep::Fit::parse(&fit)?;
            let edit = build_edit(rotate, crop)?;
            let data =
                image_prep::prepare_jpeg_with_edit(&image, PRINT_WIDTH, PRINT_HEIGHT, fit, &edit)?;
            std::fs::write(&out, &data)
                .with_context(|| format!("could not write {}", out.display()))?;
            println!(
                "{} bytes -> {} ({}x{}, rotation {}°, crop {:.2},{:.2} {:.2}x{:.2})",
                data.len(),
                out.display(),
                PRINT_WIDTH,
                PRINT_HEIGHT,
                edit.normalized_rotation(),
                edit.crop.x,
                edit.crop.y,
                edit.crop.w,
                edit.crop.h,
            );
        }

        Command::Status => {
            let mut printer = Printer::connect(&cli.name, cli.scan_secs, cli.debug).await?;
            let value = printer
                .request_value(
                    methods::GET_PROP,
                    json!([
                        "printer-state",
                        "printer-sub-state",
                        "printer-state-alerts",
                        "media-size",
                        "battery"
                    ]),
                    timeout,
                )
                .await?;
            println!("{}", serde_json::to_string_pretty(&value)?);
        }

        Command::Info => {
            let mut printer = Printer::connect(&cli.name, cli.scan_secs, cli.debug).await?;
            let value = printer
                .request_value(
                    methods::GET_PROP,
                    json!([
                        "model",
                        "mac-address",
                        "serial-number",
                        "sn-pcba",
                        "firmware-revision",
                        "hardware-revision",
                        "media-size",
                        "auto-off-interval"
                    ]),
                    timeout,
                )
                .await?;
            println!("{}", serde_json::to_string_pretty(&value)?);
        }

        Command::Print {
            image,
            copies,
            fit,
            rotate,
            crop,
            chunk_delay_ms,
            no_wait,
        } => {
            if !(1..=9).contains(&copies) {
                bail!("copies must be between 1 and 9");
            }
            let fit = image_prep::Fit::parse(&fit)?;
            let edit = build_edit(rotate, crop)?;
            let data =
                image_prep::prepare_jpeg_with_edit(&image, PRINT_WIDTH, PRINT_HEIGHT, fit, &edit)?;
            eprintln!(
                "# prepared {} ({}x{}, {} bytes)",
                image.display(),
                PRINT_WIDTH,
                PRINT_HEIGHT,
                data.len()
            );

            let mut printer = Printer::connect(&cli.name, cli.scan_secs, cli.debug).await?;
            let job = PrintJob {
                copies,
                media_size: MEDIA_SIZE_2X3,
                ..PrintJob::default()
            };
            let document_name = "liene-print.jpg";
            let params =
                liene_protocol::PrintJobParams::photo(job, document_name, data.len() as u64);

            eprintln!("# creating print job");
            let response = printer
                .request(methods::PRINT_JOB, json!(params), timeout)
                .await?;
            let job_id = response
                .job_id()
                .with_context(|| format!("printer did not return a job id: {response:?}"))?;
            eprintln!("# job id {job_id}; uploading {} bytes", data.len());

            printer
                .upload(
                    job_id,
                    &data,
                    Duration::from_millis(chunk_delay_ms),
                    |sent, total| {
                        if sent == total || sent % 50 == 0 {
                            eprintln!("# uploaded {sent}/{total} frames");
                        }
                    },
                )
                .await?;

            // Give the controller and printer time to drain the last frames
            // before the link is torn down.
            tokio::time::sleep(Duration::from_secs(2)).await;
            eprintln!("# upload complete; printer should start automatically");

            if !no_wait {
                printer.watch_job(job_id, timeout).await?;
            } else {
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
            printer.close().await;
        }

        Command::Jobs => {
            let mut printer = Printer::connect(&cli.name, cli.scan_secs, cli.debug).await?;
            let value = match printer
                .request_value(methods::GET_JOB_ID_LIST, Value::Null, timeout)
                .await
            {
                Ok(value) => value,
                Err(err) => {
                    eprintln!("# get-job-id-list failed ({err:#}); trying job-id-list");
                    printer
                        .request_value(methods::JOB_ID_LIST, Value::Null, timeout)
                        .await?
                }
            };
            println!("{}", serde_json::to_string_pretty(&value)?);
        }

        Command::Job { id } => {
            let mut printer = Printer::connect(&cli.name, cli.scan_secs, cli.debug).await?;
            let value = printer
                .request_value(
                    methods::GET_JOB_INFO,
                    json!(JobInfoParams { job_id: id }),
                    timeout,
                )
                .await?;
            println!("{}", serde_json::to_string_pretty(&value)?);
        }

        Command::Resume => {
            let mut printer = Printer::connect(&cli.name, cli.scan_secs, cli.debug).await?;
            let value = printer
                .request_value(methods::RESUME_PRINTER, json!([]), timeout)
                .await?;
            println!("{}", serde_json::to_string_pretty(&value)?);
        }

        Command::Cancel { id } => {
            let mut printer = Printer::connect(&cli.name, cli.scan_secs, cli.debug).await?;
            let value = printer
                .request_value(methods::CANCEL_JOB, json!([id]), timeout)
                .await?;
            println!("{}", serde_json::to_string_pretty(&value)?);
        }

        Command::Serve => {
            use std::io::{BufRead, Write};

            let (cmd_tx, cmd_rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let stdin = std::io::stdin();
                for line in stdin.lock().lines() {
                    let Ok(line) = line else { break };
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    match serde_json::from_str::<daemon::Command>(line) {
                        Ok(command) => {
                            if cmd_tx.send(command).is_err() {
                                break;
                            }
                        }
                        Err(err) => eprintln!("serve: invalid command: {err}"),
                    }
                }
            });

            daemon::run(cmd_rx, |event| {
                if let Ok(line) = serde_json::to_string(&event) {
                    let stdout = std::io::stdout();
                    let mut out = stdout.lock();
                    let _ = writeln!(out, "{line}");
                    let _ = out.flush();
                }
            })
            .await;
        }

        Command::Raw { method, params } => {
            let params: Value =
                serde_json::from_str(&params).with_context(|| format!("invalid JSON: {params}"))?;
            let mut printer = Printer::connect(&cli.name, cli.scan_secs, cli.debug).await?;
            let value = printer.request_value(&method, params, timeout).await?;
            println!("{}", serde_json::to_string_pretty(&value)?);
        }
    }

    Ok(())
}
