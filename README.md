# Tau

Tau is a private, Tailnet-native client for independent Pi coding-agent sessions. It runs beside the existing Telegram gateway without sharing processes, sessions, or state.

## Components

- `daemon/`: `taud`, the Linux service that owns Tau's Pi RPC processes and session files.
- `app/`: one Compose Multiplatform client for Android and desktop JVM targets.
- `windows/`: the portable launcher and version-aware self-extracting Windows setup.

Tau starts a Pi RPC process when a chat receives a prompt and stops it after one idle hour. Tau reads Pi's JSONL session files directly when browsing or opening chats, without starting Pi. Chats resume from those files after client, process, or daemon restarts. The active chat branch is rebuilt from Pi entry IDs rather than from a second transcript database. New chats currently use `/root` as their working directory.

## Current client operations

- List, create, rename, and permanently delete chats.
- Stream assistant text and tool activity.
- Render completed assistant and system messages as selectable Markdown with clickable links.
- Send prompts, steer an active run, and abort.
- Show accepted prompts immediately while they wait in Pi's steering queue.
- Fork from any visible user message.
- Attach local files for Pi to inspect and download files produced through Pi's `send_image` and `send_file` tools.
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

The build runs on Linux and produces `dist/Tau-<version>-windows-x64.exe`. The EXE contains the app, the Windows Compose native runtime, and a small Windows runtime image built from a checksum-pinned JDK.

On Windows it installs without UAC under `%LOCALAPPDATA%\Tau\versions`, writes a stable `%LOCALAPPDATA%\Tau\Tau.exe` launcher, and adds Tau directly to the user's Start Menu. Running the same setup again performs no extraction and reports that the version is already installed. A newer setup installs beside the old version and switches `current.txt` only after extraction completes.

## Verification

```bash
cargo nextest run --workspace
cargo build --workspace --release
cd app
./gradlew :composeApp:desktopTest :androidApp:assembleDebug
```
