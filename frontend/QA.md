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
