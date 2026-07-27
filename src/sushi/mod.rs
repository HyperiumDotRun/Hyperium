//! Sushi agent.
//!
//! The agent leads: you ask in plain English and it answers. Below it sit the
//! two boards it draws on — what is launching on Robinhood Chain right now, and
//! what is moving across the wider market. The model only ever parses intent;
//! resolution, arithmetic and API calls happen in Rust. The model itself never
//! signs or broadcasts anything — a swap only ever moves once a human has
//! confirmed the exact amount, recipient and gas shown in this app's own
//! confirmation panel, backed by a locally-held key (`signer.rs`) that never
//! leaves this machine.

mod api;
mod bonding;
mod chain_rpc;
mod erc20;
mod guardian;
mod trending;
mod intent;
mod market;
mod ondo;
mod signer;
mod tokens;

/// Entry point for `hyperium --sign-worker <config-dir>`: reads one
/// transaction as JSON from stdin, signs it against the key stored in that
/// config dir, and writes the signed raw tx to stdout. See `signer.rs` for
/// why this runs as its own short-lived process rather than inline.
pub fn sign_worker_main(cfg: &std::path::Path) -> i32 {
    signer::run_worker(cfg)
}

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use eframe::egui;
use egui::{Color32, FontId, RichText};

use crate::tools::{ACCENT, BG_ELEVATED, DIM, FAINT, FG, RED, Tool, ToolCtx, tool_button, tool_button_hint};

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
/// One "try" chip in the composer. `hint` is a hover tooltip, kept for only
/// the examples whose label alone doesn't say what's actually happening
/// behind it (a computed screener) — a plain lookup or swap doesn't need
/// one, that's exactly what it looks like.
struct Example {
    text: &'static str,
    hint: Option<&'static str>,
}

const EXAMPLES: &[Example] = &[
    Example { text: "price of WETH", hint: None },
    Example { text: "1 WETH in USDG", hint: None },
    Example {
        text: "what's heating up on Robinhood Chain",
        hint: Some(
            "Ranks every token by last hour's volume against its own normal hourly \
             pace — a real ratio computed in Rust, not a guess.",
        ),
    },
    Example {
        text: "which Ondo stock has unusual volume",
        hint: Some(
            "Scans all of Ondo's supported assets in one call and ranks them by \
             today's volume against each one's own average.",
        ),
    },
    Example {
        text: "any Ondo stock near a 52-week high",
        hint: Some(
            "Uses each stock's real 52-week high/low to find how close today's \
             price sits to either edge.",
        ),
    },
    Example {
        text: "dividend yield of AAPL on Ondo",
        hint: Some(
            "Pulls this stock's real dividend data straight from Ondo: annualized \
             yield, how often it pays (monthly/quarterly/etc), and the last cash \
             amount and date paid.",
        ),
    },
];

/// Only shown once the bonding-curve feature is switched on in settings —
/// otherwise clicking it would just error, and the chip row should never
/// advertise a tool that can't actually run. This is the fix for the gap
/// where the feature was reachable only by finding it in the settings
/// panel first: same "try" row every other capability gets discovered
/// through.
const BONDING_EXAMPLE: Example = Example {
    text: "launch a token paired with TSLA (testnet)",
    hint: Some(
        "Experimental, Robinhood Chain TESTNET only — creates a new token whose \
         bonding-curve liquidity is a test stock token instead of ETH/USDC (the \
         longdotxyz idea). No real money. Once it exists, ask to buy on it or check \
         its status too.",
    ),
};

/// Phrases cycled while a request is in flight. They name the step actually
/// under way, so the wait reads as work rather than as a hang.
const THINKING: &[&str] =
    &["reading the chain", "pulling the pools", "analysing the move", "thinking it through"];

/// How many launches the board shows. Dexscreener returns 30 pairs per quote
/// token and most of the tail is dust, so the board keeps the busy end.
const TRENDING_ROWS: usize = 14;

/// How many rows the Ondo tab's dashboard shows — the busy end of
/// `ondo::screen`'s ranking, same "keep the busy end" idea as
/// `TRENDING_ROWS`.
const ONDO_DASHBOARD_ROWS: usize = 12;

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

/// Same hard line as `TAKE_SYSTEM` (no buy/sell calls, no "safe"), a
/// different voice — Ondo Stocks are real equities, reference-only in this
/// app, not a degen chain board, so this stays plain rather than performing
/// the same "unhinged crypto-twitter" bit that would be tone-deaf here.
const STOCK_TAKE_SYSTEM: &str = "\
You are reading one Ondo Finance tokenized-stock's reference data — a real US equity \
traded on-chain, not a crypto/degen token. Talk plainly, like someone glancing at a \
stock quote, not a research desk and not crypto-twitter.

Reply with one or two short sentences reacting to what the numbers show: whether \
today's volume is running above or below its own average, and — if given — where the \
price sits in its 52-week range. Cite the actual figures you are reading.

Never tell anyone to buy or sell this or any other stock, never say a price will go up \
or down, never call anything safe. This is reference data only — say so if it's not \
already obvious from context — this app can't trade it.";

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

/// Narrates a quote/swap's path in plain language. Deliberately restricted
/// to what the API actually returns (see `api::Quote::route_hops`'s doc
/// comment): no pool addresses, no per-hop percentages — that data doesn't
/// exist in the response, so the model must not invent it.
const ROUTE_SYSTEM: &str = "\
You are explaining, in one or two short plain-language sentences, the path a token swap \
takes and what its price impact means for the person making it. You are given the token \
path (e.g. \"ETH -> USDC -> DAI\") and the price impact percentage — nothing more: no pool \
addresses, no per-hop breakdown, because the API this reads from does not expose that. Do \
not invent pool names or percentages beyond what you were given.

If the path is direct (no intermediate token), just note that plainly rather than making \
a big deal of it. If there are one or more hops, say so and, if the price impact is \
non-trivial, connect the two (e.g. a multi-hop route can compound impact across each leg).

Never tell anyone to go ahead or not to, never say a price will move, never call anything \
safe. Describe the route, not the decision.";

/// System prompt for the actual conversation (`ask_model`) — as opposed to
/// `TAKE_SYSTEM`/`SCREEN_SYSTEM`, which each write one card's commentary from
/// data already fetched. This is the one the user is actually talking to:
/// real chat, tool calls when a question needs real numbers, same degen
/// voice, same hard limits (no invented numbers, no buy/sell calls, no
/// pretending it can sign anything itself).
fn agent_system_prompt(bonding_enabled: bool) -> String {
    let bonding_paragraph = if bonding_enabled {
        "\n\nA separate, experimental, TESTNET-ONLY feature is also on: launch_token, \
         buy_on_curve and curve_status let someone launch a brand-new token whose bonding-curve \
         liquidity is a Robinhood Chain TESTNET stock token instead of ETH/USDC (the longdotxyz \
         idea), buy on it, and check how close it is to graduating into a pool. This is \
         completely separate from execute_swap/lookup_token's real Robinhood Chain (mainnet) — \
         say plainly this is testnet, no real money, whenever it comes up. curve_address (from \
         launch_token's result) is required for every later buy_on_curve/curve_status call on \
         that token — never invent or guess one; if it isn't in this conversation already, say \
         you don't have it rather than making one up."
    } else {
        ""
    };
    format!(
        "You are Hyperium's Sushi agent — a crypto-market chatbot embedded in a desktop app. \
Talk like a knowledgeable person having a normal conversation, not a press release and not \
a try-hard crypto-twitter impression — no forced slang, no stacking emoji, no \"yo\"/\"fam\"/ \
👀 filler in every line. A little personality and directness is good; sounding like you're \
performing \"degen\" is not. If someone says \"how's it going\" or asks something with \
nothing to do with crypto, just talk to them normally — do not force every reply into \
market commentary, and do not refuse small talk. Save the tools for when a question \
actually needs real data.

Tools available: market_overview (whole crypto market via CoinGecko, 24h volume only), \
screen_chain (what's trading on Robinhood Chain right now via Dexscreener, real 5m/1h/6h/24h \
windows), chain_screen (the same chain ranked by unusualness instead of raw volume — see the \
paragraph below), lookup_token (everything on one Robinhood Chain ticker — works for ANY \
token there, including ones minutes old — and is also what puts up the swap card the user \
can act on), get_price and get_quote (restricted to this short curated whitelist only:\n{}\
wallet_balance (the imported wallet's real on-chain balance of one token, read live — this \
is the ONLY way you ever know what's in the wallet), execute_swap (actually sends a swap on \
Robinhood Chain from the imported wallet — see the paragraph below on when to call it), \
lookup_stock (reference price for one named Ondo Finance tokenized US stock — TSLA/TSLAon, \
AAPL/AAPLon, etc), lookup_dividend (that same stock's dividend info — yield, payout \
frequency, last payment — a separate call, use it only when dividends/yield are actually \
asked about), ondo_screen (scans EVERY Ondo Stocks asset at once, ranked by unusualness \
— see the paragraph below). Prefer screen_chain over market_overview when Robinhood Chain is \
what's actually meant, since it has finer windows market_overview structurally can't. Prefer \
lookup_token over get_price/get_quote for anything not in that whitelist above — that's the \
common case, not the exception, for tokens on this chain.

Two tools exist purely to answer \"what's unusual\" rather than \"what's busy\", and both do \
the actual comparison in Rust rather than handing you a pile of numbers to eyeball — read the \
ratio, don't recompute it: chain_screen ranks Robinhood Chain by heat_ratio (last hour's \
volume against that token's own flat 24h pace) and also carries a buy-pressure percentage \
per row (share of last-hour trades that were buys); ondo_screen ranks the Ondo watchlist by \
volume_ratio (today's volume against each stock's own trailing average) and also carries a \
52-week range position per row (0% at the 52w low, 100% at the 52w high — useful for \"near \
its high/low\" questions). Call these instead of screen_chain/lookup_stock whenever the \
question is about the set as a whole rather than one specific token — \"what's heating up\", \
\"more volume than usual\", \"anything unusual today\", \"near a 52-week high\".

Ondo Finance's tokenized stocks are a completely separate thing from everything else here: a \
different chain (Ethereum/BNB/Solana, not Robinhood Chain), a different product, no swap card, \
no trading. If someone asks about a tokenized stock or something that sounds like a ticker \
with \"on\" tacked on (TSLAon, AAPLon...), or explicitly says \"Ondo\", call lookup_stock and \
give them the reference price plainly — but say clearly this app can't trade it (yet), rather \
than implying a Buy button exists somewhere for it. Don't confuse this with Robinhood Chain's \
own tokenized-equities tokens, which DO trade here via lookup_token like anything else on \
that chain — if it's ambiguous which one someone means, ask.

If someone names two tokens and wants to swap between them and one isn't on the \
get_price/get_quote whitelist, don't just refuse — call lookup_token on the one they want \
to receive, then execute_swap with whichever token they said they're holding as token_in \
(check wallet_balance first if you're not sure the wallet actually holds it).

Never invent a number — every price, volume, quote, or balance in a reply came from one of \
these tools this turn or a tool call earlier in the conversation, never from memory. This \
includes any dollar conversion: wallet_balance returns a token amount, not a USD value — if \
you're about to say a balance is \"worth about $X\" or do any other math that needs a live \
price, call get_price/market_overview/lookup_token for that token FIRST, in the same turn, \
and use the number that call returns. Do not convert using a price you remember or think \
you know, even approximately, even for ETH — token prices move constantly and your training \
data is stale by definition. If you can't get a live price, say the balance in the token \
itself and say plainly that you don't have a live USD figure, rather than estimating one.

Never tell anyone to buy or sell anything, never call a token safe, never rank tokens by \
which is the better trade — you can describe a chart, not pick a winner. If asked what's in \
the wallet or what it's worth, call wallet_balance for the specific token asked about rather \
than guessing or deflecting — and if none is named, ask which one, or check the native ETH \
balance as the obvious default. There is no balance/holdings view anywhere else in this \
app's UI: never claim one exists or tell someone to go look for it there. If wallet_balance \
itself fails or no wallet is imported, say that plainly instead of making something up.

How swapping actually works here: say what to swap in plain English — \"swap 0.5 ETH for \
PONS\" — and call execute_swap directly rather than only describing the card; you don't \
need to ask permission first if the request was already clear, since the app itself still \
stops for a human before anything moves. That tool sends the transaction up to the exact \
same point every manual swap does: an in-app confirmation panel showing the real amount, \
recipient and gas, which the user has to explicitly approve there before anything is \
signed — you cannot skip or auto-answer that panel, it's outside this conversation on \
purpose, and execute_swap simply waits (or reports back cleanly if it's cancelled or times \
out after ten minutes). If no wallet is imported yet, \"Import wallet\" is in the bar above \
the message box — the key is encrypted on this machine and only a short-lived signing \
process ever touches it, never this chat.{bonding_paragraph}",
        tokens::catalog()
    )
}

const DEFAULT_SLIPPAGE: f64 = 0.005;
/// How long the amount field must sit still before a preview quote fires —
/// long enough that typing "0.1" doesn't fire three requests for "0", "0.",
/// "0.1".
const PREVIEW_DEBOUNCE: Duration = Duration::from_millis(450);
const LOG_MAX: usize = 20;
const MARKET_ROWS: usize = 10;
const MARKET_TTL: Duration = Duration::from_secs(60);
/// How long a row's price stays tinted after it changes between refreshes —
/// there is no live tick, so this is the one honest signal that a number just
/// moved rather than having always been what it now shows.
const FLASH_DURATION: Duration = Duration::from_millis(900);

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

// Ondo Stocks dashboard. The watchlist table itself stays price/change/spark
// (volume swings by orders of magnitude across large-caps, a bad fit for a
// fixed column next to a sparkline) — `ondo_screen` is where volume actually
// gets used, ranking by today's volume against each stock's own average
// rather than displaying a raw number here that would mean nothing on its
// own. The name column takes the room the market table gives volume instead.
const O_RANK: f32 = 30.0;
const O_SYM: f32 = 74.0;
const O_NAME: f32 = 240.0;
const O_PRICE: f32 = 116.0;
const O_CHG: f32 = 88.0;
const O_SPARK: f32 = 140.0;
const O_TABLE_W: f32 = O_RANK + O_SYM + O_NAME + O_PRICE + O_CHG + O_SPARK;

enum Outcome {
    Price { chain: &'static str, symbol: String, address: String, usd: f64 },
    /// A ticker looked up on the chain, with the agent's read of it. The take
    /// is carried alongside the numbers rather than fetched separately so the
    /// card can never show a comment about figures it is no longer displaying.
    Token { info: Box<trending::Info>, take: String },
    /// `route_note` is the model's plain-language read of the path
    /// (`ROUTE_SYSTEM`), empty with no AI key configured — same shape as
    /// `Token`'s `take`. `route_path_words` (deterministic, free) is derived
    /// from `quote.route_hops` at render time rather than stored here.
    Quote { chain: &'static str, quote: api::Quote, route_note: String },
    /// Feeds the table rather than the result card.
    Market { rows: Vec<market::Row>, sort: market::Sort, limit: usize },
    /// A chat-requested read of the Robinhood Chain board — same data as the
    /// always-visible table, plus a take on the set as a whole.
    Screen { rows: Vec<trending::Row>, take: String, window: trending::Window },
    /// A genuinely conversational turn from the tool-calling agent
    /// (`ask_model`) — "how's it going", "what can you do", small talk, or
    /// the tail end of a data question once the model has the numbers back.
    /// `card` is whichever tool result (if any) the model called during this
    /// turn, still the same `Outcome` shapes above so the same rendering
    /// code shows it — the model never invents the numbers in `reply`, it's
    /// just the one putting them into a sentence.
    Chat { reply: String, card: Option<Box<Outcome>> },
    /// A reference read from Ondo Finance's tokenized-stocks API — a
    /// *different* chain and product from everything else in this file
    /// (Ondo Stocks live on Ethereum/BNB/Solana, not Robinhood Chain).
    /// Informational only: there is no swap card, no mint/redeem — see
    /// `ondo.rs`. Reachable by clicking a symbol in the Ondo tab's tables,
    /// same as `Token` is on the Robinhood side.
    Stock {
        symbol: String,
        name: Option<String>,
        price_usd: f64,
        change_24h: Option<f64>,
        /// Same-day closes, straight from `ondo::Market` — carried through so
        /// the single-ticker card can draw the same sparkline the watchlist
        /// rows do, instead of a lookup answering with less than a screen.
        spark: Vec<f32>,
        /// The model's short read on this one stock (`STOCK_TAKE_SYSTEM`),
        /// empty with no AI key configured — same shape as `Token`'s `take`.
        take: String,
    },
    /// The Ondo watchlist ranked by `volume_ratio` — answers "which of these
    /// is trading more than usual", the kind of screening question a single
    /// `lookup_stock` call can't (it only ever sees one symbol at a time).
    StockScreen { rows: Vec<ondo::Market> },
    /// Robinhood Chain ranked by `heat_ratio` instead of raw volume — answers
    /// "what's heating up right now", a different question from `Screen`'s
    /// "what's busiest today" even though both draw on the same board.
    ChainHeat { rows: Vec<trending::Row> },
    /// Which of the wallet's Robinhood Chain holdings are currently on the
    /// trending board, with each one's live trending data alongside the
    /// held balance — answers "what am I sitting on that's moving right
    /// now", a join `wallet_balance` (one token) and `chain_screen` (no
    /// wallet) can't each answer alone.
    HoldingsDigest { rows: Vec<DigestRow> },
    /// One Ondo Stock's dividend info — a separate Ondo endpoint from
    /// `Stock`'s market data, explaining *why* the price drifts (dividends
    /// reinvest into the token rather than paying out) rather than what the
    /// price is doing right now.
    Dividend {
        ticker: String,
        yield_frac: Option<f64>,
        payout_frequency: Option<String>,
        last_cash_amount: Option<f64>,
        last_payment_date: Option<String>,
        /// How much the shares multiplier has grown over the last year —
        /// real, on-chain-recorded history of dividends compounding in
        /// (`ondo::multiplier_growth`), not a current-vs-current guess.
        /// `None` when Ondo has no history for it yet (a token minutes old)
        /// or the lookup itself failed — this stays optional rather than
        /// failing the whole dividend read over it.
        multiplier_growth_1y: Option<f64>,
    },
    /// A new bonding-curve token launched via the configured testnet factory
    /// (Phase C — see `bonding.rs`). `curve_address` is what every later
    /// `buy_on_curve`/`curve_status` call on this token needs; the model has
    /// to carry it forward from here rather than guess or reuse a different
    /// token's — nothing else in this app can look it up after the fact.
    TokenLaunch {
        token_symbol: String,
        token_name: String,
        paired_symbol: String,
        curve_address: String,
        tx_hash: String,
    },
    /// A buy against an already-launched curve.
    CurveBuy {
        curve_address: String,
        paired_symbol: String,
        stock_in: String,
        min_tokens_out: String,
        tx_hash: String,
    },
    /// A read-only check on a curve: how much of the paired stock token has
    /// been raised against its graduation threshold, and the pool address
    /// once it's graduated. `price_per_token`/`progress_pct` are `None` once
    /// `graduated` — the curve stops pricing trades at that point, spot
    /// price lives on the pool instead, which this doesn't read.
    CurveStatus {
        curve_address: String,
        paired_symbol: String,
        raised: String,
        graduation_threshold: String,
        graduated: bool,
        pool_address: Option<String>,
        price_per_token: Option<f64>,
        progress_pct: Option<f64>,
    },
}

/// What a completed swap actually did, re-read from the chain side rather
/// than assumed from the request — `sent`/`got_symbol` describe the leg that
/// was signed, not the one that was asked for.
struct SwapDone {
    tx_hash: String,
    sent: String,
    sent_symbol: String,
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
/// carries a final result and this needs to update mid-flight, while the
/// user's attention is on this app's own confirmation panel.
#[derive(Clone, PartialEq)]
enum SwapStep {
    Idle,
    Quoting,
    CheckingAllowance,
    /// The `PendingConfirm` panel is up and waiting on the user — the step
    /// that can least afford to look like the app has frozen.
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
            Self::WaitingOnApproval => "waiting for approval",
            Self::ConfirmingApproval => "confirming the approval on-chain",
            Self::WaitingOnSwap => "waiting for the swap",
        }
    }
}

/// A transaction the local-signer path has priced and gassed, waiting for
/// the user to look at it before anything is signed. The background thread
/// blocks on `reply` until the UI answers — that block, on a thread nothing
/// else depends on, is the entire mechanism: no timers, no polling, just a
/// human on the other end of a channel.
struct PendingConfirm {
    label: &'static str,
    amount: String,
    to: String,
    estimated_fee_native: String,
    /// The model's plain-language read of the route (`ROUTE_SYSTEM`), when
    /// an AI key is configured and the call succeeded. `None` for the
    /// approval leg of a swap (nothing route-shaped to say about it) and
    /// whenever narration fails — silently absent, never an error shown on
    /// a confirmation screen.
    route_note: Option<String>,
    reply: std::sync::mpsc::SyncSender<bool>,
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

/// "ETH → USDC → DAI (2 hops)" — the free half of route x-ray: no network
/// call, no model, just what `route_hops` already carries. `None` for a
/// direct pair, where there's nothing worth saying.
fn route_path_words(token_in: &str, token_out: &str, hops: &[api::QuoteToken]) -> Option<String> {
    if hops.is_empty() {
        return None;
    }
    let path = std::iter::once(token_in)
        .chain(hops.iter().map(|h| h.symbol.as_str()))
        .chain(std::iter::once(token_out))
        .collect::<Vec<_>>()
        .join(" \u{2192} ");
    Some(format!("{path} ({} hops)", hops.len() + 1))
}

/// Numeric brief fed to `ROUTE_SYSTEM` — the model narrates only what's in
/// here, nothing it wasn't given.
fn route_brief(token_in: &str, token_out: &str, hops: &[api::QuoteToken], price_impact: Option<f64>) -> String {
    let path = std::iter::once(token_in)
        .chain(hops.iter().map(|h| h.symbol.as_str()))
        .chain(std::iter::once(token_out))
        .collect::<Vec<_>>()
        .join(" -> ");
    format!(
        "path: {path} ({} hop{})\nprice impact: {}",
        hops.len() + 1,
        if hops.len() + 1 == 1 { "" } else { "s" },
        price_impact.map(|p| format!("{:.3}%", p * 100.0)).unwrap_or_else(|| "unknown".into()),
    )
}

/// A written line for the turn, not just a data card underneath it — this is
/// the difference between a widget and a chat message. Built in Rust from the
/// numbers already fetched, never a separate model call: Price/Quote/Market
/// are simple enough to phrase deterministically, and doing it this way means
/// the sentence cannot possibly disagree with the card below it. Token and
/// Screen already carry a written line of their own — the model's take — so
/// this returns empty for both rather than saying the same thing twice.
/// `Chat` already carries the model's own words in full, so it passes them
/// through rather than trying to summarize them again.
fn chat_reply(outcome: &Outcome) -> String {
    match outcome {
        Outcome::Chat { reply, .. } => reply.clone(),
        Outcome::Price { chain, symbol, usd, .. } => {
            format!("{symbol} is at {} on {chain}.", market::money_price(*usd))
        }
        Outcome::Quote { chain, quote, .. } => {
            let (sent, got) = Outcome::legs(quote);
            let hop_note = match quote.route_hops.len() {
                0 => String::new(),
                1 => format!(" (via {})", quote.route_hops[0].symbol),
                _ => format!(" ({} hops)", quote.hop_count()),
            };
            match quote.status.as_str() {
                "Success" => format!(
                    "{sent} {} gets you about {got} {}{hop_note} on {chain}.",
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
        // `Stock` carries its own `take` the same way `Token` does — shown
        // inside the card itself, not duplicated as a written line above it.
        Outcome::Token { .. } | Outcome::Screen { .. } | Outcome::Stock { .. } => String::new(),
        // Sorted by `ondo::screen` already, so the first entry with a usable
        // ratio is the whole answer — same "top of a ranked list" shape as
        // `Outcome::Market` above.
        Outcome::StockScreen { rows } => match rows.first().and_then(|r| r.volume_ratio().map(|ratio| (r, ratio))) {
            Some((top, ratio)) => format!(
                "{} is running about {ratio:.1}x its usual volume today — the most unusual on \
                 the watchlist right now.",
                top.symbol,
            ),
            None => "Nothing on the watchlist has a usable volume reading right now.".to_string(),
        },
        // Sorted by `screen_heat` already, same "top of a ranked list" shape.
        Outcome::ChainHeat { rows } => match rows.first().and_then(|r| r.heat_ratio().map(|ratio| (r, ratio))) {
            Some((top, ratio)) => format!(
                "{} is running about {ratio:.1}x its normal hourly pace on Robinhood Chain — \
                 the most unusual right now.",
                top.symbol,
            ),
            None => "Nothing on Robinhood Chain has a usable volume reading right now.".to_string(),
        },
        Outcome::HoldingsDigest { rows } => match rows.first() {
            Some(top) => format!(
                "{} of your holdings {} currently trending on Robinhood Chain — {} leads at {}.",
                rows.len(),
                if rows.len() == 1 { "is" } else { "are" },
                top.row.symbol,
                market::money_price(top.row.price_usd),
            ),
            None => "None of your holdings are currently on the trending board.".to_string(),
        },
        Outcome::Dividend { ticker, yield_frac, payout_frequency, last_cash_amount, .. } => {
            match (yield_frac, payout_frequency.as_deref()) {
                (Some(y), Some(freq)) => format!(
                    "{ticker} yields about {:.2}% ({freq}){}.",
                    y * 100.0,
                    last_cash_amount
                        .map(|c| format!(", last paid ${c:.2}/share"))
                        .unwrap_or_default(),
                ),
                _ => format!("{ticker} doesn't have dividend data on file right now."),
            }
        }
        // All three carry a full sentence already built by `tool_result_text`
        // — nothing further to phrase on top of a launch/buy/status read.
        Outcome::TokenLaunch { .. } | Outcome::CurveBuy { .. } | Outcome::CurveStatus { .. } => {
            String::new()
        }
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

/// The three faces of the panel — split because a chat box asking "what's
/// pumping" and a dashboard row you scan at a glance want different amounts
/// of screen, and forcing both into one long scroll made the composer's
/// place on screen unpredictable. All three still answer through the same
/// `chat` vector: picking a row still asks the agent, it just no longer
/// shows the question-and-answer bubbles on top of the table.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum Tab {
    #[default]
    Chat,
    Robinhood,
    Ondo,
}

pub struct SushiTool {
    active_tab: Tab,
    ask: String,
    ticker: String,
    sushi_key: String,
    /// Ondo Finance's `x-api-key`, stored the same way as `sushi_key` — see
    /// `ondo_key_path`.
    ondo_key: String,
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
    /// The same conversation, in the exact shape the Anthropic API wants it
    /// back (`{"role": ..., "content": ...}`, including past `tool_use` /
    /// `tool_result` blocks) — real history, not a hand-summarized recap, so
    /// a follow-up has the model's own past reasoning to refer back to, not
    /// just Rust's compressed guess at what mattered. Shared with the
    /// background job directly (rather than shuttled back through a channel)
    /// since only one `ask_model` call is ever in flight at a time — the
    /// composer disables the input while busy.
    agent_messages: std::sync::Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
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
    /// Symbol -> (when it last changed, whether it went up), pruned once
    /// `FLASH_DURATION` has passed. Diffed against the previous snapshot the
    /// moment a refresh lands (`poll`), not while drawing — by the time a row
    /// paints, "did this change" has to already be a fact, not a comparison
    /// happening mid-frame against whatever `self.market` was replaced with.
    market_flash: HashMap<String, (Instant, bool)>,
    trending_flash: HashMap<String, (Instant, bool)>,

    /// Ondo Stocks dashboard — a curated large-cap list, refreshed the same
    /// way as `market`/`trending`, but each row is its own request against
    /// `ondo::market` (no bulk endpoint exists), fanned out on one thread.
    ondo: Vec<ondo::Market>,
    ondo_rx: Option<Receiver<Vec<ondo::Market>>>,
    ondo_err: Option<String>,
    ondo_at: Option<Instant>,
    ondo_flash: HashMap<String, (Instant, bool)>,

    /// The imported wallet's address — `None` until a key is imported
    /// (`signer::set_key`), either just now or on an earlier run
    /// (`signer::address` is checked on load).
    wallet: Option<String>,
    wallet_err: Option<String>,
    /// Edit buffer for pasting a private key to import. Kept apart from the
    /// stored key so the import block doesn't vanish on the first keystroke.
    wallet_key_input: String,
    wallet_key_open: bool,
    /// A transaction the local-signer path has priced and gassed, and is
    /// blocked waiting on the user to look at before it signs anything. Set
    /// by the swap-job thread, read and cleared by the UI thread.
    pending_confirm: std::sync::Arc<std::sync::Mutex<Option<PendingConfirm>>>,
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
    /// (token out, source symbol, amount text) the current preview/err/
    /// in-flight fetch answers — compared each frame to know when it's gone
    /// stale (switching the source token counts as stale, same as an edited
    /// amount).
    preview_key: Option<(String, String, String)>,
    /// When the amount field last settled on `preview_key`, so a fetch waits
    /// out the debounce before firing.
    preview_dirty_since: Option<Instant>,

    /// A real slippage curve (1x/10x/100x) for the current preview, fetched
    /// only once the base 1x quote already shows some impact — cheap swaps
    /// never pay for the extra two calls. `None` while unfetched, not yet
    /// worth fetching, or the extra calls failed; `token_card` falls back to
    /// the old flat-threshold read in every one of those cases.
    guardian: Option<guardian::Reading>,
    guardian_rx: Option<Receiver<guardian::Reading>>,
    /// Mirrors `preview_key` — invalidated at the same point, so a stale
    /// reading can never outlive the quote it was graded against.
    guardian_key: Option<(String, String, String)>,

    /// What the connected wallet actually holds among the curated tokens —
    /// candidates for the swap card's "from" picker. Empty until
    /// `scan_wallet_holdings` answers; re-scanned when the wallet address
    /// changes, not every frame — it's one RPC call per curated token.
    holdings: Vec<Holding>,
    holdings_rx: Option<Receiver<Vec<Holding>>>,
    /// The wallet address `holdings` currently answers for — importing or
    /// switching wallets is what triggers a re-scan, not every frame.
    holdings_for: Option<String>,
    /// Symbol of the token chosen as the swap's source. Defaults to WETH —
    /// the one token every pool on this chain is guaranteed to pair against
    /// — until the user picks something else from their own holdings.
    source_symbol: String,

    /// Bonding-curve launch feature settings (factory address + testnet
    /// toggle) — loaded/saved like `sushi_key`/`ondo_key` but through
    /// `bonding::load_config`/`save_config`'s plain flat files, since neither
    /// value is a secret.
    bonding: bonding::BondingConfig,
}

impl Default for SushiTool {
    fn default() -> Self {
        Self {
            active_tab: Tab::default(),
            ask: String::new(),
            ticker: String::new(),
            sushi_key: String::new(),
            ondo_key: String::new(),
            loaded: false,
            ai_key: String::new(),
            ai_input: String::new(),
            rx: None,
            busy_since: None,
            pending_question: None,
            chat: Vec::new(),
            agent_messages: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
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
            market_flash: HashMap::new(),
            trending_flash: HashMap::new(),

            ondo: Vec::new(),
            ondo_rx: None,
            ondo_err: None,
            ondo_at: None,
            ondo_flash: HashMap::new(),

            wallet: None,
            wallet_err: None,
            wallet_key_input: String::new(),
            wallet_key_open: false,
            pending_confirm: std::sync::Arc::new(std::sync::Mutex::new(None)),
            swap_amount: "0.01".into(),
            swap_rx: None,
            swap_result: None,
            swap_step: std::sync::Arc::new(std::sync::Mutex::new(SwapStep::Idle)),

            preview: None,
            preview_err: None,
            preview_rx: None,
            preview_key: None,
            preview_dirty_since: None,

            guardian: None,
            guardian_rx: None,
            guardian_key: None,

            holdings: Vec::new(),
            holdings_rx: None,
            holdings_for: None,
            source_symbol: "WETH".to_string(),

            bonding: bonding::BondingConfig::default(),
        }
    }
}

fn sushi_key_path(cfg: &std::path::Path) -> std::path::PathBuf {
    cfg.join("sushi.key")
}

fn ondo_key_path(cfg: &std::path::Path) -> std::path::PathBuf {
    cfg.join("ondo.key")
}

/// Flattens the board into the same fields it renders — symbol, price, 24h
/// change, volume, age, DEX — so the model reads exactly what the table below
/// its take will show, not a differently-shaped summary that could disagree
/// with it.
fn screen_brief(rows: &[trending::Row], window: trending::Window) -> String {
    let mut out = format!("Robinhood Chain, ranked by {} volume:\n", window.label());
    for r in rows {
        let age = r
            .age_hours()
            .map(|h| if h < 48.0 { format!("{h:.0}h old") } else { format!("{:.0}d old", h / 24.0) })
            .unwrap_or_else(|| "age unknown".into());
        let pct = |c: Option<f64>| c.map(|c| format!("{c:+.1}%")).unwrap_or_else(|| "n/a".into());
        let money = |m: Option<f64>| m.map(|m| format!("${m:.0}")).unwrap_or_else(|| "n/a".into());
        out.push_str(&format!(
            "- {} · ${} · chg 5m {} / 1h {} / 24h {} · vol 5m ${:.0} / 1h ${:.0} / 6h ${:.0} / \
             24h ${:.0} · buys/sells 5m {}/{} · 24h {}/{} · mcap {} · fdv {} · {} · {}{}\n",
            r.symbol,
            r.price_usd,
            pct(r.change_m5),
            pct(r.change_h1),
            pct(r.change_h24),
            r.volume_m5,
            r.volume_h1,
            r.volume_h6,
            r.volume_h24,
            r.buys_m5,
            r.sells_m5,
            r.buys_h24,
            r.sells_h24,
            money(r.market_cap),
            money(r.fdv),
            age,
            r.dex_id,
            if r.labels.is_empty() { String::new() } else { format!(" · {}", r.labels.join(",")) },
        ));
    }
    out
}

/// Execute an already-resolved intent. Past this point nothing invents a
/// number — every branch either calls a real API or fails.
fn run(intent: Intent, api_key: &str, ai_key: &str, ondo_key: &str) -> Result<Outcome, String> {
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
        Intent::ChainScreen { limit, window } => {
            let rows = trending::fetch(limit, window)?;
            let take = if ai_key.trim().is_empty() {
                String::new()
            } else {
                use crate::llm::LlmProvider;
                crate::llm::Anthropic::new(ai_key.to_string())
                    .complete(SCREEN_SYSTEM, &screen_brief(&rows, window))
                    .unwrap_or_else(|e| format!("(no read: {e})"))
            };
            Ok(Outcome::Screen { rows, take, window })
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
            let route_note = if ai_key.trim().is_empty() {
                String::new()
            } else {
                use crate::llm::LlmProvider;
                let brief = route_brief(
                    &quote.token_in.symbol,
                    &quote.token_out.symbol,
                    &quote.route_hops,
                    quote.price_impact,
                );
                crate::llm::Anthropic::new(ai_key.to_string())
                    .complete(ROUTE_SYSTEM, &brief)
                    .unwrap_or_else(|e| format!("(no read: {e})"))
            };
            Ok(Outcome::Quote { chain: chain.name, quote, route_note })
        }
        Intent::StockLookup { ticker } => {
            let m = ondo::market(&ticker, ondo_key)?;
            let take = if ai_key.trim().is_empty() {
                String::new()
            } else {
                use crate::llm::LlmProvider;
                let brief = format!(
                    "{} ({}) = {}{}{}",
                    m.symbol,
                    m.name.as_deref().unwrap_or("Ondo Stock"),
                    market::money_price(m.price_usd),
                    m.change_24h.map(|c| format!(", 24h {c:+.2}%")).unwrap_or_default(),
                    m.volume_ratio()
                        .map(|r| format!(", volume {r:.2}x its own average today"))
                        .unwrap_or_default(),
                );
                crate::llm::Anthropic::new(ai_key.to_string())
                    .complete(STOCK_TAKE_SYSTEM, &brief)
                    .unwrap_or_else(|e| format!("(no read: {e})"))
            };
            Ok(Outcome::Stock {
                symbol: m.symbol,
                name: m.name,
                price_usd: m.price_usd,
                change_24h: m.change_24h,
                spark: m.spark,
                take,
            })
        }
        Intent::DividendLookup { ticker } => {
            let d = ondo::dividends(&ticker, ondo_key)?;
            // Best-effort: a missing/failed multiplier history still leaves
            // the current yield/frequency/last-payment answer intact, it
            // just answers without the historical trend.
            let multiplier_growth_1y = ondo::multiplier_history(&ticker, "1year", ondo_key)
                .ok()
                .and_then(|h| ondo::multiplier_growth(&h));
            Ok(Outcome::Dividend {
                ticker: d.ticker,
                yield_frac: d.yield_frac,
                payout_frequency: d.payout_frequency,
                last_cash_amount: d.last_cash_amount,
                last_payment_date: d.last_payment_date,
                multiplier_growth_1y,
            })
        }
    }
}

/// The tool set `ask_model` hands to Anthropic. Every schema here is what
/// actually gets validated on Anthropic's side before `dispatch_tool` ever
/// sees an `input` — there's no free-text JSON to parse or get subtly wrong.
/// `bonding_enabled` gates the three testnet-launch tools at the bottom —
/// offered to the model only once the user has explicitly turned the
/// feature on in settings, so it never suggests an action that would just
/// fail (or send a testnet-only tx someone forgot they'd enabled).
fn agent_tools(bonding_enabled: bool) -> Vec<crate::llm::ToolSpec> {
    use serde_json::json;
    let mut tools = vec![
        crate::llm::ToolSpec {
            name: "market_overview",
            description: "The whole crypto market (CoinGecko) — what's pumping, biggest \
                coins, top movers, with no chain named. 24h-granularity only; CoinGecko \
                doesn't expose anything finer across the whole market.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "sort": {
                        "type": "string",
                        "enum": ["volume", "market_cap", "gainers", "losers"],
                        "description": "volume/market_cap = biggest right now; gainers/losers = biggest 24h movers"
                    },
                    "limit": { "type": "integer", "description": "default 10, max 50" }
                }
            }),
        },
        crate::llm::ToolSpec {
            name: "screen_chain",
            description: "What's actually trading on Robinhood Chain right now \
                (Dexscreener), ranked by volume. Use for \"on Robinhood\", \"this chain\", \
                \"what just launched\", \"what's hot right now\" — unlike market_overview \
                this has real 5m/1h/6h/24h windows.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "limit": { "type": "integer", "description": "default 20, max 30" },
                    "window": {
                        "type": "string",
                        "enum": ["m5", "h1", "h6", "h24"],
                        "description": "m5 for \"right now\"/\"last 5 minutes\", h1 for \"the last hour\", h24 (default) for \"today\""
                    }
                }
            }),
        },
        crate::llm::ToolSpec {
            name: "chain_screen",
            description: "Scans Robinhood Chain and ranks it by how unusual the LAST HOUR's \
                activity is, not by raw volume like screen_chain — this is the tool for \
                \"what's heating up\", \"more volume than usual\", \"anything unusual right \
                now\" on this chain. Each row carries three numbers, all computed in Rust, \
                never estimate any of them yourself: a heat ratio (last hour's volume against \
                that token's own flat 24h pace — 1.0x is normal, 3.0x is running 3x its usual \
                hourly rate), a buy-pressure percentage for the last hour (share of trades that \
                were buys, not sells — 50% is neutral), and a volume/liquidity ratio (24h \
                volume against the pool's own liquidity — a high number, roughly 3x or more, \
                means trades in that pool move price more per dollar than a deeper pool doing \
                the same volume; it's a fact about the pool's depth, not a verdict on the \
                token — never phrase it as risky/unsafe, just describe the number). A token \
                with almost no volume is excluded first so the ratios aren't just noise from a \
                near-empty pool. Someone asking \"what's heating up\" wants the handful that \
                actually are, not a long list — keep limit small. Use screen_chain instead for \
                \"what's busiest\" or \"what just launched\" — those are about raw activity, \
                not unusualness.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "limit": { "type": "integer", "description": "default 8, max 15 — how many ranked rows to return" }
                }
            }),
        },
        crate::llm::ToolSpec {
            name: "lookup_token",
            description: "Everything known about one specific ticker trading on Robinhood \
                Chain — price, every volume/change window, liquidity, market cap, FDV, \
                buy/sell pressure, pool age, socials. Works for ANY token on this chain, \
                including ones launched minutes ago. This is also what puts up the swap \
                card the user can act on, so call it whenever they name a token they \
                might want to swap into. Note for route/path questions on this chain: this \
                tool itself doesn't return a route — the swap card it opens does, live, once \
                the user types an amount into it (route path plus a liquidity read at that \
                size). Tell the user to enter an amount in the card to see it, rather than \
                saying there's no way to see the route on this chain at all.",
            input_schema: json!({
                "type": "object",
                "properties": { "ticker": { "type": "string" } },
                "required": ["ticker"]
            }),
        },
        crate::llm::ToolSpec {
            name: "get_price",
            description: "One specific token's USD price — but ONLY for a short curated \
                list of well-known tokens on major chains (Ethereum, Base, etc). For \
                anything on Robinhood Chain, or any ticker not in that short list, use \
                lookup_token instead.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "chain": { "type": "string", "description": "defaults to Ethereum if not stated" },
                    "token": { "type": "string" }
                },
                "required": ["token"]
            }),
        },
        crate::llm::ToolSpec {
            name: "get_quote",
            description: "Preview exchange rate between two tokens — but ONLY for the same \
                short curated list as get_price. For Robinhood Chain tokens, use \
                lookup_token on the token the user wants instead; that surfaces a real \
                swap card, this tool doesn't. The result also names the actual route \
                Sushi's router took — which token(s), if any, it hopped through before \
                reaching the output, and the price impact of that route. If the user asks \
                what path/route a swap would take, this is how to answer it: read the route \
                back from the result rather than saying you have no way to show it. A \
                direct pair has no hops, which is itself worth saying plainly.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "chain": { "type": "string" },
                    "token_in": { "type": "string" },
                    "token_out": { "type": "string" },
                    "amount": { "type": "string", "description": "plain decimal string of the input token, e.g. \"0.5\"" }
                },
                "required": ["token_in", "token_out", "amount"]
            }),
        },
        crate::llm::ToolSpec {
            name: "lookup_stock",
            description: "Reference price for one Ondo Finance tokenized US stock (e.g. \
                TSLA/TSLAon, AAPL/AAPLon) — a completely different chain and product from \
                everything else here (Ondo Stocks live on Ethereum/BNB/Solana, not Robinhood \
                Chain). Informational only: there is no swap card, no trading, nothing to \
                click here for these — just say the numbers if asked. Only call this for \
                stock/equity tickers explicitly framed as Ondo/tokenized-stock questions, not \
                for anything on Robinhood Chain (use lookup_token for that).",
            input_schema: json!({
                "type": "object",
                "properties": { "ticker": { "type": "string", "description": "e.g. \"TSLA\" or \"TSLAon\"" } },
                "required": ["ticker"]
            }),
        },
        crate::llm::ToolSpec {
            name: "lookup_dividend",
            description: "Dividend info for one named Ondo Finance tokenized stock: current \
                annualized yield, how often it pays (monthly/quarterly/etc, or none), the last \
                cash amount/date, AND real historical context — how much the token's shares \
                multiplier (the on-chain mechanism that absorbs reinvested dividends) has grown \
                over the last year. That growth figure is a genuine trailing-12-month read, \
                computed from Ondo's own recorded history, not the current yield repeated or \
                estimated — use it whenever someone asks about the yield historically or \
                compares it to a past period; don't say you have no historical data, this tool \
                is exactly that. A separate call from lookup_stock — that one is price, this \
                one is dividends, and most questions only need one of the two. Important: Ondo \
                doesn't pay any of this out in cash to a holder here — it compounds into the \
                token's own price instead, so mention that if someone sounds like they're \
                expecting a cash payment.",
            input_schema: json!({
                "type": "object",
                "properties": { "ticker": { "type": "string", "description": "e.g. \"TSLA\" or \"TSLAon\"" } },
                "required": ["ticker"]
            }),
        },
        crate::llm::ToolSpec {
            name: "ondo_screen",
            description: "Scans EVERY Ondo Stocks asset Ondo supports in one call (Ondo's own \
                bulk market-data endpoint, not a fixed shortlist) and ranks the whole set by \
                today's real trading volume against each stock's own trailing average — this \
                is the tool for \"which Ondo stock has unusual volume\", \"more volume than \
                usual\", \"what's active on Ondo right now\", or any question about the set as \
                a whole rather than one ticker. A ratio of 1.0 is normal; 2.0 means twice the \
                usual volume today. Do the comparison by reading the ratios this tool returns \
                — never estimate \"usual\" volume yourself, it's a real trailing average from \
                Ondo, not something to guess at. Scans everything internally but only returns \
                the top of the ranking (see limit) — someone asking \"what's unusual\" wants \
                the handful that actually are, not all several hundred. Use lookup_stock \
                instead once you already know which one ticker someone wants.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "limit": { "type": "integer", "description": "default 8, max 15 — how many ranked rows to return" }
                }
            }),
        },
        crate::llm::ToolSpec {
            name: "execute_swap",
            description: "Actually swap on Robinhood Chain from the imported wallet — this \
                sends a real transaction, not a preview. Safe to call as soon as the user has \
                clearly asked for a swap; it always stops at the app's own confirmation panel \
                (exact amount, recipient, gas) before anything is signed, so this tool doing \
                the asking rather than the user clicking a card doesn't skip that check. \
                token_in must be something the wallet actually holds — call wallet_balance \
                first if that's not already established this conversation. token_out is \
                resolved the same way lookup_token resolves it.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "token_in": { "type": "string", "description": "symbol already held, e.g. \"ETH\" or \"WETH\"" },
                    "token_out": { "type": "string", "description": "ticker to receive, e.g. \"PONS\"" },
                    "amount": { "type": "string", "description": "plain decimal amount of token_in, e.g. \"0.5\"" }
                },
                "required": ["token_in", "token_out", "amount"]
            }),
        },
        crate::llm::ToolSpec {
            name: "wallet_balance",
            description: "The imported wallet's real on-chain balance of one token on \
                Robinhood Chain, read directly from the chain (never from memory, never \
                estimated). Leave ticker empty or use \"ETH\" for the native balance. \
                Fails cleanly if no wallet is imported yet.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "ticker": { "type": "string", "description": "empty or \"ETH\" for native; any ticker on Robinhood Chain otherwise" }
                }
            }),
        },
        crate::llm::ToolSpec {
            name: "holdings_digest",
            description: "Which of the imported wallet's Robinhood Chain holdings are \
                CURRENTLY on the trending board, with each one's live price, volume and \
                liquidity alongside the held balance — the tool for \"what am I holding that's \
                moving\", \"anything in my bag that's hot right now\". Scans the whole wallet, \
                takes no arguments. A held token that isn't currently trending just doesn't \
                appear in the result — that's not an error. Fails cleanly if no wallet is \
                imported yet.",
            input_schema: json!({ "type": "object", "properties": {} }),
        },
    ];
    if bonding_enabled {
        tools.extend([
            crate::llm::ToolSpec {
                name: "launch_token",
                description: "TESTNET ONLY, experimental: launch a brand-new token whose \
                    bonding-curve liquidity is denominated in a Robinhood Chain testnet stock \
                    token instead of ETH/USDC — the longdotxyz idea. Sends a real testnet \
                    transaction (no real money — everything here is Robinhood Chain TESTNET, \
                    say so plainly), through the same confirmation panel as execute_swap. \
                    paired_stock must be one of the Robinhood Chain testnet stock tokens (ask \
                    wallet_balance or just try the ticker — TSLA/AMD/NFLX/PLTR/AMZN are the ones \
                    configured). graduation_threshold is how much of paired_stock must be raised \
                    before the curve auto-migrates its liquidity into a pool — pick something \
                    reasonable for a demo (tens to low hundreds), not an enormous number nobody \
                    will ever reach. The result gives back a curve_address — tell the user to \
                    save it, since every later buy_on_curve/curve_status call on this token needs \
                    it and there's no other way to look it up from inside this app.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "the new token's full name" },
                        "symbol": { "type": "string", "description": "the new token's ticker" },
                        "paired_stock": { "type": "string", "description": "e.g. \"TSLA\" — must be a Robinhood Chain testnet stock token" },
                        "graduation_threshold": { "type": "string", "description": "plain decimal amount of paired_stock, e.g. \"500\"" }
                    },
                    "required": ["name", "symbol", "paired_stock", "graduation_threshold"]
                }),
            },
            crate::llm::ToolSpec {
                name: "buy_on_curve",
                description: "TESTNET ONLY: buy a bonding-curve token launched via launch_token, \
                    spending the same stock token it's paired with. curve_address must be the \
                    exact address launch_token (or curve_status) returned for this token — never \
                    guess one. Approves the stock token to the curve first if needed, then buys, \
                    both through the same confirmation panel as execute_swap — this sends real \
                    testnet transactions.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "curve_address": { "type": "string" },
                        "paired_stock": { "type": "string", "description": "the stock token this curve is paired with, e.g. \"TSLA\"" },
                        "stock_amount": { "type": "string", "description": "plain decimal amount of the paired stock token to spend, e.g. \"10\"" }
                    },
                    "required": ["curve_address", "paired_stock", "stock_amount"]
                }),
            },
            crate::llm::ToolSpec {
                name: "curve_status",
                description: "TESTNET ONLY, read-only: the curve's current live price (paired \
                    stock token per new token, read straight from the contract's own pricing \
                    function — never estimate this yourself), how much of the paired stock \
                    token has been raised against its graduation threshold as a percentage, \
                    whether it's graduated yet, and the pool address once it has. Price and \
                    progress are only meaningful before graduation — once graduated, trading \
                    has moved to the pool and this stops pricing new trades. curve_address must \
                    be an address launch_token already returned this conversation.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "curve_address": { "type": "string" },
                        "paired_stock": { "type": "string", "description": "the stock token this curve is paired with, e.g. \"TSLA\"" }
                    },
                    "required": ["curve_address", "paired_stock"]
                }),
            },
        ]);
    }
    tools
}

/// Flattens an `Outcome` into the text a model reads back as a `tool_result`
/// — same numbers the card will show, phrased as plain text instead of
/// widget layout, so the model's final reply can't disagree with the card
/// underneath it.
fn tool_result_text(outcome: &Outcome) -> String {
    match outcome {
        Outcome::Token { info, .. } => info.brief(),
        Outcome::Screen { rows, window, .. } => screen_brief(rows, *window),
        Outcome::Market { rows, sort, limit } => {
            let mut out = format!("Whole crypto market, ranked by {}:\n", sort.label());
            for r in rows.iter().take(*limit) {
                let chg = r.change_24h.map(|c| format!("{c:+.2}%")).unwrap_or_else(|| "n/a".into());
                out.push_str(&format!(
                    "- {} · {} · 24h {chg}\n",
                    r.symbol,
                    market::money_price(r.price),
                ));
            }
            out
        }
        Outcome::Price { chain, symbol, address, usd } => {
            format!("{symbol} on {chain} ({address}) = {}", market::money_price(*usd))
        }
        Outcome::Quote { chain, quote, route_note } => {
            let (sent, got) = Outcome::legs(quote);
            let route = route_path_words(&quote.token_in.symbol, &quote.token_out.symbol, &quote.route_hops)
                .map(|p| format!(" [route: {p}]"))
                .unwrap_or_default();
            let note = if route_note.is_empty() { String::new() } else { format!(" — {route_note}") };
            format!(
                "{sent} {} -> {got} {} on {chain} (status: {}){route}{note}",
                quote.token_in.symbol,
                quote.token_out.symbol,
                quote.status,
            )
        }
        Outcome::Chat { reply, .. } => reply.clone(),
        Outcome::Stock { symbol, name, price_usd, change_24h, .. } => {
            let chg = change_24h.map(|c| format!(", 24h {c:+.2}%")).unwrap_or_default();
            match name {
                Some(n) => format!(
                    "{symbol} ({n}) on Ondo Finance = {}{chg} (reference price, not tradable \
                     from this app)",
                    market::money_price(*price_usd)
                ),
                None => format!(
                    "{symbol} on Ondo Finance = {}{chg} (reference price, not tradable from \
                     this app)",
                    market::money_price(*price_usd)
                ),
            }
        }
        Outcome::StockScreen { rows } => {
            let mut out = String::from(
                "Ondo watchlist, ranked by today's volume vs each stock's own average (1.0x = \
                 normal). Also shown: 52-week range position (0% = at the 52w low, 100% = at \
                 the 52w high):\n",
            );
            for r in rows {
                let ratio = r
                    .volume_ratio()
                    .map(|x| format!("{x:.2}x average volume"))
                    .unwrap_or_else(|| "no volume reading".to_string());
                let range = r
                    .range_position()
                    .map(|p| format!("{:.0}% of 52w range", p * 100.0))
                    .unwrap_or_else(|| "no 52w range".to_string());
                out.push_str(&format!(
                    "- {} · {} · {ratio} · {range}\n",
                    r.symbol,
                    market::money_price(r.price_usd),
                ));
            }
            out
        }
        Outcome::ChainHeat { rows } => {
            let mut out = String::from(
                "Robinhood Chain, ranked by the last hour's volume against its own flat daily \
                 pace (1.0x = normal). Also shown: buy pressure over the last hour (share of \
                 trades that were buys, not sells), and 24h volume against the pool's own \
                 liquidity (a high number means trades here move price more per dollar than a \
                 deeper pool doing the same volume — a fact about the pool, not a verdict on \
                 the token):\n",
            );
            for r in rows {
                let heat = r
                    .heat_ratio()
                    .map(|x| format!("{x:.2}x hourly pace"))
                    .unwrap_or_else(|| "no volume reading".to_string());
                let pressure = r
                    .buy_pressure_h1()
                    .map(|p| format!("{:.0}% buys", p * 100.0))
                    .unwrap_or_else(|| "no trades this hour".to_string());
                let vol_liq = r
                    .volume_to_liquidity()
                    .map(|x| format!("{x:.1}x volume/liquidity"))
                    .unwrap_or_else(|| "no liquidity reading".to_string());
                out.push_str(&format!(
                    "- {} · {} · {heat} · {pressure} · {vol_liq}\n",
                    r.symbol,
                    market::money_price(r.price_usd),
                ));
            }
            out
        }
        Outcome::HoldingsDigest { rows } if rows.is_empty() => {
            "None of the wallet's holdings are currently on the Robinhood Chain trending board."
                .to_string()
        }
        Outcome::HoldingsDigest { rows } => {
            let mut out = String::from(
                "Wallet holdings that are currently on the Robinhood Chain trending board, \
                 busiest first:\n",
            );
            for d in rows {
                let chg = d.row.change_h24.map(|c| format!("{c:+.2}%")).unwrap_or_else(|| "n/a".into());
                out.push_str(&format!(
                    "- {} · holding {} · {} · 24h {chg} · liquidity {}\n",
                    d.row.symbol,
                    market::token_amount(&d.balance),
                    market::money_price(d.row.price_usd),
                    market::money_compact(d.row.liquidity_usd),
                ));
            }
            out
        }
        Outcome::Dividend {
            ticker,
            yield_frac,
            payout_frequency,
            last_cash_amount,
            last_payment_date,
            multiplier_growth_1y,
        } => {
            let growth = multiplier_growth_1y
                .map(|g| format!(" Over the last year, the on-chain shares multiplier that \
                     absorbs reinvested dividends grew {:.2}% — real recorded history, not an \
                     estimate.", g * 100.0))
                .unwrap_or_default();
            match (yield_frac, payout_frequency.as_deref()) {
                (Some(y), Some(freq)) => format!(
                    "{ticker} dividend: {:.2}% annualized yield, paid {freq}. Last payment: {}{}.{growth}",
                    y * 100.0,
                    last_cash_amount.map(|c| format!("${c:.2}/share")).unwrap_or_else(|| "unknown amount".into()),
                    last_payment_date.as_deref().map(|d| format!(" on {d}")).unwrap_or_default(),
                ),
                _ => format!(
                    "{ticker}: no current dividend data on file (either it doesn't pay one, or \
                     Ondo hasn't recorded one for it).{growth}"
                ),
            }
        }
        Outcome::TokenLaunch { token_symbol, token_name, paired_symbol, curve_address, tx_hash } => {
            format!(
                "launched {token_symbol} (\"{token_name}\") on Robinhood Chain testnet, paired \
                 with {paired_symbol} (tx {tx_hash}). curve_address = {curve_address} — remember \
                 this exact address, it's required for every buy_on_curve/curve_status call on \
                 this token from now on."
            )
        }
        Outcome::CurveBuy { curve_address, paired_symbol, stock_in, min_tokens_out, tx_hash } => {
            format!(
                "bought on curve {curve_address}: spent {stock_in} {paired_symbol}, minimum \
                 {min_tokens_out} tokens out after slippage (tx {tx_hash})"
            )
        }
        Outcome::CurveStatus {
            curve_address,
            paired_symbol,
            raised,
            graduation_threshold,
            graduated,
            pool_address,
            price_per_token,
            progress_pct,
        } => {
            if *graduated {
                format!(
                    "curve {curve_address}: graduated — {raised} {paired_symbol} raised (threshold \
                     was {graduation_threshold}). Trading now happens on the pool at {}.",
                    pool_address.as_deref().unwrap_or("unknown"),
                )
            } else {
                let price = price_per_token
                    .map(|p| format!(" — current price ~{} {paired_symbol} per token", fmt_price(p)))
                    .unwrap_or_default();
                let pct = progress_pct.map(|p| format!(" ({p:.1}% of the way there)")).unwrap_or_default();
                format!(
                    "curve {curve_address}: {raised} of {graduation_threshold} {paired_symbol} \
                     raised{pct} — not graduated yet, still trading on the curve.{price}"
                )
            }
        }
    }
}

/// Runs one tool call and reports what happened two ways: the text the model
/// reads back (`tool_result` content), and — when the call actually pulled
/// real data — the same `Outcome` shape the rest of the UI already knows how
/// to render, stashed in `last_card` for `ask_model` to attach to the turn's
/// final reply. Only the last successful call in a turn survives there:
/// one card per turn is the existing UX, same as before this was a tool loop.
/// Reads a balance straight off the chain — never from anywhere else. `ETH`/
/// `native`/empty reads the chain's native balance (`eth_getBalance`);
/// anything else resolves an ERC-20 address (the curated quote tokens first,
/// then Dexscreener for anything else, same as `lookup_token`), reads its
/// `decimals()` (falling back to 18 if that call fails — most tokens are 18,
/// and a wrong balance from a wrong decimals guess is still better than no
/// answer at all, clearly hedged as a fallback rather than presented as
/// exact), and calls `balanceOf`.
fn wallet_balance_text(wallet: Option<&str>, ticker: &str) -> String {
    fn inner(owner: &str, ticker: &str) -> Result<String, String> {
        let ticker = ticker.trim().trim_start_matches('$');
        let chain_id = 4663; // Robinhood Chain — the only chain a local wallet is imported for
        if ticker.is_empty() || ticker.eq_ignore_ascii_case("ETH") || ticker.eq_ignore_ascii_case("native")
        {
            let raw = chain_rpc::native_balance(chain_id, owner)?;
            return Ok(format!("{} ETH", tokens::format_units(raw, 18)));
        }
        let (address, decimals) =
            match tokens::chain_by_name(trending::CHAIN_NAME).and_then(|c| c.token(ticker)) {
                Some(t) => (t.address.to_string(), t.decimals),
                None => {
                    let info = trending::lookup(ticker)?;
                    let decimals = chain_rpc::eth_call(chain_id, &info.address, erc20::DECIMALS_CALL)
                        .map(|hex| erc20::decode_uint256(&hex) as u32)
                        .unwrap_or(18);
                    (info.address, decimals)
                }
            };
        let call_data = erc20::encode_balance_of(owner)?;
        let hex = chain_rpc::eth_call(chain_id, &address, &call_data)?;
        Ok(format!(
            "{} {}",
            tokens::format_units(erc20::decode_uint256(&hex), decimals),
            ticker.to_uppercase()
        ))
    }
    match wallet {
        None => "no wallet imported yet — the user needs to click \"Import wallet\" first".to_string(),
        Some(owner) => match inner(owner, ticker) {
            Ok(s) => s,
            Err(e) => format!("error: {e}"),
        },
    }
}

/// The agent's own path into `run_local_swap_job` — same function the manual
/// Swap button calls, same `pending_confirm` gate, same ten-minute timeout.
/// Called from the tool-loop thread (`ask_model`'s job closure), which is
/// already a background thread, so blocking here on the human's answer to
/// the confirmation panel is exactly the pattern `run_swap` already uses,
/// just reached from a different door.
#[allow(clippy::too_many_arguments)]
fn execute_swap_text(
    input: &serde_json::Value,
    wallet: Option<&str>,
    api_key: &str,
    ai_key: &str,
    busy: bool,
    pending: std::sync::Arc<std::sync::Mutex<Option<PendingConfirm>>>,
    swap_step: std::sync::Arc<std::sync::Mutex<SwapStep>>,
) -> String {
    let Some(sender) = wallet else {
        return "no wallet imported yet — the user needs to click \"Import wallet\" first".into();
    };
    if busy {
        return "a swap is already in progress in this app — wait for it to finish first".into();
    }
    let Some(chain) = tokens::chain_by_name(trending::CHAIN_NAME) else {
        return "error: chain not configured".into();
    };
    let token_in = input["token_in"].as_str().unwrap_or_default().trim();
    let token_out_ticker = input["token_out"].as_str().unwrap_or_default().trim();
    let amount = input["amount"].as_str().unwrap_or_default().trim();
    if token_in.is_empty() || token_out_ticker.is_empty() || amount.is_empty() {
        return "error: token_in, token_out and amount are all required".into();
    }
    let Some((in_address, in_decimals)) = resolve_source_token(chain, token_in) else {
        return format!(
            "error: \"{token_in}\" isn't a token this wallet can swap from here — check \
             wallet_balance for what it actually holds"
        );
    };
    let info = match trending::lookup(token_out_ticker) {
        Ok(i) => i,
        Err(e) => return format!("error: couldn't find {token_out_ticker} on Robinhood Chain — {e}"),
    };
    let amount_raw = match tokens::parse_units(amount, in_decimals) {
        Ok(0) => return "error: amount is zero".into(),
        Ok(v) => v,
        Err(e) => return format!("error: {e}"),
    };

    let set = move |s: SwapStep| *swap_step.lock().unwrap() = s;
    match run_local_swap_job(
        chain.id,
        in_address,
        &info.address,
        amount_raw,
        sender,
        api_key,
        ai_key,
        info.symbol.clone(),
        set,
        pending,
    ) {
        Ok(done) => format!(
            "swap sent: {} {} -> {} {} (tx {})",
            market::token_amount(&done.sent),
            done.sent_symbol,
            market::token_amount(&done.expected_out),
            done.got_symbol,
            done.tx_hash,
        ),
        Err(e) => format!("swap failed: {e}"),
    }
}

/// One curated-registry token the wallet holds a nonzero balance of — the
/// candidate list for the swap card's "from" picker, built by asking the
/// chain directly rather than through any indexer. Scoped to the curated
/// list on purpose, the same reason `tokens.rs` exists: a token this app
/// doesn't already know isn't offered as a swap source without being looked
/// up first.
#[derive(Clone)]
struct Holding {
    symbol: &'static str,
    address: &'static str,
    decimals: u32,
    balance_raw: u128,
}

fn scan_wallet_holdings(owner: &str) -> Vec<Holding> {
    let Some(chain) = tokens::chain_by_name(trending::CHAIN_NAME) else { return Vec::new() };
    // Native ETH first, checked separately from the curated list below: it
    // deliberately has no `tokens.rs` entry for this chain (a price lookup
    // 404s on the NATIVE sentinel, even though quotes and swaps route fine
    // through it), so it would never otherwise show up as something the
    // wallet can swap from — even when it's the only thing the wallet holds.
    let native = chain_rpc::native_balance(chain.id, owner).ok().filter(|&b| b > 0).map(|balance_raw| {
        Holding { symbol: "ETH", address: tokens::NATIVE, decimals: 18, balance_raw }
    });
    native
        .into_iter()
        .chain(chain.tokens.iter().filter_map(|t| {
            let call_data = erc20::encode_balance_of(owner).ok()?;
            let hex = chain_rpc::eth_call(chain.id, t.address, &call_data).ok()?;
            let balance_raw = erc20::decode_uint256(&hex);
            (balance_raw > 0).then_some(Holding {
                symbol: t.symbol,
                address: t.address,
                decimals: t.decimals,
                balance_raw,
            })
        }))
        .collect()
}

/// One held token that's also on the trending board right now — a
/// `holdings_digest` row. `row` is the untouched trending data (same shape
/// `chain_screen`/`ChainHeat` already render), `balance` is the held amount
/// formatted for display.
struct DigestRow {
    balance: String,
    row: trending::Row,
}

/// Joins what the wallet holds against what's trending right now, by
/// address — a held token that's simply not on the trending board isn't an
/// error, it just doesn't make the list. Sorted by 24h volume, busiest
/// first, same "top of the list is the answer" shape as `Market`/`ChainHeat`.
fn holdings_digest(owner: &str) -> Result<Outcome, String> {
    let holdings = scan_wallet_holdings(owner);
    if holdings.is_empty() {
        return Ok(Outcome::HoldingsDigest { rows: Vec::new() });
    }
    let trending_rows = trending::fetch(intent::SCREEN_LIMIT_MAX, trending::Window::H24)?;
    let mut rows: Vec<DigestRow> = holdings
        .into_iter()
        .filter_map(|h| {
            trending_rows.iter().find(|r| r.address.eq_ignore_ascii_case(h.address)).map(|r| DigestRow {
                balance: tokens::format_units(h.balance_raw, h.decimals),
                row: r.clone(),
            })
        })
        .collect();
    rows.sort_by(|a, b| b.row.volume_h24.partial_cmp(&a.row.volume_h24).unwrap_or(std::cmp::Ordering::Equal));
    Ok(Outcome::HoldingsDigest { rows })
}

/// Resolves the swap's chosen source symbol to an (address, decimals) pair —
/// native ETH as a special case `Chain::token` can never answer, since
/// Robinhood Chain's curated list has no entry for it (see
/// `scan_wallet_holdings`), and everything else through the normal
/// curated-registry lookup.
fn resolve_source_token(chain: &'static tokens::Chain, symbol: &str) -> Option<(&'static str, u32)> {
    if symbol.eq_ignore_ascii_case("ETH") {
        return Some((tokens::NATIVE, 18));
    }
    chain.token(symbol).map(|t| (t.address, t.decimals))
}

/// The one chain the whole bonding-curve launch feature targets (Phase C) —
/// a distinct `tokens.rs` entry from mainnet Robinhood, never reachable
/// through price/quote/swap.
fn testnet_chain() -> Result<&'static tokens::Chain, String> {
    tokens::chain_by_name("Robinhood Testnet")
        .ok_or_else(|| "Robinhood Chain testnet isn't configured".to_string())
}

/// The agent's door into launching a bonding-curve token (`launch_token`) —
/// a state-changing call, so like `execute_swap_text` it goes through
/// `PendingConfirm`/`local_send_transaction` rather than the generic `run()`
/// match, and is special-cased in `dispatch_tool` ahead of it.
fn launch_token_text(
    input: &serde_json::Value,
    wallet: Option<&str>,
    factory_address: &str,
    busy: bool,
    pending: std::sync::Arc<std::sync::Mutex<Option<PendingConfirm>>>,
    last_card: &std::cell::RefCell<Option<Outcome>>,
) -> String {
    let Some(sender) = wallet else {
        return "no wallet imported yet — the user needs to click \"Import wallet\" first".into();
    };
    if busy {
        return "a swap or launch is already in progress in this app — wait for it to finish first"
            .into();
    }
    let name = input["name"].as_str().unwrap_or_default().trim().to_string();
    let symbol = input["symbol"].as_str().unwrap_or_default().trim().to_string();
    let paired_stock = input["paired_stock"].as_str().unwrap_or_default().trim().to_string();
    let threshold_human = input["graduation_threshold"].as_str().unwrap_or_default().trim();
    if name.is_empty() || symbol.is_empty() || paired_stock.is_empty() || threshold_human.is_empty() {
        return "error: name, symbol, paired_stock and graduation_threshold are all required".into();
    }
    let chain = match testnet_chain() {
        Ok(c) => c,
        Err(e) => return format!("error: {e}"),
    };
    let Some(stock) = chain.token(&paired_stock) else {
        return format!(
            "error: \"{paired_stock}\" isn't a known Robinhood Chain testnet stock token ({})",
            chain.symbols().join(", ")
        );
    };
    let threshold_raw = match tokens::parse_units(threshold_human, stock.decimals) {
        Ok(0) => return "error: graduation_threshold is zero".into(),
        Ok(v) => v,
        Err(e) => return format!("error: {e}"),
    };
    let calldata = match bonding::encode_launch(&name, &symbol, stock.address, threshold_raw) {
        Ok(c) => c,
        Err(e) => return format!("error: {e}"),
    };

    // Simulate the exact same call first (`eth_call`, no state change) to
    // read back `launch`'s own return value — the deployed curve's address
    // — before spending real gas on it. Safe for a single-user testnet
    // flow: the deployment address only depends on the factory's own
    // nonce, which nothing else here advances between this read and the
    // real send below.
    let curve_address =
        chain_rpc::eth_call(chain.id, factory_address, &calldata).map(|hex| bonding::decode_address(&hex));

    match local_send_transaction(
        chain.id,
        sender,
        factory_address,
        &calldata,
        0,
        "Launch token",
        format!("{symbol} paired with {paired_stock}"),
        None,
        &pending,
    ) {
        Ok(tx_hash) => {
            let outcome = Outcome::TokenLaunch {
                token_symbol: symbol,
                token_name: name,
                paired_symbol: stock.symbol.to_string(),
                curve_address: curve_address.unwrap_or_default(),
                tx_hash,
            };
            let text = tool_result_text(&outcome);
            *last_card.borrow_mut() = Some(outcome);
            text
        }
        Err(e) => format!("launch failed: {e}"),
    }
}

/// The agent's door into buying on an already-launched curve
/// (`buy_on_curve`) — same allowance-check-then-send shape as
/// `run_local_swap_job`'s approve leg, just against the curve instead of
/// Sushi's router, and against `bonding::encode_preview_buy`/`encode_buy`
/// instead of the aggregator's own calldata.
fn buy_on_curve_text(
    input: &serde_json::Value,
    wallet: Option<&str>,
    busy: bool,
    pending: std::sync::Arc<std::sync::Mutex<Option<PendingConfirm>>>,
    last_card: &std::cell::RefCell<Option<Outcome>>,
) -> String {
    let Some(sender) = wallet else {
        return "no wallet imported yet — the user needs to click \"Import wallet\" first".into();
    };
    if busy {
        return "a swap or launch is already in progress in this app — wait for it to finish first"
            .into();
    }
    let curve_address = input["curve_address"].as_str().unwrap_or_default().trim().to_string();
    let paired_stock = input["paired_stock"].as_str().unwrap_or_default().trim().to_string();
    let amount_human = input["stock_amount"].as_str().unwrap_or_default().trim().to_string();
    if curve_address.is_empty() || paired_stock.is_empty() || amount_human.is_empty() {
        return "error: curve_address, paired_stock and stock_amount are all required".into();
    }
    let chain = match testnet_chain() {
        Ok(c) => c,
        Err(e) => return format!("error: {e}"),
    };
    let Some(stock) = chain.token(&paired_stock) else {
        return format!("error: \"{paired_stock}\" isn't a known Robinhood Chain testnet stock token");
    };
    let stock_in = match tokens::parse_units(&amount_human, stock.decimals) {
        Ok(0) => return "error: stock_amount is zero".into(),
        Ok(v) => v,
        Err(e) => return format!("error: {e}"),
    };

    // Preview first — the curve's own `previewBuy` is the only place this
    // price is computed; nothing here re-derives it from raw reserves.
    let preview_call = bonding::encode_preview_buy(stock_in);
    let expected_out = match chain_rpc::eth_call(chain.id, &curve_address, &preview_call) {
        Ok(hex) => bonding::decode_uint256(&hex),
        Err(e) => return format!("error: couldn't preview that buy — {e}"),
    };
    if expected_out == 0 {
        return "error: previewed output is zero — check the curve address and amount".into();
    }
    let min_tokens_out = (expected_out as f64 * (1.0 - DEFAULT_SLIPPAGE)) as u128;

    let allowance_call = match erc20::encode_allowance(sender, &curve_address) {
        Ok(c) => c,
        Err(e) => return format!("error: {e}"),
    };
    let current_allowance = match chain_rpc::eth_call(chain.id, stock.address, &allowance_call) {
        Ok(hex) => erc20::decode_uint256(&hex),
        Err(e) => return format!("error: {e}"),
    };
    if current_allowance < stock_in {
        let approve_data = match erc20::encode_approve(&curve_address, MAX_APPROVAL) {
            Ok(c) => c,
            Err(e) => return format!("error: {e}"),
        };
        let approve_hash = match local_send_transaction(
            chain.id,
            sender,
            stock.address,
            &approve_data,
            0,
            "Approve token spending",
            format!("infinite {} allowance", stock.symbol),
            None,
            &pending,
        ) {
            Ok(h) => h,
            Err(e) => return format!("approval failed: {e}"),
        };
        if let Err(e) = chain_rpc::wait_for_receipt(chain.id, &approve_hash, APPROVAL_TIMEOUT) {
            return format!("approval didn't confirm: {e}");
        }
    }

    let buy_data = bonding::encode_buy(stock_in, min_tokens_out);
    match local_send_transaction(
        chain.id,
        sender,
        &curve_address,
        &buy_data,
        0,
        "Buy on curve",
        format!("{amount_human} {}", stock.symbol),
        None,
        &pending,
    ) {
        Ok(tx_hash) => {
            let outcome = Outcome::CurveBuy {
                curve_address,
                paired_symbol: stock.symbol.to_string(),
                stock_in: amount_human,
                min_tokens_out: tokens::format_units(min_tokens_out, 18),
                tx_hash,
            };
            let text = tool_result_text(&outcome);
            *last_card.borrow_mut() = Some(outcome);
            text
        }
        Err(e) => format!("buy failed: {e}"),
    }
}

/// Read-only curve check (`curve_status`) — no signing, so unlike
/// launch/buy it fits the generic `run()`-less match in `dispatch_tool`
/// directly rather than needing its own special case.
fn curve_status(input: &serde_json::Value) -> Result<Outcome, String> {
    let curve_address = input["curve_address"].as_str().unwrap_or_default().trim().to_string();
    let paired_stock = input["paired_stock"].as_str().unwrap_or_default().trim().to_string();
    if curve_address.is_empty() || paired_stock.is_empty() {
        return Err("curve_address and paired_stock are both required".to_string());
    }
    let chain = testnet_chain()?;
    let Some(stock) = chain.token(&paired_stock) else {
        return Err(format!("\"{paired_stock}\" isn't a known Robinhood Chain testnet stock token"));
    };
    let raised = chain_rpc::eth_call(chain.id, &curve_address, bonding::RAISED_CALL)
        .map(|h| bonding::decode_uint256(&h))?;
    let threshold = chain_rpc::eth_call(chain.id, &curve_address, bonding::GRADUATION_THRESHOLD_CALL)
        .map(|h| bonding::decode_uint256(&h))?;
    let graduated = chain_rpc::eth_call(chain.id, &curve_address, bonding::GRADUATED_CALL)
        .map(|h| bonding::decode_bool(&h))?;
    let (pool_address, price_per_token, progress_pct) = if graduated {
        let pool = chain_rpc::eth_call(chain.id, &curve_address, bonding::POOL_CALL)
            .map(|h| bonding::decode_address(&h))
            .ok();
        (pool, None, None)
    } else {
        // Spot price, read the same way any bonding-curve UI reads one: a
        // tiny reference buy (0.001 of the paired stock — small enough next
        // to the curve's virtual reserves that it barely moves the price,
        // big enough that the output doesn't round down to zero) run
        // through the curve's own `previewBuy`, never recomputed from raw
        // reserves here.
        const PRICE_SAMPLE_RAW: u128 = 1_000_000_000_000_000; // 0.001 token, 18 decimals
        let price = chain_rpc::eth_call(chain.id, &curve_address, &bonding::encode_preview_buy(PRICE_SAMPLE_RAW))
            .map(|h| bonding::decode_uint256(&h))
            .ok()
            .filter(|&tokens_out| tokens_out > 0)
            .map(|tokens_out| PRICE_SAMPLE_RAW as f64 / tokens_out as f64);
        let progress = (threshold > 0).then(|| raised as f64 / threshold as f64 * 100.0);
        (None, price, progress)
    };
    Ok(Outcome::CurveStatus {
        curve_address,
        paired_symbol: stock.symbol.to_string(),
        raised: tokens::format_units(raised, stock.decimals),
        graduation_threshold: tokens::format_units(threshold, stock.decimals),
        graduated,
        pool_address,
        price_per_token,
        progress_pct,
    })
}

#[allow(clippy::too_many_arguments)]
fn dispatch_tool(
    name: &str,
    input: &serde_json::Value,
    api_key: &str,
    ondo_key: &str,
    ai_key: &str,
    wallet: Option<&str>,
    swap_busy: bool,
    pending_confirm: std::sync::Arc<std::sync::Mutex<Option<PendingConfirm>>>,
    swap_step: std::sync::Arc<std::sync::Mutex<SwapStep>>,
    bonding_cfg: &bonding::BondingConfig,
    last_card: &std::cell::RefCell<Option<Outcome>>,
) -> String {
    if name == "wallet_balance" {
        return wallet_balance_text(wallet, input["ticker"].as_str().unwrap_or(""));
    }
    if name == "execute_swap" {
        return execute_swap_text(input, wallet, api_key, ai_key, swap_busy, pending_confirm, swap_step);
    }
    if name == "launch_token" {
        if !bonding_cfg.use_testnet {
            return "error: bonding-curve launches aren't enabled — the user needs to turn on \
                \"Enable on Robinhood Chain testnet\" in the agent's settings first"
                .to_string();
        }
        if bonding_cfg.factory_address.trim().is_empty() {
            return "error: no bonding-curve factory address configured yet in settings".to_string();
        }
        return launch_token_text(
            input,
            wallet,
            &bonding_cfg.factory_address,
            swap_busy,
            pending_confirm,
            last_card,
        );
    }
    if name == "buy_on_curve" {
        if !bonding_cfg.use_testnet {
            return "error: bonding-curve trading isn't enabled — the user needs to turn on \
                \"Enable on Robinhood Chain testnet\" in the agent's settings first"
                .to_string();
        }
        return buy_on_curve_text(input, wallet, swap_busy, pending_confirm, last_card);
    }
    let result: Result<Outcome, String> = match name {
        "curve_status" => curve_status(input),
        "market_overview" => {
            let sort = input["sort"].as_str().and_then(market::Sort::parse).unwrap_or(market::Sort::Volume);
            let limit = input["limit"].as_u64().unwrap_or(10).clamp(1, market::LIMIT_MAX as u64) as usize;
            run(Intent::Market { sort, limit }, api_key, "", "")
        }
        "screen_chain" => {
            let limit = input["limit"].as_u64().unwrap_or(20).clamp(1, intent::SCREEN_LIMIT_MAX as u64) as usize;
            let window = input["window"].as_str().and_then(trending::Window::parse).unwrap_or(trending::Window::H24);
            run(Intent::ChainScreen { limit, window }, api_key, "", "")
        }
        "chain_screen" => {
            let limit = input["limit"].as_u64().unwrap_or(8).clamp(1, 15) as usize;
            trending::screen_heat(limit).map(|rows| Outcome::ChainHeat { rows })
        }
        "lookup_token" => {
            let ticker = input["ticker"].as_str().unwrap_or_default().trim().trim_start_matches('$').to_string();
            if ticker.is_empty() {
                Err("no ticker given".to_string())
            } else {
                run(Intent::TokenLookup { ticker }, api_key, "", "")
            }
        }
        "get_price" => (|| {
            let chain = intent::chain_of(input)?;
            let token = intent::token_of(chain, input, "token")?;
            run(Intent::Price { chain, token }, api_key, "", "")
        })(),
        "get_quote" => intent::quote_of(input).and_then(|i| run(i, api_key, ai_key, "")),
        "lookup_stock" => {
            let ticker = input["ticker"].as_str().unwrap_or_default().trim().to_string();
            if ticker.is_empty() {
                Err("no ticker given".to_string())
            } else {
                run(Intent::StockLookup { ticker }, "", "", ondo_key)
            }
        }
        "lookup_dividend" => {
            let ticker = input["ticker"].as_str().unwrap_or_default().trim().to_string();
            if ticker.is_empty() {
                Err("no ticker given".to_string())
            } else {
                run(Intent::DividendLookup { ticker }, "", "", ondo_key)
            }
        }
        "ondo_screen" => {
            let limit = input["limit"].as_u64().unwrap_or(8).clamp(1, 15) as usize;
            let mut rows = ondo::screen();
            rows.truncate(limit);
            Ok(Outcome::StockScreen { rows })
        }
        "holdings_digest" => match wallet {
            None => Err("no wallet imported yet — the user needs to click \"Import wallet\" first".to_string()),
            Some(owner) => holdings_digest(owner),
        },
        other => Err(format!("unknown tool {other}")),
    };
    match result {
        Ok(outcome) => {
            let text = tool_result_text(&outcome);
            *last_card.borrow_mut() = Some(outcome);
            text
        }
        Err(e) => format!("error: {e}"),
    }
}

/// Approvals typically land in one or two blocks; five minutes is generous
/// headroom for a slow one without leaving the UI waiting indefinitely on a
/// stuck one.
const APPROVAL_TIMEOUT: Duration = Duration::from_secs(300);
/// "Infinite" approval — the standard practice so the same token doesn't ask
/// to be approved again on the next swap. The user sees exactly this amount
/// in the confirmation panel before it's signed.
const MAX_APPROVAL: u128 = u128::MAX;

/// The whole swap sequence, off the UI thread: quote, allowance check, an
/// approval if one is needed (with a wait for it to actually be mined before
/// the next step trusts it), then the swap itself. Every read goes straight
/// to the chain's own RPC (`chain_rpc`), and every signature is preceded by
/// a `PendingConfirm` the UI has to explicitly answer before this ever
/// reaches for `signer::sign_via_worker` — the only place a key is touched.
#[allow(clippy::too_many_arguments)]
fn run_local_swap_job(
    chain_id: u64,
    token_in: &str,
    token_out: &str,
    amount_in: u128,
    sender: &str,
    api_key: &str,
    ai_key: &str,
    symbol: String,
    set_step: impl Fn(SwapStep),
    pending: std::sync::Arc<std::sync::Mutex<Option<PendingConfirm>>>,
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

    // A failed narration must never surface as an error on a screen where
    // the user is about to sign something — `.ok()`, not `.unwrap_or_else`.
    let route_note = if ai_key.trim().is_empty() {
        None
    } else {
        use crate::llm::LlmProvider;
        let brief =
            route_brief(&swap.token_in.symbol, &swap.token_out.symbol, &swap.route_hops, swap.price_impact);
        crate::llm::Anthropic::new(ai_key.to_string()).complete(ROUTE_SYSTEM, &brief).ok()
    };

    let is_native = token_in.eq_ignore_ascii_case(tokens::NATIVE);
    if !is_native {
        set_step(SwapStep::CheckingAllowance);
        let call_data = erc20::encode_allowance(sender, &swap.tx.to)?;
        let raw = chain_rpc::eth_call(chain_id, token_in, &call_data)?;
        let current = erc20::decode_uint256(&raw);

        if current < amount_in {
            set_step(SwapStep::WaitingOnApproval);
            let approve_data = erc20::encode_approve(&swap.tx.to, MAX_APPROVAL)?;
            let approve_hash = local_send_transaction(
                chain_id,
                sender,
                token_in,
                &approve_data,
                0,
                "Approve token spending",
                format!("infinite {} allowance", swap.token_in.symbol),
                None,
                &pending,
            )?;
            set_step(SwapStep::ConfirmingApproval);
            chain_rpc::wait_for_receipt(chain_id, &approve_hash, APPROVAL_TIMEOUT)?;
        }
    }

    set_step(SwapStep::WaitingOnSwap);
    let amount_display = format!(
        "{} {}",
        market::token_amount(&tokens::format_units(swap.amount_in, swap.token_in.decimals)),
        swap.token_in.symbol
    );
    let tx_hash = local_send_transaction(
        chain_id,
        sender,
        &swap.tx.to,
        &swap.tx.data,
        swap.tx.value,
        "Swap",
        amount_display,
        route_note,
        &pending,
    )?;

    Ok(SwapDone {
        tx_hash,
        sent: tokens::format_units(swap.amount_in, swap.token_in.decimals),
        sent_symbol: swap.token_in.symbol,
        expected_out: tokens::format_units(swap.amount_out, swap.token_out.decimals),
        price_impact: swap.price_impact,
        got_symbol: if symbol.is_empty() { swap.token_out.symbol } else { symbol },
    })
}

/// Prices gas, parks the transaction in `pending` for the UI to show, and
/// blocks until it's answered — cancelled, timed out (ten minutes: long
/// enough that stepping away doesn't auto-cancel, short enough that a
/// forgotten prompt doesn't leak a thread forever), or confirmed, in which
/// case only then does this reach for `signer::sign_via_worker`.
#[allow(clippy::too_many_arguments)]
fn local_send_transaction(
    chain_id: u64,
    from: &str,
    to: &str,
    data: &str,
    value: u128,
    label: &'static str,
    amount_display: String,
    route_note: Option<String>,
    pending: &std::sync::Arc<std::sync::Mutex<Option<PendingConfirm>>>,
) -> Result<String, String> {
    let nonce = chain_rpc::nonce(chain_id, from)?;
    let fees = chain_rpc::fees(chain_id)?;
    let gas_limit = chain_rpc::estimate_gas(chain_id, from, to, data, value)?;
    let fee_native = (gas_limit as u128 * fees.max_fee_per_gas) as f64 / 1e18;

    let (reply_tx, reply_rx) = std::sync::mpsc::sync_channel::<bool>(1);
    *pending.lock().unwrap() = Some(PendingConfirm {
        label,
        amount: amount_display,
        to: to.to_string(),
        estimated_fee_native: format!("~{fee_native:.6} ETH"),
        route_note,
        reply: reply_tx,
    });

    let confirmed = match reply_rx.recv_timeout(Duration::from_secs(600)) {
        Ok(v) => v,
        Err(_) => {
            *pending.lock().unwrap() = None;
            return Err("confirmation timed out — nothing was signed".into());
        }
    };
    if !confirmed {
        return Err("cancelled".into());
    }

    let unsigned = signer::UnsignedTx {
        chain_id,
        nonce,
        max_priority_fee_per_gas: fees.max_priority_fee_per_gas,
        max_fee_per_gas: fees.max_fee_per_gas,
        gas_limit,
        to: to.to_string(),
        value,
        data: data.to_string(),
    };
    let signed = signer::sign_via_worker(&unsigned)?;
    chain_rpc::send_raw_transaction(chain_id, &signed)
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
            let _ = tx.send(trending::fetch(TRENDING_ROWS, trending::Window::H24));
        });
        self.trending_rx = Some(rx);
        ctx.request_repaint();
    }

    /// No key required — `ondo::market` falls through to Hyperium's shared
    /// proxy for a blank key, which is what every row here passes. The
    /// dashboard shows the busy end of `ondo::screen`'s ranking — whatever's
    /// actually moving right now across all of Ondo's assets — rather than a
    /// fixed roster, so it isn't the same ten mega-caps on a quiet day for
    /// them and a busy one somewhere else.
    fn refresh_ondo(&mut self, ctx: &egui::Context) {
        if self.ondo_rx.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut rows = ondo::screen();
            rows.truncate(ONDO_DASHBOARD_ROWS);
            let _ = tx.send(rows);
        });
        self.ondo_rx = Some(rx);
        ctx.request_repaint();
    }

    /// The whole swap, start to finish, on one worker thread: quote, allowance
    /// check, an approval if one is needed (with a wait for it to actually be
    /// mined before the next step trusts it), then the swap itself, all
    /// against the chain's own RPC. Every signature is preceded by an
    /// in-app `PendingConfirm` the UI has to answer before this thread
    /// reaches for `signer.rs` — the only place a key is touched.
    fn run_swap(&mut self, symbol: String, token_out_address: String, ctx: &egui::Context) {
        if self.swap_rx.is_some() {
            return;
        }
        let Some(sender) = self.wallet.clone() else { return };
        let Some(robinhood) = tokens::chain_by_name(trending::CHAIN_NAME) else { return };
        let Some((source_address, source_decimals)) = resolve_source_token(robinhood, &self.source_symbol)
        else {
            return;
        };
        let amount_raw = match tokens::parse_units(self.swap_amount.trim(), source_decimals) {
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
        let token_in = source_address.to_string();
        let api_key = self.sushi_key.clone();
        let ai_key = self.ai_key.clone();
        let step = self.swap_step.clone();
        let set = move |s: SwapStep| *step.lock().unwrap() = s;
        let pending = self.pending_confirm.clone();

        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = run_local_swap_job(
                chain_id,
                &token_in,
                &token_out_address,
                amount_raw,
                &sender,
                &api_key,
                &ai_key,
                symbol,
                set,
                pending,
            );
            let _ = tx.send(result);
        });
        self.swap_rx = Some(rx);
        self.swap_result = None;
        ctx.request_repaint();
    }

    /// The local-signer path's entire safety net: nothing gets signed until
    /// this is answered. Modal on purpose — the rest of the panel is
    /// unreachable while a transaction is waiting.
    fn confirm_modal(&mut self, ctx: &egui::Context) {
        // Snapshot out of the mutex before drawing anything — the reply
        // sender clones cheaply (it's a handle, not the channel itself), so
        // nothing here needs to hold the lock across a closure the borrow
        // checker can't see into.
        let snapshot = {
            let guard = self.pending_confirm.lock().unwrap();
            guard.as_ref().map(|p| {
                (
                    p.label,
                    p.amount.clone(),
                    p.to.clone(),
                    p.estimated_fee_native.clone(),
                    p.route_note.clone(),
                    p.reply.clone(),
                )
            })
        };
        let Some((label, amount, to, fee, route_note, reply)) = snapshot else { return };

        let mut decision: Option<bool> = None;
        egui::Modal::new(egui::Id::new("sushi_local_confirm")).show(ctx, |ui| {
            ui.set_max_width(360.0);
            // `Modal`'s own backdrop doesn't reliably pick up this app's dark
            // theme — without an explicit fill here, the light text below
            // (styled for the dark cards used everywhere else) can land on
            // whatever light default background the modal falls back to.
            egui::Frame::default()
                .fill(BG_ELEVATED)
                .corner_radius(10.0)
                .inner_margin(egui::Margin::same(16))
                .show(ui, |ui| {
                    ui.label(RichText::new(label).color(ACCENT).strong());
                    ui.add_space(10.0);
                    row(ui, "Amount", &amount);
                    row(ui, "To", &short_addr(&to));
                    row(ui, "Est. network fee", &fee);
                    if let Some(note) = &route_note {
                        ui.add_space(8.0);
                        ui.label(RichText::new(note).color(FAINT).small().italics());
                    }
                    ui.add_space(14.0);
                    ui.label(
                        RichText::new(
                            "Signed locally with your imported key — nothing else can see it.",
                        )
                        .color(FAINT)
                        .small(),
                    );
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if tool_button(ui, "Cancel", true) {
                            decision = Some(false);
                        }
                        if tool_button(ui, "Confirm & Sign", true) {
                            decision = Some(true);
                        }
                    });
                });
        });

        if let Some(d) = decision {
            let _ = reply.try_send(d);
            *self.pending_confirm.lock().unwrap() = None;
        }
    }

    /// Re-quotes the swap amount against Sushi's router a short beat after
    /// the field stops changing, so the amount/button row always has a real
    /// "what would I get" underneath it instead of asking the user to click
    /// Swap to find out. Keyed on (token, amount text): switching to a fresh
    /// lookup or editing the amount drops the stale answer immediately
    /// rather than showing yesterday's numbers under a new question.
    fn maybe_preview_quote(&mut self, token_out: &str, ctx: &egui::Context) {
        let amount = self.swap_amount.trim().to_string();
        let key = (token_out.to_string(), self.source_symbol.clone(), amount.clone());
        if self.preview_key.as_ref() != Some(&key) {
            self.preview_key = Some(key);
            self.preview = None;
            self.preview_err = None;
            self.preview_rx = None;
            self.preview_dirty_since = Some(Instant::now());
            self.guardian = None;
            self.guardian_rx = None;
            self.guardian_key = None;
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
        let Some((source_address, source_decimals)) = resolve_source_token(robinhood, &self.source_symbol)
        else {
            return;
        };
        let amount_raw = match tokens::parse_units(&amount, source_decimals) {
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
        let token_in = source_address.to_string();
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

    /// Scans the wallet's balance across the curated token list once per
    /// wallet address (import or switch), not every frame — the "from"
    /// picker just reads whatever `holdings` last resolved to, stale by at
    /// most one scan's worth of chain state.
    fn maybe_fetch_holdings(&mut self, ctx: &egui::Context) {
        let Some(owner) = self.wallet.clone() else { return };
        if let Some(rx) = &self.holdings_rx {
            if let Ok(list) = rx.try_recv() {
                self.holdings = list;
                self.holdings_rx = None;
            }
            return;
        }
        if self.holdings_for.as_deref() == Some(owner.as_str()) {
            return;
        }
        self.holdings_for = Some(owner.clone());
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let list = scan_wallet_holdings(&owner);
            let _ = tx.send(list);
            ctx.request_repaint();
        });
        self.holdings_rx = Some(rx);
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
            run(Intent::TokenLookup { ticker }, &api_key, &ai_key, "")
        });
    }

    /// Natural language path: a real tool-calling conversation, not a
    /// classify-then-execute step. Real message history (`agent_messages`)
    /// rides along, not a hand-summarized recap, so a follow-up like "and
    /// the second one" has the model's own past turns to resolve against.
    /// The model decides when a question needs a tool; every number in its
    /// final reply came out of one, never out of the model's own memory.
    fn ask_model(&mut self, ctx: &egui::Context) {
        let question = self.ask.trim().to_string();
        if question.is_empty() {
            return;
        }
        self.ask.clear();
        let ai_key = self.ai_key.clone();
        let api_key = self.sushi_key.clone();
        let ondo_key = self.ondo_key.clone();
        let wallet = self.wallet.clone();
        let messages = self.agent_messages.clone();
        // A swap already running (from the card's own button, or a previous
        // execute_swap call) is checked here, once, before the thread starts
        // — not inside `execute_swap_text` on every call — since this is the
        // one place that still has `self.swap_rx` to ask.
        let swap_busy = self.swap_rx.is_some();
        let pending_confirm = self.pending_confirm.clone();
        let swap_step = self.swap_step.clone();
        let bonding_cfg = self.bonding.clone();
        self.spawn(ctx, question.clone(), move || {
            let last_card = std::cell::RefCell::new(None);
            // Sonnet, not the default Haiku: this agent has to reliably call a
            // tool for every number rather than answer from memory, and Haiku
            // was caught doing exactly that (a guessed ETH/USD conversion).
            let ai_key_for_tools = ai_key.clone();
            let anthropic = crate::llm::Anthropic::new(ai_key).with_model("claude-sonnet-5");
            let tools = agent_tools(bonding_cfg.use_testnet);
            let mut guard = messages.lock().unwrap();
            guard.push(serde_json::json!({ "role": "user", "content": question }));
            let system = agent_system_prompt(bonding_cfg.use_testnet);
            let reply = anthropic.converse(&system, &mut guard, &tools, |name, input| {
                dispatch_tool(
                    name,
                    input,
                    &api_key,
                    &ondo_key,
                    &ai_key_for_tools,
                    wallet.as_deref(),
                    swap_busy,
                    pending_confirm.clone(),
                    swap_step.clone(),
                    &bonding_cfg,
                    &last_card,
                )
            })?;
            drop(guard);
            Ok(Outcome::Chat { reply, card: last_card.into_inner().map(Box::new) })
        });
    }

    fn record(&mut self, outcome: &Result<Outcome, String>) {
        let (text, ok) = match outcome {
            Ok(Outcome::Price { chain, symbol, usd, .. }) => {
                (format!("{symbol} on {chain} = ${usd}"), true)
            }
            Ok(Outcome::Quote { chain, quote, .. }) => {
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
            Ok(Outcome::Stock { symbol, .. }) => (format!("read {symbol} on Ondo"), true),
            Ok(Outcome::StockScreen { rows }) => {
                (format!("screened {} Ondo watchlist symbols by volume", rows.len()), true)
            }
            Ok(Outcome::ChainHeat { rows }) => {
                (format!("screened {} Robinhood Chain tokens by heat", rows.len()), true)
            }
            Ok(Outcome::HoldingsDigest { rows }) => {
                (format!("found {} held token(s) currently trending", rows.len()), true)
            }
            Ok(Outcome::Dividend { ticker, .. }) => (format!("read {ticker}'s dividend info"), true),
            Ok(Outcome::TokenLaunch { token_symbol, paired_symbol, .. }) => {
                (format!("launched {token_symbol} paired with {paired_symbol} (testnet)"), true)
            }
            Ok(Outcome::CurveBuy { paired_symbol, stock_in, .. }) => {
                (format!("bought on curve with {stock_in} {paired_symbol} (testnet)"), true)
            }
            Ok(Outcome::CurveStatus { curve_address, graduated, .. }) => (
                format!(
                    "checked curve {curve_address} status ({})",
                    if *graduated { "graduated" } else { "still curving" }
                ),
                true,
            ),
            Ok(Outcome::Chat { card, .. }) => (
                match card.as_deref() {
                    Some(Outcome::Token { info, .. }) => format!("chatted, read {}", info.symbol),
                    Some(_) => "chatted, pulled data".to_string(),
                    None => "chatted".to_string(),
                },
                true,
            ),
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
                        Ok(o) => {
                            // `ask_model` (the free-text path) keeps
                            // `agent_messages` current itself, inside
                            // `converse`. A click-through lookup
                            // (`look_up`/`look_up_stock` — a ticker box or a
                            // table row, no model call involved) never
                            // touches it at all, so without this a follow-up
                            // typed right after clicking a row — "what do
                            // you think about it" — lands with no memory of
                            // what "it" was. Not every outcome needs an AI
                            // read of its own to still be worth remembering.
                            if !matches!(o, Outcome::Chat { .. }) {
                                let text = tool_result_text(&o);
                                let mut guard = self.agent_messages.lock().unwrap();
                                guard.push(
                                    serde_json::json!({ "role": "user", "content": question.clone() }),
                                );
                                guard.push(
                                    serde_json::json!({ "role": "assistant", "content": text }),
                                );
                            }
                            self.chat.push(ChatTurn { question, answer: ChatAnswer::Result(o) });
                        }
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
                    note_flashes(&mut self.market_flash, &self.market, &rows, |r| &r.symbol, |r| r.price);
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
                    note_flashes(
                        &mut self.trending_flash,
                        &self.trending,
                        &rows,
                        |r| &r.symbol,
                        |r| r.price_usd,
                    );
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
        if let Some(rx) = &self.ondo_rx {
            match rx.try_recv() {
                Ok(rows) => {
                    note_flashes(&mut self.ondo_flash, &self.ondo, &rows, |r| &r.symbol, |r| r.price_usd);
                    self.ondo_err = if rows.is_empty() { Some("no symbols answered".into()) } else { None };
                    self.ondo = rows;
                    self.ondo_at = Some(Instant::now());
                    self.ondo_rx = None;
                }
                Err(TryRecvError::Empty) => ui.ctx().request_repaint(),
                Err(TryRecvError::Disconnected) => self.ondo_rx = None,
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
                    self.maybe_start_guardian(&q, ui.ctx());
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
        if let Some(rx) = &self.guardian_rx {
            match rx.try_recv() {
                Ok(reading) => {
                    self.guardian = Some(reading);
                    self.guardian_rx = None;
                }
                Err(TryRecvError::Empty) => ui.ctx().request_repaint(),
                // A failed/aborted guardian thread just leaves `guardian`
                // `None` — `token_card` already treats that as "fall back
                // to the flat threshold", not an error to surface.
                Err(TryRecvError::Disconnected) => self.guardian_rx = None,
            }
        }
    }

    /// Fires the two extra `/quote` calls (10x, 100x) only once the base 1x
    /// quote already shows some impact — a clean swap never pays for the
    /// curve check. Reuses the 1x impact already in hand rather than a third
    /// redundant call for it.
    fn maybe_start_guardian(&mut self, q: &api::Quote, ctx: &egui::Context) {
        const GUARDIAN_FLOOR: f64 = 0.005;
        let impact_1x = q.price_impact;
        if impact_1x.map(f64::abs).unwrap_or(0.0) <= GUARDIAN_FLOOR {
            return;
        }
        let Some(key) = self.preview_key.clone() else { return };
        if self.guardian_key.as_ref() == Some(&key) {
            return; // already fetched (or fetching) a reading for this exact quote
        }
        let Some(robinhood) = tokens::chain_by_name(trending::CHAIN_NAME) else { return };
        let amount_1x = q.amount_in;
        let chain_id = robinhood.id;
        let token_in = q.token_in.address.clone();
        let token_out = q.token_out.address.clone();
        let api_key = self.sushi_key.clone();

        self.guardian = None;
        self.guardian_key = Some(key);
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let leg = |mult: u128| -> Option<f64> {
                let amount = amount_1x.checked_mul(mult)?;
                api::quote(
                    &api::QuoteRequest {
                        chain_id,
                        token_in: token_in.clone(),
                        token_out: token_out.clone(),
                        amount,
                        max_slippage: DEFAULT_SLIPPAGE,
                    },
                    &api_key,
                )
                .ok()?
                .price_impact
            };
            let impact_10x = leg(10);
            let impact_100x = leg(100);
            let reading = guardian::Reading {
                grade: guardian::grade(impact_1x, impact_10x, impact_100x),
                impact_1x,
                impact_10x,
                impact_100x,
            };
            let _ = tx.send(reading);
        });
        self.guardian_rx = Some(rx);
        ctx.request_repaint();
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

        if !self.trending_flash.is_empty() {
            ui.ctx().request_repaint();
        }
        trending_header(ui);
        let mut picked = None;
        for (i, row) in self.trending.iter().enumerate() {
            let flash = self.trending_flash.get(&row.symbol);
            if trending_row(ui, i, row, flash) {
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

        if !self.market_flash.is_empty() {
            ui.ctx().request_repaint();
        }
        table_header(ui);
        for (i, row) in self.market.iter().enumerate() {
            let flash = self.market_flash.get(&row.symbol);
            market_row(ui, i, row, flash);
        }
    }

    /// Ondo Stocks — reference prices only (see `ondo.rs`): no swap card,
    /// click a row to look it up. The rows themselves are the busy end of
    /// `ondo::screen`'s ranking, not a fixed list — see `refresh_ondo`.
    fn ondo_section(&mut self, ui: &mut egui::Ui) {
        let loading = self.ondo_rx.is_some();

        ui.horizontal(|ui| {
            ui.label(RichText::new("ONDO STOCKS").color(ACCENT).small().strong());
            ui.label(RichText::new("most active right now · reference only").color(FAINT).small());
            if loading {
                ui.add(egui::Spinner::new().size(13.0).color(ACCENT));
            } else if let Some(at) = self.ondo_at {
                ui.label(RichText::new(ago(at.elapsed().as_secs())).color(FAINT).small());
            }
            if tool_button(ui, "Refresh", !loading) && !loading {
                let ctx = ui.ctx().clone();
                self.refresh_ondo(&ctx);
            }
        });
        ui.add_space(2.0);
        ui.label(
            RichText::new(
                "The busiest of Ondo's on-chain stock trackers right now, ranked by today's \
                 volume against each one's own average — read through Hyperium's own shared \
                 key, so this works with no Ondo API key of your own. Prices drift slightly \
                 above the real stock over time (dividends compound in rather than pay out); \
                 nothing here is tradable from this app yet.",
            )
            .color(DIM)
            .small(),
        );
        ui.add_space(8.0);

        if let Some(e) = &self.ondo_err {
            ui.label(RichText::new(format!("✗ {e}")).color(RED).small());
        }
        if self.ondo.is_empty() {
            if self.ondo_err.is_none() {
                ui.label(RichText::new("loading…").color(FAINT).small());
            }
            return;
        }
        if !self.ondo_flash.is_empty() {
            ui.ctx().request_repaint();
        }

        ondo_header(ui);
        let mut picked = None;
        for (i, row) in self.ondo.iter().enumerate() {
            let flash = self.ondo_flash.get(&row.symbol);
            if ondo_row(ui, i, row, flash) {
                picked = Some(row.symbol.clone());
            }
        }
        if let Some(sym) = picked {
            let ctx = ui.ctx().clone();
            self.look_up_stock(&ctx, sym);
        }
    }

    /// Ondo's own lookup path — same shape as `look_up`, but for
    /// `StockLookup` rather than `TokenLookup`, and reachable by clicking a
    /// symbol in either Ondo table (`ondo_section`, `ondo_row`) instead of
    /// typing a ticker. Runs with a real AI key when one's configured, same
    /// as `look_up`, so clicking a name gets a real one-line take, not a
    /// bare number.
    fn look_up_stock(&mut self, ctx: &egui::Context, ticker: String) {
        let ai_key = self.ai_key.clone();
        let ondo_key = self.ondo_key.clone();
        let label = format!("${ticker} (Ondo)");
        self.spawn(ctx, label, move || {
            run(Intent::StockLookup { ticker }, "", &ai_key, &ondo_key)
        });
    }

    /// The last turn's card, if it was a stock lookup — same "reuse the
    /// chat turn, skip the bubble" idea as `robinhood_card`, just without
    /// any wallet/swap state to thread through since there's nothing
    /// interactive on an Ondo card.
    fn ondo_card(&mut self, ui: &mut egui::Ui) {
        if self.rx.is_some() {
            ui.horizontal(|ui| thinking_line(ui, self.busy_since, ui.input(|i| i.time)));
            return;
        }
        let Some(turn) = self.chat.last() else { return };
        let ChatAnswer::Result(outcome) = &turn.answer else { return };
        let Some(stock) = as_stock_outcome(outcome) else { return };
        egui::Frame::NONE
            .fill(BG_ELEVATED)
            .corner_radius(12.0)
            .inner_margin(egui::Margin::same(14))
            .show(ui, |ui| result_card_inner(ui, stock));
        ui.add_space(14.0);
    }
}

impl Tool for SushiTool {
    fn title(&self) -> &'static str {
        "AI Agent"
    }
    fn about(&self) -> &'static str {
        "Chat, trade on Robinhood Chain, and check Ondo's tokenized stocks — one agent, three tabs."
    }
    fn uses_output_dir(&self) -> bool {
        false
    }

    fn ui(&mut self, ui: &mut egui::Ui, octx: &ToolCtx) {
        if !self.loaded {
            self.ai_key = crate::secret::load_secret(&octx.config_dir.join("anthropic.key"));
            self.sushi_key = crate::secret::load_secret(&sushi_key_path(octx.config_dir));
            self.ondo_key = crate::secret::load_secret(&ondo_key_path(octx.config_dir));
            self.bonding = bonding::load_config(octx.config_dir);
            // No "connect" handshake needed — if a key's already imported,
            // the wallet is simply already here.
            self.wallet = signer::address(octx.config_dir);
            self.loaded = true;
            let ctx = ui.ctx().clone();
            self.refresh_market(&ctx);
            self.refresh_trending(&ctx);
            self.refresh_ondo(&ctx);
        }

        self.poll(ui);
        self.confirm_modal(ui.ctx());
        if self.pending_confirm.lock().unwrap().is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(200));
        }

        // Keep every table warm without the user asking, whichever tab they
        // are actually looking at.
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
        let ondo_stale = self.ondo_at.map(|t| t.elapsed() >= MARKET_TTL).unwrap_or(false);
        if ondo_stale && self.ondo_rx.is_none() {
            let ctx = ui.ctx().clone();
            self.refresh_ondo(&ctx);
        }
        if self.market_at.is_some() || self.trending_at.is_some() || self.ondo_at.is_some() {
            ui.ctx().request_repaint_after(Duration::from_secs(1));
        }
        self.maybe_fetch_holdings(&ui.ctx().clone());

        self.tab_bar(ui);

        match self.active_tab {
            Tab::Chat => {
                // The composer is pinned to the bottom of the panel, chat-app
                // style — it needs to be claimed before the scroll area below
                // so it always gets its strip regardless of how tall the
                // conversation grows. Only mounted on this tab: the other two
                // have their own, narrower ways in (a ticker box, nothing).
                egui::Panel::bottom("sushi_composer")
                    .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(0, 10)))
                    .show_separator_line(true)
                    .show_inside(ui, |ui| {
                        self.composer(ui, octx);
                    });

                egui::ScrollArea::vertical().id_salt("chat_scroll").show(ui, |ui| {
                    self.chat_thread(ui);
                    ui.add_space(14.0);
                    self.log_and_settings(ui, octx);
                });
            }
            Tab::Robinhood => {
                egui::ScrollArea::vertical().id_salt("robinhood_scroll").show(ui, |ui| {
                    self.robinhood_lookup_bar(ui);
                    self.robinhood_card(ui);

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
            Tab::Ondo => {
                egui::ScrollArea::vertical().id_salt("ondo_scroll").show(ui, |ui| {
                    self.ondo_card(ui);
                    self.ondo_section(ui);
                });
            }
        }
    }
}

impl SushiTool {
    fn tab_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for (tab, label) in
                [(Tab::Chat, "Chat"), (Tab::Robinhood, "Robinhood"), (Tab::Ondo, "Ondo")]
            {
                let active = self.active_tab == tab;
                let text = RichText::new(label).color(if active { ACCENT } else { DIM }).strong();
                if ui.selectable_label(active, text).clicked() {
                    self.active_tab = tab;
                }
            }
        });
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(8.0);
    }

    /// Robinhood tab's own way in — a bare ticker box, the same fast path
    /// `submit` already gives a plain ticker (no model call, works with no AI
    /// key). The free-text composer stays on the Chat tab; this dashboard
    /// only ever asks one kind of question.
    fn robinhood_lookup_bar(&mut self, ui: &mut egui::Ui) {
        let busy = self.rx.is_some();
        ui.horizontal(|ui| {
            let field = ui.add_enabled(
                !busy,
                egui::TextEdit::singleline(&mut self.ticker)
                    .hint_text("look up a ticker — \"PONS\", \"WETH\"…")
                    .desired_width(240.0),
            );
            let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let ready = !self.ticker.trim().is_empty() && !busy;
            if (tool_button(ui, "Look up", ready) || (entered && ready)) && ready {
                let ctx = ui.ctx().clone();
                self.look_up(&ctx);
            }
            if busy {
                ui.add(egui::Spinner::new().size(13.0).color(ACCENT));
            }
        });
        ui.add_space(10.0);
    }

    /// The last turn's card, if it was a token lookup — the same swap card
    /// `chat_thread` shows inline, just without the question/answer bubbles
    /// around it, since this tab is the dashboard, not the transcript. Chat
    /// questions still land in `self.chat` either way, so switching to the
    /// Chat tab shows the exact same lookup as part of the conversation.
    fn robinhood_card(&mut self, ui: &mut egui::Ui) {
        if self.rx.is_some() {
            ui.horizontal(|ui| thinking_line(ui, self.busy_since, ui.input(|i| i.time)));
            return;
        }
        let Some(turn) = self.chat.last() else { return };
        let ChatAnswer::Result(outcome) = &turn.answer else { return };
        if as_token(outcome).is_none() {
            return;
        }

        let step = self.swap_step.lock().unwrap().clone();
        let action = egui::Frame::NONE
            .fill(BG_ELEVATED)
            .corner_radius(12.0)
            .inner_margin(egui::Margin::same(14))
            .show(ui, |ui| {
                render_outcome(
                    ui,
                    outcome,
                    self.wallet.as_deref(),
                    &mut self.swap_amount,
                    self.swap_rx.is_some(),
                    step.label(),
                    &self.swap_result,
                    self.preview.as_ref(),
                    self.preview_err.as_deref().filter(|e| !e.is_empty()),
                    self.preview_rx.is_some(),
                    self.guardian.as_ref(),
                    true,
                    &self.holdings,
                    &mut self.source_symbol,
                )
            })
            .inner;
        ui.add_space(14.0);

        if let Some(action) = action {
            let ctx = ui.ctx().clone();
            self.run_swap(action.symbol, action.token_out, &ctx);
        } else if self.wallet.is_some() && self.swap_rx.is_none() {
            if let Some(info) = as_token(outcome) {
                let addr = info.address.clone();
                let ctx = ui.ctx().clone();
                self.maybe_preview_quote(&addr, &ctx);
            }
        }
    }

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
                         sends on its own: you confirm every send yourself, right here.",
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
                        // Token/Screen/Chat's own text fills this role
                        // instead, so nothing doubles up.
                        let reply = chat_reply(outcome);
                        if !reply.is_empty() {
                            ui.label(RichText::new(reply).color(FG));
                            ui.add_space(6.0);
                        }
                        let interactive = i == last_idx;
                        let step = self.swap_step.lock().unwrap().clone();
                        let a = render_outcome(
                            ui,
                            outcome,
                            self.wallet.as_deref(),
                            &mut self.swap_amount,
                            self.swap_rx.is_some(),
                            step.label(),
                            &self.swap_result,
                            self.preview.as_ref(),
                            self.preview_err.as_deref().filter(|e| !e.is_empty()),
                            self.preview_rx.is_some(),
                            self.guardian.as_ref(),
                            interactive,
                            &self.holdings,
                            &mut self.source_symbol,
                        );
                        if interactive {
                            swap_action = a;
                            if let Some(info) = as_token(outcome) {
                                if self.wallet.is_some() && self.swap_rx.is_none() {
                                    preview_for = Some(info.address.clone());
                                }
                            }
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

        // `CollapsingHeader` rather than a hand-rolled arrow+label: it
        // highlights on hover and shows a pointing-hand cursor for free,
        // which a static label with a click `Sense` bolted on doesn't — that
        // silence read as "not clickable" and is exactly what buried this
        // section before.
        egui::CollapsingHeader::new(RichText::new("Sushi API key (optional)").color(FAINT).small())
            .id_salt("sushi_key_section")
            .show(ui, |ui| {
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
            });

        egui::CollapsingHeader::new(
            RichText::new("Ondo Stocks API key (optional)").color(FAINT).small(),
        )
        .id_salt("ondo_key_section")
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.ondo_key)
                        .password(true)
                        .hint_text("x-api-key…")
                        .desired_width(240.0),
                );
                if tool_button(ui, "Save", true) {
                    crate::secret::save_secret(&ondo_key_path(octx.config_dir), &self.ondo_key);
                }
            });
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Reference prices for tokenized US stocks (TSLAon, AAPLon…). \
                        Not tradable from here yet — read-only.")
                        .color(FAINT)
                        .small(),
                );
                ui.hyperlink_to(
                    RichText::new("request access ↗").color(ACCENT).small(),
                    "https://docs.ondo.finance/api-reference/quickstart",
                );
            });
        });

        egui::CollapsingHeader::new(
            RichText::new("Bonding-curve launches (testnet, experimental)").color(FAINT).small(),
        )
        .id_salt("bonding_section")
        .show(ui, |ui| {
            ui.checkbox(&mut self.bonding.use_testnet, "Enable on Robinhood Chain testnet");
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.bonding.factory_address)
                        .hint_text("0x… deployed BondingCurveFactory address")
                        .desired_width(320.0),
                );
                if tool_button(ui, "Save", true) {
                    bonding::save_config(octx.config_dir, &self.bonding);
                }
            });
            ui.label(
                RichText::new(
                    "Lets the agent launch a token whose bonding-curve liquidity is a Robinhood \
                     Chain testnet stock token, and buy on it. Testnet only — no real funds, no \
                     mainnet factory exists yet.",
                )
                .color(FAINT)
                .small(),
            );
        });
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

        // Starters — always visible, not just before the first turn: the
        // agent picked up several screeners over time (chain_screen,
        // ondo_screen...) that are easy to forget are even there once a
        // conversation is under way, so the reminder stays up rather than
        // vanishing the moment it might actually be useful.
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("try").color(FAINT).small());
            let bonding_chip = self.bonding.use_testnet.then_some(&BONDING_EXAMPLE);
            for ex in EXAMPLES.iter().chain(bonding_chip) {
                let ready = has_ai && !busy;
                let clicked = match ex.hint {
                    Some(h) => tool_button_hint(ui, ex.text, ready, h),
                    None => tool_button(ui, ex.text, ready),
                };
                if clicked && ready {
                    self.ask = ex.text.to_string();
                    let ctx = ui.ctx().clone();
                    self.ask_model(&ctx);
                }
            }
        });
        ui.add_space(8.0);

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
            let wallet_snapshot = self.wallet.clone();
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                match &wallet_snapshot {
                    Some(addr) => {
                        if tool_button(ui, "forget", true) {
                            signer::clear_key(octx.config_dir);
                            self.wallet = None;
                        }
                        ui.label(RichText::new(short_addr(addr)).color(UP).small());
                        ui.label(RichText::new("●").color(UP).small());
                    }
                    None => {
                        if tool_button(ui, "Import wallet", true) {
                            self.wallet_key_open = !self.wallet_key_open;
                        }
                    }
                }
            });
        });
        if let Some(e) = &self.wallet_err {
            ui.label(RichText::new(format!("✗ {e}")).color(RED).small());
        }
        if self.wallet.is_none() && self.wallet_key_open {
            self.local_key_import(ui, octx);
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

    /// Import path for the local-signer backend. The key is validated,
    /// encrypted at rest (`signer::set_key`), and never kept in this field —
    /// only the derived address (public, harmless to hold in memory) does.
    fn local_key_import(&mut self, ui: &mut egui::Ui, octx: &ToolCtx) {
        egui::Frame::default()
            .fill(BG_ELEVATED)
            .corner_radius(10.0)
            .inner_margin(egui::Margin::same(12))
            .show(ui, |ui| {
                ui.label(
                    RichText::new(
                        "Encrypted at rest on this machine. You'll confirm the amount, address \
                         and gas yourself before anything is signed.",
                    )
                    .color(FAINT)
                    .small(),
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.wallet_key_input)
                            .password(true)
                            .hint_text("private key (hex)")
                            .desired_width(280.0),
                    );
                    let ready = !self.wallet_key_input.trim().is_empty();
                    if tool_button(ui, "Import", ready) && ready {
                        match signer::set_key(octx.config_dir, &self.wallet_key_input) {
                            Ok(addr) => {
                                self.wallet = Some(addr);
                                self.wallet_err = None;
                                self.wallet_key_open = false;
                            }
                            Err(e) => self.wallet_err = Some(e),
                        }
                        self.wallet_key_input.clear();
                    }
                });
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

fn market_row(ui: &mut egui::Ui, i: usize, r: &market::Row, flash: Option<&(Instant, bool)>) {
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
        flash_tint(FG, flash),
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

fn ondo_header(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(O_TABLE_W, 20.0), egui::Sense::hover());
    let p = ui.painter();
    let cy = rect.center().y;
    let f = FontId::proportional(10.5);
    let mut x = rect.left() + 8.0;

    x += O_RANK;
    p.text(egui::pos2(x, cy), egui::Align2::LEFT_CENTER, "TICKER", f.clone(), FAINT);
    x += O_SYM + O_NAME;
    x += O_PRICE;
    p.text(egui::pos2(x, cy), egui::Align2::RIGHT_CENTER, "PRICE", f.clone(), FAINT);
    x += O_CHG;
    p.text(egui::pos2(x, cy), egui::Align2::RIGHT_CENTER, "24H", f.clone(), FAINT);
    x += 12.0;
    p.text(egui::pos2(x, cy), egui::Align2::LEFT_CENTER, "TODAY", f, FAINT);
}

/// Returns true when the row was clicked — same gesture as `trending_row`:
/// a symbol is the thing worth asking about, so pointing at one should be
/// the whole gesture, not a separate button hunted for elsewhere.
fn ondo_row(ui: &mut egui::Ui, i: usize, r: &ondo::Market, flash: Option<&(Instant, bool)>) -> bool {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(O_TABLE_W, 26.0), egui::Sense::click());
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
    x += O_RANK;

    p.text(egui::pos2(x, cy), egui::Align2::LEFT_CENTER, &r.symbol, FontId::proportional(13.5), FG);
    x += O_SYM;

    p.text(
        egui::pos2(x, cy),
        egui::Align2::LEFT_CENTER,
        truncate(r.name.as_deref().unwrap_or("—"), 34),
        FontId::proportional(12.0),
        DIM,
    );
    x += O_NAME;

    x += O_PRICE;
    p.text(
        egui::pos2(x, cy),
        egui::Align2::RIGHT_CENTER,
        market::money_price(r.price_usd),
        num.clone(),
        flash_tint(FG, flash),
    );

    x += O_CHG;
    let (chg_text, chg_color) = match r.change_24h {
        Some(c) => (market::pct_signed(c), if c < 0.0 { RED } else { UP }),
        None => ("—".to_string(), FAINT),
    };
    p.text(egui::pos2(x, cy), egui::Align2::RIGHT_CENTER, chg_text, num, chg_color);

    let spark_rect = egui::Rect::from_min_size(
        egui::pos2(x + 12.0, rect.top() + 6.0),
        egui::vec2(O_SPARK - 16.0, rect.height() - 12.0),
    );
    sparkline(p, spark_rect, &r.spark, chg_color);
    resp.clicked()
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
fn trending_row(
    ui: &mut egui::Ui,
    i: usize,
    r: &trending::Row,
    flash: Option<&(Instant, bool)>,
) -> bool {
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
        flash_tint(FG, flash),
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

/// `sparkline`, but for a chat card rather than a table row: allocates its
/// own space in the current layout instead of being handed a rect a caller
/// already carved out of a fixed-width table. Same "no axes, no labels, \
/// shape only" idea, just reachable from `result_card_inner` where there is
/// no table to sit inside.
fn chart_widget(ui: &mut egui::Ui, spark: &[f32], color: Color32, size: egui::Vec2) {
    if spark.len() < 2 {
        return;
    }
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    sparkline(ui.painter(), rect, spark, color);
}

/// Diffs a table's previous snapshot against the one that just landed and
/// records who moved, keyed by symbol — called once, right when a refresh
/// arrives, rather than every frame. Entries older than `FLASH_DURATION` are
/// dropped here too, so the map never grows past whatever changed in the
/// last refresh or two.
fn note_flashes<R>(
    flash: &mut HashMap<String, (Instant, bool)>,
    old: &[R],
    new: &[R],
    symbol: impl Fn(&R) -> &str,
    price: impl Fn(&R) -> f64,
) {
    let prev: HashMap<&str, f64> = old.iter().map(|r| (symbol(r), price(r))).collect();
    let now = Instant::now();
    for r in new {
        if let Some(&p) = prev.get(symbol(r)) {
            let np = price(r);
            if (np - p).abs() > f64::EPSILON {
                flash.insert(symbol(r).to_string(), (now, np > p));
            }
        }
    }
    flash.retain(|_, (at, _)| at.elapsed() < FLASH_DURATION);
}

/// The flash colour for a row whose price just moved: full brightness the
/// instant it changes, fading linearly back to `base` over `FLASH_DURATION`
/// so the eye catches the change without the number staying tinted forever.
fn flash_tint(base: Color32, flash: Option<&(Instant, bool)>) -> Color32 {
    let Some((at, up)) = flash else { return base };
    let mag = (1.0 - at.elapsed().as_secs_f32() / FLASH_DURATION.as_secs_f32()).clamp(0.0, 1.0);
    let full = if *up { UP } else { RED };
    Color32::from_rgb(
        (base.r() as f32 + (full.r() as f32 - base.r() as f32) * mag) as u8,
        (base.g() as f32 + (full.g() as f32 - base.g() as f32) * mag) as u8,
        (base.b() as f32 + (full.b() as f32 - base.b() as f32) * mag) as u8,
    )
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

/// A label/value line in the confirmation modal — label dim and fixed-width,
/// value in the normal foreground colour, right where the eye expects a
/// number after reading "Amount".
fn row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).color(DIM).small());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(value).color(FG));
        });
    });
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
fn screen_card(ui: &mut egui::Ui, rows: &[trending::Row], take: &str, window: trending::Window) {
    ui.label(
        RichText::new(format!(
            "ROBINHOOD CHAIN · {} tokens · by {} volume",
            rows.len(),
            window.label()
        ))
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
        trending_row(ui, i, row, None);
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
/// The `Token` at the bottom of a possibly-nested `Chat.card` — peels
/// through however many `Chat` layers there are (there's only ever at most
/// one in practice) to find the swap-eligible token underneath, or `None`
/// if this turn's card wasn't a token lookup at all.
fn as_token(outcome: &Outcome) -> Option<&trending::Info> {
    match outcome {
        Outcome::Token { info, .. } => Some(info),
        Outcome::Chat { card: Some(inner), .. } => as_token(inner),
        _ => None,
    }
}

/// Same idea as `as_token`, for `ondo_card` — returns the `Outcome` itself
/// rather than unpacking it, since `result_card_inner` (unlike `token_card`)
/// takes the whole `Outcome` and needs no extra wallet/swap state alongside it.
fn as_stock_outcome(outcome: &Outcome) -> Option<&Outcome> {
    match outcome {
        Outcome::Stock { .. } => Some(outcome),
        Outcome::Chat { card: Some(inner), .. } => as_stock_outcome(inner),
        _ => None,
    }
}

/// Renders whatever card belongs under this turn's written line — a `Token`
/// gets the full interactive swap card (needs the wallet/swap state this
/// takes as plain parameters rather than `&mut self`, so the caller can
/// still hold `self.chat.iter()` borrowed while calling this), a `Chat`
/// unwraps to whatever it's carrying, and everything else goes through the
/// generic, non-interactive `result_card_inner`.
#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
fn render_outcome(
    ui: &mut egui::Ui,
    outcome: &Outcome,
    wallet: Option<&str>,
    swap_amount: &mut String,
    swap_busy: bool,
    swap_step: &str,
    swap_result: &Option<Result<SwapDone, String>>,
    preview: Option<&api::Quote>,
    preview_err: Option<&str>,
    preview_loading: bool,
    guardian: Option<&guardian::Reading>,
    interactive: bool,
    holdings: &[Holding],
    source_symbol: &mut String,
) -> Option<SwapAction> {
    match outcome {
        Outcome::Token { info, take } => token_card(
            ui,
            info,
            take,
            wallet,
            swap_amount,
            swap_busy,
            swap_step,
            swap_result,
            preview,
            preview_err,
            preview_loading,
            guardian,
            interactive,
            holdings,
            source_symbol,
        ),
        Outcome::Chat { card: Some(inner), .. } => render_outcome(
            ui,
            inner,
            wallet,
            swap_amount,
            swap_busy,
            swap_step,
            swap_result,
            preview,
            preview_err,
            preview_loading,
            guardian,
            interactive,
            holdings,
            source_symbol,
        ),
        Outcome::Chat { card: None, .. } => None,
        other => {
            result_card_inner(ui, other);
            None
        }
    }
}

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
    guardian: Option<&guardian::Reading>,
    interactive: bool,
    holdings: &[Holding],
    source_symbol: &mut String,
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
                // The candidate list is only ever what the wallet actually
                // holds among the curated tokens (`scan_wallet_holdings`) —
                // WETH stays selectable even at a zero balance since it's
                // the one token every pool here is guaranteed to pair
                // against, and picking it costs nothing to try.
                egui::ComboBox::from_id_salt(("swap_source", &info.address))
                    .selected_text(RichText::new(source_symbol.as_str()).color(DIM))
                    .show_ui(ui, |ui| {
                        if holdings.is_empty() {
                            ui.selectable_value(source_symbol, "WETH".to_string(), "WETH");
                        }
                        for h in holdings {
                            let label = format!(
                                "{} ({})",
                                h.symbol,
                                market::token_amount(&tokens::format_units(h.balance_raw, h.decimals))
                            );
                            ui.selectable_value(source_symbol, h.symbol.to_string(), label);
                        }
                    });
                ui.label(RichText::new("→").color(DIM));
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
                // Graded on the real 1x/10x/100x curve when the guardian has
                // one; falls straight back to the old flat 3% read whenever
                // it hasn't fired (below its floor), hasn't landed yet, or
                // failed — never a missing or broken state, just the
                // pre-guardian behavior.
                let (hot, warning) = match guardian {
                    Some(g) => (g.grade != guardian::Grade::Fine, Some(g.message())),
                    None if impact.abs() > 0.03 => {
                        (true, Some("thin pool — that's a real haircut, maybe size down"))
                    }
                    None => (false, None),
                };
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "≈ {} {} in the bag",
                            market::token_amount(&got),
                            q.token_out.symbol
                        ))
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
                if let Some(route) =
                    route_path_words(&q.token_in.symbol, &q.token_out.symbol, &q.route_hops)
                {
                    ui.label(RichText::new(route).color(FAINT).small());
                }
                // The number above is the router's own output amount, not a
                // separate price call — this is that same amount times the
                // token's already-fetched spot price, so it can't drift from
                // what the swap actually quotes.
                if info.price_usd > 0.0 {
                    if let Ok(units) = got.parse::<f64>() {
                        ui.label(
                            RichText::new(format!("≈ {}", market::money_compact(units * info.price_usd)))
                                .color(FAINT)
                                .small(),
                        );
                    }
                }
                if let Some(warning) = warning {
                    ui.label(RichText::new(warning).color(RED).small());
                }
            } else if let Some(e) = preview_err {
                ui.label(RichText::new(format!("no route — {e}")).color(FAINT).small());
            }
            ui.add_space(4.0);
            ui.label(
                RichText::new(
                    "You'll be asked to review and confirm the exact amount, address and gas \
                     before anything is signed.",
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
                        "✓ swapped {} {} for ~{} {}",
                        market::token_amount(&done.sent),
                        done.sent_symbol,
                        market::token_amount(&done.expected_out),
                        done.got_symbol
                    ))
                    .color(UP),
                );
                if info.price_usd > 0.0 {
                    if let Ok(units) = done.expected_out.parse::<f64>() {
                        ui.label(
                            RichText::new(format!("≈ {}", market::money_compact(units * info.price_usd)))
                                .color(FAINT)
                                .small(),
                        );
                    }
                }
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
            RichText::new("Connect a wallet above to swap into this token.").color(FAINT).small(),
        );
    }

    action
}

/// Robinhood Chain testnet's own Blockscout explorer — the bonding-curve
/// cards below link straight into it (an address or a tx) rather than just
/// printing the hex, since there's nowhere else in this app to look one up.
const TESTNET_EXPLORER: &str = "https://explorer.testnet.chain.robinhood.com";

fn testnet_explorer_address_url(addr: &str) -> String {
    format!("{TESTNET_EXPLORER}/address/{addr}")
}

fn testnet_explorer_tx_url(hash: &str) -> String {
    format!("{TESTNET_EXPLORER}/tx/{hash}")
}

/// Content only, no frame of its own — the caller (`assistant_bubble`)
/// already supplies the card, so this would otherwise double up.
fn result_card_inner(ui: &mut egui::Ui, outcome: &Outcome) {
    match outcome {
        // Token is rendered by `chat_thread` directly (it needs the wallet
        // and swap state this function isn't given) — rendering nothing here
        // keeps that invariant instead of panicking on it.
        Outcome::Token { .. } => {}
        // `chat_thread` reaches for `render_outcome` before this function
        // ever sees a `Chat` — this arm only exists for exhaustiveness, and
        // does the best it can (no wallet/swap state here) if it's ever hit
        // some other way.
        Outcome::Chat { card: Some(inner), .. } => result_card_inner(ui, inner),
        Outcome::Chat { card: None, .. } => {}
        Outcome::Market { rows, sort, limit } => {
            ui.label(
                RichText::new(format!("top {limit} · {}", sort.label())).color(FAINT).small(),
            );
            ui.add_space(6.0);
            table_header(ui);
            for (i, row) in rows.iter().enumerate() {
                market_row(ui, i, row, None);
            }
        }
        Outcome::Screen { rows, take, window } => screen_card(ui, rows, take, *window),
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
        Outcome::Quote { chain, quote, route_note } => {
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
            if let Some(route) =
                route_path_words(&quote.token_in.symbol, &quote.token_out.symbol, &quote.route_hops)
            {
                ui.label(RichText::new(route).color(DIM).small());
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
            if !route_note.is_empty() {
                ui.add_space(4.0);
                ui.label(RichText::new(route_note).color(FAINT).small().italics());
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
        Outcome::Stock { symbol, name, price_usd, change_24h, spark, take } => {
            ui.horizontal(|ui| {
                ui.label(RichText::new(symbol).color(FG).strong());
                ui.label(RichText::new("Ondo Finance").color(DIM).small());
            });
            if let Some(n) = name {
                ui.label(RichText::new(n).color(FAINT).small());
            }
            ui.label(
                RichText::new(market::money_price(*price_usd))
                    .color(ACCENT)
                    .font(FontId::monospace(20.0)),
            );
            let chg_color = match change_24h {
                Some(c) => {
                    ui.label(RichText::new(market::pct_signed(*c)).color(if *c >= 0.0 { UP } else { RED }).small());
                    if *c >= 0.0 { UP } else { RED }
                }
                None => ACCENT,
            };
            if spark.len() >= 2 {
                ui.add_space(4.0);
                chart_widget(ui, spark, chg_color, egui::vec2(260.0, 56.0));
            }
            if !take.is_empty() {
                ui.add_space(6.0);
                ui.label(RichText::new(take).color(FG));
            }
            ui.add_space(4.0);
            ui.label(
                RichText::new("Reference price only — not tradable from this app yet.")
                    .color(FAINT)
                    .small(),
            );
        }
        Outcome::StockScreen { rows } => {
            ui.label(
                RichText::new("ONDO WATCHLIST · by volume vs average").color(ACCENT).small().strong(),
            );
            ui.add_space(6.0);
            if rows.is_empty() {
                ui.label(RichText::new("nothing came back").color(FAINT).small());
            }
            for r in rows {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(&r.symbol)
                            .color(FG)
                            .font(FontId::monospace(13.5))
                            .strong(),
                    );
                    ui.label(
                        RichText::new(market::money_price(r.price_usd))
                            .color(DIM)
                            .font(FontId::monospace(12.5)),
                    );
                    let (text, color) = match r.volume_ratio() {
                        Some(x) if x >= 1.0 => (format!("{x:.2}x average"), UP),
                        Some(x) => (format!("{x:.2}x average"), DIM),
                        None => ("no volume reading".to_string(), FAINT),
                    };
                    ui.label(RichText::new(text).color(color).small());
                    if let Some(p) = r.range_position() {
                        ui.label(
                            RichText::new(format!("{:.0}% of 52w range", p * 100.0))
                                .color(if p >= 0.9 || p <= 0.1 { ORANGE } else { FAINT })
                                .small(),
                        );
                    }
                    if r.spark.len() >= 2 {
                        chart_widget(ui, &r.spark, color, egui::vec2(64.0, 18.0));
                    }
                });
            }
            ui.add_space(4.0);
            ui.label(
                RichText::new("Reference prices only — not tradable from this app yet.")
                    .color(FAINT)
                    .small(),
            );
        }
        Outcome::ChainHeat { rows } => {
            ui.label(
                RichText::new("ROBINHOOD CHAIN · by heat vs normal pace")
                    .color(ORANGE)
                    .small()
                    .strong(),
            );
            ui.add_space(6.0);
            if rows.is_empty() {
                ui.label(RichText::new("nothing came back").color(FAINT).small());
            }
            for r in rows {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(&r.symbol)
                            .color(FG)
                            .font(FontId::monospace(13.5))
                            .strong(),
                    );
                    ui.label(
                        RichText::new(market::money_price(r.price_usd))
                            .color(DIM)
                            .font(FontId::monospace(12.5)),
                    );
                    let (text, color) = match r.heat_ratio() {
                        Some(x) if x >= 1.0 => (format!("{x:.2}x hourly pace"), ORANGE),
                        Some(x) => (format!("{x:.2}x hourly pace"), DIM),
                        None => ("no volume reading".to_string(), FAINT),
                    };
                    ui.label(RichText::new(text).color(color).small());
                    if let Some(p) = r.buy_pressure_h1() {
                        ui.label(
                            RichText::new(format!("{:.0}% buys", p * 100.0))
                                .color(if p >= 0.5 { UP } else { RED })
                                .small(),
                        );
                    }
                    if let Some(vl) = r.volume_to_liquidity() {
                        ui.label(
                            RichText::new(format!("{vl:.1}x vol/liq"))
                                .color(if vl >= 3.0 { ORANGE } else { FAINT })
                                .small(),
                        );
                    }
                });
            }
        }
        Outcome::HoldingsDigest { rows } => {
            ui.label(
                RichText::new("YOUR HOLDINGS · currently trending").color(ACCENT).small().strong(),
            );
            ui.add_space(6.0);
            if rows.is_empty() {
                ui.label(
                    RichText::new("none of your holdings are on the trending board right now")
                        .color(FAINT)
                        .small(),
                );
            }
            for d in rows {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(&d.row.symbol)
                            .color(FG)
                            .font(FontId::monospace(13.5))
                            .strong(),
                    );
                    ui.label(
                        RichText::new(format!("holding {}", market::token_amount(&d.balance)))
                            .color(DIM)
                            .small(),
                    );
                    ui.label(
                        RichText::new(market::money_price(d.row.price_usd))
                            .color(DIM)
                            .font(FontId::monospace(12.5)),
                    );
                    if let Some(c) = d.row.change_h24 {
                        ui.label(
                            RichText::new(format!("{c:+.2}% 24h")).color(if c >= 0.0 { UP } else { RED }).small(),
                        );
                    }
                    ui.label(
                        RichText::new(format!("liq {}", market::money_compact(d.row.liquidity_usd)))
                            .color(FAINT)
                            .small(),
                    );
                });
            }
        }
        Outcome::Dividend {
            ticker,
            yield_frac,
            payout_frequency,
            last_cash_amount,
            last_payment_date,
            multiplier_growth_1y,
        } => {
            ui.horizontal(|ui| {
                ui.label(RichText::new(ticker).color(FG).strong());
                ui.label(RichText::new("Dividend · Ondo Finance").color(DIM).small());
            });
            ui.add_space(4.0);
            match (yield_frac, payout_frequency.as_deref()) {
                (Some(y), Some(freq)) if freq != "none" => {
                    ui.label(
                        RichText::new(format!("{:.2}% annualized yield", y * 100.0))
                            .color(ACCENT)
                            .font(FontId::monospace(18.0)),
                    );
                    ui.label(RichText::new(format!("paid {freq}")).color(DIM).small());
                    if let Some(c) = last_cash_amount {
                        let when = last_payment_date
                            .as_deref()
                            .map(|d| format!(" on {d}"))
                            .unwrap_or_default();
                        ui.label(
                            RichText::new(format!("last payment: ${c:.2}/share{when}"))
                                .color(FAINT)
                                .small(),
                        );
                    }
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(
                            "Not paid out here — it compounds into the token's price instead \
                             (the shares multiplier), which is why the price drifts above the \
                             real stock's over time.",
                        )
                        .color(FAINT)
                        .small(),
                    );
                }
                _ => {
                    ui.label(
                        RichText::new("No current dividend on file for this one.")
                            .color(FAINT)
                            .small(),
                    );
                }
            }
            if let Some(g) = multiplier_growth_1y {
                ui.add_space(6.0);
                ui.label(
                    RichText::new(format!(
                        "Shares multiplier grew {:.2}% over the last year — the real, \
                         on-chain-recorded trail of dividends compounding in.",
                        g * 100.0
                    ))
                    .color(if *g >= 0.0 { UP } else { RED })
                    .small(),
                );
            }
        }
        Outcome::TokenLaunch { token_symbol, token_name, paired_symbol, curve_address, tx_hash } => {
            ui.horizontal(|ui| {
                ui.label(RichText::new(token_symbol).color(FG).font(FontId::monospace(16.0)).strong());
                ui.label(RichText::new("Launched · testnet").color(ORANGE).small());
            });
            ui.label(RichText::new(token_name).color(DIM).small());
            ui.add_space(6.0);
            ui.label(RichText::new(format!("paired with {paired_symbol}")).color(FAINT).small());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("curve: {curve_address}"))
                        .color(ACCENT)
                        .font(FontId::monospace(11.0)),
                );
                ui.hyperlink_to(
                    RichText::new("view on explorer ↗").color(ACCENT).small(),
                    testnet_explorer_address_url(curve_address),
                );
            });
            ui.hyperlink_to(
                RichText::new(format!("tx {tx_hash} ↗")).color(FAINT).font(FontId::monospace(11.0)),
                testnet_explorer_tx_url(tx_hash),
            );
        }
        Outcome::CurveBuy { curve_address, paired_symbol, stock_in, min_tokens_out, tx_hash } => {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Buy on curve").color(FG).strong());
                ui.label(RichText::new("testnet").color(ORANGE).small());
            });
            ui.add_space(4.0);
            ui.label(
                RichText::new(format!("spent {stock_in} {paired_symbol}, min {min_tokens_out} tokens out"))
                    .color(DIM)
                    .small(),
            );
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("curve: {curve_address}")).color(FAINT).font(FontId::monospace(11.0)));
                ui.hyperlink_to(
                    RichText::new("view ↗").color(ACCENT).small(),
                    testnet_explorer_address_url(curve_address),
                );
            });
            ui.hyperlink_to(
                RichText::new(format!("tx {tx_hash} ↗")).color(FAINT).font(FontId::monospace(11.0)),
                testnet_explorer_tx_url(tx_hash),
            );
        }
        Outcome::CurveStatus {
            curve_address,
            paired_symbol,
            raised,
            graduation_threshold,
            graduated,
            pool_address,
            price_per_token,
            progress_pct,
        } => {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Curve status").color(FG).strong());
                ui.label(
                    RichText::new(if *graduated { "graduated" } else { "still curving" })
                        .color(if *graduated { UP } else { ORANGE })
                        .small(),
                );
            });
            ui.add_space(4.0);
            if let Some(price) = price_per_token {
                ui.label(
                    RichText::new(format!("{} {paired_symbol} per token", fmt_price(*price)))
                        .color(ACCENT)
                        .font(FontId::monospace(16.0)),
                );
            }
            ui.label(
                RichText::new(format!("{raised} of {graduation_threshold} {paired_symbol} raised"))
                    .color(DIM)
                    .small(),
            );
            if let Some(pct) = progress_pct {
                let bar_color = if *pct >= 100.0 { UP } else { ORANGE };
                ui.add(
                    egui::ProgressBar::new((*pct / 100.0).clamp(0.0, 1.0) as f32)
                        .text(format!("{pct:.1}%"))
                        .fill(bar_color),
                );
            }
            if let Some(pool) = pool_address {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("pool: {pool}")).color(ACCENT).font(FontId::monospace(11.0)));
                    ui.hyperlink_to(
                        RichText::new("view ↗").color(ACCENT).small(),
                        testnet_explorer_address_url(pool),
                    );
                });
            }
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("curve: {curve_address}")).color(FAINT).font(FontId::monospace(11.0)));
                ui.hyperlink_to(
                    RichText::new("view ↗").color(ACCENT).small(),
                    testnet_explorer_address_url(curve_address),
                );
            });
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
