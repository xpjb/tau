# Stuck block loading and retained viewport QA

Source fix on `fix/tau2-block-loading-stability`, based on `origin/tau2` at
`003de72` (beta 0.7.10). **Not merged, packaged or deployed.** The user has so far
observed the problem on Windows, not Android, with explicitly limited sampling.
The two reproduced failures below are in shared frontend code. They do not prove
that every reported stuck block has the same cause or exclude a Windows-native
transport problem.

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
mutex deadlock**. Transport timeouts, connection retry policy, QUIC parameters,
wire protocol, schemas, daemon publication and durable operations are unchanged.

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

## Socket-recovery follow-up: still open

The user explicitly connected this report to backlog 014. The added
`native_link::block_outage` case uses the production Controller, native watch
workers and replica, a real QUIC/SQLite source, and a read-only WebSocket fixture.
It drops UDP in both directions for eight seconds while actual WebSocket
ping/pong continues. It checks **each** foreground/background reply's first
committed suffix, exact complete bytes, the selected Feed, verified-offset
requests and unchanged control epoch/selection/unread state. No provider can run.
The test's fifteen-second no-progress guard is not a new application timeout.

The first isolated pass recorded suffix progress at approximately 1.94 / 2.49
seconds after restoration. Final run `32da56e1-1f40-4289-b213-349acb5ef408`
passed **both native-link cases**, recording approximately 7.93 / 8.49 seconds
and complete replies at 8.57 seconds. The existing full daemon/upload/download
case still passes after the test proxy gained its optional UDP blackhole.

This demonstrates a variable native recovery delay despite healthy control, not
its exact cause or a permanent wedge. It does **not** reproduce/close the original
seed-73 per-file failure, certify Windows/Android network recovery, or justify
forcing replacement of healthy connections. Backlog 014 remains open. A failing
Windows run still needs its acquisition/receive-credit/commit boundary captured;
Settings → Copy connection diagnostics is useful context but its native counters
are aggregate, five-second samples, not per-block wait traces.
