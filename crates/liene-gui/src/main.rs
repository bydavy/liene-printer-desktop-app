#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Stdio};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;
use egui::{Color32, RichText};
use liene_client::collage::{self, CollageLayout, PhotoFrame};
use liene_client::daemon::{CollageRequest, Command, Event, PrintRequest};
use liene_client::{Fit, NormalizedRect, PhotoEdit};
use liene_gui::control::{self, ControlJob, Response};
use serde_json::{json, Value};

const MAX_ZOOM: f32 = 8.0;
const PRINT_WIDTH: u32 = 1200;
const PRINT_HEIGHT: u32 = 1800;

/// Theme-aware colors for the parts of the UI that egui does not style.
///
/// egui switches between its dark and light `Visuals` when the OS appearance
/// changes; this palette makes the app's custom cards, status colors, and
/// error banner follow the same switch.
#[derive(Clone, Copy)]
struct Palette {
    green: Color32,
    red: Color32,
    amber: Color32,
    muted: Color32,
    card: Color32,
    drop_fill: Color32,
    error_fill: Color32,
    error_text: Color32,
    accent: Color32,
    disabled_fill: Color32,
    border: Color32,
}

impl Palette {
    fn of(ui: &egui::Ui) -> Self {
        if ui.visuals().dark_mode {
            Self::DARK
        } else {
            Self::LIGHT
        }
    }

    const DARK: Self = Self {
        green: Color32::from_rgb(0x6c, 0xc6, 0x7a),
        red: Color32::from_rgb(0xe5, 0x6b, 0x6f),
        amber: Color32::from_rgb(0xe6, 0xb4, 0x50),
        muted: Color32::from_rgb(0x9a, 0xa0, 0xa6),
        card: Color32::from_rgb(0x25, 0x28, 0x2d),
        drop_fill: Color32::from_rgb(0x22, 0x33, 0x2c),
        error_fill: Color32::from_rgb(0x3a, 0x1d, 0x20),
        error_text: Color32::from_rgb(0xff, 0xc9, 0xcc),
        accent: Color32::from_rgb(0x2f, 0x6b, 0x9e),
        disabled_fill: Color32::from_rgb(0x2c, 0x30, 0x36),
        border: Color32::from_rgb(0x3c, 0x42, 0x4a),
    };

    // Darker status colors and muted text keep AA-level contrast on white.
    const LIGHT: Self = Self {
        green: Color32::from_rgb(0x1f, 0x7a, 0x3d),
        red: Color32::from_rgb(0xb4, 0x23, 0x18),
        amber: Color32::from_rgb(0x94, 0x62, 0x00),
        muted: Color32::from_rgb(0x5b, 0x62, 0x6b),
        card: Color32::WHITE,
        drop_fill: Color32::from_rgb(0xe3, 0xf2, 0xe8),
        error_fill: Color32::from_rgb(0xfd, 0xec, 0xeb),
        error_text: Color32::from_rgb(0x8f, 0x1d, 0x18),
        accent: Color32::from_rgb(0x25, 0x63, 0xa8),
        disabled_fill: Color32::from_rgb(0xe9, 0xec, 0xf0),
        border: Color32::from_rgb(0xcf, 0xd4, 0xdb),
    };
}

fn dark_visuals() -> egui::Visuals {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = Color32::from_rgb(0x1c, 0x1e, 0x22);
    visuals.window_fill = visuals.panel_fill;
    visuals.extreme_bg_color = Color32::from_rgb(0x14, 0x16, 0x19);
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(0x2e, 0x32, 0x38);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(0x31, 0x35, 0x3b);
    visuals.widgets.active.bg_fill = Color32::from_rgb(0x38, 0x3d, 0x44);
    visuals.widgets.noninteractive.bg_stroke =
        egui::Stroke::new(1.0, Color32::from_rgb(0x3c, 0x42, 0x4a));
    visuals.selection.bg_fill = Color32::from_rgb(0x2f, 0x5e, 0x8a);
    // Selected labels/buttons use this as their foreground color.
    visuals.selection.stroke = egui::Stroke::new(1.0, Color32::WHITE);
    visuals
}

fn light_visuals() -> egui::Visuals {
    let mut visuals = egui::Visuals::light();
    visuals.panel_fill = Color32::from_rgb(0xf1, 0xf3, 0xf6);
    visuals.window_fill = visuals.panel_fill;
    visuals.extreme_bg_color = Color32::from_rgb(0xe7, 0xea, 0xef);
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(0xe6, 0xe9, 0xed);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(0xdb, 0xe0, 0xe6);
    visuals.widgets.active.bg_fill = Color32::from_rgb(0xcf, 0xd5, 0xdd);
    visuals.widgets.noninteractive.bg_stroke =
        egui::Stroke::new(1.0, Color32::from_rgb(0xd5, 0xd9, 0xdf));
    visuals.selection.bg_fill = Color32::from_rgb(0x25, 0x63, 0xa8);
    visuals.selection.stroke = egui::Stroke::new(1.0, Color32::WHITE);
    visuals
}

fn main() -> eframe::Result<()> {
    let control_enabled = std::env::args().any(|arg| arg == "--control");
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1040.0, 760.0])
            .with_min_inner_size([960.0, 600.0])
            .with_title("Open Liene"),
        ..Default::default()
    };
    eframe::run_native(
        "Open Liene",
        options,
        Box::new(move |cc| Ok(Box::new(LieneApp::new(cc, control_enabled)))),
    )
}

struct ConnectedDevice {
    name: String,
    address: String,
}

struct CollageState {
    paths: Vec<PathBuf>,
    sources: Vec<image::RgbaImage>,
    textures: Vec<egui::TextureHandle>,
    frames: Vec<PhotoFrame>,
    layouts: Vec<CollageLayout>,
    selected: usize,
    borders: bool,
    copies: u32,
    editing_photo: Option<usize>,
}

struct JobUi {
    id: u32,
    total_frames: usize,
    state: Option<u32>,
    reason: Option<u64>,
    transfer_size: u64,
    file_size: u64,
    page: u64,
    print_time_ms: u64,
}

struct LieneApp {
    commands: Option<Mutex<ChildStdin>>,
    events: Receiver<Event>,
    _child: Option<Child>,

    device_prefix: String,
    scan_secs: u64,
    connected: Option<ConnectedDevice>,
    connecting: bool,
    busy: bool,
    status_text: String,

    status: Option<Value>,
    info: Option<Value>,
    last_poll: Instant,

    image_path: Option<PathBuf>,
    source: Option<Arc<image::DynamicImage>>,
    preview: Option<egui::TextureHandle>,
    preview_size: egui::Vec2,
    edit: PhotoEdit,
    aspect_lock: bool,
    crop_drag: Option<CropDrag>,
    crop_drag_start: Option<(egui::Pos2, NormalizedRect)>,

    copies: u32,
    fit: Fit,

    collage: Option<CollageState>,
    collage_previewing: bool,

    job: Option<JobUi>,
    upload: Option<(usize, usize)>,
    error: Option<String>,

    log: Vec<String>,
    auto_connect: bool,
    pair_address: String,
    screenshot_path: Option<PathBuf>,
    frames: u32,
    no_connect: bool,
    /// Keep the page at the top while taking a repeatable screenshot.
    pin_scroll_top: bool,
    control_rx: Option<Receiver<ControlJob>>,
    control_file: Option<PathBuf>,
}

fn status_dot(ui: &mut egui::Ui, color: Color32, filled: bool) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
    if filled {
        ui.painter().circle_filled(rect.center(), 4.0, color);
    } else {
        ui.painter()
            .circle_stroke(rect.center(), 3.5, egui::Stroke::new(1.5, color));
    }
}

impl LieneApp {
    fn new(cc: &eframe::CreationContext<'_>, control_enabled: bool) -> Self {
        // Follow the OS appearance by default; the env var is a screenshot /
        // diagnostics hook for forcing a theme without changing the system.
        match std::env::var("LIENE_THEME").as_deref() {
            Ok("light") => cc.egui_ctx.set_theme(egui::ThemePreference::Light),
            Ok("dark") => cc.egui_ctx.set_theme(egui::ThemePreference::Dark),
            _ => {}
        }

        // Custom palette for each theme, applied to both so runtime OS theme
        // changes work in either direction.
        for theme in [egui::Theme::Dark, egui::Theme::Light] {
            let mut style = (*cc.egui_ctx.style_of(theme)).clone();
            style.spacing.item_spacing = egui::vec2(10.0, 8.0);
            style.spacing.button_padding = egui::vec2(12.0, 7.0);
            cc.egui_ctx.set_style_of(theme, style);
        }
        cc.egui_ctx
            .set_visuals_of(egui::Theme::Dark, dark_visuals());
        cc.egui_ctx
            .set_visuals_of(egui::Theme::Light, light_visuals());

        let (control_rx, control_file) = if control_enabled {
            let path = control::default_discovery_path();
            match control::start_server(&path) {
                Ok((_discovery, rx)) => {
                    eprintln!("# UI control channel: {}", path.display());
                    (Some(rx), Some(path))
                }
                Err(err) => {
                    eprintln!("# could not start UI control channel: {err:#}");
                    (None, None)
                }
            }
        } else {
            (None, None)
        };

        let (commands, child, events, startup_error) = match spawn_helper() {
            Ok((stdin, child, events)) => (Some(Mutex::new(stdin)), Some(child), events, None),
            Err(err) => {
                let (_tx, events) = std::sync::mpsc::channel();
                (
                    None,
                    None,
                    events,
                    Some(format!("Could not start the Bluetooth helper: {err:#}")),
                )
            }
        };

        let mut app = Self {
            commands,
            events,
            _child: child,
            device_prefix: "Liene".to_owned(),
            scan_secs: 12,
            connected: None,
            connecting: false,
            busy: false,
            status_text: "Not connected".to_owned(),
            status: None,
            info: None,
            last_poll: Instant::now(),
            image_path: None,
            source: None,
            preview: None,
            preview_size: egui::Vec2::ZERO,
            edit: PhotoEdit::default(),
            aspect_lock: true,
            crop_drag: None,
            crop_drag_start: None,
            copies: 1,
            fit: Fit::Contain,
            collage: None,
            collage_previewing: false,
            job: None,
            upload: None,
            error: startup_error,
            log: Vec::new(),
            auto_connect: true,
            pair_address: String::new(),
            screenshot_path: std::env::var_os("LIENE_SCREENSHOT").map(PathBuf::from),
            frames: 0,
            no_connect: std::env::var_os("LIENE_NO_CONNECT").is_some(),
            pin_scroll_top: std::env::var_os("LIENE_SCREENSHOT").is_some(),
            control_rx,
            control_file,
        };
        if let Some(path) = std::env::var_os("LIENE_IMAGE") {
            app.load_image(PathBuf::from(path), &cc.egui_ctx);
        }
        if let Some(raw) = std::env::var_os("LIENE_COLLAGE") {
            let paths: Vec<PathBuf> = std::env::split_paths(&raw).collect();
            app.load_collage(paths, &cc.egui_ctx);
        }
        if std::env::var_os("LIENE_COLLAGE_PREVIEW").is_some() && app.collage.is_some() {
            app.collage_previewing = true;
        }
        if let Ok(value) = std::env::var("LIENE_COLLAGE_BORDERS") {
            let borders = !matches!(
                value.to_ascii_lowercase().as_str(),
                "off" | "0" | "false" | "none"
            );
            if let Some(collage) = app.collage.as_mut() {
                collage.borders = borders;
            }
        }
        if let Ok(value) = std::env::var("LIENE_COLLAGE_EDIT") {
            if let Ok(index) = value.parse::<usize>() {
                if let Some(collage) = app.collage.as_mut() {
                    if index < collage.paths.len() {
                        collage.editing_photo = Some(index);
                    }
                }
            }
        }
        if let Ok(raw) = std::env::var("LIENE_COLLAGE_FRAME") {
            let parts: Vec<f32> = raw
                .split(',')
                .filter_map(|part| part.parse().ok())
                .collect();
            if parts.len() >= 4 {
                let index = parts[0] as usize;
                if let Some(collage) = app.collage.as_mut() {
                    if index < collage.frames.len() {
                        collage.frames[index] = PhotoFrame {
                            zoom: parts[1].clamp(1.0, 8.0),
                            pan_x: parts[2].clamp(-1.0, 1.0),
                            pan_y: parts[3].clamp(-1.0, 1.0),
                            rotation: 0,
                        };
                    }
                }
            }
        }
        if let Ok(raw) = std::env::var("LIENE_EDIT") {
            match serde_json::from_str::<PhotoEdit>(&raw) {
                Ok(edit) => {
                    app.edit = edit;
                    app.refresh_preview(&cc.egui_ctx);
                }
                Err(err) => eprintln!("invalid LIENE_EDIT: {err}"),
            }
        }
        if let Ok(lock) = std::env::var("LIENE_LOCK") {
            app.aspect_lock = !matches!(
                lock.to_ascii_lowercase().as_str(),
                "off" | "0" | "false" | "unlocked"
            );
        }
        if let Ok(fit) = std::env::var("LIENE_FIT") {
            app.fit = if fit.eq_ignore_ascii_case("fit") {
                Fit::Contain
            } else {
                Fit::Cover
            };
        }
        app.push_log("Open Liene ready.");
        app
    }

    fn push_log(&mut self, line: impl Into<String>) {
        self.log.push(line.into());
        if self.log.len() > 250 {
            let excess = self.log.len() - 250;
            self.log.drain(..excess);
        }
    }

    fn send(&self, command: Command) {
        let Some(stdin) = &self.commands else {
            return;
        };
        if let Ok(mut stdin) = stdin.lock() {
            if let Ok(line) = serde_json::to_string(&command) {
                let _ = writeln!(stdin, "{line}");
                let _ = stdin.flush();
            }
        }
    }

    fn handle_event(&mut self, event: Event, ctx: &egui::Context) {
        match event {
            Event::Busy { busy } => self.busy = busy,
            Event::Connecting { name } => {
                self.connecting = true;
                self.busy = true;
                self.status_text = format!("Connecting to {name}…");
                self.error = None;
                self.push_log(format!("Connecting to {name}…"));
            }
            Event::Connected { name, address } => {
                eprintln!("# GUI connected {name} ({address})");
                self.connecting = false;
                self.status_text = "Connected".to_owned();
                self.connected = Some(ConnectedDevice { name, address });
                self.error = None;
                self.last_poll = Instant::now();
                self.push_log("Connected over Bluetooth SPP.");
            }
            Event::Disconnected => {
                self.connected = None;
                self.connecting = false;
                self.status = None;
                self.info = None;
                self.job = None;
                self.upload = None;
                self.status_text = "Not connected".to_owned();
                self.push_log("Disconnected.");
            }
            Event::Status { value } => {
                eprintln!("# GUI status {value}");
                self.status = Some(value);
                self.connecting = false;
                self.last_poll = Instant::now();
            }
            Event::Info { value } => self.info = Some(value),
            Event::Devices { lines } => {
                if lines.is_empty() {
                    self.push_log("Scan finished: no devices found.");
                } else {
                    for line in lines {
                        self.push_log(format!("device: {line}"));
                    }
                }
            }
            Event::PrintPrepared { bytes } => {
                self.push_log(format!("Prepared JPEG ({bytes} bytes)."));
            }
            Event::JobCreated {
                job_id,
                total_frames,
            } => {
                self.job = Some(JobUi {
                    id: job_id,
                    total_frames,
                    state: None,
                    reason: None,
                    transfer_size: 0,
                    file_size: 0,
                    page: 0,
                    print_time_ms: 0,
                });
                self.upload = Some((0, total_frames));
                self.push_log(format!(
                    "Print job #{job_id} created ({total_frames} frames)."
                ));
            }
            Event::UploadProgress { sent, total } => self.upload = Some((sent, total)),
            Event::JobState { value } => {
                if let Some(job) = &mut self.job {
                    apply_job_state(job, &value);
                }
            }
            Event::PrintFinished { state } => {
                if let (Some(job), Some(state)) = (&mut self.job, state) {
                    apply_job_state(job, &state);
                }
                self.upload = None;
                match self.job.as_ref().and_then(|j| j.state) {
                    Some(9) => self.push_log("Print finished."),
                    Some(7) => self.push_log("Print aborted by the printer."),
                    Some(8) => self.push_log("Print canceled."),
                    _ => self.push_log("Print tracking ended."),
                }
                self.last_poll = Instant::now();
            }
            Event::Log { message } => self.push_log(message),
            Event::Error { message } => {
                eprintln!("# GUI error: {message}");
                self.error = Some(message.clone());
                self.connecting = false;
                self.push_log(format!("error: {message}"));
            }
        }
        ctx.request_repaint();
    }

    fn load_image(&mut self, path: PathBuf, ctx: &egui::Context) {
        self.collage = None;
        self.collage_previewing = false;
        match liene_client::image_prep::load_oriented(&path) {
            Ok(image) => {
                self.source = Some(Arc::new(image));
                self.edit = PhotoEdit::default();
                self.crop_drag = None;
                self.crop_drag_start = None;
                self.refresh_preview(ctx);
                if self.aspect_lock {
                    let width = self.preview_size.x.max(1.0) as u32;
                    let height = self.preview_size.y.max(1.0) as u32;
                    self.edit.set_zoom(width, height, 1.0, false);
                }
                self.push_log(format!("Loaded {}", path.display()));
                self.image_path = Some(path);
                self.error = None;
                self.job = None;
                self.upload = None;
            }
            Err(err) => {
                self.error = Some(format!("Could not load {}: {err}", path.display()));
            }
        }
    }

    fn load_collage(&mut self, paths: Vec<PathBuf>, ctx: &egui::Context) {
        let paths: Vec<PathBuf> = paths
            .into_iter()
            .filter(|path| path.is_file())
            .take(4)
            .collect();
        if paths.len() < 2 {
            self.error = Some("Choose at least two photos for a collage.".to_owned());
            return;
        }

        let mut sources = Vec::with_capacity(paths.len());
        let mut textures = Vec::with_capacity(paths.len());
        for (index, path) in paths.iter().enumerate() {
            match liene_client::image_prep::load_oriented(path) {
                Ok(image) => {
                    let thumbnail = image.thumbnail(1200, 1200);
                    let rgba = thumbnail.to_rgba8();
                    let size = [rgba.width() as usize, rgba.height() as usize];
                    let color = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
                    textures.push(ctx.load_texture(
                        format!("collage-{index}"),
                        color,
                        egui::TextureOptions::LINEAR,
                    ));
                    sources.push(rgba);
                }
                Err(err) => {
                    self.error = Some(format!("Could not load {}: {err}", path.display()));
                    return;
                }
            }
        }

        let layouts = collage::layouts_for(paths.len());
        let count = paths.len();
        self.collage = Some(CollageState {
            paths,
            sources,
            textures,
            frames: vec![PhotoFrame::default(); count],
            layouts,
            selected: 0,
            borders: true,
            copies: 1,
            editing_photo: None,
        });
        self.collage_previewing = false;
        self.error = None;
        self.job = None;
        self.upload = None;
        self.push_log(format!("Loaded collage with {count} photos."));
    }

    /// Rebuild the preview texture from the source image, applying rotation.
    /// The crop is drawn as an overlay, not baked into the texture.
    fn refresh_preview(&mut self, ctx: &egui::Context) {
        let Some(source) = &self.source else {
            self.preview = None;
            self.preview_size = egui::Vec2::ZERO;
            return;
        };

        let thumbnail = source.thumbnail(1400, 1400);
        let rotated = self.edit.apply_rotation(&thumbnail);
        let rgba = rotated.to_rgba8();
        let size = [rgba.width() as usize, rgba.height() as usize];
        let color = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
        self.preview = Some(ctx.load_texture("photo-preview", color, egui::TextureOptions::LINEAR));
        self.preview_size = egui::vec2(size[0] as f32, size[1] as f32);
    }

    fn handle_drops(&mut self, ui: &egui::Ui) {
        let paths: Vec<PathBuf> = ui.ctx().input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| {
                    let path = f.path();
                    if path.as_os_str().is_empty() {
                        None
                    } else {
                        Some(path.to_path_buf())
                    }
                })
                .collect()
        });
        match paths.len() {
            0 => {}
            1 => self.load_image(paths.into_iter().next().unwrap(), ui.ctx()),
            _ => self.load_collage(paths, ui.ctx()),
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let palette = Palette::of(ui);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new("Open Liene").size(24.0).strong());
                ui.label(
                    RichText::new("Liene / Hannto photo printer")
                        .color(palette.muted)
                        .small(),
                );
            });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(10.0);
                if self.busy {
                    ui.spinner();
                }

                if self.connected.is_some() {
                    if ui.button("Disconnect").clicked() {
                        self.send(Command::Disconnect);
                    }
                    if ui.button("Refresh").clicked() {
                        self.send(Command::RefreshStatus);
                        self.send(Command::RefreshInfo);
                    }
                    let dot = if self.printer_has_alert() {
                        palette.red
                    } else {
                        palette.green
                    };
                    let name = self
                        .connected
                        .as_ref()
                        .map(|d| d.name.as_str())
                        .unwrap_or("printer");
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        ui.colored_label(dot, name);
                        status_dot(ui, dot, true);
                    });
                } else {
                    if ui
                        .add_enabled(!self.busy, egui::Button::new("Connect"))
                        .clicked()
                    {
                        self.send(Command::Connect {
                            name: self.device_prefix.clone(),
                            scan_secs: self.scan_secs,
                        });
                    }
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        ui.colored_label(palette.muted, "Disconnected");
                        status_dot(ui, palette.muted, false);
                    });
                }
            });
        });
    }

    fn printer_has_alert(&self) -> bool {
        self.status
            .as_ref()
            .and_then(parse_status)
            .is_some_and(|s| !s.alerts.is_empty() || s.state == 60)
    }

    fn error_banner(&mut self, ui: &mut egui::Ui) {
        let palette = Palette::of(ui);
        let Some(message) = self.error.clone() else {
            return;
        };
        let mut dismiss = false;
        let mut resume = false;
        egui::Frame::new()
            .fill(palette.error_fill)
            .corner_radius(8.0)
            .inner_margin(12.0)
            .show(ui, |ui| {
                // Allocate the buttons first so a long connection error wraps
                // in the remaining width instead of squeezing them out.
                egui::containers::Sides::new().shrink_left().wrap().show(
                    ui,
                    |ui| {
                        ui.label(RichText::new(message).color(palette.error_text));
                    },
                    |ui| {
                        if ui.button("Dismiss").clicked() {
                            dismiss = true;
                        }
                        if ui.button("Resume printer").clicked() {
                            resume = true;
                        }
                    },
                );
            });
        if dismiss {
            self.error = None;
        }
        if resume {
            self.send(Command::Resume);
        }
    }

    fn collage_panel(&mut self, ui: &mut egui::Ui) {
        let palette = Palette::of(ui);
        let Some((textures, frames, layouts, selected, count, borders)) =
            self.collage.as_ref().map(|collage| {
                (
                    collage.textures.clone(),
                    collage.frames.clone(),
                    collage.layouts.clone(),
                    collage.selected,
                    collage.paths.len(),
                    collage.borders,
                )
            })
        else {
            return;
        };
        let selected = selected.min(layouts.len().saturating_sub(1));

        ui.horizontal(|ui| {
            ui.label(RichText::new("Collage").size(20.0).strong());
            if ui.button("Close").clicked() {
                self.collage = None;
                self.collage_previewing = false;
            }
        });
        ui.add_space(8.0);

        ui.horizontal_top(|ui| {
            let total_width = ui.available_width();
            let right_width = (total_width - 380.0 - 14.0 - 12.0).max(360.0);
            ui.vertical(|ui| {
                ui.set_width(380.0);
                egui::Frame::new()
                    .fill(palette.card)
                    .corner_radius(12.0)
                    .inner_margin(16.0)
                    .show(ui, |ui| {
                        let render_layout = layout_for_render(&layouts[selected], count, borders);
                        let width = ui.available_width().clamp(220.0, 280.0);
                        ui.vertical_centered(|ui| {
                            let (rect, response) = ui.allocate_exact_size(
                                egui::vec2(width, width * 1.5),
                                egui::Sense::click(),
                            );
                            response.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::Role::Button,
                                    true,
                                    "Collage preview",
                                )
                            });
                            paint_collage(
                                &ui.painter_at(rect),
                                rect,
                                &textures,
                                &frames,
                                &render_layout,
                            );
                            if response.clicked() {
                                if let Some(position) = response.interact_pointer_pos() {
                                    let nx = (position.x - rect.left()) / rect.width().max(1.0);
                                    let ny = (position.y - rect.top()) / rect.height().max(1.0);
                                    if let Some(index) =
                                        render_layout.cells.iter().position(|cell| {
                                            nx >= cell.x
                                                && nx <= cell.x + cell.w
                                                && ny >= cell.y
                                                && ny <= cell.y + cell.h
                                        })
                                    {
                                        if let Some(collage) = self.collage.as_mut() {
                                            collage.editing_photo = Some(index);
                                        }
                                    }
                                }
                            }
                            response.on_hover_cursor(egui::CursorIcon::PointingHand);
                        });
                        ui.label(
                            RichText::new("Click a photo to zoom, rotate, or replace it.")
                                .color(palette.muted)
                                .small(),
                        );
                        ui.add_space(8.0);
                        let mut borders = borders;
                        let mut randomize = false;
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Layout").strong());
                            ui.add_space(16.0);
                            ui.checkbox(&mut borders, "Borders");
                            ui.add_space(12.0);
                            if ui.button("Randomize").clicked() {
                                randomize = true;
                            }
                        });
                        if borders != self.collage.as_ref().map(|c| c.borders).unwrap_or(true) {
                            if let Some(collage) = self.collage.as_mut() {
                                collage.borders = borders;
                            }
                        }
                        if randomize {
                            if let Some(collage) = self.collage.as_mut() {
                                shuffle_collage(collage);
                            }
                        }
                        let mut new_selected = selected;
                        ui.horizontal(|ui| {
                            for (index, layout) in layouts.iter().enumerate() {
                                if layout_icon(ui, layout, selected == index, &palette) {
                                    new_selected = index;
                                }
                            }
                        });
                        if new_selected != selected {
                            if let Some(collage) = self.collage.as_mut() {
                                collage.selected = new_selected;
                            }
                        }
                        ui.add_space(8.0);
                        let can_review = count > 0;
                        let print = egui::Button::new(RichText::new("Print").size(18.0).strong())
                            .min_size(egui::vec2(180.0, 42.0));
                        if ui.add_enabled(can_review, print).clicked() {
                            self.collage_previewing = true;
                        }
                        ui.label(
                            RichText::new(format!(
                                "{count} photos · {} · {PRINT_WIDTH}×{PRINT_HEIGHT} px",
                                layouts[selected].name
                            ))
                            .color(palette.muted)
                            .small(),
                        );
                    });
            });
            ui.add_space(14.0);
            ui.vertical(|ui| {
                ui.set_width(right_width);
                self.status_card(ui);
                ui.add_space(8.0);
                self.progress_ui(ui);
                ui.add_space(8.0);
                self.device_card(ui);
            });
        });
    }

    fn collage_photo_window(&mut self, ctx: &egui::Context) {
        let Some((index, mut source, mut texture, mut frame, cell_aspect, count)) =
            self.collage.as_ref().and_then(|collage| {
                let index = collage.editing_photo?;
                let source = collage.sources.get(index)?.clone();
                let texture = collage.textures.get(index)?.clone();
                let frame = *collage.frames.get(index)?;
                let layout = collage.layouts.get(collage.selected)?;
                let render = layout_for_render(layout, collage.paths.len(), collage.borders);
                let cell = render.cells.get(index)?;
                let aspect = (cell.w * 1200.0) / (cell.h * 1800.0);
                Some((index, source, texture, frame, aspect, collage.paths.len()))
            })
        else {
            return;
        };

        let mut viewport_width = 320.0_f32;
        let mut viewport_height = viewport_width / cell_aspect.max(0.05);
        if viewport_height > 420.0 {
            viewport_height = 420.0;
            viewport_width = viewport_height * cell_aspect.max(0.05);
        }

        let mut open = true;
        let mut done = false;
        let mut reset = false;
        let mut rotate = false;
        let mut replace = false;

        egui::Window::new(format!("Frame photo {} of {count}", index + 1))
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                let (rect, response) = ui.allocate_exact_size(
                    egui::vec2(viewport_width, viewport_height),
                    egui::Sense::click_and_drag(),
                );
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::Image, true, "Photo framing")
                });
                let local = photo_draw_rect(texture.size_vec2(), rect.size(), frame);
                let draw = local.translate(rect.min.to_vec2());
                ui.painter().with_clip_rect(rect).image(
                    texture.id(),
                    draw,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
                ui.painter().rect_stroke(
                    rect,
                    0.0,
                    egui::Stroke::new(1.0, Color32::from_gray(170)),
                    egui::StrokeKind::Inside,
                );
                let mut frame_changed = false;
                if response.dragged() {
                    let delta = response.drag_delta();
                    let slack_x = (local.width() - rect.width()).max(0.0);
                    let slack_y = (local.height() - rect.height()).max(0.0);
                    if slack_x > 0.0 {
                        frame.pan_x = (frame.pan_x + delta.x / (slack_x / 2.0)).clamp(-1.0, 1.0);
                        frame_changed = true;
                    }
                    if slack_y > 0.0 {
                        frame.pan_y = (frame.pan_y + delta.y / (slack_y / 2.0)).clamp(-1.0, 1.0);
                        frame_changed = true;
                    }
                }
                ui.add_space(8.0);
                if ui
                    .add(
                        egui::Slider::new(&mut frame.zoom, 1.0..=8.0)
                            .logarithmic(true)
                            .text("Zoom"),
                    )
                    .changed()
                {
                    frame_changed = true;
                }
                if frame_changed {
                    if let Some(collage) = self.collage.as_mut() {
                        collage.frames[index] = frame;
                    }
                    ui.ctx().request_repaint();
                }
                ui.label("Drag the photo to choose the part that prints.");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Rotate 90°").clicked() {
                        rotate = true;
                    }
                    if ui.button("Replace…").clicked() {
                        replace = true;
                    }
                    if ui.button("Reset").clicked() {
                        reset = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Done").clicked() {
                            done = true;
                        }
                    });
                });
            });

        if replace {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("Images", &["jpg", "jpeg", "png"])
                .pick_file()
            {
                match liene_client::image_prep::load_oriented(&path) {
                    Ok(image) => {
                        let rgba = image.thumbnail(1200, 1200).to_rgba8();
                        let new_texture =
                            texture_from_rgba(ctx, &rgba, format!("collage-{index}-replace"));
                        source = rgba;
                        frame = PhotoFrame::default();
                        if let Some(collage) = self.collage.as_mut() {
                            collage.paths[index] = path;
                            collage.sources[index] = source.clone();
                            collage.textures[index] = new_texture;
                            collage.frames[index] = frame;
                        }
                    }
                    Err(err) => {
                        self.error = Some(format!("Could not load {}: {err}", path.display()));
                    }
                }
            }
        }
        if reset {
            frame = PhotoFrame::default();
        }
        if rotate {
            frame.rotation = (frame.rotation + 90) % 360;
            frame.zoom = 1.0;
            frame.pan_x = 0.0;
            frame.pan_y = 0.0;
        }
        if reset || rotate {
            let rotated = rotate_rgba(&source, frame.rotation);
            texture = texture_from_rgba(
                ctx,
                &rotated,
                format!("collage-{index}-rot-{}", frame.rotation),
            );
            if let Some(collage) = self.collage.as_mut() {
                collage.textures[index] = texture;
            }
        }
        if let Some(collage) = self.collage.as_mut() {
            collage.frames[index] = frame;
            if done || !open {
                collage.editing_photo = None;
            }
        }
        ctx.request_repaint();
    }

    fn collage_print_window(&mut self, ctx: &egui::Context) {
        let Some((textures, frames, layout, paths, initial_copies, borders)) =
            self.collage.as_ref().map(|collage| {
                (
                    collage.textures.clone(),
                    collage.frames.clone(),
                    collage
                        .layouts
                        .get(collage.selected)
                        .cloned()
                        .unwrap_or_else(|| collage.layouts[0].clone()),
                    collage.paths.clone(),
                    collage.copies,
                    collage.borders,
                )
            })
        else {
            return;
        };

        let connected = self.connected.is_some();
        let busy = self.busy;
        let has_alert = self.printer_has_alert();
        let status = self.status.as_ref().and_then(parse_status);
        let status_text = if connected {
            status
                .as_ref()
                .map(|status| status.headline(&Palette::DARK).0)
                .unwrap_or("Connected")
        } else {
            "Not connected"
        };
        let battery = status
            .as_ref()
            .and_then(|status| status.battery)
            .map(battery_label);
        let printer_name = self
            .connected
            .as_ref()
            .map(|device| device.name.as_str())
            .unwrap_or("Printer");

        let preview_layout = layout_for_render(&layout, paths.len(), borders);
        let mut copies = initial_copies.clamp(1, 9);
        let mut open = true;
        let mut back = false;
        let mut print = false;
        let mut connect = false;
        let can_print = connected && !busy && !has_alert;

        egui::Window::new("Print preview")
            .collapsible(false)
            .resizable(false)
            .default_width(420.0)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .open(&mut open)
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    let width = 250.0;
                    let (rect, _) = ui
                        .allocate_exact_size(egui::vec2(width, width * 1.5), egui::Sense::hover());
                    paint_collage(
                        &ui.painter_at(rect),
                        rect,
                        &textures,
                        &frames,
                        &preview_layout,
                    );
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let label = ui.label("Copies");
                    ui.add(egui::DragValue::new(&mut copies).range(1..=9))
                        .labelled_by(label.id);
                });
                ui.add_space(4.0);
                let mut status_line = format!("{printer_name} · {status_text}");
                if let Some(battery) = &battery {
                    status_line.push_str(" · ");
                    status_line.push_str(battery);
                }
                ui.label(status_line);
                if has_alert {
                    ui.colored_label(
                        Palette::of(ui).red,
                        "Resolve the printer alert before printing.",
                    );
                }
                if !connected && ui.button("Connect printer").clicked() {
                    connect = true;
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Back").clicked() {
                        back = true;
                    }
                    let button = egui::Button::new(RichText::new("Print now").strong())
                        .min_size(egui::vec2(140.0, 36.0));
                    if ui.add_enabled(can_print, button).clicked() {
                        print = true;
                    }
                });
            });

        if let Some(collage) = self.collage.as_mut() {
            collage.copies = copies;
        }
        if connect {
            self.send(Command::Connect {
                name: self.device_prefix.clone(),
                scan_secs: self.scan_secs,
            });
        }
        if print {
            self.busy = true;
            self.job = None;
            self.upload = None;
            self.error = None;
            self.push_log(format!("Printing collage with {} photos…", paths.len()));
            self.send(Command::PrintCollage(CollageRequest {
                paths,
                layout: layout.id.to_owned(),
                copies,
                borders,
                frames,
            }));
            self.collage_previewing = false;
        } else if !open || back {
            self.collage_previewing = false;
        }
    }

    fn photo_panel(&mut self, ui: &mut egui::Ui) {
        let palette = Palette::of(ui);
        egui::Frame::new()
            .fill(palette.card)
            .corner_radius(12.0)
            .inner_margin(16.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Photo").size(18.0).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Choose…").clicked() {
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("Images", &["jpg", "jpeg", "png"])
                                .pick_file()
                            {
                                self.load_image(path, ui.ctx());
                            }
                        }
                        if ui.button("Collage…").clicked() {
                            if let Some(paths) = rfd::FileDialog::new()
                                .add_filter("Images", &["jpg", "jpeg", "png"])
                                .pick_files()
                            {
                                self.load_collage(paths, ui.ctx());
                            }
                        }
                    });
                });

                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    let mut needs_preview = false;
                    if ui
                        .add_enabled(self.preview.is_some(), egui::Button::new("Rotate 90°"))
                        .on_hover_text("Rotate clockwise in 90° steps")
                        .clicked()
                    {
                        self.edit.rotate_cw();
                        needs_preview = true;
                    }
                    if ui
                        .add_enabled(self.preview.is_some(), egui::Button::new("Reset"))
                        .clicked()
                    {
                        self.edit = PhotoEdit::default();
                        needs_preview = true;
                    }
                    if needs_preview && self.source.is_some() {
                        self.refresh_preview(ui.ctx());
                        if self.aspect_lock {
                            let width = self.preview_size.x.max(1.0) as u32;
                            let height = self.preview_size.y.max(1.0) as u32;
                            self.edit.snap_to_print_aspect(width, height);
                        }
                    }
                    ui.label(
                        RichText::new(format!("{}°", self.edit.normalized_rotation()))
                            .color(palette.muted)
                            .small(),
                    );
                });

                ui.horizontal_wrapped(|ui| {
                    if ui.checkbox(&mut self.aspect_lock, "Lock 2:3").changed()
                        && self.aspect_lock
                    {
                        let width = self.preview_size.x.max(1.0) as u32;
                        let height = self.preview_size.y.max(1.0) as u32;
                        self.edit.snap_to_print_aspect(width, height);
                    }

                    if self.aspect_lock {
                        let width = self.preview_size.x.max(1.0) as u32;
                        let height = self.preview_size.y.max(1.0) as u32;
                        let mut zoom = self.edit.zoom(width, height);
                        let response = ui.add(
                            egui::Slider::new(&mut zoom, 1.0..=MAX_ZOOM)
                                .logarithmic(true)
                                .show_value(false)
                                .text("Zoom"),
                        );
                        if response.changed() {
                            self.edit.set_zoom(width, height, zoom, true);
                        }
                        ui.label(
                            RichText::new(format!("{zoom:.1}×")).color(palette.muted).small(),
                        );
                    } else {
                        ui.label(
                            RichText::new("Unlock to drag any edge freely")
                                .color(palette.muted)
                                .small(),
                        );
                    }
                });

                ui.add_space(8.0);
                self.photo_preview(ui);

                if let Some(path) = &self.image_path {
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(path.display().to_string())
                            .color(palette.muted)
                            .small(),
                    );
                    ui.label(
                        RichText::new(
                            "Drag to frame. Lock 2:3 + Zoom keeps the print aspect; unlock for freeform cropping.",
                        )
                        .color(palette.muted)
                        .small(),
                    );
                }
            });
    }

    fn photo_preview(&mut self, ui: &mut egui::Ui) {
        let palette = Palette::of(ui);
        let drop_hover = ui.ctx().input(|i| !i.raw.hovered_files.is_empty());
        egui::Frame::new()
            .fill(if drop_hover {
                palette.drop_fill
            } else {
                ui.visuals().extreme_bg_color
            })
            .stroke(egui::Stroke::new(
                1.0,
                if drop_hover {
                    palette.green
                } else {
                    ui.visuals().widgets.noninteractive.bg_stroke.color
                },
            ))
            .show(ui, |ui| {
                let box_size = egui::vec2(370.0, 430.0);
                let (outer_rect, _) = ui.allocate_exact_size(box_size, egui::Sense::hover());
                let painter = ui.painter_at(outer_rect);

                let texture = self.preview.clone();
                if let Some(texture) = texture {
                    let texture_size = self.preview_size;
                    if texture_size.x > 0.0 && texture_size.y > 0.0 {
                        let scale = (box_size.x / texture_size.x)
                            .min(box_size.y / texture_size.y)
                            .min(1.0);
                        let display = texture_size * scale;
                        let image_rect = egui::Rect::from_center_size(outer_rect.center(), display);

                        painter.image(
                            texture.id(),
                            image_rect,
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            Color32::WHITE,
                        );

                        let response = ui.interact(
                            image_rect,
                            ui.scope_id().with("crop-overlay"),
                            egui::Sense::click_and_drag(),
                        );
                        response.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::Role::Image,
                                ui.is_enabled(),
                                "Photo crop",
                            )
                        });
                        self.crop_interactions(ui, &response, image_rect);
                        self.paint_crop_overlay(&painter, image_rect);
                    }
                } else {
                    painter.text(
                        outer_rect.center() + egui::vec2(0.0, -12.0),
                        egui::Align2::CENTER_CENTER,
                        "Drop a photo here",
                        egui::FontId::proportional(19.0),
                        palette.muted,
                    );
                    painter.text(
                        outer_rect.center() + egui::vec2(0.0, 14.0),
                        egui::Align2::CENTER_CENTER,
                        "or choose a JPEG/PNG file",
                        egui::FontId::proportional(12.0),
                        palette.muted,
                    );
                }
            });
    }

    fn crop_interactions(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        image_rect: egui::Rect,
    ) {
        let crop_rect = crop_to_screen(self.edit.crop.clamped(), image_rect);

        let corners_only = self.aspect_lock;
        if let Some(pos) = response.hover_pos() {
            if let Some(mode) = hit_test(pos, crop_rect, corners_only) {
                ui.ctx().set_cursor_icon(match mode {
                    CropDrag::Move if response.dragged() => egui::CursorIcon::Grabbing,
                    CropDrag::Move => egui::CursorIcon::Grab,
                    CropDrag::N | CropDrag::S => egui::CursorIcon::ResizeVertical,
                    CropDrag::E | CropDrag::W => egui::CursorIcon::ResizeHorizontal,
                    CropDrag::NE | CropDrag::SW => egui::CursorIcon::ResizeNeSw,
                    CropDrag::NW | CropDrag::SE => egui::CursorIcon::ResizeNwSe,
                });
            }
        }

        if response.drag_started() {
            if let Some(pos) = response.interact_pointer_pos() {
                if let Some(mode) = hit_test(pos, crop_rect, corners_only) {
                    self.crop_drag = Some(mode);
                    self.crop_drag_start = Some((pos, self.edit.crop.clamped()));
                }
            }
        }

        if response.dragged() {
            if let (Some(mode), Some((start_pos, start_crop))) =
                (self.crop_drag, self.crop_drag_start)
            {
                if let Some(pos) = response.interact_pointer_pos() {
                    if image_rect.width() > 0.0 && image_rect.height() > 0.0 {
                        let dx = (pos.x - start_pos.x) / image_rect.width();
                        let dy = (pos.y - start_pos.y) / image_rect.height();
                        let aspect = if self.aspect_lock {
                            let max = NormalizedRect::centered_aspect(
                                self.preview_size.x.max(1.0) as u32,
                                self.preview_size.y.max(1.0) as u32,
                                2.0 / 3.0,
                            );
                            Some((max.w / max.h, max.w, max.h))
                        } else {
                            None
                        };
                        self.edit.crop = resize_crop(start_crop, mode, dx, dy, aspect, MAX_ZOOM);
                    }
                }
            }
        }

        if response.drag_stopped() {
            self.crop_drag = None;
            self.crop_drag_start = None;
        }
    }

    fn paint_crop_overlay(&self, painter: &egui::Painter, image_rect: egui::Rect) {
        let crop_rect = crop_to_screen(self.edit.crop.clamped(), image_rect);
        let dim = Color32::from_black_alpha(150);

        paint_dim(
            painter,
            egui::Rect::from_min_max(
                image_rect.min,
                egui::pos2(image_rect.right(), crop_rect.top()),
            ),
            dim,
        );
        paint_dim(
            painter,
            egui::Rect::from_min_max(
                egui::pos2(image_rect.left(), crop_rect.bottom()),
                image_rect.max,
            ),
            dim,
        );
        paint_dim(
            painter,
            egui::Rect::from_min_max(
                egui::pos2(image_rect.left(), crop_rect.top()),
                egui::pos2(crop_rect.left(), crop_rect.bottom()),
            ),
            dim,
        );
        paint_dim(
            painter,
            egui::Rect::from_min_max(
                egui::pos2(crop_rect.right(), crop_rect.top()),
                egui::pos2(image_rect.right(), crop_rect.bottom()),
            ),
            dim,
        );

        painter.rect_stroke(
            crop_rect,
            0.0,
            egui::Stroke::new(2.0, Color32::WHITE),
            egui::StrokeKind::Middle,
        );

        let third = Color32::from_white_alpha(70);
        for i in 1..3 {
            let fraction = i as f32 / 3.0;
            let x = crop_rect.left() + crop_rect.width() * fraction;
            let y = crop_rect.top() + crop_rect.height() * fraction;
            painter.line_segment(
                [
                    egui::pos2(x, crop_rect.top()),
                    egui::pos2(x, crop_rect.bottom()),
                ],
                egui::Stroke::new(1.0, third),
            );
            painter.line_segment(
                [
                    egui::pos2(crop_rect.left(), y),
                    egui::pos2(crop_rect.right(), y),
                ],
                egui::Stroke::new(1.0, third),
            );
        }

        let mut handles = vec![
            (crop_rect.left(), crop_rect.top()),
            (crop_rect.right(), crop_rect.top()),
            (crop_rect.left(), crop_rect.bottom()),
            (crop_rect.right(), crop_rect.bottom()),
        ];
        if !self.aspect_lock {
            handles.extend([
                (crop_rect.center().x, crop_rect.top()),
                (crop_rect.right(), crop_rect.center().y),
                (crop_rect.center().x, crop_rect.bottom()),
                (crop_rect.left(), crop_rect.center().y),
            ]);
        }
        for (x, y) in handles {
            painter.rect_filled(
                egui::Rect::from_center_size(egui::pos2(x, y), egui::vec2(9.0, 9.0)),
                2.0,
                Color32::WHITE,
            );
        }
    }

    /// Draw a live 2:3 print preview from the rotated preview texture and
    /// the current crop. Fit mode letterboxes with white exactly like the
    /// print pipeline does.
    fn paint_print_preview(&self, ui: &mut egui::Ui, width: f32) {
        let height = width * 1.5;
        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
        let painter = ui.painter_at(rect);

        // White paper; for Fit the padding remains visible around the image.
        painter.rect_filled(rect, 0.0, Color32::WHITE);

        let texture = self.preview.clone();
        if let Some(texture) = texture {
            let crop = self.edit.crop.clamped();
            let tex = self.preview_size;
            if tex.x > 0.0
                && tex.y > 0.0
                && crop.w > 0.0
                && crop.h > 0.0
                && rect.width() > 0.0
                && rect.height() > 0.0
            {
                let crop_aspect = (crop.w * tex.x) / (crop.h * tex.y);
                let target_aspect = rect.width() / rect.height();
                let crop_uv = egui::Rect::from_min_max(
                    egui::pos2(crop.x, crop.y),
                    egui::pos2(crop.x + crop.w, crop.y + crop.h),
                );

                match self.fit {
                    Fit::Cover => {
                        // Crop a centered sub-rect of the framing selection so
                        // it fills the 2:3 paper.
                        let uv = if crop_aspect > target_aspect {
                            let w = crop.w * (target_aspect / crop_aspect);
                            let x = crop.x + (crop.w - w) * 0.5;
                            egui::Rect::from_min_max(
                                egui::pos2(x, crop.y),
                                egui::pos2(x + w, crop.y + crop.h),
                            )
                        } else {
                            let h = crop.h * (crop_aspect / target_aspect);
                            let y = crop.y + (crop.h - h) * 0.5;
                            egui::Rect::from_min_max(
                                egui::pos2(crop.x, y),
                                egui::pos2(crop.x + crop.w, y + h),
                            )
                        };
                        painter.image(texture.id(), rect, uv, Color32::WHITE);
                    }
                    Fit::Contain => {
                        // Fit the whole framing selection inside the paper;
                        // the white background shows as borders.
                        let image_rect = if crop_aspect > target_aspect {
                            let image_height = rect.width() / crop_aspect;
                            egui::Rect::from_center_size(
                                rect.center(),
                                egui::vec2(rect.width(), image_height),
                            )
                        } else {
                            let image_width = rect.height() * crop_aspect;
                            egui::Rect::from_center_size(
                                rect.center(),
                                egui::vec2(image_width, rect.height()),
                            )
                        };
                        painter.image(texture.id(), image_rect, crop_uv, Color32::WHITE);
                    }
                }
            }
        } else {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "No photo",
                egui::FontId::proportional(12.0),
                Color32::from_gray(160),
            );
        }

        painter.rect_stroke(
            rect,
            0.0,
            egui::Stroke::new(1.0, Color32::from_gray(190)),
            egui::StrokeKind::Inside,
        );
    }

    fn settings_panel(&mut self, ui: &mut egui::Ui) {
        let palette = Palette::of(ui);
        self.status_card(ui);
        ui.add_space(8.0);

        egui::Frame::new()
            .fill(palette.card)
            .corner_radius(12.0)
            .inner_margin(16.0)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("Print settings").size(18.0).strong());
                ui.add_space(8.0);

                let available = ui.available_width();
                let preview_width = (available * 0.26).clamp(100.0, 140.0);
                let left_width = (available - preview_width - 16.0).max(240.0);

                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(left_width);

                        ui.horizontal(|ui| {
                            let label = ui.label("Copies");
                            ui.add(
                                egui::DragValue::new(&mut self.copies)
                                    .range(1..=9)
                                    .speed(0.1),
                            )
                            .labelled_by(label.id);
                            ui.label(RichText::new("1–9").color(palette.muted).small());
                        });

                        ui.horizontal(|ui| {
                            ui.label("Framing");
                            ui.selectable_value(&mut self.fit, Fit::Cover, "Fill")
                                .on_hover_text("Scale and crop to fill the 2×3 print.");
                            ui.selectable_value(&mut self.fit, Fit::Contain, "Fit")
                                .on_hover_text("Keep the whole image, with white borders.");
                        });

                        ui.add_space(12.0);
                        let (width, height) = (PRINT_WIDTH, PRINT_HEIGHT);
                        ui.label(
                            RichText::new(format!("Output: {width} × {height} px, 2×3 in"))
                                .color(palette.muted)
                                .small(),
                        );

                        ui.add_space(10.0);
                        let can_print = self.connected.is_some()
                            && self.image_path.is_some()
                            && !self.busy
                            && !self.printer_has_alert();
                        let button =
                            egui::Button::new(RichText::new("Print").size(19.0).strong().color(
                                if can_print {
                                    Color32::WHITE
                                } else {
                                    palette.muted
                                },
                            ))
                            .min_size(egui::vec2(170.0, 44.0))
                            .fill(if can_print {
                                palette.accent
                            } else {
                                palette.disabled_fill
                            })
                            .stroke(egui::Stroke::new(1.0, palette.border));
                        if ui.add_enabled(can_print, button).clicked() {
                            if let Some(path) = self.image_path.clone() {
                                self.busy = true;
                                self.job = None;
                                self.upload = None;
                                self.error = None;
                                self.send(Command::Print(PrintRequest {
                                    path,
                                    copies: self.copies,
                                    width,
                                    height,
                                    fit: self.fit,
                                    edit: self.edit,
                                }));
                            }
                        }
                        if !can_print && !self.busy {
                            let reason = if self.connected.is_none() {
                                "Connect the printer to print."
                            } else if self.printer_has_alert() {
                                "Resolve the printer alert first."
                            } else if self.image_path.is_none() {
                                "Choose a photo first."
                            } else {
                                ""
                            };
                            ui.label(RichText::new(reason).color(palette.muted).small());
                        }

                        ui.add_space(12.0);
                        self.progress_ui(ui);
                    });

                    ui.add_space(12.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Print preview").size(14.0).strong());
                        ui.add_space(4.0);
                        self.paint_print_preview(ui, preview_width);
                        ui.add_space(4.0);
                        let caption = match self.fit {
                            Fit::Cover => "2×3 · Fill (no borders)",
                            Fit::Contain => "2×3 · Fit (white borders)",
                        };
                        ui.label(RichText::new(caption).color(palette.muted).small());
                    });
                });
            });

        ui.add_space(8.0);
        self.device_card(ui);
        ui.add_space(8.0);
        self.connection_tools(ui);
    }

    fn status_card(&mut self, ui: &mut egui::Ui) {
        let palette = Palette::of(ui);
        let Some(status) = self.status.as_ref().and_then(parse_status) else {
            egui::Frame::new()
                .fill(palette.card)
                .corner_radius(12.0)
                .inner_margin(16.0)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        if self.connecting || self.busy {
                            ui.spinner();
                            ui.label(
                                RichText::new("Connecting to the printer…").color(palette.muted),
                            );
                        } else {
                            ui.label(
                                RichText::new("Printer status unavailable").color(palette.muted),
                            );
                        }
                    });
                });
            return;
        };

        let (label, color) = status.headline(&palette);
        egui::Frame::new()
            .fill(palette.card)
            .corner_radius(12.0)
            .inner_margin(16.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    status_dot(ui, color, true);
                    ui.label(RichText::new(label).size(17.0).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if let Some(battery) = status.battery {
                            ui.label(
                                RichText::new(battery_label(battery))
                                    .color(if battery <= 1 {
                                        palette.red
                                    } else {
                                        palette.muted
                                    })
                                    .small(),
                            );
                        }
                    });
                });
                ui.label(
                    RichText::new(format!("state {} / sub-state {}", status.state, status.sub))
                        .color(palette.muted)
                        .small(),
                );
                if !status.alerts.is_empty() {
                    ui.add_space(4.0);
                    for code in &status.alerts {
                        ui.colored_label(palette.red, alert_label(*code));
                    }
                    if ui.button("Resume printer").clicked() {
                        self.send(Command::Resume);
                    }
                }
            });
    }

    fn device_card(&mut self, ui: &mut egui::Ui) {
        let palette = Palette::of(ui);
        let Some(info) = self.info.as_ref() else {
            return;
        };
        let Some(values) = info.as_array() else {
            return;
        };
        let cell = |index: usize| {
            values
                .get(index)
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .unwrap_or("—")
        };
        egui::Frame::new()
            .fill(palette.card)
            .corner_radius(12.0)
            .inner_margin(16.0)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("Printer").size(16.0).strong());
                egui::Grid::new("device-info")
                    .num_columns(2)
                    .spacing([18.0, 4.0])
                    .show(ui, |ui| {
                        ui.label(RichText::new("Model").color(palette.muted));
                        ui.label(cell(0));
                        ui.end_row();
                        ui.label(RichText::new("Serial").color(palette.muted));
                        ui.label(cell(2));
                        ui.end_row();
                        ui.label(RichText::new("Firmware").color(palette.muted));
                        ui.label(cell(4));
                        ui.end_row();
                        ui.label(RichText::new("Bluetooth").color(palette.muted));
                        ui.label(
                            self.connected
                                .as_ref()
                                .map(|d| d.address.clone())
                                .unwrap_or_else(|| "—".to_owned()),
                        );
                        ui.end_row();
                    });
            });
    }

    fn connection_tools(&mut self, ui: &mut egui::Ui) {
        let palette = Palette::of(ui);
        egui::CollapsingHeader::new("Bluetooth tools")
            .default_open(false)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Pair by MAC");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.pair_address)
                            .hint_text("11:22:33:44:55:66")
                            .desired_width(160.0),
                    );
                    if ui
                        .add_enabled(
                            !self.busy && !self.pair_address.trim().is_empty(),
                            egui::Button::new("Pair"),
                        )
                        .clicked()
                    {
                        self.send(Command::Pair {
                            address: self.pair_address.trim().to_owned(),
                        });
                    }
                });
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!self.busy, egui::Button::new("Scan for devices (10 s)"))
                        .clicked()
                    {
                        self.send(Command::Scan);
                    }
                    ui.label(
                        RichText::new("Results appear in the log below.")
                            .color(palette.muted)
                            .small(),
                    );
                });
            });
    }

    fn progress_ui(&mut self, ui: &mut egui::Ui) {
        if let Some((sent, total)) = self.upload {
            let fraction = if total == 0 {
                0.0
            } else {
                sent as f32 / total as f32
            };
            ui.add(
                egui::ProgressBar::new(fraction).text(format!("Uploading {sent}/{total} frames")),
            );
            return;
        }

        let Some(job) = &self.job else {
            return;
        };

        let state = job.state.unwrap_or(3);
        let palette = Palette::of(ui);
        let (state_label, color) = job_state_label(state, &palette);
        ui.horizontal(|ui| {
            ui.colored_label(color, format!("Job #{}: {state_label}", job.id));
            if state == 3 {
                ui.spinner();
            }
        });

        let fraction = if job.file_size > 0 {
            (job.transfer_size as f32 / job.file_size as f32).min(1.0)
        } else {
            0.0
        };
        ui.add(
            egui::ProgressBar::new(fraction)
                .text(format!("{} / {} bytes", job.transfer_size, job.file_size)),
        );

        if state == 3 {
            ui.label(
                RichText::new(format!("Printing page {}", job.page.max(1)))
                    .color(palette.muted)
                    .small(),
            );
        }
        if state == 9 {
            let seconds = job.print_time_ms as f32 / 1000.0;
            ui.label(
                RichText::new(format!("Finished in {seconds:.1} s"))
                    .color(palette.green)
                    .small(),
            );
        }
        if let Some(reason) = job.reason {
            ui.label(
                RichText::new(format!(
                    "reason {reason}, {total_frames} frames",
                    total_frames = job.total_frames
                ))
                .color(palette.muted)
                .small(),
            );
        }
        if matches!(state, 7 | 8) {
            ui.horizontal(|ui| {
                if ui.button("Resume printer").clicked() {
                    self.send(Command::Resume);
                }
                if ui.button(format!("Cancel job #{}", job.id)).clicked() {
                    self.send(Command::CancelJob { job_id: job.id });
                }
            });
        }
    }

    fn log_panel(&mut self, ui: &mut egui::Ui) {
        let palette = Palette::of(ui);
        egui::CollapsingHeader::new("Log")
            .default_open(false)
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(150.0)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for line in &self.log {
                            ui.label(RichText::new(line).monospace().small().color(palette.muted));
                        }
                    });
            });
    }
}

impl LieneApp {
    fn process_control(&mut self, ctx: &egui::Context) {
        let Some(rx) = self.control_rx.take() else {
            return;
        };
        while let Ok(job) = rx.try_recv() {
            self.handle_control_job(ctx, job);
        }
        self.control_rx = Some(rx);
    }

    fn handle_control_job(&mut self, ctx: &egui::Context, job: ControlJob) {
        if job.request.action == "ui.screenshot" {
            let Some(path) = job
                .request
                .params
                .get("path")
                .and_then(Value::as_str)
                .map(PathBuf::from)
            else {
                let _ = job
                    .reply
                    .send(Response::error("screenshot requires a path"));
                return;
            };
            let reply = job.reply;
            ctx.request_screenshot(move |image| {
                let mut rgba = Vec::with_capacity(image.pixels.len() * 4);
                for pixel in &image.pixels {
                    rgba.extend_from_slice(&pixel.to_array());
                }
                let response = match image::save_buffer(
                    &path,
                    &rgba,
                    image.size[0] as u32,
                    image.size[1] as u32,
                    image::ColorType::Rgba8,
                ) {
                    Ok(()) => Response::ok(format!("screenshot saved to {}", path.display())),
                    Err(err) => Response::error(format!("screenshot failed: {err}")),
                };
                let _ = reply.send(response);
            });
            return;
        }

        let response = self.apply_control(ctx, &job.request);
        let _ = job.reply.send(response);
        ctx.request_repaint();
    }

    fn apply_control(&mut self, ctx: &egui::Context, request: &control::Request) -> Response {
        let result = (|| -> anyhow::Result<Response> {
            let params = &request.params;
            match request.action.as_str() {
                "state" => Ok(Response::with_state("state", self.control_state())),
                "image.open" => {
                    let path = params
                        .get("path")
                        .and_then(Value::as_str)
                        .ok_or_else(|| anyhow::anyhow!("image.open requires path"))?;
                    self.load_image(PathBuf::from(path), ctx);
                    Ok(Response::with_state("image opened", self.control_state()))
                }
                "collage.open" => {
                    let paths: Vec<PathBuf> = params
                        .get("paths")
                        .and_then(Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(Value::as_str)
                                .map(PathBuf::from)
                                .collect()
                        })
                        .unwrap_or_default();
                    if paths.is_empty() {
                        anyhow::bail!("collage.open requires at least two paths");
                    }
                    self.load_collage(paths, ctx);
                    if self.collage.is_none() {
                        anyhow::bail!(
                            "{}",
                            self.error
                                .clone()
                                .unwrap_or_else(|| "could not open the collage".to_owned())
                        );
                    }
                    Ok(Response::with_state("collage opened", self.control_state()))
                }
                "collage.layout" => {
                    let id = params
                        .get("id")
                        .and_then(Value::as_str)
                        .ok_or_else(|| anyhow::anyhow!("collage.layout requires id"))?;
                    let Some(collage) = self.collage.as_mut() else {
                        anyhow::bail!("no collage is open");
                    };
                    let Some(index) = collage.layouts.iter().position(|layout| layout.id == id)
                    else {
                        anyhow::bail!("unknown collage layout '{id}'");
                    };
                    collage.selected = index;
                    Ok(Response::with_state(
                        "layout selected",
                        self.control_state(),
                    ))
                }
                "collage.borders" => {
                    let enabled = params
                        .get("enabled")
                        .and_then(Value::as_bool)
                        .ok_or_else(|| anyhow::anyhow!("collage.borders requires enabled"))?;
                    let Some(collage) = self.collage.as_mut() else {
                        anyhow::bail!("no collage is open");
                    };
                    collage.borders = enabled;
                    Ok(Response::with_state(
                        "borders updated",
                        self.control_state(),
                    ))
                }
                "collage.randomize" => {
                    let Some(collage) = self.collage.as_mut() else {
                        anyhow::bail!("no collage is open");
                    };
                    shuffle_collage(collage);
                    Ok(Response::with_state(
                        "photos randomized",
                        self.control_state(),
                    ))
                }
                "collage.frame.set" => {
                    let index = index_param(params)?;
                    let frame = PhotoFrame {
                        zoom: params
                            .get("zoom")
                            .and_then(Value::as_f64)
                            .map(|value| value as f32)
                            .unwrap_or(1.0),
                        pan_x: params
                            .get("pan_x")
                            .and_then(Value::as_f64)
                            .map(|value| value as f32)
                            .unwrap_or(0.0),
                        pan_y: params
                            .get("pan_y")
                            .and_then(Value::as_f64)
                            .map(|value| value as f32)
                            .unwrap_or(0.0),
                        rotation: params
                            .get("rotation")
                            .and_then(Value::as_u64)
                            .map(|value| value as u32)
                            .unwrap_or(0),
                    };
                    self.set_collage_frame(index, frame, ctx)?;
                    Ok(Response::with_state("frame updated", self.control_state()))
                }
                "collage.frame.reset" => {
                    let index = index_param(params)?;
                    self.set_collage_frame(index, PhotoFrame::default(), ctx)?;
                    Ok(Response::with_state("frame reset", self.control_state()))
                }
                "collage.frame.rotate" => {
                    let index = index_param(params)?;
                    let Some(collage) = self.collage.as_ref() else {
                        anyhow::bail!("no collage is open");
                    };
                    let Some(current) = collage.frames.get(index) else {
                        anyhow::bail!("photo index {index} is out of range");
                    };
                    let frame = PhotoFrame {
                        rotation: (current.rotation + 90) % 360,
                        ..PhotoFrame::default()
                    };
                    self.set_collage_frame(index, frame, ctx)?;
                    Ok(Response::with_state("photo rotated", self.control_state()))
                }
                "collage.frame.replace" => {
                    let index = index_param(params)?;
                    let path = params
                        .get("path")
                        .and_then(Value::as_str)
                        .ok_or_else(|| anyhow::anyhow!("collage.frame.replace requires path"))?;
                    self.replace_collage_photo(index, PathBuf::from(path), ctx)?;
                    Ok(Response::with_state("photo replaced", self.control_state()))
                }
                "collage.frame.open" => {
                    let index = index_param(params)?;
                    let Some(collage) = self.collage.as_mut() else {
                        anyhow::bail!("no collage is open");
                    };
                    if index >= collage.paths.len() {
                        anyhow::bail!("photo index {index} is out of range");
                    }
                    collage.editing_photo = Some(index);
                    Ok(Response::with_state(
                        "frame editor opened",
                        self.control_state(),
                    ))
                }
                "collage.frame.close" => {
                    if let Some(collage) = self.collage.as_mut() {
                        collage.editing_photo = None;
                    }
                    Ok(Response::with_state(
                        "frame editor closed",
                        self.control_state(),
                    ))
                }
                "collage.print_preview" => {
                    if self.collage.is_none() {
                        anyhow::bail!("no collage is open");
                    }
                    self.collage_previewing = true;
                    Ok(Response::with_state(
                        "print preview opened",
                        self.control_state(),
                    ))
                }
                "collage.print" => {
                    let copies = params
                        .get("copies")
                        .and_then(Value::as_u64)
                        .map(|value| value as u32)
                        .unwrap_or(1);
                    self.print_collage(copies)?;
                    Ok(Response::with_state("print started", self.control_state()))
                }
                "ui.quit" => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    Ok(Response::ok("quitting"))
                }
                other => anyhow::bail!("unknown UI action '{other}'"),
            }
        })();
        result.unwrap_or_else(|err| Response::error(format!("{err:#}")))
    }

    fn set_collage_frame(
        &mut self,
        index: usize,
        frame: PhotoFrame,
        ctx: &egui::Context,
    ) -> anyhow::Result<()> {
        let (rotation_changed, source) = {
            let Some(collage) = self.collage.as_ref() else {
                anyhow::bail!("no collage is open");
            };
            if index >= collage.frames.len() {
                anyhow::bail!("photo index {index} is out of range");
            }
            (
                collage.frames[index].rotation % 360 != frame.rotation % 360,
                collage.sources.get(index).cloned(),
            )
        };
        if rotation_changed {
            let source = source.ok_or_else(|| anyhow::anyhow!("missing photo source"))?;
            let rotated = rotate_rgba(&source, frame.rotation % 360);
            let texture = texture_from_rgba(
                ctx,
                &rotated,
                format!("collage-{index}-rot-{}", frame.rotation % 360),
            );
            if let Some(collage) = self.collage.as_mut() {
                collage.textures[index] = texture;
            }
        }
        if let Some(collage) = self.collage.as_mut() {
            collage.frames[index] = frame;
        }
        ctx.request_repaint();
        Ok(())
    }

    fn replace_collage_photo(
        &mut self,
        index: usize,
        path: PathBuf,
        ctx: &egui::Context,
    ) -> anyhow::Result<()> {
        let image = liene_client::image_prep::load_oriented(&path)?;
        let rgba = image.thumbnail(1200, 1200).to_rgba8();
        let texture = texture_from_rgba(
            ctx,
            &rgba,
            format!("collage-{index}-replace-{}", std::process::id()),
        );
        let Some(collage) = self.collage.as_mut() else {
            anyhow::bail!("no collage is open");
        };
        if index >= collage.paths.len() {
            anyhow::bail!("photo index {index} is out of range");
        }
        collage.paths[index] = path;
        collage.sources[index] = rgba;
        collage.textures[index] = texture;
        collage.frames[index] = PhotoFrame::default();
        ctx.request_repaint();
        Ok(())
    }

    fn print_collage(&mut self, copies: u32) -> anyhow::Result<()> {
        let Some(collage) = self.collage.as_ref() else {
            anyhow::bail!("no collage is open");
        };
        if self.connected.is_none() {
            anyhow::bail!("connect the printer before printing");
        }
        if self.busy {
            anyhow::bail!("the printer is busy");
        }
        if self.printer_has_alert() {
            anyhow::bail!("resolve the printer alert before printing");
        }
        let layout = collage
            .layouts
            .get(collage.selected)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("invalid collage layout"))?;
        let paths = collage.paths.clone();
        let frames = collage.frames.clone();
        let borders = collage.borders;
        self.busy = true;
        self.job = None;
        self.upload = None;
        self.error = None;
        self.collage_previewing = false;
        self.push_log(format!("Printing collage with {} photos…", paths.len()));
        self.send(Command::PrintCollage(CollageRequest {
            paths,
            layout: layout.id.to_owned(),
            copies: copies.clamp(1, 9),
            borders,
            frames,
        }));
        Ok(())
    }

    fn control_state(&self) -> Value {
        let collage = self.collage.as_ref().map(|collage| {
            json!({
                "paths": collage.paths.iter().map(|path| path.display().to_string()).collect::<Vec<_>>(),
                "layout": collage.layouts.get(collage.selected).map(|layout| layout.id),
                "layout_name": collage.layouts.get(collage.selected).map(|layout| layout.name),
                "borders": collage.borders,
                "copies": collage.copies,
                "editing_photo": collage.editing_photo,
                "frames": collage.frames,
                "previewing": self.collage_previewing,
                "photo_count": collage.paths.len(),
            })
        });
        json!({
            "image_path": self.image_path.as_ref().map(|path| path.display().to_string()),
            "collage": collage,
            "connected": self.connected.as_ref().map(|device| json!({
                "name": device.name,
                "address": device.address,
            })),
            "busy": self.busy,
            "error": self.error,
        })
    }
}

impl eframe::App for LieneApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.process_control(ctx);

        if std::mem::take(&mut self.auto_connect) && !self.no_connect {
            self.send(Command::Connect {
                name: self.device_prefix.clone(),
                scan_secs: self.scan_secs,
            });
        }

        while let Ok(event) = self.events.try_recv() {
            self.handle_event(event, ctx);
        }

        if self.connected.is_some()
            && !self.busy
            && self.last_poll.elapsed() >= Duration::from_secs(5)
        {
            self.send(Command::RefreshStatus);
            self.last_poll = Instant::now();
        }

        self.frames = self.frames.saturating_add(1);
        let screenshot_ready =
            self.connected.is_some() && self.status.is_some() && self.info.is_some() && !self.busy;
        if screenshot_ready || self.frames >= 300 {
            if let Some(path) = self.screenshot_path.take() {
                let callback_ctx = ctx.clone();
                ctx.request_screenshot(move |image| {
                    let mut rgba = Vec::with_capacity(image.pixels.len() * 4);
                    for pixel in &image.pixels {
                        rgba.extend_from_slice(&pixel.to_array());
                    }
                    if let Err(err) = image::save_buffer(
                        &path,
                        &rgba,
                        image.size[0] as u32,
                        image.size[1] as u32,
                        image::ColorType::Rgba8,
                    ) {
                        eprintln!("screenshot failed: {err}");
                    } else {
                        eprintln!("screenshot saved to {}", path.display());
                    }
                    callback_ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                });
            }
        }

        ctx.request_repaint_after(Duration::from_millis(200));
    }

    fn on_exit(&mut self) {
        if let Some(path) = self.control_file.take() {
            control::remove_discovery(&path);
        }
        if let Some(mut child) = self._child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.handle_drops(ui);

        egui::Frame::new()
            .fill(ui.visuals().panel_fill)
            .inner_margin(18.0)
            .show(ui, |ui| {
                let mut scroll = egui::ScrollArea::vertical().auto_shrink([false, false]);
                if self.pin_scroll_top {
                    scroll = scroll.vertical_scroll_offset(0.0);
                }
                scroll.show(ui, |ui| {
                    self.top_bar(ui);
                    ui.add_space(6.0);
                    self.error_banner(ui);
                    ui.add_space(6.0);

                    if self.collage.is_some() {
                        self.collage_panel(ui);
                    } else {
                        ui.horizontal_top(|ui| {
                            let total_width = ui.available_width();
                            let right_width = (total_width - 420.0 - 14.0 - 12.0).max(360.0);

                            ui.vertical(|ui| {
                                ui.set_width(420.0);
                                self.photo_panel(ui);
                            });
                            ui.add_space(14.0);
                            ui.vertical(|ui| {
                                ui.set_width(right_width);
                                self.settings_panel(ui);
                            });
                        });
                    }

                    ui.add_space(10.0);
                    self.log_panel(ui);
                });
            });

        if self.collage_previewing && self.collage.is_some() {
            self.collage_print_window(ui.ctx());
        }
        if self
            .collage
            .as_ref()
            .and_then(|collage| collage.editing_photo)
            .is_some()
        {
            self.collage_photo_window(ui.ctx());
        }
    }
}

fn layout_icon(
    ui: &mut egui::Ui,
    layout: &CollageLayout,
    selected: bool,
    palette: &Palette,
) -> bool {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(56.0, 84.0), egui::Sense::click());
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, Color32::WHITE);
    for cell in &layout.cells {
        let dst = egui::Rect::from_min_max(
            egui::pos2(
                rect.left() + cell.x * rect.width(),
                rect.top() + cell.y * rect.height(),
            ),
            egui::pos2(
                rect.left() + (cell.x + cell.w) * rect.width(),
                rect.top() + (cell.y + cell.h) * rect.height(),
            ),
        );
        painter.rect_filled(dst, 0.0, Color32::from_gray(190));
    }
    painter.rect_stroke(
        rect,
        0.0,
        if selected {
            egui::Stroke::new(2.0, palette.accent)
        } else {
            egui::Stroke::new(1.0, palette.border)
        },
        egui::StrokeKind::Inside,
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), selected, layout.name)
    });
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(layout.name)
        .clicked()
}

fn shuffle_collage(collage: &mut CollageState) {
    let len = collage.paths.len();
    if len < 2 {
        return;
    }
    let mut seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(1);
    let mut changed = false;
    for index in (1..len).rev() {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let other = (seed as usize) % (index + 1);
        if other != index {
            changed = true;
            collage.paths.swap(index, other);
            collage.sources.swap(index, other);
            collage.textures.swap(index, other);
            collage.frames.swap(index, other);
        }
    }
    if !changed {
        collage.paths.rotate_left(1);
        collage.sources.rotate_left(1);
        collage.textures.rotate_left(1);
        collage.frames.rotate_left(1);
    }
}

fn layout_for_render(layout: &CollageLayout, count: usize, borders: bool) -> CollageLayout {
    if borders {
        layout.clone()
    } else {
        collage::layouts_for_borders(count, false)
            .into_iter()
            .find(|candidate| candidate.id == layout.id)
            .unwrap_or_else(|| layout.clone())
    }
}

fn frame_uv(texture_size: egui::Vec2, dst_aspect: f32, frame: PhotoFrame) -> egui::Rect {
    let source_aspect = texture_size.x / texture_size.y;
    let (base_width, base_height) = if source_aspect > dst_aspect {
        (dst_aspect / source_aspect, 1.0)
    } else {
        (1.0, source_aspect / dst_aspect)
    };
    let zoom = frame.zoom.clamp(1.0, 8.0);
    let crop_width = (base_width / zoom).clamp(0.0, 1.0);
    let crop_height = (base_height / zoom).clamp(0.0, 1.0);
    // Mirror photo_draw_rect: the photo may overflow the cell on both axes,
    // not just inside the zoom-1 crop. Panning therefore has the same range
    // as in the frame editor (+1 shows the left/top edge of the source).
    let x = (1.0 - crop_width) * (1.0 - frame.pan_x.clamp(-1.0, 1.0)) / 2.0;
    let y = (1.0 - crop_height) * (1.0 - frame.pan_y.clamp(-1.0, 1.0)) / 2.0;
    egui::Rect::from_min_max(
        egui::pos2(x, y),
        egui::pos2(x + crop_width, y + crop_height),
    )
}

fn photo_draw_rect(
    texture_size: egui::Vec2,
    viewport: egui::Vec2,
    frame: PhotoFrame,
) -> egui::Rect {
    let scale = f32::max(viewport.x / texture_size.x, viewport.y / texture_size.y)
        * frame.zoom.clamp(1.0, 8.0);
    let width = texture_size.x * scale;
    let height = texture_size.y * scale;
    let slack_x = (width - viewport.x).max(0.0);
    let slack_y = (height - viewport.y).max(0.0);
    let left = (viewport.x - width) / 2.0 + frame.pan_x.clamp(-1.0, 1.0) * slack_x / 2.0;
    let top = (viewport.y - height) / 2.0 + frame.pan_y.clamp(-1.0, 1.0) * slack_y / 2.0;
    egui::Rect::from_min_size(egui::pos2(left, top), egui::vec2(width, height))
}

fn rotate_rgba(source: &image::RgbaImage, rotation: u32) -> image::RgbaImage {
    match rotation % 360 {
        90 => image::imageops::rotate90(source),
        180 => image::imageops::rotate180(source),
        270 => image::imageops::rotate270(source),
        _ => source.clone(),
    }
}

fn texture_from_rgba(
    ctx: &egui::Context,
    rgba: &image::RgbaImage,
    name: String,
) -> egui::TextureHandle {
    let color = egui::ColorImage::from_rgba_unmultiplied(
        [rgba.width() as usize, rgba.height() as usize],
        rgba.as_raw(),
    );
    ctx.load_texture(name, color, egui::TextureOptions::LINEAR)
}

fn paint_collage(
    painter: &egui::Painter,
    rect: egui::Rect,
    textures: &[egui::TextureHandle],
    frames: &[PhotoFrame],
    layout: &CollageLayout,
) {
    painter.rect_filled(rect, 0.0, Color32::WHITE);
    for (index, (texture, cell)) in textures.iter().zip(&layout.cells).enumerate() {
        let dst = egui::Rect::from_min_max(
            egui::pos2(
                rect.left() + cell.x * rect.width(),
                rect.top() + cell.y * rect.height(),
            ),
            egui::pos2(
                rect.left() + (cell.x + cell.w) * rect.width(),
                rect.top() + (cell.y + cell.h) * rect.height(),
            ),
        );
        if dst.width() <= 0.0 || dst.height() <= 0.0 {
            continue;
        }
        let source = texture.size_vec2();
        let dst_aspect = dst.width() / dst.height();
        let frame = frames.get(index).copied().unwrap_or_default();
        let uv = frame_uv(source, dst_aspect, frame);
        painter.image(texture.id(), dst, uv, Color32::WHITE);
    }
    painter.rect_stroke(
        rect,
        0.0,
        egui::Stroke::new(1.0, Color32::from_gray(190)),
        egui::StrokeKind::Inside,
    );
}

fn index_param(params: &Value) -> anyhow::Result<usize> {
    params
        .get("index")
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .ok_or_else(|| anyhow::anyhow!("action requires index"))
}

fn spawn_helper() -> anyhow::Result<(ChildStdin, Child, Receiver<Event>)> {
    let helper = locate_helper()?;
    let mut command = std::process::Command::new(&helper);
    command
        .arg("serve")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command
        .spawn()
        .map_err(|err| anyhow::anyhow!("could not start {} serve: {err}", helper.display()))?;

    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("helper stdin unavailable"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("helper stdout unavailable"))?;

    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        use std::io::BufRead;
        let reader = std::io::BufReader::new(stdout);
        for line in reader.lines() {
            let Ok(line) = line else { break };
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            match serde_json::from_str::<Event>(line) {
                Ok(event) => {
                    if tx.send(event).is_err() {
                        break;
                    }
                }
                Err(err) => eprintln!("helper: bad event ({err}): {line}"),
            }
        }
    });

    Ok((stdin, child, rx))
}

/// The helper lives next to the GUI binary in both `target/{debug,release}`
/// and the macOS app bundle's `Contents/MacOS`.
fn locate_helper() -> anyhow::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let dir = exe
        .parent()
        .ok_or_else(|| anyhow::anyhow!("executable has no parent directory"))?;
    let name = format!("open-liene-cli{}", std::env::consts::EXE_SUFFIX);
    let candidate = dir.join(&name);
    if candidate.is_file() {
        return Ok(candidate);
    }
    Ok(PathBuf::from(name))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CropDrag {
    Move,
    N,
    S,
    E,
    W,
    NE,
    NW,
    SE,
    SW,
}

fn crop_to_screen(crop: NormalizedRect, image: egui::Rect) -> egui::Rect {
    egui::Rect::from_min_max(
        image.min + egui::vec2(crop.x * image.width(), crop.y * image.height()),
        image.min
            + egui::vec2(
                (crop.x + crop.w) * image.width(),
                (crop.y + crop.h) * image.height(),
            ),
    )
}

fn hit_test(pos: egui::Pos2, crop: egui::Rect, corners_only: bool) -> Option<CropDrag> {
    let tolerance = 14.0;

    if corners_only {
        let corners = [
            (crop.left(), crop.top(), CropDrag::NW),
            (crop.right(), crop.top(), CropDrag::NE),
            (crop.left(), crop.bottom(), CropDrag::SW),
            (crop.right(), crop.bottom(), CropDrag::SE),
        ];
        let mut best: Option<(f32, CropDrag)> = None;
        for (x, y, mode) in corners {
            let dx = (pos.x - x).abs();
            let dy = (pos.y - y).abs();
            if dx <= tolerance && dy <= tolerance {
                let distance = dx.max(dy);
                if best.is_none_or(|(current, _)| distance < current) {
                    best = Some((distance, mode));
                }
            }
        }
        if let Some((_, mode)) = best {
            return Some(mode);
        }
        return crop.contains(pos).then_some(CropDrag::Move);
    }

    if pos.x < crop.left() - tolerance
        || pos.x > crop.right() + tolerance
        || pos.y < crop.top() - tolerance
        || pos.y > crop.bottom() + tolerance
    {
        return None;
    }

    let left = (pos.x - crop.left()).abs() <= tolerance;
    let right = (pos.x - crop.right()).abs() <= tolerance;
    let top = (pos.y - crop.top()).abs() <= tolerance;
    let bottom = (pos.y - crop.bottom()).abs() <= tolerance;

    if left && top {
        Some(CropDrag::NW)
    } else if right && top {
        Some(CropDrag::NE)
    } else if left && bottom {
        Some(CropDrag::SW)
    } else if right && bottom {
        Some(CropDrag::SE)
    } else if left {
        Some(CropDrag::W)
    } else if right {
        Some(CropDrag::E)
    } else if top {
        Some(CropDrag::N)
    } else if bottom {
        Some(CropDrag::S)
    } else if crop.contains(pos) {
        Some(CropDrag::Move)
    } else {
        None
    }
}

fn resize_crop(
    start: NormalizedRect,
    mode: CropDrag,
    dx: f32,
    dy: f32,
    aspect: Option<(f32, f32, f32)>,
    max_zoom: f32,
) -> NormalizedRect {
    if let Some((norm_aspect, max_w, _max_h)) = aspect {
        if norm_aspect.is_finite() && norm_aspect > 0.0 {
            return resize_locked(start, mode, dx, dy, norm_aspect, max_w, max_zoom);
        }
    }

    let mut x = start.x;
    let mut y = start.y;
    let mut w = start.w;
    let mut h = start.h;

    match mode {
        CropDrag::Move => {
            x += dx;
            y += dy;
        }
        CropDrag::N => {
            y += dy;
            h -= dy;
        }
        CropDrag::S => h += dy,
        CropDrag::W => {
            x += dx;
            w -= dx;
        }
        CropDrag::E => w += dx,
        CropDrag::NW => {
            x += dx;
            y += dy;
            w -= dx;
            h -= dy;
        }
        CropDrag::NE => {
            y += dy;
            w += dx;
            h -= dy;
        }
        CropDrag::SW => {
            x += dx;
            w -= dx;
            h += dy;
        }
        CropDrag::SE => {
            w += dx;
            h += dy;
        }
    }

    // Keep the moving edge fixed when a minimum-size clamp kicks in.
    let min = liene_client::edit::MIN_CROP;
    if w < min {
        if matches!(mode, CropDrag::W | CropDrag::NW | CropDrag::SW) {
            x -= min - w;
        }
        w = min;
    }
    if h < min {
        if matches!(mode, CropDrag::N | CropDrag::NW | CropDrag::NE) {
            y -= min - h;
        }
        h = min;
    }

    NormalizedRect { x, y, w, h }.clamped()
}

/// Resize from a corner while keeping the crop at the 2:3 print aspect.
/// All values are normalized; `norm_aspect` is the desired w/h ratio in
/// normalized coordinates for the current rotated image.
fn resize_locked(
    start: NormalizedRect,
    mode: CropDrag,
    dx: f32,
    dy: f32,
    norm_aspect: f32,
    max_w: f32,
    max_zoom: f32,
) -> NormalizedRect {
    if mode == CropDrag::Move {
        let x = (start.x + dx).clamp(0.0, 1.0 - start.w);
        let y = (start.y + dy).clamp(0.0, 1.0 - start.h);
        return NormalizedRect {
            x,
            y,
            w: start.w,
            h: start.h,
        };
    }

    let min_edge = liene_client::edit::MIN_CROP;
    let w_min = (max_w / max_zoom).max(min_edge).max(min_edge * norm_aspect);

    let (anchor_x, anchor_y, desired_w, w_max) = match mode {
        CropDrag::SE => {
            let anchor_x = start.x;
            let anchor_y = start.y;
            let desired = (start.w + dx).max((start.h + dy) * norm_aspect);
            let w_max = (1.0 - anchor_x).min((1.0 - anchor_y) * norm_aspect);
            (anchor_x, anchor_y, desired, w_max)
        }
        CropDrag::NW => {
            let anchor_x = start.x + start.w;
            let anchor_y = start.y + start.h;
            let desired = (start.w - dx).max((start.h - dy) * norm_aspect);
            let w_max = anchor_x.min(anchor_y * norm_aspect);
            (anchor_x, anchor_y, desired, w_max)
        }
        CropDrag::NE => {
            let anchor_x = start.x;
            let anchor_y = start.y + start.h;
            let desired = (start.w + dx).max((start.h - dy) * norm_aspect);
            let w_max = (1.0 - anchor_x).min(anchor_y * norm_aspect);
            (anchor_x, anchor_y, desired, w_max)
        }
        CropDrag::SW => {
            let anchor_x = start.x + start.w;
            let anchor_y = start.y;
            let desired = (start.w - dx).max((start.h + dy) * norm_aspect);
            let w_max = anchor_x.min((1.0 - anchor_y) * norm_aspect);
            (anchor_x, anchor_y, desired, w_max)
        }
        _ => return start,
    };

    if !w_max.is_finite() || w_max < w_min {
        return start.clamped();
    }

    let w = desired_w.clamp(w_min, w_max);
    let h = w / norm_aspect;

    let (x, y) = match mode {
        CropDrag::SE => (anchor_x, anchor_y),
        CropDrag::NW => (anchor_x - w, anchor_y - h),
        CropDrag::NE => (anchor_x, anchor_y - h),
        CropDrag::SW => (anchor_x - w, anchor_y),
        _ => (start.x, start.y),
    };

    NormalizedRect {
        x: x.clamp(0.0, 1.0 - w),
        y: y.clamp(0.0, 1.0 - h),
        w,
        h,
    }
}

fn paint_dim(painter: &egui::Painter, rect: egui::Rect, color: Color32) {
    if rect.width() > 0.0 && rect.height() > 0.0 {
        painter.rect_filled(rect, 0.0, color);
    }
}

#[derive(Clone, Debug)]
struct StatusView {
    state: u32,
    sub: u32,
    alerts: Vec<u32>,
    battery: Option<u32>,
}

impl StatusView {
    fn headline(&self, palette: &Palette) -> (&'static str, Color32) {
        if let Some(code) = self.alerts.first() {
            return (alert_label(*code), palette.red);
        }
        match self.state {
            10 => ("Initializing", palette.amber),
            20 => ("Ready", palette.green),
            30 => ("Sleeping", palette.muted),
            40 => ("Printing", palette.amber),
            50 => ("Powered off", palette.muted),
            60 => ("Printer error", palette.red),
            _ => ("Unknown state", palette.muted),
        }
    }
}

fn parse_status(value: &Value) -> Option<StatusView> {
    let values = value.as_array()?;
    let cell = |index: usize| values.get(index).and_then(Value::as_str);
    let state = cell(0)?.parse().ok()?;
    let sub = cell(1)?.parse().ok()?;
    let alerts = cell(2).map(parse_alerts).unwrap_or_default();
    let battery = cell(4).and_then(|value| value.parse().ok());
    Some(StatusView {
        state,
        sub,
        alerts,
        battery,
    })
}

/// The printer reports battery as a 0–4 level (0/25/50/75/100% fill), not a
/// percentage. `4` means full.
fn battery_label(level: u32) -> String {
    match level {
        0 => "Battery empty".to_owned(),
        1 => "Battery low".to_owned(),
        2 => "Battery 50%".to_owned(),
        3 => "Battery 75%".to_owned(),
        4 => "Battery full".to_owned(),
        other => format!("Battery {other}/4"),
    }
}

fn parse_alerts(value: &str) -> Vec<u32> {
    value
        .split(|c: char| !c.is_ascii_digit())
        .filter_map(|part| part.parse::<u32>().ok())
        .filter(|code| *code != 0)
        .collect()
}

fn alert_label(code: u32) -> &'static str {
    match code {
        5402 => "Paper empty — load a 2×3 cartridge",
        5403 => "Paper jam or paper error",
        5414 => "Paper was not picked up",
        5417 => "Printed paper was not removed",
        5418 => "Paper detection error",
        5419 | 5420 => "Paper feed error",
        5506 => "Printer door is open",
        6002 => "Printer needs a reboot",
        8101 => "Ribbon empty — replace the cartridge",
        8103 => "Ribbon jam",
        8106 | 8199 => "Ribbon error",
        9002 => "Battery too hot or shut down",
        9004 => "Battery temperature high",
        9005 => "Battery low — connect power",
        _ => "Printer alert",
    }
}

fn job_state_label(state: u32, palette: &Palette) -> (&'static str, Color32) {
    match state {
        3 => ("Printing", palette.amber),
        7 => ("Aborted", palette.red),
        8 => ("Canceled", palette.red),
        9 => ("Finished", palette.green),
        _ => ("Pending", palette.muted),
    }
}

fn apply_job_state(job: &mut JobUi, value: &Value) {
    let Some(object) = value
        .as_array()
        .and_then(|items| items.first())
        .or_else(|| value.as_object().map(|_| value))
    else {
        return;
    };
    if let Some(state) = object.get("job-state").and_then(Value::as_u64) {
        job.state = Some(state as u32);
    }
    if let Some(reason) = object.get("job-state-reason").and_then(Value::as_u64) {
        job.reason = Some(reason);
    }
    if let Some(transfer) = object.get("transfer-size").and_then(Value::as_u64) {
        job.transfer_size = transfer;
    }
    if let Some(size) = object.get("file-size").and_then(Value::as_u64) {
        job.file_size = size;
    }
    if let Some(page) = object.get("printing-page-number").and_then(Value::as_u64) {
        job.page = page;
    }
    if let Some(time) = object.get("print-time").and_then(Value::as_u64) {
        job.print_time_ms = time;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crop_from_draw_rect(
        texture: egui::Vec2,
        viewport: egui::Vec2,
        frame: PhotoFrame,
    ) -> egui::Rect {
        let draw = photo_draw_rect(texture, viewport, frame);
        let x = -draw.left() / draw.width();
        let y = -draw.top() / draw.height();
        egui::Rect::from_min_size(
            egui::pos2(x, y),
            egui::vec2(viewport.x / draw.width(), viewport.y / draw.height()),
        )
    }

    fn assert_close(actual: egui::Rect, expected: egui::Rect) {
        let epsilon = 1e-4;
        assert!(
            (actual.left() - expected.left()).abs() < epsilon,
            "{actual:?} != {expected:?}"
        );
        assert!(
            (actual.top() - expected.top()).abs() < epsilon,
            "{actual:?} != {expected:?}"
        );
        assert!(
            (actual.width() - expected.width()).abs() < epsilon,
            "{actual:?} != {expected:?}"
        );
        assert!(
            (actual.height() - expected.height()).abs() < epsilon,
            "{actual:?} != {expected:?}"
        );
    }

    #[test]
    fn collage_preview_crop_matches_the_frame_editor() {
        let viewport_height = 1000.0;
        for (source_width, source_height) in [(600.0, 900.0), (900.0, 600.0), (800.0, 800.0)] {
            let texture = egui::vec2(source_width, source_height);
            for dst_aspect in [0.2136, 2.0 / 3.0, 1.0, 1.5, 2.4] {
                let viewport = egui::vec2(viewport_height * dst_aspect, viewport_height);
                for zoom in [1.0, 1.7, 3.0, 8.0] {
                    for pan_x in [-1.0, -0.37, 0.0, 0.42, 1.0] {
                        for pan_y in [-1.0, 0.0, 0.65] {
                            let frame = PhotoFrame {
                                zoom,
                                pan_x,
                                pan_y,
                                rotation: 0,
                            };
                            let expected = crop_from_draw_rect(texture, viewport, frame);
                            let actual = frame_uv(texture, dst_aspect, frame);
                            assert_close(actual, expected);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn collage_preview_pans_across_the_whole_photo_at_any_zoom() {
        let texture = egui::vec2(600.0, 900.0);
        let dst_aspect = 0.2136;
        let centered = frame_uv(
            texture,
            dst_aspect,
            PhotoFrame {
                zoom: 1.0,
                pan_x: 0.0,
                ..PhotoFrame::default()
            },
        );
        let left = frame_uv(
            texture,
            dst_aspect,
            PhotoFrame {
                zoom: 1.0,
                pan_x: 1.0,
                ..PhotoFrame::default()
            },
        );
        let right = frame_uv(
            texture,
            dst_aspect,
            PhotoFrame {
                zoom: 1.0,
                pan_x: -1.0,
                ..PhotoFrame::default()
            },
        );
        assert!(centered.left() > 0.0);
        assert_eq!(left.left(), 0.0);
        assert!((right.right() - 1.0).abs() < 1e-5);
    }
}

#[cfg(test)]
mod kittest_tests {
    use super::*;
    use egui_kittest::kittest::Queryable;
    use egui_kittest::{Harness, SnapshotOptions};

    fn fixture_paths(tag: &str) -> Vec<PathBuf> {
        let dir = std::env::temp_dir().join(format!("liene-kittest-{}-{tag}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create fixture directory");
        let colors = [[214, 58, 48], [42, 158, 92], [52, 88, 214]];
        (0..3)
            .map(|index| {
                let path = dir.join(format!("photo-{index}.png"));
                let base = colors[index];
                let mut pixels = image::RgbaImage::new(600, 900);
                for (x, y, pixel) in pixels.enumerate_pixels_mut() {
                    let shade = 0.65 + 0.35 * (1.0 - (x + y) as f32 / 1500.0);
                    let mut rgba = [
                        (base[0] as f32 * shade) as u8,
                        (base[1] as f32 * shade) as u8,
                        (base[2] as f32 * shade) as u8,
                        255,
                    ];
                    if (300..360).contains(&y) {
                        rgba = [245, 245, 245, 255];
                    }
                    *pixel = image::Rgba(rgba);
                }
                pixels.save(&path).expect("save fixture image");
                path
            })
            .collect()
    }

    fn snapshot_options() -> SnapshotOptions {
        SnapshotOptions::default()
            .output_path(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots"))
    }

    fn test_harness(
        tag: &str,
        seed: impl FnOnce(&mut LieneApp, &egui::Context, Vec<PathBuf>) + 'static,
    ) -> Harness<'static, LieneApp> {
        let paths = fixture_paths(tag);
        Harness::builder()
            .with_size(egui::vec2(1040.0, 760.0))
            .with_theme(egui::Theme::Dark)
            .with_os(egui::os::OperatingSystem::Nix)
            .with_options(snapshot_options())
            .build_eframe(move |cc| {
                let mut app = LieneApp::new(cc, false);
                app.no_connect = true;
                app.auto_connect = false;
                app.error = None;
                app.pin_scroll_top = true;
                seed(&mut app, &cc.egui_ctx, paths);
                app
            })
    }

    fn seed_collage(app: &mut LieneApp, ctx: &egui::Context, paths: Vec<PathBuf>) {
        app.load_collage(paths, ctx);
        let collage = app.collage.as_mut().expect("collage should load");
        collage.selected = collage
            .layouts
            .iter()
            .position(|layout| layout.id == "h3")
            .expect("three-photo layouts include h3");
        collage.frames[0] = PhotoFrame {
            zoom: 2.0,
            pan_x: 1.0,
            ..PhotoFrame::default()
        };
    }

    fn seed_frame_editor(app: &mut LieneApp, ctx: &egui::Context, paths: Vec<PathBuf>) {
        seed_collage(app, ctx, paths);
        app.collage.as_mut().expect("collage").editing_photo = Some(0);
    }

    fn seed_print_preview(app: &mut LieneApp, ctx: &egui::Context, paths: Vec<PathBuf>) {
        seed_collage(app, ctx, paths);
        let collage = app.collage.as_mut().expect("collage");
        collage.copies = 2;
        app.collage_previewing = true;
    }

    fn seed_photo_editor(app: &mut LieneApp, ctx: &egui::Context, paths: Vec<PathBuf>) {
        app.load_image(paths.into_iter().next().expect("fixture path"), ctx);
        // Keep the machine-specific fixture path out of the snapshot.
        app.image_path = None;
    }

    #[test]
    fn collage_layout_borders_and_preview_clicks_update_state() {
        let mut harness = test_harness("collage-interactions", seed_collage);
        harness.run_steps(3);

        harness.get_by_label("Vertical").click();
        harness.run_steps(3);
        {
            let collage = harness.state().collage.as_ref().expect("collage");
            assert_eq!(collage.layouts[collage.selected].id, "v3");
        }

        harness.get_by_label("Borders").click();
        harness.run_steps(3);
        assert!(!harness.state().collage.as_ref().expect("collage").borders);

        harness.get_by_label("Randomize").click();
        harness.run_steps(3);
        assert_eq!(
            harness
                .state()
                .collage
                .as_ref()
                .expect("collage")
                .paths
                .len(),
            3
        );

        harness.get_by_label("Collage preview").click();
        harness.run_steps(3);
        assert_eq!(
            harness
                .state()
                .collage
                .as_ref()
                .expect("collage")
                .editing_photo,
            Some(1)
        );
    }

    #[test]
    fn frame_editor_rotate_reset_and_done_update_state() {
        let mut harness = test_harness("frame-editor", seed_frame_editor);
        harness.run_steps(3);

        harness.get_by_label("Rotate 90°").click();
        harness.run_steps(3);
        assert_eq!(
            harness.state().collage.as_ref().expect("collage").frames[0].rotation,
            90
        );

        harness.get_by_label("Reset").click();
        harness.run_steps(3);
        assert_eq!(
            harness.state().collage.as_ref().expect("collage").frames[0].rotation,
            0
        );

        harness.get_by_label("Done").click();
        harness.run_steps(3);
        assert!(harness
            .state()
            .collage
            .as_ref()
            .expect("collage")
            .editing_photo
            .is_none());
    }

    #[test]
    fn print_preview_back_closes_the_window() {
        let mut harness = test_harness("print-preview-back", seed_print_preview);
        harness.run_steps(3);
        assert!(harness.state().collage_previewing);

        harness.get_by_label("Back").click();
        harness.run_steps(3);
        assert!(!harness.state().collage_previewing);
    }

    #[test]
    fn photo_editor_fit_and_rotate_update_state() {
        let mut harness = test_harness("photo-editor", seed_photo_editor);
        harness.run_steps(3);

        harness.get_by_label("Fill").click();
        harness.run_steps(3);
        assert_eq!(harness.state().fit, Fit::Cover);

        harness.get_by_label("Fit").click();
        harness.run_steps(3);
        assert_eq!(harness.state().fit, Fit::Contain);

        harness.get_by_label("Rotate 90°").click();
        harness.run_steps(3);
        assert_eq!(harness.state().edit.normalized_rotation(), 90);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn empty_window_snapshot() {
        let mut harness = test_harness("snapshot-empty", |_, _, _| {});
        harness.run_steps(3);
        harness.snapshot("main/empty");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn photo_editor_snapshot() {
        let mut harness = test_harness("snapshot-photo", seed_photo_editor);
        harness.run_steps(3);
        harness.snapshot("editor/photo");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn collage_panel_snapshot() {
        let mut harness = test_harness("snapshot-collage", seed_collage);
        harness.run_steps(3);
        harness.snapshot("collage/panel");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn frame_editor_snapshot() {
        let mut harness = test_harness("snapshot-frame", seed_frame_editor);
        harness.run_steps(3);
        harness.snapshot("collage/frame-editor");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn print_preview_snapshot() {
        let mut harness = test_harness("snapshot-print", seed_print_preview);
        harness.run_steps(3);
        harness.snapshot("collage/print-preview");
    }
}
