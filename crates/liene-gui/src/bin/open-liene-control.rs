use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use liene_gui::control;
use serde_json::{json, Value};

#[derive(Parser, Debug)]
#[command(
    name = "open-liene-control",
    about = "Drive a running Open Liene window over its local control channel"
)]
struct Cli {
    /// Control discovery file (defaults to the app's temp file).
    #[arg(long, global = true)]
    file: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Print the current UI state as JSON.
    State,
    /// Capture the app framebuffer to a PNG.
    Screenshot { path: PathBuf },
    /// Close the desktop app.
    Quit,
    Image {
        #[command(subcommand)]
        command: ImageCommand,
    },
    Collage {
        #[command(subcommand)]
        command: CollageCommand,
    },
}

#[derive(Subcommand, Debug)]
enum ImageCommand {
    /// Open a single photo in the normal editor.
    Open { path: PathBuf },
}

#[derive(Subcommand, Debug)]
enum CollageCommand {
    /// Open a collage with 2–4 photos.
    Open { paths: Vec<PathBuf> },
    /// Select a layout by id (for example v3 or grid4).
    Layout { id: String },
    /// Toggle white borders between collage photos.
    Borders { state: Toggle },
    /// Shuffle the collage photos.
    Randomize,
    /// Open the print preview screen.
    PrintPreview,
    /// Print the collage.
    Print {
        #[arg(long, default_value_t = 1)]
        copies: u32,
    },
    Frame {
        #[command(subcommand)]
        command: FrameCommand,
    },
}

#[derive(Subcommand, Debug)]
enum FrameCommand {
    /// Set one photo's zoom/pan/rotation.
    Set {
        index: usize,
        #[arg(long, default_value_t = 1.0)]
        zoom: f32,
        #[arg(long, default_value_t = 0.0)]
        pan_x: f32,
        #[arg(long, default_value_t = 0.0)]
        pan_y: f32,
        #[arg(long, default_value_t = 0)]
        rotation: u32,
    },
    /// Reset one photo's framing.
    Reset { index: usize },
    /// Rotate one photo by 90 degrees.
    Rotate { index: usize },
    /// Replace one photo using a file path.
    Replace { index: usize, path: PathBuf },
    /// Open the frame editor for one photo.
    Open { index: usize },
    /// Close the frame editor.
    Close,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Toggle {
    On,
    Off,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let file = cli.file.unwrap_or_else(control::default_discovery_path);
    let (action, params): (&str, Value) = match cli.command {
        Command::State => ("state", Value::Null),
        Command::Screenshot { path } => ("ui.screenshot", json!({ "path": path })),
        Command::Quit => ("ui.quit", Value::Null),
        Command::Image { command } => match command {
            ImageCommand::Open { path } => ("image.open", json!({ "path": path })),
        },
        Command::Collage { command } => match command {
            CollageCommand::Open { paths } => ("collage.open", json!({ "paths": paths })),
            CollageCommand::Layout { id } => ("collage.layout", json!({ "id": id })),
            CollageCommand::Borders { state } => (
                "collage.borders",
                json!({ "enabled": matches!(state, Toggle::On) }),
            ),
            CollageCommand::Randomize => ("collage.randomize", Value::Null),
            CollageCommand::PrintPreview => ("collage.print_preview", Value::Null),
            CollageCommand::Print { copies } => ("collage.print", json!({ "copies": copies })),
            CollageCommand::Frame { command } => match command {
                FrameCommand::Set {
                    index,
                    zoom,
                    pan_x,
                    pan_y,
                    rotation,
                } => (
                    "collage.frame.set",
                    json!({
                        "index": index,
                        "zoom": zoom,
                        "pan_x": pan_x,
                        "pan_y": pan_y,
                        "rotation": rotation
                    }),
                ),
                FrameCommand::Reset { index } => ("collage.frame.reset", json!({ "index": index })),
                FrameCommand::Rotate { index } => {
                    ("collage.frame.rotate", json!({ "index": index }))
                }
                FrameCommand::Replace { index, path } => (
                    "collage.frame.replace",
                    json!({ "index": index, "path": path }),
                ),
                FrameCommand::Open { index } => ("collage.frame.open", json!({ "index": index })),
                FrameCommand::Close => ("collage.frame.close", Value::Null),
            },
        },
    };

    let response = control::send(&file, action, params)?;
    if let Some(state) = response.state {
        println!("{}", serde_json::to_string_pretty(&state)?);
    } else if let Some(message) = response.message {
        println!("{message}");
    }
    Ok(())
}
