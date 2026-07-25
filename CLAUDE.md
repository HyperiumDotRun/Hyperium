<!-- hyperium:pointer:start -->
> This project is open in **Hyperium** (dev cockpit). Hosting and conventions (wiki launched by Hyperium, `hyperium-notes/` notes, local-only): see `HYPERIUM.md` at the root.
<!-- hyperium:pointer:end -->



## Ondo Finance (reference — tokenized equities)

Full docs read at docs.ondo.finance (96 pages). Relevant to the `src/sushi` tokenized-equities
work: Ondo issues on-chain total-return trackers for US stocks/ETFs ("Ondo Stocks", tokens
suffixed `on`, e.g. `TSLAon`, `AAPLon`) on Ethereum, BNB Chain and Solana. Dividends
auto-reinvest via a rising "shares multiplier," so token price drifts above the raw stock price
over time — these are not 1:1 share wrappers.

**API ("GM" API):**
- Base: `https://api.gm.ondo.finance`, auth via `x-api-key` header (request via
  onboarding@ondo.finance, keys after KYC/onboarding at app.ondo.finance).
- Key endpoints (all `/v1/...`): `assets/all/metadata`, `assets/all/addresses` /
  `assets/{symbol}/addresses` (always fetch live, don't hardcode token addresses),
  `assets/{symbol}/prices/latest` (display only, not for trading/oracle use),
  `assets/{symbol}/market`, `assets/{symbol}/prices/ohlc?interval=&range=`,
  `assets/{symbol}/dividends`, `limits/session`, `limits/trading`, `status/market`,
  `status/assets`, `tickers`.
- Trading flow: `POST /v1/attestations/soft` (non-binding quote) → `POST /v1/attestations`
  (binding, EIP-712 signed, returns `attestationId` + signature) → call
  `IGMTokenManager.mintWithAttestation()` / `redeemWithAttestation()` on-chain with
  USDon/USDC(Eth)/USDT(BNB).
- Streaming: gRPC at `grpc.gm.ondo.finance:443`
  (`StreamPriceUpdates`, `StreamOHLC`, `StreamSoftQuoteDepth`).
- Symbol format: min 3 chars, must end in lowercase `on`. Valid chain IDs: `ethereum-1`,
  `bsc-56`, `solana-900`.
- No mint/burn fees (Ondo earns the quote/execution spread). Min trade $1. Instant, atomic
  mint/redeem to USDon; redeem to USDC depends on swapper liquidity.

**Trading hours (ET):** premarket 4:01–9:29am, regular 9:31am–3:59pm, postmarket 4:01–7:59pm,
overnight 8:05pm–3:55am, full week Sun 8:05pm → Fri 7:59pm. Off-hours (weekends/holidays)
trading exists for a fixed list of ~21 large-cap assets (AAPL, TSLA, NVDA, MSFT, GOOGL, META,
SPY, QQQ, etc.) with wider spreads and dynamic limits. Live status: `status.ondo.finance`.

**Eligibility gate to know about:** non-US persons only (US, Canada, and several other
countries hard-banned); KYC currently institutional-only, retail is waitlisted. Not directly
usable by anonymous end users — relevant if the agent ever needs to explain why a swap/mint
isn't available to a given user.

Also exists: **USDY** (yield-bearing tokenized Treasury note, general access) and **OUSG**
(tokenized Treasury fund, accredited/qualified-investor only) — separate products, not
equities, lower priority for this project.

### What's actually implemented (Phase A — read-only)

Full plan: `C:\Users\tomch\.claude\plans\cuddly-jumping-bachman.md`.

- `src/sushi/ondo.rs` — read-only REST client (`GET /v1/assets/{symbol}/market`), ticker
  normalization (`TSLA` → `TSLAon`). JSON field names are **best-effort**, not yet validated
  against a real response (no Ondo API key held yet) — see the doc comment on `ondo::market`.
- `Intent::StockLookup` (`intent.rs`), `Outcome::Stock` (`mod.rs`) — same pattern as the
  existing Robinhood Chain intents/outcomes.
- New tool `lookup_stock`, exposed to the chat model in `agent_tools()` — system prompt
  explicitly tells the model this is a different chain/product from Robinhood Chain, read-only,
  no swap card, no trading.
- Settings panel: "Ondo Stocks API key" field, stored via `secret::save_secret` at
  `config_dir/ondo.key` (same encrypted-on-Windows pattern as `sushi.key`/`anthropic.key`).
- `cargo check` + `cargo test` pass (40/40, incl. 2 new tests in `ondo.rs`). **Not yet
  live-tested**: no Ondo API key exists to actually call the endpoint with.

### What's NOT done yet (Phase B — real trading)

Blocked on an external step, not code: get an Ondo API key + wallet allowlisting via
institutional onboarding (`onboarding@ondo.finance`). Once that exists: add Ethereum RPC to
`chain_rpc::rpc_url()`, generalize `wallet_balance_text`'s hardcoded `chain_id = 4663`, add
attestation calls (`POST /v1/attestations/soft` → `POST /v1/attestations`) and
`IGMTokenManager.mintWithAttestation`/`redeemWithAttestation` calldata encoding, reusing the
existing `local_send_transaction` signing pipeline as-is. Full detail in the plan file above.
