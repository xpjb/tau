# Tau 2 integrated agent

Integrated release branch: `tau2`. The completed SQLite backend originated at
`cddb8b7` on `tau2/integrated-agent`; this release uses that single implementation.

## Ownership

`taud` owns acceptance, the durable queue, model HTTP streams, tools, history,
compaction and cancellation. No Node/Pi worker, pipes, worker capability HTTP
endpoint, RPC correlation, second transcript sequence, or process recovery.
Titles are native and run after prompt acknowledgement. The default uses the first
nonempty prompt line without a provider call. Optional `daemon.generateTitles`
uses a bounded, no-tools native request with `titlePrompt` and optional `titleModel`
(unset uses the chat model), falls back on failure,
and cannot overwrite a later manual rename. There is no external title helper.

The conversation tools run with the daemon's OS permissions, just as Pi did.
This is not a sandbox. Keep the daemon behind authenticated access.

## Shared native protocol

Protocol **13** keeps flat transcript, paging, session, upload and attachment
shapes. The transfer crate and iroh routes are unchanged.

- `prompt` success means its queue item, receipt and session metadata have committed to SQLite. It does not
  wait for a provider or title generation. `uncertain` is always false. A lost network
  response is still possible: reopen with request IDs, or resend the **same ID
  and original text**. Duplicate acceptance never executes a second turn.
- Pending requests, revisions and deleted-request receipts survive restart.
  Recovered pending work, including an accepted user turn that had already left
  the queue but never finished, is paused; Resume is explicit, not an automatic rerun
  of tools with potentially unknown effects.
- Built-in commands reserve their request ID before executing. A retry of a
  completed command returns its saved result. After an interrupted command, the
  same ID returns an explicit error rather than rerunning a potentially billed
  compaction or an operation with unknown effects. Settings remain a separate
  JSON document, not part of a database transaction.
- Only `turn` is advertised as a queue boundary. Pause and prefix requests check
  the run ID; edits/deletes check revisions. No reasoning-checkpoint emulation.
- The daemon does not emit `starting` or interactive Pi extension dialogs.
  Extension request/response types and title-only settings aliases are removed.
  Native flag notifications use `notice`, not a fake extension dialog.
- `get_settings {id}` returns `settings {requestId, settings}`, then the usual success response.
- `set_settings {id, revision, settings}` replaces the complete document with a
  compare-and-swap revision check. It returns the new document. Reload on conflict;
  do not silently overwrite another client's changes.
- Queue controls and abort commit their own durable receipts. Duplicate receipt
  lookup precedes stale generation checks; replaying an old abort cannot cancel a
  newer run. Aborts cancel promptly but acknowledge success only after persistence.

The Rust settings UI has Daemon, Agent, Prompts, Providers and Model metadata
sections. It edits one complete CAS document, preserves edits on conflict, and has
staged per-field reset. Prompt selection is exactly model override → saved default.
`agent.systemPrompt` is text; `agent.modelSystemPrompts` maps exact provider/model IDs
to text. A missing key inherits and an empty string overrides. There is no project,
provider or built-in prompt fallback. `loadAgentsFiles` controls separate AGENTS.md
context. Advanced maps use JSON editors. Secrets are never sent to the frontend.
Chat context actions expose native model/thinking/compact/priority commands and keep
their target chat ID. Shared settings/DTOs live in `tau-protocol`; filesystem loading,
validation and prompt composition remain daemon-only through `SettingsExt`.

The real frontend integration fixture uses the native daemon, local provider, tools,
SQLite, uploads and restart—not Pi or a hand-written substitute WebSocket server.

### Backend follow-ups from frontend notes

Reviewed `frontend/MERGE.md`, `frontend/QA.md` and the timing/idle flags. New native
assistant reasoning, text and individual function-call sections now capture their
first daemon stream-observation time in the persisted content block. Projection
uses that time for the existing `timestampMs` field, unchanged by later deltas,
completion or reopening. This is **not** a provider-reported generation start.
Old Pi entries still fall back to their entry/message timestamp; historical
section times cannot be reconstructed. The current agent aggregates one reasoning
section and one answer section per model response, plus individual tool calls.

The old paused-queue idle retention problem does not require a polling/retry layer:
native queues are already durable. An idle runtime now evicts even with paused or
pending work; reopening restores that work paused and still deduplicates accepted
requests. This is only in-memory agent lifetime. It is unrelated to provider cache
expiration or the frontend's explicitly estimated cache ring; no worker-TTL wire
field is added.

## Files and migration

- `TAU_SETTINGS_PATH`: `/var/lib/tau/settings.json` by default. JSON, schema 2,
  camelCase, `daemon`, `agent`, `providers`, `models`, and a monotonic revision.
  Writes use a private temporary file, fsync, atomic rename, directory fsync.
- `auth.json` beside settings: private provider credentials, separate from menu
  settings. Supports Pi's Codex OAuth and API-key record shapes. Codex tokens are
  refreshed in-process, serialized across all chats. With the daemon stopped,
  `TAU_SETTINGS_PATH=/path/settings.json taud --login-codex` signs in without Pi
  using the Codex device-code flow. API-key environment variable
  names are configured per provider. For side-by-side beta operation,
  `TAU_CODEX_AUTH_SOURCE` optionally reads a primary auth file without copying,
  refreshing or writing it. Beta's own Codex record takes precedence. An expired or
  rejected shared token reports that its primary owner must refresh, or that beta
  needs its own login; it never races the primary refresh credential.
- `TAU_DATABASE_PATH`: `/var/lib/tau/tau.sqlite3` by default. SQLite schema 1,
  WAL, `synchronous=FULL`, foreign keys enabled. The database is private (0600).
  It replaces both native `state.json` and per-chat JSONL files. Old
  `TAU_STATE_PATH` / `TAU_SESSION_DIR` variables now fail with an actionable error.
- `flags.jsonl` and `client-crashes.jsonl` remain operational logs, not conversation
  stores. Flags and full oversized shell output (`tool-output/`) live beside the
  database. Attachments/uploads remain separate files.

A new Tau 2 installation simply creates its SQLite database. **There is no migration
for the unreleased native JSONL format and no dual-write compatibility store.**

For existing **Tau 1/Pi** data only, there is an explicit offline import:

1. Stop the old daemon/workers and back up metadata, histories, settings, credentials
   and attachments. Do not deploy this branch with a protocol-10 client.
2. Remove `TAU_STATE_PATH` and `TAU_SESSION_DIR` from the environment. Set
   `TAU_DATABASE_PATH` to a new database and `TAU_SETTINGS_PATH` to a new settings
   file; retain the other normal daemon environment values, including `TAU_TOKEN`.
3. Optionally set `TAU_IMPORT_PI_DIR=/root/.pi/agent`, then run
   `taud --import-state /var/lib/tau/state.json`. This exits after import; it does
   not start the server. The destination must have no sessions. Session metadata,
   selected active history branches and their display projections import in one
   transaction. Original files, including malformed/torn lines, are never modified.
   Malformed legacy lines are skipped; abandoned Pi branches remain only in the
   untouched originals. Pending Pi worker commands are not imported.
4. Remove the one-time import environment variable before normal startup.
   `scripts/install-daemon.sh` installs only the separate beta unit/data/ports;
   it cannot silently replace the stable Tau 1 executable or service.

Only when settings do not yet exist, `TAU_IMPORT_PI_DIR` imports the optional model
metadata, selected model, thinking defaults, steering, compaction/retry, shell
settings, global SYSTEM.md / APPEND_SYSTEM.md text,
Codex priority/native compaction settings and credentials (without overwriting
an existing auth file). `--import-state` also supplies the old title prompt on
first settings creation. Never run old and copied refresh credentials concurrently.

File edits are read at daemon startup; API edits are live. Runtime limits and
prompt configuration apply at the next model turn; model/thinking defaults apply
to new chats. `/model` and `/thinking` change a stopped chat explicitly.

## Provider and tool scope

Codex Responses SSE and OpenAI-compatible Chat Completions (including OpenRouter).
Supports streamed text/reasoning/tools, encrypted Codex replay, bounded responses,
HTTP retry before output, OAuth renewal, native Codex checkpoints, and text
summaries for other providers. Non-Codex chats can use the native `generate_image`
bridge when an actual Codex model is configured, without changing their active or
default model. No third-party JavaScript extension loader, TUI,
provider WebSocket transport, Anthropic adapter, npm packages, or Pi skill/prompt
catalog discovery. Unported terminal commands are rejected, not acknowledged as
if they ran. Current Tau media and flag tools are native; flags write directly to
StateStore. Project AGENTS.md files are loaded root-to-working-directory.

### Media

`send_file` is the only local delivery tool, matching master's `9a38a50` behavior.
It resolves paths relative to the working directory (including the existing `@`
path prefix), accepts any accessible regular file up to 50,000,000 bytes, and
copies files outside the outbox into a private per-send directory, preserving the
basename. Already-staged files are reused. PNG/JPEG/WebP are identified by bytes,
not extension; up to 10,000,000 bytes they appear inline, otherwise as files.
Captions are trimmed and limited to 1,024 characters. The download endpoint still
independently confines access to the outbox; the ability to stage a file is not
an unauthenticated arbitrary-path download API.

Codex chat requests offer native `image_generation` with `gpt-image-2` and PNG
output, as in glmbot, using the existing Codex OAuth account. No second agent,
image subprocess or separate image API credential. Search/summary/compaction
requests and Chat Completions do not advertise this tool. All returned images
must validate before any are staged: completed status, unique IDs, valid base64,
PNG signature, at most four files and 10,000,000 decoded bytes **in total**.
Partial images are never delivered. Started response streams are never retried.

Generated output uses the same private staging and authenticated attachment
transport as `send_file`; an image-only answer is a successful answer. The wire
remains an ordinary assistant image event with an attachment and stable entry ID.
The database stores file references, not base64 bytes or non-replayable generation
IDs. On the next turn (also after restart/fork), files are loaded as labeled
reference-image inputs, because `store:false` cannot replay those IDs. Missing
originals become explicit unavailable-reference notices, not automatic paid
regeneration. Compaction includes references in the compacted context; it is not
an archival image index. Staged files remain separate from chat history and may
outlive a chat; automatic outbox garbage collection is not implemented.

For the cross-provider `generate_image` tool, prompts are bounded, optional local
reference inputs are limited to four PNG/JPEG/WebP files of at most 10 MB each,
and the forced Codex image-only turn must produce exactly one complete image with
no function calls. It shares native validation, staging and reference replay; a
started response is not automatically retried. Image-only requests also disable
pre-header HTTP retries because billing may already have happened; delivery failure
explicitly asks for user approval before another generation. This replaces master's TypeScript
image-generation path, including its later hardening, rather than retaining it.

### SQLite session storage

One database owns sessions, immutable history entries, saved display events,
current queue items and idempotency receipts. Provider-specific replay data stays
JSON, not a model-specific SQL schema. The event table is a bounded/public display
projection, committed with its source entry; it never includes private provider
payloads or image bytes. There is no second writable history store.

Acceptance inserts the receipt and queue item and retains the chat in one
transaction. Consuming a queue prefix and recording its user messages is another
single transaction. Deleted-request receipts remain available for reconnect
reconciliation. Model/history metadata changes commit with their entries. Changes
are published only after commit. A per-session database revision rejects stale
writers; use one daemon per database, not multiple competing agents.

Snapshots load the latest bounded event page; older pages use the
`(session_id, position)` index. The live runtime keeps only that tail plus current
streaming events, not all saved history or receipts. Provider turns read only the
latest applicable checkpoint and retained history suffix (the full conversation
until its first compaction). Encrypted checkpoints remain account/model/endpoint
scoped. Compaction never deletes saved history.

Fork/clone copies the selected prefix with SQL inside one transaction, including
already-consumed user receipts but **not** queued work or unrelated receipts.
There is no copy-on-write DAG or new branch-coordination layer. Deletion cascades
entries, events, queue and receipts; children retain their copied history and
lose only the parent link. External tool effects and attachment files are not
transactional with SQLite: interrupted tools still get explicit unknown-effects
recovery, never automatic re-execution.

`taud --export-session ID PATH` writes a private, consistent `tau-history` version-1
JSON snapshot containing metadata/entries and private replay fields, but no pending
work, receipts, auth or duplicated generated-image bytes. It is a portable history
export, not a complete operational backup.

Back up with SQLite's backup API / `.backup`, or stop the daemon before copying
data. Do not copy only a running database's main file and omit its WAL. Back up
settings, auth and attachment/upload files separately as well.

Tests use isolated state directories, local scripted HTTP providers, real
WebSockets and real filesystem/shell tools. They do not use production credentials
or send paid model requests. Live-provider acceptance remains a release check.

## Validation on this branch

- Managed `cargo nextest run --locked --workspace`: **49/49 passed** across native
  daemon, shared protocol, frontend, Markdown and transfer binaries. Provider fixtures split SSE into
  small byte fragments; the framer also checks every possible byte split.
- Real WebSocket coverage includes injected transaction failures before acceptance,
  duplicate IDs, queue edits/deletes/pause/prefix/cancel/resume and stale revisions,
  settings conflicts, native titles, reconnect, fork/clone, paused-work idle eviction
  and restart recovery. A gated stream checks distinct section timestamps and their
  persistence through compaction/reopening.
- SQLite coverage includes rollback after receipt insertion / queue removal / fork
  creation, indexed cold-history paging with bounded runtime memory, lineage and
  cascading deletion, integrity/foreign-key checks, and read-only Tau 1 import.
- A subprocess test kills the actual daemon with **SIGKILL**, leaving an uncheckpointed
  WAL. Restart preserves accepted/deleted IDs and edited pending work, performs no
  automatic model calls, and resumes the unfinished turn only on request. A second
  kill during compaction verifies that its reserved command ID cannot execute twice.
- Real tools cover exact writes/edits, overlapping-match rejection, shell output,
  process-group cancellation, bounded image input, automatic staging, inline/file classification, outbox confinement and flags.
- Codex fixtures cover encrypted reasoning replay, native compaction and retained
  history, plus generated image download/restart/fork/reference replay and failure/size limits,
  without exposing image bytes or encrypted/private provider payloads in client events.
- Native cross-provider image delivery/export preserves the current/default model;
  shared-auth tests read rotations without refreshing or rewriting the primary.
- Workspace all-target check and shell syntax/diff checks; storage branch also ran
  `cargo clippy -p taud --all-targets -- -D warnings` before integration.

Provider fixtures do not call paid Codex/OpenRouter completions. Beta device login
is an explicit deployment step, not a mocked test. Shared-protocol integration and
native settings/menu UI are implemented; the real GPU/settings path has been
exercised locally. Physical Windows/Android acceptance remains device QA. An account/endpoint mismatch
on an encrypted checkpoint fails explicitly; use the original account or fork
before that checkpoint. There is no silent lossy fallback.

## Provider context limits (unreleased protocol-15 branch)

The first beta imported optional model metadata from Pi, but that is not an
independent source for a percentage. Tau 2 now makes a bounded, read-only GET
against its configured inference provider, with the same authentication and
Codex originator. Codex `/models` supplies `context_window` (falling back to
`max_context_window`); OpenRouter-compatible `/models` supplies `context_length`.
Only an exact model ID from a nonempty catalog with valid limits is accepted.
The validated minimal catalog is persisted in private `model-catalog.json` next to
daemon settings. On startup its credential identity and endpoint are checked
before use; a missing/invalid file triggers a provider GET and alerts clients on
failure. Connection settings has an explicit **Refresh models** control for
fetching newly released models without expiring otherwise valid saved metadata.
A failed refresh leaves the last good file intact. No configured/Pi metadata
fallback is used for the meter or automatic compaction; an exact ID missing from
the provider catalog remains unknown. The same saved capacity gates compaction. The public OpenAI `/v1/models` listing is not used as a source of
context windows. No Pi worker, Pi settings mirror, or guessed ID aliases are used.

## Unshipped settings update

See `tau2-backlog/README.md` for the 0.7.1/protocol-13 handoff. The live daemon remains
0.7.0. Model metadata is optional, not an allowlist; absent capacity remains unknown
and disables threshold-based auto-compaction. Provider errors are bounded and
credential-redacted. Default prompt text from older settings stays intact. The
loader does not rewrite the existing file merely by opening it; save uses the
normal revision check. Nonempty removed project overrides fail explicitly rather
than silently losing text. This does not import conversations or add a migration
service. Astra's explicit empty override must be set during the coordinated update.
