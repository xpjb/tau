# October 2 discovery correction — accepted for beta 0.7.11

The user requested fixing the release finding, then proceeding with beta. The
failed seed-29 packet trace and a new failing-before regression identify a
five-second client idle limit racing Iroh's discovery retry. The client now allows
its existing connection budget for discovery; the server still negotiates a
five-second established-peer limit. No extra retry loop or relaxed test guard.

Native transfer/impaired-link/pressure tests pass **26/26**, plus **2/2** slow-read
regressions. Seed 29 has one attempt per client and no transport errors. Both UDP
outage tests keep individual progress below five seconds; recovery seed 91 keeps
all three files progressing and completes exact bytes. See
[the cause, timings and run IDs](native-discovery-timeout.md). Backlog 013 and the
original per-file seed-73 finding remain open; no device acceptance is claimed.

The following rollup and September records preserve the earlier failures and
holds. The discovery correction above supersedes the initial release hold.

---

# October 2 branch rollup — initial result

The user requested carrying this harness into current Tau2, not archiving it.
The port keeps one shared link fixture for `native_link` and `network_pressure`;
UDP-only outages keep the control socket live, while pressure blackholes still
stop both TCP and UDP. The only API port is reading text from the typed Notice.
All assertions, original failure reports and diagnostic scopes remain intact.

Combined workspace/all-target compilation passes. Focused nextest covering all
daemon cases, native-link/pressure, new-chat and background-sync cases is
**82/83 passed**, 282 filtered out, run
`5e1485e8-dee2-4702-87ee-715777dd9486`, 115.613 seconds. The provisional-starter
integration issue is corrected and its unchanged background-sync regression passes.

Normal seed 7 and recovery seed 91 pass. After the deliberate eight-second outage,
all three files first advance within 0.79–1.58 seconds; socket reacquisition is
130 ms. Files complete with exact bytes, no alerts or duplicated authored messages.
A passing sample does not close the original seed-73 or queue-lifetime findings.

**Dodgy seed 29 fails the unchanged no-transport-errors guard.** Its retained
[report](network-pressure/rollup-dodgy-29-failure.json) records one failed native
connection attempt and one successful connection for client 0; client 1 connects
once. Both files finish with correct bytes, no alerts, no control reconnects,
no integrity failures and one canonical copy per message. The transport detail is
`Content sync: Content connection unavailable: failed connecting to remote endpoint: timed out`.
This is not the original per-file outage stall and does not prove a production
regression or a root cause. Preserve the failed result; do not retry it into a
green result, suppress its error, or lengthen a timeout without diagnosis.
Deployment is awaiting the user's call on this new release finding.

The September 27 record below is historical; its harness is now carried into the
rollup, but its known bug reports remain open in backlog 013/014.

---

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
- Two 1 MiB downloads, a setup upload and a distinct 128 KiB upload concurrent
  with bulk traffic, a 24-operation two-client control burst, queued inputs and
  queue-to-history handoff, tool-body interest changes, and a seeded download
  cancellation/resume point. One advertised file remains unread.
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
`tau::content` debug logs identify the failed local sync interest, current
header version/length/sealing and cached prefix length, without changing or
suppressing the popup. The pressure observer captures these scoped failures in
its report, and queued-message action traces now include the request ID and
authoring client for correlation. Message bodies and credentials are not logged.

The test's bounded observer collects p50/p95/max samples separately for reader
and writer connections and admission. Reports also include client-observed read
and durable-control latencies, controller poll duration, application bulk goodput,
TCP/UDP bytes, loss/stalls, connection counts, resume offsets, integrity failures,
and recovery time. Recovery has separate guards: 5 s for socket reacquisition,
15 s without per-file application progress, and a byte-scaled completion budget
of 10 s plus remaining bytes at 32 KiB/s. These are explicit test acceptance
budgets, not changes to application timeouts. Goodput includes preparation, the
injected pause, cancellation/resume and any deliberate outage; it is not raw link
capacity. Samples are small local runs,
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

## Validation and findings

The default pressure matrix, including concurrent upload, initially passed 3/3
(`a59a6bb6-06f8-4ecf-bd9e-41d9ff23cdf5`), and an earlier workspace run passed
233/233 (`119436a6-9049-445c-9e46-f895ee222517`). **The latest workspace run is
232/233**, not green: default recovery seed 91 caught the content race after
scoped diagnostics were added (`1e68635b-be14-4a5f-86dd-d32134c433d3`). No failing
case is ignored or retried into a passing result. Findings:

- **Recovery seed 73:** one client's two files made no progress for over 15 s
  after the link returned. Control recovered; reader checkout max was 0.14 ms,
  and the daemon DB timings did not account for the gap. The report is retained at
  [`network-pressure/recovery-73-failure.json`](network-pressure/recovery-73-failure.json).
  An instrumented rerun passed with the same seed. Temporary numeric QUIC
  diagnostics showed small congestion windows and repeated loss, but that does
  **not establish the cause of the failed run**. No QUIC parameters or application
  deadlines were changed; this remains a scheduling-sensitive finding.
- **Dodgy seed 173:** two unexpected `Content sync: Unknown block` alerts occurred
  while transfers and receipts otherwise completed. See
  [`network-pressure/dodgy-173-failure.json`](network-pressure/dodgy-173-failure.json).
  A diagnostic rerun passed; the alert is not dismissed as noise. The original
  sealed, already-held-body regression remains fixed, but this demonstrates that
  not every content-interest lifetime race is covered. Scoped diagnostics now
  identify the failed key and current local header on subsequent failures.
- **Default recovery seed 91, latest workspace run:** scoped diagnostics caught
  a request for `queued:<request-id>` while the local replica still advertised
  version 1, length 37, sealed. The server returned `Unknown block`; the authored
  messages and file checks later completed. This identifies a request for a
  retired queue ID before the local metadata caught up. That original report
  did not retain request-to-author mapping or cached byte counts, so it does
  **not** establish whether this particular fetch was for locally authored text
  or newly arriving text from the other client. Both clients were already
  connected and both submitted messages; queue headers do not include their
  body bytes. The test releases the provider after one client sees three queue
  entries, not after both clients have every body. The new tests do not suppress
  the error. See
  [`network-pressure/recovery-default-91-content-failure.json`](network-pressure/recovery-default-91-content-failure.json).
  This needs an explicitly verified retirement/revalidation fix, not blind
  success for arbitrary missing blocks.

At `bcc541b`, managed workspace all-target compilation and rustdoc for
`tau-frontend`, `taud` and `tau-transfer` passed after adding scoped diagnostics.
The subsequent request-to-author / cached-prefix diagnostic follow-up has not
been runtime-validated: its targeted seed-173 nextest invocation timed out after
20 s waiting for the managed build lock, before compilation or tests. The run was
deferred, not bypassed; this adds no new reproduction or passing result.
No deployment or service restart was performed.
This test/diagnostic branch is not merged into `tau2`;
the prior alert fixes were merged separately before this investigation.

The additional seed runs were `9967207d-1c24-406e-b821-e90658e7f569` (73) and
`0b1fea70-a535-4acf-b295-d132fe068ea7` (173). Each passed 2/3. Passing repeats do not
replace those failed results or prove that the findings are resolved.

### Negative control

Temporarily routing reads through the writer connection again made the normal
pressure case fail its read-latency guard: **p95 574.7 ms, max 619.2 ms**. The
original read-pool implementation was then restored (negative-control run
`e01f7513-bdd4-444f-a815-b656b254079c`). This confirms the harness
can detect the lock-sharing regression, rather than merely produce green output.
The mutation was never committed.

### Correcting a test assumption

The first recovery draft used one fixed 40 s completion timeout. Per-second
progress sampling showed files continuing to advance, not hanging. That was an
incorrect completion budget for three competing files on this lossy link. It was
replaced with the separate liveness/throughput guards above, not a longer
application timeout. This correction is distinct from seed 73's later genuine
**no-progress** guard failure.

In the passing default recovery sample, sockets were reacquired in **159 ms**,
per-file application progress resumed in **3–9.1 s**, and all three files finished
about **58 s after link restoration**. Slow content recovery despite responsive
control is now visible in the report, not hidden behind eventual success.

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
