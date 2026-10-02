# Consistent scrolling and touch momentum — October 2, 2026

Feature branch: `fix/tau2-scroll-momentum`, worktree
`/root/tau2-scroll-momentum`, based on `origin/tau2` at `05879ef`.
Regression commit `b3578c7`; implementation and added coverage `60f2833`.
At the user's request, merged as **`ae2714d`** into `tau2-integration` / `origin/tau2`.
The merge tree exactly matches validated feature tip `29150bf`.
**Source integration only:** no beta deployment, packages, service restart,
production-data access, dependency/version/protocol/schema change, or stable change.

## What was wrong

The retained scroll owner already had inertia, but it used the last finger delta
and event interval, capped velocity at 3000 logical pixels/second, and decayed
velocity with a 111ms time constant. Its displacement-based stop condition killed
a freshly released fling on a zero-time update and could stop small high-refresh
steps. A tiny final movement could also dominate the entire release estimate.
Touch slop was reapplied even after capture, freezing motion when a drag returned
near its starting point.

Files and picker previews applied wheel deltas directly, bypassing the transcript's
65ms wheel filter. Composer/settings/dialog fields had independent immediate
wheel/touch scrolling and no released-finger inertia. The desktop adapter also
threw away the distinction between discrete and precision scrolling.

## Prior art and behavior

The implementation uses established physics rather than copying a whole UI:

- [Flutter 3.35.3 ClampingScrollSimulation](https://github.com/flutter/flutter/blob/3.35.3/packages/flutter/lib/src/widgets/scroll_simulation.dart)
  supplies the reference Android default-friction distance law and restart-safe
  ballistic deceleration. The Rust implementation integrates incremental
  displacement analytically, so layout/reading-anchor shifts do not reset it.
  Default friction is 0.015; gestures below 50 logical pixels/second do not fling,
  and release speed is capped at 8000. There is no overscroll bounce.
- Velocity is estimated by least squares over up to twenty samples from the last
  100ms, rather than the final event alone. Pauses and deliberate reversals retire
  stale estimates; a tiny lift-off move does not erase a clean swipe.
- [VS Code's scrollbar input/classifier](https://github.com/microsoft/vscode/blob/main/src/vs/base/browser/ui/scrollbar/scrollableElement.ts)
  is the reference for distinguishing notches from continuous input, rather than
  applying wheel smoothing to every device. Tau retains its existing 65ms easing
  and 48-logical-pixel notch units, with accumulated bursts and immediate reversal
  of a pending target.
- Pixel deltas are already physical pixels and pass through directly, including
  platform-delivered momentum. No second synthetic touchpad fling is layered on.
  On Windows, [Winit 0.30.13's adapter](https://github.com/rust-windowing/winit/blob/v0.30.13/src/platform_impl/windows/event_loop.rs)
  reports `WM_MOUSEWHEEL` as line deltas even for high-resolution devices. Fractional
  or two-axis line input is therefore treated as precision input, with a 200ms
  burst classification window. This is a bounded heuristic, not device detection;
  high-resolution mice receive the same direct precision treatment. See also
  [Microsoft's precision-touchpad guidance](https://learn.microsoft.com/en-us/windows/compatibility/precision-touchpad-devices).

One `ScrollMotion` serves transcript, chat list, topic strip, Attachments, menus,
Files, picker result/preview axes and shared text inputs. Text fields retain their
own em-space viewport, shaped geometry, selection, caret-follow and IME fences;
only their scrolling physics is shared. Their mounted owners advance released
flings even when there is no pointer capture. Paint's temporary `hide()` is not
an unmount/cancellation boundary.

Touch slop is only an acquisition threshold. Two-axis views share direction
locking, including horizontal preview gestures. A touch catches moving content
immediately without activating a moving child button. New gestures, wheel input,
explicit cancellation, navigation, modals, selection and actual editor changes
retire the relevant motion. Claimed selections are not stolen. Reflow preserves
transcript reading anchors and fling displacement; reaching the bottom restores
tail-follow. Existing image-viewer zoom/pan controls are not treated as list scroll.

Animations request their first, continuing and terminal on-demand frames, then
return to idle. No timer, continuously running render loop or additional runtime
was introduced.

## Validation

All Rust work used managed `/usr/local/bin/cargo`, single build/Rayon worker and
nextest; no Clippy, built-in Cargo test runner or shared-lock bypass.

- Both initial regressions **fail before the fix**, nextest run
  `8d68e500-0981-4638-8c00-16e123cfb69c`: a zero-time frame kills touch momentum,
  and Files bypasses smooth wheel scrolling.
- Native frontend/all-target check: **passed**.
- Final complete frontend nextest run: **243/243 passed, zero skipped**, thirteen
  binaries, run `5894ed58-5cf5-47a6-a20f-5cd515290a77`. Includes the existing real
  remote-file, navigation, IME, selection, storage and network-pressure suites.
- Focused input/physics coverage: **16/16 passed**, run
  `174a66f8-44e3-4b79-a30b-898477450fc7`. The full run includes these same cases.
- Physics tests check the independent Android reference distance for a
  1000-logical-pixel/second fling (194.31 pixels), at 30/60/120/240Hz and 1x/2.5x;
  zero-time/stalled frames, jitter, pause, reversal, speed limits, bounds, wheel
  accumulation/reversal, exact precision deltas and settled motion.
- Real App/GPU tests check visible released-finger movement, streaming-anchor
  retention, stopping on touch/cancel, nested-button promotion and catch-without-
  activation, horizontal file/preview flings, composer/dialog animation without
  capture or draft writes, and unchanged IME identity. Existing selection/clipboard
  and tail-follow regressions remain passing.
- Android ARM64/API29 NDK frontend-library check: **passed**.
- Windows x64 MSVC frontend-library check via managed xwin: **passed**.
- Merged native frontend/all-target check and **18/18 focused nextest cases**:
  **passed**, run `7c8b6cc0-10c9-48ae-9eb6-42de5e396a9b`. The full 243-case and
  platform runs above are reused for identical source, not claimed as rerun.
- `git diff --check`: **passed**. Dead-code/platform warnings remain; no unrelated
  lint-driven rewrites.

Logs: `/tmp/tau2-scroll-momentum-{before,check,input-focus,tests,android,windows}.log`
and `/tmp/tau2-scroll-momentum-merged-{check,tests}.log`.

## Acceptance still needed

This is headless real-Rust/GPU behavior plus platform compilation, **not** a
physical-phone or Windows touchpad feel/latency measurement. A client release is
still needed. On the released candidate, check slow drags, fast/repeated/reversed
flicks, pause-before-lift and tap-to-stop on Android; Windows precision-touchpad
and ordinary wheel at different DPI; chat/tabs/Attachments/Files/preview/editors;
selection and keyboard interruption; live reflow, tail-follow and idle CPU after
settling. Devices that do not deliver native touchpad momentum do not gain an
invented duplicate tail from this change.
