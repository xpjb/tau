# Tau

Tau is a private, Tailnet-native client for independent Pi coding-agent sessions. It runs beside the existing Telegram gateway without sharing processes or chat history. Both use Pi's global settings. A model chosen in Tau becomes Pi's default for new chats; existing chats keep their saved model. Tau does not override Pi with a separate model default.

## Components

- `daemon/`: `taud`, the Linux service that owns Tau's Pi RPC processes and session files.
- `app/`: one Compose Multiplatform client for Android and desktop JVM targets.
- `windows/`: the portable launcher and version-aware self-extracting Windows setup.

Tau starts a Pi RPC process when needed and stops it after one idle hour, while preserving held queue work. Pi's JSONL remains the transcript source of truth. The daemon projects ordered content events; clients keep remote history in memory only. SQLite preserves drafts, pending sends/controls, attached files, preferences and session metadata across client restarts. Cold chats can be read without starting Pi. Loaded history and chat feeds stay in memory across selection changes. Running, unread and recent chats warm in the background; older events also load on demand. Reconnect checks the retained event windows. Thinking and tools remain collapsible across fetch boundaries. New chats use `/root` as their working directory.

Tau 0.5.12 clients and daemon use protocol 7 and must be updated together. This release includes full title-prompt editing, copying message text without Details, running/unread priority, stopped-model failure status, full-screen image zoom and quiet background reconnects. It also retains the scrolling and remembered-download fixes. Android versionCode is 33. `TauClientVersion` in `Platform.kt` supplies the version for Settings, crash reports and both client installers. Client schema 4 preserves local work while remote history remains memory-only. The existing identified-transcript Pi fork and JSONL files stay unchanged.

## Current client operations

- List, create, rename, and permanently delete chats.
- Stream assistant text and tool activity.
- Render completed assistant and system messages as selectable Markdown with clickable links and width-wrapped tables.
- Discover Pi extension, prompt-template, and skill commands when `/` is entered, with command and supported built-in argument completion.
- Run extension dialogs inside Tau and expose extension notices, status text, widgets, and composer updates.
- Send prompts, steer an active run, and abort.
- Delete queued messages or run the inclusive prefix through a selected message at a safe boundary; later messages stay held until another prefix or explicit Resume.
- Show a circular context-usage estimate beside the composer, with token counts on Windows hover or Android tap.
- Show sent prompts immediately, keep unconfirmed sends visible across reconnects, and never automatically resend them.
- Keep loaded history across chat switches and warm running, unread and recent chats without starting Pi.
- Bump chats on assistant replies and stops; preserve unread markers across app restarts.
- Detect failed connections and restore retained chat feeds without replaying sends or controls.
- Fork from any visible user message.
- Attach local files for Pi to inspect, view images from Pi inline, and download files produced through Pi's `send_image` and `send_file` tools.
- Save viewed images privately for offline inline/full-screen viewing. Export a saved original without downloading it again.
- Zoom and pan full-screen images with pinch or mouse wheel/drag, plus minus/plus/Fit controls.
- Edit the entire shared title prompt in Settings, with exact whitespace and empty overrides.
- Use the connection indicator for routine reconnect failures; keep actionable errors visible.
- On Windows, drop files onto the chat, paste clipboard images as attachments, use Enter to send, Shift+Enter for a newline, and Escape to interrupt Pi.
- Use the same chats from Android and Windows.

## Incidental flags (unreleased)

Tau agents can call `flag_it(str)` to record a new finding outside the current
work: technical debt, environment problems, or wasted resources. Include what
was observed, where, and why it matters. Omit secrets and continue the current
task; flagging does not authorize extra work or start another agent.

The daemon appends one JSON record to `flags.jsonl` beside its configured
`state.json` (normally `/var/lib/tau/flags.jsonl`). Each record contains `id`,
`timestampMs`, `sessionId`, `sessionTitle`, and the full `text`, limited to 4096
characters. The daemon serializes and syncs the write before confirming it and
broadcasting a **Flagged** notice through the existing client banner. The file
is created with owner-only permissions. Accepted writes and their notifications
finish even if the calling connection closes. If the tool reports an unconfirmed
save, inspect the log before retrying; it never automatically retries.

The tool is registered only in Tau workers. The daemon supplies a worker-scoped,
flag-only capability; it does not expose the full client bearer token. A worker
cannot flag another chat, and its capability expires when it stops. The new
`POST /v1/sessions/{session_id}/flags` endpoint accepts only that capability.
Temporary fork workers do not receive it. Existing client notification messages
and protocol 7 stay unchanged. Notifications reach connected clients; this adds
no offline push service, issue tracker, or automatic investigation.

Later, ask an agent to read the log and investigate a flag by ID. Deployment must
include both the updated daemon and Tau extension; no installer has been issued
for this feature yet.

## Daemon installation

Build and install the independent systemd service:

```bash
cargo build --release --package taud
sudo ./scripts/install-daemon.sh
```

The installer generates `/etc/tau.env` once with a random bearer token, binds `taud` to `127.0.0.1:8787`, and publishes that loopback listener through Tailscale Serve on Tailnet port 8787. It prints the URL and token required by the clients. Tailnet traffic is already encrypted; no public listener is created. This works with both kernel and userspace Tailscale networking.

The title helper needs both `scripts/title_gen.py` and its adjacent `scripts/title_prompt.txt`.

Tau state is stored under `/var/lib/tau`. Client uploads are isolated by chat under `/root/.local/share/tau/uploads` and deleted with the chat. Pi stages outgoing files under `/root/.local/share/tau/outbox`; `taud` independently canonicalizes and validates every requested file before streaming it through an authenticated endpoint. Client crash reports are bounded, omit chat content and exception messages, and are appended to `/var/lib/tau/client-crashes.jsonl`. Each accepted report also appears in `journalctl -u tau.service`.

## Android

```bash
cd app
./gradlew :androidApp:assembleDebug
```

The APK is written below `app/androidApp/build/outputs/apk/debug`. Release builds use a private signing key configured through ignored `app/local.properties`.

## Windows self-extractor

```bash
./scripts/build-windows-sfx.sh
```

The build runs on Linux and produces `dist/Tau-<version>-windows-x64.exe`. The EXE contains the app and its Windows Compose native runtime.

On first launch, Tau downloads a checksum-pinned private Temurin Java 21 runtime into `%LOCALAPPDATA%\Tau\runtimes` and reuses it across later client updates. The launcher verifies the exact archive size and SHA-256 before atomically installing it. Tau requires no system JVM, but its first launch requires internet access.

On Windows it installs without UAC under `%LOCALAPPDATA%\Tau\versions`, writes a stable `%LOCALAPPDATA%\Tau\Tau.exe` launcher, and adds Tau directly to the user's Start Menu. Running the same setup again performs no extraction and reports that the version is already installed. A newer setup installs beside the old version and switches `current.txt` only after extraction completes.

## Verification

```bash
cargo nextest run --workspace
cargo build --workspace --release
cd app
./gradlew :composeApp:desktopTest :androidApp:assembleDebug
```
