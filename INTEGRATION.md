# Tau 2 integration work log

User authorized integration, cleanup, remote `tau2` publication and Windows/Android
build delivery. Existing production daemon/data stay untouched. Build/test work
uses managed Cargo, low concurrency, sequential release targets; exit 75 defers.

Sources: frontend `70abf41` (preserved remotely as `tau2-rust-frontend`), native
backend `13097eb`, followed by the storage agent’s `cddb8b7`. Local integration branch is `tau2-integration` because the
other worktrees still own `tau2/*` refs; final remote branch is `tau2`.

## Work plan

- [x] Merge native backend with shared protocol and native frontend, retaining
  transfer/packaging changes, first-observed section timestamps and durable idle eviction.
- [x] SQLite canonical session/entry/queue/receipt store, transactional acceptance,
  explicit read-only Tau 1 import, indexed history reads and fork/restart tests.
- [ ] Portable history export (SQLite backup is already documented).
- [ ] Unified revisioned daemon/agent/system/project/provider settings in Rust UI;
  native chat actions and explicit durable acknowledgement/recovery semantics.
- [ ] Remove superseded Kotlin/Pi runtime, FFI/packaging and protocol shims; retain
  import compatibility and installer isolation, last-chosen model and estimated cache TTL.
- [ ] Native frontend↔daemon↔provider/tool/transfer/restart end-to-end acceptance,
  protocol/storage tests, rendering validation, all low-concurrency.
- [ ] Commit/push remote `tau2`, sequential Windows/Android builds, verify and deliver.

No live provider requests or production migration/deployment have been performed.

## Storage-agent reconciliation

Adopted `cddb8b7` as the sole storage implementation, including SQL prefix copies,
bounded hot transcripts, checkpoint/suffix-only provider replay, database revisions
and the real SIGKILL/WAL/compaction-receipt test. No competing schema or writable
JSONL path was retained. The earlier integration-side work is parked locally at
`scratch/tau2-storage-reconciliation` (`4603a95`), not part of the release tree.

The merge retains the shared owned/serde protocol types; daemon projection uses
one thin wire wrapper and daemon-only conversion trait. The frontend’s obsolete
Pi subprocess fixture is replaced with its real controller/transport against the
native daemon, a gated local provider, SQLite, shell tools, uploads, settings CAS,
forking and client restart. Fixture handshakes use the shared version constant.

Validation: `cargo check --locked --workspace --all-targets`; managed
`cargo nextest run --locked --workspace --no-fail-fast`: **46/46 passed**.
One Cargo attempt deferred on shared-lock exit 75. One nextest invocation rejected
an extra test-thread option; subsequent runs use the wrapper’s two-test limit,
with `CARGO_BUILD_JOBS=1 RAYON_NUM_THREADS=1`. No host-policy bypasses.

Remaining reconciliation wins to carry forward: durable control/abort receipts,
a database owner lock, and keeping untouched model-selected starters reusable.
Settings UI, obsolete-path cleanup, runtime rendering and release builds are still
pending; these passing tests are not a production/live-provider acceptance claim.
