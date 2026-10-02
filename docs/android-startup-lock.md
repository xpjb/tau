# Android startup lock contention — integrated source, October 2, 2026

Branch `fix/tau2-android-startup-lock`, worktree
`/root/tau2-android-startup-lock`, based on `origin/tau2` at `8cceeb8`.
Reproductions: `7d71909`; source fix and further regressions: `dd2a831`,
with pending-work reuse correction `b5937b2`.
User-authorized merge **`1321f65`** integrates feature `1a8ee0c` into
`origin/tau2` from freshly fetched `d3de9ad`, in the separate worktree
`/root/tau2-android-startup-merge` on `merge/tau2-android-startup-lock`. The model
catalog integration is preserved; the two documentation conflicts retain both
handoff/QA histories. **Merged, not packaged or deployed. Physical-phone diagnosis
remains open.**

## Report and reproduced causes

The user reports normally near-instant Android launch; a delay exceeding one
second is their recognizable recurrence. They suspect failure to acquire a DB
lock. This is a useful QA signal, not proof that every slow launch is SQLite.

Android synchronously opens `Store` and constructs `App` before drawing. In that
shared path, database setup and optional viewport maintenance attempted writes:

- `Store::open` rewrote `user_version` and ran schema setup on every launch.
- `Cache::open` reran schema/counter initialization and updated limits on every
  reopen, even for an already current replica.
- Reading the saved viewport, including the **first frame's** viewport change,
  tried to synchronously update disposable body-LRU metadata. An unrelated WAL
  writer could delay a verified read by the existing five-second busy timeout.
- When restored local work was pending, optional echo reuse tried a deferred
  read-to-write transaction. Contention produced a DB-lock error popup even
  though the authored intent was already safe and available to render.

Separately, concurrent replica opens used `try_lock()` on `.replica-admin.lock`.
A normal lease/GC overlap failed immediately with **“lock acquisition failed
because the operation would block.”** The lock also spanned SQLite setup, so a
migration/writer wait unnecessarily owned the entire replica directory.

The real writer-held Store, Cache and selected-chat regressions all fail the
one-second completion deadline before the fix. The directory-overlap regression
fails with the lock error. Nextest run:
`b34e9ad9-fcb4-410a-85bb-4e982c8a6906`.

An intermediate patch removed initialization writes and made Controller restore
read-only. A real phone-size App/GPU regression still failed at its first frame
(`14e3ee5a-3ac0-4fdb-b535-7ae76b3c96b2`): viewport projection also writes recency.
The final patch fixes the actual projection path, rather than adding a second
startup hydration algorithm; Controller/navigation code is unchanged.

Extending the first-frame fixture with an uncertain saved intent exposed the
remaining local-echo popup (`7496ab0a-8763-45c2-bbbf-9a95b92036f1`). Commit
`b5937b2` puts that optional optimization behind the same nonblocking admission;
the test now preserves both the draft and the uncertain intent without a popup.

## Correction and safety

- Current authored state opens without schema writes. Actual initialization and
  migrations use an atomic immediate transaction; future versions still fail
  closed, without replacing authored bytes.
- Current replica opens skip initialization writes. Earlier version-2 replicas
  without the additive echo table/index still migrate. Migration rollback retains
  verified bytes and the previous schema; no schema/version/protocol bump.
- Read recency and local-echo reuse try `BEGIN IMMEDIATE` with zero busy wait,
  then immediately restore the connection's five-second write timeout under its
  existing mutex. Only **busy optional-maintenance admission** is skipped.
  Uncontended visits still update LRU and reuse local bodies; non-contention
  faults still propagate. Real content writes,
  authored intents, WAL and `synchronous=FULL` retain their durability/locking.
  Under contention that particular read may retain its older eviction rank or
  local-body reuse may wait for another viewport/sync refresh. Neither discards
  authored intent, changes delivery state, or loses the verified cached view.
- Directory administration waits for a normal open/GC handoff instead of failing
  fast. The target's shared live-handle lease and SQLite file are established
  under that lock, preserving quota counting for not-yet-initialized live files.
  The directory lock is released **before journal/schema setup**. Shared leases
  still prevent GC from unlinking any live database/WAL/SHM, including clones.

No lock-file deletion workaround, database reset, authored-data GC, timeout
extension, continuous repaint loop, transport change or dependency update.

## Diagnostics and acceptance boundaries

Android logs one startup sequence with `tau-startup` stages `storage-begin`,
`storage-ready`, `controller-ready`, `app-ready` and `first-frame-rendered`.
Elapsed values begin after Chad's GPU/surface setup, at storage initialization;
**they are not total Activity launch time or a presentation measurement**. Chad's
existing GPU/surface logs provide the preceding boundary. Restore errors retain
context and their underlying chain, including `Open transcript replica`. The
markers contain no credentials, chat text or URLs.

No Android device was attached. The GPU regression is a 360×720 headless native
App, not Android lifecycle/driver QA. It checks first-frame cached-body pixels,
restored draft/uncertain intent and absence of a storage popup while a real WAL
writer is still held. It allows three seconds for GPU/font setup, below the old five-second
busy wait; storage-only tests retain their one-second deadline. The test does
not assert a physical phone's launch latency.

A future package still needs repeated phone cold launch, task removal/relaunch,
background/resume and launch during transcript sync. For any remaining recurrence,
compare the stage boundaries and crash/logcat output; do not assume the original
phone issue is closed solely because these local reproductions pass.

## Feature-branch validation on the source fix

All Rust work used managed `/usr/local/bin/cargo`, one Cargo/Rayon job and nextest.
No Clippy, built-in Cargo test runner, production-data access, paid provider
request, service restart, installer/APK delivery or beta version change.

- Focused startup, actual first frame, migration rollback/future rejection,
  read-LRU/write-timeout and live-replica GC/quota tests: **11/11 passed**,
  nextest `1de9d75c-bd76-49e0-8963-1bb3c9e02116`. Six are storage integration
  cases, including eight simultaneous account-cache opens retaining exactly four
  live replicas and the authored store. Saved-anchor/LRU behavior remains covered.
- Full frontend nextest: **210/210 passed**, zero skipped, 11 binaries,
  `0d5b5ca5-da61-4c57-9ecf-47b980a370fd` on final source `b5937b2`.
- Native frontend all-target compiler check: passed.
- Android ARM64/API29 frontend library compiler check with NDK 27.2 and the
  existing 16 KiB linker flags: passed.
- Windows x64 MSVC frontend library compiler check: passed (shared storage code).
- Frontend rustdoc (`doc --offline --locked -p tau-frontend --no-deps`): passed.
- `git diff --check`: passed. Existing platform/dead-code warnings are unchanged.

A first-frame test attempt hit the managed shared-build-lock timeout before
running; it was retried normally, without changing the lock or wrapper policy.
Logs are `/tmp/tau2-android-startup-{before,focused,native-check,android-check,
frontend-tests,windows-check,rustdoc}.log` and
`/tmp/tau2-android-first-frame-{before,pending-before}.log` on this host.
Measured storage-only fixture completion was sub-millisecond for Store/Cache and about 2.2 ms for a
selected cached Controller with the writer held; these are **host fixture
measurements**, not Android launch performance guarantees.

## User-approved integration — October 2, 2026

Merge `1321f65` combines the startup feature with the already integrated model
catalog/retained-usage fix. There are no source conflicts or dependency changes;
all startup frontend source/tests are byte-for-byte the validated feature, and
all intervening catalog source/test files are retained unchanged. Both sets of
handoff and frontend QA notes are preserved rather than selecting one conflict
side. The feature branch/worktree remain intact.

Fresh merged-tree managed validation:

- Native daemon/frontend all-target compiler check: passed.
- Startup, migration, LRU/local-reuse/write-timeout, live quota/GC and the catalog's
  real-controller regression: **12/12 passed**, three binaries,
  `a8923dc4-52c5-4251-b184-4ff75b449ae3` (165 unrelated cases not selected).
- Diff/conflict-marker checks and updated documentation links: passed.

Logs: `/tmp/tau2-android-startup-merge-check.log` and
`/tmp/tau2-android-startup-merge-tests.log`. The feature's 210-case suite and
Android/Windows/rustdoc checks are reused for unchanged frontend source, not
claimed as freshly rerun merged-tree gates. No full suite was repeated. Subsequent
integration edits are Markdown only. No Clippy, built-in Cargo test runner,
package build, service restart, production-data access or deployment. Physical
Android recurrence/launch-time acceptance remains open.
