# Alert and control/content error follow-up — September 27, 2026

This is a source change, not a package or deployment. The control/content wire
formats, message representation and database schemas are unchanged.

## Alerts

- Popups last four seconds. Identical updates do not extend that deadline or
  resurrect an expired popup; a changed or cleared-and-reposted message gets a
  new deadline. The existing one-shot UI wake removes an idle popup.
- Text is 16dp and the card fits its measured, wrapped content instead of placing
  12dp text inside a fixed oversized card. The vector close icon is centered in a
  40dp hit target. Tapping the card still dismisses it without activating controls
  underneath.
- Popup lifetime is separate from persistent inline settings/recovery errors.
- Temporary control connection failures and typed content-transport failures go
  to connection details and copied diagnostics, not global popups. Socket/DNS/
  connection setup errors are tagged at their boundary. Generic I/O errors are
  **not** all network errors: disk failures and corrupt compressed content still
  surface, as do unknown-block and integrity failures. Durable intent recovery and
  inline download failures retain their existing semantics.

## Reproduced `Content sync: Unknown block`

The existing local-body reuse fix was working: after queue consumption, the
canonical message already held the verified text. But a previously built plan
could reach the content service after that merge. It still named the former queue
block, even though its own header said the body was sealed and complete. Looking
up that now-removed ID again incorrectly turned the stale interest into a fetch.

Complete sealed bodies are now excluded when forming download interests from a
plan. Changed, unknown and evicted bodies still get the normal fresh plan and
verified fetch. The regression exercises the real native scheduler, delivers the
old plan after queue consumption, and requires **zero body reads** for either the
old queue ID or the already-known canonical message. Missing content is not
reported as success, and this is not a queue/protocol redesign.

## Read/write lock contention and `Control is busy`

The unnecessary serialization was operations taking turns on one application
mutex/SQLite connection, **not** encoding a whole conversation to append a row.
SQLite WAL was already enabled. Short status/catalogue/receipt and content reads
now use four read-only connections rather than waiting for the writer's mutex.
Each read closure has one committed snapshot; mutation and receipt transactions
retain their existing ordered writer. Pool checkout waits asynchronously and is
bounded; cancelling a caller cannot lend its connection while blocking work still
uses it. Readers cannot mutate state or observe an uncommitted write.

A **synthetic local fixture**, not an observed production workload, published a
16 MiB block and issued eight status reads. Before this change the write took
about 178 ms, while individual reads did about 66–105 microseconds of query work
but waited about 178 ms for the mutex. After separating the read connections, the
read batch finished in about 2.3 ms while the write continued. These figures
illustrate the reproduced contention; they are not a production latency promise
or evidence that this particular write size occurred in the user's workload.

The old control admission path also immediately rejected request nine on a busy
socket. Short bursts now have a two-second admission deadline, without blocking
the WebSocket reader/Pong path. Running limits remain eight per socket / 32 total;
waiting plus running work is capped at 32 per socket / 128 total. Hard overload or
admission expiry is an explicit failure before request execution/reservation, not
an automatic replay. Disconnect cancels not-started work; already-running effects
retain durable completion/receipt handling.

## Validation

- Targeted before/after reproductions produced the exact two reported errors
  before their fixes, then passed afterward.
- Real authenticated socket tests cover bursts, Pong responsiveness, hard bounds,
  disconnect, admission expiry with no later execution/reservation, and status/
  catalogue/receipt reads while a write remains uncommitted.
- Reader tests cover consistent snapshots across a concurrent commit, rollback,
  read-only enforcement, bounded checkout and cancellation ownership.
- Native transport/controller tests distinguish actual connection loss from
  unknown blocks, storage I/O failures and invalid compressed content.
- Headless GPU tests cover desktop, phone and 2.5× phone layouts, measured text,
  centered vector icons, mouse/touch dismissal and expiry without user input.
  Set `TAU_NOTICE_PREVIEW_DIR` when running `app::notices` to save PNGs.

Final workspace nextest: **230/230 passed**, run
`263a1d0d-3822-4fd4-97fc-347014741cb9` (101.5 seconds). This includes the retained
normal and impaired-link end-to-end tests. The focused connection-classification
and crash-recovery rerun also passed 3/3 (`196084e3-545b-43e7-9012-1dbdd6d63f01`).
The first broad run passed 228/229; the crash-recovery subprocess inherited the
host's fixed IPv6 port and could not bind. Its fixture now removes the IPv6
override and live credential-source override, keeping it isolated from host
configuration. The full rerun above passed without skipping that recovery test.

Managed `cargo check --locked --workspace --all-targets` and rustdoc generation
for `tau-frontend`, `taud` and `tau-transfer` passed. All Rust work used managed
Cargo and nextest, with no Clippy or built-in Cargo test runner.

Physical-device acceptance, live-workload profiling and a broader network stress
campaign are not claimed. Existing normal/impaired-loopback tests remain in the
workspace; no arbitrary WAN or production-performance certification is implied.
