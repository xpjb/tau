# Tau 2 integration work log

User authorized integration, cleanup, remote `tau2` publication and Windows/Android
build delivery. Existing production daemon/data stay untouched. Build/test work
uses managed Cargo, low concurrency, sequential release targets; exit 75 defers.

Sources: frontend `70abf41` (preserved remotely as `tau2-rust-frontend`), native
backend `13097eb`. Local integration branch is `tau2-integration` because the
other worktrees still own `tau2/*` refs; final remote branch is `tau2`.

## Work plan

- [ ] Merge native backend with shared protocol and native frontend, retaining
  transfer/packaging changes, first-observed section timestamps and durable idle eviction.
- [ ] SQLite canonical session/entry/queue/receipt store, transactional acceptance,
  safe one-time legacy import, indexed history reads, export and fork/restart tests.
- [ ] Unified revisioned daemon/agent/system/project/provider settings in Rust UI;
  native chat actions and explicit durable acknowledgement/recovery semantics.
- [ ] Remove superseded Kotlin/Pi runtime, FFI/packaging and protocol shims; retain
  import compatibility and installer isolation, last-chosen model and estimated cache TTL.
- [ ] Native frontend↔daemon↔provider/tool/transfer/restart end-to-end acceptance,
  protocol/storage tests, rendering validation, all low-concurrency.
- [ ] Commit/push remote `tau2`, sequential Windows/Android builds, verify and deliver.

No live provider requests or production migration/deployment have been performed.
