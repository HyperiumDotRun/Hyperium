# Hyperium

A terminal on steroids for building in Web3 — one Rust binary, one window,
everything a dev cockpit needs to actually ship: a real terminal, encrypted
notes, an AI copilot, and a Web3 agent that can quote and route swaps without
ever holding your keys.

Built in Rust on [egui](https://github.com/emilk/egui) /
[eframe](https://github.com/emilk/egui). Windows for now.

## Terminal, on steroids

Not a shell-out wrapper — a real PTY-backed terminal (`alacritty_terminal` +
`portable-pty`), sitting inside a project-first cockpit:

- **Per-project sessions** — `claude`, `ssh`, `ftp`, RDP, or a local server,
  each one a launcher entry (`launcher.rs`), not a manually retyped command.
- **Single-instance, file-triggered** — drop a `.hyp` / `.stg` launcher file
  anywhere and double-clicking it reopens (or focuses) the right project in
  the *same* running instance, over a local socket (`stg.rs`,
  `127.0.0.1:52473`). No duplicate windows, no hunting for the right tab.
- **System tray-resident** — lives in the tray (`tray.rs`), stays out of the
  way until you need it.
- **Environment doctor** — probes what's actually installed on the machine
  (`doctor.rs`: toolchains, engines, versions) so the cockpit knows what it
  can launch before you find out the hard way.

## Web3-native, and built to stay that way

Hyperium ships a Sushi-powered trading agent for Robinhood Chain — you ask in
plain English ("what's trending", "swap 100 USDC for ETH"), and it answers
and quotes in real time. But the point isn't the swap, it's the shape of how
it's built:

- **The model never touches money.** Natural language is parsed into a
  strict, structured intent (`sushi/intent.rs`) — a symbol and an amount,
  nothing else. Every address lookup, every bit of arithmetic, every API call
  happens afterwards, in plain Rust, against a curated registry. The LLM
  proposes; it can't execute.
- **Hyperium never holds a private key.** Signing goes through
  [Frame](https://frame.sh) (`sushi/wallet.rs`) over its local JSON-RPC
  endpoint — every write is a *request to sign*, built and confirmed inside
  Frame's own window. A bug in Hyperium can misquote a price; it cannot move
  funds on its own.

That LLM-proposes / Rust-and-wallet-disposes boundary isn't specific to
swaps — it's the pattern the rest of the Web3 surface gets built on. The
Rust core is the part that stays solid while more protocols, chains, and
agent capabilities get added on top of it.

## AI-assisted, not AI-dependent

- **Notes copilot** — optional Claude (Anthropic) integration for the notes
  workspace; only note text is ever sent, nothing else (`llm.rs`).
- **Asset generation** — `kie.ai`-backed image/video generation, dropped
  straight into a project's assets (`genai.rs`).
- **Voice input** — local mic capture via `cpal` (`voice.rs`) for
  hands-free note-taking.
- **Habit coach** — a lightweight local habit/streak tracker (`coach.rs`).

## Notes that sync, without trusting the server

- Client-side encrypted (XChaCha20-Poly1305, Argon2id passphrase KDF,
  `vault.rs`) before anything leaves the machine.
- Synced over FTP/FTPS (`ftp.rs`, `sync.rs`) to whatever server you point it
  at — that server only ever sees ciphertext.

## Self-updating, and signed

Releases are Authenticode-signed, and the updater (`update.rs`,
`authenticode.rs`) verifies the signature before ever swapping the running
binary.

## Building

```
cargo build --release
```

Windows-only for now — secret storage (DPAPI), the tray icon, and process
job objects all go through the Win32 API directly via the `windows` crate.
An Inno Setup script for a per-user installer is at `release/hyperium.iss`.

## Secrets

Hyperium never ships or hardcodes an API key, password, or credential. Every
secret (Anthropic key, `kie.ai` key, FTP/FTPS login, sync passphrase) is
entered at runtime through the Settings UI and stored locally, encrypted at
rest with Windows DPAPI (`src/secret.rs`).

## Status

Actively developed, currently at `v0.62.1`. Expect rough edges.
