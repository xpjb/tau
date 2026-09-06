# Retained transcript contract

## Client hotfix — 0.5.1

Windows and Android 0.5.1 use the existing 0.5.0 daemon and protocol 3. No daemon or Pi change is required.

- Heartbeats use the existing read-only session-list request, with one request every 15 seconds and a 30-second response limit. They do not start Pi or repeat prompts or controls. Native control-frame ping timers are disabled.
- Details follow transcript content order. Tool input and output share one nested tool block, matched by tool-call ID. Presentation groups reference retained entries; storage, ancestry and model input are unchanged.
- Expansion pins the clicked header, including later text layout changes. User input releases that temporary pin.
- Empty loading views use the compact spinner. Repeated explanatory status text is removed.
- Crash reports always encode schema 1, including reports restored from older clients.
- Command completions reload for an existing slash draft after reconnect or chat selection. Unanswered loads stop after 10 seconds; delayed command lists remain usable. Only read-only requests are retried.

Acceptance: 11 client tests passed, including a working message path with control-frame pongs withheld and a failed return path that reconnects without replay. Android and minified Windows builds passed. Windows/Wine checked the prior 0.4.8 and restored presentation on the same fixture, plus details/tool expansion and collapse with a fixed header position. The final Android APK has versionCode 18 and the same signing certificate as 0.5.0. The command follow-up passed the extended desktop end-to-end test for `/model astra`, reconnect retry, timeout and delayed recovery. The live server returned 374 model completions, including Astra, in 11 ms. These checks do not establish physical-device acceptance or the original cause of the user's missing pong.

Status: Tau 0.5.0 deployed; protocol-3 Windows and Android installers delivered.
Replaces Tau protocol 2 without migrating or replacing existing JSONL.

## Release — 2026-09-06

Tau 0.5.0 (release code `d8f7d1c`) and Pi `29b43c7` are deployed together. Ten client
tests, eight daemon tests, strict Clippy, six Pi RPC tests and read-only Pi checks
pass. Windows packaging and signed Android versionCode 17 builds pass. The new
installed Pi and release daemon pass the isolated synthetic pipeline, including
queue prefix/Resume, context counts, existing JSONL, commands, forks and media.
The session-table change is separate in `e2cfe8c` and includes migration and
changed-row write checks.

The user gave the cutover signal after receiving both installers. All 28 chats
were idle. A fresh backup preceded the switch. Post-cutover checks reopened all
28 chats: 12,719 retained entries, 7,417 thinking blocks, an image thumbnail and a
file download. All 27 existing JSONL files retain their original byte prefixes.
Telegram's process and Pi installation are unchanged. Deployment evidence is in
`/root/tau-release/0.5.0/`; backup is
`/var/backups/tau/0.5.0-20260906T080437Z`.

Independent review, physical-device acceptance and a new live-provider run remain
uncompleted; synthetic and read-only production checks do not claim those results.
The checkpoints below record the earlier development state.

## Development checkpoint — 2026-09-06

Pi is committed through `b043181` on `feat/transcript-identity`. The isolated
`/opt/pi-fork/b043181/pi` package passes installed SDK and both CLI checks.
The latest focused run passes 357 tests: 263 coding-agent, 46 agent-core and
48 Codex tests. Read-only lint, types, dependencies, imports, entry graphs,
lockfiles and browser checks pass. Production Pi remains unchanged.

Pi transcript identity and ordered snapshots (`68225d2`, `f6bcc89`), queue controls
(`4f12801`) and identified initial/no-assistant persistence (`3c543e3`) are in place.
`b043181` fixes idle held prompt admission, preserves prepared extension input
across a late pause, and lets Resume consume queued input in a fresh session.
Legacy lazy session persistence and immediate handled commands remain intact.

Tau's daemon projects retained entries, queue revisions, run identity and controls.
Eight daemon tests and strict Clippy pass. Held work prevents idle retirement;
forking requires an unpaused, empty queue. `9f8e857` refreshes actual runtime status
after queued acceptance, keeping idle held input from displaying false activity.
An installed Pi/daemon check confirms this without provider activity.

The shared SQLite store and protocol-3 client are connected (`f0942d1`, `4c2f9b8`).
Nine client tests pass, covering atomic rollback, 10,000 historical entries,
1,000 Unicode deltas, stable rows, interrupted thinking, gaps/stale snapshots,
frozen controls, reconnect, process loss, files and a final draft edit at shutdown.
Android owns its controller in a ViewModel; desktop owns it at application scope.
Local restoration precedes networking. Old history/live/details caches and
whole-history measurement are removed.

The pending menu implements Delete and inclusive Do up to here. Models without
reusable checkpoints show an explicit after-turn action. Held queues expose
Resume; pending waits expose Cancel wait. `4694989` excludes popup labels from
transcript selection and fixes the Windows popup crash. Actual transcript text
remains selectable. `62e5bba` publishes editor changes to the controller first and
applies only current external drafts in the composition apply phase. This removes
the asynchronous draft effect that could overwrite newer typing.

Android release and minified Windows builds pass. SQLite JNI loading, Unicode
transactions and reopening pass under Windows JRE 21/Wine. Android emulator
acceptance covers the 10,002-entry offline fixture, long-press menus, rotation,
force-stop and restoration of thinking, unconfirmed sends and drafts.

Installed Pi -> daemon -> native Windows/Android acceptance passes with synthetic
SSE and all Pi socket access blocked. Checks cover native Delete/prefix menus,
one-request inclusive batches, later arrivals, wait cancellation, the same wait
across Android process loss, reusable reasoning in the next request, buffered
answer cutoff, saved aborted thinking, explicit Resume, exactly-once input and
preservation of the original JSONL prefix. This is not live-provider acceptance.

The final repeat streams 20 thinking updates per second. Windows retains repeated
1,296-character zero-delay input batches. Android retains every tested addition,
restores its 3,905-character draft and visible thinking after force-stop, and
submits that exact draft through the native button. It stays outside the frozen
prefix until Resume. Both checkpoint and final Resume receipts pass.

Earlier evidence remains intact: the paused-idle failure, popup crash, original
Android character loss and an incorrectly transcribed expected string are separate
findings. The corrected assertions do not erase the typing failure. A first draft
guard passed Android; an intermediate Windows stress probe still failed. Final
synchronous/current-draft code and focus-aware native input checks pass on both
clients. The exact cause of every intermediate native timing failure is not proven.

Evidence: `/tmp/tau-pipeline/{native-acceptance,streaming-acceptance,status-acceptance,client-native-acceptance}.json`,
its screenshots/logs, `/tmp/tau-pipeline-before-final-stream.tgz`, and
`/tmp/tau-native-check/`. Helper scripts consume planned fixture state; do not
rerun them blindly. The six synthetic provider responses are now consumed.

Remaining: broader feature/race review, physical Windows/Android acceptance and
live-provider acceptance before a coordinated versioned release. Candidate builds
still say 0.4.8 and remain build outputs only. Do not distribute them, overwrite
released dist files or deploy the protocol-3 daemon alone. No production service,
configuration, chat or deployed Pi installation changed; no rebuild was pushed
or delivered.

## Context meter follow-up

A separate addition shows a small circular meter beside Send. Hover on Windows
or tap on Android shows estimated used/capacity tokens. Unknown usage stays
unknown; offline or inactive-process values are labelled last known. Pi's existing
context estimate is included in `get_state`; Tau carries it in existing session
metadata and the existing client store. Turn/compaction boundaries refresh it;
there is no polling timer, new token counter or transcript protocol change.

Checks pass: six Pi RPC tests, nine client tests, eight daemon tests, read-only Pi
checks, Clippy and both native builds. Windows/Wine hover and Android emulator
tap show 64,000 of 200,000 tokens (32%) through an isolated daemon/Pi fixture.
The Pi source change has RPC coverage; a new installed Pi package is still part
of coordinated release preparation. Evidence: `/tmp/tau-context/`. This addition
is separate from core review baselines Tau `01b4983` and Pi `b043181`.

## Authority and identity

Pi's existing JSONL entries remain authoritative. Tau retains the entry ID, parent
ID, type, selected leaf and all display content. It excludes provider-internal
fields and inline binary image data. Attachments keep the existing authenticated
file endpoints and validation.

Pi adds optional entry origin metadata: requestId for a submitted user prompt and
streamId for a live assistant response. These links are saved with the entry and
survive reconnect, process restart and branching. Request IDs are client-generated
UUIDs; Pi's RPC response ID remains a separate transport identifier. Live response
IDs are UUIDs generated by Pi's RPC adapter. No content, timestamp or occurrence
matching is permitted. Existing extension dispatch, model/retry policy, and the
current Stop and queued-send behavior remain unchanged. New explicit controls
are separate operations, as defined below.

A Pi session-store subscription reports appended entries and branch/head resets.
The RPC adapter reports live entry creation, content changes and saved entries,
and provides one consistent snapshot. Its generation and sequence identify the
snapshot cut and allow Tau to reject buffered events already included in it.

## Tau protocol 3

One entry representation serves both history and streaming. An entry contains its
Pi identity, parent, origin, display role/content, completion state, and attachment
metadata. Non-display Pi entries retain identity and ancestry. Tool results retain
their tool-call IDs. Rendering groups entries; the daemon does not build a second
chat transcript.

Each session has a generation and monotonic update sequence. A snapshot contains
the full retained entry set and selected head at that sequence. An update contains
entry changes, explicit live-to-saved replacement and head changes. The daemon
publishes a snapshot and subsequent updates through the same session gate.
Duplicates are ignored. Gaps or generation changes request a snapshot; they never
clear the displayed transcript. No second durable log of token events is added.

Chat listing, command responses, extension UI and process status remain explicit
operations. Socket write success is not command acceptance. Unacknowledged sends
remain visible as unconfirmed and are never automatically repeated. A saved
requestId resolves a pending prompt even if its acknowledgement was lost.

## Queue and run controls

Status: implemented for acceptance. Delete and Do up to here use the first
pending-message menu: right-click on Windows, long-press on Android. Resume and
Cancel wait are present. Broader edit, pause and reorder UI remain deferred.
Pi owns queue selection and run control; Tau does not add a second scheduler.

An identified queued message contains its requestId, revision, queue kind and
editable content. Tau receives display-safe content and attachment references,
not provider fields or inline image bytes. Queue entries use request IDs, not
saved entry IDs or text matching. An edit keeps the requestId and advances the
revision. The eventual saved entry identifies the delivered request revision.

The operations are:

- Edit or delete a queued message, using requestId and expected revision.
- Do up to here: select the inclusive pending prefix through the clicked message,
  using every member's requestId and expected revision. Deliver those messages,
  in order, together in one next model request at the next reusable thinking
  checkpoint. Keep later messages queued until explicit resume or another prefix
  selection. If already idle, deliver the selected prefix without a checkpoint
  wait. Selection claims the existing messages; it never resubmits them.
- Pause at a boundary, resume, or cancel a pending boundary request.

A boundary is one of:

- `now`: interrupt the active model request. This is not permission to kill a
  running tool. If tools have already started, return the current state and
  require the caller to choose a turn boundary or the existing Stop operation.
- `reasoning_checkpoint`: wait for the next completed, reusable reasoning item
  from the active response, then interrupt. A paragraph or thinking-text delta
  is not a checkpoint. Pi checks provider completion and replay metadata where
  the item is received; a delayed UI event is not a safe control boundary.
- `turn`: finish the assistant response and its tool batch, then act before
  another model request or queued message is started.

Reasoning checkpoints are provider-specific. The current Codex adapter receives
completed items with encrypted replay data, but the provider controls their
frequency. A checkpoint request can wait as long as the response. If the response
finishes first, use its completed turn boundary. Never silently escalate to
`now`. Even a checkpoint interrupt starts a new provider request; it does not
insert an instruction into an active request or guarantee zero repeated thinking.
All received display content remains retained at every boundary.

Pi checks queue revisions, selected session and active run identity when
accepting and applying a control. An active run ID changes for every agent run;
null identifies an idle session. The accepted prefix freezes its member IDs and
revisions, not its length in a changing queue. New messages stay behind that
selection even when their normal queue kind has higher priority. An edit or
deletion of any selected member cancels the pending selection. Editing later
messages leaves it intact. Pi gates its existing queue polling while a boundary
control waits and while the queue is paused; retries and post-run continuation
must obey that same gate. Ordinary prompt admission also obeys it while idle,
including a recheck after asynchronous preparation. Prepared input and extension
context stay queued without repeated preparation. Resume can drain a fresh,
empty-history session; empty history without deliverable input still rejects.
A stale command cannot affect a replacement
run or an already delivered message. Pi serializes selection, interruption and
delivery so other queued work cannot slip between Stop and Send. No partial tool
call is executed or replayed by an interrupt. Editing or deleting a selected
message cancels its pending selection before changing the queue. Already-started
delivery rejects edits and deletion. Stop, model/session replacement or a terminal
failure cancels pending boundary controls rather than moving them to another
run. Stop, model changes and response failure hold affected queued messages; an
automatic retry cannot consume them. Prefix delivery keeps the existing compaction
checks and checks the selection again after any awaited compaction. Initially allow one pending boundary control per session, not a list of
future pause points.

Command acceptance is distinct from the boundary being reached. Snapshot and
ordered queue updates include current queue revisions, pause state and any
pending control with its command ID, target and boundary. This lets reconnect
show "waiting for checkpoint" or "paused" from Pi's actual state. Conflict,
already-delivered and unsupported operations return definite outcomes. Lost
acknowledgements remain unconfirmed until authoritative state resolves them;
process replacement never causes automatic command replay.

The protocol advertises implemented control capabilities, including the boundary
modes available for the selected session/model. Clients send only advertised
operations. Unsupported boundaries return an explicit unsupported result, not a
quiet fallback. Pi and Tau do not advertise a control merely because its type is
reserved. Add optional fields and new requested operations within protocol 3;
keep required transcript semantics stable. Put control state in the existing
snapshot/queue-update shape, so these additions do not introduce unknown update
variants or holes in the shared sequence for earlier clients. Unknown optional
fields/capability names are safe to ignore; unknown transcript mutations are not.

Implement and test queue identity/revision and capability handling with the new
store/protocol. Implement control behavior before advertising its capability;
UI work does not set the wire release boundary. Defer per-message pause markers,
reordering UI, timers and general scheduling.

## Client store

One shared store, backed by androidx.sqlite:sqlite-bundled:2.7.0, owns transcript
entries. SQLite transactions commit entry changes and sync position together.
The store is scoped by connection identity and chat; switching servers cannot
mix their data. An ordered working view and ID index reference those same entries.
The view is not another writable transcript cache.

Session metadata uses a separate `sessions` table keyed by connection and chat ID,
with typed columns for title, status, model, lineage, timestamps and context usage.
Individual state updates write only the matching row. Full session lists reconcile
metadata and order in one transaction, skipping unchanged rows. Removing a row
from a list does not delete its retained transcript. Schema 2 atomically imports
the old summary-list JSON once, preserving all other records and attachment bytes.
This changes neither the transcript representation nor the wire protocol.

Snapshots reconcile by ID. A saved entry's streamId replaces its provisional
entry explicitly. An abandoned provisional entry keeps its received text and is
marked interrupted; it is not represented as saved Pi history. Content omitted
from a summary never overwrites acquired bodies. This protocol sends full text
bodies, so thinking is not separately reacquired from the network.

Pending sends, queue-edit drafts, unconfirmed controls and view preferences are
separate durable records. Pi-confirmed queue revisions and control state commit
with their transcript sequence. Selection, drafts, expansion and scroll state do
not control transcript lifetime. The store outlives an Android Activity. A fresh
process loads the local selected chat before network synchronization. Disk and parsing run off the UI thread. Streaming
changes update the affected entry; rendering is virtualized.

## Required removals

Remove the grouped History/StreamReset/StreamDelta/StreamSnapshot/StreamEnd
transcript protocol, network details endpoints, histories/liveAttempts/messageDetails
as competing client stores, preserveAttemptContent, mergeLiveAttempts, and
text/occurrence-based outgoing reconciliation. Remove collapse-while-fetching
behavior. Keep geometry machinery only where it is required by the resulting UI;
no full-transcript remeasurement is permitted for each streamed chunk.

## Ordered implementation

1. Add and test Pi saved-origin metadata, append/head notifications, correlated
   prompts, and the snapshot/live RPC adapter. Build in an isolated versioned path.
2. Replace Tau's transcript protocol and daemon adapter. Add queue revision and
   control-capability contracts at the Pi boundary before freezing protocol 3.
   Keep current files, process management, commands, auth and file validation.
3. Add the shared SQLite store and tests for atomic snapshots/updates, branch
   selection, origin/queue revision reconciliation, restart and connection isolation.
4. Move shared controller and both platform lifecycles to the store.
5. Replace transcript rendering and persist view state. Delete the old paths.
6. Exercise end-to-end outage, abort, retry, queued/duplicate prompts, branches,
   app recreation and offline restoration. Before advertising a control, test
   delivery races, stale revisions, boundary cancellation, provider capability,
   tool safety and reconnect during a pending control. Check long-history cost.
7. Review the complete deletion and feature diff, build signed Android/Windows
   packages, merge tested commits, push Tau master and deploy while Tau is idle.
   Keep Tele and the working fork installation untouched during development.

## Acceptance and checks

Previously received content survives screen recreation and is readable offline.
Reconnect never blanks content or automatically repeats a command. Saved entries
and the selected branch agree with Pi's JSONL. Late snapshots, duplicate updates,
missing updates and process changes cannot silently regress the displayed state.
Streaming remains responsive on long threads. Existing slash commands, extension
UI, model selection, forking, uploads, images and downloads continue to work.

Use a small set of end-to-end and store/identity tests, not UI layout tests. Run
Rust through cargo nextest and Clippy. Run Pi's dependency, import, entry-graph,
shrinkwrap, type and browser checks read-only. Its default check/pre-commit invokes
an automatic formatter; replace that invocation with equivalent read-only checks.
Never run automated formatting. User chat files and bearer tokens stay out of Git.

## Tau 0.5.2: scoped transcript traffic

Opening a chat returns its snapshot and pending dialogs only to the requesting
connection. Each chat has its own bounded transcript feed. Subscription and the
initial cut share the session content lock. The socket sends the initial cut
before reading later updates. Replacing a subscription waits for the old task to
stop, so a cancelled open cannot publish after its replacement.

Protocol 3 adds optional `open_session.exclusive` and
`resync_required.sessionId`. The current client selects one exclusive feed,
ignores transcript traffic for other chats, and recovers only its selected chat.
Repeated recovery notices share an outstanding open. Session metadata remains
connection-wide. Older clients can retain multiple explicit subscriptions.

This is the urgent isolation patch, not pagination or incremental reconnect.
Opening or recovering the selected chat still transfers a full snapshot. Pi,
JSONL, SQLite schema, transcript layout and queue scheduling remain unchanged.

Checks: eight daemon tests, Clippy, the extended shared-controller socket test,
and a three-client/two-chat WebSocket check. The latter checks private reopens,
live fan-out, snapshot/update ordering, exclusive switching and metadata access.
The warm Android build and its existing signing certificate passed. No Android
UI testing or provider calls were used for this patch.

Tau 0.5.2 was deployed on 2026-09-06 after the game-development run finished and
the user approved the restart. All 30 prior chat IDs and 29 JSONL byte prefixes
were preserved. Health reports 0.5.2/protocol 3. Configuration, the immutable Pi
installation and Telegram were unchanged. Both client installers were sent.

## Tau 0.5.3: on-demand history pages

Protocol 4 replaces full client snapshots with a recent page and current queue
state. Pages target 50 whole saved entries and 256 KiB of entry JSON; an oversized
entry stays whole. Live/interrupted entries belonging to that page remain visible.
`open_session` also accepts outstanding request and stream IDs, so saved work
outside the page can be reconciled without transferring its body.

`get_history(sessionId,generation,before)` follows saved parent links starting at
`before`, inclusive. Its private `transcript_page` reply includes the request ID,
full entries and the next unfetched ancestor. IDs are opaque. A history read does
not change the live sequence. Source replacement and head changes send scoped
resync notices; the next bootstrap is another recent page, never full history.

Store schema 3 clears remote entries and positions once. Sessions, drafts,
preferences, pending work and file bodies remain. The saved position records only
recent entry membership. Reopen reads those indexed rows. Load older first walks
cached parent links, then requests missing history. Transactions precede display;
stale page replies do not replace current content or advance the live cursor.

Each page is a lazy-list item with the existing components and grouping inside.
Tool results across a page boundary remain standalone rather than disappearing.
Loaded older pages stay in memory until leaving the chat, when memory returns to
the recent set. Older disk rows remain cached. This first version starts from the
recent window; it adds neither a replay journal nor a Pi/server memory index.

Checks include nine daemon tests and Clippy, five store tests, two protocol tests,
and two focused controller/socket tests. The paging tests were rerun after review.
An isolated 43,419,257-byte saved-chat copy opened with an 84,009-byte first page
(50 entries) in 202 ms. Five older reads returned another 250 entries; a second
connection received none of those replies. This was a local cold-transcript check,
not a physical-device or provider test. Warm native builds passed; Android UI
acceptance remains deferred. Pi and its JSONL format are unchanged. Deployment
requires the matching 0.5.3 clients because the wire protocol is now 4.
