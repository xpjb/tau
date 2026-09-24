# Tau2 native block sync — implementation checkpoint

**Status:** functioning native read path; not a completed control/write cutover and not release-ready. See [../HANDOFF.md](../HANDOFF.md) for the branch, safety rules, latest validation, and continuation instructions.

## Contract and bounds

`protocol/src/blocks.rs` defines the generic surface:

- `BlockHeader`: ID, optional parent, sibling order, kind, metadata, content version, byte length, sealed flag, metadata revision.
- Kinds: Text, Thinking, Code, Tool, File, Image, State, Queue.
- `FeedRequest`: scope, direct parent, optional `(lineage, sequence)` change cursor, floor hint, optional compound history position `(order, id)`.
- `FeedPage`: reset flag, bounded Put/Remove records, durable cursor, history boundary, delta continuation flag.
- `BlockRequest`: scope, ID, version, byte offset, follow flag.
- `BlockWatch`: one feed, a batch of feeds, or one block body.
- `BulkOffer`: authenticated daemon node ID, UDP port, persistent source lineage.

The control protocol is **version 16** (base was 15). Added control commands: `ConnectBlocks`, `GetSession`, `GetReceipts`; added responses: `BlockConnection`, `Receipts`. Receipt requests are batched four at a time and do not include the original prompt.

Current limits:

| Resource | Bound |
|---|---:|
| Raw chunk/range payload | 16 KiB |
| Serialized block header | 4 KiB |
| Serialized transport header | 5 KiB |
| Block body | 64 MiB |
| Feed page | 32 records |
| Feed batch | 16 requests, also bounded by encoded request size |
| Per-stream credit | 64 KiB of encoded wire bytes |
| Client concurrent streams | 14 |
| Client background/bulk admission | 6 |
| Server streams per connection | 16 |
| Authorization grant | 1 hour; frontend renews after 30 minutes |

Credit is an **encoded-wire** bound, not a raw-byte read-ahead bound for compressible data. Each decompression is separately bounded and hash-checked. Aggregate metadata/cache quotas and strict whole-system fairness are still outstanding.

A feed transmits headers/tombstones only, never block body bytes, even for an explicitly expanded parent's child directory. Body reads do not implicitly fetch descendants. A client must explicitly express each body interest.

## Storage and resume invariants

New workspace crate: `tau-blocks` (`blocks/`). SQLite tables contain flat headers, content-addressed chunks, version/offset chunk references, a coalesced change index, client feed checkpoints, and client tombstones. Foreign keys and triggers remove unreferenced chunks.

All mutators require the caller's transaction. The daemon commits source entries, projected blocks, queue changes and applicable receipts together. Change notifications are emitted after committed sequence changes. The client commits validated metadata with its feed checkpoint, and verified bytes before advertising their offset.

The change index stores **latest Put/Remove per `(scope, parent, id)`**, not an append log of every token. A hot block therefore does not generate an unbounded metadata journal or expire quiet chats' cursors. Intermediate display states need not be replayed. Tombstones are retained; their overall retention/quota policy remains to be implemented.

- Append writes at the exact version/offset, replacing at most the partial 16 KiB tail.
- Genuine content replacement increments the version and invalidates only that block's old content.
- Sealing/metadata updates do not change the content version or retransmit its body.
- Parent moves leave a tombstone in the old directory. Source parent validation rejects cycles/deep nesting.
- History positions include both order and ID. The maximum sibling order and equal-position siblings are handled correctly.
- `floor` is currently a cached-window hint, **not a delta filter**. Filtering changes by an old floor lost new messages below a max-order queue root and missed reorders.
- Older finite pages can extend the cached window, but must not advance the live cursor to the history query's newer database head.
- Cached verified prefixes validate retained chunk hashes. A corrupt shared CAS entry is repaired when verified bytes arrive; it is not blindly reused forever.
- Authenticated source-lineage changes clear the old cache namespace. Late data from the old connection is rejected inside the cache transaction.
- Manual restoration of an older source database with the same lineage still needs an explicit lineage-rotation/recovery policy. Do not claim backup rollback is covered by ordinary crash/reconnect tests.

File staging is deliberately not journaled: `stage_append`, `discard_stage`, `discard_staging`, `publish_stage`. Imports append up to eight chunks per transaction into `@staging`; publication atomically installs chunk references into an existing empty/unsealed file block and seals it, without a whole-file buffer. Staging is inaccessible through the daemon's session-scoped backend.

## Shared data transport

`transfer/src/blocks.rs` implements ALPN `tau/blocks/1` over Iroh/QUIC. The production `TransferProvider::bind_with_blocks` shares its existing endpoint/UDP port between the native acceptor and the remaining legacy blob ALPN. The standalone native `Server` is mostly a test fixture. Do not accidentally instantiate a second production endpoint for files.

Frames have an eight-byte length prefix, bounded JSON header, and optional bounded binary data. Relevant headers are Watch, Credit, indexed Record/Page, Block, Data, End, Error. Optional zstd is independent per chunk; identity/integrity is over raw bytes. Version mismatch restarts at byte zero with the new header.

One frontend `Client` owns the shared endpoint/connection. Chat watches and file downloads use it. Same-peer grant renewal keeps the connection and streams. A changed daemon node reconnects; persisted cache offsets allow continuation if the database lineage survived restart.

Priority/admission:

- Metadata stream priority 10; text/thinking/code 5; file/image -10.
- `watch_bulk` uses the additional six-slot semaphore before acquiring a total stream slot.
- Foreground queue/live/latest-text reads are not gated by stalled bulk slots.
- Frontend watch keys include foreground/background classification, so priority changes cancel and reopen with verified offsets rather than leaving old reads in foreground forever.
- Up to 16 directory interests share a stream. Plans also respect the encoded request-header budget.
- Change hints coalesce for 100 ms (feeds) or 50 ms (body tails); one-second polling is a correctness fallback. Actual state comes from durable queries, not the lossy notification channel.
- Drop cancels streams; owned watch-task collections abort their tasks on drop. Backend body reads are selected against peer cancellation, including file imports.

Still needed: extreme fanout admission/fairness, production byte/latency counters, actual constrained-network testing. Many metadata batches can still fill total stream slots; batching plus the bulk cap is not a proof of arbitrary-fanout fairness. Finite responses can finish the credit half before End is drained; callers currently ignore credit-send errors and use the response read side to decide completion.

## Daemon projection and crash behavior

`daemon/src/blocks.rs` is the producer/backend. `daemon/src/state.rs` schema version **3** initializes native tables and projects existing events/queues.

Projection convention (client presentation conventions, not extra transport RPCs):

- Chat ID is the scope. Event IDs remain stable across live/final phases.
- Tool root header contains render attributes, but **no argument body**.
- Tool input is a separate Code block: `<event-id>/input`.
- Result events are Code children of the corresponding tool; imported orphan results remain root blocks.
- Parent tool metadata carries completion/error status without requiring result content.
- Attachments are File/Image children, `file:<entry-id>`, initially unmaterialized.
- `@queue` is a max-order root Queue block containing queue state without request texts; queued items are separate `queued:<request-id>` children.
- Metadata still uses legacy `Event` attributes as a rendering adapter, with `text` removed. It is not a transcript payload on the new wire.

Live projection reserves display ordering in durable session state without changing the agent's history CAS revision. Restart does not reuse interrupted live positions. Generation is based on database lineage + session ID, not a fresh runtime random ID.

Startup recovery runs before serving:

- Remove unpublished staging.
- Keep live block IDs/chunks, mark interrupted and seal.
- Mark unfinished tools interrupted/outcome unknown, seal their input.
- Clear stale queue run IDs, pause unfinished work, mark waiting control failed as appropriate.
- Complete unfinished receipts with an explicit interrupted/reconcile-effects error, rather than automatically rerunning an uncertain paid operation.

`GetSession` reads durable metadata/runtime state without forcing the provider/agent to load. `GetReceipts` returns bounded acceptance/completion/error/notice reports independent of a display subscription. This is **not** yet a universal mutation journal.

Native file bodies materialize only on a body request. Two imports at a time are allowed. They resolve the durable outbox file, perform async 128 KiB reads, compute SHA-256 outside the database lock, append short staging transactions, check size/mtime, and atomically publish. Duplicate imports recheck publication. Cancellation owns staging cleanup; startup removes crash leftovers. Backend scope checks use a small SQL existence query rather than deserializing a full session/project prompt on each range.

## Frontend cache and interests

`frontend/src/blocks.rs` owns a private SQLite cache per configured identity and a network-runtime watch service. The cache is WAL/FULL, mode 0600. `Store::block_cache` selects the identity-hashed path. Cached views are available immediately after frontend restart, before connection/authentication, but are not advertised as freshly synchronized.

Control events and bulk notices use separate channels. Bulk notices are bounded and coalesced by scope when the controller projects the cache into the existing `Feed`/`TranscriptSnapshot` render adapter. Local outbox JSON is not rewritten on every streamed token.

The controller opens `GetSession`/receipt queries; it does **not** request `OpenSession`/transcript streams. Ordinary loaded root text is prefetched. Thinking/tool interests use the exact Details grouping and tool/section preferences from `details.rs`:

- Closed tool: no child directory or argument/result body interest.
- Open Details + tool: child-directory metadata interest.
- Small Input/Output section: body may be prefetched.
- Large section: body only when explicitly expanded (or Copy Details explicitly asks for it).
- File attachment bytes are not fetched by tool directory expansion.
- Only the selected chat has ordinary content interests; recent-chat bulk warming was removed.

Root history is user-driven. Requested non-root directories automatically page metadata to completion. Queue `available` stays false until all queued-item headers are present; this prevents prefix commands using only the newest 32 items. Queue text completeness is tracked separately; Copy/Edit are not offered for missing/partial text. The UTF-8 render prefix handles a code point split across chunk boundaries without dropping stored bytes.

Tool status lives in the header; large unloaded sections still show an expansion control. Copy Details uses temporary interests, leaving expansion preferences unchanged, and waits for known directory/body completeness. It checks the parent directory cursor against the known tool revision to avoid omitting an output whose completed parent arrived first. It is **not** yet a source-fenced linearizable snapshot of an actively changing hidden subtree; see gates below.

`frontend/src/blocks/files.rs` shares the same `Client` and cache. Downloads are managed tasks, cancellable, with progress at most 10 Hz through the bulk-notice lane. A complete verified cache exports offline. Otherwise data is validated and cached before credit. Export uses bounded reads, SHA-256 when supplied, temporary file + fsync + atomic persist + directory fsync. Destination namespaces include source lineage. A same-size corrupt local export is repaired from verified cached bytes, not trusted by size alone.

## Validation map

Full workspace: **129 passing nextest tests** after the last implementation changes.

Notable coverage:

- `blocks/src/tests.rs`: metadata-only/direct-child reads; exact append-byte accounting and zero-byte seal/resume; replacement; parent isolation; atomic checkpoint rollback; tombstones; header/cycle bounds; corrupt hash/offset handling; paging; coalesced hot blocks; tied/max orders; history/live cursor race; shared CAS repair; unpublished staging/publication/cleanup.
- `transfer/src/blocks/tests.rs`: requested-only body access; live append/resume; stalled incompressible bulk credit without blocking a feed; one peer/connection through grant renewal; rejected unauthenticated reads; bounded codec/hash checks; indexed batch feeds; background admission preserving foreground and metadata access.
- `frontend/src/blocks/tests.rs`: complete multi-page queue metadata, partial-text readiness, exact per-group disclosure, explicit large input, copy fetch interests, split UTF-8, stale-source rejection.
- `frontend/tests/end_to_end.rs`: real native daemon chat + queue + frontend restart with immediate cached content; two-client project behavior; native file grants/downloads with **zero legacy HTTP requests**, shared authorization, verified offline reuse/repair.
- `frontend/tests/recovery.rs`: lost receipt/response recovery without requiring the display/data connection to become ready.
- `daemon/src/agent_test.rs`: existing real tool suite now also exercises native lazy file publication/read/resume.
- `daemon/src/blocks.rs` unit test: a 40 KB raw streamed tool input keeps its ID/version through finalization; sealing produces no body bytes.
- `daemon/tests/sqlite_crash.rs`: actual process crash/durable recovery coverage.

The earlier audit tests are not all acceptance gates for the new design: for example, the oversized legacy transcript-page characterization still intentionally passes because the old server API has not yet been deleted.

## Remaining release gates — do not lose these

### 1. Complete the control/write cutover

- **Uploads are still whole-buffer HTTP**, with the old timeout path. Implement native resumable client-originated content using the shared connection and durable upload state; do not create another endpoint or per-file stack.
- Large prompts, queued edits, settings/project bodies, model catalogs, session lists, and other descriptors still cross the control socket in old shapes. Introduce bounded references/data reads and a strict small-control budget. Current daemon request budget is still 1 MiB; frontend incoming WS limit is 16 MiB.
- Remove legacy `OpenSession`/`GetHistory` subscriptions and TranscriptSnapshot/Update/Page network paths, plus old raw/HTTP file/offer paths. Production frontend no longer uses transcript reads or HTTP downloads, but daemon routes and many raw-WebSocket test helpers still do.
- Adapt `daemon/src/agent_test.rs`'s raw client/open helper to consume native blocks; rewrite the few tests asserting old transcript updates/history. Internal render/provider adapters need not be removed just to delete old wire APIs.
- Existing server WebSocket FIFO/global broadcasts and frontend `Events.send(...).await` in the WS read loop can still couple UI backpressure to heartbeat/control progress. Separate/coalesce health and ephemeral state without dropping durable receipts.

### 2. Close intent/UI convergence races

- Receipts are independent of display now. Audit **all** code that previously relied on FIFO ordering between a transcript update and its response.
- Queue editing cannot start with partial text now, but accepted edit/delete responses may remove optimistic pending UI before replicated queue bytes catch up. Preserve the correct optimistic view until convergence; add delayed-data/immediate-receipt tests, including reconnect.
- General mutation outbox/idempotency, prompt-builtin immediate acceptance, and starter/alias identity issues from the earlier audit are not comprehensively solved. Do not assume the prompt/queue/create receipt subset covers every mutation.
- Review dangling private tool-call context and explicit reconciliation after interrupted external effects; never silently replay an uncertain paid/tool operation.

### 3. Bound interests, storage and work

- Wire interests are selected/disclosure-driven, **not yet actual viewport-driven**. All cached ordinary root text is eligible for prefetch; old cached history/expanded preferences can produce too many watchers and too much work.
- Add viewport/overscan ownership, cancellation, cache eviction/quota policy, source tombstone/metadata quotas, and aggregate bounds. Explicit history tasks should also have clear scope lifetime/cancellation.
- Six bulk permits plus batches are useful but do not reserve a hard slot for every class under arbitrarily many metadata batches/live blocks. Test extreme fanout/fair scheduling rather than only two-stream examples.
- `put`/projection still compare full growing source prefixes; cache snapshots/copy/rendering can repeatedly materialize large text. Wire/storage append behavior is linear, but CPU/allocation work is not proven linear. Use the append primitive and more incremental projections where appropriate.
- Client task lifecycle, cancellation during staging, disk-full recovery, closed channels, auth expiry/renewal over real time, and source swap need expanded stress/fault tests. Source hints are global, causing irrelevant feed queries.

### 4. Preserve complete content while meeting metadata budgets

- Producer metadata (long captions/errors/tool names, etc.) can exceed the 4 KiB header limit. The current hard validation may reject an entire projection/commit. Move full values to referenced bodies and keep bounded summaries; do not silently discard full content or weaken the wire budget.
- Copy Details can return a fully cached observed prefix of a still-live hidden block. Decide/document snapshot semantics or require an explicit fresh metadata checkpoint/version cut when copying hidden live content. Test replacement, deletion, source changes, and cancellation while copy waits.
- Queue completeness/copy guards are implemented, not a proof that every copy/edit/selection action in the app is safe on unloaded data. Audit remaining actions.
- Define lineage rotation for old-backup restore; define migration/cleanup policy for native DB/cache schemas before deployment.

### 5. Measure and certify, then obtain deployment approval

- Add production/test counters for requested/unrequested body bytes, metadata overhead, resumes, hashes, connection count, queue lengths and cancellation.
- Exercise weak links/latency/loss, unread large files, live 40 KB code, many expanded tools, both clients, disconnect/restart, and offline cache corruption. Separate logical-plane isolation from real shared-link congestion.
- No full mobile/manual UI validation yet. No claim of a hard end-to-end control latency target is justified by the current tests alone.
- Re-run managed checks and nextest. **Beta remains stopped and stable untouched until the user explicitly authorizes deployment.**
