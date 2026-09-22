# Tau 2 integrated agent

Branch: `tau2/integrated-agent`. This is a backend branch, not a deployment.

## Ownership

`taud` owns acceptance, the durable queue, model HTTP streams, tools, history,
compaction and cancellation. No Node/Pi worker, pipes, worker capability HTTP
endpoint, RPC correlation, second transcript sequence, or process recovery.
The optional existing title helper remains separate from the conversation agent;
it runs **after** prompt acknowledgement.

The conversation tools run with the daemon's OS permissions, just as Pi did.
This is not a sandbox. Keep the daemon behind authenticated access.

## Frontend merge contract

Protocol **11** keeps flat transcript, paging, session, upload and attachment
shapes. The transfer crate and iroh routes are unchanged.

- `prompt` success means its queue item, receipt and session metadata have committed to SQLite. It does not
  wait for a provider or title helper. `uncertain` is always false. A lost network
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
  `starting` and `extension_error` are removed; old dialog responses receive an explicit error.
- `get_settings {id}` returns `settings {requestId, settings,
  defaultSystemPrompt}`, then the usual success response.
- `set_settings {id, revision, settings}` replaces the complete document with a
  compare-and-swap revision check. It returns the new document. Reload on conflict;
  do not silently overwrite another client's changes.
- `get_title_prompt`/`set_title_prompt` remain aliases into the same daemon section.
  The new frontend should use the unified settings API.

Frontend settings menu: a **Daemon** section for title prompt and idle timeout;
an **Agent** section for model, thinking, queue mode, priority service, retry,
compaction, shell and output limits; a multiline **System prompt** editor with
an explicit “use default” (`null`) action and a separate append editor; and
project-path overrides with the same two editors. Empty strings are intentional,
not reset-to-default. Expose model/thinking/compact/fast as convenient chat menu
commands as well. Provider URLs and model catalog belong in an advanced section.
Settings contain no keys or tokens.

The frontend branch extracts `tau-protocol`. At merge, move Settings and its
nested wire structs into that crate along with the new protocol variants.
Keep filesystem loading, validation and prompt composition in the daemon (free
functions or an extension trait after moving the structs). Keep
its transfer changes. Keep this branch's native manager and removal of Pi source
positions. Do not reintroduce a second event sequence to resolve the conflict.
The native fixture uses `Config.database_path`, not `state_path` / `session_dir`;
the frontend's old Pi-based fixture must still be replaced at merge.

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

- `TAU_SETTINGS_PATH`: `/var/lib/tau/settings.json` by default. JSON, schema 1,
  camelCase, `daemon`, `agent`, `providers`, `models`, and a monotonic revision.
  Writes use a private temporary file, fsync, atomic rename, directory fsync.
- `auth.json` beside settings: private provider credentials, separate from menu
  settings. Supports Pi's Codex OAuth and API-key record shapes. Codex tokens are
  refreshed in-process, serialized across all chats. With the daemon stopped,
  `TAU_SETTINGS_PATH=/path/settings.json taud --login-codex` signs in without Pi
  using the Codex device-code flow. API-key environment variable
  names are configured per provider.
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
4. Remove the one-time import environment variable before normal startup. The
   installer requires `TAU2_INSTALL_CONFIRMED=yes` when old state is present, so it
   cannot silently replace a working Tau 1 installation.

Only when settings do not yet exist, `TAU_IMPORT_PI_DIR` imports the provider/model
catalog, selected model, thinking defaults, steering, compaction/retry, shell
settings, global SYSTEM.md / APPEND_SYSTEM.md, working-directory .pi overrides,
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
summaries for other providers. No third-party JavaScript extension loader, TUI,
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

Back up with SQLite's backup API / `.backup`, or stop the daemon before copying
data. Do not copy only a running database's main file and omit its WAL. Back up
settings, auth and attachment/upload files separately as well.

Tests use isolated state directories, local scripted HTTP providers, real
WebSockets and real filesystem/shell tools. They do not use production credentials
or send paid model requests. Live-provider acceptance remains a release check.

## Validation on this branch

- `cargo nextest run --workspace`: 16 tests, including eight native end-to-end
  scenarios and the existing transfer tests. Provider fixtures split SSE into
  small byte fragments; the framer also checks every possible byte split.
- Real WebSocket coverage includes injected transaction failures before acceptance,
  duplicate IDs, queue edits/deletes/pause/prefix/cancel/resume and stale revisions,
  settings conflicts, delayed titles, reconnect, fork/clone, paused-work idle eviction
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
- `cargo clippy -p taud --all-targets -- -D warnings` and shell syntax/diff checks.

The actual Codex/OpenRouter services and interactive device login were **not**
called in testing. Their live acceptance, the shared-protocol merge and the Rust
settings/menu UI remain release/integration work. An account/endpoint mismatch
on an encrypted checkpoint fails explicitly; use the original account or fork
before that checkpoint. There is no silent lossy fallback.
