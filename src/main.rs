#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::time::{Duration, Instant};

use eframe::egui;
use egui::{Color32, CornerRadius, FontFamily, FontId, RichText, Stroke, TextStyle};
use sysinfo::System;

mod audio;
use audio::AudioPlayer;

mod terminal;
use terminal::PtySession;

mod launcher;
use launcher::Command;

mod tools;
use tools::{Tool, ToolCtx};

mod sushi;

mod templates;

mod doctor;

mod coach;

mod notify;

mod tray;

mod stg;

mod icon;

mod llm;

mod notes;

mod voice;

mod backup;

mod genai;

mod sync;

mod update;

mod wiki;

mod manifest;
mod secret;
mod vault;
mod ftp;
mod authenticode;

#[derive(Default)]
struct OpenReq {
    path: std::sync::Mutex<Option<String>>,
    focus: std::sync::atomic::AtomicBool,
}

use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor};

const VERSION: &str = env!("CARGO_PKG_VERSION");

const BUILD_HASH: &str = env!("HYPERIUM_BUILD_HASH");
const BUILD_DATE: &str = env!("HYPERIUM_BUILD_DATE");

const BG_WINDOW: Color32 = Color32::from_rgb(14, 14, 16);
const BG_PANEL: Color32 = Color32::from_rgb(19, 20, 23);
const BG_TERMINAL: Color32 = Color32::from_rgb(9, 9, 11);
const BG_ELEVATED: Color32 = Color32::from_rgb(26, 27, 31);
const BG_HOVER: Color32 = Color32::from_rgb(34, 36, 41);
const BG_SELECTED: Color32 = Color32::from_rgb(30, 32, 28);
const SELECT_BG: Color32 = Color32::from_rgb(54, 62, 40);
const BORDER: Color32 = Color32::from_rgb(32, 33, 38);
const BORDER_SOFT: Color32 = Color32::from_rgb(24, 25, 29);
const SEP: Color32 = Color32::from_rgb(18, 19, 22);

const FG: Color32 = Color32::from_rgb(224, 226, 230);
const DIM: Color32 = Color32::from_rgb(178, 232, 44);
const FAINT: Color32 = Color32::from_rgb(112, 116, 127);

const ACCENT: Color32 = Color32::from_rgb(178, 232, 44);
const ACCENT_DIM: Color32 = Color32::from_rgb(120, 150, 40);
const ORANGE: Color32 = Color32::from_rgb(255, 149, 56);
const RED: Color32 = Color32::from_rgb(226, 92, 92);
const PINK: Color32 = Color32::from_rgb(255, 64, 200);
const DOT_DIRTY: Color32 = ORANGE;
const DOT_CLEAN: Color32 = ACCENT;

const UI_ZOOM: f32 = 1.0;
const TERM_FONT_PX: f32 = 15.5;
const UI_FONT_BYTES: &[u8] = include_bytes!("../assets/fonts/Inter-Medium.ttf");

const MAX_TERMS: usize = 6;
const SPLASH_SECS: f32 = 1.9;

struct Term {
    title: String,
    agent: String,
    session: Option<PtySession>,
    spawned: bool,
}

impl Term {
    fn new(agent: &str) -> Self {
        Self {
            title: agent.to_string(),
            agent: agent.to_string(),
            session: None,
            spawned: false,
        }
    }
}

struct Project {
    name: String,
    path: String,
    branch: String,
    agent: String,
    favorite: bool,
    dirty: bool,
    open: bool,
    terms: Vec<Term>,
    focused: usize,
    out_dir: String,
    col_frac: Vec<f32>,
    row_frac: Vec<f32>,
}

impl Project {
    fn from_path(path: &str, open: bool) -> Self {
        let name = std::path::Path::new(path)
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| path.to_string());
        Self {
            name,
            path: path.to_string(),
            branch: String::new(),
            agent: "claude".into(),
            favorite: false,
            dirty: false,
            open,
            terms: if open { vec![Term::new("shell")] } else { vec![] },
            focused: 0,
            out_dir: default_out_dir(path),
            col_frac: Vec::new(),
            row_frac: Vec::new(),
        }
    }
}

fn default_out_dir(path: &str) -> String {
    std::path::Path::new(path)
        .join("hyperium-out")
        .to_string_lossy()
        .into_owned()
}

enum Screen {
    Splash(Instant),
    Onboard,
    Cockpit,
}

struct HyperiumApp {
    screen: Screen,
    projects: Vec<Project>,
    selected: usize,

    sys: System,
    last_refresh: Instant,
    proc_children: std::collections::HashMap<sysinfo::Pid, Vec<sysinfo::Pid>>,
    cpu: f32,
    mem_used: u64,
    mem_total: u64,

    audio: Option<AudioPlayer>,

    coach: coach::Coach,

    commands: Vec<Command>,
    palette_open: bool,
    palette_query: String,
    palette_sel: usize,
    palette_focus: bool,
    settings_open: bool,
    settings_tab: SettingsTab,
    new_name: String,
    new_path: String,
    new_args: String,
    new_habit_label: String,
    new_habit_noun: String,
    new_habit_verb: String,
    new_habit_icon: String,
    new_habit_target: u32,
    new_habit_min: u32,
    new_habit_max: u32,
    active_tool: Option<Box<dyn Tool>>,
    tool_opened: Instant,

    coach_nudge: Option<coach::Nudge>,
    coach_next_at: Option<Instant>,
    confirm_reset_streaks: bool,
    backup_dir_edit: String,
    confirm_restore: Option<std::path::PathBuf>,

    projects_root_edit: String,
    templates_dir_edit: String,

    ftp_host: String,
    ftp_port: String,
    ftp_user: String,
    ftp_password: String,
    ftp_dir: String,
    ftp_tls: bool,
    sync_passphrase: String,
    sync: std::sync::Arc<std::sync::Mutex<sync::Shared>>,
    confirm_pull: Option<String>,
    sync_badges: std::collections::HashMap<String, sync::Badge>,
    sync_badge_at: Option<Instant>,
    wiki_present: std::collections::HashMap<String, bool>,

    update: std::sync::Arc<std::sync::Mutex<update::Shared>>,

    tray: Option<tray::Tray>,

    open_req: std::sync::Arc<OpenReq>,

    brand: Vec<(u32, egui::TextureHandle)>,

    ai_key: String,
    kie_key: String,
    talker_open: bool,
    talker_voice: bool,
    talker_text: String,
    talker_focus: bool,
    recorder: Option<voice::Recorder>,
    rec_anim: std::sync::Arc<std::sync::Mutex<Option<RecFrames>>>,
    rec_tex: Option<egui::TextureHandle>,
    rec_start: Instant,
    voice_status: Option<(Instant, VoiceStatus)>,
    whisper: std::sync::Arc<std::sync::Mutex<WhisperInstall>>,
    whisper_model_sel: String,

    market_open: bool,
    market_focus: bool,
    market_query: String,
    market_category: Option<String>,
    market_detail: Option<String>,
    market: std::sync::Arc<std::sync::Mutex<templates::Shared>>,
    market_thumbs: std::collections::HashMap<String, egui::TextureHandle>,
    market_opened: Instant,
}

struct VoiceStatus {
    mic: bool,
    model: Option<std::path::PathBuf>,
    cli: bool,
    server: bool,
}

#[derive(Default)]
struct WhisperInstall {
    busy: bool,
    message: String,
    phase: String,
    downloaded: u64,
    total: u64,
    models: Vec<sync::WhisperModel>,
    manifest_tried: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingsTab {
    Launchers,
    Health,
    Ai,
    Backup,
    Sync,
    About,
}

impl HyperiumApp {
    fn new(
        cc: &eframe::CreationContext<'_>,
        listener: std::net::TcpListener,
        launch_target: Option<String>,
    ) -> Self {
        install_fonts(&cc.egui_ctx);
        apply_theme(&cc.egui_ctx);

        std::thread::spawn(|| {
            let dir = config_dir();
            let probes = doctor::load_probes(&doctor::probes_path(&dir));
            doctor::save_cache(&doctor::cache_path(&dir), &doctor::scan(&probes));
        });

        std::thread::spawn(|| voice::warm(&config_dir()));

        std::thread::spawn(|| notify::ensure_registered(&config_dir()));
        std::thread::spawn(|| {
            let ico = icon::ensure_ico(&config_dir());
            stg::register_association(ico.as_deref());
        });

        let rec_anim = std::sync::Arc::new(std::sync::Mutex::new(None));
        {
            let slot = rec_anim.clone();
            std::thread::spawn(move || {
                let frames = decode_rec_gif();
                if !frames.is_empty() {
                    *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(frames);
                }
            });
        }

        let projects = load_projects();
        for p in &projects {
            stg::ensure_file(&p.path);
            manifest::ensure(&p.path);
        }

        {
            let project_dirs: Vec<std::path::PathBuf> =
                projects.iter().map(|p| std::path::PathBuf::from(&p.path)).collect();
            std::thread::spawn(move || {
                let cfg = config_dir();
                let out = backup::configured_dir(&cfg);
                if backup::list(&out).is_empty() || backup::changed_since_last(&cfg, &project_dirs) {
                    let _ = backup::snapshot(&cfg, &project_dirs, &out, backup::KEEP, &backup::stamp());
                }
            });
        }

        let sync_shared = std::sync::Arc::new(std::sync::Mutex::new(sync::Shared::default()));
        let ftp_cfg0 = ftp::load_config(&config_dir());
        if ftp_cfg0.connectable() && !ftp_cfg0.passphrase.is_empty() {
            let shared = sync_shared.clone();
            let ctx = cc.egui_ctx.clone();
            let cfg = ftp_cfg0.clone();
            std::thread::spawn(move || {
                if let Ok(map) = ftp::read_manifest(&cfg) {
                    let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
                    s.message = format!("{} project(s) on the server", map.len());
                    s.server = map;
                    s.fetched = true;
                    drop(s);
                    ctx.request_repaint();
                }
            });
        }

        update::cleanup_old();
        let update_shared = std::sync::Arc::new(std::sync::Mutex::new(update::Shared::default()));
        {
            let cfg = config_dir();
            let shared = update_shared.clone();
            let ctx = cc.egui_ctx.clone();
            std::thread::spawn(move || {
                if let Ok(Some(rel)) = sync::fetch_app_manifest(&cfg) {
                    let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
                    s.checked = true;
                    if update::is_update(&rel) {
                        s.message = format!("update available: v{}", rel.version);
                        s.available = Some(rel);
                    }
                    drop(s);
                    ctx.request_repaint();
                }
            });
        }

        let open_req = std::sync::Arc::new(OpenReq::default());
        if let Some(t) = launch_target {
            *open_req.path.lock().unwrap_or_else(|e| e.into_inner()) = Some(t);
        }
        stg::serve(listener, {
            let ctx = cc.egui_ctx.clone();
            let req = open_req.clone();
            move |msg| {
                let msg = msg.trim().to_string();
                if !msg.is_empty() {
                    *req.path.lock().unwrap_or_else(|e| e.into_inner()) = Some(msg);
                }
                req.focus.store(true, std::sync::atomic::Ordering::SeqCst);
                ctx.request_repaint();
            }
        });

        let brand: Vec<(u32, egui::TextureHandle)> = [18u32, 30, 72]
            .into_iter()
            .filter_map(|px| {
                icon::rgba_resized(px).map(|(rgba, w, h)| {
                    let tex = cc.egui_ctx.load_texture(
                        format!("brand_{px}"),
                        egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba),
                        egui::TextureOptions::LINEAR,
                    );
                    (px, tex)
                })
            })
            .collect();

        let mut sys = System::new_all();
        sys.refresh_all();

        Self {
            screen: Screen::Splash(Instant::now()),
            projects,
            selected: 0,
            sys,
            last_refresh: Instant::now(),
            proc_children: std::collections::HashMap::new(),
            cpu: 0.0,
            mem_used: 0,
            mem_total: 0,
            audio: AudioPlayer::init(&music_dir()),
            coach: coach::Coach::load(&config_dir()),
            commands: launcher::load_commands(&commands_path()),
            palette_open: false,
            palette_query: String::new(),
            palette_sel: 0,
            palette_focus: false,
            settings_open: false,
            settings_tab: SettingsTab::Launchers,
            new_name: String::new(),
            new_path: String::new(),
            new_args: String::new(),
            new_habit_label: String::new(),
            new_habit_noun: String::new(),
            new_habit_verb: String::new(),
            new_habit_icon: String::new(),
            new_habit_target: 20,
            new_habit_min: 2,
            new_habit_max: 5,
            active_tool: None,
            tool_opened: Instant::now(),
            coach_nudge: None,
            coach_next_at: None,
            confirm_reset_streaks: false,
            backup_dir_edit: backup::configured_dir(&config_dir()).display().to_string(),
            confirm_restore: None,
            projects_root_edit: load_projects_root(),
            templates_dir_edit: load_templates_dir_override(),
            ftp_host: ftp_cfg0.host,
            ftp_port: ftp_cfg0.port.to_string(),
            ftp_user: ftp_cfg0.user,
            ftp_password: ftp_cfg0.password,
            ftp_dir: ftp_cfg0.dir,
            ftp_tls: ftp_cfg0.tls,
            sync_passphrase: ftp_cfg0.passphrase,
            sync: sync_shared,
            confirm_pull: None,
            sync_badges: std::collections::HashMap::new(),
            sync_badge_at: None,
            wiki_present: std::collections::HashMap::new(),
            update: update_shared,
            tray: tray::build(cc.egui_ctx.clone()),
            open_req,
            brand,
            ai_key: load_ai_key_local(),
            kie_key: genai::load_key(&config_dir()),
            talker_open: false,
            talker_voice: false,
            talker_text: String::new(),
            talker_focus: false,
            recorder: None,
            rec_anim,
            rec_tex: None,
            rec_start: Instant::now(),
            voice_status: None,
            whisper: std::sync::Arc::new(std::sync::Mutex::new(WhisperInstall::default())),
            whisper_model_sel: String::new(),
            market_open: false,
            market_focus: false,
            market_query: String::new(),
            market_category: None,
            market_detail: None,
            market: std::sync::Arc::new(std::sync::Mutex::new(templates::Shared::default())),
            market_thumbs: std::collections::HashMap::new(),
            market_opened: Instant::now(),
        }
    }

    fn voice_status(&mut self) -> &VoiceStatus {
        let stale = self
            .voice_status
            .as_ref()
            .is_none_or(|(at, _)| at.elapsed() >= Duration::from_secs(2));
        if stale {
            let cfg = config_dir();
            let status = VoiceStatus {
                mic: voice::has_mic(),
                model: voice::find_model(&cfg),
                cli: voice::find_cli(&cfg).is_some(),
                server: voice::find_server(&cfg).is_some(),
            };
            self.voice_status = Some((Instant::now(), status));
        }
        &self.voice_status.as_ref().unwrap().1
    }

    fn effective_ai_key(&self) -> String {
        self.ai_key.trim().to_string()
    }

    fn brand_icon(&self, px: u32) -> Option<&egui::TextureHandle> {
        self.brand.iter().find(|(s, _)| *s == px).map(|(_, t)| t)
    }

    fn refresh_metrics(&mut self) {
        if self.last_refresh.elapsed() >= Duration::from_millis(900) {
            self.sys.refresh_memory();
            self.sys.refresh_cpu_usage();
            self.sys
                .refresh_processes(sysinfo::ProcessesToUpdate::All, true);
            self.cpu = self.sys.global_cpu_usage();
            self.mem_used = self.sys.used_memory();
            self.mem_total = self.sys.total_memory();
            self.proc_children.clear();
            for (pid, p) in self.sys.processes() {
                if let Some(parent) = p.parent() {
                    self.proc_children.entry(parent).or_default().push(*pid);
                }
            }
            self.last_refresh = Instant::now();
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        use egui::{Key, Modifiers};
        let cs = Modifiers::CTRL | Modifiers::SHIFT;

        if self.active_tool.is_some() {
            return;
        }

        if ctx.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::Space)) {
            self.palette_open = !self.palette_open;
            if self.palette_open {
                self.palette_query.clear();
                self.palette_sel = 0;
                self.palette_focus = true;
            }
        }
        if ctx.input_mut(|i| i.consume_key(cs, Key::P)) {
            self.talker_open = true;
            self.talker_voice = false;
            self.talker_text.clear();
            self.talker_focus = true;
            self.recorder = None;
        } else if ctx.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::P)) {
            self.talker_open = true;
            self.talker_voice = true;
            self.talker_text.clear();
            self.talker_focus = false;
            self.rec_start = Instant::now();
            self.recorder = if voice::find_model(&config_dir()).is_some() {
                voice::Recorder::start()
            } else {
                None
            };
        }

        if ctx.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::T)) && !self.market_open {
            self.sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
            if self.focused_pane_has_claude() {
                self.open_market(ctx);
            } else {
                notify::toast(
                    "Hyperium - templates",
                    "Open a claude in this terminal to use the templates.",
                );
            }
        }

        if self.palette_open || self.talker_open || self.market_open {
            return;
        }

        if ctx.input_mut(|i| i.consume_key(cs, Key::T))
            && let Some(p) = self.projects.get_mut(self.selected).filter(|p| p.open)
            && p.terms.len() < MAX_TERMS
        {
            p.terms.push(Term::new("shell"));
            p.focused = p.terms.len() - 1;
        }

        if ctx.input_mut(|i| i.consume_key(cs, Key::W))
            && let Some(p) = self.projects.get_mut(self.selected).filter(|p| p.open)
            && p.terms.len() > 1
        {
            p.terms.remove(p.focused);
            p.focused = p.focused.min(p.terms.len() - 1);
        }

        let next_pane = ctx.input_mut(|i| i.consume_key(cs, Key::ArrowRight));
        let prev_pane = ctx.input_mut(|i| i.consume_key(cs, Key::ArrowLeft));
        if (next_pane || prev_pane)
            && let Some(p) = self.projects.get_mut(self.selected).filter(|p| p.open)
        {
            let n = p.terms.len();
            if n > 0 {
                p.focused = if next_pane {
                    (p.focused + 1) % n
                } else {
                    (p.focused + n - 1) % n
                };
            }
        }

        let next_tab = ctx.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::Tab));
        let prev_tab = ctx.input_mut(|i| i.consume_key(cs, Key::Tab));
        if next_tab || prev_tab {
            let open: Vec<usize> = self
                .projects
                .iter()
                .enumerate()
                .filter(|(_, p)| p.open)
                .map(|(i, _)| i)
                .collect();
            if !open.is_empty() {
                let cur = open.iter().position(|&i| i == self.selected).unwrap_or(0);
                let m = open.len();
                let ni = if next_tab { (cur + 1) % m } else { (cur + m - 1) % m };
                self.selected = open[ni];
            }
        }
    }

    fn open_project(&mut self, i: usize) {
        if let Some(p) = self.projects.get_mut(i) {
            p.open = true;
            if p.terms.is_empty() {
                p.terms.push(Term::new("shell"));
                p.focused = 0;
            }
            self.selected = i;
        }
    }

    fn add_project(&mut self, path: String) {
        let i = match self.projects.iter().position(|p| p.path == path) {
            Some(i) => i,
            None => {
                self.projects.push(Project::from_path(&path, false));
                self.projects.len() - 1
            }
        };
        self.open_project(i);
        self.save_state();
        stg::ensure_file(&path);
        manifest::ensure(&path);
    }

    fn save_state(&self) {
        let mut out = String::new();
        for p in &self.projects {
            out.push(if p.open { '1' } else { '0' });
            out.push('\t');
            out.push_str(&p.path);
            out.push('\t');
            out.push_str(&p.out_dir);
            out.push('\n');
        }
        let _ = std::fs::write(state_path(), out);
    }

    fn save_commands(&self) {
        launcher::save_commands(&commands_path(), &self.commands);
    }

    fn schedule_next_nudge(&mut self) {
        let secs = self.coach.next_interval_secs();
        self.coach_next_at = Some(Instant::now() + Duration::from_secs(secs));
    }

    fn draw_coach(&mut self, ctx: &egui::Context) {
        let Some(nudge) = self.coach_nudge.clone() else {
            return;
        };
        let screen = ctx.content_rect();
        let prog = self.coach.progress_of(&nudge.habit_id);
        let habit = self.coach.habits.iter().find(|h| h.id == nudge.habit_id).cloned();

        egui::Area::new(egui::Id::new("coach_backdrop"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                ui.painter().rect_filled(screen, 0.0, Color32::from_black_alpha(225));
            });

        let panel = egui::Rect::from_center_size(screen.center(), egui::vec2(440.0, 230.0));
        let mut done = false;
        egui::Area::new(egui::Id::new("coach_panel"))
            .order(egui::Order::Foreground)
            .fixed_pos(panel.min)
            .show(ctx, |ui| {
                egui::Frame::default()
                    .fill(BG_WINDOW)
                    .stroke(Stroke::new(1.0, ACCENT))
                    .corner_radius(14.0)
                    .inner_margin(egui::Margin::same(24))
                    .show(ui, |ui| {
                        ui.set_width(panel.width() - 48.0);
                        ui.vertical_centered(|ui| {
                            ui.label(
                                RichText::new("MOVE - maintain the archer").color(DIM).small(),
                            );
                            ui.add_space(12.0);
                            ui.label(RichText::new(nudge.message()).color(FG).size(26.0).strong());
                            ui.add_space(8.0);
                            if let Some(h) = &habit {
                                ui.label(
                                    RichText::new(format!(
                                        "{}/{} {} today  ·  🔥 {}",
                                        prog.done, h.daily_target, h.noun, prog.streak
                                    ))
                                    .color(DIM)
                                    .small(),
                                );
                            }
                            ui.add_space(18.0);
                            if pill_button(ui, "Done ✓  -  back to flow", 0.0, true) {
                                done = true;
                            }
                        });
                    });
            });

        if done {
            self.coach.mark_done(&nudge.habit_id, nudge.amount);
            let cfg = config_dir();
            self.coach.save_state(&cfg);
            self.coach.save_history(&cfg);
            self.coach_nudge = None;
            self.schedule_next_nudge();
        }
    }

    fn open_tool(&mut self, id: &str) {
        if let Some(tool) = tools::make_tool(id) {
            self.active_tool = Some(tool);
            self.tool_opened = Instant::now();
            self.palette_open = false;
        }
    }

    fn palette_candidates(&self) -> Vec<Command> {
        let mut all: Vec<Command> = tools::BUILTIN
            .iter()
            .map(|(id, title)| Command {
                name: (*title).to_string(),
                kind: launcher::CommandKind::Internal { id: (*id).to_string() },
            })
            .collect();
        all.extend(self.commands.iter().cloned());
        all
    }

    fn draw_palette(&mut self, ctx: &egui::Context) {
        let candidates = self.palette_candidates();
        let matched = launcher::match_commands(&candidates, &self.palette_query);
        let n = matched.len();
        if self.palette_sel >= n {
            self.palette_sel = 0;
        }

        let none = egui::Modifiers::NONE;
        if n > 0 && ctx.input_mut(|i| i.consume_key(none, egui::Key::ArrowDown)) {
            self.palette_sel = (self.palette_sel + 1) % n;
        }
        if n > 0 && ctx.input_mut(|i| i.consume_key(none, egui::Key::ArrowUp)) {
            self.palette_sel = (self.palette_sel + n - 1) % n;
        }
        if ctx.input_mut(|i| i.consume_key(none, egui::Key::Escape)) {
            self.palette_open = false;
            return;
        }
        let mut launch: Option<usize> = None;
        if n > 0 && ctx.input_mut(|i| i.consume_key(none, egui::Key::Enter)) {
            launch = Some(self.palette_sel);
        }

        let screen = ctx.content_rect();

        egui::Area::new(egui::Id::new("palette_backdrop"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                ui.painter().rect_filled(screen, 0.0, Color32::from_black_alpha(180));
            });

        let bw = 600.0;
        let bx = screen.center().x - bw / 2.0;
        let by = screen.top() + (screen.height() * 0.16).max(70.0);
        let area = egui::Area::new(egui::Id::new("palette_box"))
            .order(egui::Order::Foreground)
            .fixed_pos(egui::pos2(bx, by))
            .show(ctx, |ui| {
                egui::Frame::default()
                    .fill(BG_ELEVATED)
                    .stroke(Stroke::new(1.0, ACCENT_DIM))
                    .corner_radius(12.0)
                    .inner_margin(egui::Margin::same(14))
                    .show(ui, |ui| {
                        ui.set_width(bw);

                        let resp = ui.add(
                            egui::TextEdit::singleline(&mut self.palette_query)
                                .hint_text("type a command…  (3+ letters to filter)")
                                .desired_width(f32::INFINITY)
                                .font(FontId::new(19.0, FontFamily::Proportional)),
                        );
                        if self.palette_focus {
                            resp.request_focus();
                            self.palette_focus = false;
                        }
                        if resp.changed() {
                            self.palette_sel = 0;
                        }

                        ui.add_space(8.0);
                        ui.separator();
                        ui.add_space(6.0);

                        if matched.is_empty() {
                            ui.label(RichText::new("no match").color(DIM).small());
                        } else {
                            for (i, c) in matched.iter().enumerate() {
                                let selected = i == self.palette_sel;
                                let (rect, r) = ui.allocate_exact_size(
                                    egui::vec2(ui.available_width(), 34.0),
                                    egui::Sense::click(),
                                );
                                let hov = r.hovered();
                                if selected || hov {
                                    ui.painter().rect_filled(
                                        rect,
                                        7.0,
                                        if selected { BG_SELECTED } else { BG_HOVER },
                                    );
                                }
                                if selected {
                                    let bar = egui::Rect::from_min_max(
                                        rect.left_top(),
                                        egui::pos2(rect.left() + 3.0, rect.bottom()),
                                    );
                                    ui.painter().rect_filled(bar, CornerRadius::same(2), ACCENT);
                                }
                                let p = ui.painter();
                                p.text(
                                    egui::pos2(rect.left() + 14.0, rect.center().y),
                                    egui::Align2::LEFT_CENTER,
                                    &c.name,
                                    FontId::new(15.0, FontFamily::Proportional),
                                    if selected { ACCENT } else { FG },
                                );
                                p.text(
                                    egui::pos2(rect.right() - 12.0, rect.center().y),
                                    egui::Align2::RIGHT_CENTER,
                                    c.detail(),
                                    FontId::new(11.5, FontFamily::Monospace),
                                    DIM,
                                );
                                if r.clicked() {
                                    launch = Some(i);
                                }
                                if hov {
                                    self.palette_sel = i;
                                }
                            }
                        }

                        ui.add_space(10.0);
                        ui.label(
                            RichText::new("↑↓ navigate     ⏎ launch     esc close")
                                .color(FAINT)
                                .small(),
                        );
                    });
            });

        let box_rect = area.response.rect;
        let outside_click = ctx.input(|i| {
            i.pointer.primary_clicked()
                && i.pointer.interact_pos().is_some_and(|p| !box_rect.contains(p))
        });
        if outside_click {
            self.palette_open = false;
        }

        if let Some(i) = launch {
            if let Some(c) = matched.get(i) {
                match &c.kind {
                    launcher::CommandKind::Internal { id } => self.open_tool(id),
                    launcher::CommandKind::External { .. } => {
                        let _ = c.launch();
                    }
                }
            }
            self.palette_open = false;
        }
    }

    fn focused_pane_has_claude(&self) -> bool {
        let Some(proj) = self.projects.get(self.selected) else {
            return false;
        };
        let Some(term) = proj.terms.get(proj.focused) else {
            return false;
        };
        let Some(pid) = term.session.as_ref().and_then(|s| s.pid()) else {
            return false;
        };
        let mut children: std::collections::HashMap<sysinfo::Pid, Vec<sysinfo::Pid>> =
            std::collections::HashMap::new();
        for (p, proc_) in self.sys.processes() {
            if let Some(parent) = proc_.parent() {
                children.entry(parent).or_default().push(*p);
            }
        }
        proc_tree_has_claude(&self.sys, &children, pid)
    }

    fn open_market(&mut self, ctx: &egui::Context) {
        self.market_open = true;
        self.market_focus = true;
        self.market_detail = None;
        self.market_query.clear();
        self.market_opened = Instant::now();
        let loaded = self.market.lock().unwrap_or_else(|e| e.into_inner()).loaded;
        if !loaded {
            self.start_market_refresh(ctx);
        }
    }

    fn start_market_refresh(&mut self, ctx: &egui::Context) {
        let shared = self.market.clone();
        let cfg = config_dir();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            templates::refresh(shared, cfg);
            ctx.request_repaint();
        });
    }

    fn draw_market(&mut self, ctx: &egui::Context) {
        if let Some(cmd) = self.market.lock().unwrap_or_else(|e| e.into_inner()).to_type.take() {
            if let Some(proj) = self.projects.get_mut(self.selected)
                && let Some(term) = proj.terms.get_mut(proj.focused)
                && let Some(s) = term.session.as_mut()
            {
                s.send_input(format!("{cmd}\r").as_bytes());
            }
            self.market_open = false;
            return;
        }

        let screen = ctx.content_rect();
        let t = (self.market_opened.elapsed().as_secs_f32() / 0.18).clamp(0.0, 1.0);
        if t < 1.0 {
            ctx.request_repaint();
        }

        let none = egui::Modifiers::NONE;
        if ctx.input_mut(|i| i.consume_key(none, egui::Key::Escape)) {
            if self.market_detail.is_some() {
                self.market_detail = None;
            } else {
                self.market_open = false;
                return;
            }
        }

        let (entries, busy, message) = {
            let s = self.market.lock().unwrap_or_else(|e| e.into_inner());
            (s.entries.clone(), s.busy, s.message.clone())
        };

        for e in &entries {
            if !e.has_thumb || self.market_thumbs.contains_key(&e.id) {
                continue;
            }
            let p = templates::thumb_path(&config_dir(), &e.id);
            if let Ok(bytes) = std::fs::read(&p)
                && let Ok(img) = image::load_from_memory(&bytes)
            {
                let rgba = img
                    .resize_to_fill(800, 450, image::imageops::FilterType::Lanczos3)
                    .to_rgba8();
                let (w, h) = (rgba.width() as usize, rgba.height() as usize);
                let color = egui::ColorImage::from_rgba_unmultiplied([w, h], rgba.as_raw());
                let tex =
                    ctx.load_texture(format!("tpl_{}", e.id), color, egui::TextureOptions::LINEAR);
                self.market_thumbs.insert(e.id.clone(), tex);
            }
        }

        egui::Area::new(egui::Id::new("market_backdrop"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                ui.painter().rect_filled(screen, 0.0, Color32::from_black_alpha((225.0 * t) as u8));
            });

        let cats = templates::categories(&entries);
        let mut query = self.market_query.clone();
        let mut category = self.market_category.clone();
        let detail = self.market_detail.clone();
        let want_focus = std::mem::take(&mut self.market_focus);
        let thumbs = &self.market_thumbs;

        let mut close = false;
        let mut open_id: Option<String> = None;
        let mut back = false;
        let mut use_id: Option<String> = None;
        let mut do_refresh = false;

        let scale = 0.97 + 0.03 * t;
        let w = (screen.width() * 0.60).clamp(560.0, 1400.0);
        let h = (screen.height() * 0.74).clamp(420.0, 920.0);
        let rect = egui::Rect::from_center_size(screen.center(), egui::vec2(w, h) * scale);
        egui::Area::new(egui::Id::new("market_panel"))
            .order(egui::Order::Foreground)
            .fixed_pos(rect.min)
            .show(ctx, |ui| {
                ui.set_opacity(t);
                egui::Frame::default()
                    .fill(BG_WINDOW)
                    .stroke(Stroke::new(1.0, ACCENT_DIM))
                    .corner_radius(12.0)
                    .inner_margin(egui::Margin::same(18))
                    .show(ui, |ui| {
                        ui.set_width(rect.width() - 36.0);
                        ui.set_height(rect.height() - 36.0);
                        {
                            let v = ui.visuals_mut();
                            v.widgets.inactive.weak_bg_fill = BG_ELEVATED;
                            v.widgets.inactive.bg_fill = BG_ELEVATED;
                            v.widgets.hovered.weak_bg_fill = BG_HOVER;
                            v.widgets.hovered.bg_fill = BG_HOVER;
                            v.widgets.active.weak_bg_fill = BG_HOVER;
                            v.widgets.active.bg_fill = BG_HOVER;
                            v.selection.bg_fill = SELECT_BG;
                            v.extreme_bg_color = BG_TERMINAL;
                        }

                        if let Some(id) = &detail {
                            if let Some(e) = entries.iter().find(|e| &e.id == id) {
                                draw_market_detail(ui, e, thumbs.get(id), &mut back, &mut use_id);
                            } else {
                                back = true;
                            }
                            return;
                        }

                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("Template market").color(FG).size(20.0).strong(),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let close_btn =
                                        egui::Button::new(RichText::new("Close").color(DIM))
                                            .frame(false);
                                    if ui.add(close_btn).clicked() {
                                        close = true;
                                    }
                                    let refresh_btn =
                                        egui::Button::new(RichText::new("Refresh").color(DIM))
                                            .frame(false);
                                    if ui.add(refresh_btn).clicked() {
                                        do_refresh = true;
                                    }
                                    if busy {
                                        ui.spinner();
                                    }
                                },
                            );
                        });
                        ui.add_space(6.0);
                        let te = egui::TextEdit::singleline(&mut query)
                            .hint_text("Search templates...")
                            .desired_width(f32::INFINITY);
                        let resp = ui.add(te);
                        if want_focus {
                            resp.request_focus();
                        }
                        ui.add_space(8.0);

                        ui.horizontal_top(|ui| {
                            ui.vertical(|ui| {
                                ui.set_width(150.0);
                                if ui.selectable_label(category.is_none(), "All").clicked() {
                                    category = None;
                                }
                                for c in &cats {
                                    let on = category.as_deref() == Some(c.as_str());
                                    if ui.selectable_label(on, c).clicked() {
                                        category = if on { None } else { Some(c.clone()) };
                                    }
                                }
                            });
                            ui.separator();
                            ui.vertical(|ui| {
                                if entries.is_empty() {
                                    ui.add_space(20.0);
                                    if busy {
                                        ui.horizontal(|ui| {
                                            ui.spinner();
                                            ui.label(
                                                RichText::new("Loading templates...").color(DIM),
                                            );
                                        });
                                    } else if !message.is_empty() {
                                        ui.label(
                                            RichText::new(format!("Could not load: {message}"))
                                                .color(DIM),
                                        );
                                    } else {
                                        ui.label(
                                            RichText::new("No templates on the server yet.")
                                                .color(DIM),
                                        );
                                    }
                                    return;
                                }
                                egui::ScrollArea::vertical().auto_shrink([false, false]).show(
                                    ui,
                                    |ui| {
                                        ui.spacing_mut().item_spacing = egui::vec2(16.0, 18.0);
                                        ui.horizontal_wrapped(|ui| {
                                            for e in entries.iter().filter(|e| {
                                                templates::matches(e, &query)
                                                    && category
                                                        .as_ref()
                                                        .is_none_or(|c| &e.category == c)
                                            }) {
                                                if market_card(ui, e, thumbs.get(&e.id)) {
                                                    open_id = Some(e.id.clone());
                                                }
                                            }
                                        });
                                    },
                                );
                            });
                        });
                    });
            });

        self.market_query = query;
        self.market_category = category;
        if back {
            self.market_detail = None;
        }
        if let Some(id) = open_id {
            self.market_detail = Some(id);
        }
        if let Some(id) = use_id {
            self.use_market_template(ctx, &id, &entries);
        }
        if do_refresh {
            self.start_market_refresh(ctx);
        }
        if close {
            self.market_open = false;
        }
    }

    fn use_market_template(&mut self, ctx: &egui::Context, id: &str, entries: &[templates::Entry]) {
        let Some(entry) = entries.iter().find(|e| e.id == id).cloned() else {
            return;
        };
        let Some(project_dir) = self.projects.get(self.selected).map(|p| p.path.clone()) else {
            return;
        };
        if project_dir.is_empty() {
            notify::toast("Hyperium - templates", "Open a project first.");
            return;
        }
        let shared = self.market.clone();
        let cfg = config_dir();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            templates::use_template(shared, cfg, std::path::PathBuf::from(project_dir), entry);
            ctx.request_repaint();
        });
    }

    fn draw_tool(&mut self, ctx: &egui::Context) {
        let screen = ctx.content_rect();
        let t = (self.tool_opened.elapsed().as_secs_f32() / 0.18).clamp(0.0, 1.0);
        if t < 1.0 {
            ctx.request_repaint();
        }

        let sel = self.selected;
        let (pname, ppath, mut out_dir) = match self.projects.get(sel) {
            Some(p) => (p.name.clone(), p.path.clone(), p.out_dir.clone()),
            None => (String::new(), String::new(), String::new()),
        };
        if out_dir.is_empty() && !ppath.is_empty() {
            out_dir = default_out_dir(&ppath);
        }

        egui::Area::new(egui::Id::new("tool_backdrop"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                ui.painter().rect_filled(screen, 0.0, Color32::from_black_alpha((225.0 * t) as u8));
            });

        let scale = 0.97 + 0.03 * t;
        let rect = egui::Rect::from_center_size(screen.center(), screen.size() * scale).shrink(12.0);
        let mut close = false;
        egui::Area::new(egui::Id::new("tool_panel"))
            .order(egui::Order::Foreground)
            .fixed_pos(rect.min)
            .show(ctx, |ui| {
                ui.set_opacity(t);
                egui::Frame::default()
                    .fill(BG_WINDOW)
                    .stroke(Stroke::new(1.0, ACCENT_DIM))
                    .corner_radius(12.0)
                    .inner_margin(egui::Margin::same(18))
                    .show(ui, |ui| {
                        let inner = egui::vec2(
                            (rect.width() - 36.0).max(0.0),
                            (rect.height() - 36.0).max(0.0),
                        );
                        ui.set_min_size(inner);
                        ui.set_max_size(inner);

                        ui.horizontal(|ui| {
                            if pill_button(ui, "←  back", 0.0, true) {
                                close = true;
                            }
                            ui.add_space(10.0);
                            let title = self.active_tool.as_ref().map(|t| t.title()).unwrap_or("");
                            ui.label(RichText::new(title).color(ACCENT).heading());
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let tag = if pname.is_empty() {
                                    "no project open".to_string()
                                } else {
                                    format!("project · {pname}")
                                };
                                ui.label(RichText::new(tag).color(DIM).small());
                            });
                        });
                        if let Some(about) = self.active_tool.as_ref().map(|t| t.about()) {
                            ui.label(RichText::new(about).color(DIM).small());
                        }
                        ui.add_space(10.0);

                        let shows_out_dir =
                            self.active_tool.as_ref().map(|t| t.uses_output_dir()).unwrap_or(false);
                        if shows_out_dir {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("output dir").color(DIM).small());
                                ui.add(egui::TextEdit::singleline(&mut out_dir).desired_width(380.0));
                                if pill_button(ui, "browse…", 0.0, true)
                                    && let Some(dir) = rfd::FileDialog::new().pick_folder()
                                {
                                    out_dir = dir.to_string_lossy().into_owned();
                                }
                            });
                            ui.add_space(8.0);
                        }
                        ui.separator();
                        ui.add_space(12.0);

                        let cfg = config_dir();
                        let octx = ToolCtx {
                            out_dir: std::path::Path::new(&out_dir),
                            config_dir: &cfg,
                            project_path: std::path::Path::new(&ppath),
                        };
                        if let Some(tool) = self.active_tool.as_mut() {
                            tool.ui(ui, &octx);
                        }
                    });
            });

        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            close = true;
        }

        if let Some(p) = self.projects.get_mut(sel)
            && p.out_dir != out_dir
        {
            p.out_dir = out_dir;
            self.save_state();
        }
        if close {
            self.active_tool = None;
        }
    }

    fn draw_talker(&mut self, ctx: &egui::Context) {
        let project = self
            .projects
            .get(self.selected)
            .filter(|p| p.open)
            .map(|p| (p.name.clone(), p.path.clone()));

        let key_missing = self.effective_ai_key().is_empty();

        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.talker_open = false;
            self.recorder = None;
            return;
        }
        let send_key = ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::Enter));

        let mut do_send = send_key;
        let screen = ctx.content_rect();

        if self.talker_voice {
            let frame: Option<egui::ColorImage> = {
                let guard = self.rec_anim.lock().unwrap_or_else(|e| e.into_inner());
                guard.as_ref().filter(|f| !f.is_empty()).map(|frames| {
                    let total: u32 = frames.iter().map(|(_, d)| *d).sum::<u32>().max(1);
                    let mut t = (self.rec_start.elapsed().as_millis() as u32) % total;
                    let mut idx = frames.len() - 1;
                    for (i, (_, d)) in frames.iter().enumerate() {
                        if t < *d {
                            idx = i;
                            break;
                        }
                        t -= *d;
                    }
                    frames[idx].0.clone()
                })
            };
            if let Some(img) = frame {
                match &mut self.rec_tex {
                    Some(tex) => tex.set(img, egui::TextureOptions::LINEAR),
                    None => {
                        self.rec_tex =
                            Some(ctx.load_texture("rec_anim", img, egui::TextureOptions::LINEAR));
                    }
                }
            }
            ctx.request_repaint();

            egui::Area::new(egui::Id::new("talker_voice"))
                .order(egui::Order::Foreground)
                .fixed_pos(screen.min)
                .show(ctx, |ui| {
                    let p = ui.painter();
                    p.rect_filled(screen, 0.0, Color32::from_black_alpha(150));
                    let c = screen.center();
                    if let Some(tex) = &self.rec_tex {
                        let s = 110.0;
                        p.image(
                            tex.id(),
                            egui::Rect::from_center_size(c, egui::vec2(s, s)),
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
                    if project.is_none() {
                        p.text(
                            egui::pos2(c.x, c.y + 80.0),
                            egui::Align2::CENTER_CENTER,
                            "Open a project first",
                            FontId::new(14.0, FontFamily::Proportional),
                            DIM,
                        );
                    } else if self.recorder.is_none() {
                        p.text(
                            egui::pos2(c.x, c.y + 80.0),
                            egui::Align2::CENTER_CENTER,
                            "🎤 no mic - Ctrl+Shift+P to type",
                            FontId::new(14.0, FontFamily::Proportional),
                            ORANGE,
                        );
                    } else if key_missing {
                        p.text(
                            egui::pos2(c.x, c.y + 80.0),
                            egui::Align2::CENTER_CENTER,
                            "⚠ Add your Anthropic key in Settings → AI to file voice notes",
                            FontId::new(14.0, FontFamily::Proportional),
                            ORANGE,
                        );
                    }
                });
        } else {
            egui::Area::new(egui::Id::new("talker_backdrop"))
                .order(egui::Order::Foreground)
                .fixed_pos(screen.min)
                .show(ctx, |ui| {
                    ui.painter().rect_filled(screen, 0.0, Color32::from_black_alpha(180));
                });

            let bw = 600.0;
            let bx = screen.center().x - bw / 2.0;
            let by = screen.top() + (screen.height() * 0.16).max(70.0);
            egui::Area::new(egui::Id::new("talker_box"))
                .order(egui::Order::Foreground)
                .fixed_pos(egui::pos2(bx, by))
                .show(ctx, |ui| {
                    egui::Frame::default()
                        .fill(BG_ELEVATED)
                        .stroke(Stroke::new(1.0, ACCENT_DIM))
                        .corner_radius(12.0)
                        .inner_margin(egui::Margin::same(16))
                        .show(ui, |ui| {
                            ui.set_width(bw);
                            match &project {
                                Some((name, _)) => {
                                    ui.horizontal(|ui| {
                                        if let Some(t) = self.brand_icon(18) {
                                            ui.add(egui::Image::new((t.id(), egui::vec2(18.0, 18.0))));
                                            ui.add_space(4.0);
                                        }
                                        ui.label(RichText::new("Braindump").color(ACCENT).strong().size(16.0));
                                        ui.label(RichText::new(format!("→ {name}")).color(DIM));
                                    });
                                    ui.label(
                                        RichText::new("Type your note - the LLM files it into NOTES.md (Ctrl+P for voice)")
                                            .color(FAINT)
                                            .small(),
                                    );
                                    ui.add_space(8.0);

                                    let resp = ui.add(
                                        egui::TextEdit::multiline(&mut self.talker_text)
                                            .hint_text("whatever's on your mind…")
                                            .desired_width(f32::INFINITY)
                                            .desired_rows(5)
                                            .font(FontId::new(16.0, FontFamily::Proportional)),
                                    );
                                    if self.talker_focus {
                                        resp.request_focus();
                                        self.talker_focus = false;
                                    }

                                    ui.add_space(10.0);
                                    ui.horizontal(|ui| {
                                        let ready = !self.talker_text.trim().is_empty();
                                        if pill_button(ui, "Send  (Ctrl+Enter)", 0.0, ready) {
                                            do_send = true;
                                        }
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            if key_missing {
                                                ui.label(RichText::new("⚠ Add your Anthropic key (⚙ → AI) to file notes").color(ORANGE).small());
                                            }
                                        });
                                    });
                                }
                                None => {
                                    ui.label(RichText::new("Open a project first - notes are per project.").color(DIM));
                                }
                            }
                        });
                });
        }

        if do_send {
            self.talker_open = false;
            let typed = self.talker_text.trim().to_string();
            let audio = self.recorder.take().map(|r| r.finish()).unwrap_or_default();
            if let Some((_, path)) = &project
                && (!typed.is_empty() || !audio.is_empty())
            {
                if key_missing {
                    notify::toast(
                        "Hyperium - note not filed",
                        "Add your Anthropic key in Settings → AI to file braindumps.",
                    );
                } else {
                    let key = self.effective_ai_key();
                    let path = path.clone();
                    std::thread::spawn(move || {
                        let mut dump = String::new();
                        if !audio.is_empty() {
                            match voice::transcribe(&config_dir(), &audio) {
                                Ok(t) => dump.push_str(&t),
                                Err(e) => notify::toast("Hyperium - voice", &e),
                            }
                        }
                        if !typed.is_empty() {
                            if !dump.is_empty() {
                                dump.push('\n');
                            }
                            dump.push_str(&typed);
                        }
                        if dump.trim().is_empty() {
                            return;
                        }
                        let provider = llm::Anthropic::new(key);
                        match notes::capture(&path, &dump, &provider) {
                            Ok(_) => notify::toast("Hyperium - note ✓", "Filed into NOTES.md."),
                            Err(e) => notify::toast("Hyperium - note failed", &e),
                        }
                    });
                }
            }
        }
    }

    fn draw_settings(&mut self, ctx: &egui::Context) {
        let mut open = self.settings_open;
        egui::Window::new(RichText::new("⚙  Settings").color(FG).strong())
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .fixed_size(egui::vec2(720.0, 480.0))
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .frame(
                egui::Frame::default()
                    .fill(BG_PANEL)
                    .stroke(Stroke::new(1.0, BORDER))
                    .corner_radius(10.0),
            )
            .show(ctx, |ui| {
                ui.horizontal_top(|ui| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(150.0, 452.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            egui::Frame::default()
                                .fill(BG_WINDOW)
                                .corner_radius(8.0)
                                .inner_margin(egui::Margin::same(10))
                                .show(ui, |ui| {
                                    ui.set_min_size(egui::vec2(150.0, 452.0));
                                    ui.label(RichText::new("SECTIONS").color(DIM).small().strong());
                                    ui.add_space(8.0);
                                    for (tab, label) in [
                                        (SettingsTab::Launchers, "Launchers"),
                                        (SettingsTab::Health, "Health"),
                                        (SettingsTab::Ai, "AI"),
                                        (SettingsTab::Backup, "Backup"),
                                        (SettingsTab::Sync, "Sync"),
                                        (SettingsTab::About, "About"),
                                    ] {
                                        if settings_tab_button(ui, label, self.settings_tab == tab) {
                                            self.settings_tab = tab;
                                        }
                                        ui.add_space(2.0);
                                    }
                                });
                        },
                    );
                    ui.add_space(8.0);

                    let content_size = egui::vec2(ui.available_width(), 452.0);
                    ui.allocate_ui_with_layout(
                        content_size,
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            egui::ScrollArea::vertical()
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    ui.add_space(6.0);
                                    match self.settings_tab {
                                        SettingsTab::Launchers => self.draw_settings_launchers(ui),
                                        SettingsTab::Health => self.draw_settings_health(ui),
                                        SettingsTab::Ai => self.draw_settings_ai(ui),
                                        SettingsTab::Backup => self.draw_settings_backup(ui),
                                        SettingsTab::Sync => self.draw_settings_sync(ui),
                                        SettingsTab::About => {
                                            ui.horizontal(|ui| {
                                                if let Some(t) = self.brand_icon(30) {
                                                    ui.add(egui::Image::new((
                                                        t.id(),
                                                        egui::vec2(30.0, 30.0),
                                                    )));
                                                    ui.add_space(6.0);
                                                }
                                                ui.label(
                                                    RichText::new("Hyperium").color(ACCENT).heading(),
                                                );
                                            });
                                            ui.add_space(4.0);
                                            ui.label(
                                                RichText::new(format!("v{VERSION} · {BUILD_HASH}"))
                                                    .color(DIM),
                                            );
                                            ui.label(
                                                RichText::new(format!("built {BUILD_DATE}"))
                                                    .color(FAINT)
                                                    .small(),
                                            );
                                            ui.add_space(14.0);
                                            ui.label(
                                                RichText::new("An exoskeleton for the supercharged developer.")
                                                    .color(ORANGE)
                                                    .size(15.0)
                                                    .italics(),
                                            );
                                            ui.add_space(4.0);
                                            ui.label(
                                                RichText::new(
                                                    "Keep your flow in one place - let tools absorb the friction.",
                                                )
                                                .color(DIM),
                                            );
                                            ui.add_space(14.0);
                                            ui.label(
                                                RichText::new(
                                                    "Pure Rust · egui · portable-pty + alacritty_terminal",
                                                )
                                                .color(DIM)
                                                .small(),
                                            );
                                        }
                                    }
                                });
                        },
                    );
                });
            });
        self.settings_open = open;
    }

    fn draw_settings_launchers(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("YOUR LAUNCHERS").color(ORANGE).small().strong());
        ui.add_space(8.0);
        if self.commands.is_empty() {
            ui.label(RichText::new("- none yet - add one below").color(FAINT).small());
        } else {
            let mut remove: Option<usize> = None;
            for (i, c) in self.commands.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&c.name).color(FG).monospace());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(8.0);
                        if pill_button(ui, "remove", 0.0, true) {
                            remove = Some(i);
                        }
                        ui.add_space(10.0);
                        ui.label(RichText::new(c.detail()).color(DIM).small().monospace());
                    });
                });
                ui.add_space(4.0);
            }
            if let Some(i) = remove {
                self.commands.remove(i);
                self.save_commands();
            }
        }

        ui.add_space(16.0);
        ui.separator();
        ui.add_space(12.0);

        let field_w = (ui.available_width() - 4.0).min(440.0);
        ui.label(RichText::new("ADD A LAUNCHER").color(ORANGE).small().strong());
        ui.add_space(10.0);

        ui.label(RichText::new("name / keyword").color(DIM).small());
        ui.add(
            egui::TextEdit::singleline(&mut self.new_name)
                .desired_width(field_w)
                .hint_text("e.g. ftp"),
        );
        ui.add_space(8.0);

        ui.label(RichText::new("executable").color(DIM).small());
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.new_path)
                    .desired_width(field_w - 96.0)
                    .hint_text(r"C:\…\app.exe"),
            );
            if pill_button(ui, "browse…", 0.0, true)
                && let Some(file) = rfd::FileDialog::new()
                    .add_filter("programs", &["exe", "bat", "cmd", "lnk"])
                    .pick_file()
            {
                if self.new_name.trim().is_empty() {
                    self.new_name = file
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default();
                }
                self.new_path = file.to_string_lossy().into_owned();
            }
        });
        ui.add_space(8.0);

        ui.label(RichText::new("arguments (optional)").color(DIM).small());
        ui.add(
            egui::TextEdit::singleline(&mut self.new_args)
                .desired_width(field_w)
                .hint_text("passed to the program"),
        );
        ui.add_space(12.0);

        let can_add = !self.new_name.trim().is_empty() && !self.new_path.trim().is_empty();
        if pill_button(ui, "+  Add launcher", 0.0, can_add) {
            self.commands.push(Command {
                name: self.new_name.trim().to_string(),
                kind: launcher::CommandKind::External {
                    path: self.new_path.trim().to_string(),
                    args: self.new_args.trim().to_string(),
                },
            });
            self.save_commands();
            self.new_name.clear();
            self.new_path.clear();
            self.new_args.clear();
        }
    }

    fn draw_onboarding(&mut self, ui: &mut egui::Ui) -> bool {
        let rect = ui.max_rect();
        ui.painter().rect_filled(rect, 0.0, BG_TERMINAL);
        let ctx = ui.ctx().clone();

        let (voice_model, voice_cli) = {
            let vs = self.voice_status();
            (vs.model.is_some(), vs.cli)
        };

        let mut enter = false;
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.add_space(44.0);
            ui.vertical_centered(|ui| {
                let w = 540.0_f32.min((ui.available_width() - 32.0).max(300.0));
                ui.allocate_ui(egui::vec2(w, 0.0), |ui| {
                    ui.set_width(w);

                    ui.vertical_centered(|ui| {
                        if let Some(tex) = self.brand_icon(44) {
                            ui.add(egui::Image::new((tex.id(), egui::vec2(44.0, 44.0))));
                        }
                        ui.add_space(12.0);
                        ui.label(
                            RichText::new("WELCOME TO HYPERIUM").color(ACCENT).size(24.0).strong(),
                        );
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new(
                                "Your dev cockpit. Everything below is optional - skip \
                                 anything now and set it up later in Settings (⚙).",
                            )
                            .color(DIM)
                            .size(13.0),
                        );
                    });
                    ui.add_space(26.0);

                    ui.label(RichText::new("① AI SCRIBE").color(ORANGE).small().strong());
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new(
                            "Ctrl+P drops a thought; Claude files it into the project's \
                             NOTES.md. Paste your Anthropic key to enable it.",
                        )
                        .color(FAINT)
                        .small(),
                    );
                    ui.add_space(6.0);
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut self.ai_key)
                            .hint_text("sk-ant-…  (optional)")
                            .password(true)
                            .desired_width(w - 8.0),
                    );
                    if resp.lost_focus() {
                        save_ai_key(&self.ai_key);
                    }

                    ui.add_space(20.0);
                    ui.separator();
                    ui.add_space(14.0);

                    ui.label(RichText::new("② NOTES SYNC").color(ORANGE).small().strong());
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new(
                            "Push / pull each project's notes to YOUR OWN FTP server, \
                             end-to-end encrypted. Optional - add it anytime.",
                        )
                        .color(FAINT)
                        .small(),
                    );
                    ui.add_space(4.0);
                    egui::CollapsingHeader::new("Set up FTP / FTPS sync")
                        .default_open(false)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("Host").color(DIM).small());
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.ftp_host)
                                        .hint_text("ftp.your-server.com")
                                        .desired_width(240.0),
                                );
                                ui.label(RichText::new("Port").color(DIM).small());
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.ftp_port)
                                        .desired_width(48.0),
                                );
                            });
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("User").color(DIM).small());
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.ftp_user)
                                        .desired_width(150.0),
                                );
                                ui.label(RichText::new("Password").color(DIM).small());
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.ftp_password)
                                        .password(true)
                                        .desired_width(150.0),
                                );
                            });
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("Folder").color(DIM).small());
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.ftp_dir)
                                        .hint_text("/hyperium-notes")
                                        .desired_width(240.0),
                                );
                                ui.checkbox(&mut self.ftp_tls, "TLS (FTPS)");
                            });
                            ui.add_space(6.0);
                            ui.label(
                                RichText::new("Sync passphrase (end-to-end)").color(ACCENT).small(),
                            );
                            ui.add_space(2.0);
                            ui.add(
                                egui::TextEdit::singleline(&mut self.sync_passphrase)
                                    .password(true)
                                    .hint_text("a strong secret - the SAME on all your PCs")
                                    .desired_width(360.0),
                            );
                            ui.label(
                                RichText::new(
                                    "⚠ Not recoverable - lose it and synced notes can't be \
                                     decrypted.",
                                )
                                .color(FAINT)
                                .small(),
                            );
                            ui.add_space(8.0);
                            let busy = self.sync.lock().unwrap_or_else(|e| e.into_inner()).busy;
                            ui.horizontal(|ui| {
                                if pill_button(ui, "Save", 0.0, true) {
                                    ftp::save_config(&config_dir(), &self.ftp_cfg());
                                }
                                if pill_button(ui, "Test connection", 0.0, !busy) {
                                    let c = self.ftp_cfg();
                                    let shared = self.sync.clone();
                                    let ctx2 = ctx.clone();
                                    self.sync.lock().unwrap_or_else(|e| e.into_inner()).busy = true;
                                    std::thread::spawn(move || {
                                        let r = ftp::test_connection(&c);
                                        let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
                                        s.busy = false;
                                        s.message = match r {
                                            Ok(()) => {
                                                "✓ connection OK (folder writable)".to_string()
                                            }
                                            Err(e) => format!("✗ {e}"),
                                        };
                                        drop(s);
                                        ctx2.request_repaint();
                                    });
                                }
                            });
                            let msg = self.sync.lock().unwrap_or_else(|e| e.into_inner()).message.clone();
                            if !msg.is_empty() {
                                ui.add_space(4.0);
                                ui.label(RichText::new(msg).color(DIM).small());
                            }
                        });

                    ui.add_space(20.0);
                    ui.separator();
                    ui.add_space(14.0);

                    ui.label(RichText::new("③ VOICE (OPTIONAL)").color(ORANGE).small().strong());
                    ui.add_space(2.0);
                    if voice_model && voice_cli {
                        ui.label(
                            RichText::new("✓ Local whisper detected - voice braindump is ready.")
                                .color(ACCENT)
                                .small(),
                        );
                    } else {
                        ui.label(
                            RichText::new(
                                "Voice transcription runs 100% locally (whisper). Install it now \
                                 in one click, or later in Settings → AI - text braindump works \
                                 either way.",
                            )
                            .color(FAINT)
                            .small(),
                        );
                        ui.add_space(6.0);
                        self.draw_whisper_installer(ui, &ctx);
                    }

                    ui.add_space(30.0);
                    ui.vertical_centered(|ui| {
                        if pill_button(ui, "Enter Hyperium  →", 180.0, true) {
                            enter = true;
                        }
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new("you can change all of this later in Settings")
                                .color(FAINT)
                                .small(),
                        );
                    });
                    ui.add_space(40.0);
                });
            });
        });

        if enter {
            save_ai_key(&self.ai_key);
            ftp::save_config(&config_dir(), &self.ftp_cfg());
        }
        enter
    }

    fn draw_settings_ai(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("AI - project notes scribe").color(ACCENT).heading());
        ui.add_space(4.0);
        ui.label(
            RichText::new(
                "Ctrl+P opens a braindump: drop a thought and the LLM files it into the \
                 project's NOTES.md (organized, dated). Voice comes next.",
            )
            .color(DIM),
        );
        ui.add_space(14.0);

        ui.label(RichText::new("Anthropic API key").color(ACCENT).small());
        ui.add_space(4.0);
        let resp = ui.add(
            egui::TextEdit::singleline(&mut self.ai_key)
                .hint_text("sk-ant-…")
                .password(true)
                .desired_width(420.0),
        );
        if resp.lost_focus() {
            save_ai_key(&self.ai_key);
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if pill_button(ui, "Save", 0.0, true) {
                save_ai_key(&self.ai_key);
            }
            let state = if !self.ai_key.trim().is_empty() {
                RichText::new("✓ key saved").color(ACCENT).small()
            } else {
                RichText::new("⚠ no key (enter your Anthropic key to enable the scribe)").color(ORANGE).small()
            };
            ui.label(state);
        });
        ui.add_space(8.0);
        ui.label(
            RichText::new(
                "Model: Claude Haiku (fast, ~$1/1M input tokens). Only your notes' text is \
                 sent to the API - nothing else.",
            )
            .color(FAINT)
            .small(),
        );

        ui.add_space(18.0);
        ui.separator();
        ui.add_space(12.0);

        ui.label(RichText::new("Mirage - image / video (kie.ai)").color(ACCENT).heading());
        ui.add_space(4.0);
        ui.label(
            RichText::new(
                "Powers the Ctrl+Space \"Mirage\" tool and the `hyperium gen-image` command. \
                 Get a key at kie.ai.",
            )
            .color(DIM),
        );
        ui.add_space(10.0);
        ui.label(RichText::new("kie.ai API key").color(ACCENT).small());
        ui.add_space(4.0);
        let resp = ui.add(
            egui::TextEdit::singleline(&mut self.kie_key)
                .hint_text("kie.ai API key")
                .password(true)
                .desired_width(420.0),
        );
        if resp.lost_focus() {
            genai::save_key(&config_dir(), &self.kie_key);
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if pill_button(ui, "Save", 0.0, true) {
                genai::save_key(&config_dir(), &self.kie_key);
            }
            let state = if !self.kie_key.trim().is_empty() {
                RichText::new("✓ key saved").color(ACCENT).small()
            } else {
                RichText::new("⚠ no key (enter your kie.ai key to enable Mirage)").color(ORANGE).small()
            };
            ui.label(state);
        });
        ui.add_space(8.0);
        ui.label(
            RichText::new(
                "Stored encrypted, local only. Each generation costs money on kie.ai; a \
                 per-project cap is set inside the Mirage tool.",
            )
            .color(FAINT)
            .small(),
        );

        ui.add_space(18.0);
        ui.separator();
        ui.add_space(12.0);

        ui.label(RichText::new("Voice - local dictation (whisper)").color(ACCENT).heading());
        ui.add_space(4.0);
        let dir = voice::models_dir(&config_dir());
        let status = self.voice_status();
        let mic = status.mic;
        let model = status.model.clone();
        let has_cli = status.cli;
        let has_server = status.server;
        ui.horizontal(|ui| {
            ui.label(RichText::new("Mic:").color(DIM));
            if mic {
                ui.label(RichText::new("✓ detected").color(ACCENT).small());
            } else {
                ui.label(RichText::new("✗ none").color(ORANGE).small());
            }
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new("Model:").color(DIM));
            match &model {
                Some(p) => {
                    let name = p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                    ui.label(RichText::new(format!("✓ {name}")).color(ACCENT).small());
                }
                None => {
                    ui.label(RichText::new("✗ absent").color(ORANGE).small());
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new("whisper-cli :").color(DIM));
            if has_cli {
                ui.label(RichText::new("✓ found").color(ACCENT).small());
            } else {
                ui.label(RichText::new("✗ absent").color(ORANGE).small());
            }
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new("whisper-server :").color(DIM));
            if has_server {
                ui.label(RichText::new("✓ warm (model kept in memory)").color(ACCENT).small());
            } else {
                ui.label(RichText::new("- optional (without it: reloads on every dump)").color(FAINT).small());
            }
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if pill_button(ui, "Open models folder", 0.0, true) {
                let _ = std::fs::create_dir_all(&dir);
                #[cfg(windows)]
                {
                    use std::os::windows::process::CommandExt;
                    let _ = std::process::Command::new("explorer")
                        .arg(&dir)
                        .creation_flags(0x0800_0000)
                        .spawn();
                }
            }
        });

        ui.add_space(12.0);

        let ctx = ui.ctx().clone();
        ui.label(RichText::new("INSTALL WHISPER").color(ORANGE).small().strong());
        ui.add_space(4.0);
        self.draw_whisper_installer(ui, &ctx);

        ui.add_space(6.0);
        ui.label(
            RichText::new(
                "One click fetches the whisper runtime from Hyperium's server and a speech \
                 model from Hugging Face (verified by checksum). A single multilingual model \
                 handles every language automatically. Or drop the files in by hand below.",
            )
            .color(FAINT)
            .small(),
        );
        ui.add_space(6.0);
        ui.label(
            RichText::new(
                "Drop into this folder: (1) a `ggml-*.bin` model - the largest one is used, \
                 large-v3-turbo recommended, from huggingface.co/ggerganov/whisper.cpp ; \
                 (2) `whisper-cli.exe` (+ its DLLs) from a whisper.cpp release (github.com/ggerganov/\
                 whisper.cpp/releases). No build required. Speed tip: also add \
                 `whisper-server.exe` (same release) - Hyperium starts it once, keeps the model \
                 in memory, and each dictation no longer waits for a reload.",
            )
            .color(FAINT)
            .small(),
        );
        ui.label(
            RichText::new(
                "Ctrl+P → the overlay listens to the mic; you speak, Enter, and transcription \
                 + filing happen in the background (everything stays on your machine).",
            )
            .color(FAINT)
            .small(),
        );
    }

    fn draw_settings_backup(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Backup").color(ACCENT).heading());
        ui.add_space(4.0);
        ui.label(
            RichText::new(
                "Ambient snapshots of what git doesn't cover: Hyperium's config (your \
                 projects, launchers, command memo, habits + streaks) and every project's \
                 hyperium-notes (braindumps + NOTES.md). The whisper models and your \
                 project code are left out.",
            )
            .color(FAINT)
            .small(),
        );
        ui.add_space(14.0);

        let cfg = config_dir();
        let dir = backup::configured_dir(&cfg);

        ui.label(RichText::new("DESTINATION").color(ORANGE).small().strong());
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.backup_dir_edit).desired_width(360.0));
            if pill_button(ui, "browse…", 0.0, true)
                && let Some(folder) = rfd::FileDialog::new()
                    .set_title("Backup destination folder")
                    .pick_folder()
            {
                self.backup_dir_edit = folder.to_string_lossy().into_owned();
            }
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if pill_button(ui, "Save destination", 0.0, true) {
                backup::set_dir(&cfg, &self.backup_dir_edit);
                self.backup_dir_edit = backup::configured_dir(&cfg).display().to_string();
            }
            if pill_button(ui, "Reset to default", 0.0, true) {
                backup::set_dir(&cfg, "");
                self.backup_dir_edit = backup::configured_dir(&cfg).display().to_string();
            }
        });
        ui.add_space(4.0);
        ui.label(
            RichText::new(
                "Point it at a synced drive (OneDrive / Dropbox / a network share) to keep \
                 an off-machine copy. Save to apply - new snapshots land there; existing \
                 ones stay where they were.",
            )
            .color(FAINT)
            .small(),
        );

        ui.add_space(14.0);
        ui.separator();
        ui.add_space(12.0);

        let zips = backup::list(&dir);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Snapshots:").color(DIM));
            ui.label(RichText::new(format!("{}", zips.len())).color(ACCENT).small());
            ui.label(
                RichText::new(format!("(keeping the newest {})", backup::KEEP))
                    .color(FAINT)
                    .small(),
            );
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if pill_button(ui, "Backup now", 0.0, true) {
                let project_dirs = self.project_dirs();
                let out = dir.clone();
                std::thread::spawn(move || {
                    let cfg = config_dir();
                    match backup::snapshot(&cfg, &project_dirs, &out, backup::KEEP, &backup::stamp())
                    {
                        Ok(o) => {
                            let name = o
                                .path
                                .file_name()
                                .map(|s| s.to_string_lossy().into_owned())
                                .unwrap_or_default();
                            notify::toast(
                                "Hyperium - backup ✓",
                                &format!("{name}  ·  {} files -> {}", o.files, fmt_mem(o.bytes)),
                            )
                        }
                        Err(e) => notify::toast("Hyperium - backup failed", &e.to_string()),
                    }
                });
            }
            if pill_button(ui, "Open folder", 0.0, true) {
                let _ = std::fs::create_dir_all(&dir);
                #[cfg(windows)]
                {
                    use std::os::windows::process::CommandExt;
                    let _ = std::process::Command::new("explorer")
                        .arg(&dir)
                        .creation_flags(0x0800_0000)
                        .spawn();
                }
            }
        });

        ui.add_space(14.0);
        ui.separator();
        ui.add_space(12.0);

        ui.label(RichText::new("RESTORE A SNAPSHOT").color(ORANGE).small().strong());
        ui.add_space(2.0);
        ui.label(
            RichText::new(
                "Puts a snapshot's files back where they came from (config + each project's \
                 notes). A safety backup of the current state is taken first, and only files \
                 in the snapshot are overwritten. Restart Hyperium afterwards to load it.",
            )
            .color(FAINT)
            .small(),
        );
        ui.add_space(8.0);

        if zips.is_empty() {
            ui.label(RichText::new("No snapshots yet.").color(FAINT).small());
        } else {
            for z in zips.iter().rev() {
                let label = backup_label(z)
                    .unwrap_or_else(|| z.file_name().unwrap_or_default().to_string_lossy().into_owned());
                let size = std::fs::metadata(z).map(|m| m.len()).unwrap_or(0);
                let armed = self.confirm_restore.as_deref() == Some(z.as_path());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(label).color(FG).small());
                    ui.label(RichText::new(format!("· {}", fmt_mem(size))).color(FAINT).small());
                    if armed {
                        if pill_button(ui, "Confirm restore", 0.0, true) {
                            let project_dirs = self.project_dirs();
                            let out = dir.clone();
                            let zc = z.clone();
                            std::thread::spawn(move || {
                                let cfg = config_dir();
                                let _ = backup::snapshot(
                                    &cfg,
                                    &project_dirs,
                                    &out,
                                    backup::KEEP,
                                    &backup::stamp(),
                                );
                                match backup::restore(&cfg, &zc, &project_dirs) {
                                    Ok(n) => notify::toast(
                                        "Hyperium - restore ✓",
                                        &format!("{n} files restored - restart Hyperium to load them"),
                                    ),
                                    Err(e) => {
                                        notify::toast("Hyperium - restore failed", &e.to_string())
                                    }
                                }
                            });
                            self.confirm_restore = None;
                        }
                        if pill_button(ui, "Cancel", 0.0, true) {
                            self.confirm_restore = None;
                        }
                    } else if pill_button(ui, "Restore", 0.0, true) {
                        self.confirm_restore = Some(z.clone());
                    }
                });
                ui.add_space(2.0);
            }
        }

        ui.add_space(12.0);
        ui.label(
            RichText::new(format!("Folder: {}", dir.display()))
                .color(FAINT)
                .small(),
        );
        ui.add_space(4.0);
        ui.label(
            RichText::new(
                "A snapshot is also taken automatically at launch whenever something changed \
                 since the last one.",
            )
            .color(FAINT)
            .small(),
        );
    }

    fn project_dirs(&self) -> Vec<std::path::PathBuf> {
        self.projects
            .iter()
            .map(|p| std::path::PathBuf::from(&p.path))
            .collect()
    }

    fn refresh_sync_badges(&mut self) {
        let (fetched, server) = {
            let s = self.sync.lock().unwrap_or_else(|e| e.into_inner());
            (s.fetched, s.server.clone())
        };
        self.sync_badges.clear();
        if !fetched {
            return;
        }
        let cfg = config_dir();
        let last = sync::load_synced(&cfg);
        for p in &self.projects {
            let notes = notes::dir(&p.path);
            if !notes.is_dir() {
                continue;
            }
            let local = sync::dir_hash(&notes);
            let badge = sync::classify(
                &local,
                server.get(&p.name).map(String::as_str),
                last.get(&p.name).map(String::as_str),
            );
            if badge == sync::Badge::Synced && last.get(&p.name).map(String::as_str) != Some(&local) {
                sync::save_synced(&cfg, &p.name, &local);
            }
            self.sync_badges.insert(p.name.clone(), badge);
        }
    }

    fn refresh_wiki_present(&mut self) {
        self.wiki_present.clear();
        for p in &self.projects {
            self.wiki_present.insert(p.name.clone(), wiki::has_wiki(&p.path));
        }
    }

    fn start_update_check(&self, ctx: egui::Context) {
        let cfg = config_dir();
        let shared = self.update.clone();
        shared.lock().unwrap_or_else(|e| e.into_inner()).busy = true;
        std::thread::spawn(move || {
            let res = sync::fetch_app_manifest(&cfg);
            let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
            s.busy = false;
            s.checked = true;
            match res {
                Ok(Some(rel)) if update::is_update(&rel) => {
                    s.message = format!("update available: v{}", rel.version);
                    s.available = Some(rel);
                }
                Ok(_) => {
                    s.message = "you're on the latest published build".into();
                    s.available = None;
                }
                Err(e) => s.message = e,
            }
            drop(s);
            ctx.request_repaint();
        });
    }

    fn start_install(&self, _ctx: egui::Context) {
        let cfg = config_dir();
        let rel = { self.update.lock().unwrap_or_else(|e| e.into_inner()).available.clone() };
        let Some(rel) = rel else { return };
        let shared = self.update.clone();
        shared.lock().unwrap_or_else(|e| e.into_inner()).busy = true;
        std::thread::spawn(move || {
            let result = (|| -> Result<(), String> {
                if rel.sha256.trim().is_empty() {
                    return Err("manifest has no sha256 - refusing to install".into());
                }
                let zip = sync::download_app(&cfg, &rel)?;
                let got = sync::sha256_bytes(&zip);
                if !got.eq_ignore_ascii_case(rel.sha256.trim()) {
                    return Err(format!(
                        "checksum mismatch (expected {}, got {got}) - aborting",
                        rel.sha256
                    ));
                }
                let exe = update::exe_from_zip(&zip).map_err(|e| e.to_string())?;
                update::apply_and_relaunch(&exe)
            })();
            match result {
                Ok(()) => std::process::exit(0),
                Err(e) => {
                    let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
                    s.busy = false;
                    s.message = format!("update failed: {e}");
                    drop(s);
                    _ctx.request_repaint();
                }
            }
        });
    }

    fn draw_settings_sync(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let cfg = config_dir();

        ui.label(RichText::new("Sync").color(ACCENT).heading());
        ui.add_space(4.0);
        ui.label(
            RichText::new(
                "Push / pull each project's hyperium-notes to YOUR OWN FTP server - no script \
                 to install. Every payload is end-to-end encrypted with your sync passphrase \
                 before upload, so the server only ever holds ciphertext. No auto-sync, no \
                 merge: Hyperium compares a content hash per project and you pick the \
                 direction; a Pull takes a safety snapshot first.",
            )
            .color(FAINT)
            .small(),
        );
        ui.add_space(14.0);

        let (busy, message, server, fetched) = {
            let s = self.sync.lock().unwrap_or_else(|e| e.into_inner());
            (s.busy, s.message.clone(), s.server.clone(), s.fetched)
        };

        ui.label(RichText::new("NOTES SYNC · YOUR FTP").color(ORANGE).small().strong());
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Host").color(DIM).small());
            ui.add(
                egui::TextEdit::singleline(&mut self.ftp_host)
                    .hint_text("ftp.your-server.com")
                    .desired_width(260.0),
            );
            ui.label(RichText::new("Port").color(DIM).small());
            ui.add(egui::TextEdit::singleline(&mut self.ftp_port).desired_width(48.0));
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("User").color(DIM).small());
            ui.add(egui::TextEdit::singleline(&mut self.ftp_user).desired_width(170.0));
            ui.label(RichText::new("Password").color(DIM).small());
            ui.add(
                egui::TextEdit::singleline(&mut self.ftp_password)
                    .password(true)
                    .desired_width(170.0),
            );
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Folder").color(DIM).small());
            ui.add(
                egui::TextEdit::singleline(&mut self.ftp_dir)
                    .hint_text("/hyperium-notes")
                    .desired_width(260.0),
            );
            ui.checkbox(&mut self.ftp_tls, "TLS (FTPS)");
        });
        if !self.ftp_tls {
            ui.label(
                RichText::new("⚠ TLS off: password + notes travel in clear. Prefer FTPS.")
                    .color(ORANGE)
                    .small(),
            );
        }
        ui.add_space(6.0);
        ui.label(RichText::new("Sync passphrase (end-to-end encryption)").color(ACCENT).small());
        ui.add_space(2.0);
        ui.add(
            egui::TextEdit::singleline(&mut self.sync_passphrase)
                .password(true)
                .hint_text("a strong secret - the SAME on all your PCs")
                .desired_width(420.0),
        );
        ui.label(
            RichText::new(
                "Notes are encrypted with this before upload; the server never sees it. \
                 ⚠ It is NOT recoverable - lose it and your synced notes can't be decrypted.",
            )
            .color(FAINT)
            .small(),
        );
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if pill_button(ui, "Save", 0.0, true) {
                ftp::save_config(&cfg, &self.ftp_cfg());
            }
            if pill_button(ui, "Test connection", 0.0, !busy) {
                let c = self.ftp_cfg();
                let shared = self.sync.clone();
                let ctx2 = ctx.clone();
                self.sync.lock().unwrap_or_else(|e| e.into_inner()).busy = true;
                std::thread::spawn(move || {
                    let r = ftp::test_connection(&c);
                    let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
                    s.busy = false;
                    s.message = match r {
                        Ok(()) => "✓ connection OK (folder writable)".to_string(),
                        Err(e) => format!("✗ {e}"),
                    };
                    drop(s);
                    ctx2.request_repaint();
                });
            }
        });

        ui.add_space(14.0);
        ui.separator();
        ui.add_space(12.0);

        ui.label(RichText::new("APP UPDATE").color(ORANGE).small().strong());
        ui.add_space(4.0);
        ui.label(
            RichText::new(format!(
                "You're running v{VERSION} ({BUILD_HASH}). Hyperium checks its official server \
                 for a newer build at launch and offers a one-click \"Install & restart\". Every \
                 update is verified (SHA-256 + code signature) before it replaces this exe.",
            ))
            .color(FAINT)
            .small(),
        );
        ui.add_space(6.0);

        let (upd_busy, upd_msg, upd_avail) = {
            let s = self.update.lock().unwrap_or_else(|e| e.into_inner());
            (s.busy, s.message.clone(), s.available.clone())
        };
        ui.horizontal(|ui| {
            if pill_button(ui, "Check for updates", 0.0, !upd_busy) {
                self.start_update_check(ctx.clone());
            }
            if upd_avail.is_some() && pill_button(ui, "Install & restart", 0.0, !upd_busy) {
                self.start_install(ctx.clone());
            }
        });
        ui.add_space(4.0);
        if upd_busy {
            ui.label(RichText::new("working…").color(ORANGE).small());
        } else if let Some(rel) = &upd_avail {
            let mb = rel.size as f64 / 1_048_576.0;
            let when = if rel.date.is_empty() { String::new() } else { format!(" · built {}", rel.date) };
            let hash = if rel.hash.is_empty() { String::new() } else { format!(" ({})", rel.hash) };
            ui.label(
                RichText::new(format!(
                    "● update available: v{}{} · {:.1} MB{}",
                    rel.version, hash, mb, when
                ))
                .color(ORANGE)
                .small(),
            );
        } else if !upd_msg.is_empty() {
            ui.label(RichText::new(upd_msg).color(DIM).small());
        }

        ui.add_space(14.0);
        ui.separator();
        ui.add_space(12.0);

        ui.horizontal(|ui| {
            if pill_button(ui, "Check status", 0.0, !busy) {
                let c = self.ftp_cfg();
                let shared = self.sync.clone();
                let ctx2 = ctx.clone();
                self.sync.lock().unwrap_or_else(|e| e.into_inner()).busy = true;
                std::thread::spawn(move || {
                    let res = ftp::read_manifest(&c);
                    let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
                    s.busy = false;
                    match res {
                        Ok(map) => {
                            s.message = format!("{} project(s) on the server", map.len());
                            s.server = map;
                            s.fetched = true;
                        }
                        Err(e) => s.message = e,
                    }
                    drop(s);
                    ctx2.request_repaint();
                });
            }
            if busy {
                ui.label(RichText::new("working…").color(ORANGE).small());
            } else if !message.is_empty() {
                ui.label(RichText::new(message).color(DIM).small());
            }
        });

        ui.add_space(14.0);
        ui.separator();
        ui.add_space(12.0);

        ui.label(RichText::new("TEMPLATES").color(ORANGE).small().strong());
        ui.add_space(2.0);
        ui.label(
            RichText::new(
                "The shared _templates folder (reusable scaffolds the assistant reads, like \
                 bootstrap-wiki.md). Push it from the PC that has it, pull it on the others. \
                 A pull keeps a timestamped copy of the current folder first.",
            )
            .color(FAINT)
            .small(),
        );
        ui.add_space(4.0);

        let tdir = templates_dir();
        let has_local = tdir.is_dir();
        let tlocal = if has_local { sync::dir_hash(&tdir) } else { String::new() };
        let tserver = server.get(TEMPLATES_KEY);
        let tarmed = self.confirm_pull.as_deref() == Some(TEMPLATES_KEY);

        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.templates_dir_edit)
                    .hint_text(std::path::PathBuf::from(load_projects_root()).join("_templates").display().to_string())
                    .desired_width(300.0),
            );
            if pill_button(ui, "browse…", 0.0, true)
                && let Some(folder) = rfd::FileDialog::new()
                    .set_title("The _templates folder on this PC")
                    .pick_folder()
            {
                self.templates_dir_edit = folder.to_string_lossy().into_owned();
                save_templates_dir_override(&self.templates_dir_edit);
            }
            if pill_button(ui, "Save", 0.0, true) {
                save_templates_dir_override(&self.templates_dir_edit);
            }
            if pill_button(ui, "Reset", 0.0, true) {
                self.templates_dir_edit.clear();
                save_templates_dir_override("");
            }
        });
        ui.label(RichText::new(format!("→ {}", tdir.display())).color(DIM).small());
        ui.horizontal(|ui| {
            if !fetched {
                ui.label(RichText::new("run Check status to compare").color(FAINT).small());
            } else if !has_local && tserver.is_none() {
                ui.label(RichText::new("○ nothing here or on the server").color(DIM).small());
            } else if !has_local {
                ui.label(RichText::new("↓ on the server, not here").color(ACCENT).small());
            } else if tserver.is_none() {
                ui.label(RichText::new("○ local only (never pushed)").color(DIM).small());
            } else if tserver == Some(&tlocal) {
                ui.label(RichText::new("✓ in sync").color(ACCENT).small());
            } else {
                ui.label(RichText::new("● differs").color(ORANGE).small());
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if tserver.is_some() {
                    if tarmed {
                        if pill_button(ui, "Confirm pull", 0.0, !busy) {
                            self.spawn_pull_templates(&ctx);
                            self.confirm_pull = None;
                        }
                        if pill_button(ui, "Cancel", 0.0, true) {
                            self.confirm_pull = None;
                        }
                    } else if pill_button(ui, "Pull", 0.0, !busy) {
                        self.confirm_pull = Some(TEMPLATES_KEY.to_string());
                    }
                }
                if !tarmed && has_local && pill_button(ui, "Push", 0.0, !busy) {
                    self.spawn_push(&ctx, TEMPLATES_KEY.to_string(), tdir.clone(), tlocal.clone());
                }
            });
        });

        ui.add_space(14.0);
        ui.separator();
        ui.add_space(12.0);

        let projects: Vec<(String, std::path::PathBuf)> = self
            .projects
            .iter()
            .map(|p| (p.name.clone(), notes::dir(&p.path)))
            .filter(|(_, notes)| notes.is_dir())
            .collect();
        let project_dirs = self.project_dirs();

        if !fetched {
            ui.label(
                RichText::new("Run \"Check status\" to compare your projects with the server.")
                    .color(FAINT)
                    .small(),
            );
        } else if projects.is_empty() {
            ui.label(
                RichText::new("No local project has notes yet (nothing to sync).")
                    .color(FAINT)
                    .small(),
            );
        }

        for (name, notes) in &projects {
            let local = sync::dir_hash(notes);
            let server_hash = server.get(name);
            let armed = self.confirm_pull.as_deref() == Some(name.as_str());

            ui.horizontal(|ui| {
                ui.label(RichText::new(name).color(FG));
                match server_hash {
                    Some(h) if *h == local => {
                        ui.label(RichText::new("✓ in sync").color(ACCENT).small());
                    }
                    Some(_) => {
                        ui.label(RichText::new("● differs").color(ORANGE).small());
                    }
                    None => {
                        ui.label(RichText::new("○ local only").color(DIM).small());
                    }
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if server_hash.is_some() {
                        if armed {
                            if pill_button(ui, "Confirm pull", 0.0, !busy) {
                                self.spawn_pull(&ctx, name.clone(), notes.clone(), project_dirs.clone());
                                self.confirm_pull = None;
                            }
                            if pill_button(ui, "Cancel", 0.0, true) {
                                self.confirm_pull = None;
                            }
                        } else if pill_button(ui, "Pull", 0.0, !busy) {
                            self.confirm_pull = Some(name.clone());
                        }
                    }
                    if !armed && pill_button(ui, "Push", 0.0, !busy) {
                        self.spawn_push(&ctx, name.clone(), notes.clone(), local.clone());
                    }
                });
            });
            ui.add_space(2.0);
        }

        if fetched {
            let local_names: std::collections::HashSet<&str> =
                projects.iter().map(|(n, _)| n.as_str()).collect();
            let mut server_only: Vec<&String> = server
                .keys()
                .filter(|k| k.as_str() != TEMPLATES_KEY && !local_names.contains(k.as_str()))
                .collect();
            server_only.sort();
            if !server_only.is_empty() {
                ui.add_space(10.0);
                ui.label(RichText::new("ON THE SERVER ONLY").color(ORANGE).small().strong());
                ui.add_space(2.0);
                ui.label(
                    RichText::new(
                        "These have notes on the server but no matching local project. \
                         Pull them into a folder to start working here.",
                    )
                    .color(FAINT)
                    .small(),
                );
                ui.add_space(4.0);
                for name in server_only {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(name).color(FG).small());
                        if pill_button(ui, "Pull to folder…", 0.0, !busy)
                            && let Some(folder) = rfd::FileDialog::new()
                                .set_title(format!("Pull \"{name}\" notes into…"))
                                .pick_folder()
                        {
                            let dest = notes::dir(&folder.to_string_lossy());
                            self.spawn_pull(&ctx, name.clone(), dest, Vec::new());
                        }
                    });
                    ui.add_space(2.0);
                }
            }
        }
    }

    fn ftp_cfg(&self) -> ftp::FtpConfig {
        ftp::FtpConfig {
            host: self.ftp_host.trim().to_string(),
            port: self.ftp_port.trim().parse().unwrap_or(21),
            user: self.ftp_user.trim().to_string(),
            password: self.ftp_password.clone(),
            dir: self.ftp_dir.trim().to_string(),
            tls: self.ftp_tls,
            passphrase: self.sync_passphrase.clone(),
        }
    }

    fn spawn_push(
        &self,
        ctx: &egui::Context,
        name: String,
        notes: std::path::PathBuf,
        hash: String,
    ) {
        let cfg = self.ftp_cfg();
        let shared = self.sync.clone();
        let ctx = ctx.clone();
        self.sync.lock().unwrap_or_else(|e| e.into_inner()).busy = true;
        std::thread::spawn(move || {
            let result = match sync::zip_dir(&notes) {
                Ok(bytes) => ftp::push(&cfg, &name, &hash, &bytes),
                Err(e) => Err(format!("zip failed: {e}")),
            };
            let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
            s.busy = false;
            match result {
                Ok(()) => {
                    sync::save_synced(&config_dir(), &name, &hash);
                    s.server.insert(name.clone(), hash);
                    s.message = format!("pushed \"{name}\"");
                    notify::toast("Hyperium - sync ✓", &format!("pushed \"{name}\" to the server"));
                }
                Err(e) => {
                    s.message = e.clone();
                    notify::toast("Hyperium - push failed", &e);
                }
            }
            drop(s);
            ctx.request_repaint();
        });
    }

    fn spawn_pull(
        &self,
        ctx: &egui::Context,
        name: String,
        notes: std::path::PathBuf,
        project_dirs: Vec<std::path::PathBuf>,
    ) {
        let ftp_cfg = self.ftp_cfg();
        let shared = self.sync.clone();
        let ctx = ctx.clone();
        self.sync.lock().unwrap_or_else(|e| e.into_inner()).busy = true;
        std::thread::spawn(move || {
            let cfg = config_dir();
            if !project_dirs.is_empty() {
                let out = backup::configured_dir(&cfg);
                let _ = backup::snapshot(&cfg, &project_dirs, &out, backup::KEEP, &backup::stamp());
            }
            let result = match ftp::pull(&ftp_cfg, &name) {
                Ok(bytes) => {
                    std::fs::create_dir_all(&notes)
                        .map_err(|e| format!("mkdir failed: {e}"))
                        .and_then(|()| {
                            sync::unzip_into(&notes, &bytes).map_err(|e| format!("extract failed: {e}"))
                        })
                }
                Err(e) => Err(e),
            };
            let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
            s.busy = false;
            match result {
                Ok(n) => {
                    sync::save_synced(&config_dir(), &name, &sync::dir_hash(&notes));
                    s.message = format!("pulled \"{name}\" ({n} files)");
                    notify::toast(
                        "Hyperium - sync ✓",
                        &format!("pulled \"{name}\" - {n} files (a safety snapshot was taken)"),
                    );
                }
                Err(e) => {
                    s.message = e.clone();
                    notify::toast("Hyperium - pull failed", &e);
                }
            }
            drop(s);
            ctx.request_repaint();
        });
    }

    fn spawn_pull_templates(&self, ctx: &egui::Context) {
        let ftp_cfg = self.ftp_cfg();
        let shared = self.sync.clone();
        let ctx = ctx.clone();
        let dir = templates_dir();
        self.sync.lock().unwrap_or_else(|e| e.into_inner()).busy = true;
        std::thread::spawn(move || {
            if dir.is_dir()
                && let Ok(bytes) = sync::zip_dir(&dir)
            {
                let out = backup::configured_dir(&config_dir());
                let _ = std::fs::create_dir_all(&out);
                let _ = std::fs::write(out.join(format!("templates-backup-{}.zip", backup::stamp())), bytes);
            }
            let result = match ftp::pull(&ftp_cfg, TEMPLATES_KEY) {
                Ok(bytes) => std::fs::create_dir_all(&dir)
                    .map_err(|e| format!("mkdir failed: {e}"))
                    .and_then(|()| {
                        sync::unzip_into(&dir, &bytes).map_err(|e| format!("extract failed: {e}"))
                    }),
                Err(e) => Err(e),
            };
            let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
            s.busy = false;
            match result {
                Ok(n) => {
                    sync::save_synced(&config_dir(), TEMPLATES_KEY, &sync::dir_hash(&dir));
                    s.server.insert(TEMPLATES_KEY.to_string(), sync::dir_hash(&dir));
                    s.message = format!("pulled _templates ({n} files)");
                    notify::toast(
                        "Hyperium - sync ✓",
                        &format!("pulled _templates - {n} files (a safety copy was kept)"),
                    );
                }
                Err(e) => {
                    s.message = e.clone();
                    notify::toast("Hyperium - pull failed", &e);
                }
            }
            drop(s);
            ctx.request_repaint();
        });
    }

    fn ensure_whisper_models(&mut self, ctx: &egui::Context) {
        {
            let s = self.whisper.lock().unwrap_or_else(|e| e.into_inner());
            if s.busy || s.manifest_tried {
                return;
            }
        }
        self.whisper.lock().unwrap_or_else(|e| e.into_inner()).manifest_tried = true;
        let shared = self.whisper.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            if let Ok(m) = sync::whisper_manifest(&config_dir()) {
                shared.lock().unwrap_or_else(|e| e.into_inner()).models = m.models;
                ctx.request_repaint();
            }
        });
    }

    fn draw_whisper_installer(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let (wbusy, wmsg, wphase, wdone, wtotal) = {
            let s = self.whisper.lock().unwrap_or_else(|e| e.into_inner());
            (s.busy, s.message.clone(), s.phase.clone(), s.downloaded, s.total)
        };
        self.ensure_whisper_models(ctx);
        let models = { self.whisper.lock().unwrap_or_else(|e| e.into_inner()).models.clone() };
        if !models.is_empty() {
            if self.whisper_model_sel.is_empty()
                || !models.iter().any(|m| m.id == self.whisper_model_sel)
            {
                self.whisper_model_sel = models[0].id.clone();
            }
            ui.label(RichText::new("Model (multilingual - covers every language):").color(DIM).small());
            ui.add_space(2.0);
            for m in &models {
                let label = if m.label.is_empty() {
                    format!("{} ({})", m.name, human_bytes(m.size))
                } else {
                    m.label.clone()
                };
                ui.radio_value(&mut self.whisper_model_sel, m.id.clone(), label);
            }
            ui.add_space(6.0);
        } else if !wbusy {
            ui.label(RichText::new("(loading the model list from the server…)").color(FAINT).small());
            ui.add_space(4.0);
        }
        ui.horizontal(|ui| {
            if pill_button(ui, "Install Whisper", 0.0, !wbusy) {
                self.spawn_install_whisper(ctx, self.whisper_model_sel.clone());
            }
        });
        if wbusy && !wphase.is_empty() {
            ui.add_space(4.0);
            let frac =
                if wtotal > 0 { (wdone as f32 / wtotal as f32).clamp(0.0, 1.0) } else { 0.0 };
            let txt = if wtotal > 0 {
                format!("{wphase} : {} / {}", human_bytes(wdone), human_bytes(wtotal))
            } else {
                format!("{wphase} : {}", human_bytes(wdone))
            };
            let mut bar = egui::ProgressBar::new(frac).text(txt).desired_width(420.0);
            if wtotal == 0 {
                bar = bar.animate(true);
            }
            ui.add(bar);
        }
        if !wmsg.is_empty() {
            ui.add_space(2.0);
            ui.label(RichText::new(&wmsg).color(if wbusy { DIM } else { ACCENT }).small());
        }
    }

    fn spawn_install_whisper(&mut self, ctx: &egui::Context, model_id: String) {
        let shared = self.whisper.clone();
        let ctx = ctx.clone();
        {
            let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
            s.busy = true;
            s.message = "fetching the manifest…".into();
            s.phase = String::new();
            s.downloaded = 0;
            s.total = 0;
        }
        std::thread::spawn(move || {
            let cfg = config_dir();
            let dir = voice::models_dir(&cfg);
            let _ = std::fs::create_dir_all(&dir);
            let download = |url: &str, dest: &std::path::Path, label: &str, total_hint: u64| {
                {
                    let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
                    s.phase = label.to_string();
                    s.downloaded = 0;
                    s.total = total_hint;
                }
                let sh = shared.clone();
                let cx = ctx.clone();
                let mut last = Instant::now();
                sync::download_to_file(url, dest, |d, t| {
                    let mut s = sh.lock().unwrap_or_else(|e| e.into_inner());
                    s.downloaded = d;
                    if t > 0 {
                        s.total = t;
                    }
                    drop(s);
                    if last.elapsed() >= Duration::from_millis(200) {
                        last = Instant::now();
                        cx.request_repaint();
                    }
                })
            };
            let result = (|| -> Result<String, String> {
                let manifest = sync::whisper_manifest(&cfg)?;
                if manifest.bin.file.is_empty() && manifest.models.is_empty() {
                    return Err("nothing published on the server yet".into());
                }
                let mut got_bin = false;
                if !manifest.bin.file.is_empty() {
                    let zip_path = dir.join(".whisper-bin.zip");
                    let bin_url = sync::whisper_bin_url(&cfg, &manifest.bin.file);
                    download(&bin_url, &zip_path, "binaries", manifest.bin.size)?;
                    if !manifest.bin.sha256.is_empty() {
                        let got = sync::sha256_file(&zip_path)
                            .map_err(|e| format!("hashing binaries: {e}"))?;
                        if !got.eq_ignore_ascii_case(manifest.bin.sha256.trim()) {
                            let _ = std::fs::remove_file(&zip_path);
                            return Err("binaries checksum mismatch - aborting".into());
                        }
                    }
                    let bytes = std::fs::read(&zip_path)
                        .map_err(|e| format!("re-reading the binaries zip: {e}"))?;
                    sync::unzip_into(&dir, &bytes)
                        .map_err(|e| format!("extracting the binaries: {e}"))?;
                    let _ = std::fs::remove_file(&zip_path);
                    got_bin = true;
                }
                let mut got_model = false;
                let picked = manifest
                    .models
                    .iter()
                    .find(|m| !model_id.is_empty() && m.id == model_id)
                    .or_else(|| manifest.models.first());
                if let Some(model) = picked
                    && !model.url.is_empty()
                    && !model.name.is_empty()
                {
                    let model_path = dir.join(&model.name);
                    download(&model.url, &model_path, "model", model.size)?;
                    if !model.sha256.is_empty() {
                        let got = sync::sha256_file(&model_path)
                            .map_err(|e| format!("hashing model: {e}"))?;
                        if !got.eq_ignore_ascii_case(model.sha256.trim()) {
                            let _ = std::fs::remove_file(&model_path);
                            return Err("model checksum mismatch - aborting".into());
                        }
                    }
                    got_model = true;
                }
                Ok(match (got_bin, got_model) {
                    (true, true) => "Whisper installed (binaries + model)".to_string(),
                    (true, false) => "binaries installed (no model offered)".to_string(),
                    (false, true) => "model installed (no binaries offered)".to_string(),
                    _ => "nothing to install".to_string(),
                })
            })();
            let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
            s.busy = false;
            s.phase = String::new();
            match result {
                Ok(m) => {
                    s.message = m.clone();
                    drop(s);
                    notify::toast("Hyperium - whisper", &m);
                }
                Err(e) => {
                    s.message = e.clone();
                    drop(s);
                    notify::toast("Hyperium - whisper", &format!("failed: {e}"));
                }
            }
            ctx.request_repaint();
        });
    }

    fn draw_settings_health(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;

        ui.label(RichText::new("YOUR HABITS").color(ORANGE).small().strong());
        ui.add_space(8.0);
        if self.coach.habits.is_empty() {
            ui.label(RichText::new("- none - add one below").color(FAINT).small());
        } else {
            let mut remove: Option<usize> = None;
            for (i, h) in self.coach.habits.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{} {}", h.icon, h.label)).color(FG));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(8.0);
                        if pill_button(ui, "remove", 0.0, true) {
                            remove = Some(i);
                        }
                    });
                });
                ui.horizontal(|ui| {
                    ui.label(RichText::new("target/day").color(DIM).small());
                    changed |= ui
                        .add(egui::DragValue::new(&mut h.daily_target).range(1..=100_000))
                        .changed();
                    ui.add_space(10.0);
                    ui.label(RichText::new("chunk").color(DIM).small());
                    changed |=
                        ui.add(egui::DragValue::new(&mut h.min_chunk).range(1..=10_000)).changed();
                    ui.label(RichText::new("-").color(DIM).small());
                    changed |=
                        ui.add(egui::DragValue::new(&mut h.max_chunk).range(1..=10_000)).changed();
                    ui.label(RichText::new(&h.noun).color(FAINT).small());
                });
                if h.max_chunk < h.min_chunk {
                    h.max_chunk = h.min_chunk;
                }
                ui.add_space(10.0);
            }
            if let Some(i) = remove {
                self.coach.habits.remove(i);
                changed = true;
            }
        }

        if !self.coach.habits.is_empty() {
            ui.add_space(6.0);
            ui.separator();
            ui.add_space(12.0);
            ui.label(RichText::new("STREAKS").color(ORANGE).small().strong());
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                for h in &self.coach.habits {
                    let s = self.coach.progress_of(&h.id).streak;
                    ui.label(
                        RichText::new(format!("{} 🔥{}", h.icon, s))
                            .color(if s > 0 { ORANGE } else { DIM }),
                    );
                    ui.add_space(12.0);
                }
            });
            ui.add_space(8.0);
            if !self.confirm_reset_streaks {
                if pill_button(ui, "Reset streaks", 0.0, true) {
                    self.confirm_reset_streaks = true;
                }
            } else {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Reset every streak to 0?").color(ORANGE).small());
                    ui.add_space(8.0);
                    if pill_button(ui, "Confirm", 0.0, true) {
                        self.coach.reset_streaks();
                        let cfg = config_dir();
                        self.coach.save_state(&cfg);
                        self.coach.save_history(&cfg);
                        self.confirm_reset_streaks = false;
                    }
                    ui.add_space(4.0);
                    if pill_button(ui, "Cancel", 0.0, true) {
                        self.confirm_reset_streaks = false;
                    }
                });
            }
            ui.add_space(6.0);
            ui.label(
                RichText::new(
                    "Daily totals are logged to coach_history.tsv - see the \"Health log\" tool \
                     (Ctrl+Space) for the activity graph.",
                )
                .color(FAINT)
                .small(),
            );
        }

        ui.add_space(6.0);
        ui.separator();
        ui.add_space(12.0);

        ui.label(RichText::new("NUDGE CADENCE").color(ORANGE).small().strong());
        ui.add_space(8.0);
        let mut min_m = (self.coach.interval_min_secs / 60).max(1);
        let mut max_m = (self.coach.interval_max_secs / 60).max(1);
        ui.horizontal(|ui| {
            ui.label(RichText::new("every").color(DIM).small());
            let a = ui.add(egui::DragValue::new(&mut min_m).range(1..=240)).changed();
            ui.label(RichText::new("to").color(DIM).small());
            let b = ui.add(egui::DragValue::new(&mut max_m).range(1..=240)).changed();
            ui.label(RichText::new("min").color(DIM).small());
            if a || b {
                if max_m < min_m {
                    max_m = min_m;
                }
                self.coach.interval_min_secs = min_m * 60;
                self.coach.interval_max_secs = max_m * 60;
            }
        });
        ui.add_space(10.0);
        if pill_button(ui, "▶  Test nudge now", 0.0, true)
            && let Some(n) = self.coach.next_nudge()
        {
            notify::toast("Hyperium - move", &n.message());
            self.coach_nudge = Some(n);
        }

        ui.add_space(16.0);
        ui.separator();
        ui.add_space(12.0);

        let field_w = (ui.available_width() - 4.0).min(440.0);
        ui.label(RichText::new("ADD A HABIT").color(ORANGE).small().strong());
        ui.add_space(10.0);
        ui.label(RichText::new("name").color(DIM).small());
        ui.add(
            egui::TextEdit::singleline(&mut self.new_habit_label)
                .desired_width(field_w)
                .hint_text("e.g. Pull-ups"),
        );
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("icon").color(DIM).small());
            ui.add(egui::TextEdit::singleline(&mut self.new_habit_icon).desired_width(46.0).hint_text("🎯"));
            ui.add_space(8.0);
            ui.label(RichText::new("verb").color(DIM).small());
            ui.add(egui::TextEdit::singleline(&mut self.new_habit_verb).desired_width(76.0).hint_text("Do"));
            ui.add_space(8.0);
            ui.label(RichText::new("noun").color(DIM).small());
            ui.add(
                egui::TextEdit::singleline(&mut self.new_habit_noun)
                    .desired_width(130.0)
                    .hint_text("pull-ups"),
            );
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("target/day").color(DIM).small());
            ui.add(egui::DragValue::new(&mut self.new_habit_target).range(1..=100_000));
            ui.add_space(10.0);
            ui.label(RichText::new("chunk").color(DIM).small());
            ui.add(egui::DragValue::new(&mut self.new_habit_min).range(1..=10_000));
            ui.label(RichText::new("-").color(DIM).small());
            ui.add(egui::DragValue::new(&mut self.new_habit_max).range(1..=10_000));
        });
        ui.add_space(12.0);
        let can_add = !self.new_habit_label.trim().is_empty();
        if pill_button(ui, "+  Add habit", 0.0, can_add) {
            let label = self.new_habit_label.trim().to_string();
            let id = make_habit_id(&label, &self.coach.habits);
            let noun = if self.new_habit_noun.trim().is_empty() {
                label.to_lowercase()
            } else {
                self.new_habit_noun.trim().to_string()
            };
            let verb = if self.new_habit_verb.trim().is_empty() {
                "Do".to_string()
            } else {
                self.new_habit_verb.trim().to_string()
            };
            let icon = if self.new_habit_icon.trim().is_empty() {
                "🎯".to_string()
            } else {
                self.new_habit_icon.trim().to_string()
            };
            self.coach.habits.push(coach::Habit {
                id,
                label,
                noun,
                verb,
                icon,
                daily_target: self.new_habit_target,
                min_chunk: self.new_habit_min,
                max_chunk: self.new_habit_max.max(self.new_habit_min),
            });
            changed = true;
            self.new_habit_label.clear();
            self.new_habit_noun.clear();
            self.new_habit_verb.clear();
            self.new_habit_icon.clear();
            self.new_habit_target = 20;
            self.new_habit_min = 2;
            self.new_habit_max = 5;
        }

        if changed {
            self.coach.save_habits(&config_dir());
        }
    }
}

fn make_habit_id(label: &str, existing: &[coach::Habit]) -> String {
    let base: String = label
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    let base = base.trim_matches('-').to_string();
    let base = if base.is_empty() { "habit".to_string() } else { base };
    if !existing.iter().any(|h| h.id == base) {
        return base;
    }
    for n in 2..1000 {
        let cand = format!("{base}-{n}");
        if !existing.iter().any(|h| h.id == cand) {
            return cand;
        }
    }
    base
}

fn settings_tab_button(ui: &mut egui::Ui, label: &str, selected: bool) -> bool {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 30.0), egui::Sense::click());
    let hov = resp.hovered();
    let p = ui.painter();
    if selected || hov {
        p.rect_filled(rect, 6.0, if selected { BG_SELECTED } else { BG_HOVER });
    }
    if selected {
        let bar = egui::Rect::from_min_max(rect.left_top(), egui::pos2(rect.left() + 3.0, rect.bottom()));
        p.rect_filled(bar, CornerRadius::same(2), ACCENT);
    }
    p.text(
        egui::pos2(rect.left() + 12.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        FontId::new(13.5, FontFamily::Proportional),
        if selected { ACCENT } else { FG },
    );
    resp.clicked()
}

const MONO_BOLD: &str = "mono_bold";
const MONO_ITALIC: &str = "mono_italic";
const MONO_BOLD_ITALIC: &str = "mono_bold_italic";

fn install_fonts(ctx: &egui::Context) {
    use std::sync::Arc;
    let mut fonts = egui::FontDefinitions::default();

    let load = |fonts: &mut egui::FontDefinitions, name: &str, path: &str| -> bool {
        match std::fs::read(path) {
            Ok(bytes) => {
                fonts
                    .font_data
                    .insert(name.to_string(), Arc::new(egui::FontData::from_owned(bytes)));
                true
            }
            Err(_) => false,
        }
    };

    fonts
        .font_data
        .insert("ui".to_string(), Arc::new(egui::FontData::from_static(UI_FONT_BYTES)));
    fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "ui".into());

    let have_mono = load(&mut fonts, "mono", r"C:\Windows\Fonts\CascadiaMono.ttf")
        || load(&mut fonts, "mono", r"C:\Windows\Fonts\CascadiaCode.ttf")
        || load(&mut fonts, "mono", r"C:\Windows\Fonts\consola.ttf");
    if have_mono {
        fonts.families.entry(FontFamily::Monospace).or_default().insert(0, "mono".into());
    }

    let have_sym = load(&mut fonts, "sym", r"C:\Windows\Fonts\seguisym.ttf");
    if have_sym {
        for fam in [FontFamily::Proportional, FontFamily::Monospace] {
            fonts.families.entry(fam).or_default().push("sym".into());
        }
    }

    let styled = [
        (MONO_BOLD, "mono_b", r"C:\Windows\Fonts\consolab.ttf"),
        (MONO_ITALIC, "mono_i", r"C:\Windows\Fonts\consolai.ttf"),
        (MONO_BOLD_ITALIC, "mono_z", r"C:\Windows\Fonts\consolaz.ttf"),
    ];
    for (family, data_name, path) in styled {
        let mut chain: Vec<String> = Vec::new();
        if load(&mut fonts, data_name, path) {
            chain.push(data_name.into());
        }
        if have_mono {
            chain.push("mono".into());
        }
        if have_sym {
            chain.push("sym".into());
        }
        fonts.families.insert(FontFamily::Name(family.into()), chain);
    }

    ctx.set_fonts(fonts);
}

fn apply_theme(ctx: &egui::Context) {
    ctx.set_zoom_factor(UI_ZOOM);
    let mut style = (*ctx.global_style()).clone();
    let v = &mut style.visuals;
    v.dark_mode = true;
    v.panel_fill = BG_WINDOW;
    v.window_fill = BG_WINDOW;
    v.extreme_bg_color = BG_TERMINAL;
    v.faint_bg_color = BG_ELEVATED;
    v.code_bg_color = BG_TERMINAL;
    v.hyperlink_color = ACCENT;
    v.window_stroke = Stroke::new(1.0, BORDER);
    v.window_corner_radius = CornerRadius::same(8);

    v.widgets.noninteractive.bg_fill = BG_PANEL;

    let r = CornerRadius::same(6);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, SEP);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, FG);
    v.widgets.inactive.weak_bg_fill = BG_ELEVATED;
    v.widgets.inactive.bg_fill = BG_ELEVATED;
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, FG);
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER_SOFT);
    v.widgets.inactive.corner_radius = r;
    v.widgets.hovered.weak_bg_fill = BG_HOVER;
    v.widgets.hovered.bg_fill = BG_HOVER;
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, FG);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, BORDER);
    v.widgets.hovered.corner_radius = r;
    v.widgets.active.weak_bg_fill = BG_HOVER;
    v.widgets.active.bg_fill = BG_HOVER;
    v.widgets.active.fg_stroke = Stroke::new(1.0, ACCENT);
    v.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT_DIM);
    v.widgets.active.corner_radius = r;
    v.selection.bg_fill = BG_SELECTED;
    v.selection.stroke = Stroke::new(1.0, ACCENT_DIM);

    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(10.0, 5.0);
    style.spacing.window_margin = egui::Margin::same(10);

    style.text_styles = [
        (TextStyle::Heading, FontId::new(19.0, FontFamily::Proportional)),
        (TextStyle::Body, FontId::new(14.5, FontFamily::Proportional)),
        (TextStyle::Monospace, FontId::new(13.5, FontFamily::Monospace)),
        (TextStyle::Button, FontId::new(14.0, FontFamily::Proportional)),
        (TextStyle::Small, FontId::new(14.0, FontFamily::Proportional)),
    ]
    .into();

    ctx.set_global_style(style);
}

fn draw_splash(ui: &mut egui::Ui, t: f32, brand: Option<&egui::TextureHandle>) {
    let rect = ui.max_rect();
    let p = ui.painter();
    p.rect_filled(rect, 0.0, BG_TERMINAL);
    let c = rect.center();

    if let Some(tex) = brand {
        let s = 72.0;
        p.image(
            tex.id(),
            egui::Rect::from_center_size(c - egui::vec2(0.0, 112.0), egui::vec2(s, s)),
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }

    p.text(
        c - egui::vec2(0.0, 46.0),
        egui::Align2::CENTER_CENTER,
        "HYPERIUM",
        FontId::new(48.0, FontFamily::Proportional),
        ACCENT,
    );
    p.text(
        c - egui::vec2(0.0, 12.0),
        egui::Align2::CENTER_CENTER,
        "D E V   O N   S T E R O I D S",
        FontId::new(13.0, FontFamily::Proportional),
        DIM,
    );

    let steps = [
        (0.05, "booting kernel"),
        (0.22, "loading theme · lime / orange"),
        (0.42, "probing system · cpu / ram"),
        (0.62, "detecting engines"),
        (0.82, "spawning terminals"),
        (0.99, "ready"),
    ];
    let mut y = c.y + 24.0;
    for (thr, label) in steps {
        if t >= thr {
            let ready = thr >= 0.99;
            p.text(
                egui::pos2(c.x - 110.0, y),
                egui::Align2::LEFT_CENTER,
                format!("⏺ {label}"),
                FontId::new(12.0, FontFamily::Monospace),
                if ready { ACCENT } else { DIM },
            );
            y += 18.0;
        }
    }

    let bar_w = 320.0;
    let bar = egui::Rect::from_center_size(
        egui::pos2(c.x, rect.bottom() - 70.0),
        egui::vec2(bar_w, 4.0),
    );
    p.rect_filled(bar, 2.0, BG_ELEVATED);
    let fill = egui::Rect::from_min_size(bar.min, egui::vec2(bar_w * t.clamp(0.0, 1.0), 4.0));
    p.rect_filled(fill, 2.0, ACCENT);
    p.text(
        egui::pos2(c.x, rect.bottom() - 50.0),
        egui::Align2::CENTER_CENTER,
        format!("{:>3.0}%   ·   v{VERSION}", t * 100.0),
        FontId::new(11.0, FontFamily::Monospace),
        DIM,
    );
}

fn locate_dir(rel: &std::path::Path) -> std::path::PathBuf {
    if rel.is_dir() {
        return rel.to_path_buf();
    }
    if let Ok(exe) = std::env::current_exe() {
        for ancestor in exe.ancestors() {
            let candidate = ancestor.join(rel);
            if candidate.is_dir() {
                return candidate;
            }
        }
    }
    rel.to_path_buf()
}

fn music_dir() -> std::path::PathBuf {
    locate_dir(std::path::Path::new("music"))
}

#[cfg(windows)]
fn attach_parent_console() {
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_GENERIC_READ, FILE_GENERIC_WRITE,
        FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows::Win32::System::Console::{
        AttachConsole, ATTACH_PARENT_PROCESS, GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE,
        STD_OUTPUT_HANDLE, SetStdHandle,
    };
    use windows::core::w;
    unsafe {
        let has_stdout = GetStdHandle(STD_OUTPUT_HANDLE)
            .map(|h| {
                let v = h.0 as isize;
                v != 0 && v != -1
            })
            .unwrap_or(false);
        if has_stdout {
            return;
        }
        if AttachConsole(ATTACH_PARENT_PROCESS).is_err() {
            return;
        }
        let open = |name| {
            CreateFileW(
                name,
                (FILE_GENERIC_READ | FILE_GENERIC_WRITE).0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAGS_AND_ATTRIBUTES(0),
                None,
            )
            .ok()
        };
        if let Some(h) = open(w!("CONOUT$")) {
            let _ = SetStdHandle(STD_OUTPUT_HANDLE, h);
            let _ = SetStdHandle(STD_ERROR_HANDLE, h);
        }
        if let Some(h) = open(w!("CONIN$")) {
            let _ = SetStdHandle(STD_INPUT_HANDLE, h);
        }
    }
}

#[cfg(not(windows))]
fn attach_parent_console() {}

fn config_dir() -> std::path::PathBuf {
    if let Ok(appdata) = std::env::var("APPDATA") {
        return std::path::Path::new(&appdata).join("Hyperium");
    }
    std::path::PathBuf::from(".")
}

fn ensure_config_dir() {
    let _ = std::fs::create_dir_all(config_dir());
}

fn human_bytes(b: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let f = b as f64;
    if f >= GB {
        format!("{:.2} Go", f / GB)
    } else if f >= MB {
        format!("{:.1} Mo", f / MB)
    } else if f >= KB {
        format!("{:.0} Ko", f / KB)
    } else {
        format!("{b} o")
    }
}

fn backup_label(p: &std::path::Path) -> Option<String> {
    let name = p.file_name()?.to_str()?;
    let s = name.strip_prefix("hyperium-backup-")?.strip_suffix(".zip")?;
    let (d, t) = s.split_once('-')?;
    if d.len() == 8 && t.len() == 6 {
        Some(format!(
            "{}-{}-{} {}:{}:{}",
            &d[0..4], &d[4..6], &d[6..8], &t[0..2], &t[2..4], &t[4..6]
        ))
    } else {
        Some(s.to_string())
    }
}

fn ai_key_path() -> std::path::PathBuf {
    config_dir().join("anthropic.key")
}

fn load_ai_key_local() -> String {
    secret::load_secret(&ai_key_path())
}

fn save_ai_key(key: &str) {
    secret::save_secret(&ai_key_path(), key);
}

type RecFrames = Vec<(egui::ColorImage, u32)>;

const REC_GIF: &[u8] = include_bytes!("../assets/ball.gif");

fn decode_rec_gif() -> RecFrames {
    use image::AnimationDecoder;
    let Ok(decoder) = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(REC_GIF)) else {
        return Vec::new();
    };
    let Ok(frames) = decoder.into_frames().collect_frames() else {
        return Vec::new();
    };
    let key_white = !frames.iter().any(|f| f.buffer().pixels().any(|p| p.0[3] < 250));
    frames
        .into_iter()
        .map(|f| {
            let (num, den) = f.delay().numer_denom_ms();
            let ms = num.checked_div(den).map_or(40, |v| v.max(10));
            let mut buf = f.into_buffer();
            if key_white {
                for px in buf.pixels_mut() {
                    let [r, g, b, _] = px.0;
                    let m = r.min(g).min(b);
                    px.0[3] = if m >= 244 {
                        0
                    } else if m >= 200 {
                        ((244 - m) as u32 * 255 / 44) as u8
                    } else {
                        255
                    };
                }
            }
            let (w, h) = buf.dimensions();
            let img = egui::ColorImage::from_rgba_unmultiplied(
                [w as usize, h as usize],
                buf.as_raw(),
            );
            (img, ms)
        })
        .collect()
}

fn state_path() -> std::path::PathBuf {
    let dir = config_dir();
    if dir == std::path::Path::new(".") {
        return std::path::PathBuf::from("hyperium_projects.tsv");
    }
    dir.join("projects.tsv")
}

fn commands_path() -> std::path::PathBuf {
    let dir = config_dir();
    if dir == std::path::Path::new(".") {
        return std::path::PathBuf::from("hyperium_commands.tsv");
    }
    dir.join("commands.tsv")
}

fn default_projects_root() -> String {
    let home =
        std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_default();
    if home.is_empty() {
        String::new()
    } else {
        std::path::Path::new(&home).join("Hyperium").display().to_string()
    }
}

fn projects_root_path() -> std::path::PathBuf {
    config_dir().join("projects_root.txt")
}

fn load_projects_root() -> String {
    let saved = std::fs::read_to_string(projects_root_path()).unwrap_or_default();
    let saved = saved.trim();
    if saved.is_empty() { default_projects_root() } else { saved.to_string() }
}

fn save_projects_root(root: &str) {
    let _ = std::fs::write(projects_root_path(), root.trim());
}

fn onboarded_path() -> std::path::PathBuf {
    config_dir().join("onboarded.txt")
}

fn load_onboarded() -> bool {
    onboarded_path().exists()
}

fn mark_onboarded() {
    let _ = std::fs::write(onboarded_path(), "1");
}

const TEMPLATES_KEY: &str = "_templates";

fn templates_dir_path() -> std::path::PathBuf {
    config_dir().join("templates_dir.txt")
}

fn load_templates_dir_override() -> String {
    std::fs::read_to_string(templates_dir_path()).unwrap_or_default().trim().to_string()
}

fn save_templates_dir_override(path: &str) {
    let _ = std::fs::write(templates_dir_path(), path.trim());
}

fn templates_dir() -> std::path::PathBuf {
    let over = load_templates_dir_override();
    if !over.is_empty() {
        return std::path::PathBuf::from(over);
    }
    std::path::PathBuf::from(load_projects_root()).join("_templates")
}

fn load_projects() -> Vec<Project> {
    let mut projects = Vec::new();
    if let Ok(content) = std::fs::read_to_string(state_path()) {
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let mut fields = line.splitn(3, '\t');
            let flag = fields.next().unwrap_or("0");
            let Some(path) = fields.next().filter(|p| !p.is_empty()) else {
                continue;
            };
            let out_dir = fields.next().filter(|s| !s.is_empty());
            let mut project = Project::from_path(path, flag == "1");
            if let Some(dir) = out_dir {
                project.out_dir = dir.to_string();
            }
            projects.push(project);
        }
    }
    if projects.is_empty() {
        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| ".".into());
        projects.push(Project::from_path(&cwd, true));
    }
    projects
}

fn media_btn(ui: &mut egui::Ui, glyph: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(38.0, 28.0), egui::Sense::click());
    let hov = resp.hovered();
    let p = ui.painter();
    p.rect_filled(rect, 6.0, if hov { BG_HOVER } else { BG_ELEVATED });
    p.rect_stroke(
        rect,
        6.0,
        Stroke::new(1.0, if hov { BORDER } else { BORDER_SOFT }),
        egui::StrokeKind::Inside,
    );
    p.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        glyph,
        FontId::proportional(15.0),
        if hov { ACCENT } else { FG },
    );
    resp.clicked()
}

fn audio_card(ui: &mut egui::Ui, audio: &mut AudioPlayer) {
    card(ui, "MOOD · AUDIO", |ui| {
        if audio.playlist().is_empty() {
            ui.label(RichText::new("no tracks - drop .mp3 in /music").color(DIM).small());
            return;
        }

        let title = audio
            .current_track()
            .map(|t| t.title.clone())
            .unwrap_or_else(|| "- idle -".to_string());
        ui.horizontal(|ui| {
            ui.label(RichText::new("♪").color(ACCENT).small());
            ui.label(RichText::new(title).color(FG).monospace().small());
        });
        ui.add_space(8.0);

        ui.horizontal(|ui| {
            if media_btn(ui, "⏮") {
                audio.prev();
            }
            if media_btn(ui, if audio.is_playing() { "⏸" } else { "▶" }) {
                audio.toggle_pause();
            }
            if media_btn(ui, "⏭") {
                audio.next();
            }
        });
        ui.add_space(8.0);

        ui.horizontal(|ui| {
            ui.label(RichText::new("VOL").color(DIM).small());
            let mut v = audio.volume();
            if ui
                .add(egui::Slider::new(&mut v, 0.0..=1.0).show_value(false).trailing_fill(true))
                .changed()
            {
                audio.set_volume(v);
            }
        });
    });
}

struct ProjMeta {
    name: String,
    branch: String,
    agent: String,
    favorite: bool,
    dirty: bool,
    open: bool,
    terms_open: usize,
    sync_badge: sync::Badge,
    has_wiki: bool,
    wiki_running: bool,
}

fn read_clipboard() -> Option<String> {
    arboard::Clipboard::new()
        .and_then(|mut cb| cb.get_text())
        .ok()
        .filter(|s| !s.is_empty())
}

fn encode_input(events: &[egui::Event]) -> Vec<u8> {
    use egui::{Event, Key};
    let mut out = Vec::new();
    for ev in events {
        match ev {
            Event::Text(text) => out.extend_from_slice(text.as_bytes()),
            Event::Paste(text) => out.extend_from_slice(text.as_bytes()),
            Event::Key { key, pressed: true, modifiers, .. } => {
                if modifiers.ctrl && !modifiers.alt && let Some(b) = ctrl_byte(*key) {
                    out.push(b);
                    continue;
                }
                match key {
                    Key::Enter => out.push(b'\r'),
                    Key::Backspace => out.push(0x7f),
                    Key::Tab => out.push(b'\t'),
                    Key::Escape => out.push(0x1b),
                    Key::ArrowUp => out.extend_from_slice(b"\x1b[A"),
                    Key::ArrowDown => out.extend_from_slice(b"\x1b[B"),
                    Key::ArrowRight => out.extend_from_slice(b"\x1b[C"),
                    Key::ArrowLeft => out.extend_from_slice(b"\x1b[D"),
                    Key::Home => out.extend_from_slice(b"\x1b[H"),
                    Key::End => out.extend_from_slice(b"\x1b[F"),
                    Key::Delete => out.extend_from_slice(b"\x1b[3~"),
                    Key::PageUp => out.extend_from_slice(b"\x1b[5~"),
                    Key::PageDown => out.extend_from_slice(b"\x1b[6~"),
                    _ => {}
                }
            }
            _ => {}
        }
    }
    out
}

fn ctrl_byte(key: egui::Key) -> Option<u8> {
    let name = key.name();
    let bytes = name.as_bytes();
    if bytes.len() == 1 && bytes[0].is_ascii_alphabetic() {
        Some(bytes[0].to_ascii_uppercase() & 0x1f)
    } else {
        None
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_terminal_grid(
    painter: &egui::Painter,
    origin: egui::Pos2,
    char_w: f32,
    line_h: f32,
    font_size: f32,
    session: &PtySession,
    focused: bool,
    blink_on: bool,
) {
    let font = FontId::monospace(font_size);
    let font_bold = FontId::new(font_size, FontFamily::Name(MONO_BOLD.into()));
    let font_italic = FontId::new(font_size, FontFamily::Name(MONO_ITALIC.into()));
    let font_bold_italic = FontId::new(font_size, FontFamily::Name(MONO_BOLD_ITALIC.into()));
    let content = session.term().renderable_content();
    let colors = content.colors;
    let selection = content.selection;

    let resolve = |c: Color, default: Color32| -> Color32 {
        let rgb = match c {
            Color::Spec(rgb) => Some(rgb),
            Color::Named(n) => colors[n],
            Color::Indexed(i) => colors[i as usize],
        };
        rgb.map(|r| Color32::from_rgb(r.r, r.g, r.b)).unwrap_or(default)
    };

    let cursor = content.cursor;
    let mut cursor_glyph: Option<(char, Flags)> = None;
    let display_offset = content.display_offset as i32;
    for cell in content.display_iter {
        let row = cell.point.line.0 + display_offset;
        if row < 0 {
            continue;
        }
        let x = origin.x + cell.point.column.0 as f32 * char_w;
        let y = origin.y + row as f32 * line_h;
        let flags = cell.flags;

        let mut fg = resolve(cell.fg, FG);
        let mut bg = if matches!(cell.bg, Color::Named(NamedColor::Background)) {
            None
        } else {
            Some(resolve(cell.bg, BG_TERMINAL))
        };
        if flags.contains(Flags::INVERSE) {
            let prev_fg = fg;
            fg = bg.unwrap_or(BG_TERMINAL);
            bg = Some(prev_fg);
        }

        if let Some(sel) = selection
            && sel.contains(cell.point)
        {
            bg = Some(SELECT_BG);
        }

        if let Some(bg) = bg {
            painter.rect_filled(
                egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(char_w, line_h)),
                0.0,
                bg,
            );
        }

        let ch = cell.c;
        if cell.point == cursor.point {
            cursor_glyph = Some((ch, flags));
        }
        let visible = ch != ' ' && ch != '\0' && !flags.contains(Flags::HIDDEN);
        if visible {
            if flags.contains(Flags::DIM) {
                fg = fg.gamma_multiply(0.6);
            }
            let f = match (flags.contains(Flags::BOLD), flags.contains(Flags::ITALIC)) {
                (true, true) => &font_bold_italic,
                (true, false) => &font_bold,
                (false, true) => &font_italic,
                (false, false) => &font,
            };
            painter.text(egui::pos2(x, y), egui::Align2::LEFT_TOP, ch, f.clone(), fg);
        }
        if flags.contains(Flags::UNDERLINE) {
            let uy = y + line_h - 1.5;
            painter.line_segment(
                [egui::pos2(x, uy), egui::pos2(x + char_w, uy)],
                Stroke::new(1.0, fg),
            );
        }
        if flags.contains(Flags::STRIKEOUT) {
            let sy = y + line_h * 0.5;
            painter.line_segment(
                [egui::pos2(x, sy), egui::pos2(x + char_w, sy)],
                Stroke::new(1.0, fg),
            );
        }
    }

    if !matches!(cursor.shape, CursorShape::Hidden) {
        let cx = origin.x + cursor.point.column.0 as f32 * char_w;
        let cy = origin.y + (cursor.point.line.0 + display_offset).max(0) as f32 * line_h;
        let rect = egui::Rect::from_min_size(egui::pos2(cx, cy), egui::vec2(char_w, line_h));
        if focused {
            if blink_on {
                match cursor.shape {
                    CursorShape::Beam => {
                        painter.rect_filled(
                            egui::Rect::from_min_size(egui::pos2(cx, cy), egui::vec2(2.0, line_h)),
                            0.0,
                            ACCENT,
                        );
                    }
                    CursorShape::Underline => {
                        painter.rect_filled(
                            egui::Rect::from_min_size(
                                egui::pos2(cx, cy + line_h - 2.0),
                                egui::vec2(char_w, 2.0),
                            ),
                            0.0,
                            ACCENT,
                        );
                    }
                    _ => {
                        painter.rect_filled(rect, 0.0, ACCENT);
                        if let Some((ch, flags)) = cursor_glyph
                            && ch != ' '
                            && ch != '\0'
                            && !flags.contains(Flags::HIDDEN)
                        {
                            let f = match (
                                flags.contains(Flags::BOLD),
                                flags.contains(Flags::ITALIC),
                            ) {
                                (true, true) => &font_bold_italic,
                                (true, false) => &font_bold,
                                (false, true) => &font_italic,
                                (false, false) => &font,
                            };
                            painter.text(
                                egui::pos2(cx, cy),
                                egui::Align2::LEFT_TOP,
                                ch,
                                f.clone(),
                                BG_TERMINAL,
                            );
                        }
                    }
                }
            }
        } else {
            painter.rect_stroke(rect, 0.0, Stroke::new(1.0, ACCENT_DIM), egui::StrokeKind::Inside);
        }
    }
}

fn grid_dims(n: usize) -> (usize, usize) {
    match n {
        0 | 1 => (1, 1),
        2 => (1, 2),
        3 => (1, 3),
        4 => (2, 2),
        _ => (2, 3),
    }
}

#[derive(Default)]
struct PaneAct {
    focus: bool,
    close: bool,
}

#[allow(clippy::too_many_arguments)]
fn draw_pane(
    ui: &mut egui::Ui,
    cell: egui::Rect,
    name: &str,
    path: &str,
    t: &mut Term,
    focused: bool,
    idx: usize,
    closable: bool,
    input_enabled: bool,
) -> PaneAct {
    let mut act = PaneAct::default();

    let pane_resp = ui.interact(cell, egui::Id::new(("pane", name, idx)), egui::Sense::click());
    let hovered = ui.rect_contains_pointer(cell);
    let pane_clicked = pane_resp.clicked();
    if pane_clicked {
        act.focus = true;
    }

    let border = if focused {
        ACCENT
    } else if hovered {
        ACCENT_DIM
    } else {
        BORDER_SOFT
    };
    ui.painter().rect_filled(cell, 8.0, BG_TERMINAL);
    ui.painter()
        .rect_stroke(cell, 8.0, Stroke::new(1.0, border), egui::StrokeKind::Inside);

    let inner = cell.shrink(11.0);
    let mut cui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    cui.set_clip_rect(inner);

    let pane_title = t
        .session
        .as_ref()
        .and_then(|s| s.title())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| t.title.clone());

    cui.horizontal(|ui| {
        let dot = if t.agent == "shell" { ORANGE } else { ACCENT };
        ui.label(RichText::new("●").color(dot).small());
        ui.label(
            RichText::new(&pane_title)
                .color(if focused { FG } else { DIM })
                .monospace()
                .small(),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if closable {
                let (r, resp) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::click());
                let hov = resp.hovered();
                if hov {
                    ui.painter().rect_filled(r, 4.0, BG_HOVER);
                }
                ui.painter().text(
                    r.center(),
                    egui::Align2::CENTER_CENTER,
                    "✕",
                    FontId::proportional(12.0),
                    if hov { RED } else { DIM },
                );
                if resp.clicked() {
                    act.close = true;
                }
            }
        });
    });
    cui.add_space(6.0);

    let body = cui.available_rect_before_wrap();
    let term_size = TERM_FONT_PX / cui.ctx().zoom_factor();
    let font = FontId::monospace(term_size);
    let sample = cui.painter().layout_no_wrap("M".to_string(), font.clone(), FG);
    let char_w = sample.size().x.max(1.0);
    let line_h = sample.size().y.max(1.0);
    let cols = ((body.width() / char_w).floor() as usize).max(1);
    let lines = ((body.height() / line_h).floor() as usize).max(1);

    if !t.spawned {
        t.spawned = true;
        let ctx = cui.ctx().clone();
        t.session =
            PtySession::spawn(path, cols, lines, std::sync::Arc::new(move || ctx.request_repaint()));
    }

    if let Some(session) = t.session.as_mut() {
        session.resize(cols, lines);
        if session.pump() {
            cui.ctx().request_repaint();
        }
        let sel_resp = cui.interact(
            body,
            egui::Id::new(("term_sel", name, idx)),
            egui::Sense::click_and_drag(),
        );
        let to_cell = |pos: egui::Pos2| -> (usize, usize) {
            let c = (((pos.x - body.min.x) / char_w) as i32).clamp(0, cols as i32 - 1) as usize;
            let r = (((pos.y - body.min.y) / line_h) as i32).clamp(0, lines as i32 - 1) as usize;
            (c, r)
        };

        if hovered {
            let (dy, shift) = cui.input(|i| (i.smooth_scroll_delta.y, i.modifiers.shift));
            let by = (dy / line_h).round() as i32;
            if by != 0 {
                let (c, r) = cui.input(|i| i.pointer.hover_pos()).map(to_cell).unwrap_or((0, 0));
                if session.scroll_wheel(by, c, r, shift) {
                    cui.ctx().request_repaint();
                }
            }
        }

        if sel_resp.drag_started() {
            act.focus = true;
            if let Some(pos) = sel_resp.interact_pointer_pos() {
                let (c, r) = to_cell(pos);
                session.selection_start(c, r);
            }
        }
        if sel_resp.dragged()
            && let Some(pos) = sel_resp.interact_pointer_pos()
        {
            let (c, r) = to_cell(pos);
            session.selection_update(c, r);
        }
        if sel_resp.clicked() {
            act.focus = true;
            session.clear_selection();
        }
        if sel_resp.drag_stopped()
            && let Some(text) = session.selection_text()
        {
            cui.ctx().copy_text(text);
        }
        if sel_resp.secondary_clicked() {
            act.focus = true;
            if input_enabled && let Some(text) = read_clipboard() {
                session.send_input(text.as_bytes());
                session.clear_selection();
            }
        }
        if sel_resp.clicked()
            || sel_resp.secondary_clicked()
            || sel_resp.drag_started()
            || sel_resp.dragged()
            || sel_resp.drag_stopped()
        {
            cui.ctx().request_repaint();
        }

        if input_enabled && focused && cui.memory(|m| m.focused().is_none()) {
            let (events, shift) = cui.input(|i| (i.events.clone(), i.modifiers.shift));
            let mut copy: Option<String> = None;
            for ev in &events {
                match ev {
                    egui::Event::Copy | egui::Event::Cut => {
                        if shift {
                            copy = session.selection_text();
                        } else {
                            let b = if matches!(ev, egui::Event::Cut) { 0x18 } else { 0x03 };
                            session.send_input(&[b]);
                            session.clear_selection();
                        }
                    }
                    _ => {}
                }
            }
            let bytes = encode_input(&events);
            if !bytes.is_empty() {
                session.send_input(&bytes);
                session.clear_selection();
            }
            if let Some(text) = copy {
                cui.ctx().copy_text(text);
            }
            if !bytes.is_empty() || !events.is_empty() {
                cui.ctx().request_repaint();
            }
        }
        let blink_on = if focused {
            let phase = (cui.input(|i| i.time) / 1.06).rem_euclid(1.0);
            let to_flip = if phase < 0.5 { 0.5 - phase } else { 1.0 - phase } * 1.06;
            cui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64(to_flip.max(0.016)));
            phase < 0.5
        } else {
            false
        };
        paint_terminal_grid(
            cui.painter(),
            body.min,
            char_w,
            line_h,
            term_size,
            session,
            focused,
            blink_on,
        );
    } else {
        cui.painter().text(
            body.center(),
            egui::Align2::CENTER_CENTER,
            "⚠ could not start shell (powershell.exe)",
            font,
            RED,
        );
    }

    act
}

fn win_button(ui: &mut egui::Ui, glyph: &str, danger: bool) -> bool {
    let size = egui::vec2(46.0, ui.available_height().max(28.0));
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let hov = resp.hovered();
    if hov {
        ui.painter()
            .rect_filled(rect, 0.0, if danger { RED } else { BG_HOVER });
    }
    let col = if hov && danger { Color32::WHITE } else { FG };
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        glyph,
        egui::FontId::proportional(15.0),
        col,
    );
    resp.clicked()
}

fn card<R>(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::default()
        .fill(BG_PANEL)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(8.0)
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if !title.is_empty() {
                ui.label(RichText::new(title).color(ORANGE).small().strong());
                ui.add_space(8.0);
            }
            add(ui)
        })
        .inner
}

fn pill_button(ui: &mut egui::Ui, label: &str, min_width: f32, enabled: bool) -> bool {
    let font = FontId::new(13.0, FontFamily::Proportional);
    let text_w = ui.painter().layout_no_wrap(label.to_string(), font.clone(), FG).size().x;
    let w = (text_w + 26.0).max(min_width);
    let sense = if enabled { egui::Sense::click() } else { egui::Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 30.0), sense);
    let hov = enabled && resp.hovered();

    let (fill, stroke, text) = if !enabled {
        (BG_ELEVATED, BORDER_SOFT, FAINT)
    } else if hov {
        (BG_SELECTED, ACCENT_DIM, ACCENT)
    } else {
        (BG_ELEVATED, BORDER, FG)
    };
    let p = ui.painter();
    p.rect_filled(rect, 8.0, fill);
    p.rect_stroke(rect, 8.0, Stroke::new(1.0, stroke), egui::StrokeKind::Inside);
    p.text(rect.center(), egui::Align2::CENTER_CENTER, label, font, text);
    enabled && resp.clicked()
}

fn draw_project(ui: &mut egui::Ui, p: &ProjMeta, selected: bool) -> (egui::Response, bool) {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 44.0), egui::Sense::click());
    let hov = resp.hovered();
    let painter = ui.painter();

    let fill = if selected {
        BG_SELECTED
    } else if hov {
        BG_HOVER
    } else {
        Color32::TRANSPARENT
    };
    if fill != Color32::TRANSPARENT {
        painter.rect_filled(rect, 6.0, fill);
    }
    if selected {
        let bar = egui::Rect::from_min_max(rect.left_top(), egui::pos2(rect.left() + 3.0, rect.bottom()));
        painter.rect_filled(bar, CornerRadius::same(2), ACCENT);
    }

    let x = rect.left() + 14.0;
    let y1 = rect.top() + 14.0;
    let y2 = rect.top() + 31.0;

    let dot = if p.dirty { DOT_DIRTY } else { DOT_CLEAN };
    painter.circle_filled(egui::pos2(x + 2.0, y1), 3.0, dot);

    let mut nx = x + 14.0;
    if p.favorite {
        nx += painter
            .text(
                egui::pos2(nx, y1),
                egui::Align2::LEFT_CENTER,
                "★ ",
                FontId::new(12.5, FontFamily::Proportional),
                ORANGE,
            )
            .width();
    }
    let badge_glyph = match p.sync_badge {
        sync::Badge::PushNeeded | sync::Badge::LocalOnly => Some(("⚡", ORANGE)),
        sync::Badge::PullNeeded => Some(("↓", ACCENT)),
        sync::Badge::Diverged => Some(("⚡", PINK)),
        sync::Badge::Synced | sync::Badge::Unknown => None,
    };
    if let Some((glyph, color)) = badge_glyph {
        nx += painter
            .text(
                egui::pos2(nx, y1),
                egui::Align2::LEFT_CENTER,
                format!("{glyph} "),
                FontId::new(12.5, FontFamily::Proportional),
                color,
            )
            .width();
    }
    painter.text(
        egui::pos2(nx, y1),
        egui::Align2::LEFT_CENTER,
        &p.name,
        FontId::new(14.5, FontFamily::Proportional),
        if selected { ACCENT } else { FG },
    );
    let subtitle = if p.branch.is_empty() {
        p.agent.clone()
    } else {
        format!("⎇ {}  ·  {}", p.branch, p.agent)
    };
    painter.text(
        egui::pos2(x + 14.0, y2),
        egui::Align2::LEFT_CENTER,
        subtitle,
        FontId::new(12.0, FontFamily::Proportional),
        DIM,
    );

    let mut right = rect.right() - 12.0;

    if p.open && p.terms_open > 0 {
        let label = p.terms_open.to_string();
        let font = FontId::new(11.0, FontFamily::Proportional);
        let tw = painter.layout_no_wrap(label.clone(), font.clone(), ACCENT).size().x;
        let bh = 17.0;
        let bw = (tw + 12.0).max(bh);
        let brect = egui::Rect::from_min_size(
            egui::pos2(right - bw, rect.center().y - bh / 2.0),
            egui::vec2(bw, bh),
        );
        painter.rect_filled(brect, CornerRadius::same(8), BG_ELEVATED);
        painter.rect_stroke(
            brect,
            CornerRadius::same(8),
            Stroke::new(1.0, ACCENT_DIM),
            egui::StrokeKind::Inside,
        );
        painter.text(brect.center(), egui::Align2::CENTER_CENTER, label, font, ACCENT);
        right -= bw + 8.0;
    }

    let mut wiki_clicked = false;
    if p.has_wiki {
        let glyph = "📖";
        let font = FontId::new(14.0, FontFamily::Proportional);
        let gw = painter.layout_no_wrap(glyph.to_string(), font.clone(), FG).size().x;
        let bw = gw.max(16.0);
        let center = egui::pos2(right - bw / 2.0, rect.center().y);
        let hit = egui::Rect::from_center_size(center, egui::vec2(bw + 8.0, 22.0));
        let id = egui::Id::new(("wiki-btn", &p.name));
        let r = ui.interact(hit, id, egui::Sense::click());
        let color = if p.wiki_running {
            ACCENT
        } else if r.hovered() {
            FG
        } else {
            DIM
        };
        painter.text(center, egui::Align2::CENTER_CENTER, glyph, font, color);
        wiki_clicked = r.clicked();
        let _ = r.on_hover_text(if p.wiki_running {
            "Wiki running · click to open in the browser"
        } else {
            "Launch the project wiki"
        });
    }
    let _ = right;

    (resp, wiki_clicked)
}

#[derive(Default)]
struct TabAct {
    select: bool,
    close: bool,
}

fn draw_tab(ui: &mut egui::Ui, name: &str, active: bool) -> TabAct {
    let font = FontId::new(13.5, FontFamily::Proportional);
    let text_w = ui
        .painter()
        .layout_no_wrap(name.to_string(), font.clone(), FG)
        .size()
        .x;
    let close_w = 18.0;
    let (rect, resp) = ui.allocate_exact_size(
        egui::vec2(text_w + 26.0 + close_w, 32.0),
        egui::Sense::click(),
    );
    let hov = resp.hovered();
    let mut act = TabAct::default();

    {
        let painter = ui.painter();
        if active {
            painter.rect_filled(rect, CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 }, BG_TERMINAL);
        } else if hov {
            painter.rect_filled(rect, CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 }, BG_HOVER);
        }
        let col = if active || hov { FG } else { DIM };
        painter.text(
            egui::pos2(rect.left() + 12.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            name,
            font,
            col,
        );
        if active {
            let y = rect.bottom() - 1.25;
            painter.line_segment(
                [egui::pos2(rect.left() + 2.0, y), egui::pos2(rect.right() - 2.0, y)],
                Stroke::new(2.5, ACCENT),
            );
        }
    }

    if active || hov {
        let close_rect = egui::Rect::from_center_size(
            egui::pos2(rect.right() - close_w / 2.0 - 4.0, rect.center().y),
            egui::vec2(close_w, close_w),
        );
        let cr = ui.interact(close_rect, egui::Id::new(("tabclose", name)), egui::Sense::click());
        let chov = cr.hovered();
        if chov {
            ui.painter().rect_filled(close_rect, 4.0, BG_HOVER);
        }
        ui.painter().text(
            close_rect.center(),
            egui::Align2::CENTER_CENTER,
            "✕",
            FontId::proportional(11.0),
            if chov { RED } else { DIM },
        );
        if cr.clicked() {
            act.close = true;
        }
    }

    if resp.clicked() && !act.close {
        act.select = true;
    }
    if resp.middle_clicked() {
        act.close = true;
    }
    act
}

fn fmt_mem(bytes: u64) -> String {
    let mb = bytes as f64 / 1.0e6;
    if mb >= 1000.0 {
        format!("{:.1} GB", mb / 1000.0)
    } else {
        format!("{:.0} MB", mb)
    }
}

fn proc_tree_usage(
    sys: &System,
    children: &std::collections::HashMap<sysinfo::Pid, Vec<sysinfo::Pid>>,
    root: u32,
) -> (f32, u64, usize) {
    let procs = sys.processes();
    let mut stack = vec![sysinfo::Pid::from_u32(root)];
    let mut seen = std::collections::HashSet::new();
    let (mut cpu, mut mem, mut count) = (0.0f32, 0u64, 0usize);
    while let Some(pid) = stack.pop() {
        if !seen.insert(pid) {
            continue;
        }
        if let Some(p) = procs.get(&pid) {
            cpu += p.cpu_usage();
            mem += p.memory();
            count += 1;
        }
        if let Some(kids) = children.get(&pid) {
            stack.extend(kids.iter().copied());
        }
    }
    (cpu, mem, count)
}

fn proc_tree_has_claude(
    sys: &System,
    children: &std::collections::HashMap<sysinfo::Pid, Vec<sysinfo::Pid>>,
    root: u32,
) -> bool {
    let procs = sys.processes();
    let mut stack = vec![sysinfo::Pid::from_u32(root)];
    let mut seen = std::collections::HashSet::new();
    while let Some(pid) = stack.pop() {
        if !seen.insert(pid) {
            continue;
        }
        if let Some(p) = procs.get(&pid)
            && proc_is_claude(p)
        {
            return true;
        }
        if let Some(kids) = children.get(&pid) {
            stack.extend(kids.iter().copied());
        }
    }
    false
}

fn proc_is_claude(p: &sysinfo::Process) -> bool {
    if p.name().to_string_lossy().to_lowercase().contains("claude") {
        return true;
    }
    if let Some(exe) = p.exe()
        && exe.to_string_lossy().to_lowercase().contains("claude")
    {
        return true;
    }
    p.cmd().iter().any(|a| a.to_string_lossy().to_lowercase().contains("claude"))
}

fn category_color(cat: &str) -> Color32 {
    const PALETTE: [Color32; 6] = [
        Color32::from_rgb(46, 64, 82),
        Color32::from_rgb(64, 52, 78),
        Color32::from_rgb(44, 72, 60),
        Color32::from_rgb(78, 62, 44),
        Color32::from_rgb(56, 56, 74),
        Color32::from_rgb(72, 48, 54),
    ];
    let sum: usize = cat.bytes().map(|b| b as usize).sum();
    PALETTE[sum % PALETTE.len()]
}

fn ellipsize(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
    t.push('\u{2026}');
    t
}

fn market_card(ui: &mut egui::Ui, e: &templates::Entry, thumb: Option<&egui::TextureHandle>) -> bool {
    let w: f32 = 250.0;
    let thumb_h = (w * 9.0 / 16.0).round();
    let gap = 8.0;
    let title_h = 20.0;
    let desc_h = 32.0;
    let card = egui::vec2(w, thumb_h + gap + title_h + desc_h);

    let resp = ui.allocate_response(card, egui::Sense::click());
    let rect = resp.rect;
    let hovered = resp.hovered();
    if hovered {
        ui.painter().rect_filled(rect.expand(8.0), 12.0, BG_ELEVATED);
        ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::PointingHand);
    }

    let thumb_rect = egui::Rect::from_min_size(rect.min, egui::vec2(w, thumb_h));
    match thumb {
        Some(tex) => {
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            ui.painter().image(tex.id(), thumb_rect, uv, Color32::WHITE);
        }
        None => {
            ui.painter().rect_filled(thumb_rect, 8.0, category_color(&e.category));
            if let Some(ch) = e.title.chars().next() {
                ui.painter().text(
                    thumb_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    ch.to_uppercase().to_string(),
                    egui::FontId::proportional(40.0),
                    Color32::from_white_alpha(210),
                );
            }
        }
    }

    let p = ui.painter();
    p.text(
        egui::pos2(rect.min.x, thumb_rect.max.y + gap),
        egui::Align2::LEFT_TOP,
        ellipsize(&e.title, 30),
        egui::FontId::proportional(14.5),
        FG,
    );
    let desc_rect = egui::Rect::from_min_size(
        egui::pos2(rect.min.x, thumb_rect.max.y + gap + title_h),
        egui::vec2(w, desc_h),
    );
    let desc_col = Color32::from_gray(150);
    let galley = p.layout(e.short.clone(), egui::FontId::proportional(12.0), desc_col, w);
    p.with_clip_rect(desc_rect).galley(desc_rect.min, galley, desc_col);

    resp.clicked()
}

fn draw_market_detail(
    ui: &mut egui::Ui,
    e: &templates::Entry,
    thumb: Option<&egui::TextureHandle>,
    back: &mut bool,
    use_id: &mut Option<String>,
) {
    ui.horizontal(|ui| {
        if ui.button("\u{2190} Back").clicked() {
            *back = true;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let btn = egui::Button::new(
                RichText::new("Use this template").color(Color32::BLACK).strong(),
            )
            .fill(ACCENT);
            if ui.add(btn).clicked() {
                *use_id = Some(e.id.clone());
            }
        });
    });
    ui.add_space(10.0);
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        let w = ui.available_width().min(680.0);
        let h = (w * 9.0 / 16.0).round();
        match thumb {
            Some(tex) => {
                let img = egui::Image::new(egui::load::SizedTexture::new(tex.id(), egui::vec2(w, h)))
                    .maintain_aspect_ratio(false)
                    .corner_radius(10.0);
                ui.add_sized(egui::vec2(w, h), img);
            }
            None => {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(w, h), egui::Sense::hover());
                ui.painter().rect_filled(rect, 10.0, category_color(&e.category));
            }
        }
        ui.add_space(12.0);
        ui.label(RichText::new(&e.title).color(FG).size(22.0).strong());
        if !e.category.is_empty() {
            ui.label(RichText::new(&e.category).color(DIM).small());
        }
        ui.add_space(8.0);
        let long = if e.long.is_empty() { e.short.clone() } else { e.long.clone() };
        ui.label(RichText::new(long).color(FG));
        if !e.tags.is_empty() {
            ui.add_space(8.0);
            ui.label(RichText::new(format!("tags: {}", e.tags.join(", "))).color(DIM).small());
        }
    });
}

fn metric_bar(ui: &mut egui::Ui, label: &str, frac: f32, text: String, color: Color32) {
    ui.label(RichText::new(label).color(DIM).small());
    let bar = egui::ProgressBar::new(frac.clamp(0.0, 1.0))
        .fill(color)
        .corner_radius(CornerRadius::same(0))
        .text(RichText::new(text).small());
    ui.add(bar);
    ui.add_space(6.0);
}

fn hard_exit() -> ! {
    std::process::exit(0)
}

impl eframe::App for HyperiumApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if let Some(tray) = &self.tray {
            match tray::take_action(tray) {
                tray::TrayAction::Show => {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                tray::TrayAction::Quit => hard_exit(),
                tray::TrayAction::None => {}
            }
            if ui.ctx().input(|i| i.viewport().close_requested()) {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::CancelClose);
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Visible(false));
            }
        } else if ui.ctx().input(|i| i.viewport().close_requested()) {
            hard_exit();
        }

        {
            let path = self.open_req.path.lock().unwrap_or_else(|e| e.into_inner()).take();
            let focus = self.open_req.focus.swap(false, std::sync::atomic::Ordering::SeqCst);
            if let Some(p) = path {
                self.add_project(p);
            }
            if focus {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Visible(true));
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Focus);
            }
        }

        if self.sync_badge_at.is_none_or(|t| t.elapsed() > Duration::from_secs(2)) {
            self.refresh_sync_badges();
            self.refresh_wiki_present();
            self.sync_badge_at = Some(Instant::now());
        }

        if let Screen::Splash(t0) = self.screen {
            let elapsed = t0.elapsed().as_secs_f32();
            draw_splash(ui, (elapsed / SPLASH_SECS).min(1.0), self.brand_icon(72));
            ui.ctx().request_repaint();
            if elapsed >= SPLASH_SECS {
                self.screen =
                    if load_onboarded() { Screen::Cockpit } else { Screen::Onboard };
            }
            return;
        }

        if matches!(self.screen, Screen::Onboard) {
            if self.draw_onboarding(ui) {
                mark_onboarded();
                self.screen = Screen::Cockpit;
            }
            return;
        }

        self.refresh_metrics();
        if let Some(audio) = self.audio.as_mut() {
            audio.tick();
        }
        ui.ctx().request_repaint_after(Duration::from_millis(500));

        if self.coach_next_at.is_none() {
            self.schedule_next_nudge();
        }
        if self.coach_nudge.is_none()
            && self.active_tool.is_none()
            && !self.palette_open
            && !self.settings_open
            && self.coach_next_at.is_some_and(|t| Instant::now() >= t)
        {
            match self.coach.next_nudge() {
                Some(n) => {
                    notify::toast("Hyperium - move", &n.message());
                    self.coach_nudge = Some(n);
                }
                None => self.schedule_next_nudge(),
            }
        }

        self.handle_shortcuts(ui.ctx());

        egui::Panel::top("titlebar")
            .exact_size(40.0)
            .show_separator_line(false)
            .frame(
                egui::Frame::default()
                    .fill(BG_PANEL)
                    .inner_margin(egui::Margin::symmetric(12, 0)),
            )
            .show_inside(ui, |ui| {
                let bar = ui.max_rect();
                let drag = ui.interact(
                    bar,
                    egui::Id::new("titlebar_drag"),
                    egui::Sense::click_and_drag(),
                );
                if drag.drag_started_by(egui::PointerButton::Primary) {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                if drag.double_clicked() {
                    let m = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!m));
                }
                ui.horizontal_centered(|ui| {
                    if let Some(t) = self.brand_icon(18) {
                        ui.add(egui::Image::new((t.id(), egui::vec2(18.0, 18.0))));
                        ui.add_space(3.0);
                    }
                    ui.label(RichText::new("HYPERIUM").color(ACCENT).strong().size(16.0));
                    ui.label(
                        RichText::new(format!("v{VERSION} · {BUILD_HASH}"))
                            .color(FAINT)
                            .small(),
                    );
                    let (upd_avail, upd_busy) = {
                        let s = self.update.lock().unwrap_or_else(|e| e.into_inner());
                        (s.available.clone(), s.busy)
                    };
                    if let Some(rel) = upd_avail {
                        ui.add_space(12.0);
                        let label = if upd_busy {
                            "⬆ installing…".to_string()
                        } else {
                            format!("⬆ Update v{} — Install & restart", rel.version)
                        };
                        let btn = egui::Button::new(RichText::new(label).color(BG_WINDOW).small())
                            .fill(ORANGE);
                        if ui
                            .add_enabled(!upd_busy, btn)
                            .on_hover_text(format!(
                                "New build v{} ({}) is published on your server.\nClick to download, replace hyperium.exe and relaunch.",
                                rel.version, rel.hash
                            ))
                            .clicked()
                        {
                            self.start_install(ui.ctx().clone());
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if win_button(ui, "✕", true) {
                            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        if win_button(ui, "□", false) {
                            let m = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
                            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!m));
                        }
                        if win_button(ui, "─", false) {
                            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                        }
                        ui.add_space(6.0);
                        if win_button(ui, "⚙", false) {
                            self.settings_open = !self.settings_open;
                        }
                        ui.add_space(14.0);
                        ui.horizontal(|ui| {
                            for (idx, h) in self.coach.habits.iter().enumerate() {
                                let p = self.coach.progress_of(&h.id);
                                if idx > 0 {
                                    ui.add_space(12.0);
                                }
                                ui.label(
                                    RichText::new(format!("{} {}/{}", h.icon, p.done, h.daily_target))
                                        .color(FG),
                                );
                                let frac = if h.daily_target == 0 {
                                    0.0
                                } else {
                                    (p.done as f32 / h.daily_target as f32).clamp(0.0, 1.0)
                                };
                                ui.add(
                                    egui::ProgressBar::new(frac)
                                        .fill(ACCENT)
                                        .desired_width(48.0)
                                        .desired_height(6.0),
                                );
                                if p.streak > 0 {
                                    ui.label(RichText::new(format!("🔥{}", p.streak)).color(ORANGE));
                                }
                            }
                            if let Some(t) = self.coach_next_at {
                                let secs = t.saturating_duration_since(Instant::now()).as_secs();
                                let eta = if secs == 0 {
                                    "now".to_string()
                                } else if secs < 60 {
                                    format!("{secs}s")
                                } else {
                                    format!("{}m", secs / 60)
                                };
                                ui.add_space(12.0);
                                ui.label(RichText::new(format!("next ~{eta}")).color(DIM));
                            }
                        });
                    });
                });
            });

        let metas: Vec<ProjMeta> = self
            .projects
            .iter()
            .map(|p| ProjMeta {
                name: p.name.clone(),
                branch: p.branch.clone(),
                agent: p.agent.clone(),
                favorite: p.favorite,
                dirty: p.dirty,
                open: p.open,
                terms_open: p.terms.len(),
                sync_badge: self.sync_badges.get(&p.name).copied().unwrap_or_default(),
                has_wiki: self.wiki_present.get(&p.name).copied().unwrap_or(false),
                wiki_running: wiki::is_running(&p.path),
            })
            .collect();
        egui::Panel::left("projects")
            .resizable(true)
            .show_separator_line(false)
            .default_size(238.0)
            .frame(
                egui::Frame::default()
                    .fill(BG_WINDOW)
                    .inner_margin(egui::Margin::same(12)),
            )
            .show_inside(ui, |ui| {
                ui.add_space(2.0);
                ui.label(RichText::new("PROJECTS").color(FG).small().strong());
                ui.add_space(8.0);

                let mut want_open: Option<usize> = None;
                let mut want_remove: Option<usize> = None;
                let mut want_toggle_wiki: Option<usize> = None;
                let mut want_create_wiki: Option<usize> = None;
                let mut want_stop_wiki: Option<usize> = None;
                let mut want_reveal: Option<usize> = None;
                for (i, p) in metas.iter().enumerate() {
                    let (resp, wiki_clicked) = draw_project(ui, p, self.selected == i);
                    if wiki_clicked {
                        want_toggle_wiki = Some(i);
                    }
                    let base = if p.open {
                        let plural = if p.terms_open > 1 { "s" } else { "" };
                        format!("{} - {} terminal{} open", p.name, p.terms_open, plural)
                    } else {
                        format!("{} - closed · double-click to open", p.name)
                    };
                    let sync_note = match p.sync_badge {
                        sync::Badge::PushNeeded => "  ·  ⚡ local changes not pushed",
                        sync::Badge::LocalOnly => "  ·  ⚡ notes never pushed to the server",
                        sync::Badge::PullNeeded => "  ·  ↓ server has a newer version (pull)",
                        sync::Badge::Diverged => "  ·  ⚡ diverged - both sides changed",
                        sync::Badge::Synced | sync::Badge::Unknown => "",
                    };
                    let resp = resp.on_hover_text(format!("{base}{sync_note}"));
                    resp.context_menu(|ui| {
                        ui.label(RichText::new(&p.name).color(DIM).small().strong());
                        ui.add_space(2.0);
                        if p.has_wiki && p.wiki_running && ui.button("■  Stop wiki server").clicked() {
                            want_stop_wiki = Some(i);
                            ui.close();
                        }
                        if !p.has_wiki && ui.button("📖  Create wiki").clicked() {
                            want_create_wiki = Some(i);
                            ui.close();
                        }
                        if ui.button("⊞  Open in Explorer").clicked() {
                            want_reveal = Some(i);
                            ui.close();
                        }
                        if ui.button("✕  Remove from Hyperium").clicked() {
                            want_remove = Some(i);
                            ui.close();
                        }
                        ui.label(
                            RichText::new("Forgets it from this list only · your files on disk are untouched")
                                .color(FAINT)
                                .small(),
                        );
                    });
                    if !wiki_clicked {
                        if resp.double_clicked() {
                            want_open = Some(i);
                        } else if resp.clicked() && p.open {
                            self.selected = i;
                        }
                    }
                    ui.add_space(2.0);
                }
                if let Some(i) = want_toggle_wiki
                    && let Some(path) = self.projects.get(i).map(|p| p.path.clone())
                {
                    let ctx = ui.ctx().clone();
                    wiki::toggle(&path, move || ctx.request_repaint());
                    ui.ctx().request_repaint_after(Duration::from_millis(1500));
                }
                if let Some(i) = want_create_wiki
                    && let Some((path, name)) =
                        self.projects.get(i).map(|p| (p.path.clone(), p.name.clone()))
                {
                    match wiki::create_wiki(&path, &name) {
                        Ok(()) => {
                            self.refresh_wiki_present();
                            notify::toast("Wiki", "Wiki created · launching...");
                            let ctx = ui.ctx().clone();
                            wiki::toggle(&path, move || ctx.request_repaint());
                            ui.ctx().request_repaint_after(Duration::from_millis(1500));
                        }
                        Err(e) => notify::toast("Wiki", &e),
                    }
                }
                if let Some(i) = want_stop_wiki
                    && let Some(path) = self.projects.get(i).map(|p| p.path.clone())
                {
                    wiki::stop(&path);
                    ui.ctx().request_repaint();
                }
                if let Some(i) = want_reveal
                    && let Some(path) = self.projects.get(i).map(|p| p.path.clone())
                {
                    #[cfg(windows)]
                    {
                        use std::os::windows::process::CommandExt;
                        let _ = std::process::Command::new("explorer")
                            .arg(&path)
                            .creation_flags(0x0800_0000)
                            .spawn();
                    }
                    #[cfg(not(windows))]
                    let _ = std::process::Command::new("xdg-open").arg(&path).spawn();
                }
                if let Some(i) = want_open {
                    self.open_project(i);
                    self.save_state();
                }
                if let Some(i) = want_remove {
                    self.projects.remove(i);
                    if self.selected > i {
                        self.selected -= 1;
                    }
                    if self.projects.get(self.selected).map(|p| !p.open).unwrap_or(true) {
                        self.selected = self.projects.iter().position(|p| p.open).unwrap_or(0);
                    }
                    self.save_state();
                }

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(8.0);

                ui.label(RichText::new("PROJECTS ROOT").color(DIM).small());
                ui.add_space(2.0);
                let root_edit = ui.add(
                    egui::TextEdit::singleline(&mut self.projects_root_edit)
                        .desired_width(ui.available_width())
                        .hint_text(default_projects_root()),
                );
                if root_edit.lost_focus() {
                    save_projects_root(&self.projects_root_edit);
                }
                ui.add_space(6.0);

                if pill_button(ui, "+  Add folder", ui.available_width(), true) {
                    let root = self.projects_root_edit.trim();
                    let mut dlg = rfd::FileDialog::new().set_title("Add a project folder");
                    if !root.is_empty() {
                        let _ = std::fs::create_dir_all(root);
                        dlg = dlg.set_directory(root);
                    }
                    if let Some(folder) = dlg.pick_folder() {
                        self.add_project(folder.to_string_lossy().into_owned());
                    }
                }
            });

        egui::Panel::right("inspector")
            .resizable(true)
            .show_separator_line(false)
            .default_size(258.0)
            .frame(
                egui::Frame::default()
                    .fill(BG_WINDOW)
                    .inner_margin(egui::Margin::same(12)),
            )
            .show_inside(ui, |ui| {
                ui.add_space(2.0);
                ui.label(RichText::new("INSPECTOR").color(FG).small().strong());
                ui.add_space(10.0);

                if let Some(audio) = self.audio.as_mut() {
                    audio_card(ui, audio);
                    ui.add_space(10.0);
                }

                card(ui, "MACHINE · LIVE", |ui| {
                    let cpu_frac = self.cpu / 100.0;
                    metric_bar(ui, "CPU", cpu_frac, format!("{:.0} %", self.cpu), ACCENT);
                    let mem_frac = if self.mem_total > 0 {
                        self.mem_used as f32 / self.mem_total as f32
                    } else { 0.0 };
                    let gb = |b: u64| b as f64 / 1.0e9;
                    metric_bar(ui, "RAM", mem_frac,
                        format!("{:.1} / {:.1} GB", gb(self.mem_used), gb(self.mem_total)),
                        ORANGE);
                });

                ui.add_space(10.0);

                card(ui, "PROJECT · LOAD", |ui| {
                    let Some(proj) = self.projects.get(self.selected).filter(|p| p.open) else {
                        ui.label(RichText::new("no project open").color(DIM).small());
                        return;
                    };
                    ui.label(RichText::new(&proj.name).color(FG).small().strong());
                    ui.add_space(6.0);

                    let ncores = self.sys.cpus().len().max(1) as f32;
                    let children = &self.proc_children;

                    let (mut tot_cpu, mut tot_mem) = (0.0f32, 0u64);
                    let mut any = false;
                    for (i, t) in proj.terms.iter().enumerate() {
                        let label = format!("#{}", i + 1);
                        match t.session.as_ref().and_then(|s| s.pid()) {
                            Some(pid) => {
                                let (cpu, mem, _n) = proc_tree_usage(&self.sys, &children, pid);
                                tot_cpu += cpu;
                                tot_mem += mem;
                                any = true;
                                let pct = cpu / ncores;
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(&label).color(DIM).monospace().small());
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            ui.label(
                                                RichText::new(fmt_mem(mem)).color(DIM).monospace().small(),
                                            );
                                            ui.label(
                                                RichText::new(format!("{pct:.1} %"))
                                                    .color(FG)
                                                    .monospace()
                                                    .small(),
                                            );
                                        },
                                    );
                                });
                                ui.add(
                                    egui::ProgressBar::new((pct / 100.0).clamp(0.0, 1.0))
                                        .fill(ACCENT)
                                        .desired_height(3.0),
                                );
                                ui.add_space(5.0);
                            }
                            None => {
                                ui.label(
                                    RichText::new(format!("{label}   starting…"))
                                        .color(FAINT)
                                        .monospace()
                                        .small(),
                                );
                                ui.add_space(5.0);
                            }
                        }
                    }
                    if any {
                        ui.add_space(1.0);
                        ui.label(
                            RichText::new(format!(
                                "Σ  {:.1} %  ·  {}",
                                tot_cpu / ncores,
                                fmt_mem(tot_mem)
                            ))
                            .color(ORANGE)
                            .small(),
                        );
                    }
                });
            });

        egui::CentralPanel::default()
            .frame(
                egui::Frame::default()
                    .fill(BG_WINDOW)
                    .inner_margin(egui::Margin::same(10)),
            )
            .show_inside(ui, |ui| {
            let mut add_term = false;
            let mut want_close: Option<usize> = None;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (i, p) in metas.iter().enumerate() {
                    if !p.open {
                        continue;
                    }
                    let act = draw_tab(ui, &p.name, i == self.selected);
                    if act.select {
                        self.selected = i;
                    }
                    if act.close {
                        want_close = Some(i);
                    }
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(p) = self.projects.get(self.selected).filter(|p| p.open) {
                        let n = p.terms.len();
                        if pill_button(ui, "+ terminal", 0.0, n < MAX_TERMS) {
                            add_term = true;
                        }
                        ui.label(
                            RichText::new(format!("{}/{}", n, MAX_TERMS))
                                .color(DIM)
                                .small()
                                .monospace(),
                        );
                    }
                });
            });
            let sep_y = ui.cursor().top();
            ui.painter().line_segment(
                [
                    egui::pos2(ui.max_rect().left(), sep_y),
                    egui::pos2(ui.max_rect().right(), sep_y),
                ],
                Stroke::new(1.0, SEP),
            );
            ui.add_space(8.0);

            if let Some(i) = want_close {
                let p = &mut self.projects[i];
                p.open = false;
                p.terms.clear();
                p.focused = 0;
                self.save_state();
            }
            if !self.projects.get(self.selected).is_some_and(|p| p.open) {
                self.selected = self.projects.iter().position(|p| p.open).unwrap_or(0);
            }

            if !self.projects.iter().any(|p| p.open) {
                let area = ui.available_rect_before_wrap();
                ui.painter().text(
                    area.center(),
                    egui::Align2::CENTER_CENTER,
                    "double-click a project in the sidebar to open it",
                    FontId::new(14.0, FontFamily::Monospace),
                    DIM,
                );
                ui.allocate_rect(area, egui::Sense::hover());
                return;
            }

            if add_term {
                let proj = &mut self.projects[self.selected];
                if proj.terms.len() < MAX_TERMS {
                    proj.terms.push(Term::new("shell"));
                    proj.focused = proj.terms.len() - 1;
                }
            }

            let sel = self.selected;
            let name = self.projects[sel].name.clone();
            let path = self.projects[sel].path.clone();
            let input_enabled = self.active_tool.is_none()
                && !self.palette_open
                && !self.market_open
                && self.coach_nudge.is_none();
            let proj = &mut self.projects[sel];

            let n = proj.terms.len();
            let (rows, cols) = grid_dims(n);
            let gap = 8.0;
            let area = ui.available_rect_before_wrap();

            if proj.col_frac.len() != cols {
                proj.col_frac = vec![1.0 / cols as f32; cols];
            }
            if proj.row_frac.len() != rows {
                proj.row_frac = vec![1.0 / rows as f32; rows];
            }

            let inner_w = (area.width() - gap * (cols as f32 - 1.0)).max(1.0);
            let inner_h = (area.height() - gap * (rows as f32 - 1.0)).max(1.0);
            let col_w: Vec<f32> = proj.col_frac.iter().map(|f| f * inner_w).collect();
            let row_h: Vec<f32> = proj.row_frac.iter().map(|f| f * inner_h).collect();
            let col_x: Vec<f32> = (0..cols)
                .scan(area.left(), |x, c| {
                    let at = *x;
                    *x += col_w[c] + gap;
                    Some(at)
                })
                .collect();
            let row_y: Vec<f32> = (0..rows)
                .scan(area.top(), |y, r| {
                    let at = *y;
                    *y += row_h[r] + gap;
                    Some(at)
                })
                .collect();

            let mut to_close: Option<usize> = None;
            let mut new_focus = proj.focused;
            for i in 0..n {
                let (r, c) = (i / cols, i % cols);
                let cell = egui::Rect::from_min_size(
                    egui::pos2(col_x[c], row_y[r]),
                    egui::vec2(col_w[c], row_h[r]),
                );
                let focused = proj.focused == i;
                let act = draw_pane(
                    ui, cell, &name, &path, &mut proj.terms[i], focused, i, n > 1, input_enabled,
                );
                if act.focus {
                    new_focus = i;
                }
                if act.close {
                    to_close = Some(i);
                }
            }
            proj.focused = new_focus.min(n.saturating_sub(1));
            if let Some(i) = to_close
                && proj.terms.len() > 1
            {
                proj.terms.remove(i);
                proj.focused = proj.focused.min(proj.terms.len() - 1);
            }

            const MIN_FRAC: f32 = 0.12;
            let hint = egui::Color32::from_rgb(120, 150, 40);
            for c in 0..cols.saturating_sub(1) {
                let x = col_x[c] + col_w[c] + gap * 0.5;
                let handle =
                    egui::Rect::from_min_size(egui::pos2(x - 4.0, area.top()), egui::vec2(8.0, area.height()));
                let resp = ui.allocate_rect(handle, egui::Sense::drag());
                if resp.hovered() || resp.dragged() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                    ui.painter().rect_filled(
                        egui::Rect::from_center_size(handle.center(), egui::vec2(2.0, area.height())),
                        0.0,
                        hint,
                    );
                }
                if resp.dragged() {
                    let df = resp.drag_delta().x / inner_w;
                    let (a, b) = (proj.col_frac[c] + df, proj.col_frac[c + 1] - df);
                    if a >= MIN_FRAC && b >= MIN_FRAC {
                        proj.col_frac[c] = a;
                        proj.col_frac[c + 1] = b;
                    }
                }
            }
            for r in 0..rows.saturating_sub(1) {
                let y = row_y[r] + row_h[r] + gap * 0.5;
                let handle =
                    egui::Rect::from_min_size(egui::pos2(area.left(), y - 4.0), egui::vec2(area.width(), 8.0));
                let resp = ui.allocate_rect(handle, egui::Sense::drag());
                if resp.hovered() || resp.dragged() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
                    ui.painter().rect_filled(
                        egui::Rect::from_center_size(handle.center(), egui::vec2(area.width(), 2.0)),
                        0.0,
                        hint,
                    );
                }
                if resp.dragged() {
                    let df = resp.drag_delta().y / inner_h;
                    let (a, b) = (proj.row_frac[r] + df, proj.row_frac[r + 1] - df);
                    if a >= MIN_FRAC && b >= MIN_FRAC {
                        proj.row_frac[r] = a;
                        proj.row_frac[r + 1] = b;
                    }
                }
            }
            ui.allocate_rect(area, egui::Sense::hover());
        });

        if self.active_tool.is_some() {
            self.draw_tool(ui.ctx());
        }
        if self.palette_open {
            self.draw_palette(ui.ctx());
        }
        if self.settings_open {
            self.draw_settings(ui.ctx());
        }
        if self.talker_open {
            self.draw_talker(ui.ctx());
        }
        if self.coach_nudge.is_some() {
            self.draw_coach(ui.ctx());
        }
        if self.market_open {
            self.draw_market(ui.ctx());
        }
    }
}

fn main() -> eframe::Result<()> {
    ensure_config_dir();
    if let Some(first) = std::env::args().nth(1)
        && matches!(first.as_str(), "gen-image" | "gen-video" | "gen")
    {
        attach_parent_console();
        let args: Vec<String> = std::env::args().skip(2).collect();
        std::process::exit(genai::cli_main(&config_dir(), &args));
    }

    notify::set_process_aumid();

    let target = std::env::args().nth(1).and_then(|a| stg::resolve_target(&a));
    let listener = match stg::acquire(target.as_deref().unwrap_or("")) {
        stg::Role::Primary(listener) => listener,
        stg::Role::Secondary => return Ok(()),
    };

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1180.0, 720.0])
        .with_min_inner_size([820.0, 520.0])
        .with_maximized(true)
        .with_decorations(false)
        .with_resizable(true)
        .with_title("Hyperium");
    if let Some(ic) = icon::egui_icon() {
        viewport = viewport.with_icon(ic);
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native(
        "Hyperium",
        options,
        Box::new(move |cc| Ok(Box::new(HyperiumApp::new(cc, listener, target)))),
    )
}
