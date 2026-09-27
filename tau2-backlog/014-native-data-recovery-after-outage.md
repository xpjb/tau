# 014 — Native data progress does not promptly recover with control

Status: **Open; deferred at the user's request on September 27, 2026.** The observed
recovery gap is real; its exact wait/root cause and the appropriate fix remain
unresolved. No timeout change or recovery-architecture rewrite is approved here.

## Observed problem

In the seeded network-pressure test, after an eight-second TCP/UDP blackhole
ended, control connectivity recovered but one client's two file downloads made
**no application progress for more than 15 seconds**. The other client progressed.
There were no alerts. This is an observed per-file progress gap, not merely a long
total download time and not proof that the connection could never recover.

The data transport is Iroh/QUIC, **not a second WebSocket**. Reconnecting control
intentionally preserves the native connection and transfer jobs. Consequently,
control readiness currently does not establish that those jobs are progressing.

## Evidence and related work

- [Recovery seed-73 failure](../docs/network-pressure/recovery-73-failure.json),
  nextest run `9967207d-1c24-406e-b821-e90658e7f569`, retained in `bcc541b` on
  `fix/tau2-network-pressure`. Both affected files had a recorded 15,006 ms gap
  after link restoration; neither had a terminal failure.
- Reader checkout max was approximately 0.14 ms; measured daemon DB timings did
  not explain the gap. Do not attribute it to the earlier shared-writer read lock.
- An instrumented repeat of the same seed passed. Small QUIC congestion windows
  and loss seen in that passing repeat do **not** establish the failed run's cause.
- The harness is `frontend/tests/network_pressure.rs` on the unmerged QA branch.
  It drives real controllers, transports, caches and daemon with a local provider
  through a private impaired link. Seeds reproduce choices, not exact OS/packet
  scheduling. These are local fixture results, not production measurements.

## Timing and ownership context

At investigation, control had a one-second **minimum interval between connection
attempt starts**, with five-second acquisition and unanswered-Pong deadlines. It
was not a one-second failure detector. Native operations have several 30-second
limits and QUIC has a 40-second idle timeout, but the download receive loop has no
explicit per-file no-progress deadline. The 15-second value above is the test's
acceptance guard, not an application retry setting.

Relevant code: `frontend/src/connection.rs`, `frontend/src/transport.rs`,
`transfer/src/blocks.rs`, and `frontend/src/blocks/files.rs`. The transport loop
already owns the service and jobs across control reconnects. Inspect the actual
wait and recovery decisions rather than assuming separate modules require a
centralized rewrite or that identical control/data timeouts would fix the issue.

## Acceptance when revisited

- Isolate where progress stops: connection/stream acquisition, transport loss
  recovery, credits, or local processing/storage. Capture evidence on a failing
  run; do not infer its cause from a passing repeat.
- After the retained bounded outage, original downloads must resume from verified
  state and complete with correct bytes. Observe each transfer's progress, not
  just a reconnected control socket or aggregate traffic on another file.
- Keep a justified, explicit per-file progress budget distinct from total transfer
  duration. Do not make the test green by arbitrarily increasing its timeout.
- Preserve healthy native work across unrelated control reconnects; do not force
  needless reconnects, re-download verified prefixes or replay durable controls.
- Add a targeted regression for the identified cause and retain seeded pressure
  coverage. Do not close this item based only on a successful same-seed rerun.
