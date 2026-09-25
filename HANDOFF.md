# Integration release override — beta 0.7.4 / protocol 18

The user explicitly authorized pushing/merging the native rewrite and redeploying
beta, with **no new backup**, then sending Windows and Android packages. The
feature below is being released through `/root/tau2`, local `tau2-integration`
tracking `origin/tau2`; stable remains untouched. The historical no-deployment
restriction below is superseded for this authorized beta rollout only.

Reuse the completed merged check and **169/169 nextest result**
(`422f6a71-95bd-4ce0-88dd-f3f6d581ee83`) and release daemon build. **Do not rerun
unnecessary tests.** The user explicitly requested minimal rollout checks after
an opaque combined build/test command caused unacceptable delay. Build client
targets sequentially with one Cargo job/Rayon thread through the managed wrapper.
Current release/delivery facts belong in `INTEGRATION.md`.

---

# Tau2 native network rewrite — completed local implementation

## Location and safety

- **Branch:** `feat/tau2-block-sync`; **worktree:** `/root/tau2-block-sync`.
- Previous checkpoint: `0b08126`; use `git log -1 --oneline` for the completed local checkpoint.
- **Not pushed, merged or deployed.** Preserve `/root/tau2`, `/root/tau2-remove-pause` and unrelated worktrees.
- **Do not start/deploy beta or change/restart stable without explicit approval.** Read-only verification: `tau2-beta.service` inactive / PID 0; `tau.service` active / PID 474496.
- `/root/AGENTS.md`: **Clippy is banned in every form**. Use managed `/usr/local/bin/cargo`, nextest and rustdoc. No Cargo built-in test runner or wrapper bypass; retry shared-lock contention normally.

## User context and status

After the previous handoff explicitly acknowledged unfinished work, the user said **“sounds pretty based, go ahead and finish the full task.”** This continuation completed the remaining implementation and automated local validation. It did not alter any production service.

**Code/local automated gates are complete; deployment and real device/WAN certification are not authorized or claimed.** Read [docs/tau2-block-sync.md](docs/tau2-block-sync.md) for the current contract, limits, measured evidence and backup/restore/rollback procedure. The older protocol audit is historical, not an outstanding implementation checklist.

## Completed since `0b08126`

- Protocol **18**, source schema **5**, authored client schema **2**, replica schema **2**. Required durable Hello lineage binding; transactional source-change fences and persisted cross-handle replica epochs.
- Cold acceptance independent of history/provider preparation; context parsing outside locks, bounded/cancellable preparation, eight owned runs, two title jobs, 128-runtime admission and cold-project deletion without runtime inflation. Startup recovery is batched; portable export streams SQLite blobs.
- Private-context call-ID preservation and per-turn dangling-result repair. Native display pairing uses block parents/identities rather than reused provider IDs.
- Saved-action inspection/check/retry/forget UX, clear-replica and live diagnostic copy. Transactional alias recovery preserves distinct draft/file bundles. Missing-source authored work remains reachable and can be copied into an unsent new draft. File-save failure and explicit unreferenced-file removal are handled safely.
- Source/cache metadata and SQLite limits, bounded journal retention and whole-scope reset. Retained upload manifests/fork owners, periodic lease/orphan maintenance and conservative legacy-file handling. Detached finalization keeps ownership/capacity through cancellation. Outbox snapshots carry integrity digests and do not depend on later edits to originals.
- Keyset-paged sessions/projects, structural revision fencing, coalesced traversals and delayed status protection. Bounded receipt/submission cohorts; read-only request IDs no longer accumulate unnecessarily.
- Sparse cache projection, dirty plans, bounded previews/working sets, capped ordinary prefetch, explicit complete copy with aggregate metadata preflight, closed-child copy, full file captions and attachment cards independent of collapsed tools. Disposable account replicas/downloads have separate collection policies; authored files are not silently evicted.
- Independent bounded control writer/health lane, healthy-probe-based jittered backoff, native class priorities, dual-stack direct routes and diagnostic counters.
- Offline `taud --rotate-lineage DATABASE`: exclusive writer lease, preserved mutation ownership, no replay, restored-chat execution guards and explicit review that does not resume anything. No in-place downgrade to protocol 17/schema 4.

## Validation

Managed commands, all successful after implementation:

```sh
/usr/local/bin/cargo check --locked --workspace --all-targets
/usr/local/bin/cargo nextest run --locked --workspace --no-fail-fast
/usr/local/bin/cargo doc --locked --workspace --no-deps
git diff --check
```

**168 tests passed, 0 skipped, 14 binaries**; full nextest run `0737f711-2d72-4778-8040-6254e06351df`. Compiler/rustdoc are warning-free. Logs: `/tmp/tau2-finish-check.log`, `/tmp/tau2-finish-tests.log`, `/tmp/tau2-finish-doc.log`.

Coverage includes actual-process crash recovery, SQLite FULL and kernel ENOSPC, alias/source-fence rollback, post-rename cancellation, duplicate finish/fork retention/deletion, catalogue/status races, migrations/future-version rejection, sparse/large-copy limits, full metadata captions, cross-handle reset fencing, replica collection, IPv6 and 16-peer fanout. Actual headless GPU mobile/desktop tests passed; the inspected mobile restore dialog screenshot is `/tmp/tau2-restore-mobile.png`.

The genuine native shared-link regression runs taud plus two production Controllers and an unpaid local provider fixture through raw TCP/UDP proxies: **64 KiB/s per direction, 35 ms one-way delay, every 23rd UDP packet dropped**. It covers a 256 KiB upload, >40 KiB fenced code, 24 open tool disclosures versus a collapsed client, a 512 KiB cancel/resume download, concurrent rename, unread-file nonmaterialization and no provider replay.

Two recorded focused repeats: `f0b0fb1e-3a40-4418-bf3a-c0db38481990`, `7e53784c-b23b-4d21-9fd8-384d3151290d`; control receipts **207 / 203 ms**, worst control RTT **366 / 224 ms**, roughly 1.15 MB shaped UDP, one native connection per client, >=64 KiB resumed offset, zero integrity failures. Logs: `/tmp/tau2-native-final-1.log`, `/tmp/tau2-native-final-2.log`. The final full suite also reran this scenario.

**Important transport finding:** the loss test triggered a fatal assertion in pinned `iroh-quinn-proto 0.13.0` multi-datagram pacing. Supported segmentation offload is now disabled; the shaped regression passes without weakening assertions or removing loss. Rerun it before changing that workaround or transport version. Initial evidence: `/tmp/tau2-native-link-gso-crash.log`.

## What remains operational, not an unfinished cutover

- Obtain approval for target-device packaging, Pixel/Shlap/manual UI testing, real WAN/Tailscale reachability and sustained resource/throughput measurements. Headless rendering and loopback shaping are not those certifications or hard latency guarantees.
- Follow the documented offline backup + owned-tree restore + lineage rotation procedure. A failed/interrupted rotation must be repaired/rerun before serving. A matching schema-compatible rollback is required; do not downgrade upgraded state in place.
- Application/page quotas need physical WAL/temp headroom; layout still walks bounded retained metadata. Unknown legacy files and ambiguous paid outputs require deliberate inspection/archiving, not automatic deletion or regeneration.
- **No push, merge, deployment, beta start or stable service change is authorized by this completion.**
