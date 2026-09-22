# Streaming section QA — release on hold

Unreleased work after `eacaab4` / the shipped 0.6.3 packages. User resumed
streaming notes; no new packaging, version bump or artifact delivery until their
next signal. Continue keeping build/test/verification resource use low.

## Implemented

- Independent top timestamps on each logical bubble, including Details, text,
  pending/queued sends and extension sections. Local time, date + seconds, using
  source event milliseconds/RFC3339 (first contributing event for Details).
  Local pending creation times are saved once; old local records remain readable.
  Extension section times stay fixed while their content changes. Missing source
  time says “Time unavailable”; history is never dated with its arrival/render time.
- Same-sender neighbors touch with a faint inset divider, instead of a 12dp gap.
  Only the outside of the visual group is rounded; logical IDs, disclosure state,
  selection and copy boundaries stay independent. Different senders retain spacing.
- Whole-section hover/press tint, including header/timestamp/padding/nested panels,
  clipped to the actual rounded section and transcript viewport. Text/photos are
  not washed out. Hit testing uses the same per-corner geometry and half-open
  shared edges. Pointer transitions between otherwise non-interactive sections
  request redraw without continuous polling.
- Context menus keep their target section highlighted by stable key during
  streaming/reflow, not whatever later occupies the original pointer coordinates.
  Details “Copy message” includes its thinking/tool input/output even if collapsed,
  but excludes the neighboring answer. The copy text is only assembled on demand.
- Paragraph color matches Tau 1's muted onSurfaceVariant #B7C2CE, correctly
  converted from sRGB to linear GPU color. Links/code retain their distinct colors.
- Markdown block spacing reduced from 12.8 to 8dp at normal 16dp text size; no
  trailing paragraph gap after the final block. Explicit authored line breaks and
  line leading are unchanged. Empty/hidden no-op events no longer split Details.

- Removed the duplicate bottom-left connection label and its reserved space.
  The dot beside Tau now supports hover and click/tap-to-pin diagnostics: endpoint
  origin (no token/path/query), TLS, real application-heartbeat RTT, last-eight
  average/range, last reply, connected-since and reconnect count. It reuses the
  existing 20s probe; no extra traffic, guessed quality grade, loss or speed claim.
  Epoch changes clear samples; late-epoch results are ignored. Heartbeat timeout
  gets a specific reconnect reason. Absolute local timestamps need no ticking UI.
- Tooltip hover bridges cross the anchor/card gap; cards intercept clicks instead
  of activating underlying content. Sidebar labels now clip to the list viewport.

- Restored Stop's always-visible tonal circle, 40dp like Tau 1, centered in the
  56dp header. Mobile Back uses the same size/center; title/status no longer reserve
  an overflow slot. Removed the chat-actions button and its modal entirely.
- Chat actions now use the contextual menu on a sidebar chat or its title/header
  (also empty transcript background), with touch-and-hold support. Message menus
  remain independent. Rename/delete confirmations retain the clicked session ID;
  right-clicking another chat does not select it or retarget the operation to the
  active chat. Target highlight follows the stable ID during list reordering.
  Touch hold duration no longer resets on sub-threshold finger movement.

- Chat-list rings are now a **one-hour provider-cache estimate**, per the user's
  clarification, not worker-idle deadlines. Existing transcript `timestampMs` or
  RFC3339 `timestamp` supplies the latest received assistant reply time (including
  thinking/tool calls, excluding local tool results, errors and hidden entries).
  Unloaded chats may use existing `updatedAt` as a clearly labeled activity proxy;
  its tooltip notes that metadata changes can affect it. Fresh empty chats and
  missing/unusable/future timestamps show unknown rather than an invented deadline.
  Reconnect/history receipt and heartbeats do not renew source timestamps; worker
  running/sleeping state does not freeze or empty the estimate. No extra history
  requests, probes or render timer. Minute-sized rings use bounded, distinct
  texture variants and clip to the list. The one-hour assumption is **not** a
  confirmed OpenAI/ChatGPT/OpenRouter cache lifetime.
- Removed the mistaken daemon/protocol worker-TTL additions completely; the
  existing daemon is sufficient. The unrelated worker-sleep retry flag remains
  a separate backend follow-up, not a requirement for these rings.
- Untouched starter chats expose responsive, scrollable model tiles until the
  first pending/queued/actual conversation turn. Draft text/files are preserved.
  Settings → Quick model selection edits only the per-account tile list, searches
  the daemon catalog, adds/removes choices, restores presets, or disables tiles.
  Presets include Codex GPT-6 Luna/Sol/Astra and OpenRouter DeepSeek v4.1 Flash;
  **none is forced as a default**. New chats retain the existing last-chosen-model
  behavior via the daemon's persistent `/model` command. Removed the fixed default,
  `*` syntax, automatic startup selection and pending-default send gate; a legacy
  saved `default` field is ignored. Explicit selection still gates send until
  resolved, never overwrites drafts/files, never optimistically marks a new model
  selected, and is never replayed after reconnect. Selecting an already-active
  tile still persists that choice, since another chat may have changed the default.
  Only unambiguous real built-in catalog entries can be sent; missing choices are
  disabled. Editing/resetting the tile list alone never changes the selected model.

## Checks / limits

Managed `cargo check --locked -p tau-frontend --lib` passed with one Cargo job;
formatting/diff checks passed. No GUI/emulator session, release build, full suite
or new trivial API test was started. Per-corner WGSL pipeline/rendering, hover at
clipped group ends, timestamps/DST, clipboard boundaries and scroll anchoring still
need device acceptance before delivery. Connection hover/tap, timeout/reconnect
and RTT display also await device acceptance; the incremental library check for
this change took 1.28s (one managed Cargo job). Header/context-menu library check
also passed (0.78s); GUI/touch acceptance remains deferred.
The earlier TTL/model iteration passed frontend and daemon library checks
(1.18s / 8.00s, then 0.71s frontend); its worker-TTL backend wiring is now removed.
One Cargo attempt exited 75 (shared build busy); work continued on source rather
than contending. The corrected frontend passed a managed single-job library check
(1.64s; final check 0.67s). No full suites, GUI/emulators, release binaries or new
trivial API tests.

**Before delivery:** verify header centering and right-click/long-press targets;
cache-ring aging across worker state changes, reconnect, offline, old history and
missing timestamps; proxy-vs-reply tooltip labels; model tile scrolling/settings on
narrow/landscape screens; last-chosen model reuse, unchanged choice after editing
presets, preserved drafts/files, rapid send during selection, no replay after
connection loss, and hiding after first turn. Model availability must come from
the user's daemon catalog, not be assumed from preset text.

No daemon deployment, protocol change, version bump or package delivery is needed
for this correction or was performed. Releases remain on hold.

**Timing source limitation (user accepted; deferred to backend integration):** Pi/daemon currently clones an entry/message timestamp
onto its content blocks. Details and text within that same entry can therefore
show the same source time; exact distinct historical block-start times need
upstream capture/persistence. Do not claim those times were reconstructed or
fabricate them from receipt time. Follow-up: `d9fa2e39-2f9f-4ce8-a55e-e91a27b6b0d3`.

---

# QA build 0.6.3 — packaging

The user requested optimized, directly installable Android/Windows packages.
0.6.3 uses system fonts on both shipped targets, compresses and strips Android's
native library, and omits unneeded font assets/licenses from these packages.
Windows uses GDI to select/read Segoe UI and Consolas (OS substitutions allowed).
Android prefers installed static Roboto style files, otherwise uses the API-29
font matcher for sans/monospace and script fallback, honoring collection indices.
Font-file bytes are shared across chains instead of re-reading TTC collections.
Linux development builds retain bundled fallbacks for machines without fonts.

## Packaging checks completed

- Windows x64 and Android ARM64 release builds succeeded, sequentially, one Cargo
  job each. APK Java tools used one active processor. No GUI/emulator was started.
- Android v3 signing verified, with the same certificate as 0.6.2. versionCode 4,
  versionName 0.6.3-beta; not debuggable; extractNativeLibs=true.
- Verified ZIP compression/CRC, stripped static symbol tables, unchanged dynamic
  symbols, and 16KiB ELF load alignment. The original unstripped library remains
  local for symbolication; the shared Cargo artifact is never modified in place.
- Verified all eight previously bundled TTF payloads are absent from both native
  binaries. Windows payload contains only the expected application executable;
  its hash matches the fresh build. No PDBs, JVM or font assets in the installer.
- Windows PE has no COFF symbol table to remove. Preset-6 LZMA reduces the download
  without a large extreme-compression dictionary or an executable runtime packer.
- See [PACKAGING.md](PACKAGING.md) for actual sizes and installed-footprint caveats.

## Deferred

The extra native Clippy invocation was blocked by the shared build lock (exit 75);
it was not bypassed or retried while the host was busy. No full suite, Android
x86_64 build, or new physical-device/font rendering check was run.

The pinned Sanscale API has no variation-axis or synthetic-style controls. Static
Roboto style files are preferred where present; devices with only variable faces
or regular-only monospace fonts still need typography QA. Follow-up recorded as
`df089cfc-e80f-4801-875a-b722173408e8`.

Further builds/deliveries remain on request; this does not authorize a stable
cutover, daemon deployment, or GitHub release.

---

# QA build 0.6.2

The user explicitly requested both Windows and Android builds after `773e002`,
lifting the hold for this QA delivery. This is not a stable-client cutover or a
request to publish a GitHub release. Further deliveries remain on request.

Keep resource use low: sequential builds, `CARGO_BUILD_JOBS=1`, limited Java
processor count, and `TAU_LZMA_PRESET=3` for Windows packaging. No GUI/emulator
sessions or full regression run are being started for this delivery.

## Implemented

- Windows-style middle-button autoscroll: quick click latches, held press ends on
  release; dead zone and speed match Tau 1. Escape/click/wheel/focus loss cancel it.
- Wheel input accumulates a target, followed by a frame-rate-independent one-pole
  filter (65ms time constant). Idle rendering remains on-demand.
- Draggable right-hand transcript scrollbar, track paging, and sidebar scrollbar.
- Earlier history loads near the top automatically; no “Load earlier” button.
  Pages retain the visible anchor, including while a wheel animation is active.
- “Details”, with 12dp thinking/tool text and headers, 11dp section labels, versus
  16dp main chat text. Tool calls/results pair by call ID. Individual tools start
  collapsed; large Input/Output/Error sections also collapse like Tau 1.
- Clicking the same disclosure header opens/closes it. No bottom Collapse button.
  Expansion preferences survive parent collapse and are saved per chat. The clicked
  header is pinned during reflow, within scroll bounds.
- Copy/Fork/queue/local-message actions move to a right-click/long-press context
  menu. File transfer controls remain on attachment cards, as in Tau 1.
- Tau 1's exact send/paperclip/stop vector paths, centered at its icon sizes.
  Empty/offline send is disabled. Icons rasterize once per size/state, not per frame.
- Tau 1's context usage ring and formatted tooltip values. The opaque tooltip
  expands in X and Y (160ms), rather than fading; supports hover and click pinning.
- Selection starts/continues in transcript whitespace, spans multiple messages,
  scrolls at the edges, and copies projected text. No floating Copy button.
- Editing shortcuts prefer the layout's Latin shortcut, then physical keys for
  non-Latin/named-key layouts. This also fixes Wine's Ctrl+C mapping to a named
  volume key without produced text.

## Verification already completed before the resource pause

No new trivial input/API tests. Used the Windows development executable under
Wine/Vulkan against an isolated real taud with a temporary deterministic fixture;
no production service, stable app, or release installer was changed.

- Clippy (frontend, all targets, warnings denied) and Windows cross-build passed.
- Typed/sent a message; expanded Details and a tool, re-collapsed using the original
  header, confirmed the nested preference remained saved.
- Observed smaller thinking/tool labels and code, hidden tool outputs, red error
  tool header, context menus without a message action strip, and the new icons.
- Hovered/pinned the usage tooltip and confirmed formatted usage/capacity.
- Latched middle-click autoscroll, moved upward, canceled with Escape.
- Recorded a wheel burst at 60Hz: scrollbar Y decayed
  `459 → 452 → 447 → 444 → 442 → 441 → 440 → 439 → 438`, then stayed still.
- Selected from the blank left margin to the right margin across three messages;
  context-menu copy/paste retained all 613 characters. Ctrl+C separately copied
  a shorter selection after the keyboard-layout fallback correction.
- Dragged the scrollbar upward repeatedly across a 160-message fixture. Automatic
  50-event paging reached anchors History 116, 066, 016, then 000; no load button.
- Stopped this task's Wine, fixture daemon and private Xvfb after the resource request.

## Deferred verification / release gate

- A small saved-anchor guard was added during the final static review after builds
  stopped: an empty/unsynchronized frame must not overwrite a saved scroll position.
  Both delivered targets now compile with it; restart restoration, especially an
  anchor on an older page, still needs a manual check.
- Exercise held-middle release, focus-loss cancellation, section-level large output
  toggles, partial tooltip-animation frames, sidebar scroll and touch long-press.
- Rerun the existing suite, both Android builds, and physical Windows/DirectX/DPI
  acceptance when resources permit. Do not infer physical Windows acceptance from Wine.
- Check mixed empty/hidden transcript blocks and tools straddling page boundaries
  against Tau 1 before declaring full presentation parity.
- The current request authorizes this beta build delivery; broader checks above
  remain deferred, not silently marked passed. Physical Windows and Android
  acceptance still depends on device QA.

## 0.6.2 delivery checks

Windows x64 installer and Android ARM64 APK built sequentially with one Cargo job.
APK versionCode is 3; signing and 16KiB ZIP alignment checks passed. Packaged native
payloads were hash-checked against the freshly built executable/library. No new GUI
session, emulator, full test suite, or Android x86_64 build was started for delivery.
