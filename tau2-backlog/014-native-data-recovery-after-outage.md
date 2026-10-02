# 014 — Native data progress does not promptly recover with control

**September 29 triage:** remains separate deferred reliability diagnosis. First capture a failing run's per-file wait/progress boundary (acquisition, receive/credits, commit or export), then scope a cause-specific fix. Neither the shared-state direction nor receiver extraction establishes its cause. Not a UI simplification prerequisite or LOC budget; do not silently close it.

Status: **Partially addressed in source; original per-file gap and device acceptance
remain open.** The September 27 deferral is historical: the user subsequently
requested investigation and correction during block-loading QA. A packet-traced
socket-wide wait now has a bounded peer-liveness/read-resumption fix below.
That does not establish the original seed-73 failure's precise wait boundary.

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
- The harness is `frontend/tests/network_pressure.rs`, carried into the October 2 branch rollup.
  It drives real controllers, transports, caches and daemon with a local provider
  through a private impaired link. Seeds reproduce choices, not exact OS/packet
  scheduling. These are local fixture results, not production measurements.

## Timing and ownership context

At investigation, control had a one-second **minimum interval between connection
attempt starts**, with five-second acquisition and unanswered-Pong deadlines. It
was not a one-second failure detector. Native operations have several 30-second
limits and QUIC then had a 40-second idle timeout. That baseline is superseded
by the peer-packet correction below; it is not a five-second per-file response
deadline. The 15-second value above is the original test's acceptance guard, not
an application retry setting.

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


## Initial block-loading QA follow-up — historical delay observation

The user raised this item again while reporting Windows transcript blocks stuck
on Loading and intermittent reflow. [The bounded QA fix](../docs/block-loading-stability.md)
reproduces and fixes fetch-cohort starvation and missing repaint after disk
hydration; neither is a proven data-socket deadlock.

A new production-controller/QUIC/replica case drops UDP for eight seconds while
read-only WebSocket control remains healthy. Foreground and background transcript
bodies both resume their verified suffix and complete exactly. Observed first
progress varied from about 2 seconds to about 8.5 seconds after restoration.
The per-body guard remains fifteen seconds; no application retry/timeout setting
was changed, and healthy connections were not forcibly replaced.

This is Linux loopback evidence for transcript bodies, **not** reproduction or
closure of the original seed-73 per-file gap or physical Windows acceptance.
The reported Windows-only pattern may be sample/viewport-related or a separate
native issue. Capture a failing run's wait boundary before selecting that fix.
Aggregate Copy connection diagnostics alone cannot identify a particular stuck
block's receive/credit or commit phase. This item stays open.


## Packet-traced native recovery correction

[The current evidence](../docs/block-loading-stability.md) includes a failing
17-second UDP blackout: streams were admitted and waiting on receive, cache
processing had already completed, source reads had produced the suffix, and
packet probe waits grew while the healthy control socket continued replying.
The old forty-second idle policy retained that silent connection. This is a
captured failure, not a root-cause claim inferred from a passing seed-73 repeat.

The shared native transport now negotiates a five-second max-idle parameter and
uses two-second keep-alives. This limits a packet-silent peer, not a slow backend
response: six-second page/chunk reads and twelve-second healthy idle retain their
connection. Normal QUIC high-RTT timeout floors still apply. Interrupted immutable
file reads resume their verified offsets with bounded transport-only retry; user
cancellation, source/cache change, missing/corrupt content and local storage/export
failures remain terminal, with no control or paid-operation replay.

Both eight- and seventeen-second outage regressions now include a real in-flight
file along with independent foreground/background replies and healthy WebSocket
ping/pong. All three make individual progress within five seconds after link
restoration and complete exactly, without another Download click. The existing
full daemon/impaired-link test, source/integrity guards and Windows compiler check
pass. No service/deployment change or physical Windows/Android acceptance occurred.

This fixes the demonstrated socket-wide delayed reacquisition. The original
historical two-file seed-73 pressure case is **not** certified by these separate
fixtures, so this backlog item remains open for that wait boundary/device evidence.

## October 2 rollup pressure finding

The harness is included at the user's request. Default normal/recovery cases pass
on the rollup; that does not close the historical per-file finding above.
Dodgy seed 29 instead records a failed native connection attempt, then correct
file completion with one successful native connection per client, no alerts,
no integrity failures and no duplicate messages. Its strict no-transport-errors
guard fails. [Exact retained report](../docs/network-pressure/rollup-dodgy-29-failure.json)
and [run/disposition](../docs/network-pressure.md). Cause and production relevance
remain unproven; do not present this as a diagnosed healthy-stream stall or
suppress the test. The user later requested a cause-specific fix and beta release.
The initial release hold is superseded by the correction below.

## October 2 discovery correction

The failed packet trace and a new deterministic UDP regression identify Tau's
five-second client idle limit ending an initial handshake before Iroh can retry
lost discovery. The client now uses its existing 30-second connection budget;
the server still negotiates five-second established-peer silence detection.
[Diagnosis and acceptance](../docs/native-discovery-timeout.md) retain failing-before
and passing-after evidence. No application retry loop or weakened guard is added.

All 26 native transfer/link/pressure cases and two slow-read cases pass. The
8-/17-second UDP outage bodies and file each progress within the unchanged
five-second guard; seed-29 cold connection succeeds once per client without
transport errors. Seed-91 pressure files resume and complete exactly. This closes
the diagnosed cold-discovery timeout, not the historical seed-73 per-file gap or
physical-device acceptance. Those remain open.
