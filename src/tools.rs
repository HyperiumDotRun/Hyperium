use std::path::{Path, PathBuf};

use eframe::egui;
use egui::{Color32, FontFamily, FontId, RichText, Stroke};

use crate::doctor::{self, MachineReport};

pub(crate) const FG: Color32 = Color32::from_rgb(224, 226, 230);
pub(crate) const DIM: Color32 = Color32::from_rgb(122, 126, 136);
pub(crate) const FAINT: Color32 = Color32::from_rgb(78, 81, 90);
pub(crate) const ACCENT: Color32 = Color32::from_rgb(178, 232, 44);
const ACCENT_DIM: Color32 = Color32::from_rgb(120, 150, 40);
pub(crate) const RED: Color32 = Color32::from_rgb(226, 92, 92);
pub(crate) const BG_ELEVATED: Color32 = Color32::from_rgb(26, 27, 31);
const BG_HOVER: Color32 = Color32::from_rgb(34, 36, 41);

pub struct ToolCtx<'a> {
    pub out_dir: &'a Path,
    pub config_dir: &'a Path,
    pub project_path: &'a Path,
}

pub trait Tool {
    fn title(&self) -> &'static str;
    fn about(&self) -> &'static str;
    fn uses_output_dir(&self) -> bool {
        true
    }
    fn ui(&mut self, ui: &mut egui::Ui, octx: &ToolCtx);
}

pub const BUILTIN: &[(&str, &str)] = &[
    ("genai", "Mirage"),
    ("sushi", "Sushi agent"),
    ("memory", "Memory"),
    ("cheats", "Command memo"),
    ("health", "Health log"),
    ("favicon", "Favicon generator"),
    ("convert", "Media converter"),
    ("doctor", "Machine doctor"),
];

pub fn make_tool(id: &str) -> Option<Box<dyn Tool>> {
    match id {
        "genai" => Some(Box::<GenAiTool>::default()),
        "sushi" => Some(Box::<crate::sushi::SushiTool>::default()),
        "memory" => Some(Box::<MemoryTool>::default()),
        "cheats" => Some(Box::<CommandMemoTool>::default()),
        "health" => Some(Box::<HealthLogTool>::default()),
        "favicon" => Some(Box::<FaviconTool>::default()),
        "convert" => Some(Box::<ConvertTool>::default()),
        "doctor" => Some(Box::<DoctorTool>::default()),
        _ => None,
    }
}

struct Preview {
    tex: egui::TextureHandle,
    dims: (u32, u32),
    path: PathBuf,
}

#[derive(Default)]
pub struct FaviconTool {
    source: Option<PathBuf>,
    preview: Option<Preview>,
    status: Option<Result<String, String>>,
    gen_rx: Option<std::sync::mpsc::Receiver<Result<String, String>>>,
}

impl Tool for FaviconTool {
    fn title(&self) -> &'static str {
        "Favicon generator"
    }
    fn about(&self) -> &'static str {
        "Drop an image → the full web favicon set in your project"
    }

    fn ui(&mut self, ui: &mut egui::Ui, octx: &ToolCtx) {
        let dropped: Option<PathBuf> = ui.ctx().input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .find(|p| is_supported_image(p))
        });
        if let Some(p) = dropped {
            self.source = Some(p);
            self.status = None;
        }
        let hovering = ui.ctx().input(|i| !i.raw.hovered_files.is_empty());

        match &self.source {
            Some(src) if self.preview.as_ref().map(|p| &p.path) != Some(src) => {
                self.preview = load_preview(ui.ctx(), src);
            }
            None => self.preview = None,
            _ => {}
        }

        if let Some(rx) = &self.gen_rx {
            match rx.try_recv() {
                Ok(res) => {
                    self.status = Some(res);
                    self.gen_rx = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => ui.ctx().request_repaint(),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.gen_rx = None,
            }
        }
        let generating = self.gen_rx.is_some();

        ui.label(
            RichText::new("Drop a square PNG or JPG (or click to browse), then generate.")
                .color(DIM)
                .small(),
        );
        ui.add_space(12.0);

        let (rect, resp) = ui.allocate_exact_size(
            egui::vec2(ui.available_width().min(520.0), 240.0),
            egui::Sense::click(),
        );
        let active = hovering || resp.hovered();
        ui.painter().rect_filled(rect, 10.0, if active { BG_HOVER } else { BG_ELEVATED });
        ui.painter().rect_stroke(
            rect,
            10.0,
            Stroke::new(1.0, if active { ACCENT } else { ACCENT_DIM }),
            egui::StrokeKind::Inside,
        );
        if let Some(p) = &self.preview {
            let max = 150.0;
            let sz = p.tex.size_vec2();
            let s = (max / sz.x).min(max / sz.y).min(1.0);
            let img = egui::Rect::from_center_size(
                rect.center() - egui::vec2(0.0, 22.0),
                sz * s,
            );
            ui.painter().image(
                p.tex.id(),
                img,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
            let name = self
                .source
                .as_ref()
                .and_then(|s| s.file_name())
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            ui.painter().text(
                egui::pos2(rect.center().x, rect.bottom() - 38.0),
                egui::Align2::CENTER_CENTER,
                format!("{} × {} px", p.dims.0, p.dims.1),
                FontId::new(13.0, FontFamily::Monospace),
                FG,
            );
            ui.painter().text(
                egui::pos2(rect.center().x, rect.bottom() - 18.0),
                egui::Align2::CENTER_CENTER,
                name,
                FontId::new(11.5, FontFamily::Proportional),
                DIM,
            );
        } else {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "drop an image here  ·  or click to browse",
                FontId::new(14.0, FontFamily::Proportional),
                DIM,
            );
        }
        if resp.clicked()
            && let Some(file) = rfd::FileDialog::new()
                .add_filter("images", &["png", "jpg", "jpeg"])
                .pick_file()
        {
            self.source = Some(file);
            self.status = None;
        }

        ui.add_space(14.0);

        if generating {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(22.0).color(ACCENT));
                ui.add_space(4.0);
                ui.label(RichText::new("Generating favicons…").color(ACCENT));
            });
        } else {
            let ready = self.source.is_some();
            if tool_button(ui, "Generate favicons", ready)
                && let Some(src) = self.source.clone()
            {
                let out = octx.out_dir.to_path_buf();
                let (tx, rx) = std::sync::mpsc::channel();
                std::thread::spawn(move || {
                    let _ = tx.send(generate_favicons(&src, &out));
                });
                self.gen_rx = Some(rx);
                self.status = None;
                ui.ctx().request_repaint();
            }
        }

        if let Some(result) = &self.status {
            ui.add_space(12.0);
            match result {
                Ok(msg) => ui.label(RichText::new(format!("✓ {msg}")).color(ACCENT).small()),
                Err(msg) => ui.label(RichText::new(format!("⚠ {msg}")).color(RED).small()),
            };
        }
    }
}

fn load_preview(ctx: &egui::Context, src: &Path) -> Option<Preview> {
    let img = image::open(src).ok()?;
    let dims = (img.width(), img.height());
    let thumb = img.thumbnail(256, 256).to_rgba8();
    let (w, h) = thumb.dimensions();
    let color = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], thumb.as_raw());
    let tex = ctx.load_texture("favicon_preview", color, egui::TextureOptions::LINEAR);
    Some(Preview { tex, dims, path: src.to_path_buf() })
}

fn is_supported_image(p: &Path) -> bool {
    matches!(
        p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref(),
        Some("png" | "jpg" | "jpeg")
    )
}

fn generate_favicons(src: &Path, out: &Path) -> Result<String, String> {
    std::fs::create_dir_all(out).map_err(|e| format!("can't create output dir: {e}"))?;
    let img = image::open(src).map_err(|e| format!("can't read image: {e}"))?;

    const BASE: u32 = 512;
    let base = img.resize_exact(BASE, BASE, image::imageops::FilterType::Lanczos3);

    let png_bytes = |size: u32| -> Result<Vec<u8>, String> {
        let resized = if size == BASE {
            base.clone()
        } else {
            base.resize_exact(size, size, image::imageops::FilterType::Lanczos3)
        };
        let mut buf = std::io::Cursor::new(Vec::new());
        resized
            .write_to(&mut buf, image::ImageFormat::Png)
            .map_err(|e| e.to_string())?;
        Ok(buf.into_inner())
    };
    let write = |name: &str, bytes: &[u8]| -> Result<(), String> {
        std::fs::write(out.join(name), bytes).map_err(|e| format!("{name}: {e}"))
    };

    let png_set: [(&str, u32); 6] = [
        ("favicon-16x16.png", 16),
        ("favicon-32x32.png", 32),
        ("favicon-48x48.png", 48),
        ("apple-touch-icon.png", 180),
        ("android-chrome-192x192.png", 192),
        ("android-chrome-512x512.png", 512),
    ];
    for (name, size) in png_set {
        write(name, &png_bytes(size)?)?;
    }

    let ico = build_ico(&[(16, png_bytes(16)?), (32, png_bytes(32)?), (48, png_bytes(48)?)]);
    write("favicon.ico", &ico)?;

    write("site.webmanifest", WEBMANIFEST.as_bytes())?;

    Ok(format!("8 files written to {}", out.display()))
}

pub(crate) fn build_ico(frames: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&(frames.len() as u16).to_le_bytes());

    let mut offset = 6 + 16 * frames.len();
    for (size, data) in frames {
        let dim = if *size >= 256 { 0u8 } else { *size as u8 };
        out.push(dim);
        out.push(dim);
        out.push(0);
        out.push(0);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(offset as u32).to_le_bytes());
        offset += data.len();
    }
    for (_, data) in frames {
        out.extend_from_slice(data);
    }
    out
}

const WEBMANIFEST: &str = r##"{
  "name": "",
  "short_name": "",
  "icons": [
    { "src": "/android-chrome-192x192.png", "sizes": "192x192", "type": "image/png" },
    { "src": "/android-chrome-512x512.png", "sizes": "512x512", "type": "image/png" }
  ],
  "theme_color": "#ffffff",
  "background_color": "#ffffff",
  "display": "standalone"
}
"##;

pub(crate) fn tool_button(ui: &mut egui::Ui, label: &str, enabled: bool) -> bool {
    let font = FontId::new(14.0, FontFamily::Proportional);
    let w = ui.painter().layout_no_wrap(label.to_string(), font.clone(), FG).size().x + 28.0;
    let sense = if enabled { egui::Sense::click() } else { egui::Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 34.0), sense);
    let hov = enabled && resp.hovered();
    let (fill, stroke, text) = if !enabled {
        (BG_ELEVATED, ACCENT_DIM, FAINT)
    } else if hov {
        (BG_HOVER, ACCENT, ACCENT)
    } else {
        (BG_ELEVATED, ACCENT_DIM, FG)
    };
    ui.painter().rect_filled(rect, 8.0, fill);
    ui.painter().rect_stroke(rect, 8.0, Stroke::new(1.0, stroke), egui::StrokeKind::Inside);
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, label, font, text);
    enabled && resp.clicked()
}

use crate::genai::{self, Kind};

struct Shot {
    path: PathBuf,
    tex: Option<egui::TextureHandle>,
    is_video: bool,
}

enum GenMsg {
    Status(String),
    Done(Result<genai::Saved, String>),
}

pub struct GenAiTool {
    loaded: bool,
    last_model: usize,
    model_idx: usize,
    prompt: String,
    aspect_idx: usize,
    quality_idx: usize,
    duration_idx: usize,
    cap: u32,
    cap_edit: String,
    used: u32,
    rx: Option<std::sync::mpsc::Receiver<GenMsg>>,
    status: String,
    result: Option<Result<String, String>>,
    gallery: Vec<Shot>,
}

impl Default for GenAiTool {
    fn default() -> Self {
        Self {
            loaded: false,
            last_model: usize::MAX,
            model_idx: 0,
            prompt: String::new(),
            aspect_idx: 0,
            quality_idx: 0,
            duration_idx: 0,
            cap: genai::DEFAULT_CAP,
            cap_edit: String::new(),
            used: 0,
            rx: None,
            status: String::new(),
            result: None,
            gallery: Vec::new(),
        }
    }
}

impl Tool for GenAiTool {
    fn title(&self) -> &'static str {
        "Mirage"
    }
    fn about(&self) -> &'static str {
        "Prompt → image/video (kie.ai), saved into your project"
    }

    fn ui(&mut self, ui: &mut egui::Ui, octx: &ToolCtx) {
        if !self.loaded {
            self.cap = genai::load_cap(octx.config_dir);
            self.cap_edit = self.cap.to_string();
            self.used = genai::usage_for(octx.config_dir, &octx.project_path.to_string_lossy());
            self.loaded = true;
        }

        if self.last_model != self.model_idx {
            let m = &genai::MODELS[self.model_idx];
            self.aspect_idx = m.aspects.iter().position(|a| *a == "16:9").unwrap_or(0);
            self.quality_idx = 0;
            self.duration_idx = 0;
            self.last_model = self.model_idx;
        }

        if let Some(rx) = &self.rx {
            loop {
                match rx.try_recv() {
                    Ok(GenMsg::Status(s)) => self.status = s,
                    Ok(GenMsg::Done(res)) => {
                        match res {
                            Ok(saved) => {
                                let is_video = matches!(
                                    saved.path.extension().and_then(|e| e.to_str()),
                                    Some("mp4" | "webm" | "mov")
                                );
                                let tex = if is_video { None } else { load_thumb(ui.ctx(), &saved.path) };
                                let name = saved
                                    .path
                                    .file_name()
                                    .map(|s| s.to_string_lossy().into_owned())
                                    .unwrap_or_default();
                                crate::notify::toast("Hyperium - Mirage ✓", &name);
                                self.gallery.insert(0, Shot { path: saved.path, tex, is_video });
                                self.result = Some(Ok(format!("saved {name}")));
                                self.used = genai::usage_for(
                                    octx.config_dir,
                                    &octx.project_path.to_string_lossy(),
                                );
                            }
                            Err(e) => {
                                crate::notify::toast("Hyperium - Mirage failed", &e);
                                self.result = Some(Err(e));
                            }
                        }
                        self.rx = None;
                        self.status.clear();
                        break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        ui.ctx().request_repaint();
                        break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        self.rx = None;
                        break;
                    }
                }
            }
        }
        let generating = self.rx.is_some();

        if !genai::has_key(octx.config_dir) {
            ui.add_space(6.0);
            ui.label(RichText::new("No kie.ai API key yet.").color(RED));
            ui.add_space(4.0);
            ui.label(
                RichText::new("Add it in Settings → AI (\"Mirage - kie.ai\"), then reopen this tool.")
                    .color(DIM)
                    .small(),
            );
            return;
        }

        let m = &genai::MODELS[self.model_idx];
        ui.horizontal(|ui| {
            ui.label(RichText::new("Model").color(ACCENT).small());
            ui.add_space(6.0);
            egui::ComboBox::from_id_salt("genai_model")
                .width(300.0)
                .selected_text(m.label)
                .show_ui(ui, |ui| {
                    for (i, mm) in genai::MODELS.iter().enumerate() {
                        let tag = if mm.kind == Kind::Image { "image" } else { "video" };
                        ui.selectable_value(&mut self.model_idx, i, format!("{}  ·  {} · {tag}", mm.label, mm.provider));
                    }
                });
            let badge = if m.kind == Kind::Image { "🖼 image" } else { "🎞 video" };
            ui.label(RichText::new(badge).color(DIM).small());
        });
        ui.label(RichText::new(m.about).color(FAINT).small());
        ui.add_space(10.0);

        ui.label(RichText::new("Prompt").color(ACCENT).small());
        ui.add(
            egui::TextEdit::multiline(&mut self.prompt)
                .desired_rows(3)
                .desired_width(f32::INFINITY)
                .hint_text("describe what to generate…"),
        );
        ui.add_space(10.0);

        ui.label(RichText::new("Aspect ratio").color(ACCENT).small());
        pill_select(ui, m.aspects, &mut self.aspect_idx);
        if let Some(k) = &m.quality {
            ui.add_space(6.0);
            ui.label(RichText::new(k.label).color(ACCENT).small());
            pill_select(ui, k.values, &mut self.quality_idx);
        }
        if let Some(k) = &m.duration {
            ui.add_space(6.0);
            ui.label(RichText::new("Duration (s)").color(ACCENT).small());
            pill_select(ui, k.values, &mut self.duration_idx);
        }
        ui.add_space(14.0);

        if generating {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(20.0).color(ACCENT));
                ui.add_space(6.0);
                let s = if self.status.is_empty() { "working…" } else { &self.status };
                ui.label(RichText::new(format!("generating…  {s}")).color(ACCENT));
            });
        } else {
            let ready = !self.prompt.trim().is_empty() && self.used < self.cap;
            if tool_button(ui, "Generate", ready) && ready {
                self.start_generation(octx);
            }
            if self.used >= self.cap {
                ui.add_space(6.0);
                ui.label(
                    RichText::new(format!(
                        "⚠ project cap reached ({}/{}). Raise it below to keep going.",
                        self.used, self.cap
                    ))
                    .color(RED)
                    .small(),
                );
            }
        }

        if let Some(res) = &self.result {
            ui.add_space(8.0);
            match res {
                Ok(msg) => ui.label(RichText::new(format!("✓ {msg}")).color(ACCENT).small()),
                Err(msg) => ui.label(RichText::new(format!("⚠ {msg}")).color(RED).small()),
            };
        }

        if !self.gallery.is_empty() {
            ui.add_space(14.0);
            ui.separator();
            ui.add_space(10.0);
            let mut open_dir: Option<PathBuf> = None;
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));

            {
                let shot = &self.gallery[0];
                let avail = ui.available_width().min(520.0);
                let box_h = 360.0;
                let (rect, resp) =
                    ui.allocate_exact_size(egui::vec2(avail, box_h), egui::Sense::click());
                ui.painter().rect_filled(rect, 12.0, BG_ELEVATED);
                if let Some(tex) = &shot.tex {
                    let sz = tex.size_vec2();
                    let s = ((avail - 16.0) / sz.x).min((box_h - 16.0) / sz.y);
                    let img = egui::Rect::from_center_size(rect.center(), sz * s);
                    ui.painter().image(tex.id(), img, uv, Color32::WHITE);
                } else {
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        if shot.is_video {
                            "🎞  video ready  ·  click to open"
                        } else {
                            "🖼  image saved  ·  click to open"
                        },
                        FontId::new(16.0, FontFamily::Proportional),
                        DIM,
                    );
                }
                ui.painter().rect_stroke(
                    rect,
                    12.0,
                    Stroke::new(1.0, if resp.hovered() { ACCENT } else { ACCENT_DIM }),
                    egui::StrokeKind::Inside,
                );
                if resp.clicked() {
                    open_dir = shot.path.parent().map(|p| p.to_path_buf());
                }
                let name =
                    shot.path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                ui.add_space(6.0);
                ui.label(RichText::new(name).color(DIM).small());
            }

            if self.gallery.len() > 1 {
                ui.add_space(12.0);
                ui.label(RichText::new("Earlier this session").color(DIM).small());
                ui.add_space(6.0);
                egui::ScrollArea::horizontal().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for shot in &self.gallery[1..] {
                            let (rect, resp) = ui
                                .allocate_exact_size(egui::vec2(170.0, 170.0), egui::Sense::click());
                            ui.painter().rect_filled(rect, 10.0, BG_ELEVATED);
                            if let Some(tex) = &shot.tex {
                                let sz = tex.size_vec2();
                                let s = (154.0 / sz.x).min(154.0 / sz.y);
                                let img = egui::Rect::from_center_size(rect.center(), sz * s);
                                ui.painter().image(tex.id(), img, uv, Color32::WHITE);
                            } else {
                                ui.painter().text(
                                    rect.center(),
                                    egui::Align2::CENTER_CENTER,
                                    if shot.is_video { "🎞\nvideo" } else { "🖼" },
                                    FontId::new(15.0, FontFamily::Proportional),
                                    DIM,
                                );
                            }
                            ui.painter().rect_stroke(
                                rect,
                                10.0,
                                Stroke::new(1.0, if resp.hovered() { ACCENT } else { FAINT }),
                                egui::StrokeKind::Inside,
                            );
                            if resp.hovered() {
                                let name = shot
                                    .path
                                    .file_name()
                                    .map(|s| s.to_string_lossy().into_owned())
                                    .unwrap_or_default();
                                resp.clone().on_hover_text(name);
                            }
                            if resp.clicked() {
                                open_dir = shot.path.parent().map(|p| p.to_path_buf());
                            }
                        }
                    });
                });
            }
            if let Some(dir) = open_dir {
                open_folder(&dir);
            }
        }

        ui.add_space(14.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("Project usage: {}/{}", self.used, self.cap))
                    .color(DIM)
                    .small(),
            );
            ui.add_space(10.0);
            ui.label(RichText::new("cap").color(FAINT).small());
            if ui
                .add(egui::TextEdit::singleline(&mut self.cap_edit).desired_width(48.0))
                .lost_focus()
                && let Ok(n) = self.cap_edit.trim().parse::<u32>()
            {
                self.cap = n.max(1);
                self.cap_edit = self.cap.to_string();
                genai::save_cap(octx.config_dir, self.cap);
            }
            ui.add_space(8.0);
            if self.used > 0 && tool_button(ui, "Reset", true) {
                genai::reset_usage(octx.config_dir, &octx.project_path.to_string_lossy());
                self.used = 0;
            }
        });
    }
}

impl GenAiTool {
    fn start_generation(&mut self, octx: &ToolCtx) {
        let m = &genai::MODELS[self.model_idx];
        let req = genai::Request {
            model_idx: self.model_idx,
            prompt: self.prompt.trim().to_string(),
            aspect: m.aspects.get(self.aspect_idx).copied().unwrap_or("").to_string(),
            quality: m.quality.as_ref().and_then(|k| k.values.get(self.quality_idx).map(|v| v.to_string())),
            duration: m.duration.as_ref().and_then(|k| k.values.get(self.duration_idx).map(|v| v.to_string())),
        };
        let cfg = octx.config_dir.to_path_buf();
        let out = octx.out_dir.to_path_buf();
        let project = octx.project_path.to_string_lossy().into_owned();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let tx2 = tx.clone();
            let res = genai::run(&cfg, &out, &project, &req, move |s| {
                let _ = tx2.send(GenMsg::Status(s.to_string()));
            });
            let _ = tx.send(GenMsg::Done(res));
        });
        self.rx = Some(rx);
        self.status = "submitting".to_string();
        self.result = None;
    }
}

pub(crate) fn pill_select(ui: &mut egui::Ui, values: &[&str], idx: &mut usize) {
    ui.horizontal_wrapped(|ui| {
        for (i, v) in values.iter().enumerate() {
            let selected = *idx == i;
            let font = FontId::new(12.5, FontFamily::Proportional);
            let w = ui.painter().layout_no_wrap((*v).to_string(), font.clone(), FG).size().x + 18.0;
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 26.0), egui::Sense::click());
            let hov = resp.hovered();
            let (fill, stroke, text) = if selected {
                (BG_HOVER, ACCENT, ACCENT)
            } else if hov {
                (BG_HOVER, ACCENT_DIM, FG)
            } else {
                (BG_ELEVATED, FAINT, DIM)
            };
            ui.painter().rect_filled(rect, 6.0, fill);
            ui.painter().rect_stroke(rect, 6.0, Stroke::new(1.0, stroke), egui::StrokeKind::Inside);
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, *v, font, text);
            if resp.clicked() {
                *idx = i;
            }
        }
    });
}

fn load_thumb(ctx: &egui::Context, path: &Path) -> Option<egui::TextureHandle> {
    let img = image::open(path).ok()?;
    let thumb = img.thumbnail(768, 768).to_rgba8();
    let (w, h) = thumb.dimensions();
    let color = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], thumb.as_raw());
    Some(ctx.load_texture("genai_thumb", color, egui::TextureOptions::LINEAR))
}

#[derive(Clone, Copy)]
struct Fmt {
    id: &'static str,
    label: &'static str,
    ext: &'static str,
    video: bool,
    encoder: &'static str,
    uses_quality: bool,
    note: &'static str,
}

const FORMATS: &[Fmt] = &[
    Fmt { id: "webp", label: "WebP", ext: "webp", video: false, encoder: "libwebp",
        uses_quality: true, note: "lossy, great for the web" },
    Fmt { id: "avif", label: "AVIF", ext: "avif", video: false, encoder: "libaom-av1",
        uses_quality: true, note: "AV1 still - smallest, modern" },
    Fmt { id: "jxl", label: "JPEG XL", ext: "jxl", video: false, encoder: "libjxl",
        uses_quality: true, note: "exotic, high quality/size" },
    Fmt { id: "jpg", label: "JPEG", ext: "jpg", video: false, encoder: "",
        uses_quality: true, note: "universal" },
    Fmt { id: "png", label: "PNG", ext: "png", video: false, encoder: "",
        uses_quality: false, note: "lossless" },
    Fmt { id: "webm_vp9", label: "WebM (VP9)", ext: "webm", video: true, encoder: "libvpx-vp9",
        uses_quality: true, note: "VP9 + Opus - web standard" },
    Fmt { id: "webm_av1", label: "WebM (AV1)", ext: "webm", video: true, encoder: "libsvtav1",
        uses_quality: true, note: "AV1 + Opus - smaller, slower" },
    Fmt { id: "mp4_h264", label: "MP4 (H.264)", ext: "mp4", video: true, encoder: "libx264",
        uses_quality: true, note: "H.264 + AAC - most compatible" },
    Fmt { id: "mp4_h265", label: "MP4 (H.265)", ext: "mp4", video: true, encoder: "libx265",
        uses_quality: true, note: "H.265 + AAC - smaller than H.264" },
    Fmt { id: "gif", label: "GIF", ext: "gif", video: true, encoder: "",
        uses_quality: false, note: "clean palette, 15 fps, 480px" },
];

struct QueueItem {
    path: PathBuf,
    is_video: bool,
    bytes: u64,
}

enum ConvMsg {
    FileStart { idx: usize, total: usize, name: String },
    Stage { frac: Option<f32>, detail: String },
    FileDone { idx: usize, ok: bool, out_bytes: u64 },
    Finished(Result<String, String>),
}

pub struct ConvertTool {
    sources: Vec<PathBuf>,
    items: Vec<QueueItem>,
    encoders: Option<std::collections::HashSet<String>>,
    ffmpeg_missing: bool,
    probed: bool,
    img_fmt_idx: usize,
    vid_fmt_idx: usize,
    quality: u32,
    cur: Option<(usize, usize, String)>,
    frac: Option<f32>,
    detail: String,
    results: Vec<Option<(bool, u64)>>,
    status: Option<Result<String, String>>,
    rx: Option<std::sync::mpsc::Receiver<ConvMsg>>,
}

impl Default for ConvertTool {
    fn default() -> Self {
        Self {
            sources: Vec::new(),
            items: Vec::new(),
            encoders: None,
            ffmpeg_missing: false,
            probed: false,
            img_fmt_idx: 0,
            vid_fmt_idx: 0,
            quality: 75,
            cur: None,
            frac: None,
            detail: String::new(),
            results: Vec::new(),
            status: None,
            rx: None,
        }
    }
}

impl Tool for ConvertTool {
    fn title(&self) -> &'static str {
        "Media converter"
    }
    fn about(&self) -> &'static str {
        "Drop images/videos → batch convert/compress (ffmpeg) into your project"
    }

    fn ui(&mut self, ui: &mut egui::Ui, octx: &ToolCtx) {
        if !self.probed {
            match detect_encoders() {
                Some(set) => self.encoders = Some(set),
                None => self.ffmpeg_missing = true,
            }
            self.probed = true;
        }

        let dropped: Vec<PathBuf> = ui.ctx().input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .filter(|p| is_supported_media(p))
                .collect()
        });
        if !dropped.is_empty() {
            self.add_sources(dropped);
        }
        let hovering = ui.ctx().input(|i| !i.raw.hovered_files.is_empty());

        if self.items.len() != self.sources.len()
            || self.items.iter().zip(&self.sources).any(|(it, p)| &it.path != p)
        {
            self.items = self
                .sources
                .iter()
                .map(|p| QueueItem {
                    path: p.clone(),
                    is_video: is_video_ext(p),
                    bytes: file_len(p),
                })
                .collect();
            self.results.clear();
        }

        if let Some(rx) = &self.rx {
            loop {
                match rx.try_recv() {
                    Ok(ConvMsg::FileStart { idx, total, name }) => {
                        self.cur = Some((idx, total, name));
                        self.frac = None;
                        self.detail = "starting…".to_string();
                    }
                    Ok(ConvMsg::Stage { frac, detail }) => {
                        self.frac = frac;
                        self.detail = detail;
                    }
                    Ok(ConvMsg::FileDone { idx, ok, out_bytes }) => {
                        if let Some(slot) = self.results.get_mut(idx) {
                            *slot = Some((ok, out_bytes));
                        }
                    }
                    Ok(ConvMsg::Finished(res)) => {
                        match &res {
                            Ok(msg) => crate::notify::toast("Hyperium - conversion ✓", msg),
                            Err(msg) => crate::notify::toast("Hyperium - conversion failed", msg),
                        }
                        self.status = Some(res);
                        self.rx = None;
                        self.cur = None;
                        break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        ui.ctx().request_repaint();
                        break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        self.rx = None;
                        self.cur = None;
                        break;
                    }
                }
            }
        }
        let converting = self.rx.is_some();

        ui.label(
            RichText::new("Drop images/videos (or click to browse - multi-select), pick a format, convert all.")
                .color(DIM)
                .small(),
        );
        ui.add_space(12.0);

        let (rect, resp) = ui.allocate_exact_size(
            egui::vec2(ui.available_width().min(520.0), 90.0),
            egui::Sense::click(),
        );
        let active = hovering || resp.hovered();
        ui.painter().rect_filled(rect, 10.0, if active { BG_HOVER } else { BG_ELEVATED });
        ui.painter().rect_stroke(
            rect,
            10.0,
            Stroke::new(1.0, if active { ACCENT } else { ACCENT_DIM }),
            egui::StrokeKind::Inside,
        );
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            if self.sources.is_empty() {
                "drop images or videos here  ·  or click to browse"
            } else {
                "drop more  ·  or click to add files"
            },
            FontId::new(14.0, FontFamily::Proportional),
            DIM,
        );
        if resp.clicked()
            && let Some(files) = rfd::FileDialog::new().add_filter("media", MEDIA_EXTS).pick_files()
        {
            self.add_sources(files);
        }

        if !self.items.is_empty() {
            ui.add_space(8.0);
            let (imgs, vids) = self.counts();
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!(
                        "{} files  ·  {imgs} images  ·  {vids} videos",
                        self.items.len()
                    ))
                    .color(ACCENT)
                    .small(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if !converting && tool_button(ui, "Clear", true) {
                        self.sources.clear();
                        self.status = None;
                    }
                });
            });
            ui.add_space(4.0);

            let mut remove: Option<usize> = None;
            egui::ScrollArea::vertical().max_height(150.0).show(ui, |ui| {
                for (i, it) in self.items.iter().enumerate() {
                    let name = it.path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                    ui.horizontal(|ui| {
                        if !converting {
                            let x = egui::Button::new(RichText::new("✕").color(RED).small())
                                .frame(false);
                            if ui.add(x).on_hover_text("remove from queue").clicked() {
                                remove = Some(i);
                            }
                        }
                        ui.label(
                            RichText::new(format!(
                                "{}  {}",
                                if it.is_video { "🎞" } else { "🖼" },
                                name
                            ))
                            .color(FG)
                            .font(FontId::new(12.5, FontFamily::Proportional)),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            match self.results.get(i).copied().flatten() {
                                Some((true, out)) => {
                                    ui.label(
                                        RichText::new(format!("✓ → {}", human_size(out)))
                                            .color(ACCENT)
                                            .small(),
                                    );
                                }
                                Some((false, _)) => {
                                    ui.label(RichText::new("✗ failed").color(RED).small());
                                }
                                None => {
                                    ui.label(RichText::new(human_size(it.bytes)).color(DIM).small());
                                }
                            }
                        });
                    });
                }
            });
            if let Some(i) = remove {
                self.sources.remove(i);
                self.status = None;
            }
        }

        ui.add_space(12.0);

        if self.ffmpeg_missing {
            ui.label(RichText::new("⚠ ffmpeg not found on PATH - the converter needs it.").color(RED));
            ui.label(
                RichText::new(
                    "Install ffmpeg (the Machine doctor will then detect it) and reopen this tool.",
                )
                .color(DIM)
                .small(),
            );
            return;
        }
        let Some(encoders) = self.encoders.clone() else { return };

        let (has_imgs, has_vids) = {
            let (i, v) = self.counts();
            (i > 0, v > 0)
        };
        let img_formats: Vec<&Fmt> = applicable_formats(false, &encoders);
        let vid_formats: Vec<&Fmt> = applicable_formats(true, &encoders);
        self.img_fmt_idx = self.img_fmt_idx.min(img_formats.len().saturating_sub(1));
        self.vid_fmt_idx = self.vid_fmt_idx.min(vid_formats.len().saturating_sub(1));

        if has_imgs && !img_formats.is_empty() {
            format_combo(ui, "Images →", "convert_img_fmt", &img_formats, &mut self.img_fmt_idx);
        }
        if has_vids && !vid_formats.is_empty() {
            format_combo(ui, "Videos →", "convert_vid_fmt", &vid_formats, &mut self.vid_fmt_idx);
        }

        let img_lossy = has_imgs && img_formats.get(self.img_fmt_idx).is_some_and(|f| f.uses_quality);
        let vid_lossy = has_vids && vid_formats.get(self.vid_fmt_idx).is_some_and(|f| f.uses_quality);
        ui.add_space(8.0);
        if img_lossy || vid_lossy {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Quality").color(ACCENT).small());
                ui.add_space(6.0);
                ui.add(egui::Slider::new(&mut self.quality, 1..=100).show_value(true));
            });
            ui.label(
                RichText::new("higher = better looking & bigger; lower = smaller file")
                    .color(FAINT)
                    .small(),
            );
        } else {
            ui.label(
                RichText::new("selected format is lossless - quality not used").color(FAINT).small(),
            );
        }

        ui.add_space(14.0);

        if converting {
            match &self.cur {
                Some((idx, total, name)) => {
                    let overall = (*idx as f32 + self.frac.unwrap_or(0.0)) / *total as f32;
                    ui.add(
                        egui::ProgressBar::new(overall.clamp(0.0, 1.0))
                            .desired_height(16.0)
                            .corner_radius(0)
                            .fill(ACCENT_DIM)
                            .text(RichText::new(format!("{}/{}", idx + 1, total)).color(FG).small()),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(format!("{name}   {}", self.detail)).color(ACCENT).small(),
                    );
                }
                None => {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(20.0).color(ACCENT));
                        ui.add_space(4.0);
                        ui.label(RichText::new("Starting ffmpeg…").color(ACCENT));
                    });
                }
            }
        } else {
            let n = self.sources.len();
            let ready = n > 0;
            let label = if n > 1 { format!("Convert all ({n})") } else { "Convert".to_string() };
            if tool_button(ui, &label, ready) && ready {
                let jobs: Vec<(PathBuf, bool)> =
                    self.items.iter().map(|it| (it.path.clone(), it.is_video)).collect();
                let img_fmt = img_formats.get(self.img_fmt_idx).map(|f| **f);
                let vid_fmt = vid_formats.get(self.vid_fmt_idx).map(|f| **f);
                let out = octx.out_dir.to_path_buf();
                let q = self.quality;
                let (tx, rx) = std::sync::mpsc::channel();
                std::thread::spawn(move || {
                    run_batch(&jobs, &out, img_fmt, vid_fmt, q, &encoders, &tx);
                });
                self.rx = Some(rx);
                self.status = None;
                self.cur = Some((0, n, String::new()));
                self.frac = None;
                self.detail = "starting…".to_string();
                self.results = vec![None; n];
                ui.ctx().request_repaint();
            }
        }

        if let Some(result) = &self.status {
            ui.add_space(12.0);
            match result {
                Ok(msg) => ui.label(RichText::new(format!("✓ {msg}")).color(ACCENT)),
                Err(msg) => ui.label(RichText::new(format!("⚠ {msg}")).color(RED).small()),
            };
            if self.results.iter().any(|r| matches!(r, Some((true, _))))
                && tool_button(ui, "Open output folder", true)
            {
                open_folder(octx.out_dir);
            }
        }
    }
}

fn open_folder(dir: &Path) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let _ = std::process::Command::new("explorer")
            .arg(dir)
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = dir;
    }
}

impl ConvertTool {
    fn add_sources(&mut self, paths: Vec<PathBuf>) {
        for p in paths {
            if !self.sources.contains(&p) {
                self.sources.push(p);
            }
        }
        self.status = None;
    }

    fn counts(&self) -> (usize, usize) {
        let v = self.items.iter().filter(|it| it.is_video).count();
        (self.items.len() - v, v)
    }
}

fn format_combo(ui: &mut egui::Ui, label: &str, id: &str, formats: &[&Fmt], idx: &mut usize) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).color(ACCENT).small());
        ui.add_space(6.0);
        egui::ComboBox::from_id_salt(id)
            .width(280.0)
            .selected_text(formats.get(*idx).map(|f| f.label).unwrap_or("-"))
            .show_ui(ui, |ui| {
                for (i, f) in formats.iter().enumerate() {
                    ui.selectable_value(idx, i, format!("{}   ·   {}", f.label, f.note));
                }
            });
    });
}

const MEDIA_EXTS: &[&str] = &[
    "png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff", "avif", "heic", "heif", "mp4", "mov",
    "mkv", "webm", "avi", "m4v", "wmv", "flv", "mpg", "mpeg", "ts", "m2ts", "3gp", "ogv", "gif",
];

fn is_supported_media(p: &Path) -> bool {
    ext_lower(p).is_some_and(|e| MEDIA_EXTS.contains(&e.as_str()))
}

fn is_video_ext(p: &Path) -> bool {
    matches!(
        ext_lower(p).as_deref(),
        Some(
            "mp4" | "mov" | "mkv" | "webm" | "avi" | "m4v" | "wmv" | "flv" | "mpg" | "mpeg" | "ts"
                | "m2ts" | "3gp" | "ogv" | "gif"
        )
    )
}

fn ext_lower(p: &Path) -> Option<String> {
    p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase())
}

fn applicable_formats<'a>(
    is_video: bool,
    encoders: &std::collections::HashSet<String>,
) -> Vec<&'a Fmt> {
    FORMATS.iter().filter(|f| f.video == is_video && encoder_ok(f, encoders)).collect()
}

fn encoder_ok(f: &Fmt, encoders: &std::collections::HashSet<String>) -> bool {
    match f.id {
        "webm_av1" => {
            encoders.contains("libsvtav1") || encoders.contains("libaom-av1")
        }
        _ if f.encoder.is_empty() => true,
        _ => encoders.contains(f.encoder),
    }
}

fn detect_encoders() -> Option<std::collections::HashSet<String>> {
    let out = doctor::quiet_command("ffmpeg")
        .args(["-hide_banner", "-encoders"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut set = std::collections::HashSet::new();
    let mut past_legend = false;
    for line in text.lines() {
        if !past_legend {
            if line.trim_start().starts_with("------") {
                past_legend = true;
            }
            continue;
        }
        if let Some(name) = line.split_whitespace().nth(1) {
            set.insert(name.to_string());
        }
    }
    Some(set)
}

fn run_batch(
    jobs: &[(PathBuf, bool)],
    out_dir: &Path,
    img_fmt: Option<Fmt>,
    vid_fmt: Option<Fmt>,
    quality: u32,
    encoders: &std::collections::HashSet<String>,
    tx: &std::sync::mpsc::Sender<ConvMsg>,
) {
    if let Err(e) = std::fs::create_dir_all(out_dir) {
        let _ = tx.send(ConvMsg::Finished(Err(format!("can't create output dir: {e}"))));
        return;
    }
    let total = jobs.len();
    let (mut ok, mut in_sum, mut out_sum) = (0usize, 0u64, 0u64);
    let mut first_err: Option<String> = None;

    for (i, (src, is_video)) in jobs.iter().enumerate() {
        let name = src.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let _ = tx.send(ConvMsg::FileStart { idx: i, total, name: name.clone() });
        let Some(fmt) = (if *is_video { vid_fmt } else { img_fmt }) else {
            let _ = tx.send(ConvMsg::FileDone { idx: i, ok: false, out_bytes: 0 });
            continue;
        };
        match convert_one(src, out_dir, fmt, quality, encoders, tx) {
            Ok((in_sz, out_sz)) => {
                ok += 1;
                in_sum += in_sz;
                out_sum += out_sz;
                let _ = tx.send(ConvMsg::FileDone { idx: i, ok: true, out_bytes: out_sz });
            }
            Err(e) => {
                let _ = tx.send(ConvMsg::FileDone { idx: i, ok: false, out_bytes: 0 });
                if first_err.is_none() {
                    first_err = Some(format!("{name}: {e}"));
                }
            }
        }
    }

    let failed = total - ok;
    let result = if ok == 0 {
        Err(first_err.unwrap_or_else(|| "nothing converted".into()))
    } else if total == 1 {
        Ok(format!("converted → {} ({} → {}, {})", out_dir.display(), human_size(in_sum), human_size(out_sum), size_delta(in_sum, out_sum)))
    } else {
        let mut msg = format!(
            "{ok}/{total} converted  ·  {} → {} ({})",
            human_size(in_sum),
            human_size(out_sum),
            size_delta(in_sum, out_sum)
        );
        if failed > 0 {
            msg.push_str(&format!("  ·  {failed} failed"));
            if let Some(e) = &first_err {
                msg.push_str(&format!(" ({e})"));
            }
        }
        Ok(msg)
    };
    let _ = tx.send(ConvMsg::Finished(result));
}

fn convert_one(
    src: &Path,
    out_dir: &Path,
    fmt: Fmt,
    quality: u32,
    encoders: &std::collections::HashSet<String>,
    tx: &std::sync::mpsc::Sender<ConvMsg>,
) -> Result<(u64, u64), String> {
    let stem = src.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "out".into());
    let output = out_dir.join(format!("{stem}.{}", fmt.ext));
    let args = build_args(&fmt, src, &output, quality, encoders);
    run_ffmpeg_live(&args, tx)?;
    Ok((file_len(src), file_len(&output)))
}

fn run_ffmpeg_live(
    args: &[String],
    tx: &std::sync::mpsc::Sender<ConvMsg>,
) -> Result<(), String> {
    use std::io::Read;
    let mut child = doctor::quiet_command("ffmpeg")
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("can't launch ffmpeg: {e}"))?;
    let mut stderr = child.stderr.take().ok_or("no ffmpeg stderr")?;

    let mut buf = [0u8; 4096];
    let mut line: Vec<u8> = Vec::new();
    let mut tail: std::collections::VecDeque<String> = std::collections::VecDeque::new();
    let mut duration: Option<f32> = None;

    loop {
        let n = match stderr.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => break,
        };
        for &b in &buf[..n] {
            if b == b'\r' || b == b'\n' {
                if !line.is_empty() {
                    let text = String::from_utf8_lossy(&line).into_owned();
                    parse_progress_line(&text, &mut duration, tx);
                    if tail.len() == 6 {
                        tail.pop_front();
                    }
                    tail.push_back(text);
                    line.clear();
                }
            } else {
                line.push(b);
            }
        }
    }
    if !line.is_empty() {
        tail.push_back(String::from_utf8_lossy(&line).into_owned());
    }

    let status = child.wait().map_err(|e| e.to_string())?;
    if !status.success() {
        let last: Vec<String> =
            tail.iter().rev().filter(|l| !l.trim().is_empty()).take(2).cloned().collect();
        let msg: String = last.into_iter().rev().collect::<Vec<_>>().join(" · ");
        return Err(if msg.trim().is_empty() { "ffmpeg error".into() } else { msg });
    }
    Ok(())
}

fn parse_progress_line(
    line: &str,
    duration: &mut Option<f32>,
    tx: &std::sync::mpsc::Sender<ConvMsg>,
) {
    if duration.is_none()
        && let Some(rest) = line.split("Duration:").nth(1)
    {
        *duration = parse_hms(rest.split(',').next().unwrap_or("").trim());
    }
    if let Some(rest) = line.split("time=").nth(1) {
        let t = parse_hms(rest.split_whitespace().next().unwrap_or(""));
        let speed = line.split("speed=").nth(1).and_then(|s| s.split_whitespace().next());
        let frac = match (t, *duration) {
            (Some(t), Some(d)) if d > 0.0 => Some((t / d).clamp(0.0, 1.0)),
            _ => None,
        };
        let pct = frac.map(|f| format!("{:.0}%", f * 100.0));
        let detail = match (pct, speed) {
            (Some(p), Some(s)) => format!("{p} · {s}"),
            (Some(p), None) => p,
            (None, Some(s)) => s.to_string(),
            (None, None) => "working…".to_string(),
        };
        let _ = tx.send(ConvMsg::Stage { frac, detail });
    }
}

fn parse_hms(s: &str) -> Option<f32> {
    let s = s.trim();
    if s.is_empty() || s.starts_with("N/A") {
        return None;
    }
    let mut secs = 0.0f32;
    for part in s.split(':') {
        secs = secs * 60.0 + part.parse::<f32>().ok()?;
    }
    Some(secs)
}

fn file_len(p: &Path) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

fn build_args(
    fmt: &Fmt,
    input: &Path,
    output: &Path,
    quality: u32,
    encoders: &std::collections::HashSet<String>,
) -> Vec<String> {
    let q = quality.clamp(1, 100);
    let inp = input.to_string_lossy().into_owned();
    let outp = output.to_string_lossy().into_owned();
    let mut a: Vec<String> = Vec::new();
    let push = |a: &mut Vec<String>, parts: &[&str]| a.extend(parts.iter().map(|s| s.to_string()));

    push(&mut a, &["-y", "-i", &inp]);
    match fmt.id {
        "webp" => {
            let qs = q.to_string();
            push(&mut a, &["-c:v", "libwebp", "-quality", &qs, "-compression_level", "6"]);
        }
        "avif" => {
            let crf = ((100 - q) * 63 / 99).to_string();
            push(&mut a, &[
                "-c:v", "libaom-av1", "-still-picture", "1", "-crf", &crf, "-b:v", "0",
                "-cpu-used", "5",
            ]);
        }
        "jxl" => {
            let dist = format!("{:.1}", (100 - q) as f32 * 15.0 / 99.0);
            push(&mut a, &["-c:v", "libjxl", "-distance", &dist, "-effort", "7"]);
        }
        "jpg" => {
            let qs = (2 + (100 - q) * 29 / 99).to_string();
            push(&mut a, &["-c:v", "mjpeg", "-q:v", &qs]);
        }
        "png" => {
            push(&mut a, &["-c:v", "png", "-compression_level", "9"]);
        }
        "webm_vp9" => {
            let crf = ((100 - q) * 63 / 99).to_string();
            push(&mut a, &[
                "-c:v", "libvpx-vp9", "-crf", &crf, "-b:v", "0", "-row-mt", "1",
                "-c:a", "libopus", "-b:a", "128k",
            ]);
        }
        "webm_av1" => {
            let crf = ((100 - q) * 63 / 99).to_string();
            if encoders.contains("libsvtav1") {
                push(&mut a, &["-c:v", "libsvtav1", "-crf", &crf, "-preset", "6"]);
            } else {
                push(&mut a, &[
                    "-c:v", "libaom-av1", "-crf", &crf, "-b:v", "0", "-cpu-used", "5", "-row-mt", "1",
                ]);
            }
            push(&mut a, &["-c:a", "libopus", "-b:a", "128k"]);
        }
        "mp4_h264" => {
            let crf = ((100 - q) * 51 / 99).to_string();
            push(&mut a, &[
                "-c:v", "libx264", "-crf", &crf, "-preset", "medium", "-pix_fmt", "yuv420p",
                "-c:a", "aac", "-b:a", "160k", "-movflags", "+faststart",
            ]);
        }
        "mp4_h265" => {
            let crf = ((100 - q) * 51 / 99).to_string();
            push(&mut a, &[
                "-c:v", "libx265", "-crf", &crf, "-preset", "medium", "-pix_fmt", "yuv420p",
                "-tag:v", "hvc1", "-c:a", "aac", "-b:a", "160k", "-movflags", "+faststart",
            ]);
        }
        "gif" => {
            push(&mut a, &[
                "-vf",
                "fps=15,scale=480:-1:flags=lanczos,split[s0][s1];[s0]palettegen[p];[s1][p]paletteuse",
                "-loop", "0", "-an",
            ]);
        }
        _ => {}
    }
    a.push(outp);
    a
}

fn human_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if b < KB {
        format!("{bytes} B")
    } else if b < KB * KB {
        format!("{:.0} KB", b / KB)
    } else if b < KB * KB * KB {
        format!("{:.1} MB", b / (KB * KB))
    } else {
        format!("{:.1} GB", b / (KB * KB * KB))
    }
}

fn size_delta(input: u64, output: u64) -> String {
    if input == 0 || output == 0 {
        return "size n/a".to_string();
    }
    let pct = (output as f64 / input as f64 - 1.0) * 100.0;
    if pct <= 0.0 {
        format!("−{:.0}% smaller", -pct)
    } else {
        format!("+{:.0}% bigger", pct)
    }
}

#[derive(Default)]
pub struct DoctorTool {
    report: Option<MachineReport>,
    loaded: bool,
    scan_rx: Option<std::sync::mpsc::Receiver<MachineReport>>,
}

impl Tool for DoctorTool {
    fn title(&self) -> &'static str {
        "Machine doctor"
    }
    fn about(&self) -> &'static str {
        "What runtimes & SDKs does this machine have?"
    }
    fn uses_output_dir(&self) -> bool {
        false
    }

    fn ui(&mut self, ui: &mut egui::Ui, octx: &ToolCtx) {
        if !self.loaded {
            self.report = doctor::load_cache(&doctor::cache_path(octx.config_dir));
            self.loaded = true;
        }

        if let Some(rx) = &self.scan_rx {
            match rx.try_recv() {
                Ok(report) => {
                    doctor::save_cache(&doctor::cache_path(octx.config_dir), &report);
                    self.report = Some(report);
                    self.scan_rx = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => ui.ctx().request_repaint(),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.scan_rx = None,
            }
        }
        let scanning = self.scan_rx.is_some();

        ui.horizontal(|ui| {
            let when = match &self.report {
                Some(r) => format!("scanned {}", ago(r.age_secs())),
                None => "never scanned".to_string(),
            };
            ui.label(RichText::new(when).color(DIM).small());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if scanning {
                    ui.add(egui::Spinner::new().size(18.0).color(ACCENT));
                    ui.add_space(4.0);
                    ui.label(RichText::new("Scanning…").color(ACCENT).small());
                } else if tool_button(ui, "Refresh", true) {
                    let cfg = octx.config_dir.to_path_buf();
                    let (tx, rx) = std::sync::mpsc::channel();
                    std::thread::spawn(move || {
                        let probes = doctor::load_probes(&doctor::probes_path(&cfg));
                        let _ = tx.send(doctor::scan(&probes));
                    });
                    self.scan_rx = Some(rx);
                    ui.ctx().request_repaint();
                }
            });
        });
        ui.add_space(10.0);

        let Some(report) = &self.report else {
            ui.label(
                RichText::new("No scan yet - hit Refresh to probe this machine.")
                    .color(DIM)
                    .small(),
            );
            return;
        };

        let (found, missing): (Vec<_>, Vec<_>) = report.engines.iter().partition(|e| e.found);

        egui::ScrollArea::vertical().show(ui, |ui| {
            for e in &found {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("✓").color(ACCENT));
                    ui.label(RichText::new(&e.display).color(FG).strong());
                    if let Some(v) = &e.version {
                        ui.label(
                            RichText::new(v).color(ACCENT).font(FontId::monospace(13.0)),
                        );
                    }
                });
                if let Some(p) = &e.path {
                    ui.label(RichText::new(p).color(FAINT).small());
                }
                ui.add_space(6.0);
            }

            if !missing.is_empty() {
                ui.add_space(8.0);
                ui.label(RichText::new("- not installed -").color(FAINT).small());
                ui.add_space(4.0);
                for e in &missing {
                    ui.label(RichText::new(format!("✗  {}", e.display)).color(DIM));
                }
            }
        });
    }
}

fn ago(secs: u64) -> String {
    match secs {
        0..=4 => "just now".to_string(),
        5..=59 => format!("{secs}s ago"),
        60..=3599 => format!("{}m ago", secs / 60),
        3600..=86399 => format!("{}h ago", secs / 3600),
        _ => format!("{}d ago", secs / 86400),
    }
}

#[derive(Default)]
pub struct MemoryTool {
    loaded: Option<(PathBuf, String)>,
}

impl Tool for MemoryTool {
    fn title(&self) -> &'static str {
        "Memory"
    }
    fn about(&self) -> &'static str {
        "Your project's living NOTES.md, rendered"
    }
    fn uses_output_dir(&self) -> bool {
        false
    }

    fn ui(&mut self, ui: &mut egui::Ui, octx: &ToolCtx) {
        let proj = octx.project_path.to_string_lossy().into_owned();
        if proj.is_empty() {
            ui.label(RichText::new("Open a project first - memory is per project.").color(DIM));
            return;
        }
        let path = crate::notes::notes_path(&proj);

        if self.loaded.as_ref().map(|(p, _)| p != &path).unwrap_or(true) {
            let md = std::fs::read_to_string(&path).unwrap_or_default();
            self.loaded = Some((path.clone(), md));
        }

        ui.horizontal(|ui| {
            ui.label(RichText::new("hyperium-notes/NOTES.md").color(DIM).small());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if tool_button(ui, "Open file", path.exists()) {
                    open_path(&path);
                }
                ui.add_space(6.0);
                if tool_button(ui, "Refresh", true) {
                    let md = std::fs::read_to_string(&path).unwrap_or_default();
                    self.loaded = Some((path.clone(), md));
                }
            });
        });
        ui.add_space(10.0);

        let md = self.loaded.as_ref().map(|(_, m)| m.as_str()).unwrap_or("");
        if md.trim().is_empty() {
            ui.add_space(24.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("No notes yet for this project.").color(FG).size(16.0));
                ui.add_space(6.0);
                ui.label(
                    RichText::new(
                        "Hit Ctrl+P to braindump by voice (or Ctrl+Shift+P to type) - \
                         the scribe files it here, organized.",
                    )
                    .color(DIM),
                );
            });
            return;
        }

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.set_max_width(840.0);
            render_markdown(ui, md);
        });
    }
}

fn open_path(path: &Path) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let _ = std::process::Command::new("explorer")
            .arg(path)
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
    }
    #[cfg(not(windows))]
    let _ = path;
}

fn render_markdown(ui: &mut egui::Ui, md: &str) {
    const BODY: f32 = 15.0;
    const MEASURE: f32 = 840.0;
    for raw in md.lines() {
        let t = raw.trim_end().trim_start();

        if t.is_empty() {
            ui.add_space(8.0);
            continue;
        }
        if let Some(h) = t.strip_prefix("### ") {
            ui.add_space(6.0);
            ui.label(RichText::new(h).color(FG).size(15.5).strong());
            continue;
        }
        if let Some(h) = t.strip_prefix("## ") {
            ui.add_space(12.0);
            ui.label(RichText::new(h).color(ACCENT).size(19.0).strong());
            ui.add_space(3.0);
            let w = ui.available_width().min(MEASURE);
            let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 1.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, 0.0, ACCENT_DIM.gamma_multiply(0.45));
            ui.add_space(5.0);
            continue;
        }
        if let Some(h) = t.strip_prefix("# ") {
            ui.add_space(4.0);
            ui.label(RichText::new(h).color(FG).size(25.0).strong());
            ui.add_space(2.0);
            continue;
        }

        let checkbox = t
            .strip_prefix("- [ ] ")
            .or_else(|| t.strip_prefix("- [] "))
            .map(|r| (false, r))
            .or_else(|| {
                t.strip_prefix("- [x] ").or_else(|| t.strip_prefix("- [X] ")).map(|r| (true, r))
            });
        if let Some((done, rest)) = checkbox {
            let (marker, mcol, tcol) =
                if done { ("☑", ACCENT_DIM, DIM) } else { ("☐", ACCENT, FG) };
            bullet_row(ui, marker, mcol, rest, BODY, tcol, done);
            continue;
        }
        if let Some(rest) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")) {
            bullet_row(ui, "•", ACCENT_DIM, rest, BODY, FG, false);
            continue;
        }

        ui.label(inline_job(t, BODY, FG, ui.available_width().min(MEASURE), false));
    }
}

fn bullet_row(
    ui: &mut egui::Ui,
    marker: &str,
    marker_color: Color32,
    text: &str,
    size: f32,
    color: Color32,
    strike: bool,
) {
    ui.horizontal_top(|ui| {
        ui.add_space(6.0);
        ui.label(RichText::new(marker).color(marker_color).size(size));
        ui.add_space(5.0);
        let w = ui.available_width().max(60.0);
        ui.label(inline_job(text, size, color, w, strike));
    });
}

fn inline_job(text: &str, size: f32, color: Color32, wrap_width: f32, strike: bool) -> egui::text::LayoutJob {
    use egui::text::{LayoutJob, TextFormat};

    fn seg(
        job: &mut LayoutJob,
        buf: &mut String,
        bold: bool,
        code: bool,
        size: f32,
        color: Color32,
        strike: bool,
    ) {
        if buf.is_empty() {
            return;
        }
        let family = if code { FontFamily::Monospace } else { FontFamily::Proportional };
        let mut fmt = TextFormat {
            font_id: FontId::new(if code { size * 0.95 } else { size }, family),
            color: if code {
                ACCENT
            } else if bold {
                Color32::from_rgb(245, 247, 250)
            } else {
                color
            },
            ..Default::default()
        };
        if code {
            fmt.background = BG_HOVER;
        }
        if strike {
            fmt.strikethrough = Stroke::new(1.0, color);
        }
        job.append(buf, 0.0, fmt);
        buf.clear();
    }

    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap_width;
    let chars: Vec<char> = text.chars().collect();
    let (mut i, mut bold, mut code) = (0usize, false, false);
    let mut buf = String::new();
    while i < chars.len() {
        if !code && chars[i] == '*' && chars.get(i + 1) == Some(&'*') {
            seg(&mut job, &mut buf, bold, code, size, color, strike);
            bold = !bold;
            i += 2;
            continue;
        }
        if chars[i] == '`' {
            seg(&mut job, &mut buf, bold, code, size, color, strike);
            code = !code;
            i += 1;
            continue;
        }
        buf.push(chars[i]);
        i += 1;
    }
    seg(&mut job, &mut buf, bold, code, size, color, strike);
    job
}

#[derive(Default)]
pub struct HealthLogTool {
    loaded: bool,
    scores: std::collections::HashMap<String, f32>,
    heat_dates: Vec<String>,
    heat_day: String,
}

impl Tool for HealthLogTool {
    fn title(&self) -> &'static str {
        "Health log"
    }
    fn about(&self) -> &'static str {
        "Your daily habit completion - a year at a glance"
    }
    fn uses_output_dir(&self) -> bool {
        false
    }

    fn ui(&mut self, ui: &mut egui::Ui, octx: &ToolCtx) {
        if !self.loaded {
            self.scores = crate::coach::history_scores(octx.config_dir);
            self.loaded = true;
        }

        ui.horizontal(|ui| {
            let active = self.scores.values().filter(|s| **s > 0.0).count();
            ui.label(
                RichText::new(format!("{active} active day{}", if active == 1 { "" } else { "s" }))
                    .color(DIM)
                    .small(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if tool_button(ui, "Refresh", true) {
                    self.scores = crate::coach::history_scores(octx.config_dir);
                }
            });
        });
        ui.add_space(12.0);

        if self.scores.is_empty() {
            ui.label(
                RichText::new(
                    "No health activity logged yet - confirm a coach nudge and it shows up here.",
                )
                .color(DIM)
                .small(),
            );
            ui.add_space(10.0);
        }

        egui::ScrollArea::both().show(ui, |ui| {
            draw_heatmap(ui, &self.scores, &mut self.heat_dates, &mut self.heat_day);
        });
    }
}

fn draw_heatmap(
    ui: &mut egui::Ui,
    scores: &std::collections::HashMap<String, f32>,
    date_cache: &mut Vec<String>,
    cache_day: &mut String,
) {
    const WEEKS: i64 = 53;
    const CELL: f32 = 13.0;
    const STEP: f32 = 16.0;
    const LEFT: f32 = 32.0;
    const TOP: f32 = 18.0;

    let today = crate::coach::local_date();
    let (ty, tm, td) = parse_ymd(&today);
    let today_n = days_from_civil(ty, tm, td);
    let today_wd = (today_n + 4).rem_euclid(7);
    let start_n = (today_n - today_wd) - (WEEKS - 1) * 7;

    if *cache_day != today || date_cache.len() != (WEEKS * 7) as usize {
        date_cache.clear();
        for i in 0..WEEKS * 7 {
            let (y3, m3, d3) = civil_from_days(start_n + i);
            date_cache.push(format!("{y3:04}-{m3:02}-{d3:02}"));
        }
        *cache_day = today;
    }

    let grid_w = LEFT + WEEKS as f32 * STEP;
    let grid_h = TOP + 7.0 * STEP;
    let (area, _) = ui.allocate_exact_size(egui::vec2(grid_w, grid_h + 28.0), egui::Sense::hover());
    let painter = ui.painter_at(area);
    let origin = area.min;

    const MONTHS: [&str; 12] =
        ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

    for (row, lbl) in [(1i64, "Mon"), (3, "Wed"), (5, "Fri")] {
        let y = origin.y + TOP + row as f32 * STEP;
        painter.text(
            egui::pos2(origin.x, y),
            egui::Align2::LEFT_TOP,
            lbl,
            FontId::new(10.0, FontFamily::Proportional),
            FAINT,
        );
    }

    let mut prev_month = -1i64;
    for col in 0..WEEKS {
        let x = origin.x + LEFT + col as f32 * STEP;
        let (_, m0, _) = civil_from_days(start_n + col * 7);
        if m0 != prev_month {
            painter.text(
                egui::pos2(x, origin.y + 2.0),
                egui::Align2::LEFT_TOP,
                MONTHS[(m0 - 1).clamp(0, 11) as usize],
                FontId::new(10.0, FontFamily::Proportional),
                DIM,
            );
            prev_month = m0;
        }
        for row in 0..7 {
            let n = start_n + col * 7 + row;
            if n > today_n {
                continue;
            }
            let y = origin.y + TOP + row as f32 * STEP;
            let cell = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(CELL, CELL));
            let date = &date_cache[(col * 7 + row) as usize];
            let score = scores.get(date).copied().unwrap_or(0.0);
            painter.rect_filled(cell, 2.0, level_color(score));
            let resp = ui.interact(cell, ui.id().with(("hm", col, row)), egui::Sense::hover());
            if resp.hovered() {
                let txt = if score > 0.0 {
                    format!("{date}  ·  {}%", (score * 100.0).round() as i32)
                } else {
                    format!("{date}  ·  -")
                };
                resp.on_hover_text(txt);
            }
        }
    }

    let ly = origin.y + grid_h + 8.0;
    let mut lx = origin.x + LEFT;
    painter.text(
        egui::pos2(lx, ly + CELL / 2.0),
        egui::Align2::LEFT_CENTER,
        "Less",
        FontId::new(10.0, FontFamily::Proportional),
        FAINT,
    );
    lx += 32.0;
    for level in 0..5 {
        let cell = egui::Rect::from_min_size(egui::pos2(lx, ly), egui::vec2(CELL, CELL));
        painter.rect_filled(cell, 2.0, level_color(level as f32 / 4.0));
        lx += STEP;
    }
    painter.text(
        egui::pos2(lx + 2.0, ly + CELL / 2.0),
        egui::Align2::LEFT_CENTER,
        "More",
        FontId::new(10.0, FontFamily::Proportional),
        FAINT,
    );
}

fn level_color(score: f32) -> Color32 {
    if score <= 0.0 {
        Color32::from_rgb(32, 34, 39)
    } else if score < 0.34 {
        Color32::from_rgb(58, 72, 30)
    } else if score < 0.67 {
        Color32::from_rgb(96, 126, 34)
    } else if score < 1.0 {
        Color32::from_rgb(138, 182, 40)
    } else {
        ACCENT
    }
}

fn parse_ymd(s: &str) -> (i64, i64, i64) {
    let mut it = s.split('-').map(|p| p.parse::<i64>().unwrap_or(0));
    let y = it.next().unwrap_or(0);
    let m = it.next().unwrap_or(1).clamp(1, 12);
    let d = it.next().unwrap_or(1).clamp(1, 31);
    (y, m, d)
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[derive(Clone, Default)]
struct Snippet {
    command: String,
    description: String,
    tags: String,
}

#[derive(Default)]
pub struct CommandMemoTool {
    loaded: Option<PathBuf>,
    items: Vec<Snippet>,
    query: String,
    new_command: String,
    new_desc: String,
    new_tags: String,
    copied: Option<usize>,
}

impl Tool for CommandMemoTool {
    fn title(&self) -> &'static str {
        "Command memo"
    }
    fn about(&self) -> &'static str {
        "Your personal cheatsheet - save commands, search, click to copy"
    }
    fn uses_output_dir(&self) -> bool {
        false
    }

    fn ui(&mut self, ui: &mut egui::Ui, octx: &ToolCtx) {
        let path = cheats_path(octx.config_dir);

        if self.loaded.as_ref() != Some(&path) {
            self.items = load_cheats(&path);
            self.loaded = Some(path.clone());
        }

        ui.label(
            RichText::new("Save the commands you keep re-looking-up. Click a row to copy it.")
                .color(DIM)
                .small(),
        );
        ui.add_space(12.0);

        let field_w = ui.available_width().min(560.0);
        ui.label(RichText::new("command").color(ACCENT).small());
        let cmd_resp = ui.add(
            egui::TextEdit::singleline(&mut self.new_command)
                .desired_width(field_w)
                .font(FontId::new(13.5, FontFamily::Monospace))
                .hint_text("e.g. tar -xzf archive.tar.gz"),
        );
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.new_desc)
                    .desired_width(field_w * 0.62)
                    .hint_text("what it does"),
            );
            ui.add(
                egui::TextEdit::singleline(&mut self.new_tags)
                    .desired_width(field_w * 0.38 - 8.0)
                    .hint_text("tags (optional)"),
            );
        });
        ui.add_space(8.0);

        let enter = cmd_resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let can_save = !self.new_command.trim().is_empty();
        if tool_button(ui, "Save", can_save) || (enter && can_save) {
            self.items.insert(
                0,
                Snippet {
                    command: self.new_command.trim().to_string(),
                    description: self.new_desc.trim().to_string(),
                    tags: self.new_tags.trim().to_string(),
                },
            );
            save_cheats(&path, &self.items);
            self.new_command.clear();
            self.new_desc.clear();
            self.new_tags.clear();
            self.copied = None;
            cmd_resp.request_focus();
        }

        ui.add_space(16.0);

        if self.items.is_empty() {
            ui.label(
                RichText::new("No commands yet - add your first one above.").color(DIM).small(),
            );
            return;
        }

        ui.horizontal(|ui| {
            ui.label(RichText::new("🔍").color(DIM));
            ui.add(
                egui::TextEdit::singleline(&mut self.query)
                    .desired_width(field_w - 28.0)
                    .hint_text("filter by command, description or tag"),
            );
        });
        ui.add_space(8.0);

        let needle = self.query.trim().to_ascii_lowercase();
        let mut remove: Option<usize> = None;
        let mut copy: Option<(usize, String)> = None;
        let mut shown = 0usize;

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.set_max_width(880.0);
            for (i, s) in self.items.iter().enumerate() {
                if !needle.is_empty() && !snippet_matches(s, &needle) {
                    continue;
                }
                shown += 1;
                snippet_row(ui, i, s, self.copied == Some(i), &mut remove, &mut copy);
            }
            if shown == 0 {
                ui.add_space(6.0);
                ui.label(RichText::new("nothing matches that filter").color(FAINT).small());
            }
        });

        if let Some((i, cmd)) = copy {
            ui.ctx().copy_text(cmd);
            self.copied = Some(i);
        }
        if let Some(i) = remove {
            self.items.remove(i);
            save_cheats(&path, &self.items);
            self.copied = None;
        }
    }
}

fn cheats_path(config_dir: &Path) -> PathBuf {
    config_dir.join("command_memo.tsv")
}

fn snippet_matches(s: &Snippet, needle: &str) -> bool {
    s.command.to_ascii_lowercase().contains(needle)
        || s.description.to_ascii_lowercase().contains(needle)
        || s.tags.to_ascii_lowercase().contains(needle)
}

fn snippet_row(
    ui: &mut egui::Ui,
    idx: usize,
    s: &Snippet,
    copied: bool,
    remove: &mut Option<usize>,
    copy: &mut Option<(usize, String)>,
) {
    let (rect, resp) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), if s.description.is_empty() { 36.0 } else { 52.0 }),
        egui::Sense::click(),
    );
    let hov = resp.hovered();
    ui.painter().rect_filled(rect, 8.0, if hov { BG_HOVER } else { BG_ELEVATED });
    if hov {
        ui.painter().rect_stroke(
            rect,
            8.0,
            Stroke::new(1.0, ACCENT_DIM),
            egui::StrokeKind::Inside,
        );
    }

    ui.painter().text(
        egui::pos2(rect.left() + 12.0, rect.top() + 10.0),
        egui::Align2::LEFT_TOP,
        &s.command,
        FontId::new(14.0, FontFamily::Monospace),
        ACCENT,
    );
    if !s.description.is_empty() {
        ui.painter().text(
            egui::pos2(rect.left() + 12.0, rect.bottom() - 9.0),
            egui::Align2::LEFT_BOTTOM,
            &s.description,
            FontId::new(12.0, FontFamily::Proportional),
            DIM,
        );
    }
    let mut right = rect.right() - 12.0;

    let del_rect = egui::Rect::from_center_size(egui::pos2(right - 6.0, rect.center().y), egui::vec2(20.0, 20.0));
    let del = ui.interact(del_rect, ui.id().with(("cheat_del", idx)), egui::Sense::click());
    ui.painter().text(
        del_rect.center(),
        egui::Align2::CENTER_CENTER,
        "✕",
        FontId::new(13.0, FontFamily::Proportional),
        if del.hovered() { RED } else { FAINT },
    );
    if del.clicked() {
        *remove = Some(idx);
    }
    right -= 26.0;

    if copied {
        ui.painter().text(
            egui::pos2(right, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            "copied ✓",
            FontId::new(12.0, FontFamily::Proportional),
            ACCENT,
        );
    } else if !s.tags.is_empty() {
        ui.painter().text(
            egui::pos2(right, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            &s.tags,
            FontId::new(11.5, FontFamily::Proportional),
            FAINT,
        );
    }

    if resp.clicked() && !del.hovered() {
        *copy = Some((idx, s.command.clone()));
    }
    resp.on_hover_text("click to copy");
    ui.add_space(6.0);
}

fn load_cheats(path: &Path) -> Vec<Snippet> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in content.lines() {
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut f = line.split('\t');
        let Some(command) = f.next().filter(|c| !c.is_empty()) else {
            continue;
        };
        out.push(Snippet {
            command: command.to_string(),
            description: f.next().unwrap_or("").to_string(),
            tags: f.next().unwrap_or("").to_string(),
        });
    }
    out
}

fn save_cheats(path: &Path, items: &[Snippet]) {
    let clean = |s: &str| s.replace(['\t', '\r', '\n'], " ");
    let mut out = String::from("# Hyperium command memo - command\\tdescription\\ttags\n");
    for s in items {
        out.push_str(&format!(
            "{}\t{}\t{}\n",
            clean(&s.command),
            clean(&s.description),
            clean(&s.tags)
        ));
    }
    let _ = std::fs::write(path, out);
}
