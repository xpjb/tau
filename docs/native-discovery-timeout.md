# Native discovery timeout — October 2, 2026

## Cause

The integrated pressure harness failed dodgy seed 29 with a native connection
timeout. An instrumented repeat failed the same guard, nextest
`0cc7e42f-588a-46ab-a94a-21f7eeed359c`; this was not diagnosed from a passing repeat.
The initial uninstrumented [failure report](network-pressure/rollup-dodgy-29-failure.json)
remains unchanged.

The packet trace shows:

- At 0.536 seconds, the proxy drops the first 124-byte Iroh discovery packet and
  the first 1200-byte QUIC Initial.
- QUIC retransmits at about 1.535 and 3.535 seconds. The proxy forwards these
  packets, but the server sends no handshake response.
- At 5.537 seconds, the client connection times out. A new attempt sends discovery
  again. This time discovery succeeds, followed by a successful QUIC handshake.

Iroh 0.35.0's `magicsock.rs` discards QUIC packets from an address with no node
mapping. Its `node_map/path_state.rs` allows a new discovery ping after five
seconds; ping expiry and the node heartbeat also use five-second intervals.
Tau's newly integrated five-second client idle parameter applied before the
handshake as well as after it. It ended the attempt before discovery could recover.
This is a Tau/Iroh timeout-policy conflict, not evidence that successfully
forwarded QUIC retransmissions were themselves lost.

## Small correction

The client uses the existing 30-second connection deadline as its local QUIC
idle parameter. The server still advertises five seconds. Once peer parameters
arrive, QUIC negotiates the smaller five-second value. The two-second keep-alive,
stream limits, packet-size/GSO workaround, framing, authentication and read-resume
behavior are unchanged. No retry loop, dependency upgrade or protocol change is
added. The five-second negotiated policy requires the matched updated daemon;
an older daemon can advertise a longer limit.

This separates discovery time from established-peer silence detection. It does
not add a five-second body-response deadline or claim that every initial
connection on a lossy link finishes within five seconds.

## Acceptance

The new real-UDP regression drops the first two client datagrams, reads exact
content on one connection attempt, then blackholes the established connection
and checks its silence timeout.

- Before the correction: **fails at 5.017 seconds**, with the same native connect
  timeout; run `27e8df0f-8d1b-4a6d-a71b-569e6dcaeefd` (1 failed, 19 filtered out).
- After: connects in **7.003 seconds**, one attempt; established peer timeout is
  **5.001 seconds**. This proves the correction retains short dead-peer detection.
- Native transfer, impaired-link and pressure selection: **26/26 pass**, 340
  filtered out, run `74939dbd-10cd-4d73-bfce-2839f0112985`, 122.781 seconds.
  Twelve-second healthy idle preserves its original connection.
- Six-second metadata/chunk read regressions: **2/2 pass**, 219 filtered out,
  run `b14ba088-4a31-4a72-805d-e4c0b5b90fcc`, 6.395 seconds.
- [Dodgy seed 29](network-pressure/rollup-dodgy-29-fixed.json): one native attempt
  and one connection per client, no transport errors or alerts, exact file bytes,
  and one canonical copy of each authored message. The guard is unchanged.
- UDP-only outages with a healthy, unchanged WebSocket: after 8 seconds of outage,
  the two bodies first progress at 0.605/1.006 seconds and the file at 1.601 seconds;
  after 17 seconds, 1.694/2.341 seconds and 1.965 seconds. All retain the unchanged
  five-second individual progress guard and complete exactly from verified state.
- [Pressure recovery seed 91](network-pressure/rollup-recovery-91-fixed.json):
  after the deliberate 8-second TCP/UDP outage, control returns in 129 ms and
  the three files first progress at 0.867/3.078/2.137 seconds. Files finish exactly
  in 50.801 seconds, within the unchanged byte-scaled completion budget. One
  expected outage timeout is recorded; there are no steady-link transport errors,
  alerts, integrity failures or duplicated authored messages.

Final workspace/all-target compiler checking passes after the trace-only proxy
address label was corrected to name its actual UDP socket. All Rust work uses
managed Cargo and nextest. Full logs, packet trace and all three passing pressure
reports are in `/root/tau-branch-rollup-20261002/`.
Trace additions are fixture-only and opt-in through `TAU_NATIVE_TRACE`; they log
packet sizes/directions, connection timers and native wait stages, not payloads.

The separate original seed-73 per-file gap and queue-reference lifetime finding
remain open in backlog 014/013. These results prove the cold-discovery correction
and preserve the already integrated socket-wide recovery fix; they do not certify
physical Windows/Android behavior or close every historical loss case.

The user considered Mog and chose to record its current adoption gaps separately,
then proceed with this correction and beta 0.7.11. `/root/mog/docs/TAU.md`, local
Mog commit `af01924`, records that source review. Mog runtime and dependencies are
unchanged; it is not part of this beta.
