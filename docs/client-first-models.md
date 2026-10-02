# Client-first model selection and quiet file rows

Feature branch: `fix/tau2-client-first-models`, developed from `05879ef` in
`/root/tau2-client-first-models`. Implementation: explorer `65041c9`, model/send
ownership `6e9895d`. The user subsequently requested integration into Tau2.
This is source work, not a deployment or installer release.

## Changes

- Files shows bare monospaced names. Directories use colour and a plain `/`
  suffix, not a font-dependent triangle. Long names ellipsize on one line while
  retaining a short extension/directory suffix. Existing 44px row targets,
  keyboard/touch navigation and retained file buffers stay intact.
- New-chat tiles previously gated selection on connection, a create receipt and
  completion of a separate `/model` control. Highlighting read the remote session
  summary. Those dependencies and the live model-request/waiting-for-model state
  machine are gone. A click atomically saves the exact local choice and the
  account's remembered default; the tile and new-chat menu share that authority.
- Sending copies the model into the ordinary durable prompt intent. On the daemon,
  any starting-model change commits with the queue entry and receipt, before
  provider execution. Receipt identity includes text **and** model. Same-model
  follow-up sends authored before confirmation do not reset thinking preferences.
- Text sends carry an outstanding named creation, so they do not wait for the
  standalone create response. Both routes use the same journal and topic gate;
  delayed creates cannot reset the model or resurrect a deleted chat. Interrupted
  creation remains explicitly uncertain. File publication still requires its
  owning server chat before upload; it never requires a model-selection round trip.
- The account catalogue is fetched independently of chat commands/transcripts,
  cached across client restart and shared by suggestions and `/model` completion.
  Connection and picker use warm it asynchronously, with a 60-second client
  request cooldown. The existing daemon hour-age/missing-ID refresh, coalescing,
  authenticated limits and cooldown remain. Provider completion pushes fresh
  suggestions without waiting for a chat's usage refresh.
- A cold/failed provider lookup preserves last-known client hints until that
  provider has an authenticated catalogue. Snapshot revisions reject late older
  deliveries within a connection; reconnect permits a restarted daemon's clock.
  Account/source changes fence caches. Metadata never acts as an allowlist or
  overwrites authored choices. Background metadata failures are logged, not
  popup/selection states; explicit refresh failures remain visible.
- Local drafts, attachments, original send IDs, source-lineage fences and uncertain
  control recovery remain. Legacy `waiting_for_model` outboxes are decoded only
  for safe recovery; no new send enters that state. Ordinary established-chat
  `/model` controls retain their existing execution semantics and also remember
  the local preference for the next new chat.

Protocol is **23** so an old daemon cannot silently ignore a pinned model.
Application version and SQLite schema are unchanged. Deploy matching client and
daemon builds together; these changes are not usable with the running protocol-22
beta. No service restart, production-data/auth modification, live provider turn,
release package or stable/master change was performed.

## Validation

All Rust commands used managed `/usr/local/bin/cargo` and nextest, never Clippy or
Cargo's built-in test runner.

- Workspace/all-target compiler check passed.
- **294/294** cases passed, zero skipped, run
  `d511b7d1-6408-4f25-b457-fd6f1d0675e9`: all frontend/daemon library cases plus
  `recovery`, `model_catalog`, `end_to_end`, `activity`, `thinking_level`, `startup`,
  `transcript_prefetch` and `remote_files` integration binaries.
- A final added no-chat/offline suggestions UI case and the explorer typography
  case pass **2/2**, run `744ff0ce-9b71-4894-a489-5aebbc543f22`. The explorer case
  overlaps the 294 run; there are **295 distinct passing cases**, not a claim of
  a single 295-case run. Production source is unchanged after the 294 run.
- Actual desktop/phone GPU gestures select repeatedly on the first offline frame;
  late creation/default/catalogue snapshots leave the selected tile pixels
  unchanged. Both new-chat entry points and cached suggestions without a selected
  chat are exercised. Filename widths match undecorated mono text; long Unicode
  names do not wrap into another row.
- A gated WebSocket returns only Hello: the controller emits the first prompt with
  the final exact choice and original creation while *all* create/catalogue replies
  are withheld. There is no `/model` command on the wire.
- Real daemon/local-provider cases cover one-call execution, no catalogue wait,
  model+queue+receipt rollback under a SQLite fault, duplicate and conflicting
  receipts, concurrent/late creation, deletion, interrupted creation, attachment
  preservation, cached restart and account/source fencing. No real account is used.
- Android ARM64/API29 and Windows x64 MSVC frontend-library compiler checks passed.
  These are not physical-device acceptance or release builds.
- `git diff --check` passed. Existing platform/dead-code warnings remain.

Earlier broad runs identified an old test expecting the deliberately removed
metadata popup and a test waiting for a catalogue notification already consumed
while awaiting creation. The former now asserts nonmodal failure and retained
cache/manual recovery; the latter explicitly queries the snapshot after metadata
is available. The final full selected suite above is green.

Logs: `/tmp/tau2-client-first-{final-check,acceptance-final,final-ui,android,windows}.log`.
Actual synthetic headless frames (not mockups/device screenshots):

- [Phone Files](../frontend/qa/client-first/phone-files.png)
- [Phone local model selection](../frontend/qa/client-first/phone-models.png)

Reproduce frames with `TAU_CODE_PREVIEW_DIR` / `TAU_PROJECT_DUMP_DIR` and the
`code_view::tests` / `project_tests` frontend nextest filters. Physical mobile
font/input acceptance remains open; source integration is not a beta deployment.

## Requested integration

Merged as **`f442792`** onto Tau2 `8a0281a` in the isolated
`/root/tau2-merge-client-first` worktree. The newer shared-scroll implementation is
preserved; the only conflict appended different tests to the same file, and both
sets were retained. The actual combined tree was revalidated, not assumed equal
to the feature tree:

- Managed workspace/all-target compiler check: pass.
- Same broad nextest selection above: **311/311 pass, zero skipped**, run
  `7193c5d5-d237-43a2-becb-c24a4841794d` (10 binaries).
- Managed Android ARM64/API29 and Windows MSVC frontend-library checks: pass.
- Logs: `/tmp/tau2-client-first-merge-{check,tests,android,windows}.log`.

The extra cases include the offline no-chat suggestions test and the newly
integrated scroll tests. Source integration still does not deploy protocol 23 or
change the running beta/stable services.
