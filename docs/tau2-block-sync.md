# Tau2 native block sync — implementation status

**Native read/write/control cutover implemented; release certification is not complete.** See [../HANDOFF.md](../HANDOFF.md) for the branch, safety rules and validation. The earlier [protocol audit](tau2-protocol-audit.md) is historical design/evidence, not the current implementation status.

## Wire contract

Control protocol **17** (original integration: 15; read checkpoint: 16). Production has **one authenticated control WebSocket and one shared Iroh/QUIC endpoint/connection per client/daemon**. Chat bodies, descriptors, large command inputs, uploads and downloads use ALPN `tau/blocks/1`. There is no second per-file transport.

- Uniform flat `BlockHeader`: ID, optional parent, sibling order, kind, metadata, content version, byte length, seal and metadata revision.
- Direct-child header/change feeds use durable `(lineage, sequence)` cursors; history positions are `(order, id)`. Feeds contain headers/tombstones, never bodies or implicit descendants.
- Explicit body reads use version + verified byte offset, optionally following appends. Replacement increments the version; append/seal do not resend complete prefixes.
- `ConnectBlocks`, `GetSession`, `GetReceipts`, `GetOperation` are small control operations. Receipt queries never require opening a transcript or loading provider history.
- Large outgoing requests upload the complete immutable `ClientRequest`, then send `Input(ContentRef)`. The daemon checks lineage, hash, sealing and matching inner/outer request IDs before reserving/executing it.
- Large responses, settings, lists/catalogs, project prompts and notices become immutable `@control` bodies with a small `Data` descriptor. Small acceptance reports/registered-operation markers travel before descriptor bodies. Full errors/notices remain in the body, not silently truncated.
- Retired `OpenSession`, `GetHistory`, `TranscriptSnapshot/Page/Update` cannot serialize/deserialize on the wire. Some types remain internal render/test adapters. HTTP file/download/offer/upload routes and the legacy Iroh-blobs stack have been removed. Health, WebSocket and crash telemetry HTTP routes remain.

### Bounds

| Resource | Bound |
|---|---:|
| WebSocket frame and whole message, both directions | 4 KiB |
| Raw native chunk | 16 KiB |
| Serialized block / transport header | 4 KiB / 5 KiB |
| Block / descriptor body | 64 MiB |
| Uploaded command | 8 MiB |
| Uploaded file | 50 MB |
| Feed page | 32 records |
| Feed batch | 16 requests and encoded request budget |
| Per-stream credit | 64 KiB encoded frame bytes |
| Client stream classes | 2 metadata + 4 foreground + 6 bulk/upload + 2 descriptor |
| Server streams per connection | 16 |
| File imports / frontend downloads | 2 each |
| Source pending upload reservations | 1 GiB / 4096 records |
| Source descriptor leases | 256 MiB / 4096 bodies, 24-hour expiry |
| Verified client body cache | 512 MiB logical CAS bytes |
| Selected viewport body working set | 128 MiB, up to 30 root interests |
| Frontend control event mailbox | 512 events / 128 MiB, plus terminal overflow indication |
| Control output queue / health queue | 32 / 8 bounded frames |
| Daemon control jobs | 32 global / 8 per socket |
| Outstanding generic client mutation intents | 32 / 16 MiB serialized inputs |
| In-flight/queued descriptor payload reservations | 128 MiB |

Credit counts encoded application frames, **not raw decompressed bytes or actual UDP traffic**. Decompression/hash validation are separately bounded per chunk. These bounds are not a proof of complete metadata, retained-file or whole-system memory quotas.

## Durable data and uploads

`tau-blocks` stores headers, CAS chunks, version/offset references, coalesced changes, feed checkpoints and tombstones. Mutators require the caller's SQLite transaction. Verified bytes/checkpoints are committed before credit or advertised resume offsets. Hash-corrupt shared CAS entries can be repaired with verified bytes.

The change index retains latest Put/Remove per `(scope, parent, id)`, not every token. Parent moves leave old-directory tombstones. Finite history pages cannot advance live cursors past unread changes. Equal/max-order siblings and queue-only initial windows are covered by tests. Tombstone/metadata retention still needs further bounds.

Unpublished lazy file imports use `@staging`, bounded async reads and atomic chunk-reference publication. Startup removes staging; requests cannot read that scope. This is distinct from durable `@uploads` reservations, which survive restart.

`blocks/src/uploads.rs` binds upload IDs to complete specs, verifies contiguous prefixes and idempotent duplicate chunks, rejects conflicting IDs/gaps/hashes, and seals only complete content. Old pure-data leases expire after seven days on a subsequent begin; operation receipts do not expire with them. A command reference includes the content hash, so a changed or expired upload cannot substitute different bytes into an old reference.

`daemon/src/uploads.rs` hashes completed input in bounded reads outside the DB gate. Files are fsynced and atomically published to a deterministic destination derived from the complete upload spec; directory publication is fsynced before recording sealed ownership. Retry after a lost final ACK returns the same path. After publication, the export owns the bytes and redundant upload chunk references are removed. Completed exports are not counted as outstanding upload reservations; this is **not** a global retained-file quota. Unsealed reservations remain resumable. Export-orphan cleanup and more cancellation/power-loss cases remain to be tested.

Frontend imports fsync file and directory ancestry before saving an attachment intent. Native uploads hash/read bounded chunks, seek to durable resume offsets, and use the same shared `Client`. Large command uploads and file preparation run outside the WebSocket reader; an epoch check prevents submitting an old prepared request on a new control connection.

## Mutation ownership and recovery

Daemon database schema **4** adds `operations(id, payload, response)` for create/fork/clone/rename/close/delete session, project mutations/moves and settings updates. The full canonical command is reserved before `Accepted` and before its effect. IDs cannot be rebound. A lost response is recovered by `GetOperation`; a duplicate does not execute the mutation again. Registry rows survive session deletion. Compatible legacy creation receipts are reconciled rather than treating them as new creates.

Reservation, existing effect transactions/filesystem work and final response persistence are not one universal transaction. Startup records an explicit uncertain/interrupted outcome for a reserved operation without a final response; it **never replays the effect**. Failures after reservation are conservatively uncertain. Native prompt/queue receipts remain their existing scoped journal; it is not automatically correct to treat every path as a single global transactional actor.

Frontend generic mutations are saved in `Account.pending_controls` before submission. Reconnect/restart queries outcomes in bounded cohorts rather than replaying effects. Unknown/uncertain intent stays local; explicitly repeating an identical command reuses its ID. Acceptance is recorded independently of descriptor completion. Deleted-project chat ownership is retained for outcome recovery. Local outcome application finishes before removing the saved intent; an existing recovered fork draft is preserved rather than overwritten. A comprehensive outbox inspector/cleanup UX and multi-record local crash audit are still needed.

Prompt, queue, abort and model-selection intents use the durable per-chat outbox. Queue edit/delete acceptance retains the optimistic overlay until complete revision/text/directory convergence, including across restart. Partial text cannot be copied or edited. Abort commits paused queue/receipt before signaling cancellation.

Compaction reserves its builtin receipt, publishes Running, starts owned work and immediately returns Accepted instead of holding the operation mutex through the provider call. Completion is persisted/reported separately; restart makes interrupted work explicit and does not rerun it. **The general first-prompt path still constructs/loads the runtime before acceptance; moving acceptance completely ahead of cold provider-history loading needs a further actor/queue audit.**

New client-named creation keeps its exact UUID rather than aliasing an older starter. Legacy alias recovery remains, and its filesystem/SQLite/account crash windows need explicit migration fault coverage.

## Scheduling, projection and UI

- Hard stream classes prevent an arbitrary metadata queue from taking foreground/descriptor permits or bulk uploads from taking health/control resources.
- Metadata watches batch within both count and encoded-size limits. Frontend live watches yield their class permit after five seconds and reopen from committed cursors/prefixes, so queued cohorts are not permanently starved. This is not a measured WAN latency guarantee.
- Feed hints coalesce at 100 ms; followed body hints at 50 ms, with polling fallback. Grants last an hour and renew at 30 minutes without replacing a healthy same-peer connection.
- Plans/configuration use latest-value channels; history requests are bounded. Scope changes cancel explicit history jobs. Drop cancels stream/task ownership.
- UI layout supplies viewport plus overscan interests. Offscreen disclosure preferences do not cause unconditional body prefetch. Headless/bootstrap selection uses recent roots. Queue bodies remain separately required for operation correctness.
- Copy Details advances through bounded cohorts without changing disclosure preferences. It waits for sealed bodies, complete relevant metadata and completed/failed/interrupted tool state. It is a complete **observed sealed** snapshot, not a source-linearizable cut of an actively changing hidden subtree. Clipboard output is capped at 64 MiB; replacement/deletion/source-swap waiting cases need broader tests.
- Long event attributes become hash-referenced `<event>/meta` bodies; small summaries remain in headers. Tool correlation IDs are normalized when oversized. The frontend preserves authoritative phase/error/order metadata while resolving full attributes.
- Growing live source events use append hints and `tau_blocks::append` instead of repeatedly reading/validating the complete old DB prefix. Existing provider/render adapters can still allocate full growing values; cache snapshots still materialize more history than the active viewport.
- WebSocket health has a separate prioritized bounded write lane. Frontend event enqueue does not await UI consumption; ephemeral state/heartbeat events coalesce while durable responses are preserved within the mailbox bound. Overflow fails closed with a visible reconnect/reconcile error rather than allowing unbounded memory.
- Session/project broadcasts are small resync/head notices, not queued copies of growing lists. Descriptor jobs have epoch/key generation fencing so a slow old descriptor does not overwrite a newer received state for the same key. This is not a universal source revision fence for every cross-message UI transition.

## Validation

See HANDOFF for the latest commands/run ID. Compiler checks and rustdoc use the managed Cargo wrapper; behavioral tests use nextest. No Clippy and no built-in Cargo test runner.

Coverage includes:

- Existing block integrity, suffix-only append/seal/resume, CAS repair, replacement, tombstones, compound paging/cursor races and staging tests.
- Persistent native uploads across client/server restart, conflicting IDs, bad hashes, incomplete input, authentication/revocation; shared connection with feeds.
- Actual daemon native chat, queue, a large uploaded prompt, file upload/download, settings/projects, fork/delete, cached frontend restart and model-selection intent ordering.
- Legacy file routes return 404; no legacy transcript serialization; bounded descriptors preserve large escaped Unicode error/draft content.
- Generic lost-ACK outbox recovery without reexecuting the mutation, interrupted source reservation without replay, and delayed queue edit convergence across frontend restart.
- Closed disclosure interests, viewport limits, sealed-copy guard, 100-card copy in bounded cohorts, body-cache eviction without deleting metadata, long metadata preservation and split UTF-8.
- Saturated bulk admission and saturated metadata admission leave other reserved classes usable. A paused UI still allows probes and keeps durable responses. Oversized old WebSocket frames are rejected from their header rather than buffered until heartbeat timeout.
- Existing actual-process SQLite crash tests, native tool execution/file materialization and in-place legacy schema migration.

## Remaining release gates

Do not label this branch release-ready or silently replace these gates with the passing unit/integration suite:

1. **Finish the intent/state audit:** cold first-prompt acceptance before runtime/history load; every early/delayed response versus data-convergence transition; legacy alias and multi-record local/file crash windows; generic uncertain-action inspection/reconciliation UX; interrupted tool/private-context behavior. No uncertain paid/external work may be automatically replayed.
2. **Complete aggregate resource/retention policies:** source/client header/tombstone growth, retained/orphan exports, lease GC maintenance, source list pagination, broader byte-based admission. Test disk-full/ENOSPC, upload/export deletion races, channel closure, many peers and extreme live fanout. Incremental cache snapshot/render/copy work remains to be proven, not just wire/storage append behavior.
3. **Define restore/migration operations:** restoring an old source backup with the old lineage is unsupported. An authorized offline restore must rotate lineage before serving, preserve/reconcile ownership records and invalidate old cache namespaces. There is no tested admin rotation/rollback tool yet. Expand native source/cache migration tests before deployment/downgrade decisions.
4. **Measure the actual new path:** production/test byte, metadata, resume, connection, queue, hash and cancellation counters; native delayed/lossy/low-bandwidth scenarios with uploads, unread files, large code, many disclosures and both clients. Also audit client send waits, reconnect backoff stability/jitter and dual-stack/direct-address reachability. The removed legacy transfer fixture/loss test is not evidence for native weak-link performance. Logical-plane tests do not certify shared-link congestion behavior or a hard end-to-end latency bound.
5. **Manual/mobile checks and deployment approval:** no real Pixel/Shlap or full UI certification has been performed. Re-run checks/nextest after further changes. Beta must remain stopped and stable untouched until explicitly authorized.
