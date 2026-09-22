# Tau architecture map

One-sentence responsibility per module. Read this before opening files.

## Daemon (`daemon/`, Rust, single binary `taud`)

- `main.rs` / `lib.rs` — entrypoint and composition root.
- `config.rs` — deployment paths, listener addresses and bearer-token validation.
- `settings.rs` — unified, revision-checked daemon/agent settings and one-time Pi import.
- `server.rs` — authenticated HTTP/WebSocket routing, uploads, attachment grants,
  crash ingest and heartbeat deadlines. No model or session logic.
- `manager.rs` — native session lifecycle, durable prompt acceptance, queue controls,
  history feeds, fork/clone and deletion.
- `agent/mod.rs` — cancellable model/tool loop, transcript publication, compaction
  and asynchronous title generation.
- `agent/journal.rs` — durable JSONL history/queue writes, legacy active-branch import
  and provider context reconstruction. Unknown tool effects are never auto-replayed.
- `agent/provider/` — bounded SSE framing, Codex Responses and Chat Completions,
  request construction and safe HTTP retries.
- `agent/auth.rs` — private provider credentials, serialized Codex OAuth refresh,
  and daemon-side device login.
- `agent/tools.rs` — native filesystem, shell, media and incidental-flag tools.
- `attachments.rs` — authenticated attachment resolution, upload storage and MIME checks.
- `commands.rs` — native command catalog and model/thinking/compact/name/fast settings.
- `transcript.rs` — flat event projection, stable live-to-saved IDs and paging.
  There is only one generation/sequence: the client-facing transcript.
- `protocol.rs` — client↔daemon wire data (to move into the frontend branch's shared crate).
- `state.rs` — session metadata and durable flags; no active agent settings.

Dependencies point one way: `server → manager → {agent, transcript, state, settings}`.
There is no Pi subprocess or internal RPC transport. See [Tau 2 integration and
migration](docs/tau2-agent.md) before merging the Rust frontend.

## Legacy client (`app/composeApp/`, Kotlin, Android + desktop)

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
  Settings edits live in the daemon section of `settings.json`.

## Rules the codebase keeps

- Acknowledge user actions in local UI state immediately, before network work.
  Remote confirmation determines completion or failure, not whether a click
  received visible feedback.
- No single-caller functions unless they are UI composables, axum route
  handlers, or platform-interface implementations.
- Wire changes are versioned. The Rust frontend merge must carry protocol 11
  in its shared crate; this backend cannot be deployed with protocol-10 clients.
- The daemon owns one reusable starter in session metadata. New Chat loads the native session
  before replying and shows its real model. Sent messages, explicit renames or
  client keep hints retain work. Idle sleep evicts the runtime, not the chat.
  Only the blank starter is labeled New chat. Untitled chats with data or kept
  outside that slot are Unnamed chats, with no count limit.
- Daemon JSONL owns saved history and acknowledged pending work. Remote transcript events stay in client
  memory only; SQLite preserves local work, files and preferences.
- The daemon owns branch selection, event order, stream lifecycle and activity
  bumps. Recovery assigns order from the source, not discovery time.
  Transport pages do not define presentation groups.
