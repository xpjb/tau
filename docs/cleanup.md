# Tau2 organization cleanup

Base: `origin/tau2-refactor` at `037bd6a`. Work stays on
`refactor/tau2-localize-net` for review; no deployment or wire/schema migration.

## Boundaries

- `tau-net`: shared messages, block/file contracts, validation, and optional
  authenticated native streams. Protocol-only users do not pull in Iroh/Tokio.
- `tau-block-store`: SQLite block journal, verified chunks, cache eviction and
  durable uploads. It depends on the contracts, never on a network runtime.
- Frontend `net`: connection lifecycle, UI handoff, replica subscriptions and
  remote file transfers. Frontend `replica`: verified local cache/view projection.
- Daemon `net`: authenticated request handling and native backend adapter.
  Provider execution and database state remain daemon policy.
- Each crate's `tests/` owns its test sources and fixtures. Private unit tests
  may be mounted with `#[cfg(test)] #[path = "..."]` to retain private access;
  that is not a reason for production modules to use path overrides.

## Triage

| Finding | Action |
| --- | --- |
| Protocol/transfer separate crates, transport importing SQLite for types | Merge into `tau-net`; move range and upload validation into contracts |
| Unused `Acceptor` duplicating native server authorization | Remove |
| `blocks` means wire types, SQLite storage, network workers and projection | Name owners explicitly; retain the block wire vocabulary |
| Frontend `transport`, `transport_events`, `connection`, `file_client`, `file_index` scattered at root | Group networking and remove fake async handoff / redundant events |
| Local fuzzy matcher in a network index module | Move to shared code-viewer finder |
| `details.rs` scans unrelated events through a generic iterator | Localize disclosure/copy at the feed model |
| Daemon protocol re-export shim and scattered backend implementations | Import shared contracts directly; group server-specific behavior |
| Inline tests, source-tree test files and integration tests mixed together | Move tests physically, preserve private access and coverage |
| Remaining serialized enums | Preserve wire compatibility unless an actual redundant representation can be removed |
| Very large controller and code-view UI | Do not hide them behind more tiny forwarding modules; follow up by behavior, not file length alone |

## Validation and measurements

The starting tree has 46,773 Rust lines in 174 files. Counts include tests;
moving tests out of `src/` is not a code reduction. Final counts and executed
checks will be recorded here after the cleanup.

Use the managed Cargo wrapper, compiler checks, nextest and rustdoc. No Clippy
or Cargo built-in test runner. Windows failures already recorded in
`backlog/windows-tests.md` require native Windows verification, not larger
timeouts or deleted assertions.
