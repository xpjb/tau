# Tau2 native block sync

**Implementation and automated local validation complete. Beta 0.7.4 / protocol 18 is deployed; matched Windows and Android packages were delivered. Device/WAN certification is not claimed.** [HANDOFF](../HANDOFF.md) records the branch, final checks and operational restrictions. The [original protocol audit](tau2-protocol-audit.md) is historical design/evidence, not the current wire contract.

## Wire and ownership

Control protocol **18**; daemon database schema **5**; authored client store schema **2**; disposable replica schema **2**. Both client and daemon must match the control protocol.

Production uses one authenticated control WebSocket and one shared Iroh/QUIC endpoint/connection per client/daemon, ALPN `tau/blocks/1`. There is no per-file transport. Both IP families are supported, including an independently advertised IPv6 port.

- Flat `BlockHeader`: ID, optional parent, sibling order, kind, metadata, content version, byte length, seal and metadata revision.
- Direct-child feeds carry headers/tombstones, not bodies or implicit descendants. Cursors are `(lineage, sequence)`; history positions are `(order, id)`.
- Explicit body interests use content version and verified offset. Append/seal retain identity and prefixes; replacement changes version. Durable cache commits precede credit and advertised resume offsets.
- Large requests upload the immutable `ClientRequest`, then send `Input(ContentRef)`. Lineage, digest, sealing and inner/outer request IDs are checked before execution.
- Large replies become immutable `@control` bodies plus small `Data` references and receipt/ownership summaries. Page routing IDs stay in the small reference rather than defeating body reuse.
- HTTP file/upload/offer routes, Iroh-blobs and transcript wire messages are removed. Internal transcript types remain renderer/test adapters. Health, WebSocket and crash telemetry HTTP routes remain.

The source owns canonical history, queues, receipts, operation reservations and retained files. Replicas are disposable. Client drafts, imported files and pending intents are **not** replica data.

## Durable operations, recovery and local work

Prompt/queue receipts commit before execution; cold acceptance loads queue/head metadata, not display/provider history. Owned runs prepare bounded provider context outside the queue mutex and database gate. Abort durably pauses before cancellation, including while waiting for execution capacity. Compaction returns acceptance independently of its provider result.

Generic mutations reserve their complete canonical command in `operations` before effects. IDs cannot be rebound. Duplicate requests return stored outcomes. Interrupted reservations become explicit uncertain outcomes; recovery never reexecutes them. This is **not** a claim that filesystem effects and every database response share one universal transaction.

Provider context preserves opaque private items and complete tool-call IDs. Aborted/error assistant turns are not reintroduced through private metadata. Missing results are repaired per assistant turn with an unknown-outcome warning, not fabricated success or automatic retry. Native display pairing follows block parents/IDs rather than assuming provider call IDs are unique across turns.

Client intents are saved before network effects. Restart/reconnect queries outcomes rather than replaying uncertain work. Receipt queries and waiting submissions use bounded cohorts; the latter require the durable source-identity gate. A failed lineage transaction prevents Ready/automatic submission. Queue edit/delete overlays remain until complete replicated convergence.

Settings → **Saved actions** provides paged inspection, full intent copy, receipt checks, explicit retry of the original ID and confirmed local forgetting. Forgetting is not cancellation or undo. Diagnostics are copied at click time. **Clear replica cache** preserves authored work.

Legacy aliases use verified file copies and one transaction for target state, source removal and alias publication. An interrupted account update can recover the persisted alias. Distinct drafts retain their own attachments as separate recoverable bundles. Failed attachment saves leave no advertised local reference; explicit removal frees only unreferenced owned imports.

Source-missing chats with local work remain reachable as **Local recovery** in General. Their old intents do not resend. The menu can copy only the draft/files into a new, unsent chat, or explicitly forget the local recovery. Original pending intents are never silently retargeted to that new chat.

## Paging, projection and scheduling

Sessions use 64-row keyset pages; projects use eight-row pages. Structural/name/model/topic changes fence a traversal. Activity alone does not force a restart. Resyncs coalesce until both traversals finish, preventing busy chats from starving a large catalogue. Per-session status stamps fence delayed pages, including statuses received before list membership. Lists do not load histories or create cold runtimes.

Metadata watches batch within request-count and byte budgets. Hard stream classes reserve capacity; live watches yield after five seconds and reopen from committed cursors/prefixes. Metadata, descriptor and foreground priorities are distinct from bulk priority. Grants renew without replacing a healthy same-peer connection.

The WebSocket reader never waits on an outbound socket write. A bounded writer has a separate health lane and write deadline; RTT includes time waiting for that writer. Retry backoff has jitter and only resets after two good probes and 30 healthy seconds, not merely Hello. UI event enqueue is bounded and independent of UI consumption; overflow fails closed once, rather than accumulating fatal events.

Cache projection is sparse for dirty roots/parents, with separate queue convergence. Viewport planning uses point lookups; idle plans are not rebuilt continuously. Retained previews are bounded across sparse updates. Large ordinary bodies stop prefetching after their preview prefix; explicit Copy resumes the rest. Closed tool children do not fetch merely because their headers exist. Delivered attachment cards remain visible independently of collapsed tools, without fetching binary payloads.

Copy requires sealed content, complete relevant metadata and terminal tool state. It preflights aggregate body **and metadata** size, deduplicates interests and supports individual closed result children. It copies a complete **observed sealed** snapshot, not a source-linearizable cut of a changing hidden tree. Full native cache cuts replace old UI history, so a pruned reset cannot retain ghost off-window rows.

Replica reset epochs are stored in SQLite and checked inside page/header/range transactions, descriptor reads and final export publication. They fence outstanding jobs even across independent cache handles. Lineage checks independently fence old sources.

## Resource and retention policy

| Resource | Admission/bound |
|---|---:|
| Control frame/message, either direction | 4 KiB |
| Raw native chunk | 16 KiB |
| Block / transport header | 4 KiB / 5 KiB |
| Block / descriptor / clipboard body | 64 MiB |
| Uploaded command / file | 8 MiB / 50 MB |
| Feed page / batch | 32 records / 16 requests plus encoded-size budget |
| Per-stream application credit | 64 KiB encoded frame bytes |
| Client stream classes | 2 metadata, 4 foreground, 6 bulk/upload, 2 descriptor |
| Server streams per connection | 16 |
| File imports / frontend downloads | 2 each |
| Owned agent runs / title jobs / resident runtimes | 8 / 2 / 128 |
| Provider context | 128 MiB raw and prepared JSON; 10,000 entries; 64 MiB per raw entry |
| Source sessions / projects | 20,000 / 128 |
| Source headers / serialized header metadata | 250,000 / 256 MiB |
| Replica headers / metadata / tombstones | 100,000 / 128 MiB / 100,000 |
| Retained source change index | approximately 65,536–66,559 records |
| Source SQLite page budget | 8 GiB, installed before migration |
| Pending upload reservations | 1 GiB / 4,096 records |
| Descriptor leases | 256 MiB / 4,096 bodies, 24-hour expiry |
| Published upload manifests | 4 GiB / 8,192 files |
| Physical upload / outbox admission scans | 4 GiB each; 100,000 entries |
| Authored client SQLite / individual chat | 512 MiB / 32 MiB; 256 per-chat intents |
| Imported client files | 2 GiB / 8,192 files; never automatic authored-file eviction |
| Verified CAS / replica SQLite | 512 MiB logical bytes / 1 GiB pages per database |
| Account replica databases | four; inactive LRU/expired replicas can be collected, live handles are leased |
| Disposable download exports | 1 GiB / 4,096 files, seven-day TTL |
| Viewport body admission / root interests | 128 MiB / 30 |
| Preview payload | 256 KiB per group; 8 MiB and 32 retained groups, plus small labels |
| Frontend event mailbox | 512 events / 128 MiB, plus one terminal overflow |
| Control output / health lanes | 32 / 8 frames |
| Daemon control jobs | 32 global / 8 per socket |
| Generic client mutation outbox | 32 intents / 16 MiB inputs |
| Descriptor payload reservations | 128 MiB |

These are application/page/admission limits, **not** a single process-RSS or physical-filesystem ceiling. SQLite WAL, temporary copies, parsed representations and render metadata require headroom. Alias migration/finalization also need temporary space. Layout can still walk retained metadata; sparse projection and bounded body rendering do not imply constant-time layout at the maximum header count. Existing oversized authored stores are not silently erased to satisfy a new limit.

Metadata counters/quotas participate in the caller's transaction. Journal pruning forces old cursors to reset the entire cached scope, including off-window deletions. Body eviction preserves metadata and authored work.

Uploads bind their IDs to immutable specs. A manifest and owner precede publication; a detached owned finalizer retains capacity through rename/fsync and SQLite sealing even if its caller disappears. Sealed retries verify the owned file. Retained publication IDs cannot be rebound to different files. Forks inherit ownership; parent deletion does not delete a fork's uploads. Deletion invalidates late upload writes.

Maintenance runs at startup and every 600 seconds. It collects bounded batches of unowned/expired unpublished upload manifests and expired pure leases. Seven-day pure upload expiry does not expire operation ownership. Unknown legacy files are retained conservatively and counted against physical admission, not silently adopted/deleted. Existing published records are migrated through surviving transitive fork relationships.

Outbox deliveries are private durable snapshots, even when the original is already inside outbox. New snapshots record SHA-256; lazy materialization and image-context rehydration reject changed owned bytes. Retained/ambiguous paid outputs are not automatically collected. Failures must not cause automatic regeneration. This does not make a paid provider call, disk publication and receipt commit atomic, or guarantee preservation through arbitrary external filesystem damage.

Client export GC touches only the private hashed download namespace, never arbitrary user export destinations. Replica GC cannot unlink a leased live database or the authored store. Portable conversation export streams SQLite blobs to a fsynced temporary file and atomic publication, rather than collecting the whole conversation in memory; it rejects the source database as its destination.

## Backup, restore and rollback operations

Normal restart preserves lineage. **Restoring an older backup with its old lineage and then serving it is unsupported.**

1. Obtain deployment/maintenance approval and stop only the target daemon. The 0.7.4 beta rollout was explicitly authorized; it is not standing authorization to modify stable or perform later restores.
2. Preserve the pre-change database, configured outbox/upload trees, settings/auth files and matching binaries. Use a consistent SQLite backup or an offline database **with its committed WAL**; copying only the main file while WAL contains commits is not a backup. Keep secrets private. Portable history export is not a backup of receipts, queues, operations or file ownership.
3. Restore the consistent database and owned trees at their original absolute paths. Do not mix snapshots, relocate file references silently, or discard unclaimed paid outputs. Quiesce other writers to those trees too.
4. Before serving, run the matching implementation's offline tool:

   ```sh
   taud --rotate-lineage /absolute/path/to/tau.sqlite3
   ```

   It requires an existing absolute database path, shares the serving daemon's writer lease, recovers interrupted state without replay, rotates lineage, invalidates pure old inputs/descriptors, and places every restored chat behind an execution-review guard. Verify successful completion and the reported new lineage. If interrupted/failed, keep the daemon stopped and rerun/repair; do not serve a partially completed restore procedure.
5. On reconnect, clients invalidate old namespaces and fence old intents. Review external/paid effects that may have happened after the snapshot. **Review restored history** merely permits future explicit execution; it does not resume, resend or regenerate anything. Forks inherit the guard.
6. Roll back data only through this same restore/fence procedure with schema-compatible binaries. There is **no in-place downgrade** from schema 5/protocol 18 to the earlier cutover. Keep old backups and matching clients/binaries, but do not run an older writer against upgraded state or claim an untested downgrade path.

Use one configured database path, not hard-link aliases. The writer lease protects ordinary serving/import/export/rotation on that path; it is not a substitute for stopping the target before filesystem replacement.

## Automated evidence and limits

The managed compiler check, complete nextest suite and rustdoc pass; see HANDOFF for final run IDs. No Clippy or built-in Cargo test runner was invoked.

Coverage includes existing actual-process crash/restart tests and native E2E workflows, plus cold context gating, SQLite FULL rollback, kernel `/dev/full` copy failure, post-rename cancellation, duplicate finish, fork ownership/deletion, changed-file rejection, alias transaction failure, source-fence rollback, missing-source local recovery, paginated catalogues/status races, migration/future-version rejection, metadata quotas/pruning, cross-handle epochs, sparse preview bounds, full copy, dormant replica collection and backoff stability. A 16-peer test uses both IPv4 and direct IPv6 live body fanout. Real headless GPU tests exercise desktop/mobile-size settings, saved-action and complete restore-warning dialogs.

`frontend/tests/native_link.rs` runs the **actual daemon and two production controllers** against a local unpaid provider fixture. Transparent TCP and UDP proxies share 64 KiB/s capacity per direction, add 35 ms one-way delay and drop every 23rd UDP packet. They do not terminate WebSocket or auto-answer probes. The scenario uploads 256 KiB, receives >40 KiB fenced code, opens 24 tool disclosures on one client while the other remains collapsed, cancels/resumes a 512 KiB download and renames the chat during it. It checks hashes, connection reuse, real shaper traffic/minimum transfer duration, zero unread-file materialization and exactly two fixture provider calls.

Two final focused repeats took about 25.2 seconds, with control receipts **207 ms / 203 ms**, worst observed control RTT **366 ms / 224 ms**, roughly **1.15 MB shaped UDP**, one native connection per client, at least **64 KiB resumed offset**, and zero integrity failures. Runs: `f0b0fb1e-3a40-4418-bf3a-c0db38481990`, `7e53784c-b23b-4d21-9fd8-384d3151290d`.

The loss test exposed a fatal `iroh-quinn-proto 0.13.0` multi-datagram pacing assertion (`untracked_bytes <= segment_size`). The supported workaround disables segmentation offload while retaining ordinary QUIC reliability/congestion control. Do not reenable it or upgrade the pinned transport without rerunning the real shared-link regression.

Diagnostics expose native attempts/connections/streams, occupied class slots, verified content and Tau-frame bytes, requested resume offsets, cancellations and integrity failures, plus current-connection QUIC UDP/loss/RTT counters. Native samples update every five seconds. App-frame credit is not UDP traffic; requested offsets are not unique bandwidth savings. Slot occupancy is not a full queue-depth metric.

**Not certified by this rollout:** physical Pixel/Shlap/native-device behavior, arbitrary WAN paths or sustained device resource/throughput performance. Loopback shaping and headless rendering do not certify arbitrary WAN latency, radio behavior, hardware power-loss durability or device frame rate. The authorized beta deployment and Windows/Android packaging/delivery are complete; stable remained untouched. See [the release log](../INTEGRATION.md) for deployed versions and checksums.
