# Seeded native network pressure (September 27, 2026)

Work branch: `fix/tau2-network-pressure`, after the alert/contention fixes merged
into `tau2` at `bdc1722`. This is isolated QA, not a deployment, live account or
production performance measurement.

## Exercise

`frontend/tests/network_pressure.rs` drives two real controllers (including their
transport, authored stores and replica caches), the real daemon, and a local
scripted provider. It reuses the existing `native_link` test's transparent
TCP/QUIC loopback proxy. The proxy never terminates WebSockets or answers Pings.
Both clients' TCP bytes and UDP packets share directional bottleneck queues.

| Profile | Rate per direction | Propagation per leg | Additional faults |
| --- | --- | --- | --- |
| Normal | 4 MiB/s | 2–4 ms | None |
| Dodgy | 192 KiB/s | 35–80 ms | Seeded 3% UDP loss; 180 ms stalls every 67 shaped chunks |
| Recovery | Same as dodgy | Same as dodgy | One deliberate 8 s TCP/UDP blackhole |

UDP buffering is bounded (650 ms scheduled delay, plus a hard task cap). TCP
uses backpressure: deleting bytes from a TCP stream would corrupt framing, not
simulate a network outage. No host qdisc, routes, service configuration or external
relay is changed. Seeds repeat workload/fault choices; OS scheduling and packet
segmentation mean they do not reproduce an identical packet trace.

The workload includes:

- Baseline read probes, then mixed status/catalogue/receipt reads under pressure.
- Two 1 MiB downloads, a 128 KiB upload, a 24-operation two-client control burst,
  queued inputs and queue-to-history handoff, tool-body interest changes, and a
  seeded download cancellation/resume point. One advertised file remains unread.
- A **test-only 400 ms pause of one actual application writer**, using the local
  diagnostic observer while it owns its connection mutex. Read-only connections
  stay available. No sleep/fault is enabled in production, and this is not a
  claim that a 400 ms pause occurred in the user's workload.
- In the recovery case, another download and an original authored intent cross
  the outage. Receipt ownership, canonical message copies, file bytes, unexpected
  alerts/errors, admission failures and connection replacement are checked.

The fixture serves its model catalogue as well as inference. An initial draft
used an external SQLite writer for the pause; that exercises SQLite transaction
upgrade/BUSY semantics, not Tau's application mutex, so it was replaced with the
actual-writer observer. Those draft fixture failures are not labelled app bugs.

## Measurements

`taud::db` debug events split connection/pool waiting, blocking-worker dispatch,
and work on the connection. Work includes SQLite lock/IO time, commit and local
processing; it is not a pure SQL CPU measurement. `taud::control_admission` records
wait duration and start/overflow/timeout/cancellation. These are opt-in local
tracing events, without SQL text, paths, request bodies, credentials or new wire
fields. Clock reads are omitted when the DB timing target is disabled.

The test's bounded observer collects p50/p95/max samples separately for reader
and writer connections and admission. Reports also include client-observed read
and durable-control latencies, controller poll duration, application bulk goodput,
TCP/UDP bytes, loss/stalls, connection counts, resume offsets, integrity failures,
and recovery time. Goodput includes preparation, the injected pause and
cancellation/resume; it is not raw link capacity. Samples are small local runs,
not production SLOs or a WAN/device certification.

## Run

Use the managed Cargo wrapper and nextest, never Clippy or Cargo's built-in test
runner. Keep builds sequential and respect its shared build lock.

```sh
TAU_PRESSURE_REPORT_DIR=/tmp/tau2-pressure \
CARGO_BUILD_JOBS=1 RAYON_NUM_THREADS=1 \
/usr/local/bin/cargo nextest run --locked -p tau-frontend \
  --test network_pressure --no-fail-fast --success-output immediate
```

Defaults are seeds 7 (normal), 29 (dodgy), and 91 (recovery). Set
`TAU_PRESSURE_SEED` to a decimal u64 to repeat the suite with another seed. Reports
are JSON named by profile/seed; preserve the failing seed, action trace, source
revision and link configuration when investigating. Retained unit/native-scheduler
regressions still cover the exact old busy and stale-known-text failures.

## Validation in progress

Workspace all-target compilation passed. In nextest run
`84ce8417-3a8f-440c-a032-aa6461682c63`, normal and dodgy pressure passed; the outage case exceeded
its bulk-completion budget after control recovered. A second run sampled progress every second and showed continued advancement,
not a hang. The fixed 40 s completion budget conflated liveness with throughput
for three competing files. Recovery checks are being separated into first
progress, no-progress gaps and a byte-scaled completion budget; application
timeouts are unchanged. The revised case is **not yet claimed accepted**.

First passing local samples (seeds 7/29, with the deliberate writer pause):

| Measurement | Normal | Dodgy |
| --- | ---: | ---: |
| Control read p95 | 31.8 ms | 621.5 ms |
| Durable control p95 | 910.3 ms | 1376.7 ms |
| Reader connection wait max | 0.086 ms | 0.127 ms |
| Writer connection wait max | 486.4 ms | 422.7 ms |
| Application bulk goodput | 985 KiB/s | 61 KiB/s |

Both had zero unexpected alerts, admission timeouts/overflow, duplicate authored
messages or unplanned WebSocket replacements. The writer delay was visible in
its timing without blocking read-only connection checkout.
