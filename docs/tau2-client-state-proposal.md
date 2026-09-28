# Proposed Tau 2 client state

September 28, 2026. Proposal against `origin/tau2` at `40a3698`, fetched again
for this pass. This describes a replacement state model; it is not implemented.
The UI architecture is outside this proposal.

The follow-up `tau2-shared-replication-proposal.md` extends the replicated portion
of this model into a common client/daemon implementation. The ownership tree below
still describes the client host; public record semantics and replication machinery
should be shared underneath it, alongside the client's local intent/view state.

## State now

Selected fields from the actual types:

```text
Controller
├─ Account
│  ├─ projects, sessions, selected_project, selected, read_at
│  ├─ pending_create
│  └─ pending_controls
├─ chats: chat ID -> Chat
│  ├─ LocalChat
│  │  ├─ draft, files
│  │  ├─ pending: Vec<Pending>
│  │  └─ expansion, details_default, position, activity
│  ├─ Feed
│  │  ├─ events: order -> Event
│  │  ├─ by_id: event ID -> order
│  │  ├─ queue, queue_transitions
│  │  ├─ block_lengths, incomplete, block_states
│  │  └─ generation, sequence, before, loading, synchronized
│  └─ commands, commands_loaded, model_request
├─ remote: Cache
│  └─ native block headers, parent relationships, cached bytes, cursors
├─ requests, receipt_queue, receipt_inflight, source_guard
├─ downloads, saved_downloads
└─ assorted settings, connection and operation-result state

App::rows / Tools::lines
└─ reconstruct Row and Line values from the above on the render path
   ├─ group tools and reasoning
   ├─ decide pending / queue / history display precedence
   ├─ synthesize string identities and loading text
   └─ prepare copy / edit / disclosure actions
```

Sources: `controller.rs:44–94`, `store.rs:65–205`, `feed.rs:8–23`,
`blocks.rs:17–21,139–223`, `app.rs:2680–2928`, `details.rs:110–203`.

For example, asking for one message's current state can mean looking up its
`Pending`, a queue entry, an `Event`, `incomplete`, `block_lengths`, and its
native cached body, then applying the renderer's precedence rules. Each piece
exists for a reason, but the complete logical object has no single home.

## Proposed state

One `ClientState` is the cohesive model consumed by application features:

```text
ClientState
├─ source: account identity + source binding / lineage
├─ connection: control/data connection state and health
├─ catalog: topics + chat summaries + ordering indexes
├─ chats: ChatId -> ChatState              [loaded working set]
│  ├─ lifecycle: local / confirmed / source-missing / review-required
│  ├─ draft: authored text + attachment references
│  ├─ transcript: Transcript
│  ├─ queue: ordered ItemIds + server queue-control state
│  ├─ agent: advertised commands + model-operation reference
│  └─ view: expanded SectionIds + read mark + scroll anchor
├─ content: BodyId -> ContentEntry         [bounded content access]
├─ operations: OperationId -> Operation    [durable local intent]
├─ transfers: verified downloads + exports + saved-file references
├─ navigation: selected topic/chat + per-topic resume target
└─ settings: client preferences + daemon settings/revision
```

Every concept has an explicit owner. Catalog entries serve the whole chat list;
loaded `ChatState`s hold the working set, rather than loading every transcript.
Ordered lists and secondary indexes contain IDs into owned records, not additional
independently editable copies of those records. Network sockets, SQLite handles
and platform APIs live in services that apply committed changes to this model.

### Transcript: logical items with stable identity

```text
Transcript
├─ items: ItemId -> Item
├─ history: ordered ItemIds + loaded history boundaries
├─ local_order: ordered ItemIds awaiting server placement
├─ sections: SectionId -> section membership / body references
├─ by_request: RequestId -> ItemId
├─ by_block: BlockId -> ItemId
└─ sync: source revisions, per-feed cursors and handoff barriers
```

The queue in `ChatState` also refers to these same items. Queue/root observations
can overlap while independent feeds catch up; one reconciliation path determines
each item's displayed placement. A queue-removal barrier retains the prior
placement until the root cursor establishes what replaced or removed it.

A sketch of the item records (names and fields are proposed, not compiled APIs):

```rust
struct Item {
    id: ItemId,
    origin: OriginRefs,
    data: ItemData,
}

enum ItemData {
    Message(MessageState),
    Reasoning(TextState),
    Tool(ToolState),
    Attachment(AttachmentState),
}

struct MessageState {
    authored_body: Option<BodyId>,
    queued: Option<QueueObservation>,
    canonical: Option<HistoryObservation>,
    operation: Option<OperationId>,
}

struct ToolState {
    name: MetadataRef,
    input: Option<BodyId>,
    results: ChildFeed<ItemId>,
    execution: ToolExecution,
}
```

`QueueObservation` and `HistoryObservation` contain their revision/ordering and
body references; they do not own duplicate strings. `OriginRefs` carries explicit
request and native-block associations. Native identity establishes tool parentage.
`ChildFeed` includes loaded child IDs and discovery/paging state, so an unloaded
output list is distinguishable from an empty completed one.

`MessageState` represents simultaneously known facts. A receipt, queued header,
history header and complete body may arrive at different times. Named queries
such as `display_body`, `display_status`, and `display_position` implement their
precedence together, using operation state and content availability. The renderer
consumes the answer; it does not perform another reconciliation.

Logical identity uses the original request ID for authored messages and native
identity for other items. Keys are scoped to account/source/chat; block versions
are included in content references. Late updates from a previous source cannot
attach themselves to a same-named current item. Provisional chat aliases also
resolve through one model-owned mapping.

### Sections: one definition of grouping

`sections` is a derived index over the items: Details groups, reasoning, tool
input, tool output and ordinary message bodies. A tool output section has a
stable typed identity such as `ToolOutput(tool_id)`; its visible Error/Output
label is presentation, not its identity. Section membership is computed once
when affected transcript metadata changes.

Each section carries its native/content references directly. Display walks the
visible sections. Copy walks the requested section's membership with complete
content requirements. Fetch planning receives the visible sections' bounded
preview requirements or the copy operation's complete-content requirements.

This replaces display-key parsing and separate grouping implementations. Group
anchors remain stable when older history is prepended; migrate existing saved
expansion keys through an explicit adapter rather than dropping preferences.

### Content: availability is data, not placeholder text

```text
ContentEntry
├─ source: local authored bytes OR scoped/versioned remote block
├─ advertised size / source revision
├─ verified cache coverage
├─ bounded preview handle
├─ finality: growing / sealed
└─ acquisition status / error
```

`content` is a facade over existing authored storage, the verified replica and
bounded in-memory previews. Item records carry handles, not a fresh copy of every
body. Cache residency and preview length are separate: a full body may be on disk
while only its prefix is resident in the display preview. A growing body may have
all currently advertised bytes without being final.

The UI can now distinguish empty content, missing content, a partial preview,
growing text and complete content without interpreting the string `Loading…`.
The scheduler retains its existing byte budgets, integrity checks and source
binding. Cache eviction updates content availability; it does not delete the
logical item or local authored intent.

### Operations: local intent gets one address

```text
Operation
├─ original ID and source binding
├─ typed intent / target
├─ authored input and attachment references
├─ dependencies, e.g. chat creation or model selection
├─ preparation / frozen submission state
├─ delivery and observed outcome
└─ explicit recovery / replay policy
```

This replaces three different homes for durable intent: `pending_create`,
`pending_controls`, and each chat's `local.pending`. Per-chat or per-topic lists
are indexes into the operation registry. Model-selection waiters refer to the
model operation rather than relying on a loosely related flag.

Each operation kind retains its actual recovery behavior. Ordinary messages can
check receipts and retry the original missing submission; uncertain effectful
controls require review. Classify that policy once from the typed intent rather
than repeatedly inspecting slash-prefixed text in different recovery paths.
Prepared attachment submissions must retain the exact frozen request specification
under the original ID, alongside the original authored input.

Local intent is persisted before network submission and before publishing a
successful model transition. Disk failure leaves the draft available. Clearing
the replica cannot clear the operation registry. Changing source lineage fences
old operations for review before further submission.

## One message, through its whole life

Suppose request `R` creates logical message `M`:

1. **Send:** persist operation `R` and authored content `L`; create `M` referencing
   them; clear the draft only after the local commit succeeds. `M` is locally
   placed while awaiting the daemon.
2. **Acknowledgement:** update `R`'s receipt state. `M` remains the same item and
   displays `L`.
3. **Queue header:** associate its request ID with `M`, add its queue observation
   and queue-order reference. There is no second message object to suppress.
4. **History header:** associate the canonical block with `M`, update canonical
   ordering, and reconcile the queue handoff against the relevant feed cursors.
   `M` can still use `L` while canonical bytes are unavailable.
5. **Verified body:** update the canonical `ContentEntry`. The common body-choice
   rule can switch `M` to the canonical content; the operation is retired only
   when its delivery/convergence evidence permits it. Verified reuse of local
   bytes keeps the existing digest/size/source checks.

The identity is constant while observations and available content change. Copy,
scroll anchors, display status and content demand resolve through that identity.
Retry updates the original operation; it does not manufacture another message.
Queue edits/deletes are operations targeting an item and revision, not fabricated
user-message rows. Unresolved outcomes remain visible through operation state.

## Update ownership

Commands and source updates enter one model boundary:

```text
user intent -> validate / prepare local transaction -> commit -> publish state
                                                        |
                                                submit network effect

verified replica commit / operation receipt
  -> validate source + revision
  -> update affected item/content/operation and derived indexes
  -> invalidate affected consumers
```

The existing native service still verifies and commits bytes. The model consumes
that evidence. It updates only affected loaded items/sections rather than scanning
and reconstructing every message during painting. Feature reducers can be separate
modules operating on their owned records; the single state model gives them shared
identities and contracts.

## What changes or disappears

| Current | Proposed owner / replacement |
| --- | --- |
| `Account.sessions/projects` and navigation fields mixed together | `catalog` and `navigation` |
| `LocalChat` mixing draft, delivery and display preferences | `draft`, operation references and `view` |
| Flat `Feed.events` plus queue payloads | typed items with history/queue ID indexes |
| `block_lengths`, `incomplete`, `block_states` | content availability and typed tool state |
| `Feed.queue_transitions` | item handoff observations with source cursor barriers |
| `pending_create`, `pending_controls`, `local.pending` | one operation registry with typed policies |
| `rows`/`open_items`/copy membership reconstruction | one section index |
| String-key-to-native-ID decoding | explicit section source references |
| `Row` and `Line` carrying recovered domain meaning | lightweight layout views referring to items/sections |

The replica remains the native storage/transport implementation. Its frontend
output would expose typed metadata/content changes rather than flattening into a
legacy `TranscriptSnapshot` and requiring the UI to reconstruct the native tree.

## Migration

1. Introduce typed item/section/source IDs and explicit references on current
   detail lines. Remove the inverse key parser; preserve persisted view keys.
2. Replace the frontend feed/row projection with `Transcript` and shared section
   membership, initially adapting the current persisted local format. Move
   display and fetch consumers to it and delete the superseded reconstruction.
3. Consolidate operation ownership through a versioned, transactional local-store
   migration preserving IDs, payloads, attachments and recovery state. Test
   interruption/restart and schema compatibility before removing the old containers.
4. Move remaining catalog/navigation/settings/transfer ownership into the named
   `ClientState` fields as consumers migrate. This part is mostly organization;
   steps 1–3 provide the important semantic simplification.

This proposal requires no immediate daemon protocol change. Implementation must
preserve native source/version validation, bounded memory/cache behavior, existing
retry semantics and deferred scroll-restoration scope. The proposal itself changes
no application code, data, services or release artifacts; no Rust builds or
behavioral tests were run for this document.
