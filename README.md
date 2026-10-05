# Tau 2 — native Rust beta

Rust client, daemon, coding agent, shared protocol, Markdown and verified file transfer.
No Pi worker, Kotlin/Compose client, Java desktop runtime, UniFFI bridge or Python title
helper. Android retains a small Java bridge for Android OS APIs.

**0.7.8 beta · protocol 20 (composer thinking level + checkpoint-yielding sync).**
Windows x64 and Android ARM64 packages were verified and attached through Tau.
The matching daemon is built but **not deployed**: wait for the user's explicit
confirmation that both packages have been downloaded before restarting beta.
The running daemon remains **0.7.7 / protocol 19**; the new clients need the pending
protocol-20 daemon update. See `HANDOFF.md` and `INTEGRATION.md` for the release
record. This is a separate installation, not a stable-Tau cutover.
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

New chats appear immediately, including offline. Drafts, attachments and exact
model choices are saved locally before network effects. Both new-chat model entry
points update immediately; there is no `/model` selection request or confirmation
state. The first prompt carries its chosen model, and text sends can carry the
pending named chat creation without waiting for a create response. File uploads
still require a confirmed owning chat before publication. The daemon commits the
model with the prompt's queue entry and receipt, and deduplicates creation by the
client's original UUID. Existing chats also accept offline sends into a visible
**Waiting for connection** state. A send is acknowledged after its server-side queue/receipt commit,
not after a model response. A socket lost after an ordinary message starts a
receipt check. If the daemon has
not accepted it, Tau retries the saved message with the **same request ID**, so a
late acknowledgement cannot create a duplicate. Temporary transport failures back
off and retry; saved pending messages also recover after restart while another
chat is selected. Explicit server rejections, invalid attachments, uncertain
controls and source-lineage changes remain available for deliberate recovery.
**Retry saved message** reuses the original ID for an older “Not sent” item.
New sends accepted into a paused queue clear a stale error indicator while still
showing that work must be resumed. Offline draft support shipped in beta 0.7.3;
the automatic saved-message recovery changes shipped in beta 0.7.6.

The connection hover distinguishes an existing WebSocket from acquiring one,
using real attempt ages and deadlines. Acquisition has one five-second timeout
and a one-second minimum between attempt starts, not a growing post-failure wait.
Sent text stays visible through acknowledgement and replication; authenticated
body hashes let the client reuse its own content rather than download it again.
See [connection and local-content design](docs/connection-and-local-content.md)
for the behavior, compatibility and validation.

Above the text box, the model slug is followed by **Thinking: LEVEL**. This is
that chat's saved selection (including `off`), not the current default for new
chats. Long model slugs shorten before the thinking label on narrow screens.
The value follows confirmed model/thinking changes and remains available from
the local account cache offline; unknown thinking metadata is omitted, and a
new local model choice never borrows the previous model's thinking or usage. The chat thinking editor is prefilled with the same selection.

New chats use the client's last explicitly chosen model (saved per account), or
the cached daemon fallback when there is no local preference. Editing quick-select
favorites does not choose a model. IDs are sent exactly; metadata is not an allowlist.
The shared account model catalogue is cached on the client, available without a
selected chat, and revalidated asynchronously on connection and picker use. Late
metadata cannot overwrite a local choice. Last-known hints survive a cold daemon
or a failed refresh; background metadata errors do not interrupt selection.

Context capacity comes only from the **selected provider's own catalogue**:
Codex's authenticated `/models` response (`context_window`, falling back to
`max_context_window`) or OpenRouter's `/models` `context_length`, matched by exact
ID. No Pi model file, name matching or guessed capacity is used. Tau saves validated
limits privately in `model-catalog.json`, bound to endpoint and credential identity.
The daemon revalidates hour-old caches and missing exact IDs with bounded request
coalescing/cooldown; valid old limits survive transient errors. **Connection settings
→ Refresh models** forces revalidation and replaces the saved file only on success.
See [client-first models and explorer validation](docs/client-first-models.md).

The context tooltip shows last provider-reported turn tokens even when capacity is unknown.
For Codex chats, its hover/pinned card also shows read-only account quota, separate
from the context gauge. Other providers show an unavailable state. The ring
percentage and threshold-based auto-compaction need a known saved limit.
Sleeping chats retain the last saved token count, marked as last known. The
new beta packages require matching protocol-20 clients and daemon. The title model
is separately configurable; unset uses the chat model. Cache rings are estimates from existing
reply timestamps, not native runtime idle timeouts. Chat context menus target the clicked chat and expose model,
thinking, compaction, priority service, rename, clone, release and delete actions.

## Attachments

The folded-paper button beside Play/Stop opens the current chat's sent files,
newest first. Desktop uses a second sidebar on the right; mobile and narrow
windows use a separate screen. Image previews and download/save controls work
as they do in the conversation, and older files load as you scroll.
See [frontend QA](frontend/QA.md) for source validation and release status.

## Mobile input, text and connection health

Mobile editing uses an opaque full-screen native editor. **Done** keeps the draft;
Send remains in the chat. Keyboard and system-bar insets keep the editor visible,
and queued keyboard edits apply before Send, navigation or suspension. Notice taps
are consumed before controls underneath can act. Disclosure arrows are drawn shapes.
Android's font matcher supplies the installed face, collection index and variation
axes, including bold weight; application packages do not bundle fonts.

The shared desktop/mobile connection dot uses the last ten heartbeat attempts.
Stable latency up to 800 ms is green. Missed probes, a range over 400 ms, or native
packet loss observed in the last 20 seconds make it yellow. Latency over 1 second
is orange; over 3 seconds, or a disconnected socket, is red. A probe still waiting
within the normal range is not counted as a loss.

See [mobile QA acceptance](docs/mobile-qa.md) for the source changes, validation and
remaining device checks. These changes shipped in beta 0.7.6. Physical Android
keyboard, font and touch acceptance remains open. The Android package was built
but not sent.

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
mouse dragging. General stays first; other topics follow the latest activity of
any contained chat (including local drafts and new chats). Ties and empty topics
keep catalogue order. The selected tab is bold and underlined; unread dots
aggregate the chat list’s unread state. On desktop, a newly finished unread chat
requests window attention while Tau is unfocused. Focusing Tau clears the window
alert; the unread dot stays until its chat is viewed. Desktop topic switches resume that topic’s
last-open chat (or its newest available chat). Mobile topic switches stay on the
chat list until a chat is tapped. Only a chat actually shown is marked read. Empty
topics show the chat list without inventing a selection. Topics and membership
are daemon-owned, durable, and shared across devices; per-topic last selections,
drafts, and read markers remain account-scoped client state. Existing `projectId`
wire fields and SQLite names remain unchanged so beta history stays compatible.
Concurrent topic edits
use revision checks and preserve the losing editor’s text. Both clients and daemon
must use protocol 19 in beta 0.7.7. Stable Tau is unchanged.

The native rewrite is merged into the beta release line. See [native block sync](docs/tau2-block-sync.md) for the protocol, bounds, recovery and restore contract.

## Build and validate

For a beta rollout, use **[`scripts/release-beta.sh`](docs/beta-release.md)** rather
than manually repeating builds/checks/deployment. It has resumable stage receipts,
sequential low-resource builds, package verification and an ordered delivery
manifest. No tests or service changes run implicitly:

```sh
scripts/release-beta.sh --plan --version 0.7.8 --push --deploy
scripts/release-beta.sh --version 0.7.8 --push --deploy
```

Choose the next unused version. Add `--merge feat/name` when needed; `--check` and
`--test` are explicit, cached opt-ins. The commands below remain for development,
not an instruction to repeat a whole suite during every deployment.

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

Codex refreshes automatically. For side-by-side use, `TAU_CODEX_AUTH_SOURCE`
selects the primary credential file. Beta uses the same `proper-lockfile`
directory lease as stable Pi, rereads after acquiring it, and saves a rotated
token back to that file. It does not make a second credential copy.

If renewal genuinely fails, **Settings → Sign in to Codex** opens the browser
sign-in flow with a code and automatic approval detection. A chat authentication
failure opens this prompt directly; no terminal command is needed. Completing
this optional sign-in creates an independent beta login for the chosen Codex
account, taking precedence over the shared file without replacing stable's login.
Authentication never automatically repeats model or tool work; **Resume chat**
is explicit. See [Codex recovery](docs/codex-signin.md).
Static OpenRouter API keys remain private in beta's own `auth.json`.

Never copy only a running SQLite main file and omit its WAL: use SQLite's backup
API / `.backup`, or stop beta before copying. Back up settings, auth and attachments
separately. `taud --export-session ID PATH` writes a private portable history JSON;
it is not a backup of pending jobs or receipts.

This daemon is not a sandbox. Keep it behind authenticated Tailnet access.

[Architecture](ARCHITECTURE.md) · [Transcript contract](TRANSCRIPT.md) ·
[Frontend QA](frontend/QA.md) · [Integration/release log](INTEGRATION.md)

Current work and acceptance: [Tau 2 backlog / settings handoff](tau2-backlog/README.md).
