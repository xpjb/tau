# Shared replicated state for Tau 2

September 28, 2026. Design evaluation against `origin/tau2` at `40a3698`, fetched
again for this pass. This extends `tau2-client-state-proposal.md`: the replicated
part of that state should have a common implementation beneath both daemon and
client. No application changes or protocol migration have been made.

## Selected first improvement: one typed tool-transcript projection

**This is the only work item proposed for implementation now.** The broader
architecture below is background, not an additional work queue. Prefer this slice
to extracting the receive pump: it removes repeated domain interpretation, not
just protocol coordination, while avoiding durable-state migrations.

### Deliverable

A shared typed contract for native tool metadata/relationships, and one derived
`ToolProjection` carrying native identity, execution state, input/result body
references and child-discovery completeness. Keep body bytes in the existing cache.
The projection is a bounded local view, not a new wire object embedding descendants.

- Daemon tool publication uses the shared metadata/relationship helpers while
  preserving the existing block encoding and IDs.
- The native client projection builds/updates tool meaning from block headers and
  parent links. Display, tool-copy membership and tool content interests consume
  that meaning rather than recovering it independently from flat events or labels.
- Rendered tool sections carry explicit source/body references. Display and
  persisted expansion keys remain presentation identifiers; never decode them
  to discover a block ID.

### Concrete replacement and scope

Replace tool pairing/section reconstruction for the native path, the matching
tool-child interpretation in fetch/copy planning, and the tool-root rescan and
reverse display-key parser in `App::chat`. Delete superseded code as consumers
move; do not retain a second live native interpretation behind the new types.
Demo/test event inputs may adapt into the same projection at their boundary.

The change is limited to native tool groups: shared metadata helpers, daemon tool
publication, frontend native projection/details/interest/copy consumers, and
focused tests. Ordinary message rendering, outer Details grouping and authored
message/queue reconciliation remain unchanged. Keep current overflow metadata,
source/version checks, copy completeness checks and content budgets.

No crate merger/rename, general replica framework, receiver extraction, catalogue
migration, operation-store migration, wire/schema change, scroll-restoration
redesign or retained-UI work is included.

### Acceptance and stop condition

- Rendering, copying and body-interest selection use the same typed tool
  relationships. Native block identity comes from references, not display strings
  or provider tool-call IDs.
- Cover missing/partial children, repeated provider call IDs, orphan results,
  multiple results, failure/interruption and overflow metadata without changing
  their existing presentation or completeness behavior.
- A tool body visible while its heading is off-screen still requests its correct
  native content. Collapsed tools do not start fetching unrequested bodies.
- Preserve persisted disclosure keys and existing body/cache bounds. Existing
  tool-copy, native sync and two-client weak-link tests must continue to pass.
- Validate with the managed compiler checks and relevant nextest runs. Report
  removed/replaced production code across shared, daemon and frontend code; moving
  code between directories is not itself the success criterion.

Stop when this one tool path has a single semantic owner and the obsolete native
interpretations are removed. Do not extend it to all transcript or client state.
This section is a proposal only; no implementation or Rust tests were run.

## Background: broader architectural direction (deferred)

**Build upward from the replication code already shared, with a common typed
model and pure state transitions.** Put client-specific ownership around that
model, rather than making the model a frontend-only abstraction.

The main opportunity is to stop translating between different understandings of
the same message, tool, queue entry or operation. A second opportunity is to bring
other replicated collections under the existing feed machinery instead of giving
each collection its own paging, resync and stale-update rules.

A universal peer-to-peer database is not a prerequisite. The useful target is
reusable publisher/receiver roles and explicit authority for each kind of record.
Both roles can be hosted by the same process. Their authentication, admission and
execution policies remain explicit.

## Derivation from the actual requirements

1. **Several machines need the same facts.** A chat, message and operation must
   retain identity across delivery, reconnection and another client's observation.
   Their public representation and revision rules should therefore be shared.
2. **Machines learn facts at different times.** The model must represent a known
   revision, partial membership and missing bodies. Those are synchronization
   states, not strings that a renderer has to interpret.
3. **Local interaction should not wait for a round trip.** Keep confirmed state
   plus durable pending intents. Derive a tentative view from the two, then
   reconcile it when outcomes and corresponding state arrive.
4. **Some operations have irreversible or paid effects.** Replaying state must
   never execute an operation. Receipt reservation, effect execution and uncertain
   outcomes need an explicit lifecycle, independent of replayable state updates.
5. **Peers need different subsets.** Replication should operate on authorized
   interests and bounded bodies, not serialize the entire daemon or client heap.

Together these imply a shared replicated model, a pending-intent overlay and a
separate effect executor. They do not imply one universal conflict policy: a
server-owned queue, immutable operation input and device-local draft have different
writers. Declaring those writers makes the common machinery simpler.

## What the Quake III comparison actually contributes

I checked id Software's original source at
`dbe4ddb10315479fc00086f08e25d968b4b43c49`:

| Mechanism in that source | Useful Tau interpretation |
| --- | --- |
| `code/qcommon/msg.c`: entity delta readers and writers use `entityStateFields` | Define the replicated record contract once, including its interpretation, not independently at each endpoint. |
| `code/server/sv_snapshot.c`: selects a previous client snapshot for delta encoding; falls back when that baseline is unavailable | Track what the receiver can actually reconstruct; support an explicit reset when its baseline is no longer retained. |
| `code/server/sv_snapshot.c`: builds visibility-filtered entity sets | Keep interest selection separate from the state codec; each client can subscribe to different content. |
| `code/cgame/cg_predict.c`: starts from snapshot player state and runs later commands through `Pmove`; server gameplay also calls `Pmove` in `code/game/g_active.c` | Share safe state-transition rules between authoritative handling and local prediction, then reconcile against confirmed state. |

`Pmove` is implemented in `code/game/bg_pmove.c`. This is a substantive example of
shared behavior, not merely matching packet definitions.

For Tau I would keep these choices:

- Durable feed checkpoints and verified content offsets, rather than a packet
  acknowledgement being sufficient evidence of installed state.
- Pending-message, rename and queue-edit prediction; no prediction or replay of
  agent/tool execution. A reconnect can recompute a tentative view, not rerun a
  shell command or a paid request.
- Coalesced latest metadata and append-only body suffixes where applicable;
  preserve authored history and operation ownership even when network updates
  are coalesced.
- Existing bounded QUIC streams, scheduling and flow control. This investigation
  provides no reason to replace the transport, add a game tick or pursue bit-level
  entity compression.

These are proposed Tau choices, not claims about Quake's persistence behavior.

## What is already common, and where commonality stops

Both `daemon/Cargo.toml` and `frontend/Cargo.toml` depend on `tau-protocol`,
`tau-blocks` and `tau-transfer`. There is substantial shared implementation:

| Existing implementation | What it already centralizes |
| --- | --- |
| `protocol/src/blocks.rs` | Block IDs/parents, revisions, feed cursors, bounded requests and content references. |
| `blocks/src/lib.rs` | Source writes and coalesced change index; delta/reset pages; receiver installation, tombstones, version checks and verified chunk storage. |
| `transfer/src/blocks.rs` | Framing, compression/integrity checks, authenticated streams, flow control, watches and resumable uploads. Both upload and download carry the same `Data` frame form. |
| `blocks/src/uploads.rs` | Immutable upload specification, duplicate-byte checks, durable progress and final sealing. |

The weaker seams are above that layer:

- **Public semantic contracts are still manually mirrored.**
  `daemon/src/blocks.rs:12,63–88,106–123` produces input IDs, `fullEvent`,
  `inputFor`, `toolState` and queue membership. `frontend/src/blocks.rs:139–223`
  decodes those conventions into flat events and side maps; its planning and copy
  paths interpret them again. For example, the server's `input_id` helper and
  the client's `format!("{}/input", h.id)` encode the same relationship separately.
- **The client owns a reusable receive pump.**
  `frontend/src/blocks.rs:536–588` assembles pages, dispatches headers/ranges,
  waits for cache commits and handles checkpoint yields. A shared receiver could
  do this against a storage sink, leaving UI notices, interests and scheduling
  priority with the host.
- **Catalogues have another synchronization system.**
  `daemon/src/listing.rs` and `frontend/src/controller.rs:1286–1313,1436–1457`
  have paged catalogue traversal, traversal fencing, status stamps and stale-page
  handling, distinct from native feeds. They are candidates for common record
  replication, provided their bounded cold reads and non-starving traversal are
  preserved. Moving them would be an actual protocol change, not just extracting
  a helper.
- **Operation outcomes remain special control messages.**
  `daemon/src/operations.rs` reserves immutable commands and retains outcomes;
  the frontend reconciles responses, receipts and replicated items separately.
  Shared operation identities and outcome semantics would help even before any
  wire migration.

Upload publication has real additional responsibilities: quotas, authorization,
owned file export and final integrity verification (`daemon/src/uploads.rs`).
Reuse transfer mechanics without disguising those responsibilities as ordinary
cache installation.

## Revised architecture

These are ownership boundaries, not a requirement to introduce three new crates:

```text
                         shared model
       typed public records, IDs, body refs, operation identities
       committed-state application and relationship/index maintenance
       pure transformations used by eligible tentative edits
                                  |
                      shared replication machinery
       existing source journal / snapshots / deltas / content ranges
       receiver commit / resume / reset / tombstone / version handling
       bounded interests and progress, with host storage adapters
                    /                              \
              client host                       daemon host
       confirmed working set             authoritative public records
       durable pending intents           authorization / command admission
       tentative view + UI               operation ownership / execution
       local drafts / navigation         provider-private state / filesystem
```

The public model should be the same contract on both sides. The storage working
sets need not be identical: the daemon can query durable records, while a client
retains a bounded, partially hydrated view. Provider-private history and secrets
remain behind the daemon adapter; publication is an explicit public projection.

Distinguish three operations in the API:

- **Submit intent:** an immutable operation ID, target, input references and any
  required expected revision. Persist before sending. A client requests an edit;
  it does not supply an authoritative revision for the result.
- **Apply committed state:** install already-authorized, revisioned facts and
  update affected model indexes. Repetition is harmless; this path emits no
  execution request.
- **Project pending intent:** reuse safe field transformations to derive a
  tentative view from confirmed state and unresolved local intents. Incomplete
  client knowledge never turns a prediction into an authorization decision.

For example, changing a queue item's text can share its value transformation.
The daemon checks that the item is still editable at the expected revision;
only it commits the accepted result. The client can show the proposed text with
pending provenance and remove that overlay on convergence or rejection.

The existing `BlockRecord::Put/Remove`, feed cursors and content versions are a
base for this API, not structures to duplicate inside a new generic patch layer.
The first typed metadata codec can preserve their current wire representation.

## How the same mechanism serves the different directions

Consider client A sending message M with operation R, observed by client B:

```text
A: persist R and its body; show tentative M
A -> daemon: submit R; transfer only missing input bytes

daemon: authenticate and validate R; durably acquire operation ownership
        publish accepted state/outcome using the public model

daemon -> A: committed records/outcome; A reconciles R with M
 daemon -> B: the same committed record contract; B applies it without A's overlay

A or B -> daemon: interests, committed progress, or a new typed intent
```

There is no separate implementation of "update the other client". Fanout uses
that peer's interests and progress. Both clients apply the same model semantics;
only A has the local R overlay. A may reuse its authored bytes after the existing
authenticated source/length/hash checks, while B fetches absent bytes.

A receipt alone is not proof that the corresponding replicated state is installed.
Do not retire local content or a tentative handoff prematurely. Keep current
queue/history cursor barriers and source fencing explicit in the shared model.

At the code-reuse boundary, sender and receiver are roles rather than entire
applications. A daemon can host a receiver and a client can host a publisher when
a feature needs it. An unchanged forwarded fact keeps its identity and authority
revision; accepting an intent creates a distinct authoritative outcome. Merely
receiving a fact must not reissue it as a new command or assign it a fresh edit ID.
That is how a server/client/server round trip avoids an echo of new mutations.

This does not make arbitrary multi-authority forwarding work in the current
store: `cache_lineage` presently binds a cache to one source and clears it on a
source change. A future multi-source host must keep distinct namespaces/stores
and resumable progress, rather than alternating authorities through that API.
Current daemon-to-many-client synchronization does not require that extension.

## Invariants the common implementation must own

- Stable source/lineage, object, operation and content-version identities.
- An authority check before installing public state or admitting an intent;
  content hashes prove bytes match a reference, not permission to publish it.
- Per-feed membership/cursor evidence, finite history boundaries and separate
  verified body coverage. A global sequence does not mean every feed was loaded.
- Durable commits before advertising resumable progress. Transport credit may
  bound in-flight frames, but is not itself a persisted state acknowledgement.
- Reset/deletion rules that cannot resurrect removed records from late pages or
  ranges. Removing an interest is not deleting the object.
- Cross-feed handoff evidence where related records arrive independently. A
  transaction at the publisher does not make different network streams arrive
  atomically at every subscriber.
- Immutable operation IDs/payloads, durable reservation and explicit outcomes.
  An uncertain external effect stays uncertain; shared code cannot make that
  effect and a database transaction universally atomic.
- Bounds and cancellation at every receiving boundary, including metadata and
  decompression, with distinct retention of authored work and disposable cache.

This extracts existing guarantees into clear owners; it must not remove them in
pursuit of superficially symmetrical endpoints.

## Broader model migration (deferred)

1. **Shared semantic contract first.** Define typed public metadata, body refs and
   relationship helpers. Use them in daemon publication and client projection,
   initially retaining the existing wire format. Move the earlier transcript
   proposal's replicated semantics out of the frontend. Keep disclosure/layout
   policy in the frontend. Delete replaced string conventions and reconstruction.
2. **Prove one complete path.** Send a message from A, queue/edit it, deliver it to
   history and observe it on B using that shared model. Include a tool with lazy
   input/output to exercise semantic references and body interests.
3. **Extract the reusable receiver.** Move protocol assembly/installation out of
   the frontend behind a committed-storage sink. Keep endpoint-specific execution,
   upload publication, UI invalidation and subscription policy outside it.
4. **Evaluate catalogue/outcome convergence next.** Reuse revisioned collections
   where this actually deletes the alternate synchronization algorithms. Plan
   explicit protocol/schema migrations rather than silently combining clocks or
   treating ephemeral runtime state as durable state.

The proof should include disconnect after server commit but before the response,
repeated submission of the same immutable operation, both clients editing an
old revision, out-of-order queue/history pages, partial bodies, source changes,
cache reset, and restart during an uncertain effect. Assert one logical message,
no execution caused by state replay, bounded bytes and convergence at a quiescent
source cut for the same authorized interests and requested bodies. Faults
should be exercised through shared model/replica code, then through two actual
controllers and a daemon; existing E2E and weak-link fixtures are starting points.

Measure combined daemon + frontend + shared production code, not just a smaller
frontend directory. The extraction earns its keep if it removes semantic and
synchronization branches while retaining the existing safety tests. Introducing
a generic framework without deleting the old paths would not satisfy this goal.

## Evidence and scope

Tau evidence is the source paths above at `40a3698`, together with
`blocks/src/tests.rs`, `frontend/tests/end_to_end.rs` and
`frontend/tests/native_link.rs`. Tests were inspected, not executed in this pass.

Quake evidence is the official `id-Software/Quake-III-Arena` repository at the
revision named above. Primary-source locations inspected: `msg.c:786–1060`,
`sv_snapshot.c:119–190,271–360`, `cg_predict.c:484–584`,
`g_active.c:939–941` and `bg_pmove.c:2020–2026`, under the paths in the table.
No upstream source is copied into this repository.

Validation for this proposal is source inspection and documentation diff checks.
No Rust builds or behavioral test runs, service changes, deployment, wire changes,
storage migrations or implemented performance claims are part of this commit.
