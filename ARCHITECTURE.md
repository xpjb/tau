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
- `protocol.rs` — re-export of `tau-protocol`, plus legacy Pi-only response/context adapters.
- `state.rs` — Tau-owned session metadata and the serialized, durable flag log.
- `pi-extension/send-media.ts` — Tau agent tools for sending media and flagging
  incidental findings; flag writes use a capability scoped to the current worker.

Dependencies point one way: `server → manager → {pi, transcript, state}`,
with `transcript` owning Pi formats and `state.rs` owning Tau's own
`state.json` and `flags.jsonl`. Events flow back up via broadcast channels, never calls.

## Tau 2 client (`frontend/`, Rust, Android + desktop)

- `controller.rs` — local-first actions, daemon messages and receipt reconciliation.
- `feed.rs` — transactional remote transcript windows; generation/sequence/order validation.
- `store.rs` — private local-only SQLite state and copied files; no remote transcript persistence.
- `transport.rs` — epoch-scoped authenticated sockets, reconnect/heartbeat, HTTP and native Iroh.
- `app.rs` — responsive screens, transcript grouping, anchors and UI actions.
- `editor.rs` / `render.rs` — input state, Sanscale text/Markdown and GPU image presentation.
- `desktop.rs` / `android.rs` — platform input/lifecycle/services; a small Android Java OS bridge.
- `protocol/` — **single Rust wire contract**, consumed by both frontend and daemon.
- `markdown/` — extracted incremental parser and window-independent Sanscale view.

The daemon's `protocol.rs` now re-exports `tau-protocol`; Pi-only conversion traits
stay in the daemon. Native-client production code does not depend on taud or Pi.
See `frontend/MERGE.md` for the transitional daemon adapters and independent
backend merge seam, and `frontend/README.md` for remaining preview release gates.

## Reference client (`app/composeApp/`, Kotlin, Android + desktop)

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
- `CrashLog.kt` (`jvmMain`) — the platform seam's shared JVM crash writer:
  bounded full local traces, safe pending reports and durable file replacement.

## Title helper (`scripts/`)

- `title_gen.py` — one-shot local title inference from daemon JSON input.
- `title_prompt.txt` — the default full title prompt, shared with daemon state.
  Settings edits are stored in the existing daemon `state.json`.

## Rules the codebase keeps

- Acknowledge user actions in local UI state immediately, before network work.
  Remote confirmation determines completion or failure, not whether a click
  received visible feedback.
- No single-caller functions unless they are UI composables, axum route
  handlers, or platform-interface implementations.
- Wire changes are versioned: `PROTOCOL_VERSION` in `tau-protocol` is shared by
  both Rust endpoints. The preserved Kotlin reference remains at protocol 10;
  a production cutover must explicitly retire or update it.
- The daemon owns one reusable starter in session metadata. New Chat starts Pi
  before replying and shows its real model. Sent messages, explicit renames or
  client keep hints retain work. Idle sleep stops the worker, not the chat.
  Only the blank starter is labeled New chat. Untitled chats with data or kept
  outside that slot are Unnamed chats, with no count limit.
- Pi JSONL owns saved history. Remote transcript events stay in client
  memory only; SQLite preserves local work, files and preferences.
- The daemon owns branch selection, event order, stream lifecycle and activity
  bumps. Recovery assigns order from the source, not discovery time.
  Transport pages do not define presentation groups.
