//! Sushi agent — read-only.
//!
//! The agent leads: you ask in plain English and it answers. Below it sit the
//! two boards it draws on — what is launching on Robinhood Chain right now, and
//! what is moving across the wider market. The model only ever parses intent;
//! resolution, arithmetic and API calls happen in Rust. Nothing here can sign
//! or broadcast a transaction.

mod api;
mod trending;
mod intent;
mod market;
mod tokens;

use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use eframe::egui;
use egui::{Color32, FontId, RichText};

use crate::tools::{ACCENT, BG_ELEVATED, DIM, FAINT, FG, RED, Tool, ToolCtx, tool_button};

use intent::Intent;

const ORANGE: Color32 = Color32::from_rgb(232, 168, 60);
/// Validated against the dark surface (OKLCH L 0.72, CVD ΔE 7.1 vs RED). That
/// separation is only legal alongside secondary encoding, which is why every
/// percentage is rendered with an explicit +/− sign, never colour alone.
const UP: Color32 = Color32::from_rgb(53, 192, 131);
/// Table row hover. Local rather than borrowed from `tools`, so the table's
/// surface can be tuned without touching the shared design tokens.
const ROW_HOVER: Color32 = Color32::from_rgb(34, 36, 41);

/// One per intent the parser can produce — price, quote, market — so clicking
/// through all three is a tour of everything the agent does. Written with bare
/// tickers because the model resolves against the whitelist, and a starter that
/// missed would teach the wrong lesson about what the agent accepts.
const EXAMPLES: &[&str] = &["price of WETH", "1 WETH in USDG", "top gainers"];

/// Phrases cycled while a request is in flight. They name the step actually
/// under way, so the wait reads as work rather than as a hang.
const THINKING: &[&str] =
    &["reading the chain", "pulling the pools", "analysing the move", "thinking it through"];

/// How many launches the board shows. Dexscreener returns 30 pairs per quote
/// token and most of the tail is dust, so the board keeps the busy end.
const TRENDING_ROWS: usize = 14;

/// The brief for the chart read.
///
/// Deliberately barred from advising. This panel shows a live market inside a
/// tool the author also holds a token on, and a model that says "buy" there is
/// a liability dressed up as a feature. Describing what the numbers do is
/// genuinely useful and carries none of that.
const TAKE_SYSTEM: &str = "\
You are reading one token's live trading data from a DEX indexer on Robinhood Chain.

Reply with two or three short sentences on what the numbers show: how the move looks \
across the 5m/1h/6h/24h windows, whether volume and the buy/sell split back it up, how \
thin liquidity is next to that volume, and how young the pool is.

Cite the actual figures you are reading. Be plain and concrete.

Describe only. Never advise, never predict, never state what someone should do. Do not \
use the words buy, sell, should, will, pump, moon, or safe.";

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

// Launch board. Prices here run to eight significant figures, so its price
// column is wider than the market table's.
const T_RANK: f32 = 30.0;
const T_SYM: f32 = 108.0;
const T_DEX: f32 = 86.0;
const T_PRICE: f32 = 120.0;
const T_CHG: f32 = 100.0;
const T_VOL: f32 = 112.0;
const T_LIQ: f32 = 108.0;
const T_AGE: f32 = 66.0;
const T_TABLE_W: f32 = T_RANK + T_SYM + T_DEX + T_PRICE + T_CHG + T_VOL + T_LIQ + T_AGE;

enum Outcome {
    Price { chain: &'static str, symbol: String, address: String, usd: f64 },
    /// A ticker looked up on the chain, with the agent's read of it. The take
    /// is carried alongside the numbers rather than fetched separately so the
    /// card can never show a comment about figures it is no longer displaying.
    Token { info: Box<trending::Info>, take: String },
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
    ticker: String,
    sushi_key: String,
    key_open: bool,
    loaded: bool,
    /// Saved value, mirrors `config_dir/anthropic.key`.
    ai_key: String,
    /// Edit buffer, kept apart from `ai_key` so the setup block does not
    /// vanish on the first keystroke.
    ai_input: String,
    rx: Option<Receiver<Result<Outcome, String>>>,
    /// When the in-flight request started, so the thinking line can advance
    /// through its phrases instead of sitting on one.
    busy_since: Option<Instant>,
    result: Option<Result<Outcome, String>>,
    log: Vec<LogLine>,
    market: Vec<market::Row>,
    market_rx: Option<Receiver<Result<Vec<market::Row>, String>>>,
    market_err: Option<String>,
    market_at: Option<Instant>,
    market_sort: market::Sort,
    market_limit: usize,
    trending: Vec<trending::Row>,
    trending_rx: Option<Receiver<Result<Vec<trending::Row>, String>>>,
    trending_err: Option<String>,
    trending_at: Option<Instant>,
}

impl Default for SushiTool {
    fn default() -> Self {
        Self {
            ask: String::new(),
            ticker: String::new(),
            sushi_key: String::new(),
            key_open: false,
            loaded: false,
            ai_key: String::new(),
            ai_input: String::new(),
            rx: None,
            busy_since: None,
            result: None,
            log: Vec::new(),
            market: Vec::new(),
            market_rx: None,
            market_err: None,
            market_at: None,
            market_sort: market::Sort::Volume,
            market_limit: MARKET_ROWS,
            trending: Vec::new(),
            trending_rx: None,
            trending_err: None,
            trending_at: None,
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
        self.busy_since = Some(Instant::now());
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

    fn refresh_trending(&mut self, ctx: &egui::Context) {
        if self.trending_rx.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(trending::fetch(TRENDING_ROWS));
        });
        self.trending_rx = Some(rx);
        ctx.request_repaint();
    }

    /// Ticker in, numbers and a reading out.
    ///
    /// Both halves happen on the worker thread: the indexer call, then the
    /// model call fed with the figures that call returned. Without a key the
    /// card still comes back — the data is the useful part, the take is the
    /// garnish.
    fn look_up(&mut self, ctx: &egui::Context) {
        let ticker = self.ticker.trim().to_string();
        if ticker.is_empty() {
            return;
        }
        let ai_key = self.ai_key.clone();
        self.spawn(ctx, move || {
            let info = trending::lookup(&ticker)?;
            let take = if ai_key.trim().is_empty() {
                String::new()
            } else {
                use crate::llm::LlmProvider;
                // A failed take must not lose the numbers, so the error is
                // shown in place of the comment rather than raised.
                crate::llm::Anthropic::new(ai_key)
                    .complete(TAKE_SYSTEM, &info.brief())
                    .unwrap_or_else(|e| format!("(no read: {e})"))
            };
            Ok(Outcome::Token { info: Box::new(info), take })
        });
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
            Ok(Outcome::Token { info, .. }) => {
                (format!("read {} on Robinhood Chain", info.symbol), true)
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
        if let Some(rx) = &self.trending_rx {
            match rx.try_recv() {
                Ok(Ok(rows)) => {
                    self.trending = rows;
                    self.trending_err = None;
                    self.trending_at = Some(Instant::now());
                    self.trending_rx = None;
                }
                Ok(Err(e)) => {
                    self.trending_err = Some(e);
                    self.trending_at = Some(Instant::now());
                    self.trending_rx = None;
                }
                Err(TryRecvError::Empty) => ui.ctx().request_repaint(),
                Err(TryRecvError::Disconnected) => self.trending_rx = None,
            }
        }
    }

    /// What is launching and trading on Robinhood Chain, busiest first.
    fn trending_section(&mut self, ui: &mut egui::Ui) {
        let loading = self.trending_rx.is_some();

        ui.horizontal(|ui| {
            ui.label(RichText::new("TRADING ON ROBINHOOD CHAIN").color(ORANGE).small().strong());
            ui.label(RichText::new("every DEX · by 24h volume").color(FAINT).small());
            if loading {
                ui.add(egui::Spinner::new().size(13.0).color(ORANGE));
            } else if let Some(at) = self.trending_at {
                ui.label(RichText::new(ago(at.elapsed().as_secs())).color(FAINT).small());
            }
            if tool_button(ui, "Refresh", !loading) && !loading {
                let ctx = ui.ctx().clone();
                self.refresh_trending(&ctx);
            }
        });
        ui.add_space(2.0);
        ui.label(
            RichText::new(
                "Straight off the chain's own pools — a token shows up here minutes after \
                 someone launches it. DEX names which AMM the pool actually runs on: most of \
                 this chain trades on Uniswap, and a Sushi-labelled row is called out below.",
            )
            .color(DIM)
            .small(),
        );
        ui.add_space(8.0);

        if let Some(e) = &self.trending_err {
            ui.label(RichText::new(format!("✗ {e}")).color(RED).small());
            return;
        }
        if self.trending.is_empty() {
            ui.label(RichText::new("loading…").color(FAINT).small());
            return;
        }

        trending_header(ui);
        let mut picked = None;
        for (i, row) in self.trending.iter().enumerate() {
            if trending_row(ui, i, row) {
                picked = Some(row.symbol.clone());
            }
        }
        if let Some(sym) = picked {
            self.ticker = sym;
            let ctx = ui.ctx().clone();
            self.look_up(&ctx);
        }
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
            self.refresh_trending(&ctx);
        }

        self.poll(ui);

        // Keep the table warm without the user asking.
        let stale = self.market_at.map(|t| t.elapsed() >= MARKET_TTL).unwrap_or(false);
        if stale && self.market_rx.is_none() {
            let ctx = ui.ctx().clone();
            self.refresh_market(&ctx);
        }
        let eq_stale = self.trending_at.map(|t| t.elapsed() >= MARKET_TTL).unwrap_or(false);
        if eq_stale && self.trending_rx.is_none() {
            let ctx = ui.ctx().clone();
            self.refresh_trending(&ctx);
        }
        if self.market_at.is_some() || self.trending_at.is_some() {
            ui.ctx().request_repaint_after(Duration::from_secs(1));
        }

        // The agent leads. Everything below it is the reference material you
        // consult after asking, not before.
        egui::ScrollArea::vertical().show(ui, |ui| {
            self.agent_section(ui, octx);

            ui.add_space(18.0);
            ui.separator();
            ui.add_space(14.0);

            self.trending_section(ui);

            ui.add_space(18.0);
            ui.separator();
            ui.add_space(14.0);

            self.market_section(ui);
        });
    }
}

impl SushiTool {
    fn agent_section(&mut self, ui: &mut egui::Ui, octx: &ToolCtx) {
        let busy = self.rx.is_some();

        let t = ui.input(|i| i.time);
        let has_ai = !self.ai_key.trim().is_empty();

        // The agent gets a surface of its own. Everything else on this page is
        // a table on the panel background; giving this block a card, a border
        // and a running light is what makes it read as the tool rather than as
        // one control among several.
        let card = egui::Frame::NONE
            .fill(BG_ELEVATED)
            .stroke(egui::Stroke::new(1.0_f32, ACCENT.gamma_multiply(0.35)))
            .corner_radius(8.0)
            .inner_margin(egui::Margin::symmetric(18, 16));

        card.show(ui, |ui| {
            ui.horizontal(|ui| {
                // Idle it breathes; working it runs. The panel should look
                // awake before you have typed anything into it.
                let phase = if busy { t * 5.0 } else { t * 1.6 };
                let pulse = 0.5 + 0.5 * (phase.sin() as f32);
                let (dot, _) =
                    ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
                let p = ui.painter();
                // A halo that swells with the pulse, so the movement is visible
                // at a glance rather than only on close inspection.
                p.circle_filled(dot.center(), 4.0 + 3.5 * pulse, ACCENT.gamma_multiply(0.18));
                p.circle_filled(dot.center(), 4.0, ACCENT.gamma_multiply(0.45 + 0.55 * pulse));

                ui.label(
                    RichText::new("AI AGENT")
                        .color(ACCENT)
                        .font(FontId::proportional(17.0))
                        .strong(),
                );
                ui.label(
                    RichText::new(if busy { "working" } else { "ready" }).color(FAINT).small(),
                );
            });
            ui.add_space(3.0);
            ui.label(
                RichText::new(
                    "Ask it anything about the chain, or drop a ticker and it reads the chart. \
                     It only ever looks — it cannot spend.",
                )
                .color(DIM)
                .small(),
            );
            ui.add_space(12.0);

            // Ticker lookup first: it is the shortest path from "what is this
            // thing scrolling past" to an answer, and it needs no grammar.
            ui.horizontal(|ui| {
                ui.label(RichText::new("$").color(ACCENT).font(FontId::monospace(15.0)));
                let field = ui.add_enabled(
                    !busy,
                    egui::TextEdit::singleline(&mut self.ticker)
                        .hint_text("ticker — try PONS, SWOGE, r0b")
                        .font(FontId::monospace(14.0))
                        .desired_width(200.0),
                );
                let entered =
                    field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if (tool_button(ui, "Read the chart", !busy) || entered) && !busy {
                    let ctx = ui.ctx().clone();
                    self.look_up(&ctx);
                }
            });
            ui.add_space(10.0);

            ui.horizontal(|ui| {
                let field = ui.add_enabled(
                    has_ai && !busy,
                    egui::TextEdit::singleline(&mut self.ask)
                        .hint_text("or ask — how much is 1 WETH in USDG?")
                        // Capped: the panel can be 1900px wide, and a field
                        // stretched that far pushes Ask off to the far edge.
                        .desired_width((ui.available_width() - 90.0).min(420.0)),
                );
                let entered =
                    field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if (tool_button(ui, "Ask", has_ai && !busy) || entered) && has_ai && !busy {
                    let ctx = ui.ctx().clone();
                    self.ask_model(&ctx);
                }
            });

            // Starters, because the hardest part of a blank box is the first
            // question. Clicking one asks it outright rather than only filling
            // the field — a starter you still have to submit is placeholder
            // text with extra steps.
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("try").color(FAINT).small());
                for q in EXAMPLES {
                    if tool_button(ui, q, has_ai && !busy) && has_ai && !busy {
                        self.ask = (*q).to_string();
                        let ctx = ui.ctx().clone();
                        self.ask_model(&ctx);
                    }
                }
            });
        });

        if !has_ai {
            self.ai_key_setup(ui, octx);
        }
        ui.add_space(14.0);

        // While a request is out, the thinking line replaces the answer rather
        // than sitting beside a stale one — the panel should never show a fresh
        // spinner above an old number and let you mistake it for the new one.
        if busy {
            thinking_line(ui, self.busy_since, t);
        } else {
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
/// The wait, narrated. The phrase advances with elapsed time and the dots keep
/// moving, so a slow pool reads as work in progress rather than a frozen panel.
/// Past three seconds it also shows the count, which is the point where silence
/// starts to feel like something went wrong.
fn thinking_line(ui: &mut egui::Ui, since: Option<Instant>, t: f64) {
    let elapsed = since.map(|s| s.elapsed().as_secs_f64()).unwrap_or(0.0);
    let step = ((elapsed / 1.4) as usize).min(THINKING.len() - 1);
    let dots = ".".repeat(1 + (t * 3.0) as usize % 3);

    ui.horizontal(|ui| {
        ui.add(egui::Spinner::new().size(15.0).color(ACCENT));
        ui.label(RichText::new(format!("{}{dots}", THINKING[step])).color(FG));
        if elapsed >= 3.0 {
            ui.label(RichText::new(format!("{elapsed:.0}s")).color(FAINT).small());
        }
    });
}

/// The one honest flourish on the board: a Sushi pool gets the tool's own
/// accent so it stands out from the Uniswap rows around it, since that
/// contrast is the whole point of carrying `dex_id` at all.
fn dex_badge(p: &egui::Painter, pos: egui::Pos2, dex_id: &str) {
    let sushi = trending::is_sushi(dex_id);
    let label = if sushi { "Sushi" } else { dex_id };
    let color = if sushi { ACCENT } else { DIM };
    let font = FontId::proportional(11.5);
    if sushi {
        let size = p.layout_no_wrap(label.to_string(), font.clone(), color).size();
        let pad = egui::vec2(5.0, 2.0);
        let rect = egui::Rect::from_min_size(
            pos - egui::vec2(pad.x, size.y / 2.0 + pad.y),
            size + pad * 2.0,
        );
        p.rect_filled(rect, 3.0, ACCENT.gamma_multiply(0.16));
    }
    p.text(pos, egui::Align2::LEFT_CENTER, truncate(label, 11), font, color);
}

fn trending_header(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(T_TABLE_W, 20.0), egui::Sense::hover());
    let p = ui.painter();
    let cy = rect.center().y;
    let f = FontId::proportional(10.5);
    let mut x = rect.left() + 8.0;

    x += T_RANK;
    p.text(egui::pos2(x, cy), egui::Align2::LEFT_CENTER, "TOKEN", f.clone(), FAINT);
    x += T_SYM;
    p.text(egui::pos2(x, cy), egui::Align2::LEFT_CENTER, "DEX", f.clone(), FAINT);
    x += T_DEX + T_PRICE;
    p.text(egui::pos2(x, cy), egui::Align2::RIGHT_CENTER, "PRICE", f.clone(), FAINT);
    x += T_CHG;
    p.text(egui::pos2(x, cy), egui::Align2::RIGHT_CENTER, "24H", f.clone(), FAINT);
    x += T_VOL;
    p.text(egui::pos2(x, cy), egui::Align2::RIGHT_CENTER, "VOLUME 24H", f.clone(), FAINT);
    x += T_LIQ;
    p.text(egui::pos2(x, cy), egui::Align2::RIGHT_CENTER, "LIQUIDITY", f.clone(), FAINT);
    x += T_AGE;
    p.text(egui::pos2(x, cy), egui::Align2::RIGHT_CENTER, "AGE", f, FAINT);
}

/// Returns true when the row was clicked, which reads that token in the agent
/// card above — the board is the list of things worth asking about, so pointing
/// at one should be the whole gesture.
fn trending_row(ui: &mut egui::Ui, i: usize, r: &trending::Row) -> bool {
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(T_TABLE_W, 26.0), egui::Sense::click());
    if !ui.is_rect_visible(rect) {
        return false;
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
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
    x += T_RANK;

    p.text(
        egui::pos2(x, cy),
        egui::Align2::LEFT_CENTER,
        truncate(&r.symbol, 12),
        FontId::proportional(13.5),
        FG,
    );

    x += T_SYM;
    dex_badge(p, egui::pos2(x, cy), &r.dex_id);

    x += T_DEX + T_PRICE;
    p.text(
        egui::pos2(x, cy),
        egui::Align2::RIGHT_CENTER,
        market::money_price(r.price_usd),
        num.clone(),
        FG,
    );

    x += T_CHG;
    // Sign as well as colour: the sign is what survives colour-blindness, and
    // on this board the swings are the whole story.
    let (chg_text, chg_color) = match r.change_h24 {
        Some(c) => (market::pct_signed(c), if c < 0.0 { RED } else { UP }),
        None => ("—".to_string(), FAINT),
    };
    p.text(egui::pos2(x, cy), egui::Align2::RIGHT_CENTER, chg_text, num.clone(), chg_color);

    x += T_VOL;
    p.text(
        egui::pos2(x, cy),
        egui::Align2::RIGHT_CENTER,
        market::money_compact(r.volume_h24),
        num.clone(),
        DIM,
    );

    x += T_LIQ;
    p.text(
        egui::pos2(x, cy),
        egui::Align2::RIGHT_CENTER,
        market::money_compact(r.liquidity_usd),
        num.clone(),
        DIM,
    );

    x += T_AGE;
    // Anything under a day is the reason to look twice, so it gets the accent.
    let (age_text, age_color) = match r.age_hours() {
        Some(h) if h < 24.0 => (format!("{h:.0}h"), ORANGE),
        Some(h) if h < 48.0 => (format!("{h:.0}h"), DIM),
        Some(h) => (format!("{:.0}d", h / 24.0), DIM),
        None => ("—".to_string(), FAINT),
    };
    p.text(egui::pos2(x, cy), egui::Align2::RIGHT_CENTER, age_text, num, age_color);
    resp.clicked()
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

/// One token, read across every window the indexer gives.
///
/// The four percentages sit side by side on purpose: a token up on the day and
/// down on the hour is a different story from one up on both, and that contrast
/// is invisible when the windows are shown one at a time.
fn token_card(ui: &mut egui::Ui, info: &trending::Info, take: &str) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(&info.symbol).color(FG).font(FontId::proportional(20.0)).strong(),
        );
        if !info.name.is_empty() && !info.name.eq_ignore_ascii_case(&info.symbol) {
            ui.label(RichText::new(&info.name).color(DIM).small());
        }
        if let Some(h) = info.age_hours() {
            let age = if h < 48.0 {
                format!("{h:.0}h old")
            } else {
                format!("{:.0}d old", h / 24.0)
            };
            ui.label(RichText::new(age).color(if h < 24.0 { ORANGE } else { FAINT }).small());
        }
        // Which AMM this pool actually runs on — most of this chain is
        // Uniswap, so this is the line that keeps the tool honest about what
        // it is showing you.
        let on_sushi = trending::is_sushi(&info.dex_id);
        ui.label(
            RichText::new(if on_sushi { "on Sushi".to_string() } else { info.dex_id.clone() })
                .color(if on_sushi { ACCENT } else { FAINT })
                .small(),
        );
    });
    ui.add_space(4.0);
    ui.label(
        RichText::new(market::money_price(info.price_usd))
            .color(ACCENT)
            .font(FontId::monospace(24.0)),
    );
    ui.add_space(8.0);

    ui.horizontal(|ui| {
        for (label, v) in [
            ("5m", info.change_m5),
            ("1h", info.change_h1),
            ("6h", info.change_h6),
            ("24h", info.change_h24),
        ] {
            ui.vertical(|ui| {
                ui.label(RichText::new(label).color(FAINT).small());
                let (text, color) = match v {
                    Some(c) => (market::pct_signed(c), if c < 0.0 { RED } else { UP }),
                    None => ("—".to_string(), FAINT),
                };
                ui.label(RichText::new(text).color(color).font(FontId::monospace(13.0)));
            });
            ui.add_space(14.0);
        }
    });
    ui.add_space(8.0);

    ui.horizontal_wrapped(|ui| {
        let stat = |ui: &mut egui::Ui, k: &str, v: String| {
            ui.label(RichText::new(k).color(FAINT).small());
            ui.label(RichText::new(v).color(DIM).font(FontId::monospace(12.0)));
            ui.add_space(12.0);
        };
        stat(ui, "vol 24h", market::money_compact(info.volume_h24));
        stat(ui, "vol 1h", market::money_compact(info.volume_h1));
        stat(ui, "liquidity", market::money_compact(info.liquidity_usd));
        if let Some(mc) = info.market_cap {
            stat(ui, "mcap", market::money_compact(mc));
        }
        stat(ui, "buys/sells", format!("{}/{}", info.buys_h24, info.sells_h24));
    });

    if !take.is_empty() {
        ui.add_space(12.0);
        ui.separator();
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("◆").color(ACCENT).small());
            ui.label(RichText::new("THE AGENT READS IT").color(ACCENT).small().strong());
        });
        ui.add_space(6.0);
        ui.label(RichText::new(take).color(FG));
        ui.add_space(6.0);
        ui.label(
            RichText::new("A description of the numbers above. Not advice.")
                .color(FAINT)
                .small(),
        );
    }

    ui.add_space(10.0);
    ui.horizontal(|ui| {
        // The contract, in full. On a chain where a dozen tokens can share a
        // ticker, this is the only line that identifies which one you are
        // looking at.
        ui.label(
            RichText::new(&info.address).color(FAINT).font(FontId::monospace(11.0)),
        );
        if !info.url.is_empty() {
            ui.hyperlink_to(
                RichText::new("Dexscreener ↗").color(FAINT).small(),
                &info.url,
            );
        }
    });
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
            Outcome::Token { info, take } => token_card(ui, info, take),
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
