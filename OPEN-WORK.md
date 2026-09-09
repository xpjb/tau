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

The reported out-of-order thinking/tool events, stranded interrupted events and new content streaming above them are expected to be covered by this cleanup. The user will report any remainder; do not open a separate investigation or add a special acceptance process for that symptom.

## Parked: existing-chat send confirmation delay

This is more important than new-chat creation when work resumes on responsiveness. Confirmation means real Pi acceptance, not the assistant's first reply.

Confirmed code paths:
- Sending waits for local-store work before writing the prompt.
- Tau can announce Running before asking Pi to accept a prompt.
- After Pi accepts, manager.prompt waits for metadata writes, get_state calls and session-list publication before returning confirmation.
- Client receipt/application is sequential and can wait behind transcript work and database access.

The relative contribution to the user's delay has not been measured. Separate acceptance from post-send maintenance while preserving truthful pending/queued/unconfirmed states and the existing no-replay behavior.

## Parked: eager fetching and performance

The existing eager fetch only loads older history in the selected chat. It does not preload other chats before selection. Implement real recent/unread-chat prefetch, bounded in concurrency, without replacing the selected chat's live subscription or starting Pi merely to read history.

The rewrite replaces the old scroll trigger's mixed pixel/item arithmetic with an item-count threshold. Broader networking, client update cost, cold full-JSONL loading and UI rendering work remain separate performance topics; flatness alone is not a measured speedup.

## Parked: new-chat responsiveness

Creation and first-message title generation are separate from regular reply confirmation. The title subprocess currently runs after Pi acceptance but before confirmation, with a 30-second timeout. Keep new-chat/title work distinct from the frequent existing-chat send path.

## Completed before this rewrite

The existing malformed-JSONL-line edit was reviewed, committed and pushed to master as b5d1478. All 12 daemon tests passed. Production services and Pi were untouched.
