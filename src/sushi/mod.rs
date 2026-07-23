//! Sushi agent — read-only.
//!
//! The agent leads: you ask in plain English and it answers. Below it sit the
//! two boards it draws on — what is launching on Robinhood Chain right now, and
//! what is moving across the wider market. The model only ever parses intent;
//! resolution, arithmetic and API calls happen in Rust. Nothing here can sign
//! or broadcast a transaction.

mod api;
mod erc20;
mod trending;
mod intent;
mod market;
mod tokens;
mod wallet;

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
const EXAMPLES: &[&str] =
    &["price of WETH", "1 WETH in USDG", "what's pumping", "what's got good volume"];

/// Phrases cycled while a request is in flight. They name the step actually
/// under way, so the wait reads as work rather than as a hang.
const THINKING: &[&str] =
    &["reading the chain", "pulling the pools", "analysing the move", "thinking it through"];

/// How many launches the board shows. Dexscreener returns 30 pairs per quote
/// token and most of the tail is dust, so the board keeps the busy end.
const TRENDING_ROWS: usize = 14;

/// The brief for the chart read.
///
/// Voice is degen on purpose — this is a shitcoin board, and a stiff analyst
/// tone reads false on it. What's still barred, deliberately: recommending a
/// token. This panel shows a live market inside a tool the author also holds
/// a token on, and a model that tells someone to buy is a liability dressed
/// up as a feature — that line doesn't move for tone.
const TAKE_SYSTEM: &str = "\
You are reading one token's live trading data from a DEX indexer on Robinhood Chain. \
This is a degen shitcoin board, not a research desk — talk like it. Blunt, funny, a \
little unhinged, crypto-twitter vernacular is fine. You are not a financial analyst \
and should not sound like one.

Reply with two or three short sentences reacting to what the numbers show: how the move \
looks across the 5m/1h/6h/24h windows, whether volume and the buy/sell split back it up, \
how thin liquidity is next to that volume, and how young the pool is. Have an opinion on \
whether the chart looks unhinged, exhausted, suspicious, or boring — that's the fun part.

Cite the actual figures you are reading. Personality doesn't excuse making numbers up.

Never tell anyone to buy or sell this or any other token, never say a price will go up \
or down, never call anything safe. You can roast a chart; you cannot recommend one.";

/// Same voice and the same hard line as `TAKE_SYSTEM`, aimed at a list
/// instead of one token — this is the one place in the whole tool that could
/// most easily slide into "buy this one", so the rule is repeated rather than
/// assumed to carry over.
const SCREEN_SYSTEM: &str = "\
You are looking at a list of tokens currently trading on Robinhood Chain, ranked by \
24h volume. For each: price, 1h and 24h price change, 1h/6h/24h volume, 24h buy/sell \
counts, market cap, pool age, and which DEX. Same board a screener would show. Same \
degen crypto-twitter voice as anywhere else on this thing — not a research desk.

Reply with two or three short sentences reacting to the *set*, not to one token. Use \
the windows to say something the 24h number alone can't: a token with big 24h volume \
but almost nothing in the 1h column has gone quiet; one where 1h volume is a large \
share of 24h is heating up right now. A buy/sell split that's lopsided vs one that's \
even tells a different story at the same volume. Also worth a line: a brand-new pool \
already doing real volume, several names that look like the same meme cycling, how \
much of this is Uniswap vs Sushi. Cite real figures from the list.

Never say which one to buy, never rank them by which is the better trade, never call \
anything safe. Describe the board. Do not pick a winner.";

const DEFAULT_SLIPPAGE: f64 = 0.005;
/// How long the amount field must sit still before a preview quote fires —
/// long enough that typing "0.1" doesn't fire three requests for "0", "0.",
/// "0.1".
const PREVIEW_DEBOUNCE: Duration = Duration::from_millis(450);
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
// Two volume windows rather than one: 24h alone can't tell a token that's
// gone quiet from one heating up right now, and that distinction is the
// whole point of a screener over a static leaderboard.
const T_VOL1H: f32 = 100.0;
const T_VOL: f32 = 112.0;
const T_LIQ: f32 = 108.0;
const T_AGE: f32 = 66.0;
const T_TABLE_W: f32 =
    T_RANK + T_SYM + T_DEX + T_PRICE + T_CHG + T_VOL1H + T_VOL + T_LIQ + T_AGE;

enum Outcome {
    Price { chain: &'static str, symbol: String, address: String, usd: f64 },
    /// A ticker looked up on the chain, with the agent's read of it. The take
    /// is carried alongside the numbers rather than fetched separately so the
    /// card can never show a comment about figures it is no longer displaying.
    Token { info: Box<trending::Info>, take: String },
    Quote { chain: &'static str, quote: api::Quote },
    /// Feeds the table rather than the result card.
    Market { rows: Vec<market::Row>, sort: market::Sort, limit: usize },
    /// A chat-requested read of the Robinhood Chain board — same data as the
    /// always-visible table, plus a take on the set as a whole.
    Screen { rows: Vec<trending::Row>, take: String },
}

/// What a completed swap actually did, re-read from the chain side rather
/// than assumed from the request — `sent`/`got_symbol` describe the leg that
/// was signed, not the one that was asked for.
struct SwapDone {
    tx_hash: String,
    sent: String,
    got_symbol: String,
    /// What the route promised before it was sent, not what actually landed —
    /// this build has no way to read the receipt's real output amount without
    /// decoding event logs, so it shows the figure it can stand behind rather
    /// than a real one it would have to guess at.
    expected_out: String,
    price_impact: Option<f64>,
}

/// Every step a swap passes through, in order. Threaded back to the UI via
/// `Arc<Mutex<SwapStep>>` rather than the channel, because the channel only
/// carries a final result and this needs to update mid-flight, while Frame's
/// window has the user's attention.
#[derive(Clone, PartialEq)]
enum SwapStep {
    Idle,
    Quoting,
    CheckingAllowance,
    /// Frame is open waiting on the user; this is the step that can least
    /// afford to look like the app has frozen.
    WaitingOnApproval,
    ConfirmingApproval,
    WaitingOnSwap,
}

impl SwapStep {
    fn label(&self) -> &'static str {
        match self {
            Self::Idle => "",
            Self::Quoting => "asking Sushi for a route",
            Self::CheckingAllowance => "checking allowance",
            Self::WaitingOnApproval => "waiting for approval in Frame",
            Self::ConfirmingApproval => "confirming the approval on-chain",
            Self::WaitingOnSwap => "waiting for the swap in Frame",
        }
    }
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

/// A written line for the turn, not just a data card underneath it — this is
/// the difference between a widget and a chat message. Built in Rust from the
/// numbers already fetched, never a separate model call: Price/Quote/Market
/// are simple enough to phrase deterministically, and doing it this way means
/// the sentence cannot possibly disagree with the card below it. Token and
/// Screen already carry a written line of their own — the model's take — so
/// this returns empty for both rather than saying the same thing twice.
fn chat_reply(outcome: &Outcome) -> String {
    match outcome {
        Outcome::Price { chain, symbol, usd, .. } => {
            format!("{symbol} is at {} on {chain}.", market::money_price(*usd))
        }
        Outcome::Quote { chain, quote } => {
            let (sent, got) = Outcome::legs(quote);
            match quote.status.as_str() {
                "Success" => format!(
                    "{sent} {} gets you about {got} {} on {chain}.",
                    quote.token_in.symbol, quote.token_out.symbol
                ),
                "Partial" => format!(
                    "Only a partial route for {sent} {} → {} on {chain} — about {got} of \
                     what was asked for.",
                    quote.token_in.symbol, quote.token_out.symbol
                ),
                other => format!("{other} on {chain} for that pair."),
            }
        }
        Outcome::Market { rows, sort, limit } => match rows.first() {
            Some(top) => format!(
                "Top {limit} by {} — {} leads at {}.",
                sort.label(),
                top.symbol,
                market::money_price(top.price)
            ),
            None => "Nothing came back for that.".to_string(),
        },
        Outcome::Token { .. } | Outcome::Screen { .. } => String::new(),
    }
}

struct LogLine {
    at: Instant,
    text: String,
    ok: bool,
}

enum ChatAnswer {
    Result(Outcome),
    Error(String),
}

struct ChatTurn {
    question: String,
    answer: ChatAnswer,
}

/// Compact recap of the last few turns, handed to the model ahead of a new
/// question so "the second one" or "and that one's volume" has something to
/// resolve against. Plain summaries, not the full card data — the model only
/// ever needs enough to disambiguate what's being asked about, and Rust
/// re-fetches real numbers for whatever it decides on regardless.
fn chat_recap(chat: &[ChatTurn], max_turns: usize) -> String {
    if chat.is_empty() {
        return String::new();
    }
    let mut out = String::from("Recent conversation, oldest first:\n");
    for turn in chat.iter().rev().take(max_turns).collect::<Vec<_>>().into_iter().rev() {
        let summary = match &turn.answer {
            ChatAnswer::Result(Outcome::Token { info, .. }) => {
                format!("looked up {} — ${}", info.symbol, info.price_usd)
            }
            ChatAnswer::Result(Outcome::Price { symbol, usd, .. }) => {
                format!("{symbol} price — ${usd}")
            }
            ChatAnswer::Result(Outcome::Quote { quote, .. }) => {
                format!("quoted {} to {}", quote.token_in.symbol, quote.token_out.symbol)
            }
            ChatAnswer::Result(Outcome::Screen { rows, .. }) => {
                let names: Vec<&str> = rows.iter().take(5).map(|r| r.symbol.as_str()).collect();
                format!("screened Robinhood Chain — top: {}", names.join(", "))
            }
            ChatAnswer::Result(Outcome::Market { rows, .. }) => {
                format!("showed the wider market, {} rows", rows.len())
            }
            ChatAnswer::Error(e) => format!("error — {e}"),
        };
        out.push_str(&format!("Q: {}\nA: {summary}\n", turn.question));
    }
    out
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
    /// The question a spawned job is answering, held here rather than read
    /// back from the text field — the user may already be typing the next
    /// one by the time this one resolves.
    pending_question: Option<String>,
    /// The conversation, oldest first. Both doors onto the agent — the
    /// ticker box and the ask box — land here, so "the second one" in a
    /// follow-up has something to mean.
    chat: Vec<ChatTurn>,
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

    /// Frame's connected account. `None` covers both "never asked" and
    /// "Frame isn't running" — the swap button's own click handler is what
    /// distinguishes them, by trying to connect.
    wallet: Option<String>,
    wallet_connecting: bool,
    wallet_err: Option<String>,
    wallet_rx: Option<Receiver<Result<String, String>>>,
    /// How much of the base token (WETH) to put into the swap, as the user
    /// typed it — parsed against WETH's decimals only when the button is
    /// pressed, same as the quote box's amount field.
    swap_amount: String,
    swap_rx: Option<Receiver<Result<SwapDone, String>>>,
    swap_result: Option<Result<SwapDone, String>>,
    /// Updated from the worker thread mid-flight; read by the UI thread every
    /// frame. Not a channel: a channel only delivers once the whole job ends,
    /// and this exists specifically to show what's happening before then.
    swap_step: std::sync::Arc<std::sync::Mutex<SwapStep>>,

    /// Live "what would I get" preview, shown above the Swap button before
    /// the user commits to anything — the route Sushi's aggregator would
    /// take, re-quoted whenever the amount settles.
    preview: Option<api::Quote>,
    /// `Some("")` means "asked, nothing to show" (empty/zero amount) — kept
    /// apart from a real error string, which the UI actually surfaces.
    preview_err: Option<String>,
    preview_rx: Option<Receiver<Result<api::Quote, String>>>,
    /// (token address, amount text) the current preview/err/in-flight fetch
    /// answers — compared each frame to know when it's gone stale.
    preview_key: Option<(String, String)>,
    /// When the amount field last settled on `preview_key`, so a fetch waits
    /// out the debounce before firing.
    preview_dirty_since: Option<Instant>,
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
            pending_question: None,
            chat: Vec::new(),
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

            wallet: None,
            wallet_connecting: false,
            wallet_err: None,
            wallet_rx: None,
            swap_amount: "0.01".into(),
            swap_rx: None,
            swap_result: None,
            swap_step: std::sync::Arc::new(std::sync::Mutex::new(SwapStep::Idle)),

            preview: None,
            preview_err: None,
            preview_rx: None,
            preview_key: None,
            preview_dirty_since: None,
        }
    }
}

fn sushi_key_path(cfg: &std::path::Path) -> std::path::PathBuf {
    cfg.join("sushi.key")
}

/// Flattens the board into the same fields it renders — symbol, price, 24h
/// change, volume, age, DEX — so the model reads exactly what the table below
/// its take will show, not a differently-shaped summary that could disagree
/// with it.
fn screen_brief(rows: &[trending::Row]) -> String {
    let mut out = String::from("Robinhood Chain, ranked by 24h volume:\n");
    for r in rows {
        let age = r
            .age_hours()
            .map(|h| if h < 48.0 { format!("{h:.0}h old") } else { format!("{:.0}d old", h / 24.0) })
            .unwrap_or_else(|| "age unknown".into());
        let pct = |c: Option<f64>| c.map(|c| format!("{c:+.1}%")).unwrap_or_else(|| "n/a".into());
        let mcap = r.market_cap.map(|m| format!("${m:.0}")).unwrap_or_else(|| "n/a".into());
        out.push_str(&format!(
            "- {} · ${} · chg 1h {} / 24h {} · vol 1h ${:.0} / 6h ${:.0} / 24h ${:.0} · \
             buys/sells 24h {}/{} · mcap {} · {} · {}\n",
            r.symbol,
            r.price_usd,
            pct(r.change_h1),
            pct(r.change_h24),
            r.volume_h1,
            r.volume_h6,
            r.volume_h24,
            r.buys_h24,
            r.sells_h24,
            mcap,
            age,
            r.dex_id
        ));
    }
    out
}

/// Execute an already-resolved intent. Past this point nothing invents a
/// number — every branch either calls a real API or fails.
fn run(intent: Intent, api_key: &str, ai_key: &str) -> Result<Outcome, String> {
    match intent {
        Intent::TokenLookup { ticker } => {
            let info = trending::lookup(&ticker)?;
            let take = if ai_key.trim().is_empty() {
                String::new()
            } else {
                use crate::llm::LlmProvider;
                crate::llm::Anthropic::new(ai_key.to_string())
                    .complete(TAKE_SYSTEM, &info.brief())
                    .unwrap_or_else(|e| format!("(no read: {e})"))
            };
            Ok(Outcome::Token { info: Box::new(info), take })
        }
        Intent::Market { sort, limit } => {
            Ok(Outcome::Market { rows: market::fetch(sort, limit)?, sort, limit })
        }
        Intent::ChainScreen { limit } => {
            let rows = trending::fetch(limit)?;
            let take = if ai_key.trim().is_empty() {
                String::new()
            } else {
                use crate::llm::LlmProvider;
                crate::llm::Anthropic::new(ai_key.to_string())
                    .complete(SCREEN_SYSTEM, &screen_brief(&rows))
                    .unwrap_or_else(|e| format!("(no read: {e})"))
            };
            Ok(Outcome::Screen { rows, take })
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

/// Approvals typically land in one or two blocks; five minutes is generous
/// headroom for a slow one without leaving the UI waiting indefinitely on a
/// stuck one.
const APPROVAL_TIMEOUT: Duration = Duration::from_secs(300);
/// "Infinite" approval — the standard practice so the same token doesn't ask
/// to be approved again on the next swap. The user sees exactly this amount
/// named in Frame's own confirmation before it's signed.
const MAX_APPROVAL: u128 = u128::MAX;

/// The whole swap sequence, off the UI thread. Every step that could fail is
/// reported through `Result`; every step that needs the user is a call into
/// `wallet::`, which is the only place a signature can happen.
#[allow(clippy::too_many_arguments)]
fn run_swap_job(
    chain_id: u64,
    token_in: &str,
    token_out: &str,
    amount_in: u128,
    sender: &str,
    api_key: &str,
    symbol: String,
    set_step: impl Fn(SwapStep),
) -> Result<SwapDone, String> {
    set_step(SwapStep::Quoting);
    let swap = api::swap(
        &api::SwapRequest {
            chain_id,
            token_in: token_in.to_string(),
            token_out: token_out.to_string(),
            amount: amount_in,
            max_slippage: DEFAULT_SLIPPAGE,
            sender: sender.to_string(),
        },
        api_key,
    )?;
    if swap.status == "Partial" {
        // Not fatal — Sushi found a route that fills less than the full
        // amount — but worth surfacing rather than silently swapping less
        // than the user typed.
    }

    // The native token needs no allowance — there is nothing to approve
    // spending, since it moves as `value`, not as an ERC-20 transfer.
    let is_native = token_in.eq_ignore_ascii_case(tokens::NATIVE);
    if !is_native {
        set_step(SwapStep::CheckingAllowance);
        let call_data = erc20::encode_allowance(sender, &swap.tx.to)?;
        let raw = wallet::call(chain_id, token_in, &call_data)?;
        let current = erc20::decode_uint256(&raw);

        if current < amount_in {
            set_step(SwapStep::WaitingOnApproval);
            let approve_data = erc20::encode_approve(&swap.tx.to, MAX_APPROVAL)?;
            let approve_hash = wallet::send_transaction(&wallet::TxRequest {
                chain_id,
                from: sender.to_string(),
                to: token_in.to_string(),
                data: approve_data,
                value: 0,
            })?;
            set_step(SwapStep::ConfirmingApproval);
            wallet::wait_for_receipt(chain_id, &approve_hash, APPROVAL_TIMEOUT)?;
        }
    }

    set_step(SwapStep::WaitingOnSwap);
    let tx_hash = wallet::send_transaction(&wallet::TxRequest {
        chain_id,
        from: sender.to_string(),
        to: swap.tx.to.clone(),
        data: swap.tx.data.clone(),
        value: swap.tx.value,
    })?;

    Ok(SwapDone {
        tx_hash,
        sent: tokens::format_units(swap.amount_in, swap.token_in.decimals),
        expected_out: tokens::format_units(swap.amount_out, swap.token_out.decimals),
        price_impact: swap.price_impact,
        got_symbol: if symbol.is_empty() { swap.token_out.symbol } else { symbol },
    })
}

impl SushiTool {
    fn spawn(
        &mut self,
        ctx: &egui::Context,
        question: String,
        job: impl FnOnce() -> Result<Outcome, String> + Send + 'static,
    ) {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(job());
        });
        self.rx = Some(rx);
        self.busy_since = Some(Instant::now());
        self.pending_question = Some(question);
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

    /// Asks Frame for the connected account. Runs on a worker thread since
    /// Frame may prompt the user and the reply can take a while — the same
    /// reason every other network call in this file is threaded. Its own
    /// channel rather than reusing `self.rx`: connecting is not an `Outcome`,
    /// and this way a wallet error can never land in a ticker lookup's card.
    fn connect_wallet(&mut self, ctx: &egui::Context) {
        if self.wallet_connecting {
            return;
        }
        self.wallet_connecting = true;
        self.wallet_err = None;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(wallet::accounts());
        });
        self.wallet_rx = Some(rx);
        ctx.request_repaint();
    }

    /// The whole swap, start to finish, on one worker thread: quote, allowance
    /// check, an approval if one is needed (with a wait for it to actually be
    /// mined before the next step trusts it), then the swap itself. Every
    /// signature happens inside Frame's own window — this thread only ever
    /// asks, it never holds a key.
    fn run_swap(&mut self, symbol: String, token_out_address: String, ctx: &egui::Context) {
        if self.swap_rx.is_some() {
            return;
        }
        let Some(sender) = self.wallet.clone() else { return };
        let Some(robinhood) = tokens::chain_by_name(trending::CHAIN_NAME) else { return };
        let Some(weth) = robinhood.token("WETH") else { return };
        let amount_raw = match tokens::parse_units(self.swap_amount.trim(), weth.decimals) {
            Ok(0) => {
                self.swap_result = Some(Err("amount is zero".into()));
                return;
            }
            Ok(v) => v,
            Err(e) => {
                self.swap_result = Some(Err(e));
                return;
            }
        };

        let chain_id = robinhood.id;
        let token_in = weth.address.to_string();
        let api_key = self.sushi_key.clone();
        let step = self.swap_step.clone();
        let set = move |s: SwapStep| *step.lock().unwrap() = s;

        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(run_swap_job(
                chain_id,
                &token_in,
                &token_out_address,
                amount_raw,
                &sender,
                &api_key,
                symbol,
                set,
            ));
        });
        self.swap_rx = Some(rx);
        self.swap_result = None;
        ctx.request_repaint();
    }

    /// Re-quotes the swap amount against Sushi's router a short beat after
    /// the field stops changing, so the amount/button row always has a real
    /// "what would I get" underneath it instead of asking the user to click
    /// Swap to find out. Keyed on (token, amount text): switching to a fresh
    /// lookup or editing the amount drops the stale answer immediately
    /// rather than showing yesterday's numbers under a new question.
    fn maybe_preview_quote(&mut self, token_out: &str, ctx: &egui::Context) {
        let amount = self.swap_amount.trim().to_string();
        let key = (token_out.to_string(), amount.clone());
        if self.preview_key.as_ref() != Some(&key) {
            self.preview_key = Some(key);
            self.preview = None;
            self.preview_err = None;
            self.preview_rx = None;
            self.preview_dirty_since = Some(Instant::now());
        }
        if self.preview_rx.is_some() || self.preview.is_some() || self.preview_err.is_some() {
            return;
        }
        let Some(since) = self.preview_dirty_since else { return };
        let elapsed = since.elapsed();
        if elapsed < PREVIEW_DEBOUNCE {
            ctx.request_repaint_after(PREVIEW_DEBOUNCE - elapsed);
            return;
        }

        let Some(robinhood) = tokens::chain_by_name(trending::CHAIN_NAME) else { return };
        let Some(weth) = robinhood.token("WETH") else { return };
        let amount_raw = match tokens::parse_units(&amount, weth.decimals) {
            Ok(v) if v > 0 => v,
            // Empty, zero or unparseable: nothing to preview. Stamped as a
            // silent "asked" marker so this stops re-parsing every frame
            // until the amount actually changes again.
            _ => {
                self.preview_err = Some(String::new());
                return;
            }
        };

        let chain_id = robinhood.id;
        let token_in = weth.address.to_string();
        let token_out = token_out.to_string();
        let api_key = self.sushi_key.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(api::quote(
                &api::QuoteRequest {
                    chain_id,
                    token_in,
                    token_out,
                    amount: amount_raw,
                    max_slippage: DEFAULT_SLIPPAGE,
                },
                &api_key,
            ));
        });
        self.preview_rx = Some(rx);
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
        self.ticker.clear();
        let ai_key = self.ai_key.clone();
        let api_key = self.sushi_key.clone();
        let label = format!("${ticker}");
        // Same intent the chat path reaches for the same question — the box
        // and a typed question are two doors onto one lookup, not two.
        self.spawn(ctx, label, move || {
            run(Intent::TokenLookup { ticker }, &api_key, &ai_key)
        });
    }

    /// Natural language path: model parses, Rust resolves and runs. Recent
    /// turns ride along as context, so a follow-up like "and the second one"
    /// has something to resolve against — the model only ever uses that to
    /// decide *what* to look up, never to invent an answer on its own.
    fn ask_model(&mut self, ctx: &egui::Context) {
        let question = self.ask.trim().to_string();
        if question.is_empty() {
            return;
        }
        self.ask.clear();
        let ai_key = self.ai_key.clone();
        let api_key = self.sushi_key.clone();
        let prompt = format!("{}New question: {question}", chat_recap(&self.chat, 3));
        self.spawn(ctx, question, move || {
            use crate::llm::LlmProvider;
            let raw = crate::llm::Anthropic::new(ai_key.clone())
                .complete(&intent::system_prompt(), &prompt)?;
            run(intent::parse(&raw)?, &api_key, &ai_key)
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
            Ok(Outcome::Screen { rows, .. }) => {
                (format!("screened {} Robinhood Chain tokens", rows.len()), true)
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
                    let question = self.pending_question.take().unwrap_or_default();
                    // Every intent becomes a turn, market included — a chat
                    // question should answer in the chat, not update a board
                    // somewhere else on the page and say nothing about it.
                    // The standalone MARKET section below still exists and
                    // still refreshes on its own; asking through chat no
                    // longer reaches into it.
                    match outcome {
                        Ok(o) => self.chat.push(ChatTurn { question, answer: ChatAnswer::Result(o) }),
                        Err(e) => self.chat.push(ChatTurn { question, answer: ChatAnswer::Error(e) }),
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
        if let Some(rx) = &self.wallet_rx {
            match rx.try_recv() {
                Ok(Ok(address)) => {
                    self.wallet = Some(address);
                    self.wallet_err = None;
                    self.wallet_connecting = false;
                    self.wallet_rx = None;
                }
                Ok(Err(e)) => {
                    self.wallet_err = Some(e);
                    self.wallet_connecting = false;
                    self.wallet_rx = None;
                }
                Err(TryRecvError::Empty) => ui.ctx().request_repaint(),
                Err(TryRecvError::Disconnected) => {
                    self.wallet_connecting = false;
                    self.wallet_rx = None;
                }
            }
        }
        if let Some(rx) = &self.swap_rx {
            match rx.try_recv() {
                Ok(result) => {
                    *self.swap_step.lock().unwrap() = SwapStep::Idle;
                    self.swap_result = Some(result);
                    self.swap_rx = None;
                }
                Err(TryRecvError::Empty) => ui.ctx().request_repaint(),
                Err(TryRecvError::Disconnected) => {
                    *self.swap_step.lock().unwrap() = SwapStep::Idle;
                    self.swap_rx = None;
                }
            }
        }
        if let Some(rx) = &self.preview_rx {
            match rx.try_recv() {
                Ok(Ok(q)) => {
                    self.preview = Some(q);
                    self.preview_err = None;
                    self.preview_rx = None;
                }
                Ok(Err(e)) => {
                    self.preview_err = Some(e);
                    self.preview = None;
                    self.preview_rx = None;
                }
                Err(TryRecvError::Empty) => ui.ctx().request_repaint(),
                Err(TryRecvError::Disconnected) => self.preview_rx = None,
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
        "Tell it what to swap — it prices the best route through Sushi and shows you before you sign."
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

        // The composer is pinned to the bottom of the panel, chat-app style —
        // it needs to be claimed before the scroll area below so it always
        // gets its strip regardless of how tall the conversation grows.
        egui::Panel::bottom("sushi_composer")
            .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(0, 10)))
            .show_separator_line(true)
            .show_inside(ui, |ui| {
                self.composer(ui, octx);
            });

        // The conversation leads. Everything below it is the reference
        // material you consult after asking, not before.
        egui::ScrollArea::vertical().show(ui, |ui| {
            self.chat_thread(ui);

            ui.add_space(14.0);
            self.log_and_settings(ui, octx);

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
    /// The conversation, oldest first, chat-app style: your line right and
    /// tinted, its line left in the panel's own card colour. Only the last
    /// turn — and only if it's a lookup — gets live swap controls; everything
    /// before it is a record of what was asked and found, not a second live
    /// surface.
    fn chat_thread(&mut self, ui: &mut egui::Ui) {
        let busy = self.rx.is_some();
        let t = ui.input(|i| i.time);

        if self.chat.is_empty() && !busy {
            ui.add_space(28.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new("FIND THE BEST SWAP")
                        .color(ACCENT)
                        .font(FontId::proportional(18.0))
                        .strong(),
                );
                ui.add_space(6.0);
                ui.label(
                    RichText::new(
                        "Say what you want to swap, or drop a ticker to have it read the chart. \
                         Every swap is priced through Sushi's own router first — you see the \
                         real route and the real number before anything moves. Nothing signs or \
                         sends on its own: you confirm every send yourself in Frame, and this \
                         thing never touches your keys.",
                    )
                    .color(DIM)
                    .small(),
                );
            });
            ui.add_space(28.0);
            return;
        }

        let mut swap_action = None;
        // Captured from the interactive turn during the loop below (borrowing
        // `self.chat` there rules out calling a `&mut self` method inline) —
        // the preview fetch itself runs once the loop's borrow is over.
        let mut preview_for: Option<String> = None;
        let last_idx = self.chat.len().saturating_sub(1);
        for (i, turn) in self.chat.iter().enumerate() {
            ui.add_space(if i == 0 { 4.0 } else { 18.0 });
            user_bubble(ui, &turn.question);
            ui.add_space(8.0);

            match &turn.answer {
                ChatAnswer::Result(outcome) => {
                    assistant_bubble(ui, |ui| {
                        // The written line first, chat-message style —
                        // Token/Screen's own take fills this role instead,
                        // so nothing doubles up.
                        let reply = chat_reply(outcome);
                        if !reply.is_empty() {
                            ui.label(RichText::new(reply).color(FG));
                            ui.add_space(6.0);
                        }
                        match outcome {
                            Outcome::Token { info, take } => {
                                let interactive = i == last_idx;
                                let step = self.swap_step.lock().unwrap().clone();
                                let a = token_card(
                                    ui,
                                    info,
                                    take,
                                    self.wallet.as_deref(),
                                    &mut self.swap_amount,
                                    self.swap_rx.is_some(),
                                    step.label(),
                                    &self.swap_result,
                                    self.preview.as_ref(),
                                    self.preview_err.as_deref().filter(|e| !e.is_empty()),
                                    self.preview_rx.is_some(),
                                    interactive,
                                );
                                if interactive {
                                    swap_action = a;
                                    if self.wallet.is_some() && self.swap_rx.is_none() {
                                        preview_for = Some(info.address.clone());
                                    }
                                }
                            }
                            other => result_card_inner(ui, other),
                        }
                    });
                }
                ChatAnswer::Error(e) => {
                    assistant_bubble(ui, |ui| {
                        ui.label(RichText::new(format!("✗ {e}")).color(RED));
                    });
                }
            }
        }
        // The in-flight turn: the question already reads like part of the
        // thread, the answer is still the thinking line until it lands.
        if busy {
            ui.add_space(if self.chat.is_empty() { 4.0 } else { 18.0 });
            if let Some(q) = &self.pending_question {
                user_bubble(ui, q);
                ui.add_space(8.0);
            }
            assistant_bubble(ui, |ui| thinking_line(ui, self.busy_since, t));
        }
        if let Some(action) = swap_action {
            let ctx = ui.ctx().clone();
            self.run_swap(action.symbol, action.token_out, &ctx);
        } else if let Some(addr) = preview_for {
            let ctx = ui.ctx().clone();
            self.maybe_preview_quote(&addr, &ctx);
        }
    }

    /// The recent-activity log and the optional Sushi key — settings, not
    /// conversation, so they sit below the thread rather than inside it.
    fn log_and_settings(&mut self, ui: &mut egui::Ui, octx: &ToolCtx) {
        if !self.log.is_empty() {
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
            ui.add_space(10.0);
        }

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

    /// Dispatches one submitted line to whichever door it actually is —
    /// short, single-word, no-space text is treated as a bare ticker (the
    /// fast path: no model call, works with no AI key at all); anything
    /// longer or with a space goes to the model to parse. One box, same two
    /// doors as before.
    fn submit(&mut self, ctx: &egui::Context) {
        let text = self.ask.trim().to_string();
        if text.is_empty() {
            return;
        }
        if looks_like_ticker(&text) {
            self.ask.clear();
            self.ticker = text.trim_start_matches('$').to_string();
            self.look_up(ctx);
        } else {
            self.ask_model(ctx);
        }
    }

    /// The input, pinned to the bottom of the panel — one box, a send
    /// button, and the wallet/status readout that decides what the box can
    /// even do right now.
    fn composer(&mut self, ui: &mut egui::Ui, octx: &ToolCtx) {
        let busy = self.rx.is_some();
        let has_ai = !self.ai_key.trim().is_empty();
        let t = ui.input(|i| i.time);

        if !has_ai {
            self.ai_key_setup(ui, octx);
            ui.add_space(8.0);
        }

        // Starters, because the hardest part of a blank box is the first
        // question — shown only before the first turn, same as a fresh
        // ChatGPT/Claude conversation. Clicking one asks it outright rather
        // than only filling the field.
        if self.chat.is_empty() && !busy {
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
            ui.add_space(8.0);
        }

        ui.horizontal(|ui| {
            // Idle it breathes; working it runs. The panel should look awake
            // before you have typed anything into it.
            let phase = if busy { t * 5.0 } else { t * 1.6 };
            let pulse = 0.5 + 0.5 * (phase.sin() as f32);
            let (dot, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
            let p = ui.painter();
            p.circle_filled(dot.center(), 3.0 + 2.5 * pulse, ACCENT.gamma_multiply(0.18));
            p.circle_filled(dot.center(), 3.0, ACCENT.gamma_multiply(0.45 + 0.55 * pulse));
            ui.label(RichText::new(if busy { "working" } else { "ready" }).color(FAINT).small());

            // Right-aligned wallet status. Whether a swap is even offered
            // downstream (in the token card) depends entirely on this.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                match (&self.wallet, self.wallet_connecting) {
                    (Some(addr), _) => {
                        ui.label(RichText::new(short_addr(addr)).color(UP).small());
                        ui.label(RichText::new("●").color(UP).small());
                    }
                    (None, true) => {
                        ui.add(egui::Spinner::new().size(12.0).color(FAINT));
                        ui.label(RichText::new("connecting…").color(FAINT).small());
                    }
                    (None, false) => {
                        if tool_button(ui, "Connect Frame", true) {
                            let ctx = ui.ctx().clone();
                            self.connect_wallet(&ctx);
                        }
                    }
                }
            });
        });
        if let Some(e) = &self.wallet_err {
            ui.label(RichText::new(format!("✗ {e}")).color(RED).small());
        }
        ui.add_space(6.0);

        let frame = egui::Frame::NONE
            .fill(BG_ELEVATED)
            .stroke(egui::Stroke::new(1.0_f32, ACCENT.gamma_multiply(0.35)))
            .corner_radius(22.0)
            .inner_margin(egui::Margin::symmetric(16, 10));
        frame.show(ui, |ui| {
            ui.horizontal(|ui| {
                let field = ui.add_enabled(
                    !busy,
                    egui::TextEdit::singleline(&mut self.ask)
                        .hint_text(
                            "swap into a ticker, or ask anything — \"1 WETH in USDG?\", \
                             \"PONS\", \"what's pumping\"",
                        )
                        .frame(egui::Frame::NONE)
                        .font(FontId::proportional(14.0))
                        .desired_width((ui.available_width() - 66.0).max(80.0)),
                );
                let entered =
                    field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let text = self.ask.trim().to_string();
                let ready = !text.is_empty() && !busy && (looks_like_ticker(&text) || has_ai);
                if (tool_button(ui, "Send", ready) || entered) && ready {
                    let ctx = ui.ctx().clone();
                    self.submit(&ctx);
                }
            });
        });
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
    x += T_VOL1H;
    p.text(egui::pos2(x, cy), egui::Align2::RIGHT_CENTER, "VOL 1H", f.clone(), FAINT);
    x += T_VOL;
    p.text(egui::pos2(x, cy), egui::Align2::RIGHT_CENTER, "VOL 24H", f.clone(), FAINT);
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

    x += T_VOL1H;
    // Flat 24h activity spread evenly would put ~4.2% of it in any given
    // hour; well above that means the last hour is carrying more than its
    // share — accent it, since that's the token actually moving right now
    // rather than one coasting on a volume spike from hours ago.
    let heating = r.volume_h24 > 0.0 && r.volume_h1 > r.volume_h24 * 0.15;
    p.text(
        egui::pos2(x, cy),
        egui::Align2::RIGHT_CENTER,
        market::money_compact(r.volume_h1),
        num.clone(),
        if heating { ORANGE } else { DIM },
    );

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

/// `0x1234…abcd` — enough to recognise a wallet at a glance, none of it
/// wasted on the middle 32 hex characters nobody reads.
fn short_addr(a: &str) -> String {
    if a.len() < 12 { a.to_string() } else { format!("{}…{}", &a[..6], &a[a.len() - 4..]) }
}

/// Short, single-word, no-space text reads as a bare ticker rather than a
/// sentence — the fast path that skips the model entirely (and works with no
/// AI key at all), same shape a `$PONS` or a trending-row click already used.
fn looks_like_ticker(text: &str) -> bool {
    let t = text.trim().trim_start_matches('$');
    !t.is_empty() && t.len() <= 15 && !t.contains(' ') && t.chars().all(|c| c.is_ascii_alphanumeric())
}

/// One user line, right-aligned like every chat app ever — the one bit of
/// "chat UI" convention this needed to actually read as a conversation
/// instead of a form with a memory.
fn user_bubble(ui: &mut egui::Ui, text: &str) {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
        let max_w = (ui.available_width() * 0.72).min(560.0);
        egui::Frame::NONE
            .fill(ACCENT.gamma_multiply(0.22))
            .corner_radius(14.0)
            .inner_margin(egui::Margin::symmetric(14, 10))
            .show(ui, |ui| {
                ui.set_max_width(max_w);
                ui.label(RichText::new(text).color(FG));
            });
    });
}

/// The agent's half of the exchange — same rounded-bubble treatment, left
/// side, wide enough to hold a data card without cramping it.
fn assistant_bubble(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
    let max_w = (ui.available_width() * 0.86).min(720.0);
    egui::Frame::NONE
        .fill(BG_ELEVATED)
        .corner_radius(14.0)
        .inner_margin(egui::Margin::symmetric(16, 14))
        .show(ui, |ui| {
            ui.set_max_width(max_w);
            add_contents(ui);
        });
}

/// One token, read across every window the indexer gives.
///
/// The four percentages sit side by side on purpose: a token up on the day and
/// down on the hour is a different story from one up on both, and that contrast
/// is invisible when the windows are shown one at a time.
/// A chat-requested read of the board: the same table the always-visible
/// section shows, plus a take on the set as a whole. Rows are display-only
/// here — the persistent board below is where a row turns into a lookup.
fn screen_card(ui: &mut egui::Ui, rows: &[trending::Row], take: &str) {
    ui.label(
        RichText::new(format!("ROBINHOOD CHAIN · {} tokens · by 24h volume", rows.len()))
            .color(ACCENT)
            .small()
            .strong(),
    );
    ui.add_space(8.0);

    if rows.is_empty() {
        ui.label(RichText::new("nothing trading right now").color(FAINT).small());
        return;
    }

    trending_header(ui);
    for (i, row) in rows.iter().enumerate() {
        trending_row(ui, i, row);
    }

    if !take.is_empty() {
        ui.add_space(12.0);
        ui.separator();
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("◆").color(ACCENT).small());
            ui.label(RichText::new("THE AGENT'S HOT TAKE").color(ACCENT).small().strong());
        });
        ui.add_space(6.0);
        ui.label(RichText::new(take).color(FG));
        ui.add_space(6.0);
        ui.label(
            RichText::new("Reacting to the board above, not a signal — it will never tell you which one to trade.")
                .color(FAINT)
                .small(),
        );
    }
}

struct SwapAction {
    token_out: String,
    symbol: String,
}

/// Returns `Some` exactly when the swap button was just clicked — the caller
/// is what actually starts the job, same split as `trending_row`'s click
/// returning `bool` rather than reaching into `self` from inside the table.
#[allow(clippy::too_many_arguments)]
/// `interactive` gates the swap block entirely — false for every turn but the
/// latest in the chat thread, so an old lookup reads as history rather than
/// as a second live control surface for the one wallet session.
#[allow(clippy::too_many_arguments)]
fn token_card(
    ui: &mut egui::Ui,
    info: &trending::Info,
    take: &str,
    wallet: Option<&str>,
    swap_amount: &mut String,
    swap_busy: bool,
    swap_step: &str,
    swap_result: &Option<Result<SwapDone, String>>,
    preview: Option<&api::Quote>,
    preview_err: Option<&str>,
    preview_loading: bool,
    interactive: bool,
) -> Option<SwapAction> {
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
            ui.label(RichText::new("THE AGENT'S HOT TAKE").color(ACCENT).small().strong());
        });
        ui.add_space(6.0);
        ui.label(RichText::new(take).color(FG));
        ui.add_space(6.0);
        ui.label(
            RichText::new("Reacting to the numbers above, not a signal — it will never tell you to buy or sell.")
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
        // The zero-setup path: opens Sushi's own swap UI with this token
        // pre-filled. Verified live — the chain comes from the URL's path
        // segment, not a query parameter, which is not the obvious way to
        // read it. No wallet code of ours is involved past this link.
        ui.hyperlink_to(
            RichText::new("swap on sushi.com ↗").color(ACCENT).small(),
            trending::sushi_swap_url(&info.address),
        );
    });

    // The swap block. Absent entirely without a connected wallet, rather than
    // shown disabled — a greyed-out button invites clicking it to see what
    // happens, and what happens should never be "nothing, silently". Absent
    // too on a non-interactive (historical) card, regardless of wallet state.
    let mut action = None;
    if !interactive {
        // Nothing — a past turn is a record, not a second place to trade from.
    } else if let Some(from) = wallet {
        ui.add_space(12.0);
        ui.separator();
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("SWAP").color(ACCENT).small().strong());
            ui.label(RichText::new(short_addr(from)).color(FAINT).small());
        });
        ui.add_space(6.0);

        if swap_busy {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(14.0).color(ACCENT));
                ui.label(RichText::new(swap_step).color(FG));
            });
        } else {
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(swap_amount)
                        .desired_width(70.0)
                        .font(FontId::monospace(13.0)),
                );
                ui.label(RichText::new("WETH →").color(DIM));
                ui.label(RichText::new(&info.symbol).color(FG).strong());
                if tool_button(ui, "Swap", true) {
                    action = Some(SwapAction {
                        token_out: info.address.clone(),
                        symbol: info.symbol.clone(),
                    });
                }
            });
            ui.add_space(6.0);
            // The whole point of asking Sushi before signing anything: this
            // is its router's actual best route, quoted live, not a promise.
            // Degen voice on purpose — it's the same board as the take above
            // it — but the numbers are real, re-fetched on every settled
            // amount rather than phrased by a model.
            if preview_loading {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().size(11.0).color(FAINT));
                    ui.label(RichText::new("sniffing out the best route…").color(FAINT).small());
                });
            } else if let Some(q) = preview {
                let (_, got) = Outcome::legs(q);
                let impact = q.price_impact.unwrap_or(0.0);
                let hot = impact.abs() > 0.03;
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("≈ {got} {} in the bag", q.token_out.symbol))
                            .color(if hot { ORANGE } else { UP })
                            .font(FontId::monospace(12.5)),
                    );
                    if q.price_impact.is_some() {
                        ui.label(
                            RichText::new(format!("impact {:.2}%", impact * 100.0))
                                .color(if hot { RED } else { FAINT })
                                .small(),
                        );
                    }
                });
                if hot {
                    ui.label(
                        RichText::new("thin pool — that's a real haircut, maybe size down")
                            .color(RED)
                            .small(),
                    );
                }
            } else if let Some(e) = preview_err {
                ui.label(RichText::new(format!("no route — {e}")).color(FAINT).small());
            }
            ui.add_space(4.0);
            ui.label(
                RichText::new(
                    "Opens in Frame for you to review and confirm — nothing is sent from here.",
                )
                .color(FAINT)
                .small(),
            );
        }

        match swap_result {
            Some(Ok(done)) => {
                ui.add_space(8.0);
                ui.label(
                    RichText::new(format!(
                        "✓ swapped {} WETH for ~{} {}",
                        done.sent, done.expected_out, done.got_symbol
                    ))
                    .color(UP),
                );
                if let Some(pct) = done.price_impact {
                    ui.label(
                        RichText::new(format!("price impact {:.2}%", pct * 100.0))
                            .color(if pct.abs() > 0.03 { ORANGE } else { FAINT })
                            .small(),
                    );
                }
                let url = format!("https://robinhoodchain.blockscout.com/tx/{}", done.tx_hash);
                ui.hyperlink_to(
                    RichText::new(format!("{} ↗", short_addr(&done.tx_hash)))
                        .color(FAINT)
                        .small()
                        .font(FontId::monospace(11.0)),
                    &url,
                );
            }
            Some(Err(e)) => {
                ui.add_space(8.0);
                ui.label(RichText::new(format!("✗ {e}")).color(RED).small());
            }
            None => {}
        }
    } else {
        ui.add_space(10.0);
        ui.label(
            RichText::new("Connect Frame above to swap into this token.").color(FAINT).small(),
        );
    }

    action
}

/// Content only, no frame of its own — the caller (`assistant_bubble`)
/// already supplies the card, so this would otherwise double up.
fn result_card_inner(ui: &mut egui::Ui, outcome: &Outcome) {
    match outcome {
        // Token is rendered by `chat_thread` directly (it needs the wallet
        // and swap state this function isn't given) — rendering nothing here
        // keeps that invariant instead of panicking on it.
        Outcome::Token { .. } => {}
        Outcome::Market { rows, sort, limit } => {
            ui.label(
                RichText::new(format!("top {limit} · {}", sort.label())).color(FAINT).small(),
            );
            ui.add_space(6.0);
            table_header(ui);
            for (i, row) in rows.iter().enumerate() {
                market_row(ui, i, row);
            }
        }
        Outcome::Screen { rows, take } => screen_card(ui, rows, take),
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
    }
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
