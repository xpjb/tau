# Tau2 latest network rewrite — resume here

## Location and safety

- **Active branch:** `feat/tau2-block-sync`
- **Active worktree:** `/root/tau2-block-sync`
- Base: `c5088dc` (`origin/tau2` at the start of this work).
- This is the **native block/network rewrite**, not the separate pause-removal work.
- Checkpoint is local; **not pushed, merged, or deployed**. Run `git log -1 --oneline` for its commit.
- `/root/tau2` is the original, unchanged integration worktree. Common Git repository: `/root/tau/.git`.
- Preserve the unrelated `/root/tau2-remove-pause` worktree and its changes.
- **Do not deploy or start beta. Do not restart/change stable.** Last verified: `tau2-beta.service` inactive, PID 0; `tau.service` active, PID 474496.
- Read `/root/AGENTS.md`: **Clippy is explicitly banned**, including wrappers/aliases/alternate binaries. Use `/usr/local/bin/cargo`. No Cargo built-in test runner; use nextest.

## Why this handoff exists

The conversation hit a disk-quota error. The user freed space, then asked to finish the current checkpoint and leave a handoff discoverable with **“read the handoff on tau2 latest net rewrite branch.”** The conversation was no longer usable for them. Do not depend on its earlier messages: this file and the linked design/status document are the continuation context.

The disk error interrupted a compiler check, not a production service. After the cleanup, checks and the complete test suite passed. No cache purge, wrapper bypass, or service change was performed.

## Status: working read-path checkpoint, NOT finished/release-ready

**Implemented and exercised end to end:** native chat reads, durable block caching, batched header watches, streamed content, and native file downloads on one shared Iroh connection. Prompt/queue/create receipt recovery remains independent of display replication.

**Still unfinished:** native uploads/large client payloads, strict small control messages, removal of legacy transcript/file server routes, bounded viewport/cache policies, and the remaining intent/outbox audit issues. Do not describe the full network rewrite as complete.

Read [docs/tau2-block-sync.md](docs/tau2-block-sync.md) for the protocol, implementation map, invariants, tests, and ordered release gates. The earlier audit is [docs/tau2-protocol-audit.md](docs/tau2-protocol-audit.md); some characterization tests intentionally still describe legacy paths.

## Approved design — preserve these decisions

1. Uniform blocks: text, thinking, code, tool cards, input/results, files/images, state, queue. Storage/sync are flat; parent links describe presentation.
2. Collapsed tool card is its own small block. Fetching its header does **not** fetch child headers or contents. The client chooses interests and disclosure state; the server has no UI-collapse state.
3. Generic reads: ordered direct-child header/change feed from durable cursor; block bytes from version + byte offset, optionally following appends. Long-lived, batchable watches; no per-token RTT.
4. Stable IDs through streaming and sealing. Append suffixes, explicit replacement versions, seal without resending contents. Hidden 16 KiB chunks, raw integrity hashes, optional independent zstd.
5. One control WebSocket and **one shared Iroh/QUIC data connection per client/daemon**. Files must not create another endpoint/connection or use a parallel HTTP download stack.
6. Durable verified cache/cursors. Intent receipts are independent of display convergence. No automatic replay of uncertain paid/external effects after restart.
7. Native Tau protocol; no requirement to keep the old transcript wire compatibility. Existing internal/render adapters can be refactored separately.

## Last validation (after disk cleanup)

From `/root/tau2-block-sync`:

```sh
/usr/local/bin/cargo check --locked --workspace --all-targets
/usr/local/bin/cargo nextest run --locked --workspace --no-fail-fast
```

Both passed, with no compiler warnings. **129 tests passed, 0 skipped, across 13 binaries.** Nextest run ID: `feae8cb2-2894-4781-a8f8-b39b971f752f`.

`git diff --check` also passed. No Clippy and no built-in Cargo test runner were run. No manual mobile UI validation or actual constrained-network latency certification has been completed.

## What was finished immediately before this checkpoint

- Batched up to 16 child-directory watches per data stream, with indexed records/pages and a serialized request budget. This replaces one stream per expanded tool.
- Six background/bulk admission slots within 14 total client streams; metadata and foreground can proceed while bulk streams stall. The queue, live non-code content, and the latest two ordinary text blocks get foreground admission. Priority changes cancel/reopen with verified offsets.
- Header notifications coalesce at 100 ms; followed body notifications at 50 ms, plus polling fallback. This avoids a full header on every provider token monopolizing a weak link.
- Client renews the one-hour data grant every 30 minutes without replacing the data connection. Closed node-identity watches no longer spin.
- Compound history positions `(order, id)` prevent skipping equal-position siblings; deltas are not incorrectly filtered by the initial floor. A queue-only initial root no longer hides later lower-order messages.
- Finite older pages cannot move a live cursor past unread changes or delete equal-order siblings. Non-root directories (especially the up-to-256-item queue) automatically fetch all metadata pages; root history stays user-driven.
- Queue copy/edit controls require fully received text, not placeholders/partial text. Prefix execution waits for a complete queue directory. Unloaded root tool results have a visible loading placeholder.
- Copy Details adds temporary content interests without changing collapse preferences; it waits for known child-directory/content completeness, rather than copying “Loading…”. Tool cards show writing/running/completed/failed/interrupted metadata.
- Native file materialization uses unpublished 128 KiB staging transactions, async file reads, SHA-256, and atomic chunk-reference publication. Cancellation/restart clean staging. Native downloads share the chat connection and verified cache; exports are atomically written and verified, including offline repair of a corrupt local export.
- Added tests for coalescing, history races/ties, corrupt deduplicated chunk repair, staging, batching, stalled-bulk admission, exact disclosure interests, full queue pagination, split UTF-8, stale-lineage rejection, actual daemon file materialization, and raw tool input sealing without replacement/retransmission.

## Suggested next session

1. Read this file and the implementation/status document. Check branch/worktree and service state without changing services.
2. Finish the **write/control cutover**: native upload/large command bodies, small control descriptors, durable acceptance independent of content/render progress, then remove legacy wire routes/tests. Do not add another per-file transfer stack.
3. Before release, close the explicit remaining read-path correctness/resource gates in the status document: viewport interest limits, cache quotas, extreme watch fanout, receipt/display UI races, long metadata fields, copy freshness, and real weak-link metrics.
4. Re-run managed compiler checks/nextest. Update the handoff when pausing. Deployment still requires the user's instruction.
