# Tau open work

## Release policy

Batch small QA fixes into one client release. Keep separate commits and focused checks, then run combined acceptance and build one set of installers for the batch. Do not bump versions or ship an installer for each QA item. The 0.5.11 download-only release was premature. The scrolling request is next, but it does not by itself authorize another release.

## Reported: crash while selecting long, off-screen tool text

The user reported a Windows crash while highlighting a long tool-use code block
that extended off screen. The daemon received report
`2a4c7da0-529d-46fb-8857-28030d9dc274` at 2026-09-12T19:25:47Z from client 0.5.12.
It records IllegalArgumentException in MultiParagraph.getPathForRange, called by
SelectionController.draw on the AWT event thread. The daemon stayed running.

This confirms a selection-highlight crash, not its exact trigger or a link to the
user's weak connection. Reports omit exception messages and chat content. Preserve
the long/off-screen detail for reproduction; no selection fix is implemented yet.
Evidence: `/root/tau-checks/text-selection-crash/report.json` and `journal.log`.
Keep this separate from the Enter submission fix.

## Implemented, unreleased: Enter submits single-line forms

Rename and extension input dialogs have no keyboard submit callback. The Settings
connection fields have the same gap. Reuse the existing button actions for Enter
and IME Done; preserve blank-title/connection validation and keep Enter as a
newline in the extension editor and title system-prompt editor. Add no new state,
platform key handler, controller change or dependency.

Plan:
1. Reproduce missing Rename submission with real keyboard input and an isolated
   client/socket fixture.
2. Add shared keyboard actions to the affected single-line fields in TauApp.
3. Check submission, validation, empty extension input and multiline newlines;
   run the client suite under Xvfb and Android compilation.
4. Review, commit off master and merge after acceptance. Hold packaging, versions
   and service changes for the client QA batch.

Acceptance passed on b5f9aec, following e67de59: all 22 client tests, no skips,
and Android compilation. The keyboard/socket regression fails on the original
missing Rename action. Review also reproduced Done submitting an empty title
when text was cleared in the same input event. The shared callback now checks
current text, not a previously composed validation boolean. Tests cover hardware
Enter, IME Done, blank titles, empty extension input, connection validation and
submission, multiline newlines, explicit editor Submit and duplicate writes.

The initial scrolling/clipboard timeout was test setup: this host's xvfb-run
uses a 640x480 screen by default, smaller than the test window. The full rerun
passes with `xvfb-run -a -s '-screen 0 1600x1200x24'` and
`--no-configuration-cache`. No scrolling code or test changed. These are isolated
desktop-JVM checks, not physical Windows/Android acceptance. No installer,
version/protocol bump, daemon/Pi change, provider prompt or service restart.
Evidence: `/root/tau-checks/dialog-enter/acceptance.json`.

## Implemented, unreleased: Tau-only `flag_it(str)`

Record incidental findings for later work without changing the current task.
Implementation: 56c3935, with cancellation and state-path safety in 44026eb.
The existing Tau extension sends a small request using a per-worker flag-only
capability. The daemon appends an ID, time, source chat/title and full text to
`flags.jsonl` beside `state.json`, syncs it, and uses the existing connected-client
notification banner. The full client token stays out of workers. Temporary fork
workers have no flag access. No tracker, offline push, provider call or automatic
investigation is added. The flag log does not change chat activity or history.

Review found caller cancellation could interrupt a write. Accepted flags now
finish in a daemon task even when the caller disconnects. The regression polls
an accepted call once, drops it, and checks both its saved record and two client
notices. It fails before the fix and passes after it. A configured state file
named `flags.jsonl` is rejected as a flag destination rather than overwritten.

Acceptance: 17 daemon tests, strict Clippy, strict extension TypeScript and the
extension HTTP test pass. Checks cover scoped/expired access, Unicode, limits,
concurrent appends, failed writes, cancellation, restart retention and two client
notifications. The tool test fails on the original extension with no `flag_it`.
An isolated daemon with a real installed Pi worker also saves and acknowledges a
flag, hides the full token, and sends the banner, with no model request. That
check uses an ephemeral listener and the production daemon route and extension.
Evidence: `/root/tau-checks/flag-it/acceptance.json`.

No production restart, installer, client edit or version/protocol bump. Deploy
both the daemon and Tau extension when the batch is released. Physical client
acceptance remains a release check.

## Implemented, unreleased: image viewer survives incoming updates

The user reports incoming chat updates closing the image viewer. Its selection
and dialog belonged to a transcript row, so replacing that row could dispose
of the open viewer. Keep one selected image in the current chat panel instead.

Plan:
1. Replace the row-local open flag with a nullable image event owned by ChatPanel,
   keyed by connection identity and session ID.
2. Move the existing dialog and its preview-download effect outside the lazy
   transcript. Keep thumbnail visibility/download behavior in the row.
3. Reuse the existing image renderer, zoom/pan, Close/Escape, retry and download
   cache. Add no controller state, persistence or transcript scroll changes.
4. Extend the existing real-window image check, verify the old-code failure,
   run the client suite and Android compilation, and hold client packaging.

Acceptance passed on 8ab00bb: all 21 client tests under Xvfb, no skips, and
Android compilation. The old code fails the final regression with "Transcript
refresh closed the image viewer". The check keeps the same viewer and image
position through incoming replies, streamed text, live-to-saved replacement,
and a refreshed transcript with a new image row ID. It also checks the preview
leaves the visible transcript, zoom/pan and Close/Escape/reopen still work, and
only one image HTTP transfer occurs. Existing table, scrolling, copy, gesture,
connection and retention checks pass. These are desktop-JVM tests, not physical
Android/Windows acceptance. No version, protocol, daemon, Pi, service or installer
change for this QA item. Evidence:
/root/tau-checks/image-viewer-updates/acceptance.json.

## Implemented, unreleased: Markdown tables without text loss

The user requests real table rows and columns, with no truncated text. The saved
comparison is valid, unfenced Markdown. Tau's table block flattened cells into
code-styled text; the former library table renderer also defaults to one-line
ellipsis. Keep the current synchronous parsing and scroll anchors instead.

Plan:
1. Extend the existing parsed table block with annotated cell rows and alignment.
2. Draw shared column widths, headers and rules. Wrap cells to full height; use
   existing horizontal scrolling when 120 dp per column exceeds the view width.
   Add no line/height limit, text cache, persistent state or new scroll model.
3. Preserve extra body cells: the parser marks content beyond the header count as
   a separator. Reuse its pipe splitter and inline parser for that remaining text,
   rather than silently dropping it. Keep complete source Markdown for copying.
4. Extend the existing parser and real-window transcript checks, then review and
   run all client tests and Android compilation before merging. Hold packaging,
   versions and service changes for a separate release request.

Acceptance passed on f6b3bcf: all 21 client tests under Xvfb, no skips, and Android
compilation. The final real-window regression fails against the old renderer with
"Table cell is not rendered separately at width 1100: Layout" and passes with the
fix. It checks long cells and unbroken text without overflow/ellipsis, wide/narrow
columns, complete copying, sideways scrolling, delayed history with tables, and
existing image zoom. Parser checks cover alignment, inline styles/links, Unicode,
escaped pipes, short rows and extra cells. These are isolated desktop-JVM checks,
not physical Android/Windows acceptance. No version bump, packaging, provider
prompt, daemon/Pi change or service restart. Evidence:
/root/tau-checks/markdown-tables/acceptance.json.

## Deployed: 0.5.12 / protocol 7

The user requested deployment of the completed batch and confirmed no chats are
running. Combine master 86b66c4 with accepted title feature 2ba569b in the isolated
release checkout. Preserve quiet reconnects: title reads stay quiet on connection
loss/timeout, while title-save failures stay visible. No new request state.

Plan:
1. Resolve the title/QA overlap, retain all completed fixes, and set clients and
   daemon to 0.5.12, protocol 7, Android code 33. Keep the live helper unchanged.
2. Run combined daemon tests/Clippy and client tests under Xvfb; build the release
   daemon, signed Android APK and minified Windows installer. Verify signatures,
   versions and artifact hashes before publishing.
3. Verify idle again, back up binary/configuration/data and the title files, stop
   Tau, merge the accepted release onto master, install the daemon and start it.
   Verify protocol-7 health, prior chat IDs and JSONL prefixes, plus a read-only
   title-settings request. Roll back binary/helper on failure without reverting
   newer chat data. Keep Telegram, Pi and configuration unchanged.
4. Send both matched installers and record deployment. No provider prompts.

The preflight check found 80 chats, one live idle queue, and no active/held work.
Combined acceptance passed: 16 daemon tests, strict Clippy, 21 client tests under
Xvfb with no skips, Android compilation, signed Android and minified Windows
packaging. The staged daemon reports 0.5.12/protocol 7. Signing is unchanged.
The Cargo wrapper builds into this worktree; staging now uses that verified path.
Evidence: /root/tau-release/0.5.12/acceptance.json.
Deployment completed at 2026-09-11T06:48:06Z after another successful idle check.
Tau is healthy at 0.5.12/protocol 7; all 80 prior chat IDs and 77 JSONL byte
prefixes are preserved. The authenticated title-setting read matches the shared
default. Telegram, Pi and configuration stay unchanged; no provider prompts.
Both matched installers were queued for delivery. Backup and deployment receipt:
/var/backups/tau/0.5.12-20260911T064802Z and
/root/tau-release/0.5.12/deployment.json. The title deployment gate is complete.
Historical unreleased/held notes below describe the earlier acceptance gates.

## Implemented: flatten the transcript for code health

User authorized execution and permits dropping the transcript store/cache. The goal is simpler, correct code that is easier to change. Performance is not the justification or acceptance gate for this rewrite.

Baseline: b5d1478. Initial estimate: 400–650 fewer production lines. Reviewed result: 431 fewer production source lines (523 added, 954 removed), excluding tests, notes and build metadata. No automated formatting was run.

Implementation:
1. Replace Tau's retained message/content tree with ordered, independently identified content events. Reuse Pi entry/stream IDs and block indices.
2. Translate Pi snapshots and live changes at the daemon boundary. Pi JSONL and Pi's branch model stay intact. The daemon owns event order and lifecycle.
3. Send flat event pages and atomic event updates in protocol 5. Preserve the selected-chat subscription and current request/queue identities.
4. Remove remote transcript persistence, cached parent walks, recent-page membership, live flush tracking, and client-side interrupted-tail reconstruction. Keep SQLite for local drafts, pending sends/controls, attached files, preferences and session metadata. History reloads after an app restart. Existing remote cache rows can be discarded.
5. Keep thinking and tools in collapsible UI groups formed across loaded events. Fetch pages are transport batches, not UI containers.
6. Update focused existing pipeline coverage, build matched daemon/clients, review the diff, commit and merge. No automatic code formatting. No live deployment as part of this task.

Acceptance: all 12 daemon tests and all 15 shared/desktop client tests pass, as does strict Clippy. Release daemon, signed Android 0.5.8/versionCode 28, and minified Windows 0.5.8 packaging pass. Checks cover event order/lifecycle, paging, stable presentation keys, queue identities, local-work migration/rollback/restart, connection recovery and attachments. No native UI test, provider prompt or live restart was part of this rewrite.

The recovery-order defect is reproduced and fixed on fix/recovery-order, now deployed in 0.5.10. Recovery now assigns order from the selected source branch and block order, inserts live content at its source parent, and keeps abandoned work between known neighbors. Stable event IDs survive a new generation; numeric positions are rebuilt. Skipped blocks and conflicting order trigger a snapshot rather than an out-of-order update. Fourteen daemon tests, strict Clippy and the client store tests pass. The exact earlier user incident was not captured; the tests reproduce the faulty ordering paths. The 0.5.9 client hotfix did not include this daemon fix.

## Client hotfix: send crash, older-history loading and version labels

Windows crash receipts identify an IndexOutOfBoundsException in the pending-message lazy-list key callback. Version 0.5.9 captures one list for its count, keys and item bodies, including transcript groups. The older-history test reproduces the false “Connect to load older history” error during refresh; reads now wait for synchronization/reconnect and resume automatically. The visible older boundary loads successive pages, with an explicit retry button.

The old shared TauClientVersion value was still 0.5.5. Settings and crash reports were already using it; installer versions were separate. Version 0.5.9 makes both installers read that existing constant as well. Fifteen client tests, both builds and an isolated 40-send/three-page desktop UI check pass. The deployed 0.5.8/protocol-5 daemon stays unchanged.

## Parked: existing-chat send confirmation delay

This is more important than new-chat creation when work resumes on responsiveness. Confirmation means real Pi acceptance, not the assistant's first reply.

Confirmed code paths:
- Sending waits for local-store work before writing the prompt.
- Tau can announce Running before asking Pi to accept a prompt.
- After Pi accepts, manager.prompt waits for metadata writes, get_state calls and session-list publication before returning confirmation.
- Client receipt/application is sequential and can wait behind transcript work and database access.

The relative contribution to the user's delay has not been measured. Separate acceptance from post-send maintenance while preserving truthful pending/queued/unconfirmed states and the existing no-replay behavior.

## Implemented: keep history, warm chats and bump on activity

The user also requests loaded history retention, background warming for running/unread/recent chats, and chat-list bumps for assistant replies and stops. These are separate from the ordered-flat-events requirement.

Facts: selectSession explicitly trims the old chat and invalidates both chats. The server replaces the previous subscription on every OpenSession. The client ignores off-screen transcript updates. Session order only changes on the existing metadata touch paths, not assistant replies/stops.

Plan:
1. Finish and commit the daemon order fix with failed-before/passed-after checks.
2. Keep loaded rows and per-chat feeds across selection. OpenSession replaces only that chat's feed. Version the matched feed semantics so a warming client cannot silently use a one-feed daemon.
3. Use those same feeds for bounded read-only warming: selected first, then running/starting, unread and recent chats. Keep two background reads in flight and target a recent history window. Retain local work and the no-replay rule. A failed background read must not loop or block other chats.
4. Put reply and stop bump rules at the daemon event boundary. Publish the resulting session order to every client. Warming, paging, thinking/tool deltas and idle sleep do not bump a chat.
5. Check multiple feeds, off-screen updates, switching without refetch, unread warming, reconnect and bump/no-bump cases. Build matched clients and daemon. Hold the production restart for approval.

Retention and warming are implemented on the branch. Protocol 6 keeps one feed per opened chat until disconnect; opening one chat no longer closes the others. The client retains all loaded rows across switches, applies off-screen updates and warms running/starting, unread and five recent chats, then restores other retained feeds after reconnect. Two background reads run at once; a warm window targets 150 events or 768 KiB, while explicit scrolling can read more. Failed reads wait for selection, resync or reconnect rather than spinning. The focused paging/retention/warming tests and all fourteen daemon tests pass, including a multi-feed check that starts no Pi process. A duplicate-open race found by the full suite was fixed by retaining the pending open until snapshot application finishes. The full sixteen-test client suite now passes.

Activity bumps are implemented on the branch. The content projection flags user messages, first assistant text and saved assistant text/images. Run settlement and stopping/failure of an active process also touch the existing activity timestamp. Thinking, tool deltas, paging, warming and idle sleep do not bump. Timestamps increase across chats even within one clock millisecond. Read markers replace raw Pi heads for unread state and persist in existing connection metadata; warming never marks a background chat read. This removes the all-chat transcript locks from session-list publication. Cached reads also bypass the prompt/control operation gate. All fifteen daemon tests, sixteen client tests and strict Clippy pass. The 0.5.10 daemon, signed Android versionCode 30 and minified Windows package are built. The isolated desktop UI check passed three history pages and forty sends with no crash. The user authorized immediate deployment and restart, including interruption of the active run. Daemon 0.5.10/protocol 6 is now live; all 69 saved chat IDs and 66 JSONL prefixes passed verification. Telegram and Pi are unchanged. See /root/tau-release/0.5.10/deployment.json.

The rewrite replaces the old scroll trigger's mixed pixel/item arithmetic with an item-count threshold. Broader networking, client update cost, cold full-JSONL loading and UI rendering work remain separate performance topics; flatness alone is not a measured speedup.

## Parked: new-chat responsiveness

Creation and first-message title generation are separate from regular reply confirmation. The title subprocess currently runs after Pi acceptance but before confirmation, with a 30-second timeout. Keep new-chat/title work distinct from the frequent existing-chat send path.

## Completed before this rewrite

The existing malformed-JSONL-line edit was reviewed, committed and pushed to master as b5d1478. All 12 daemon tests passed. Production services and Pi were untouched.

## Completed: remember saved downloads after reopening

The saved path/Android URI lives only in AttachmentDownload UI state. The byte cache survives, but the export link does not. Reopening clears that link and hides Open/Show/Extract; pressing Download can create another exported copy even when no network transfer is needed.

Plan:
1. Reproduce the lost saved-file reference by extending the existing attachment/controller test through another app restart.
2. Persist only completed SavedDownload references in the existing scoped SQLite records, using connection/chat/Pi entry identity. Restore them into the existing attachment state when loading the connection. No new cache, table or protocol change.
3. Check file/URI availability on restore and before opening. Remove stale links, keep explicit retry, and preserve the saved link if an image preview fails. Finish export and receipt persistence together during orderly app shutdown.
4. Check offline restart, missing files, same-name files, account separation and failed transfers in the existing tests. Build client-only 0.5.11 installers; keep daemon 0.5.10/protocol 6 and production services unchanged.

Older exports have no recorded identity. Do not guess a match from filename alone. This fix preserves downloads saved by the updated client; it does not invent links for previously untracked files.

The restart regression failed before the fix and now passes. Completed references restore into the existing transfer state; missing-file checks use conditional removal so an old check cannot erase a newer save. Preview failures keep Open available for the exported file. Android 8/9 exports now avoid overwriting same-name files, matching newer Android and Windows. All sixteen client tests, signed Android versionCode 31 and minified Windows packaging pass. An isolated desktop-JVM/Xvfb check visibly restored Open after restarting, with one download and one exported file. Client 0.5.11 is ready; daemon 0.5.10/protocol 6 and services are unchanged.

## Released in 0.5.11 rebuild 2: scrolling gets trapped while loading older history

The user wants scrolling to keep making progress in either direction while history
loads. They explicitly request a code fix, not packaging, shipping or a version
bump. The download fix was completed first; this change stays in the QA batch.

An isolated desktop UI reproduced the loop: after scrolling up to row 1424, 600
downward wheel ticks ended near row 1156 and fetched six more older pages. The
saved-text renderer measured raw Markdown first, then replaced it asynchronously
with a shorter formatted layout. Lazy-item re-entry repeated that height change
and pulled the reading position backward.

The fix parses saved text before its first measurement and remembers that document
for the composition. It removes the changing placeholder; existing content keys
and pixel offsets now remain stable across re-entry and page loads. There is no
new coordinate model, scroll override or page eviction, and collapsible details
stay intact.

Acceptance: the checked-in UI regression fails with the old renderer and passes
with the fix. It drives actual wheel input, holds/releases an older page and checks
stable key/offset, monotonic downward movement and return to the newest content.
Both separate replies and one expanded thinking group pass. All seventeen client
tests pass under Xvfb, with no skips; Android compilation also passes. The UI test
skips without a display. Evidence: /root/tau-checks/scroll/acceptance.json. Versions,
installers, daemon, Pi and production services remain unchanged.

The user subsequently requested builds and authorized daemon deployment if needed. The combined 0.5.11 rebuild 2 retains the public version, raises Android versionCode to 32, and uses r2 filenames without replacing the archived first build. Signed Android and minified Windows packaging passed against the accepted scrolling code. No daemon code is pending; the installed and running 0.5.10 binary matches its accepted SHA-256, so no restart is needed.

## Implemented, unreleased: Copy message excludes Details

Approved behavior: Copy message includes only actual text parts outside Details,
regardless of expansion. Keep the whole bubble's main prose, its raw Markdown and
blank-line separators. Keep explicit Copy selection literal. Image placeholders,
attachment controls and failure labels stay excluded; pending-message copy stays
unchanged.

The existing inline copy expression now reads text presentation parts instead of
raw rows. It has no expansion-state check and leaves the selection override alone.
No new state or helper was added.

Acceptance: the extended real-window transcript test reproduced copied tool output
and errors with Details collapsed. It now passes with Details collapsed and
expanded, preserving both prose blocks, raw Markdown and blank lines while
excluding thinking, tool input/output/errors and an image marker. All seventeen
client tests pass under Xvfb with no skips; Android compilation passes. These are
isolated desktop-JVM checks, not physical Windows/Android acceptance. Evidence:
/root/tau-checks/copy-message/acceptance.json. No version bump, packaging, daemon
change, provider prompt or service restart.

## Implemented, unreleased: running takes priority over unread

The user wants unread to mean a finished run is ready to check. The current list
label checks unread before running, and its separately stored unread set has no
status rule. The durable readAt markers already retain unseen activity.

Plan: derive isUnread(session) from status, selection and readAt instead of storing
another set. Use that same rule for the list and history warming. Running/starting
hide unread without consuming the read marker; status-only completion shows it.
Keep old failed-response badges below active status too. Extend the existing
connection test through activity, status-only transitions and restart, then run
focused/full client checks and Android compilation. This is a separate client-only
commit; leave the accepted title-prompt branch isolated. No release or restart.

Acceptance: the extended connection regression fails on the old starting/unread
behavior and now passes through starting/running activity, status-only completion
and resumption, sleeping/error, selection, warming and restart during a run. The
read marker stays unchanged until selection; completion needs no extra activity
update to reveal an unread chat. The separate unread set is removed, and both list
and warming use the same constant-time query. Old failure badges no longer hide
starting/running in the list or chat header.

All 17 client tests pass under Xvfb with no skips; Android compilation passes.
Evidence: /root/tau-checks/unread-running/acceptance.json. No daemon, protocol,
version, installer, title-script, Pi or production service change. The separate
accepted title-prompt branch still awaits its live-script deployment gate.

## Implemented, unreleased: Failed means a stopped model failure

The user parked the Do up to here / queue-pause question. This QA item changes
only failure status. The shared header/list helper currently promotes isError on
the last non-system event, so a bash/tool error can be mistaken for a model failure.

Plan: move that existing query into transcript presentation, reproduce the tool
error through the connection/store test, and require the latest user/assistant
event to be a non-live assistant outcome with stopReason=error. Ignore tool/system
rows, retain a newer user message as a boundary, and keep the running/starting
guards. Keep tool errors visible inside Details. No new state, daemon/protocol
change, queue change, release or restart; the title-prompt branch stays isolated.

Acceptance: the connection/store regression reproduced a live bash error being
classified as a model failure. It now checks live/saved tool errors, live versus
terminal model errors, later tool/system notices, newer user input, recovery,
abort and interrupted history. Tool errors still appear inside Details without a
response-failure card. The existing test is now named
TauConnectionTest.derives_unread_and_model_failure_status.

All 17 client tests pass under Xvfb with no skips; Android compilation passes.
Evidence: /root/tau-checks/model-failure/acceptance.json. No new state or deployed
change. The queue-pause question stays parked, and the title-prompt branch remains
isolated behind its existing deployment gate.

## Implemented, unreleased: normal zoom in the full-screen image viewer

The user chose persistent zoom instead of spring-back peek. Scope is the opened
image viewer, not inline previews, the transcript or the rest of the app.

Facts/design: LocalImage already owns bounded decoding and fitted image drawing.
Reuse that bitmap and add a clipped drawing transform, not a new image cache or
layout/scroll model. Keep only scale and pan in the open viewer. Pinch zoom stays
under the moving finger midpoint; wheel zoom stays under the cursor. Drag pans.
Clamp scale to 1–8 times fit and pan to image edges. Add minus, plus and Fit
controls. Fit, closing/reopening, or a changed viewport returns to the fitted
view. Close stays explicit through Close/Back/Escape, rather than any image tap.
No spring-back timer, rotation, inertial pan, new dependency or stored preference.

Plan:
1. Extend the isolated real-window regression to open a downloaded image and
   reproduce missing wheel zoom.
2. Add the shared bounded transform and gesture/control handling in LocalImage;
   enable it only for the existing full-screen viewer.
3. Check geometry, common multi-touch input, real mouse zoom/pan/reset and viewer
   dismissal, then run all client tests under Xvfb and Android compilation.
4. Review the diff, commit off master, and merge after acceptance. Keep this in
   the unreleased QA batch; leave the title branch and production unchanged.

Acceptance: the real-window regression reproduced wheel input leaving the image
at its fitted size. It now checks anchored wheel zoom, drag pan, persistence after
release/hover, Fit and plus/minus controls, taps without dismissal, close/reopen,
and one HTTP download across viewer operations. Existing scrolling/copy checks
still pass. Shared multi-touch input checks pinch with a moving midpoint,
one-finger pan, release persistence and resize-to-fit. Geometry checks cover
1,200 steps across viewport/image aspect ratios, scale limits and pan bounds.

All 19 client tests pass under Xvfb with no skips; Android compilation passes.
Evidence: /root/tau-checks/image-zoom/acceptance.json. These are desktop-JVM checks,
not physical Android/Windows/trackpad acceptance. Test-only iterations corrected
pixel-color matching, signed-zero equality, a stale cached DISPLAY and a short
suite timeout. No version, installer, daemon, protocol, title-script or service
change. The next requested task is routine network-error spam on reconnect.

## Implemented, unreleased: quiet background reconnect failures

The user wants routine network failures, including Android resume/disconnect and
unresolved hosts, represented by the connection indicator rather than banners.
The reconnect catch currently copies every exception into the global error field;
automatic history/command sends can do the same through the operation wrapper.

Plan:
1. Reproduce repeated connection-refused banners with an isolated socket fixture.
2. Classify connection failures by exception type/cause, not message text. Keep
   reconnect/status/retry behavior, but skip banners for those failures and for
   automatic reads interrupted by a lost connection or timeout. Report receive/application
   failures at their boundary; keep protocol, storage and user-action errors.
3. Extend the same fixture through retry, protocol mismatch, recovery, rejected
   user action and another outage. Run focused checks and combined client
   acceptance, then commit/merge separately. No new state, replay, timing change,
   release or production changes; image zoom is already accepted at 0ea6abb.

Acceptance: the socket fixture reproduced a Connection refused banner before the
fix. It now covers repeated refused connections, protocol mismatch, recovery,
quiet history timeout with its cursor retained, a rejected create action, another
outage and draft retention. Typed/wrapped host, unresolved-address, socket-abort
and heartbeat failures classify as connection failures. The existing connection
regression now requires command-load timeout cleanup without a banner.

All 20 client tests passed under Xvfb with no skips, including image zoom. Review
also covered the automatic ListSessions send in the quiet-read guard; the final
focused connection test and Android compilation passed after that one-line
addition. Evidence: /root/tau-checks/quiet-reconnect/acceptance.json, full.log,
results/, final-focused.log and final-focused.xml. No physical device acceptance
is claimed. The connection indicator, retry/heartbeat timing, failed-read state,
read-only retry policy and user-action reporting remain unchanged. No new stored
state, dependency, protocol, version, installer, daemon or production change.

## Implemented, held for deployment: editable title system prompt

The user wants the full existing title prompt string editable in Settings. Keep
its examples and {text} substitution visible; add no hidden instructions, model
change or title-latency work. Save one optional override in existing daemon state.
The default text stays in one file shared by the daemon and title helper. Empty
strings and whitespace are literal overrides; reset copies the default into the
editor. Other clients read the shared setting when opening Settings or reconnecting.

Plan:
1. Add protocol-7 get/set title-prompt requests and a request-linked prompt reply.
   Persist the exact override with the existing state write gate. Reads and edits
   never run Pi or the title model. Return the effective and default strings.
2. Pass the override through the title helper's existing JSON input. Preserve its
   legacy text-only input, CLI override, text truncation and generation options.
3. Add a multiline Settings editor and Save/Reset controls. Reuse request tracking
   for loading, failure and timeout; keep unsaved input during reconnect and never
   replay writes. Add only prompt data and pending status to client state.
4. Test the real daemon socket/state/Python path with fake Pi and llama_cpp, then
   client read/save/failure/reconnect behavior. Run daemon tests/Clippy and client
   tests under Xvfb plus Android compilation. No model or provider traffic.
5. Review and commit on fix/title-prompt. Keep the accepted branch unmerged until
   deployment is approved: production executes /root/tau/scripts/title_gen.py from
   the main checkout. No installer/version bump, live script edit or restart.

Acceptance: all 16 daemon tests and strict Clippy pass. The socket/state/helper
check covers authentication, shared reads, exact whitespace/Unicode and empty
values, oversized writes, failed persistence, restart, unchanged existing titles,
and the exact string delivered to fake llama_cpp. Legacy helper input and CLI
fallback still work. All 18 client tests pass under Xvfb with zero skips; Android
compilation passes. Client coverage includes rejected and timed-out saves, stale
replies, restart and reconnect without replaying writes. The default text is
byte-for-byte unchanged. No physical Windows/Android or real model test is claimed.

The title-generation and fallback helpers were inlined into their sole caller.
Only this branch has protocol 7; master and production remain unchanged. Evidence:
/root/tau-checks/title-prompt/acceptance.json. Keep the branch isolated until the
live-script deployment gate above is approved, then ship matched clients/daemon
and the title helper with its adjacent title_prompt.txt file.
