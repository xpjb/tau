# Tau2 latest network rewrite — resume here

## Location and safety

- **Branch:** `feat/tau2-block-sync`; **worktree:** `/root/tau2-block-sync`.
- Original integration base: `c5088dc`; previous read-path checkpoint: `4784635`. Use `git log -1 --oneline` for the current local checkpoint.
- **Not pushed, merged or deployed.** Preserve `/root/tau2`, `/root/tau2-remove-pause` and unrelated worktrees.
- **Do not deploy/start beta or restart/change stable.** Read-only verification: `tau2-beta.service` inactive / PID 0; `tau.service` active / PID 474496.
- `/root/AGENTS.md` applies: **Clippy is banned in every form**. Use managed `/usr/local/bin/cargo`, nextest and relevant rustdoc checks; never Cargo's built-in test runner. Shared build-lock contention is handled by retrying normally, not bypassing the wrapper.

## User context

The original disk-quota interruption was resolved by the user; the read checkpoint passed 129 tests. The user subsequently asked to read the handoff and **“okay please finish implementing this.”** This continuation implemented the native write/control cutover and substantial correctness/resource work. It did not change any production service.

**Status: native read/write/control cutover implemented and locally exercised, but the full rewrite's release gates are not all finished.** Do not describe a passing suite as full release certification. The exact remaining implementation/audit/measurement gates are at the end of [docs/tau2-block-sync.md](docs/tau2-block-sync.md).

The [original protocol audit](docs/tau2-protocol-audit.md) is historical evidence/design. The implementation/status document supersedes its descriptions of current wire routes and limits.

## What this continuation implemented

- Protocol **17**, strict **4 KiB** control frames/messages in both directions. Large command inputs upload immutable bodies; large responses/settings/lists/catalogs use native body references. Small acceptance/operation markers remain independent of display/body completion.
- Production uses one native Iroh endpoint/connection for chat, uploads, descriptors and files. Removed legacy HTTP file/upload/offer routes, transcript wire APIs, Iroh-blobs dependencies and legacy transfer fixture. Daemon/raw crash-test clients now consume native blocks.
- Durable resumable upload specs/prefixes/sealing, bounded hashing/reads, deterministic fsynced file publication and stable lost-ACK retry. Persistent transport upload/restart/conflict/integrity/auth tests.
- Source schema **4** generic mutation reservation/outcome journal; immutable IDs, replayed stored outcomes, explicit interrupted/uncertain recovery without reexecuting effects. `GetOperation` reconciliation. General client mutation outbox saved before submission; model-selection intents now use the durable per-chat outbox.
- Exact client-named chat creation rather than new starter aliases. Compatible legacy creation receipts remain recoverable. Async compaction returns Accepted after receipt ownership; completion is separate. Abort persists pause/receipt before cancellation.
- Queue accepted edit/delete overlays survive until full replicated convergence, including restart. Large/escaped event attributes are preserved in referenced metadata bodies rather than overflowing headers.
- Actual viewport/overscan interests, bounded root/body working sets, coalesced plan/configuration channels, hard 2/4/6/2 stream classes and renewable live-watch leases. Copy waits for sealed content and advances through bounded cohorts instead of starving after 30 cards.
- Logical body-cache eviction at 512 MiB; bounded/expiring descriptor and upload leases. Live source appends avoid rereading full old DB prefixes. These are not complete metadata/retained-file quotas or a proof of incremental render CPU.
- Prioritized control health writes; UI-independent bounded/coalescing event mailbox; state descriptor epoch/key fencing; small source resync broadcasts rather than retained large list broadcasts.

## Validation

From this worktree, using the managed wrapper:

```sh
/usr/local/bin/cargo check --locked --workspace --all-targets
/usr/local/bin/cargo nextest run --locked --workspace --no-fail-fast
/usr/local/bin/cargo doc --locked --workspace --no-deps
git diff --check
```

All commands passed after the final code changes. **138 tests passed, 0 skipped, across 13 binaries**, nextest run `95f7eeb7-24aa-4ea2-8185-67de1555fe3e`. Compiler and rustdoc passed without warnings; `git diff --check` passed.

No Clippy, built-in Cargo test runner, deployment, mobile manual validation or actual constrained-network latency certification was performed.

## Continue here — explicit remaining work

Read the full status document before modifying code. The important unclosed gates are not the old HTTP-upload cutover anymore:

1. Cold first-prompt acceptance still follows runtime/history loading; audit/refactor durable queue ownership independently of the heavy runtime. Audit every delayed receipt/data UI transition, multi-record local crash windows, legacy alias migration and interrupted private/tool context. General uncertain actions need better inspection/reconciliation UX.
2. Finish aggregate metadata/tombstone/export retention, lease maintenance and list pagination; test ENOSPC, file-upload deletion/cancellation races, many peers/fanout and incremental cache/render/copy CPU. Preserve authored data rather than silently evicting intents or uncertain receipts.
3. Provide/test the old-backup restore lineage-rotation and migration/rollback procedure. Same-lineage rollback is not covered by normal restart tests.
4. Add native byte/latency/connection/queue counters and delayed/lossy/low-bandwidth regression scenarios. Removed legacy transfer tests are not native performance evidence. Perform mobile/manual UI certification only through an authorized workflow.
5. Re-run managed checks/nextest and update these files. **Deployment still requires explicit user authorization.**

Approved design remains unchanged: uniform flat blocks; explicit direct-child/body interests; no body data in collapsed headers; stable streaming identities; durable verified offsets/cursors; one small control socket plus one shared data connection; no automatic replay of uncertain paid/external work.
