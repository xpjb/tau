# Tau2 organization cleanup

Base: `origin/tau2-refactor` at `037bd6a`. Review branch:
`refactor/tau2-localize-net`. No deployment, wire-version change, database
migration, dependency-version upgrade, or compatibility forwarding modules.

## Boundaries

- `tau-net`: shared messages, block/file contracts, validation, and optional
  authenticated native streams. Protocol-only users do not pull in Iroh/Tokio;
  the normal dependency graph has no SQLite. SQLite is used by test fixtures.
- `tau-block-store`: one SQLite owner for the block journal, verified chunks,
  cache eviction and durable uploads. Depends on contracts, not a network runtime.
- Frontend `net`: connection lifecycle, bounded UI handoff, replica subscriptions,
  viewer/index polling and transfers. Interests go directly to their workers,
  not through the WebSocket command queue and then another service queue.
  Frontend `replica` owns the verified local cache and display projection.
- Daemon `net`: authenticated socket handling, descriptor framing, paged listing
  and the native backend adapter. Command errors and journal completion have one
  boundary. `projection` owns display-block publication; `attachments` owns
  attachment validation, materialization and upload publication. Reader pooling,
  timing and the operation journal belong to `state`; slash commands to `agent`.
- Each crate's `tests/` owns its regression sources and fixtures. Private unit
  modules use test-only `#[path]` mounts to retain private access without widening
  production APIs. Android harness sources are in `frontend/tests/android`;
  release-script tests are in the repository's `tests/`.

## Audit decisions

| Finding | Resolution |
| --- | --- |
| Protocol/transfer split, transport importing storage for wire types | Merge into `tau-net`; move range types and upload validation into contracts |
| Unused `Acceptor` duplicates native server authorization | Delete it |
| Three native chunk-compression implementations | One bounded frame constructor; preserve backend-supplied hashes |
| `blocks` means contracts, SQLite, workers and projection | Name concrete owners; keep the block wire vocabulary |
| Scattered frontend transport/file services and double-queued interests | Group under `net`; keep independent cancellation without service facades |
| Async mailbox send never awaits | Make handoff synchronous; real network/database work remains asynchronous |
| `SizedMessage` duplicates `Message`; transfer progress takes a second notice path | Keep byte accounting private; use one download event path and coalesce progress |
| Mailbox close/wake behavior is ambiguous | Distinguish empty/disconnected and preserve a wake on final-sender drop |
| Local fuzzy matcher lives in network indexing | Move it to the shared code-viewer finder |
| Generic `details.rs` scans arbitrary event iterators | Put disclosure in `feed` and copy on a concrete verified `replica::View`; remove the demo-only bypass |
| Tiny tooltip text submodule and markdown's same-named facade | Fold into their actual owners |
| Daemon protocol re-export and repeated per-command error conversion | Direct contract imports and one fallible dispatcher |
| `OpenSession`/`GetHistory` have no constructors and already cannot deserialize | Delete these skipped variants and their dead handling; existing wire-rejection test remains |
| Shared history-page/change/delta models are no longer transported | Delete the page adapter; make changes/deltas daemon-local, without a wire wrapper or `Deref` |
| Transient transcript sequence/delivered fields have no consumer | Delete them; durable receipts remain unchanged in SQLite |
| Transcript retention clones a page just to find its cutoff | Walk retained events directly; test count/byte limits and preservation of live events |
| Remaining serialized enums | Retain wire meanings, including slash-command sources; lack of a current producer is not enough to break decoding |

## Measurements

Physical Rust lines across both workspaces and diagnostic examples; no
whole-tree formatting. For the non-test counts, exclude test-only files and
`cfg(test)` module spans **on both trees**, including the baseline's inline
modules. Small test hooks in production types remain counted. A source file
included by a test and by production is counted only once, as production.

| Measure | Base | Cleanup | Change |
| --- | ---: | ---: | ---: |
| Non-test Rust files | 120 | 103 | -17 |
| Non-test Rust lines | 31,470 | 31,231 | -239 |
| Nonblank non-test Rust lines | 30,877 | 30,614 | -263 |
| Test Rust lines, including test-module mounts | 15,303 | 15,469 | +166 |
| **All Rust lines** | **46,773** | **46,700** | **-73** |
| All Rust files | 174 | 200 | +26 |

The total file count increases because inline tests become separate files.
Moving tests is **not** counted as production-code deletion. The LOC reduction
is modest; the larger improvement is fewer production owners and queue/adapter
boundaries. These numbers do not count documentation as code savings.

The all-Rust total is reproducible on either revision with:

```sh
git ls-files '*.rs' | xargs wc -l
```

## Executed validation

All Cargo commands used `/usr/local/bin/cargo`, with `--locked --offline`.
No Clippy or Cargo built-in test runner was used.

- Untouched, separate baseline worktree: **398 nextest tests, 396 passed and
  2 failed**. Both UDP-outage fixture failures rejected read-only
  `GetModelCatalog`. A separate commit teaches that fixture to answer the
  catalog request; its rejection of mutations and outage assertions remain.
- Cleanup: **403/403 nextest tests passed**, zero skipped. Includes both outage
  cases and the slow blackhole/pressure tests. Five added regressions cover
  mailbox shutdown, progress coalescing, descriptor accounting, transcript
  retention and offline preview copy through the verified replica. Nextest list
  inventories retain all 398 baseline test function names; only those five are
  added (module/package names change with their owners).
- Root workspace `cargo check --workspace --all-targets` passed.
- `cargo check -p tau-net --no-default-features --lib` passed.
- `cargo doc --workspace --no-deps` passed.
- Android: `cargo check --target aarch64-linux-android -p tau-frontend --lib`
  passed using the installed NDK compiler/archiver.
- Windows: `cargo xwin check --target x86_64-pc-windows-msvc --workspace
  --all-targets` passed. This is compilation, **not** a Windows test run.
- `python tests/release.py`: **7/7 passed**. Shell syntax checks passed for the
  moved Android harness and the Android/Windows/release build entry points.
- `git diff --check` passed; no regression test definitions remain under `src`.

## Deliberately not claimed

- No native Windows execution or Android emulator/device run. The four recorded
  Windows failures in `backlog/windows-tests.md` remain open; a cross-check does
  not establish that platform-specific behavior is fixed.
- No installer packaging, release, deployment or changes to a running daemon.
- No speculative splitting of the still-large controller/code-view UI into
  forwarding modules. Their next extraction should follow a demonstrated
  behavioral boundary, not an arbitrary line limit.
- Network framing, cancellation, retry, credit and durability state machines
  remain distinct where their failure semantics differ. Fewer lines alone would
  not justify merging those meanings or weakening their tests.

## Subsequent checkpoints

The audit above describes merge `53d9a67`, not later work. Through `b9c024d`,
small follow-ups were committed directly to `tau2-refactor`: Windows path and
fixture fixes; removal of per-chat command catalogues and synthetic snapshot
messages; direct ownership of the initialized native client; retirement of
obsolete queue capability metadata; one retained control collection instead of
parallel form/button owners. The Windows runtime findings are recorded in
`backlog/windows-tests.md`; Wine's unsupported Iroh socket operation still
prevents claiming native Windows behavioral validation.

### Verified transport boundary

- `Client::read(request, priority)` now returns verified pages, block headers,
  ranges or explicit absence. Frame parsing, decompression, integrity checking,
  credit and fair stream renewal are private to `tau-net`. The old public watch
  variants, raw frame/header/codec types and manual credit API are gone, including
  their use in integration-test clients. There is no compatibility/test facade.
- Replication, downloads and descriptors enter the replica through one async
  `Cache::apply` path. The existing source/window transaction fences and the
  bounded UI notification channel remain intact.
- Reads renew on their original connection. Changing the configured node or
  lineage retires that connection; renewing a grant for the same source keeps
  it. Reconnection still resumes from the owner's persisted checkpoint.
- Uploads use one resumable `copy_from` implementation. Publication remains an
  explicit `finish`, allowing local attachment validation before publication.
- A missing source identity is `None`/`Absent`, not a generic integrity error.
  An obsolete subscription ends quietly, but absence cannot delete replica
  state: only the ordered directory tombstone can. Explicit downloads still
  fail when their file disappears. Missing/corrupt bytes of an existing block
  remain errors and are not retried as connection loss.
- Control/data independence, durable request fingerprints, authored inputs,
  stream budgets and cancellation semantics remain. Control protocol is now
  **25**, native ALPN **`tau/blocks/3`**; deploy matching client/daemon versions.
  No deployment was performed.

Validation used the managed Cargo wrapper, never Clippy or Cargo's built-in
runner. One full nextest batch ran **412 tests: 411 passed, one failed**. That
failure was a stale expected diagnostic string in the controller fixture after
moving malformed-codec coverage to its actual owner. The corrected assertion
and transport/replica boundary regressions subsequently passed **29/29**.
The full batch's real blackhole/pressure test passed, including its no-unexpected-
alerts and no-duplicate-effects assertions. Compiler checks passed for all
workspace targets, protocol-only `tau-net`, Android and Windows; rustdoc passed
with warnings denied. No test was disabled or deleted.

This checkpoint adds **50 non-test Rust lines and 31 test lines** relative to
`b9c024d`; it is an ownership/API simplification, **not a net LOC reduction**.
Across all follow-ups since `53d9a67`, non-test Rust is 31,231 → 31,176 (-55),
and all Rust is 46,700 → 46,749 (+49). Moving responsibility or adding regression
coverage is not counted as progress toward a 10k-line deletion target.

### Source-bound peer checkpoint

The next checkpoint moves data entry points from `Client` onto `Peer`:
`Client::configure` returns the source-bound handle; `Peer::{read, files,
uploader}` cannot be called with a separately supplied connection/lineage pair.
The endpoint still owns shared budgets and counters. Same-source grant renewal
returns the same peer and preserves its live connection. Replacing the source
retires the old handle, cancels admission/connect/filesystem work, and closes
its connection without waiting for an in-progress connection attempt's mutex.

Frontend workers now receive `Option<Peer>` after the cache binding commits;
they no longer carry a client plus an independent ready-lineage channel.
Transfers explicitly reacquire the current peer for retries, allowing a new
node for the same durable lineage but refusing a different source. Descriptors
and active readers remain pinned to their original binding.

**414/414 nextest tests passed**, zero skipped, including the full pressure and
outage tests. Added regressions hold all foreground slots while retiring a
queued peer, and restart a download on a new node: the matching lineage resumes
at the verified prefix; the differing lineage publishes no file. The prior 412
test cases remain. This checkpoint is a separate ownership change, not another
wire-version change; protocol 25 / ALPN 3 remain current.
