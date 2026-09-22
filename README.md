# Tau

This branch implements the **Tau 2 Rust backend**: the coding agent runs inside
`taud`, without Pi or Node. The Rust frontend is developed separately on
`tau2/rust-frontend`; the Kotlin client below remains legacy release documentation.
Do not deploy this daemon with the old protocol-10 clients.

See [the backend contract, settings, migration and scope](docs/tau2-agent.md).
The native backend uses protocol 11, durable prompt acknowledgement, one transcript
sequence, native tools, Codex/OpenRouter streaming and unified `settings.json`.
Old Pi JSONL histories are retained and readable. Transfers are unchanged here.

## Components

- `daemon/`: the Rust daemon and in-process agent.
- `app/`: legacy Compose client; replaced on the frontend branch.
- `transfer/`: shared Rust transfer engine.
- `windows/`: existing Windows launcher/installer.

## Legacy 0.5.14 client operations

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

## Selection and crash diagnostics

Protocol 10 requires matched client and daemon updates.

A build-time patch fixes the top/bottom edge mismatch in current stable Compose
1.12.0. Long horizontal selection drags retain scrolling and copying. The same
checked patch reaches desktop and Android; see `app/patches/README.md`.

Crash reporting keeps the latest full local trace, including messages, causes
and suppressed exceptions, in `client-crash.log`. It is capped at 64K characters
(under 256 KiB plus a truncation marker). On Windows it is in
`%LOCALAPPDATA%\Tau\data`; on Linux, `$XDG_DATA_HOME/Tau` or
`~/.local/share/Tau`; on Android, in the app's private files directory. This file
can contain private text or tokens: review it before sharing. Uploading a report
does not delete it. A later crash replaces this local trace.

The first pending remote report remains in `client-crash.pending.json` until its
successful upload. A reply for an older report cannot clear a newer one.
Report schema 2 adds up to three cause stacks and numeric
`selectionRange` details (`start`, `end`, `textLength`) for the known text-range
error. Arbitrary messages and the full local trace are never uploaded. Reports
stay under 24 KiB; the daemon also accepts old schema 1 pending reports. Failed
report writes are printed to stderr instead of silently discarded. The patch
prevents this selection defect; it adds no blanket UI catch-and-continue policy.

## Incidental flags

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

The native tool writes directly to the current chat's StateStore. No worker
capability, loopback flag endpoint or TypeScript extension is needed. Notifications
reach connected clients; this adds no offline push service or issue tracker.

Later, ask an agent to read the log and investigate a flag by ID.

## Daemon installation

After the matched Rust frontend is merged and the migration is accepted, build and install the service:

```bash
cargo build --release --package taud
sudo ./scripts/install-daemon.sh
```

The installer generates `/etc/tau.env` once with a random bearer token, binds `taud` to `127.0.0.1:8787`, and publishes that loopback listener through Tailscale Serve on Tailnet port 8787. It prints the URL and token required by the clients. Tailnet traffic is already encrypted; no public listener is created. This works with both kernel and userspace Tailscale networking.

The title helper needs both `scripts/title_gen.py` and its adjacent `scripts/title_prompt.txt`.

Tau state is stored under `/var/lib/tau`. Client uploads are isolated by chat under `/root/.local/share/tau/uploads` and deleted with the chat. The agent stages outgoing files under `/root/.local/share/tau/outbox`; `taud` independently canonicalizes and validates every requested file before streaming it through an authenticated endpoint. Client crash reports are bounded, omit chat content and exception messages, and are appended to `/var/lib/tau/client-crashes.jsonl`. Each accepted report also appears in `journalctl -u tau.service`.

## Native file transfers

New clients request a file grant through the existing bearer-authenticated HTTP
endpoint, then download verified blocks over QUIC/UDP using the shared Rust
`tau-transfer` engine. There is no automatic TCP fallback. Partial data stays
beside the account/chat/entry-scoped private cache across retries and restarts;
complete files are verified, synced and atomically published. Existing saved
files and exports remain usable.

`TAU_TRANSFER_BIND` defaults to `127.0.0.1:8788` (UDP). The installed userspace
Tailscale stack forwards Tailnet UDP to this loopback port. On a kernel-mode
Tailscale server, set this value to the server's Tailnet IPv4 address and port.
Allow that UDP port in Tailnet policy. Tailscale Serve remains the HTTP/chat
proxy and needs no UDP configuration. Iroh discovery and relays are disabled.
The setup response supplies the UDP port and pinned QUIC identity; clients use
the hostname from their existing Tau connection settings.

A grant covers one open, validated file and one client QUIC identity for up to
one hour. The daemon retains at most 128 recent grants and evicts the oldest
when full. It retains file handles and small BLAKE3 outboards, not duplicate
file bodies. A source modified after granting fails integrity checks.

The shared library ships for Windows x64 and all four existing Android ABIs.
Kotlin uses generated UniFFI bindings for setup, progress and cancellation; file
blocks stay in Rust. The Linux build supports local client acceptance. Build
through `scripts/build-transfers.sh` and the mandatory shared Cargo wrapper.

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
