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
  branch projection into flat events, ordered updates, recovery, paging,
  queue state and model backfill walks.
- `protocol.rs` — the client↔daemon wire contract. Pure data.

Dependencies point one way: `server → manager → {pi, transcript, state}`,
with `transcript` owning Pi formats and `state.rs` owning only Tau's own
`state.json`. Events flow back up via broadcast channels, never calls.

## Client (`app/composeApp/`, Kotlin, Android + desktop)

- `TauApp.kt` — app screens and transcript UI. Long-lived state is owned by `TauController`.
- `LocalImage.kt` — bounded image loading, fitted previews and full-screen zoom/pan controls; zoom state stays in the open viewer.
- `TauController.kt` — connection, per-chat feeds, bounded history warming,
  unread state and receive loop.
- `AttachmentDownloads.kt` — attachment transfer lifecycle (extensions on
  `TauController`).
- `TauClient.kt` — WebSocket/HTTP transport to the daemon.
- `LocalStore.kt` — durable local work and session metadata in SQLite;
  snapshot, page and update application to memory-only transcripts.
- `RetainedTranscript.kt` — flat event types, stable rows and their ordered
  in-memory collection, plus pending-work and chat-position types.
- `TranscriptPresentation.kt` — collapsible groups across loaded events,
  tool-call/result matching, presentation-key reuse and model-failure status.
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
- Pi JSONL owns saved history. Remote transcript events stay in client
  memory only; SQLite preserves local work, files and preferences.
- The daemon owns branch selection, event order, stream lifecycle and activity
  bumps. Recovery assigns order from the source, not discovery time.
  Transport pages do not define presentation groups.
