# Tau2 protocol audit and proposed contract

2026-09-25. Audited source: **`35b3b24`**, protocol **15**.
This includes the explicitly requested merge of `fix/tau2-immediate-intent`
(`5c5ef4d`) into `tau2` (`6d30668`), not the older deployed beta binary.

**Status: design proposal, not an implemented transport rewrite.** The merge is
implemented and tested. Additional changes accompanying this document are
characterization tests only. Beta is stopped; stable Tau was not restarted.

## Decision in brief

Use **two application connections per client/daemon pair**:

1. **One authenticated, long-lived control WebSocket.** Small intents, durable
   acceptance/rejection receipts, operation outcomes, status/head notices,
   subscriptions, blob authorization, and heartbeat. No transcript bodies,
   histories, files, giant queues, or whole topic prompt lists.
2. **One shared, reconnectable Iroh/QUIC bulk connection, opened on demand.**
   Bounded concurrent streams fetch missing transcript blocks and file ranges,
   in either direction. One endpoint per client process/account context, not one
   endpoint/thread/runtime per file. Multiple chats and files share it.

Do **not** add a heartbeat socket, worker socket, or per-chat connection.
Separate service classes and backpressure, not one socket per message type.

The semantic foundation is more important than that connection count:

- Persist the user's intent locally before any network effect.
- A daemon acceptance receipt means the complete operation and its required
  inputs are durably owned by the daemon, **not that an LLM has started**.
- Retry the same immutable operation ID safely after a lost acknowledgement.
- Persist verified transcript data locally; request only missing blocks/ranges.
- Worker activity, connection health, delivery, and sync freshness are separate
  facts. Do not encode their Cartesian product as a giant delivery state machine.

## 1. Scope and evidence

Reviewed the actual native Rust paths: protocol commands/events, daemon SQLite
transactions and execution, frontend persistence/controller/feed, WebSocket
reader/writer/heartbeat, HTTP uploads/offers, and `tau-transfer`'s Iroh provider
and download lifecycle. Pi is an import source, not a prerequisite for accepting
current Tau2 commands. This is not a general tool sandbox/security audit.

### Deployment observations versus source findings

Earlier read-only measurements, before beta was stopped:

- Host storage/CPU did not suggest saturation: an uncached 16 MiB read was about
  253 MB/s, Btrfs error counters were zero, and CPU was mostly idle. Independent
  Internet transfers were materially faster than Tau file transfers.
- Shlap's tailnet path was direct UDP around 335 ms with intermittent timeouts.
  Pixel showed roughly 340–935 ms and intermittent relay use. Neither is LAN-like.
- The old deployed beta recorded 104 WebSocket resets in 90 minutes, with a median
  interval about 41.7 seconds. **Those were not measurements of the merged
  2-second-probe/5-second-timeout implementation.**
- A 45-second WebSocket classification window contained **126 transcript-update
  frames, 147,261 payload bytes**. List/session/project/response traffic totalled
  6,666 bytes, plus tiny ping/pong frames. About **96% of outgoing payload in that
  window was transcript updates**, approximately 3.3 KB/s of transcript payload.
  That window did not contain a reconnect snapshot; earlier larger traffic bursts
  must not be relabelled as measured snapshots without frame evidence.
- Another short sample showed substantial beta traffic and TCP retransmission;
  a completed stable file transfer averaged approximately 18.6 KB/s. These
  observations establish a slow, shared path, **not that beta alone caused the
  entire file-throughput collapse**.

The heartbeat payload itself is not the bandwidth flood. The old client also
used list requests for heartbeat, but even those were a small share in the
classified window. A reconnect/replay feedback loop is a plausible mechanism;
its contribution to that deployment has not been quantified.

### Reproduced against the merged source

New loopback/serialization characterization tests require no deployed daemon,
phone, GPU, or live model provider:

| Experiment | Result |
| --- | --- |
| 128 growing updates, final content 32,768 bytes, through `SessionContent::live` | Text: **52,537** JSON bytes; thinking: **52,541**; tool arguments: **2,171,156**. |
| Single 512 KiB live tool event, nominal 256 KiB page budget | Snapshot serialized to **524,931** bytes. The page budget is not a hard frame limit. |
| Responsive mock WebSocket behind a nominal 25 KiB/s downstream proxy; 512 KiB transcript frame ahead of pong | Server receives and answers the ping, but the current client reports **Ping timed out** approximately 7 seconds after connection. |

The streaming experiment measures serialized application bytes, not IP/QUIC
wire bytes. Its exact amplification depends on update granularity. It does not
claim every production stream has that size or that the measured deployment's
updates were all tool calls. The slow-frame test proves the head-of-line failure
mode, not the entire historical multi-reconnect feedback loop.

Tests: `daemon/src/protocol_audit_test.rs` and
`frontend/tests/connection_probe.rs::audit_slow_transcript_frame_blocks_a_responsive_peers_pong`.
The tests explicitly characterize undesirable current behavior; replace their
expectations when the new contract is implemented.

Validation after adding them:

```
/usr/local/bin/cargo check --locked --workspace --all-targets
/usr/local/bin/cargo nextest run --locked --workspace
```

**102 passed, 0 skipped**. The merge alone passed 99 tests. The three audit tests
can be selected with `-E 'test(audit_)' --success-output immediate`.

## 2. What the current connections actually carry

| Path | Current contents/lifecycle |
| --- | --- |
| Authenticated WebSocket over HTTP/Tailscale | All commands, responses, session/topic lists, worker state, full transcript snapshots/pages, live deltas/replacements, queue bodies, receipts and heartbeat. One ordered TCP byte stream. |
| Authenticated HTTP POST | Upload entire files, up to 50 MB. Client and server buffer the body. No resumable upload ID/ranges. |
| Authenticated HTTP GET | Obtain an attachment offer for a supplied Iroh node ID. Legacy direct HTTP download is also still available. |
| Iroh 0.35 / iroh-blobs 0.35 | Download attachments only. New key/endpoint, OS thread, runtime and partial store per download. No transcript sync or uploads. Normally a separate QUIC connection per active file/attempt. |

The frontend limits active downloads to two; the provider admits sixteen
connections and the QUIC configuration allows four bidirectional streams per
connection. Relays are disabled in this Iroh configuration; it resolves IPv4
addresses and relies on the tailnet route. Tailscale may itself use a relay.

Sources: `frontend/src/transport.rs:124–188, 222–343`;
`daemon/src/server.rs:48–65, 488–544`; `transfer/src/lib.rs:91–147, 200–205, 237–367`.

## 3. Findings, in priority order

### A. Control and bulk share failure, buffering and latency domains

**High impact, reproduced.** The server sends whole JSON text messages through a
single 512-message FIFO. It is not byte-bounded or priority-scheduled. Snapshots,
updates and command responses all use it. The server also enqueues its own ping
there. A WebSocket control frame can interleave between fragmented messages,
but cannot jump ahead of bytes of an already transmitted unfragmented frame.
TCP loss also orders all later bytes behind the loss.

On the client, incoming decoded messages await space in a 256-event UI channel.
The same task handles reads, commands and heartbeat; a socket send awaits up to
15 seconds inside its select branch, and a full UI channel can stall that task.
Thus the nominal 5-second heartbeat deadline is not independent of all blocking
work, despite being scheduled independently of the 2-second cadence.

Reopening connections does not fix this. It can create more snapshots behind
which the next heartbeat waits. Backoff resets as soon as Hello succeeds, even
if the connection immediately becomes unhealthy, and has no jitter or stability
threshold. Making pings more frequent cannot repair head-of-line blocking.

Sources: `daemon/src/server.rs:116–169, 198–231, 634+`;
`frontend/src/transport.rs:68–85, 251–272, 315–340, 381–386`;
`frontend/src/connection.rs:12–13`.

### B. There is no retained missing-block transcript synchronization

**High impact, source-confirmed.** `OpenSession` carries a chat ID and pending
operation IDs, but no cached head, cursor, hashes or ranges. Opening always
builds a snapshot. A gap leads to another OpenSession, not a request for the
missing suffix. The frontend explicitly does **not persist remote transcripts**.

On reconnect the controller opens every chat in its in-memory map. Initial list
processing warms up to eight chats; the map can grow as chats are visited and
there is no normal unsubscribe/eviction protocol. This is not merely refetching
the selected chat. An unchanged restart cannot avoid resending known content.

Transcript generation is a new UUID and sequence starts at zero when a native
runtime loads. It is not a durable synchronization cursor. Runtime retirement
emits `ResyncRequired`; an attached frontend immediately reopens the chat,
reloading the runtime and taking another snapshot. Idle cache eviction should
not invalidate immutable history or force clients to wake it back up.

There is careful gap validation and some preservation of overlapping older
in-memory history today. That is useful correctness work, but not persisted,
range-addressable synchronization.

Sources: `protocol/src/lib.rs:65–74`; `daemon/src/manager.rs:167–184, 308–318,
358–395`; `daemon/src/transcript.rs:188–217`; `frontend/src/feed.rs:1–2, 26–85,
95–132`; `frontend/src/controller.rs:707–718, 857–919, 966–975`.

### C. Some streaming is linear; tool-argument streaming is quadratic

**High impact, reproduced.** The implementation already sends append deltas for
one growing text/thinking block. It would be incorrect to say every token
resends the whole answer.

The optimization excludes tool arguments, and falls back to whole events when
multiple blocks change together. For a single tool body delivered in n equal
increments, repeatedly sending the entire prefix sends n(n+1)/2 increments.
Our 32 KiB fixture sends roughly **66 times its final content size**, or about
**41 times the equivalent text stream's JSON traffic**. Completion also sends a
saved representation of live content again.

Large tools, generated code and multi-block output can therefore dominate
bandwidth without any heartbeat bug. A smaller snapshot policy alone would not
fix this live traffic.

Sources: `daemon/src/agent/mod.rs:68–81, 197–213, 238+`;
`daemon/src/transcript.rs:167–180`; audit serialization test.

### D. Nominal limits are not hard wire or memory bounds

A page aims for 50 events/256 KiB, but always admits its first event even when
larger. Snapshots add all retained live events and the full queue. Queue text can
be 4 MiB before JSON overhead. Topic lists include all prompt bodies: 128 topics
with up to 64 Ki characters each can exceed the client's 16 MiB message check
with multibyte text. Session lists are not paginated. An oversized aggregate can
therefore be rejected on every reconnect, not merely take too long to download.
Character-count limits and encoded-byte limits also need a consistent contract.

A 512-message buffer is not a meaningful byte bound in this environment.
Broadcast buffers hold 2,048 messages; slow consumers eventually request resync.
The server spawns ordinary requests without a request concurrency budget or
socket-scoped task collection. Accepted work may legitimately survive a socket,
but unbounded detached request tasks and retained outbound messages should not.

All daemon DB operations, including context reads, paging and branching, share
one connection/gate. Control latency therefore also depends on long data work
holding that gate. Transport isolation alone will not remove that coupling.

Sources: `daemon/src/transcript.rs:197–217`; `daemon/src/state.rs:83–86,
193–194, 252–280`; `daemon/src/manager.rs:19`; `daemon/src/projects.rs:8–28`;
`daemon/src/server.rs:118, 231, 430–435`; `frontend/src/transport.rs:318`.

### E. Durable acceptance is real for ordinary prompts, but not a universal contract

**Keep the good foundation.** Ordinary prompt acceptance commits queue and
receipt together in SQLite WAL/FULL before `start_run` and the response. Queue
controls have durable receipts and compare the original payload before testing
stale generation. The recent queued-edit fix publishes the finished receipt ID
immediately after the same DB commit, not at the next model output. Opening a
chat loads a lightweight native runtime; it does not need to launch Pi or make
an LLM call.

The merge also adds durable client-named creation receipts and local offline
new-chat/send intent. During integration, creation receipts were made to bind
both keep-chat and topic, including aliases of reused starters. Cross-topic ID
reuse is rejected rather than silently retargeting the operation.

However:

- Prompt/queue acceptance still loads transcript/runtime state and takes runtime
  locks. Creation returns after runtime loading and other work following its
  receipt transaction. The acceptance path can be made much smaller.
- `/compact` first writes an unfinished receipt, then waits for model work before
  returning its response. The operation guard remains held. Other prompt/queue
  operations can wait behind it. This violates a universal "accepted immediately,
  execution later" interpretation.
- `delivered()` tests receipt existence only. Snapshot reconciliation removes a
  pending item even if its builtin receipt is unfinished or contains an error.
  Receipt existence is evidence of registration, not successful completion.
- Model/default-setting changes span settings-file persistence, transcript
  persistence and receipt finalization. A crash can split those effects.
- Abort cancels the in-memory task **before** committing its receipt/queue.
  A subsequent DB failure does not mean that no effect occurred. Deletion also
  intentionally cancels work before its database transaction.
- Generic failures are flattened to `ok=false, uncertain=false`; operations that
  commit and subsequently fail to publish/clean up cannot truthfully use that as
  proof of non-acceptance.
- Rename/delete/fork/clone/topics/settings are not all covered by an operation
  journal. A stable request ID by itself does not make a command idempotent.
  Revision checks avoid stale writes, but do not return the original receipt
  after a successful write's acknowledgement is lost.
- Receipt rows cascade-delete with the chat. A late retry after deletion lacks a
  durable tombstone/receipt proving what happened.

Sources: `daemon/src/manager.rs:137–165, 186–231, 233–299`;
`daemon/src/agent/mod.rs:35–57`; `daemon/src/state.rs:104–155, 168–228`;
`daemon/src/commands.rs:28–71`; `daemon/src/schema.sql`; `daemon/src/protocol.rs`.

### F. The frontend still has "unknown forever" cases, not a durable outbox

The merge persists send/new-chat work before sending and retries unsent waiting
work and creation IDs. That fixes an important UI ordering problem.

But Sending/Preparing becomes **Unconfirmed — not resent** after disconnect or
restart. Snapshot receipts can resolve a request that was accepted; they cannot
deliver a request the daemon never received. Restoring it into a new prompt with
a new ID can duplicate the original if it later proves accepted. This is
conservative legacy behavior, not the desired end-state semantics.

The generic request map is memory-only and cleared on disconnect; model tile
selection also uses that path. Queue controls require a connection even though
they are locally journalled once issued. WaitingForModel is rejected after
restart rather than expressed as an operation dependency.

Creation still has a provisional-to-existing-starter alias. `merge_chat` copies
files, saves the target, deletes the source and removes old files in separate
steps; the account mapping is persisted later. Copied files are not explicitly
fsynced there. There are crash windows between these steps, notwithstanding the
passing normal restart/reconciliation tests. In the target protocol, avoid
rewriting a client-assigned chat ID at all; otherwise the entire alias migration
needs a recoverable transaction and a file durability protocol.

Also, each accepted live transcript update rewrites local chat state with
SQLite FULL synchronization even when no local pending state changed. The remote
transcript itself is not being persisted by those writes. This creates avoidable
UI/disk work and can exacerbate event-channel backpressure.

Sources: `frontend/src/store.rs:82–102, 154–161, 178–180, 207–224, 280–322`;
`frontend/src/controller.rs:274–356, 359–425, 462–483, 493–583, 707–742,
887–911`.

### G. Iroh provides useful integrity/resume, but the lifecycle is per-file

Keep verified range download, partial-store restart support, final size/hash
verification and atomic final file rename. Replacing those with naive HTTP
whole-file retries would be a regression.

Necessary changes before sharing the bulk connection:

- Grants are currently **NodeId → one Source**, captured when the connection is
  admitted. A second offer replaces that map entry, and an existing connection
  still has its original Source. A shared endpoint alone would break concurrent
  authorization. Use a dynamic per-principal/node set of permitted hashes/ranges
  and check authorization per request.
- Offers expire after one hour. Three transport retries do not renew the offer;
  resumed work needs authorization refresh independent of its retained bytes.
- Every offer hashes the whole source again. It holds a file descriptor, not an
  immutable content copy; later modification can cause integrity failure. Store
  immutable blob content/outboards once, then authorize access to that content.
- Every download owns a new runtime/endpoint/thread. Reuse one managed bulk
  service and its connection; do not equate a stream with a new endpoint.
- Uploads buffer the whole file on both sides, have a 90-second client request
  deadline, no range resume, and no upload deduplication. At 20 KB/s a 50 MB file
  needs about 42 minutes; that API cannot serve the slow-path requirement.
- `network_bytes` is updated from successful final attempt statistics only;
  failed-attempt traffic is omitted. Progress offsets/verified bytes are not
  actual instantaneous wire throughput. Measure and label them separately.

Sources: `transfer/src/lib.rs:84–183, 237–367, 412–429`;
`frontend/src/transport.rs:124–129, 149–188, 284–310`;
`daemon/src/server.rs:444–470`.

## 4. Deriving the replacement from the requirements

### 4.1 Two service classes, not five unrelated connections

Small causal messages must remain responsive while arbitrarily large content is
incomplete. Bulk content must be range-addressable, verified, cancellable and
restartable. These are different backpressure/lifetime contracts. Heartbeat,
receipts and worker state are all small control messages; none requires its own
socket. Files and transcript bodies are both verified content, with different
priorities inside the same bulk scheduler.

| Candidate | Assessment |
| --- | --- |
| Current single WebSocket for everything | Wrong isolation: ordered bulk bytes block control; recreates content after reconnect. |
| One custom multiplexed QUIC connection for everything | Technically sound **if** control stream priority, connection/stream flow-control reservations, auth and bounded queues are designed correctly. QUIC does not give these application guarantees automatically. |
| **Control WebSocket + shared Iroh bulk connection** | **Recommended.** Clear ownership and independent failure/backpressure, existing native transport primitives, and no need to invent an all-in-one protocol to obtain isolation. |
| Separate sockets per heartbeat/status/chat/file | No corresponding independent requirement; more lifecycles/retries, potentially more bandwidth competition. |

This is a selected design, not a claim that two is a mathematical lower bound.
One well-designed QUIC protocol could satisfy the contract later. Today's
`iroh-blobs` ALPN is a blob protocol: arbitrary control frames cannot simply be
mixed into it. Keeping WebSocket control avoids building that new multiplexed
ALPN. It is not a reason to retain bulk WebSocket traffic indefinitely.

Two connections still share the WAN, device and potentially a Tailscale relay.
They **do not reserve bandwidth**. Bulk pacing, bounded queued bytes and measured
control latency are required even with separate connections. A shared outer
relay/TCP path can still introduce common delays.

### 4.2 A real operation journal

Use a single acceptance path for all mutating operations:

```
UI action
  -> local transaction: immutable operation ID + payload/hash + dependencies
     + composer/selection changes
  -> immediate local display
  -> satisfy input-blob dependencies; submit operation with the SAME ID
  -> daemon transaction: deduplicate + validate preconditions + persist effect/
     work item + receipt
  -> return durable Accepted or terminal Rejected receipt
  -> executor progresses independently; outcome/status is separately observable
```

Required invariants:

1. **No send before local commit.** Local save failure leaves the user's draft
   intact and sends nothing. Attachment files and their directory entries must
   be durable before a saved outbox record claims to own them.
2. **Immutable identity.** Journal key is scoped to authenticated principal and
   operation ID. It binds canonical payload/schema version, target entity,
   expected revision, dependency IDs, body/attachment hashes. The same key with
   different content is a conflict, not another action.
3. **Transactional acceptance.** A successful receipt is emitted only after the
   DB commit that records the complete accepted action. Large input blobs must
   already be verified/durable and pinned to that action. Uploading remains a
   *local pending reason*, not a falsely successful daemon acceptance.
4. **Acceptance is not execution.** `/compact`, stop-at-boundary, provider startup
   and tool execution must not hold the acceptance response hostage. An accepted
   command later has a durable result, including failure/interruption.
5. **Safe retries.** Lost acknowledgement: resend/query the same ID with jittered
   backoff; return the same stored receipt. Timeouts/disconnects do not invent a
   new operation ID. Transient unavailability before commit is retryable; an
   invalid/precondition-rejected operation has a stable terminal rejection.
6. **Independent reconciliation.** `GetReceipts(ids)` and a bounded receipt
   journal/cursor work without loading any transcript. Rejection and execution
   errors are returned as structured outcomes, not erased by an existence list.
7. **Deletion and retention.** Entity tombstones and receipt retention prevent old
   retries from resurrecting deleted work. Define expiry explicitly: an expired
   or wrong-lineage ID must never silently become a new operation. Receipt GC
   requires a client watermark/retention contract or conservative retention.
8. **Ordering is explicit.** New-chat → model selection → send are dependencies or
   revision preconditions, not accidents of TCP order or independent spawned
   tasks. Serialize mutations per entity without holding the ingress lock while
   an LLM or external tool runs.
9. **Stable IDs.** Prefer creating exactly the client-assigned chat UUID. Do not
   silently coalesce it into a different starter. Reusing an existing empty tile
   is an explicit UI choice, not a protocol identity rewrite.
10. **No exactly-once claim for external effects.** At-least-once transmission
    plus at-most-once durable acceptance is achievable. A process can crash after
    a shell/payment/model side effect but before recording its result. Resume
    automatically only when the downstream operation is demonstrably idempotent;
    otherwise expose interrupted/needs-review and require explicit action.

The existing safe pause-on-recovery behavior for native work is valuable. Making
network retry safe must not turn into blindly rerunning tools after a crash.

### 4.3 Missing-block transcript synchronization

Do not send the whole conversation, or the whole recent snapshot, as the
recovery primitive.

- Persist a **database lineage ID** and per-chat durable content/head revision.
  A normal daemon restart or runtime eviction does not change history identity.
  Restoring/replacing the database establishes a new lineage/recovery boundary;
  clients must not replay old uncertain operations blindly against it.
- Keep a versioned display projection that excludes private provider payloads,
  as the current projection does. Forks have their own manifests referencing
  reusable immutable content, not duplicated downloads of every event.
- Partition display metadata and bodies into immutable content-addressed blocks.
  Manifests map stable event IDs/order/revisions to block hashes and lengths.
  Bound **encoded bytes**, splitting a giant event across blocks. A Merkle tree
  is optional; a paged/versioned block manifest and durable append sequence are
  sufficient. Never send an unbounded manifest as a control message either.
- Send small coalescible `HeadChanged(chat, revision, manifest_ref)` notices on
  control. The client requests the manifest suffix/pages and only missing hashes
  or verified ranges over bulk. Receipt/queue metadata is a separate versioned
  view; queue text bodies are content references, not repeated multi-MiB lists.
- Persist verified blocks before transactionally advancing the local manifest/
  contiguous cursor. After a crash it is safe to discover extra durable blocks;
  it is not safe to advertise a cursor whose bytes were never saved.
- Maintain a byte-bounded cache with quota/GC. Opening shows retained content
  immediately, with freshness information. Do not eagerly refill every visited
  chat on reconnect. Subscribe to selected/pinned/running interests explicitly,
  unsubscribe when no longer needed, and fetch old history on viewport demand.
- Live content uses revisioned append chunks with explicit offsets/base versions,
  for **text, thinking and tool arguments**. Seal chunks periodically; finalization
  changes metadata/references instead of transmitting the accumulated body again.
  Do not rehash and redownload the whole growing response after every token.
- A missing live chunk requests that range or a bounded current checkpoint.
  Duplicates, reordered responses and obsolete generations cannot corrupt text.
  Transient uncommitted output is marked provisional; after a daemon crash a
  durable interruption/seal tells clients whether that tail survived. Never
  advertise non-durable bytes as a durable resumable head.
- If a cursor is outside retention, return a bounded manifest/checkpoint reset.
  Retain verified blocks that still match; do not purge/redownload everything.

An unchanged reconnect exchanges receipts, subscriptions and heads. It transfers
**zero transcript body bytes**. A changed chat transfers only new/missing content
plus bounded metadata/proof overhead.

### 4.4 One bulk service for both directions

Maintain a reusable endpoint/connection and shared content store. Bounded
bidirectional streams allow client pulls of transcripts/files and server pulls
of authorized client upload blobs. Serving and requesting over the same QUIC
connection requires explicit bidirectional handler/lifetime support; today's
per-file helper is not that service yet.

Bootstrap the peer identity over authenticated control, bind capabilities to the
principal/node/content/direction/expiry, and check every requested hash/range.
Knowing a hash is not authorization. Renew a grant without discarding verified
partial data; a disconnected control channel need not cancel a still-valid
transfer. Revocation/expiry has explicit behavior.

Store immutable content and cached outboards once. Pin referenced blobs until
receipts/history and transfer leases permit collection. Client cache and account
identity must remain correctly separated, including when credentials rotate.

Prioritize currently viewed transcript chunks over old history/prefetch and
large files. Use bounded per-stream and aggregate byte credits and a small
concurrency limit, not an unbounded pipeline of bytes already queued to the
socket. QUIC stream independence does not reserve congestion or connection
flow-control capacity for the important stream. The stock provider must not be
assumed to implement this policy for us.

Do not reconnect control because one file stalls. Retry only the relevant bulk
stream/ranges. Conversely, control reconnect must not spawn a new download of
already verified data.

### 4.5 Bounded control, heartbeat, and minimal states

Start with a small hard control-frame budget, e.g. **4 KiB**, and an aggregate
byte budget. These are design starting points to benchmark, not existing limits.
Oversized prompt/topic/settings bodies become verified blob references; list
metadata and receipt batches are bounded/paged. No exception for "just one large
event". Keep JSON for small metadata unless profiling establishes a reason to
change encoding; serialization format is not the primary defect here.

- Prioritize heartbeat and receipts; coalesce superseded status/head notices.
  Keep control read/write/heartbeat scheduling independent of UI, file I/O,
  transcript decoding and model work. Bound request execution concurrency.
- Do not hold the control acceptance DB gate during full history/context scans
  or blob hashing. Use bounded/paged reads and an appropriate read connection/
  snapshot where necessary; retain one transactional authority for mutations.
- Hello establishes authentication, negotiated capabilities, daemon boot identity
  and DB lineage. It does not promise that a provider is authenticated, an LLM is
  loaded, or every future disk write will succeed.
- Ping/pong measures this control path's responsiveness. Use one client-initiated
  probe policy with a server idle limit, rather than unrelated competing probe
  loops. It must not call ListSessions or inspect every runtime. Actual operation
  acceptance latency is measured separately; an automatic pong is not proof of
  DB progress.
- Keep prompt UI feedback and health display fast. A 2-second probe can remain;
  distinguish a stale/late observation from an immediate destructive reset.
  Choose a bounded timeout tolerant of measured RTT/loss and mobile resume;
  do not present one exact timeout as justified by these loopback tests. Use
  jittered reconnect backoff, reset only after a stable interval, and reconcile
  journals/heads rather than recreating bulk work after reconnect.

Minimal orthogonal state model:

| Dimension | Meaning |
| --- | --- |
| Connection | disconnected/connecting/ready/blocked; freshness/RTT are measurements. No model-readiness gate. |
| Operation delivery | local pending → daemon accepted **or** terminal rejected. Sending/retry/upload/dependency are reasons/progress on pending, not additional authoritative outcomes. |
| Execution | idle/running/paused/failed, with run ID and reason; result/interruption belongs to the accepted operation/run. Runtime cache residency is not a delivery state. |
| Deferred control | receipt says accepted; a later outcome says applied/cancelled/failed at the named boundary. Do not conflate those two acknowledgements. |
| Sync/transfer | local versus remote head and verified ranges; fetching/stale/error is independent of whether the daemon accepted a message. |

This retains distinctions that affect correctness without exposing one combined
"waiting for chat/connection/model/worker/transcript" state for every permutation.

## 5. Implementation order and release gates

No broad redesign is implemented by this audit. Recommended sequence:

1. **Operation semantics first:** schema/journal/tombstones, typed acceptance and
   result, exact-ID retries for every mutation, receipt-only reconciliation,
   stable chat IDs and transactional local outbox. Keep existing native queue
   commit guarantees and the queued-edit receipt fix.
2. **Durable content identity/cache:** bounded manifest/block representation,
   cursor lineage, resumable local cache, revisioned live chunks and explicit
   subscriptions. Acceptance must remain independent of this cache's progress.
3. **Shared bulk service:** multi-hash capabilities, reusable connection, uploads,
   immutable stores/outboards, range scheduling and authorization renewal.
4. **Move transcript bodies to bulk and enforce control budgets.** Remove
   snapshot-on-reconnect, per-file endpoints and list-as-heartbeat assumptions.
   Use one explicit version/capability transition rather than an indefinite mesh
   of new and legacy sync mechanisms.
5. **Measure impaired-path behavior before deployment.** Preserve the old binary
   for deliberate rollback, but migrate/version local and daemon data explicitly;
   an old protocol client must fail clearly rather than misinterpret new receipts.

Required tests beyond today's merge/characterization coverage:

- Lose every possible acknowledgement around local commit, daemon commit and
  receipt persistence; restart either side; retry the same ID. One accepted
  operation, same receipt/outcome, no duplicate queue item or new chat.
- ID reuse with changed payload, stale queue/topic revision, deletion followed by
  a delayed retry, receipt expiry and restored database lineage.
- Simultaneous clients and same-chat mutations; delayed dependencies, model
  selection failure, interrupted compaction, stop-at-boundary acceptance versus
  application, DB failure before/after an external cancellation effect.
- Crash at every provisional/local-file migration/outbox checkpoint. Disk full,
  file-copy failure, durable blob with missing manifest and manifest with missing
  blob. No silently lost authored work or falsely advanced cursor.
- Unchanged reconnect/restart: **zero transcript body bytes**. One missing chunk:
  only that chunk/range plus bounded metadata. Corrupt/missing cache, a very large
  tool event, simultaneous growing blocks and runtime eviction.
- Saturate uploads/downloads/history while submitting a command and probing.
  Check acceptance/probe latency independently of bulk completion; enforce byte,
  task and memory limits. A paused UI consumer must not prevent heartbeat reads.
- Slow/lossy test matrix: 20–100 KB/s, 300–1000 ms RTT, random/burst loss,
  disconnects, address changes, relay path, mobile suspend/resume. No
  self-amplifying snapshot loop and no new connection per file retry.
- Concurrent multi-hash grants on one endpoint, revocation, expiry/renewal,
  daemon/client restart, resumed uploads, immutable-source validation and cached
  file corruption. Retained verified ranges must not be counted as new traffic.

Instrumentation should record message class and encoded bytes, queued bytes,
accepted-to-receipt latency, operation age, head/range lag, useful verified bytes,
actual attempt traffic, reconnect reason and connection/stream counts. No prompt
text, bearer tokens, provider secrets or full content payloads are needed.

## 6. Operational work completed

- Stopped `tau2-beta.service`; it remains inactive with MainPID 0.
- Left stable `tau.service` running at its original PID 474496.
- Applied worktree cleanup against `tau2-integration`: seven merged worktrees
  removed initially, then the now-merged context/intent worktrees removed after
  the merge (**nine total**, branches retained). The dirty
  `tau2-remove-pause` worktree and protected checkouts were left alone.
- Merged and pushed `35b3b24` to `origin/tau2`. Topic/intent conflict resolutions
  and normal restart cases are covered; the larger durability/network gaps above
  remain explicitly proposed work, not claims of fixes already shipped.
