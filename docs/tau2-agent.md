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

- `prompt` success means the queue record has been synced to disk. It does not
  wait for a provider or title helper. `uncertain` is always false. A lost network
  response is still possible: reopen with request IDs, or resend the **same ID
  and original text**. Duplicate acceptance never executes a second turn.
- Pending requests, revisions and deleted-request receipts survive restart.
  Recovered pending work is paused; Resume is explicit, not an automatic rerun
  of tools with potentially unknown effects.
- Only `turn` is advertised as a queue boundary. Pause and prefix requests check
  the run ID; edits/deletes check revisions. No reasoning-checkpoint emulation.
- The daemon does not emit `starting` or interactive Pi extension dialogs.
  Those old wire variants can be removed in the frontend's coordinated cleanup.
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
nested wire structs into that crate along with the new protocol variants. Keep
its transfer changes. Keep this branch's native manager and removal of Pi source
positions. Do not reintroduce a second event sequence to resolve the conflict.

## Files and migration

- `TAU_SETTINGS_PATH`: `/var/lib/tau/settings.json` by default. JSON, schema 1,
  camelCase, `daemon`, `agent`, `providers`, `models`, and a monotonic revision.
  Writes use a private temporary file, fsync, atomic rename, directory fsync.
- `auth.json` beside settings: private provider credentials, separate from menu
  settings. Supports Pi's Codex OAuth and API-key record shapes. Codex tokens are
  refreshed in-process, serialized across all chats. API-key environment variable
  names are configured per provider.
- Existing `state.json`: session identity/title/lineage only. The old title prompt
  is imported when settings are first created; subsequent edits have one owner.
- `TAU_SESSION_DIR` is unchanged (including its legacy `pi-sessions` default).
  Existing version-3 JSONL branches remain readable. Native records use the same
  message/parent/origin shapes, plus `tau_queue` records for durable acceptance.
  Full history remains on disk after compaction. Clone/fork never inherits a queue.
- `tool-output/` beside state: full oversized shell output; returned results point
  at the retained log. Short output logs are removed.

To migrate, stop the old Tau/Pi workers, back up state and sessions, and start
once with `TAU_IMPORT_PI_DIR=/root/.pi/agent` and a **new** settings path. Only if
settings do not yet exist, it imports provider/model catalog, selected model,
thinking defaults, steering, compaction/retry, shell settings, global SYSTEM.md /
APPEND_SYSTEM.md, working-directory .pi prompt overrides, Codex priority/native
compaction settings, and credentials (without overwriting an existing auth file).
It does not write to Pi's directory. Remove the import environment variable after
migration. Do not run the old and copied Codex refresh credentials concurrently.

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

Tests use isolated state directories, local scripted HTTP providers, real
WebSockets and real filesystem/shell tools. They do not use production credentials
or send paid model requests. Live-provider acceptance remains a release check.
