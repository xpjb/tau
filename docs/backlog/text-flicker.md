# Text flicker investigation — October 7, 2026

**Open; the reported flicker and freezes remain undiagnosed.** The user now
reports flicker in both Tau and Compendium, apparently including non-text UI.
Longer alt-tab/minimized periods make the next event-driven repaints flicker
more; moving the pointer starts/stops bursts and eventually rendering recovers.
Prioritize the shared graphics/presentation boundary, not a Chad-only or
text-cache-only theory. This is a hypothesis ranking, not a diagnosed AMD,
winit or wgpu defect.

A normal Windows **0.7.14** installer has been built with automatic bounded CPU
diagnostics and coalesced background wakes. No launcher/environment setup is
needed. The recorder now counts actual encoded shape/image/text/emoji draw
commands as well as inputs. It cannot prove what pixels the GPU produced or
what Windows displayed. No flicker/freeze fix or battery-life improvement is
claimed. Runtime probes use the Windows binary under **Wine/NVIDIA/Vulkan**,
not the affected Windows/AMD integrated-GPU configuration.

## Integration and packaging

Audited Tau `f5ef622` and Compendium `2f39032`. Both manifests and lockfiles resolve
one Sanscale revision: `15ad1f03e17a368e2ff4f1b269f60bad13483638`. Its ancestry includes
`4325844`, the change that renews block age on cached `TextService::prepare` hits.
Tau's ancestry includes integration `1350fd2` and follow-up tests `8cceeb8`; its
current renderer still calls `prepare` for every painted text batch. This was not
an omitted dependency update or a missing prepare call.

The originally built Windows 0.7.13 client hashed to
`72bb20d2486f0f5468dff291af43a72d0f4a2a338ea47723ff2a34a4b58f5508`, matching the
0.7.13 Windows build receipt. Its embedded source references identify Sanscale
`15ad1f0` and Chad `a0f465e`. Verified the installer and compressed payload hashes
against the receipt, found that exact payload embedded in the installer, and
hashed its `app/Tau Beta.exe` member to the same client hash above. The package
script builds the client with `--locked`. This verifies the built artifact,
**not** which version is installed on the laptop. The 0.7.11/0.7.12 source pins already included
the original residency patch; 0.7.13 uses the newer shared master pin.

## Remaining reproduced gap

Tau's Markdown/editor retain shaped handles. Reading a valid handle with
`measure` does not renew its residency. The earlier fix renews residency only
when `prepare` runs, after the entire UI has assembled its draws.

1. Render a real Markdown message and composer draft; save reference pixels.
2. Allocate other layouts until the real 131,072-block capacity is reached,
   without another preparation of those cached message/editor handles.
3. Assemble the returning message/editor draws. All three handles are still
   valid; counters show zero shape requests, shaping calls or reflows.
4. Append a new status label. Its shape allocation sweeps the cache down to
   98,303 blocks and invalidates those three previously collected draw handles.
5. Tau prepares and submits the partial frame: **2,470 unchanged-content pixels
   disappear**. The following frame detects stale layouts, re-shapes them and
   exactly restores the reference content.

This uses the actual `Renderer`, Barkdown preview, editor and GPU draw path, not
an invented implementation of them. It does not drive the whole App navigation
or Chad's window/swapchain. The existing warm-cache regression passes because it
prepares a warm frame between steps 2 and 3, renewing precisely those handles.
Removing that intervening warm preparation exposes the uncovered case.

Compendium's ordinary source-backed/transient text draw assembly reissues shaping
before preparation, renewing valid handles before a later label can sweep them.
An equivalent returning-text + late-label pressure control passed through its
real renderer and matched an independent cold reference. This is not a claim
that all Compendium paths, including its active editor, are immune.

**Elapsed time alone does not cause this reproduction.** It requires intervening
layout allocations reaching the capacity bound. No measurement establishes that
this occurs during the reported laptop flicker. Do not relabel this synthetic
pressure result as a diagnosed GPU problem or a confirmed fix for the report.

## Other differences, not established causes

- Tau uses Chad; Compendium manages winit/wgpu directly. Both submit their render
  work before presenting; no obvious unconditional blank-present path was found.
- Tau prepares fresh text buffers on repaints. Compendium retains batches when
  draw inputs and `batch_live` agree.
- Tau requests AutoVsync with maximum frame latency 1; Compendium prefers Mailbox,
  falling back to Fifo, with latency 2.
- Tau's lockfile has wgpu 30.0.1; Compendium's has 30.0.0. Inspection of the local
  registry sources found no naga/wgpu-core source difference, identical DX12
  source, and a Vulkan swapchain fence change which retains the Windows wait.
  The version number alone is not evidence of the laptop cause.

## Executed checks and evidence

The original investigation below used `/usr/local/bin/cargo` and nextest. No
Clippy, built-in Cargo test runner, formatter, deployment or service restart.
The subsequent normal-client package build is documented below. Existing
Compendium/Barkdown in-progress work was preserved.

- Tau existing repaint tests: **4/4 passed**; frontend library: **223/223 passed**.
- Tau additional returning-cache probe: **failed as described**, with exact
  next-frame recovery asserted before the missing-frame assertion.
- Compendium existing text tests: **5/5 passed**. Its control also passed in a
  fresh archive of committed `2f39032`: **6/6**, avoiding concurrent working-tree
  changes as a confounder.
- Tests ran on Linux with the available NVIDIA/Vulkan adapter, not Windows iGPU.
  A separate GL attempt failed during device creation (`Parent device is lost`),
  before any tested rendering; it supplies no GL/device-flicker validation.

Evidence is retained at `/root/tau-checks/text-flicker-20261007/`: failing Tau and
passing Compendium probe patches, nextest logs, reference/dropout PNGs and the
committed Compendium control checkout. The temporary probe additions were removed
by exact replacement; no failing or ignored test was left in either main tree.
To rerun, apply the relevant `*-probe.patch` or `*-control.patch` in a disposable
copy of its audited revision and run:

```sh
# Tau
/usr/local/bin/cargo nextest run --locked --offline -p tau-frontend --lib \
  -E 'test(investigation_returning_cached_text)'
# Compendium
/usr/local/bin/cargo nextest run --locked --offline --test text_integration \
  -E 'test(investigation_returning_text)'
```

## Ordinary-use investigation and capture support

The following checks use small caches, not forced cache exhaustion:

- Added a full-App fixture containing the demo chat and a composer draft. It
  settles, and subsequent no-input ticks do not request repaints, both focused
  and unfocused. Cache occupancy stays below 1,024 blocks.
- Rendering 32 unchanged frames preserves exactly the same pixels. Counters
  report **96 text batch-buffer allocations and 8,096,256 vertex-upload bytes**,
  despite zero shaping/reflow calls. This establishes repeated upload work,
  **not** its contribution to battery drain or any link to flicker. This test
  reads back each frame; the separate existing queued-repaint test covers
  multiple submitted frames without an intervening wait/readback.
- Ran the real desktop under Xvfb with a fresh, unconfigured, isolated data
  directory and no server/token environment. It painted three startup frames,
  then none during six seconds idle. Ctrl+Shift+F12 saved a capture, followed by
  three feedback frames, then no more frames during the remaining idle period.
  All six recorded frames had no nonlive batches. The adapter was the
  NVIDIA GTX 1060/Vulkan, not a Windows integrated GPU.
- The native desktop probe verifies the shortcut, in-memory recording and
  orderly-shutdown save. This is an offline setup view, not a connected-idle
  or streaming/battery measurement.
- Controller heartbeat/metrics notifications wake Chad. Chad's user-event path
  requests a redraw, and runs `update` inside that redraw; Tau may then request
  another frame. Consequently `App::tick == false` alone does not prove that
  the desktop avoided painting on a background wake. Do not conflate the
  no-input fixture with a measurement of connected idle behavior.

### Normal Windows recorder (0.7.14, schema 3)

The normal Windows client records automatically in memory. After a symptom,
**Ctrl+Shift+F12** saves a unique `render-*.jsonl` to the store's `diagnostics`
directory (normally `%LOCALAPPDATA%\Tau2\diagnostics`) and shows the path.
Orderly shutdown atomically replaces **`latest.jsonl`**. Explicit saves are
retained; automatic saves do not accumulate session files. Nothing is uploaded.

The fixed rings retain **4,096 records plus 32 slow records**, using **858,624
bytes** of record storage. Slow callbacks or pending-redraw delays of at least
100 ms remain available independently of ordinary ring rollover. Hover records
are sampled at most once per 50 ms; the actual input handling is not throttled.
No per-frame IO, polling thread, extra redraw timer, GPU wait, readback or changed
present policy is introduced. Saving performs synchronous file IO.

Normal recording avoids the detailed text-input hashing/measurement walk. It
records adapter/backend/driver, viewport, frame index, focus/visibility, input
category, update/paint durations, network poll and cache-mutex wait time, wake
counts, pending-redraw-to-submission delay, input counts, batch validity/cache
occupancy, and **actual encoded draw commands**. Shape/image counts are beside
the application's `pass.draw`; text/emoji counts use Sanscale's existing
thread-local `perf-counters` feature at its actual draw sites. A segment/input
count alone is insufficient: wholly clipped or invalid text can submit no draw.
`submitted` means `Queue::submit` returned, **not GPU completion or presentation**.

No message/draft text, credentials, attachment paths or screenshots are stored.
CPU timings do not measure GPU execution. The recorder-body debug benchmark with
256 text inputs measured about 6 microseconds/record, with no shaping/preparing/
uploading by the recorder. That does not measure all instrumentation overhead;
enabling Sanscale counters and per-callback clock reads also costs CPU work.

`TAU_RENDER_TRACE=off` disables recording. An explicit output directory enables
the optional detailed mode on supported native builds; it adds handle/geometry/
layout-metric fingerprints, never message text. Normal Windows operation needs
neither option. An invalid preparation is recorded, not silently repaired.

### Background wakes and real-App regressions

A pending wake gate coalesces a burst of producer notifications into one OS
wake. It clears before mailbox draining, allowing a later/racing completion to
schedule the next wake. Picker/save/extract workers use the same gate. This
removes redundant notifications; it is **not a diagnosed fix** for the reported
freezes. Cache locking semantics are unchanged; only UI-thread wait timing was
added while a recorder exists.

- Full frontend nextest suite: **240/240 passed** (final run
  `b4e6e66c-07e6-4911-9cd9-aef1fafa24da`).
- Real-App tests drive topic navigation, network progress through the real
  mailbox, and an independent indeterminate-progress timer without WebSocket
  traffic. Idle ordinary views settle, including with an unchanged draft.
- Controlled GPU-readback tests distinguish clear-only pixels **[9,13,18,255]**
  from app-background-only pixels **[14,20,27,255]**. Both can have zero text
  commands. A mixed UI verifies shape/image/text commands. Invalid or fully
  clipped nonempty text inputs correctly record zero actual text draws.
- App-level focus return/chat-hover tests submit shapes, images and text, not
  just clear/background. These offscreen checks do not exercise Windows DWM.
- The normal Windows binary has been run under Wine with an isolated offline
  demo store, with `TAU_RENDER_TRACE` unset. Default recording, F12 saves,
  orderly latest-file replacement and real topic switching pass. Win32
  `ShowWindow`/`IsIconic` controls verify minimize/restore; 30/90-second waits
  and separated pointer bursts are captured. These are not AMD reproductions.
- Normal 0.7.14 installer built via `scripts/build-windows-sfx.sh --beta` using
  managed Cargo. `scripts/beta-release.py windows 0.7.14` verifies its embedded
  payload/launcher and that the payload matches the built client.

Current artifact hashes and working-source hashes are recorded in
`normal-client-0.7.14/windows-verification.json` under the evidence directory.
The original 0.7.13 installer is unchanged; its original executable is retained
as `windows-diagnostic/tau-0.7.13-original.exe`. No daemon was deployed/restarted.

### Clear-only and shared-stack audit

- Tau `App::frame` always builds an app-background rectangle before the UI,
  then calls `Renderer::draw`. The renderer clears, draws and submits in one
  pass. Chad calls `App::frame` before `Queue::present`; failed acquisition
  skips both. No normal present-without-app-frame branch was found.
- Compendium has no Chad runner. Its cursor-move handler unconditionally
  requests redraw, explaining the pointer-linked rhythm without establishing
  an input bug. It presents after pane/chrome rendering. If every pane is
  degenerate/clipped and chrome is empty, it **can** reach present without a
  render submission; no evidence connects that condition to the user's burst.
- wgpu 30 automatically clears a never-rendered surface texture before present.
  This fallback exists; it is not evidence that the user's bad frame took it.
- Both use winit **0.30.13**; Tau uses wgpu **30.0.1**, Compendium **30.0.0**.
  Their present policies differ (see above). Compendium constructs its instance
  **without** `with_env`; blindly setting `WGPU_BACKEND` there is not a valid
  backend A/B. Tau's Chad runner does honor that override. Captured adapter
  backend, not an assumed Windows default, must identify the active path.

Upstream review on October 7, 2026 (primary GitHub issue/PR records):

- wgpu **#7922**, AMD APU/Windows/Vulkan flicker reported July 10, 2025, was
  fixed by **#7972** on July 21, 2025. The fix gives texture/view handles
  separate identities to avoid stale framebuffer reuse. Inspection confirms
  **both current dependencies already contain that fix**. Similar symptoms
  are not justification to relabel it as our cause or blindly update wgpu.
- **#9559** reports AMD/Windows/Vulkan acquisition latency after a fence-wait
  change; it is not a demonstrated match for missing UI pixels. Do not remove
  synchronization based on this report.
- **#10548**, opened October 6, 2026, proposes DX12 resize-presentation changes;
  it targets live resize and depends on other changes. Not an established
  remedy for prolonged idle followed by event-driven flicker.

Raw primary issue records, build/test logs, scripts and isolated runtime
captures are under `/root/tau-checks/text-flicker-20261007/normal-client-0.7.14/`.

## Next diagnostic boundary

Correlate the affected laptop's bad frame with the recorder. A clean capture can
exclude observed invalid-batch events in its retained interval, but **does not
prove a GPU or presentation fault**: UI omissions, clipping and occlusion still
need examination. CPU metadata does not show the pixels actually produced or
presented. A GPU/output capture is the next boundary if input evidence is clean;
compare Windows backends/present policies only one variable at a time.

Keep the synthetic cache regression separate. Any eventual fix for that case
must protect or recover handles during draw assembly, not merely warm them after
eviction. Do not claim that continuous redraw, a larger cache, forced reshaping
everywhere or a present-mode change fixes the reported symptom without evidence.
