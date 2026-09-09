# Tau open work

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

## Queued next: scrolling gets trapped while loading older history

The user reports that after scrolling back, scrolling down can loop or get pulled back as history loads. They want a stable transcript position so scrolling in either direction keeps making progress. They explicitly require finishing the download fix first. Only scroll-code references have been inspected; no scrolling implementation has changed. Reproduce with a long transcript and delayed page loads, then review stable anchors and group growth before choosing a fix. Preserve collapsible thinking/tools and daemon-owned event order.
