# October 2 rollup verification

Rechecked against `396d5ba`, including the latest catalog and Android startup
merges. The admission, disk-hydration and extended-outage regressions still fail
there and pass in the rollup; this branch is not obsolete. Source merge `64a9beb`
retains the current UI/startup code and both handoff histories. The native cases
also pass after adopting the shared pressure-link fixture. See the
[complete rollup record](branch-rollup-20261002.md) for exact runs and release state.
No physical-device result or blanket backlog-014 closure is claimed.

The earlier implementation record follows.

---

# Stuck block loading and retained viewport QA

Source fix on `fix/tau2-block-loading-stability`, based on `origin/tau2` at
`003de72` (beta 0.7.10). **Not merged, packaged or deployed.** The user has so far
observed the problem on Windows, not Android, with explicitly limited sampling.
The shared frontend failures and the separately traced native recovery delay
below have source fixes. They do not prove that every reported stuck block has
the same cause or establish physical Windows/Android acceptance.

## Confirmed causes and correction

1. **The thirty-root fetch cohort did not advance.** `Cache::plan_visible`
   selected the newest thirty viewport roots before checking whether they had any
   work. Already cached rows kept winning every plan; other visible rows could
   remain permanently unfetched. Closed tools could consume the entire cohort
   without requesting a single body. This can be exposed by short rows in a large
   desktop viewport/overscan, without any broken connection.

   Build each root's actual disclosure/body/overflow-metadata demand first.
   Ignore closed tools with no demand, and prioritize missing preview bytes or
   incomplete child-directory coverage ahead of completed roots. Keep explicit
   Copy first, the thirty-root cohort, preview and byte budgets, and stream-class
   reservations. Cached completions now let the next missing cohort start.
   Preview-limited bodies are considered against their prefix budget, not their
   entire size; Copy still requires complete sealed content.

2. **Disk hydration happened after painting without requesting another frame.**
   Workspace submits the measured viewport after its children paint. That can
   synchronously replace missing preview bodies from the existing disk replica,
   changing the Feed revision after the displayed `Loading…` was drawn. The
   on-demand UI had no repaint scheduled when no input/network event followed.

   Mark the UI dirty if this viewport projection changes the selected Feed
   revision. The next frame uses the real text/heights and the transcript's
   existing reading anchor. An unchanged viewport does not start a redraw loop.
   No block identity, message ordering or scroll-owner replacement was needed.

These are demonstrated starvation/invalidation failures, **not a demonstrated
mutex deadlock**. The initial UI fix did not change transport. The subsequent
packet-liveness correction below changes the native peer policy, not the
application wire protocol, schemas, daemon publication or durable operations.

## Evidence

All Rust commands used managed Cargo and nextest; no Clippy or built-in Cargo
runner, paid provider, live-data write, service restart or host network alteration.

- Pre-fix body-admission run `311433c3-4167-462c-abdb-186dfcf084e7`: only 30 of
  60 requested text bodies ever fetched across repeated plans.
- Pre-fix repaint run `2b62d6ec-993d-4273-bf67-76b56cac0d3f`: the actual controller
  hydrated the requested body from disk, but `App::tick` did not request the frame
  replacing the loading placeholder.
- Initial focused fixes: 2/2 passed, run
  `15dddc67-6e4b-4fab-ae5d-9596d6888bcc`.
- Extended focused validation: **72/72 passed**, run
  `0ab63ea7-f594-424c-b1d5-c974ccd4c1a5`, 48.419 seconds. Covers block/cache/copy
  scheduling, mixed open/closed tools, native reset/version fencing, retained
  lifetimes and navigation, actual desktop/phone-size GPU demand-driven repaint
  and anchor stability, recovery, background prefetch, and the real-daemon
  two-client impaired-link scenario. Other tests were deliberately filtered out.
- Windows x64 MSVC frontend-library compiler check passed. Existing platform /
  dead-code warnings were not changed. This is not physical Windows acceptance.

## Socket-recovery correction — traced and fixed in source

There is **no fixed fifteen-second application retry interval**. Fifteen seconds
was the original QA no-progress guard. The ordinary block-watch error retry starts
at 100 ms and grows to 5 seconds, but this delay occurred before that error path.

The opt-in packet/worker trace on the old peer policy captured an actual failure,
not just a passing repeat: run `a55470d7-e888-41be-9d0c-65a01f26a25a` restores UDP
after 17 seconds, then fails the **stricter five-second per-body progress guard**.
Control remains healthy at roughly 73 ms RTT. Requested streams were already
admitted and waiting on receive; the source had read the foreground suffix, and
no new client range reached the cache. Packet probes showed successively growing
waits (about 0.37, 0.74, 1.48, 2.95 and 5.90 seconds), with probe count 5 at
13.399 seconds. The native connection could remain alive under its old forty-
second idle policy, rather than report loss and start a fresh connection. This
establishes the reproduced packet-recovery wait; it is not a cache-write deadlock.

`55ba46f` changes the shared client/daemon QUIC configuration to a **five-second
max-idle transport parameter** and **two-second transport keep-alive**. These are
peer-packet liveness settings, not body/page deadlines. A healthy peer can keep
ACKing while a backend read takes longer than five seconds. The transport's normal
three-base-probe-timeout minimum on high-RTT paths still applies; this is not a
universal wall-clock five-second message deadline. Existing 30-second IO/credit
limits, checkpoint scheduling, stream classes and verified-prefix fencing remain.
No healthy connection is replaced merely because the WebSocket reconnects.

Closing an unresponsive connection must not strand a file download either.
`754096b` resumes **only interrupted read streams** from their verified cache
prefix. Eight consecutive transport failures without verified-byte progress end
in an explicit failure; retries back off from 100 ms to at most one second and
progress resets the failure cohort. Cancellation, source/cache changes, unknown
content, integrity errors and local IO/export failures are not retried. Export and
whole-file verification remain outside the read retry. No user command, paid
provider operation or durable mutation is replayed.

### Recovery evidence

- The pre-fix eight-second outage case observed roughly 7.93 / 8.49 seconds of
  extra reply delay (`32da56e1-1f40-4289-b213-349acb5ef408`). This is historical
  evidence, not the current implementation's result.
- With the shorter peer policy, **91/91 targeted checks passed** in
  `77103894-714a-495a-bf54-f7afec7b51f9` (74.299 seconds). This includes all
  transfer-library cases, actual IPv6/16-peer fanout, the six-second metadata and
  chunk checkpoints with one native connection, and a new **twelve-second idle
  peer** case retaining the same connection with no application streams. It also
  retains the UI/cache/recovery/prefetch and real-daemon impaired-link coverage.
- Extending the outage fixture with an original in-flight file exposed the old
  single-attempt file receiver: run `232f9dfb-c12d-46bd-9cf9-60305d77800e` failed
  because the short peer policy made the download terminal instead of resuming.
  That regression is corrected by the read-only retry above.
- Final file/recovery validation: **4/4 passed**, run
  `4c9e1e03-1b5f-4769-aa0f-a68adfbe88c2`. The eight-second blackout resumes
  foreground/background replies at about 0.82 / 1.09 seconds and the original
  file at 1.03 seconds, completing all at 2.91 seconds after restoration. The
  seventeen-second blackout records 1.64 / 2.59 / 1.88 seconds respectively and
  complete exact bytes at 3.78 seconds. Both keep the same healthy control epoch,
  selection and unread state. The full daemon/two-client upload/cancel/resume test
  still retains one healthy native connection per client. The new missing/corrupt
  file case fails explicitly after **one source read**, never a connection retry.
- Final Windows x64 MSVC frontend-library compiler check passed. No release build
  or physical Windows/Android test was performed. Existing warnings are unchanged.

The outage cases use the production Controller/watch/download/cache consumers,
real QUIC/SQLite source and private 64 KiB/s lossy UDP/TCP proxies. A read-only
control fixture responds to real WebSocket pings; no provider can execute. Each
reply/file has its own five-second **test** progress guard, exact bytes and
verified-offset checks, separate from total completion time. The earlier 72-case
run and later runs overlap; their counts are not summed as distinct tests.

### Remaining scope and diagnostics

The traced socket-wide recovery failure is fixed in source. Backlog 014 remains
open for the original unmerged seed-73 **per-file** failure and physical-device
acceptance; this is not proof that a particular Windows stall was reproduced or
that every healthy-connection stream wait is now bounded.

`RUST_LOG=tau_native_watch=trace` enables content-free worker stages for admission,
receive, cache processing, credit and file reconnects. In the isolated native-link
fixture, `TAU_NATIVE_TRACE=1` additionally records probe timers and packet sizes /
directions, never payloads or credentials. Copy connection diagnostics remains
aggregate, sampled every five seconds; it is not a particular block's wait trace.
