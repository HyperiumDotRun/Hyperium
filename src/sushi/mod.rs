//! Sushi agent — read-only.
//!
//! Two halves. The market table answers "what is moving" and refreshes on its
//! own; the quote box answers "what would this trade give me". The model only
//! ever parses intent — resolution, arithmetic and API calls happen in Rust.
//! Nothing here can sign or broadcast a transaction.

mod api;
mod equities;
mod intent;
mod market;
mod tokens;

use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use eframe::egui;
use egui::{Color32, FontId, RichText};

use crate::tools::{
    ACCENT, BG_ELEVATED, DIM, FAINT, FG, RED, Tool, ToolCtx, pill_select, tool_button,
};

use intent::Intent;

const ORANGE: Color32 = Color32::from_rgb(232, 168, 60);
/// Validated against the dark surface (OKLCH L 0.72, CVD ΔE 7.1 vs RED). That
/// separation is only legal alongside secondary encoding, which is why every
/// percentage is rendered with an explicit +/− sign, never colour alone.
const UP: Color32 = Color32::from_rgb(53, 192, 131);
/// Table row hover. Local rather than borrowed from `tools`, so the table's
/// surface can be tuned without touching the shared design tokens.
const ROW_HOVER: Color32 = Color32::from_rgb(34, 36, 41);

const DEFAULT_SLIPPAGE: f64 = 0.005;
const LOG_MAX: usize = 20;
const MARKET_ROWS: usize = 10;
const MARKET_TTL: Duration = Duration::from_secs(60);

// Table geometry. Numeric columns are right-aligned inside their width so the
// decimal points line up down the column.
const COL_RANK: f32 = 30.0;
const COL_SYM: f32 = 74.0;
const COL_NAME: f32 = 150.0;
const COL_PRICE: f32 = 116.0;
const COL_CHG: f32 = 88.0;
const COL_VOL: f32 = 112.0;
const COL_SPARK: f32 = 124.0;
const TABLE_W: f32 =
    COL_RANK + COL_SYM + COL_NAME + COL_PRICE + COL_CHG + COL_VOL + COL_SPARK;

// Equity tiles. Wide enough for a four-figure index price without the number
// colliding with the ticker above it.
const TILE_W: f32 = 104.0;
const TILE_H: f32 = 46.0;

enum Outcome {
    Price { chain: &'static str, symbol: String, address: String, usd: f64 },
    Quote { chain: &'static str, quote: api::Quote },
    /// Feeds the table rather than the result card.
    Market { rows: Vec<market::Row>, sort: market::Sort, limit: usize },
}

impl Outcome {
    /// Both legs re-derived from the API's own numbers rather than from what we
    /// asked for — if the two ever disagree, the display should show the truth.
    fn legs(quote: &api::Quote) -> (String, String) {
        (
            tokens::format_units(quote.amount_in, quote.token_in.decimals),
            tokens::format_units(quote.amount_out, quote.token_out.decimals),
        )
    }
}

struct LogLine {
    at: Instant,
    text: String,
    ok: bool,
}

pub struct SushiTool {
    ask: String,
    chain_idx: usize,
    in_idx: usize,
    out_idx: usize,
    amount: String,
    sushi_key: String,
    key_open: bool,
    loaded: bool,
    /// Saved value, mirrors `config_dir/anthropic.key`.
    ai_key: String,
    /// Edit buffer, kept apart from `ai_key` so the setup block does not
    /// vanish on the first keystroke.
    ai_input: String,
    rx: Option<Receiver<Result<Outcome, String>>>,
    result: Option<Result<Outcome, String>>,
    log: Vec<LogLine>,
    market: Vec<market::Row>,
    market_rx: Option<Receiver<Result<Vec<market::Row>, String>>>,
    market_err: Option<String>,
    market_at: Option<Instant>,
    market_sort: market::Sort,
    market_limit: usize,
    equities: Vec<equities::Row>,
    equities_rx: Option<Receiver<Result<Vec<equities::Row>, String>>>,
    equities_err: Option<String>,
    equities_at: Option<Instant>,
}

impl Default for SushiTool {
    fn default() -> Self {
        Self {
            ask: String::new(),
            chain_idx: 0,
            in_idx: 0,
            out_idx: 2,
            amount: "1".into(),
            sushi_key: String::new(),
            key_open: false,
            loaded: false,
            ai_key: String::new(),
            ai_input: String::new(),
            rx: None,
            result: None,
            log: Vec::new(),
            market: Vec::new(),
            market_rx: None,
            market_err: None,
            market_at: None,
            market_sort: market::Sort::Volume,
            market_limit: MARKET_ROWS,
            equities: Vec::new(),
            equities_rx: None,
            equities_err: None,
            equities_at: None,
        }
    }
}

fn sushi_key_path(cfg: &std::path::Path) -> std::path::PathBuf {
    cfg.join("sushi.key")
}

/// Execute an already-resolved intent. No model involved past this point.
fn run(intent: Intent, api_key: &str) -> Result<Outcome, String> {
    match intent {
        Intent::Market { sort, limit } => {
            Ok(Outcome::Market { rows: market::fetch(sort, limit)?, sort, limit })
        }
        Intent::Price { chain, token } => {
            let usd = api::price(chain.id, token.address, api_key)?;
            Ok(Outcome::Price {
                chain: chain.name,
                symbol: token.symbol.to_string(),
                address: token.address.to_string(),
                usd,
            })
        }
        Intent::Quote { chain, token_in, token_out, amount_raw } => {
            let quote = api::quote(
                &api::QuoteRequest {
                    chain_id: chain.id,
                    token_in: token_in.address.to_string(),
                    token_out: token_out.address.to_string(),
                    amount: amount_raw,
                    max_slippage: DEFAULT_SLIPPAGE,
                },
                api_key,
            )?;
            Ok(Outcome::Quote { chain: chain.name, quote })
        }
    }
}

impl SushiTool {
    fn chain(&self) -> &'static tokens::Chain {
        &tokens::CHAINS[self.chain_idx.min(tokens::CHAINS.len() - 1)]
    }

    fn spawn(
        &mut self,
        ctx: &egui::Context,
        job: impl FnOnce() -> Result<Outcome, String> + Send + 'static,
    ) {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(job());
        });
        self.rx = Some(rx);
        ctx.request_repaint();
    }

    fn refresh_market(&mut self, ctx: &egui::Context) {
        if self.market_rx.is_some() {
            return;
        }
        let (sort, limit) = (self.market_sort, self.market_limit);
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(market::fetch(sort, limit));
        });
        self.market_rx = Some(rx);
        ctx.request_repaint();
    }

    fn refresh_equities(&mut self, ctx: &egui::Context) {
        if self.equities_rx.is_some() {
            return;
        }
        let api_key = self.sushi_key.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(equities::fetch(&api_key));
        });
        self.equities_rx = Some(rx);
        ctx.request_repaint();
    }

    /// Natural language path: model parses, Rust resolves and runs.
    fn ask_model(&mut self, ctx: &egui::Context) {
        let question = self.ask.trim().to_string();
        if question.is_empty() {
            return;
        }
        let ai_key = self.ai_key.clone();
        let api_key = self.sushi_key.clone();
        self.spawn(ctx, move || {
            use crate::llm::LlmProvider;
            let raw =
                crate::llm::Anthropic::new(ai_key).complete(&intent::system_prompt(), &question)?;
            run(intent::parse(&raw)?, &api_key)
        });
    }

    /// Manual path: no model at all, the pills already are the intent.
    fn quote_manual(&mut self, ctx: &egui::Context) {
        let chain = self.chain();
        let symbols = chain.symbols();
        let (Some(sin), Some(sout)) =
            (symbols.get(self.in_idx).copied(), symbols.get(self.out_idx).copied())
        else {
            return;
        };
        if sin == sout {
            self.result = Some(Err("input and output token are the same".into()));
            return;
        }
        let (Some(token_in), Some(token_out)) = (chain.token(sin), chain.token(sout)) else {
            return;
        };
        let amount_raw = match tokens::parse_units(self.amount.trim(), token_in.decimals) {
            Ok(0) => {
                self.result = Some(Err("amount is zero".into()));
                return;
            }
            Ok(v) => v,
            Err(e) => {
                self.result = Some(Err(e));
                return;
            }
        };
        let api_key = self.sushi_key.clone();
        self.spawn(ctx, move || {
            run(Intent::Quote { chain, token_in, token_out, amount_raw }, &api_key)
        });
    }

    fn record(&mut self, outcome: &Result<Outcome, String>) {
        let (text, ok) = match outcome {
            Ok(Outcome::Price { chain, symbol, usd, .. }) => {
                (format!("{symbol} on {chain} = ${usd}"), true)
            }
            Ok(Outcome::Quote { chain, quote }) => {
                let (sent, got) = Outcome::legs(quote);
                (
                    format!(
                        "{sent} {} → {got} {} on {chain}",
                        quote.token_in.symbol, quote.token_out.symbol
                    ),
                    true,
                )
            }
            Ok(Outcome::Market { rows, sort, .. }) => {
                (format!("market · {} · {} rows", sort.label(), rows.len()), true)
            }
            Err(e) => (e.clone(), false),
        };
        self.log.insert(0, LogLine { at: Instant::now(), text, ok });
        self.log.truncate(LOG_MAX);
    }

    fn poll(&mut self, ui: &egui::Ui) {
        if let Some(rx) = &self.rx {
            match rx.try_recv() {
                Ok(outcome) => {
                    self.record(&outcome);
                    // A market intent updates the table; everything else the card.
                    match outcome {
                        Ok(Outcome::Market { rows, sort, limit }) => {
                            self.market = rows;
                            self.market_sort = sort;
                            self.market_limit = limit;
                            self.market_err = None;
                            self.market_at = Some(Instant::now());
                            self.result = None;
                        }
                        other => self.result = Some(other),
                    }
                    self.rx = None;
                }
                Err(TryRecvError::Empty) => ui.ctx().request_repaint(),
                Err(TryRecvError::Disconnected) => self.rx = None,
            }
        }
        if let Some(rx) = &self.market_rx {
            match rx.try_recv() {
                Ok(Ok(rows)) => {
                    self.market = rows;
                    self.market_err = None;
                    self.market_at = Some(Instant::now());
                    self.market_rx = None;
                }
                Ok(Err(e)) => {
                    self.market_err = Some(e);
                    self.market_at = Some(Instant::now());
                    self.market_rx = None;
                }
                Err(TryRecvError::Empty) => ui.ctx().request_repaint(),
                Err(TryRecvError::Disconnected) => self.market_rx = None,
            }
        }
        if let Some(rx) = &self.equities_rx {
            match rx.try_recv() {
                Ok(Ok(rows)) => {
                    self.equities = rows;
                    self.equities_err = None;
                    self.equities_at = Some(Instant::now());
                    self.equities_rx = None;
                }
                Ok(Err(e)) => {
                    self.equities_err = Some(e);
                    self.equities_at = Some(Instant::now());
                    self.equities_rx = None;
                }
                Err(TryRecvError::Empty) => ui.ctx().request_repaint(),
                Err(TryRecvError::Disconnected) => self.equities_rx = None,
            }
        }
    }

    /// Tokenised equities, laid out as tiles rather than table rows: there is
    /// one number per name, so a row of cells would be mostly whitespace.
    fn equities_section(&mut self, ui: &mut egui::Ui) {
        let loading = self.equities_rx.is_some();

        ui.horizontal(|ui| {
            ui.label(RichText::new("TOKENISED EQUITIES").color(ORANGE).small().strong());
            ui.label(
                RichText::new(format!("{} · Sushi pools", equities::CHAIN))
                    .color(FAINT)
                    .small(),
            );
            if loading {
                ui.add(egui::Spinner::new().size(13.0).color(ORANGE));
            } else if let Some(at) = self.equities_at {
                ui.label(RichText::new(ago(at.elapsed().as_secs())).color(FAINT).small());
            }
            if tool_button(ui, "Refresh", !loading) && !loading {
                let ctx = ui.ctx().clone();
                self.refresh_equities(&ctx);
            }
        });
        ui.add_space(6.0);

        if let Some(e) = &self.equities_err {
            ui.label(RichText::new(format!("✗ {e}")).color(RED).small());
            return;
        }
        if self.equities.is_empty() {
            ui.label(RichText::new("loading…").color(FAINT).small());
            return;
        }

        ui.horizontal_wrapped(|ui| {
            for row in &self.equities {
                equity_tile(ui, row);
            }
        });
        ui.add_space(4.0);
        ui.label(
            RichText::new("Pool-implied prices, not exchange quotes.").color(FAINT).small(),
        );
    }

    fn market_section(&mut self, ui: &mut egui::Ui) {
        let loading = self.market_rx.is_some();

        ui.horizontal(|ui| {
            ui.label(RichText::new("MARKET").color(ACCENT).small().strong());
            ui.label(
                RichText::new(format!(
                    "top {} {}",
                    self.market_limit,
                    self.market_sort.label()
                ))
                .color(FAINT)
                .small(),
            );
            if loading {
                ui.add(egui::Spinner::new().size(13.0).color(ACCENT));
            } else if let Some(at) = self.market_at {
                ui.label(RichText::new(ago(at.elapsed().as_secs())).color(FAINT).small());
            }
            if tool_button(ui, "Refresh", !loading) && !loading {
                let ctx = ui.ctx().clone();
                self.refresh_market(&ctx);
            }
        });
        ui.add_space(6.0);

        if let Some(e) = &self.market_err {
            ui.label(RichText::new(format!("✗ {e}")).color(RED).small());
            return;
        }
        if self.market.is_empty() {
            ui.label(RichText::new("loading…").color(FAINT).small());
            return;
        }

        table_header(ui);
        for (i, row) in self.market.iter().enumerate() {
            market_row(ui, i, row);
        }
    }
}

impl Tool for SushiTool {
    fn title(&self) -> &'static str {
        "Sushi agent"
    }
    fn about(&self) -> &'static str {
        "What's moving, and what a swap would give you. Read-only."
    }
    fn uses_output_dir(&self) -> bool {
        false
    }

    fn ui(&mut self, ui: &mut egui::Ui, octx: &ToolCtx) {
        if !self.loaded {
            self.ai_key = crate::secret::load_secret(&octx.config_dir.join("anthropic.key"));
            self.sushi_key = crate::secret::load_secret(&sushi_key_path(octx.config_dir));
            self.loaded = true;
            let ctx = ui.ctx().clone();
            self.refresh_market(&ctx);
            self.refresh_equities(&ctx);
        }

        self.poll(ui);

        // Keep the table warm without the user asking.
        let stale = self.market_at.map(|t| t.elapsed() >= MARKET_TTL).unwrap_or(false);
        if stale && self.market_rx.is_none() {
            let ctx = ui.ctx().clone();
            self.refresh_market(&ctx);
        }
        let eq_stale = self.equities_at.map(|t| t.elapsed() >= MARKET_TTL).unwrap_or(false);
        if eq_stale && self.equities_rx.is_none() {
            let ctx = ui.ctx().clone();
            self.refresh_equities(&ctx);
        }
        if self.market_at.is_some() || self.equities_at.is_some() {
            ui.ctx().request_repaint_after(Duration::from_secs(1));
        }

        egui::ScrollArea::vertical().show(ui, |ui| {
            self.equities_section(ui);

            ui.add_space(18.0);
            ui.separator();
            ui.add_space(14.0);

            self.market_section(ui);

            ui.add_space(18.0);
            ui.separator();
            ui.add_space(14.0);

            self.quote_section(ui, octx);
        });
    }
}

impl SushiTool {
    fn quote_section(&mut self, ui: &mut egui::Ui, octx: &ToolCtx) {
        let busy = self.rx.is_some();

        ui.label(RichText::new("QUOTE").color(ACCENT).small().strong());
        ui.add_space(8.0);

        let names: Vec<&str> = tokens::CHAINS.iter().map(|c| c.name).collect();
        let before = self.chain_idx;
        pill_select(ui, &names, &mut self.chain_idx);
        if self.chain_idx != before {
            let n = self.chain().tokens.len();
            self.in_idx = self.in_idx.min(n - 1);
            self.out_idx = self.out_idx.min(n - 1);
        }
        ui.add_space(10.0);

        let has_ai = !self.ai_key.trim().is_empty();
        ui.horizontal(|ui| {
            let field = ui.add_enabled(
                has_ai && !busy,
                egui::TextEdit::singleline(&mut self.ask)
                    .hint_text("how much is 0.5 ETH in USDC?")
                    // Capped: the panel can be 1900px wide, and a text field
                    // stretched that far pushes Ask off to the far edge.
                    .desired_width((ui.available_width() - 90.0).min(420.0)),
            );
            let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (tool_button(ui, "Ask", has_ai && !busy) || entered) && has_ai && !busy {
                let ctx = ui.ctx().clone();
                self.ask_model(&ctx);
            }
        });
        if !has_ai {
            self.ai_key_setup(ui, octx);
        }
        ui.add_space(10.0);

        let symbols = self.chain().symbols();
        ui.label(RichText::new("from").color(FAINT).small());
        pill_select(ui, &symbols, &mut self.in_idx);
        ui.add_space(2.0);
        ui.label(RichText::new("to").color(FAINT).small());
        pill_select(ui, &symbols, &mut self.out_idx);
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.amount)
                    .hint_text("amount")
                    .desired_width(110.0),
            );
            if tool_button(ui, "Quote", !busy) && !busy {
                let ctx = ui.ctx().clone();
                self.quote_manual(&ctx);
            }
            if busy {
                ui.add(egui::Spinner::new().size(16.0).color(ACCENT));
            }
        });
        ui.add_space(12.0);

        match &self.result {
            Some(Ok(outcome)) => result_card(ui, outcome),
            Some(Err(e)) => {
                ui.label(RichText::new(format!("✗ {e}")).color(RED));
            }
            None => {
                ui.label(
                    RichText::new("Read-only. No transaction is ever built here.")
                        .color(FAINT)
                        .small(),
                );
            }
        }

        if !self.log.is_empty() {
            ui.add_space(14.0);
            ui.label(RichText::new("- recent -").color(FAINT).small());
            ui.add_space(4.0);
            for line in &self.log {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(if line.ok { "✓" } else { "✗" })
                            .color(if line.ok { UP } else { RED })
                            .small(),
                    );
                    ui.label(RichText::new(&line.text).color(DIM).small());
                    ui.label(RichText::new(ago(line.at.elapsed().as_secs())).color(FAINT).small());
                });
            }
        }

        ui.add_space(14.0);
        let arrow = if self.key_open { "▾" } else { "▸" };
        if ui
            .add(
                egui::Label::new(
                    RichText::new(format!("{arrow} Sushi API key (optional)")).color(FAINT).small(),
                )
                .sense(egui::Sense::click()),
            )
            .clicked()
        {
            self.key_open = !self.key_open;
        }
        if self.key_open {
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.sushi_key)
                        .password(true)
                        .hint_text("sushi_…")
                        .desired_width(240.0),
                );
                if tool_button(ui, "Save", true) {
                    crate::secret::save_secret(&sushi_key_path(octx.config_dir), &self.sushi_key);
                }
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new("Raises rate limits.").color(FAINT).small());
                ui.hyperlink_to(
                    RichText::new("get one ↗").color(ACCENT).small(),
                    "https://www.sushi.com/portal",
                );
            });
        }
    }
}

impl SushiTool {
    /// Fix the missing key here instead of sending the user hunting through
    /// Settings. Writes `config_dir/anthropic.key` — the very file Settings →
    /// AI reads, so the two stay in sync rather than becoming two stores.
    fn ai_key_setup(&mut self, ui: &mut egui::Ui, octx: &ToolCtx) {
        egui::Frame::default()
            .fill(BG_ELEVATED)
            .corner_radius(10.0)
            .inner_margin(egui::Margin::same(12))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("⚠").color(ORANGE));
                    ui.label(
                        RichText::new("No Anthropic key — the ask box is off.")
                            .color(FG)
                            .small(),
                    );
                    ui.hyperlink_to(
                        RichText::new("get one ↗").color(ACCENT).small(),
                        "https://console.anthropic.com/settings/keys",
                    );
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.ai_input)
                            .password(true)
                            .hint_text("sk-ant-…")
                            .desired_width(260.0),
                    );
                    let ready = !self.ai_input.trim().is_empty();
                    if tool_button(ui, "Save", ready) && ready {
                        let key = self.ai_input.trim().to_string();
                        crate::secret::save_secret(
                            &octx.config_dir.join("anthropic.key"),
                            &key,
                        );
                        self.ai_key = key;
                        self.ai_input.clear();
                    }
                });
                ui.label(
                    RichText::new(
                        "Same key as Settings → AI. The manual row below works without it.",
                    )
                    .color(FAINT)
                    .small(),
                );
            });
    }
}

fn table_header(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(TABLE_W, 20.0), egui::Sense::hover());
    let p = ui.painter();
    let cy = rect.center().y;
    let f = FontId::proportional(10.5);
    let mut x = rect.left() + 8.0;

    x += COL_RANK;
    p.text(egui::pos2(x, cy), egui::Align2::LEFT_CENTER, "TOKEN", f.clone(), FAINT);
    x += COL_SYM + COL_NAME;
    x += COL_PRICE;
    p.text(egui::pos2(x, cy), egui::Align2::RIGHT_CENTER, "PRICE", f.clone(), FAINT);
    x += COL_CHG;
    p.text(egui::pos2(x, cy), egui::Align2::RIGHT_CENTER, "24H", f.clone(), FAINT);
    x += COL_VOL;
    p.text(egui::pos2(x, cy), egui::Align2::RIGHT_CENTER, "VOLUME 24H", f.clone(), FAINT);
    x += 12.0;
    p.text(egui::pos2(x, cy), egui::Align2::LEFT_CENTER, "7D", f, FAINT);
}

fn market_row(ui: &mut egui::Ui, i: usize, r: &market::Row) {
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(TABLE_W, 26.0), egui::Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, 4.0, ROW_HOVER);
    }

    let cy = rect.center().y;
    let num = FontId::monospace(12.5);
    let mut x = rect.left() + 8.0;

    p.text(
        egui::pos2(x, cy),
        egui::Align2::LEFT_CENTER,
        format!("{}", i + 1),
        FontId::monospace(11.0),
        FAINT,
    );
    x += COL_RANK;

    p.text(
        egui::pos2(x, cy),
        egui::Align2::LEFT_CENTER,
        &r.symbol,
        FontId::proportional(13.5),
        FG,
    );
    x += COL_SYM;

    p.text(
        egui::pos2(x, cy),
        egui::Align2::LEFT_CENTER,
        truncate(&r.name, 20),
        FontId::proportional(12.0),
        DIM,
    );
    x += COL_NAME;

    x += COL_PRICE;
    p.text(
        egui::pos2(x, cy),
        egui::Align2::RIGHT_CENTER,
        market::money_price(r.price),
        num.clone(),
        FG,
    );

    x += COL_CHG;
    // Colour AND sign: the sign is what survives colour-blindness.
    let (chg_text, chg_color) = match r.change_24h {
        Some(c) => (market::pct_signed(c), if c < 0.0 { RED } else { UP }),
        None => ("—".to_string(), FAINT),
    };
    p.text(egui::pos2(x, cy), egui::Align2::RIGHT_CENTER, chg_text, num.clone(), chg_color);

    x += COL_VOL;
    p.text(
        egui::pos2(x, cy),
        egui::Align2::RIGHT_CENTER,
        market::money_compact(r.volume),
        num,
        DIM,
    );

    let spark_rect = egui::Rect::from_min_size(
        egui::pos2(x + 12.0, rect.top() + 6.0),
        egui::vec2(COL_SPARK - 16.0, rect.height() - 12.0),
    );
    sparkline(p, spark_rect, &r.spark, chg_color);
}

/// One ticker, one price. Sized so a row of them wraps into an even grid at the
/// panel widths the cockpit actually uses.
fn equity_tile(ui: &mut egui::Ui, r: &equities::Row) {
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(TILE_W, TILE_H), egui::Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let p = ui.painter();
    p.rect_filled(rect, 5.0, if resp.hovered() { ROW_HOVER } else { BG_ELEVATED });

    p.text(
        egui::pos2(rect.left() + 10.0, rect.top() + 13.0),
        egui::Align2::LEFT_CENTER,
        r.symbol,
        FontId::proportional(13.5),
        FG,
    );
    p.text(
        egui::pos2(rect.right() - 10.0, rect.bottom() - 14.0),
        egui::Align2::RIGHT_CENTER,
        market::money_price(r.price),
        FontId::monospace(12.5),
        ORANGE,
    );
}

/// Bare 7-day line: no axes, no grid, no labels. It carries shape only — the
/// exact numbers live in the columns to its left.
fn sparkline(p: &egui::Painter, rect: egui::Rect, pts: &[f32], color: Color32) {
    if pts.len() < 2 {
        return;
    }
    let (min, max) = pts
        .iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    let span = max - min;
    let last = pts.len() as f32 - 1.0;
    let points: Vec<egui::Pos2> = pts
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            // A flat series has zero span; pin it to the middle instead of
            // dividing by zero.
            let t = if span > f32::EPSILON { (v - min) / span } else { 0.5 };
            egui::pos2(
                rect.left() + rect.width() * (i as f32 / last),
                rect.bottom() - rect.height() * t,
            )
        })
        .collect();
    p.add(egui::Shape::line(points, egui::Stroke::new(1.4_f32, color)));
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max.saturating_sub(1)).collect::<String>() + "…"
}

fn result_card(ui: &mut egui::Ui, outcome: &Outcome) {
    egui::Frame::default()
        .fill(BG_ELEVATED)
        .corner_radius(12.0)
        .inner_margin(egui::Margin::same(14))
        .show(ui, |ui| match outcome {
            // Routed to the table in `poll`, so it never reaches the card —
            // rendering nothing keeps that invariant instead of panicking on it.
            Outcome::Market { .. } => {}
            Outcome::Price { chain, symbol, address, usd } => {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(symbol).color(FG).strong());
                    ui.label(RichText::new(*chain).color(DIM).small());
                });
                ui.label(
                    RichText::new(market::money_price(*usd))
                        .color(ACCENT)
                        .font(FontId::monospace(20.0)),
                );
                ui.label(RichText::new(address).color(FAINT).font(FontId::monospace(11.0)));
            }
            Outcome::Quote { chain, quote } => {
                let (sent, got) = Outcome::legs(quote);
                ui.horizontal(|ui| {
                    let (label, color) = match quote.status.as_str() {
                        "Success" => ("Success", UP),
                        "Partial" => ("Partial route", ORANGE),
                        other => (other, DIM),
                    };
                    ui.label(RichText::new(label).color(color).strong());
                    ui.label(RichText::new(*chain).color(DIM).small());
                });
                ui.add_space(6.0);
                ui.label(
                    RichText::new(format!(
                        "{sent} {}  →  {got} {}",
                        quote.token_in.symbol, quote.token_out.symbol
                    ))
                    .color(FG)
                    .font(FontId::monospace(16.0)),
                );
                ui.add_space(6.0);
                if let Some(pi) = quote.price_impact {
                    // Fraction, not percent: 0.005 is 0.5%.
                    let pct = pi * 100.0;
                    let color = if pct.abs() >= 1.0 { RED } else { DIM };
                    ui.label(RichText::new(format!("price impact {pct:.3}%")).color(color).small());
                }
                if let Some(p) = quote.unit_price() {
                    ui.label(
                        RichText::new(format!(
                            "1 {} ≈ {} {}",
                            quote.token_in.symbol,
                            fmt_price(p),
                            quote.token_out.symbol
                        ))
                        .color(DIM)
                        .small(),
                    );
                }
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!("in  {}", quote.token_in.address))
                        .color(FAINT)
                        .font(FontId::monospace(11.0)),
                );
                ui.label(
                    RichText::new(format!("out {}", quote.token_out.address))
                        .color(FAINT)
                        .font(FontId::monospace(11.0)),
                );
            }
        });
}

/// Prices span many orders of magnitude (ETH/USDC vs SUSHI/ETH), so pick the
/// precision from the value instead of a fixed one that would print "0.000000".
fn fmt_price(p: f64) -> String {
    match p.abs() {
        v if v >= 1000.0 => format!("{p:.2}"),
        v if v >= 1.0 => format!("{p:.4}"),
        v if v >= 0.0001 => format!("{p:.8}"),
        _ => format!("{p:.3e}"),
    }
}

fn ago(secs: u64) -> String {
    match secs {
        0..=4 => "just now".to_string(),
        5..=59 => format!("{secs}s ago"),
        60..=3599 => format!("{}m ago", secs / 60),
        _ => format!("{}h ago", secs / 3600),
    }
}
