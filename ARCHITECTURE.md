# Tau architecture map

One-sentence responsibility per module. Read this before opening files.

## Daemon (`daemon/`, Rust, single binary `taud`)

- `main.rs` / `lib.rs` — entrypoint and composition root.
- `config.rs` — environment configuration, validated once at startup.
- `server.rs` — HTTP/WebSocket transport: auth, request routing, upload
  and attachment serving, crash ingest, client ping deadlines. No Pi or
  session logic.
- `manager.rs` — session lifecycle: per-chat runtime, Pi process
  supervision, transcript projection, idle sleep, fork/clone, deletion.
- `attachments.rs` — attachment media: resolution, upload storage, MIME
  sniffing.
- `commands.rs` — slash commands: the builtin interpreter and the Pi
  command catalog.
- `pi.rs` — one Pi subprocess and its RPC pipe: request correlation,
  event broadcast, shutdown.
- `transcript.rs` — Pi data formats and the transcript state machine:
  entry parsing, positional updates, paging, queue state, model backfill
  walks.
- `protocol.rs` — the client↔daemon wire contract. Pure data.

Dependencies point one way: `server → manager → {pi, transcript, state}`,
with `transcript` owning Pi formats and `state.rs` owning only Tau's own
`state.json`. Events flow back up via broadcast channels, never calls.

## Client (`app/composeApp/`, Kotlin, Android + desktop)

- `TauApp.kt` — all Compose UI. State is owned by `TauController`.
- `TauController.kt` — connection, session orchestration, receive loop.
- `AttachmentDownloads.kt` — attachment transfer lifecycle (extensions on
  `TauController`).
- `TauClient.kt` — WebSocket/HTTP transport to the daemon.
- `TranscriptStore.kt` — SQLite persistence and projection of transcripts.
- `RetainedTranscript.kt` — in-memory transcript model shared by store
  and UI, plus chat-key and position types.
- `Protocol.kt` — mirrors `daemon/src/protocol.rs` wire types.
- `TranscriptText.kt` — text utilities: markdown rendering, suggestion
  scoring, byte formatting.
- `Platform.kt` + actuals — the only OS seam (per-platform services and
  image decoding).

## Rules the codebase keeps

- No single-caller functions unless they are UI composables, axum route
  handlers, or platform-interface implementations.
- Wire changes are versioned: `PROTOCOL_VERSION` in `protocol.rs` and
  `Protocol.kt` move together with matched client releases.
- Live transcript entries persist to the client store on first appearance
  and at most once per flush window; finalization always persists.
