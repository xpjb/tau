# Tau 2 — native Rust beta

Rust client, daemon, coding agent, shared protocol, Markdown and verified file transfer.
No Pi worker, Kotlin/Compose client, Java desktop runtime, UniFFI bridge or Python title
helper. Android retains a small Java bridge for Android OS APIs.

**0.7.4 beta · protocol 18 (native block sync).**
Matched beta daemon, Windows x64 installer and Android ARM64 APK are required.
See `INTEGRATION.md` for the actual deployment/delivery status.
This is a separate installation, not a stable-Tau cutover.
The maintained frontend branch is `tau2-rust-frontend`; the integrated release
branch is `tau2`. Frontend work lands on the frontend branch before integration.

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

Settings → **Daemon settings** exposes daemon, agent, prompt, provider and optional
model-metadata configuration. Edits use revision compare-and-swap; conflicts
retain your edits instead of overwriting another client's changes. The prompt chain has exactly two levels: **model override → default system prompt**.
Both are ordinary text, including empty text. A missing model override inherits;
there is no provider/built-in fallback. A chat’s captured topic prompt is appended
after this resolved prompt and working-directory instructions, not used as a fallback.
Prompt fields use the shared editor;
advanced provider/metadata structures use JSON editors; credentials are never
part of the wire settings document.

New chats appear immediately as **Creating chat…**, including offline. Their
creation intent and any draft, attachment or send are committed to local SQLite
before network effects; queued sends wait locally for the chat's durable server ID.
Existing chats also accept offline sends into a visible **Waiting for connection**
state. Sends made during quick model selection wait locally for its confirmation;
a failed or uncertain model change retains the authored send for explicit recovery
rather than using the wrong model.
The daemon records client-named creation receipts transactionally, so a lost create
ack can be retried without creating a duplicate. If it reuses an existing starter,
local drafts, files and queued sends move to that chat without losing the old
session's work. A send is acknowledged after its server-side queue/receipt commit,
not after a model response. A socket lost after a sent prompt leaves the receipt
**unconfirmed** until history verifies it; potentially billed prompts are not
blindly retransmitted. New sends accepted into a paused queue clear a stale error
indicator while still showing that work must be resumed. This behavior ships in
beta 0.7.3.

New chats use the last explicitly chosen model. Quick-select favorites do not change
that default. IDs are sent exactly; optional metadata is not an allowlist. Unknown
context capacity comes from the **selected provider's own catalog** when available:
Codex's authenticated `/models` response (`context_window`, falling back to
`max_context_window`) with the same account and originator as inference, or
OpenRouter's `/models` `context_length`. Exact model IDs only; no Pi model file,
name matching, or guessed capacity. Tau saves validated limits in its own private
`model-catalog.json` beside daemon settings, bound to the provider endpoint and
credential identity. It queries the provider if that file is absent or unusable;
failed discovery alerts connected clients. **Connection settings → Refresh models**
forces a new GET for the chosen provider and replaces the file only on success.
The refreshed provider IDs also appear as optional model suggestions. There is no
automatic expiration or unverified fallback. On September 24, 2026 a read-only
Codex catalog request with Tau's originator and Codex catalog client version
0.156.1 reported 272,000 for GPT-6 Sol, Luna and Astra. Refresh when a provider
changes its models or limits.

The tooltip shows last provider-reported turn tokens even when capacity is unknown.
The ring percentage and threshold-based auto-compaction need a known saved limit.
Sleeping chats retain the last saved token count, marked as last known. The
protocol-15 beta has matched clients and daemon. The title model is separately
configurable; unset uses the chat model. Cache rings are estimates from existing
reply timestamps, not native runtime idle timeouts. Chat context menus target the clicked chat and expose model,
thinking, compaction, priority service, rename, clone, release and delete actions.

## Topics

Small, horizontally scrolling topic tabs sit above the chat list. General is
permanent and holds existing chats after the SQLite migration. The trailing **+**
creates a topic; right-click or touch-and-hold a tab to rename it, edit its prompt,
or delete it. Deletion first confirms the topic, then offers **Move chats to
General**, **Delete topic and its chats**, or **Cancel**. General’s prompt can be
edited, but General cannot be renamed or deleted.

Chat context menus have a scrollable **Move to topic** submenu. New chats use the
selected topic; cloning/forking preserves membership and captured instructions.
Topic prompts are captured when a chat is created. Editing a topic affects only
future chats. Moving a chat replaces its captured topic portion of the system
prompt with the destination’s current prompt (including empty). A running provider
turn/tool continuation finishes with its existing instructions; the next turn uses
the replacement. No history rewrite, appended context messages, or apply-latest
control. Moves may invalidate the provider’s prefix cache.

Tabs accept vertical mouse-wheel input, horizontal trackpad scrolling, and touch or
mouse dragging. The selected tab is bold and underlined; unread dots aggregate the
chat list’s unread state. On desktop, a newly finished unread chat requests window
attention while Tau is unfocused. Focusing Tau clears the window alert; the unread
dot stays until its chat is viewed. Switching topics resumes that topic’s
last-open chat (or its newest available chat); only a chat actually shown is
marked read. Empty
topics show the chat list without inventing a selection. Topics and membership
are daemon-owned, durable, and shared across devices; per-topic last selections,
drafts, and read markers remain account-scoped client state. Existing `projectId`
wire fields and SQLite names remain unchanged so beta history stays compatible.
Concurrent topic edits
use revision checks and preserve the losing editor’s text. Both clients and daemon
must use protocol 18 in beta 0.7.4. Stable Tau is unchanged.

The native rewrite is merged into the beta release line. See [native block sync](docs/tau2-block-sync.md) for the protocol, bounds, recovery and restore contract.

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
- HTTP/WebSocket `127.0.0.1:8791`, native UDP port `8792` in both IP families;
  userspace Tailscale forwards to loopback; kernel-mode installs bind Tailnet addresses only;
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

Current work and acceptance: [Tau 2 backlog / settings handoff](tau2-backlog/README.md).
