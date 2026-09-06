# Tau

Tau is a private, Tailnet-native client for independent Pi coding-agent sessions. It runs beside the existing Telegram gateway without sharing processes, sessions, or state.

## Components

- `daemon/`: `taud`, the Linux service that owns Tau's Pi RPC processes and session files.
- `app/`: one Compose Multiplatform client for Android and desktop JVM targets.
- `windows/`: the portable launcher and version-aware self-extracting Windows setup.

Tau starts a Pi RPC process when needed and stops it after one idle hour, while preserving held queue work. Pi's JSONL remains the transcript source of truth. The daemon projects identified entries; clients keep a SQLite disk cache and a shared in-memory view for display. Drafts, pending sends and interrupted content survive client restarts. Cold chats can be read without starting Pi. Reconnect currently synchronizes with a full snapshot while keeping cached content visible. New chats use `/root` as their working directory.

Tau 0.5.0 uses protocol 3 and requires matching daemon and client versions. Existing 0.4.8 clients must be upgraded together with the daemon. The daemon also requires the identified-transcript Pi fork; this release uses commit `29b43c7` from `xpjb/pi`. Existing JSONL files are preserved without migration.

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
- Detect failed connections and reload the selected chat and live Pi state automatically.
- Fork from any visible user message.
- Attach local files for Pi to inspect, view images from Pi inline, and download files produced through Pi's `send_image` and `send_file` tools.
- On Windows, drop files onto the chat, paste clipboard images as attachments, use Enter to send, Shift+Enter for a newline, and Escape to interrupt Pi.
- Use the same chats from Android and Windows.

## Daemon installation

Build and install the independent systemd service:

```bash
cargo build --release --package taud
sudo ./scripts/install-daemon.sh
```

The installer generates `/etc/tau.env` once with a random bearer token, binds `taud` to `127.0.0.1:8787`, and publishes that loopback listener through Tailscale Serve on Tailnet port 8787. It prints the URL and token required by the clients. Tailnet traffic is already encrypted; no public listener is created. This works with both kernel and userspace Tailscale networking.

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
