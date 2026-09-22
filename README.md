# Tau 2 — native Rust beta

Rust client, daemon, coding agent, shared protocol, Markdown and verified file transfer.
No Pi worker, Kotlin/Compose client, Java desktop runtime, UniFFI bridge or Python title
helper. Android retains a small Java bridge for Android OS APIs.

**0.7.0 beta · protocol 12.** This is a separate installation, not a stable-Tau cutover.
The older frontend snapshot is preserved at `tau2-rust-frontend`; the integrated
release branch is `tau2`.

## Components

- `frontend/`: Chad window/surface lifecycle, Sanscale text, native Windows/Android UI.
- `daemon/`: authenticated HTTP/WebSocket service and native Codex/OpenRouter agents.
- `protocol/`: one owned, bidirectional serde contract, including revisioned settings.
- `markdown/`: incremental Markdown layout; `transfer/`: authenticated, verified QUIC.
- `windows/`: native per-user beta installer and launcher.

The daemon uses the completed storage agent's SQLite implementation: WAL + FULL
synchronous commits, transactional prompt/queue/receipt acceptance, indexed display
history, bounded hot transcripts, checkpoint/suffix replay, and transactional forks.
There is no writable JSONL mirror. Interrupted work is paused; uncertain tool effects
are never automatically replayed. See [agent/storage details](docs/tau2-agent.md).

Settings → **Daemon / agent settings** exposes daemon, agent, system, project,
provider and model configuration. Edits use revision compare-and-swap; conflicts
retain your edits instead of overwriting another client's changes. System prompt
`null` means built-in/inherited; a custom empty string is deliberately empty.
Provider/model/project structures use explicit JSON editors; credentials are never
part of the wire settings document.

New chats use the last explicitly chosen model. Quick-select favorites do not change
that default. Cache rings are estimates from existing reply timestamps, not native
runtime idle timeouts. Chat context menus target the clicked chat and expose model,
thinking, compaction, priority service, rename, clone, release and delete actions.

## Build and validate

Use the host's managed Cargo wrapper. Keep `CARGO_BUILD_JOBS=1 RAYON_NUM_THREADS=1`,
build targets sequentially, and defer on shared-lock exit 75. Use nextest, not
`cargo test`; the wrapper supplies its test-concurrency limit.

```sh
cargo check --locked --workspace --all-targets
cargo nextest run --locked --workspace
cargo build --release --locked -p taud
scripts/build-windows-sfx.sh             # Tau Beta Windows x64 installer
ANDROID_ABI=arm64-v8a frontend/android/build.sh
```

Android: SDK/build tools 35, NDK 27.2.12479018, API 29+, Vulkan 1.1, ARM64 APK with
16 KiB-aligned native segments. The existing beta signing key and `app.tau.rust`
identity permit updates without replacing stable. Windows installs only beneath
`%LOCALAPPDATA%\Tau Beta`, with taskbar ID `app.tau.beta`; existing beta local work
remains in `%LOCALAPPDATA%\Tau2`. No JVM or extra runtime installer is required.

## Side-by-side beta deployment

`scripts/install-daemon.sh` is **beta-only**. It installs:

- `tau2-beta.service`, executable `/usr/local/lib/tau2-beta/taud`;
- HTTP/WebSocket `127.0.0.1:8791`, transfer UDP `127.0.0.1:8792`;
- Tailnet URL **http://vibe:8789**;
- `/var/lib/tau2-beta/{tau.sqlite3,settings.json,auth.json}`;
- `/root/.local/share/tau2-beta/{outbox,uploads}`;
- private connection environment `/etc/tau2-beta.env`.

It does not replace `/usr/local/bin/taud`, `tau.service`, stable data, or existing
Tailnet routes. The connection token is reused from stable when first installing.
Beta starts with its own conversation database; stable history is not silently
migrated. Use the explicit offline importer when a real cutover is authorized.

Codex can be signed in independently:

```sh
TAU_SETTINGS_PATH=/var/lib/tau2-beta/settings.json \
  /usr/local/lib/tau2-beta/taud --login-codex
```

For side-by-side use, `TAU_CODEX_AUTH_SOURCE` optionally reads a primary credential
file **without copying, refreshing or modifying it**. An independently signed-in
beta record takes precedence. If the primary token expires without a primary
refresh, beta reports that explicitly rather than racing its refresh token.
Static OpenRouter API keys can be stored privately in beta's own `auth.json`.

Never copy only a running SQLite main file and omit its WAL: use SQLite's backup
API / `.backup`, or stop beta before copying. Back up settings, auth and attachments
separately. `taud --export-session ID PATH` writes a private portable history JSON;
it is not a backup of pending jobs or receipts.

This daemon is not a sandbox. Keep it behind authenticated Tailnet access.

[Architecture](ARCHITECTURE.md) · [Transcript contract](TRANSCRIPT.md) ·
[Frontend QA](frontend/QA.md) · [Integration/release log](INTEGRATION.md)
