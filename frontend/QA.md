# Topics — 0.7.2 beta acceptance (protocol 14)

Topic-label and layout follow-up: tabs are shorter and less padded; desktop/mobile
native UI tests cover restoring each topic’s last-open chat, first-visit fallback,
local SQLite persistence, and stale selections after delete/move. Wire fields and
SQLite table names stay `projectId`/`projects` for compatibility. The reported
intermittent black frame has no reproducible trigger yet and is **not claimed fixed**.

- Managed workspace all-target compiler check and nextest: **69/69 tests passed**.
  Feature validation used isolated databases/providers with no paid provider calls.
  Release builds and beta deployment were checked separately below.
- Actual controller/transport against the native daemon with two clients covers
  topic creation, selection, per-topic starters, cross-client edits/conflicts,
  restart, unread aggregation, moves without losing drafts/files, and both delete
  choices. Cancel is exercised through actual native UI hit testing.
- Scripted Codex and Chat Completions provider requests verify exact captured topic
  prompt text, edits leaving old chats unchanged, new chats receiving edits, direct
  system-prompt replacement on moves, and clone/restart persistence. A gated tool run
  verifies moving/editing does not alter its continuation, but the next user turn
  receives the destination snapshot. Deletion cancels running work; an injected SQL
  failure rolls back the complete topic deletion and leaves transcripts intact.
- A real version-1 SQLite fixture migrates to General without changing old history,
  activity, revisions or instructions; starters are unique per topic afterward.
- Headless native GPU rendering at 1000×800 and 360×720 exercises clipped tabs,
  wheel/trackpad-axis routing, touch dragging and hold, selected-tab reveal, read dots,
  target-bound nested context menus, keyboard/submenu scrolling through 25 topics,
  exact-text prompt editing and the two-stage delete dialog. Rendered desktop/phone
  frames were inspected. This is not physical Windows or Android device acceptance.

Windows x64 and Android ARM64 release builds and package verification passed.
The isolated beta was deployed and its live protocol-14 topic operations verified,
with a consistent pre-migration backup and unchanged original history. Stable Tau
was untouched. See `INTEGRATION.md` and `PACKAGING.md` for release evidence. Physical
touch/IME and DirectX/DPI acceptance remain unclaimed. Protocol 14 requires matched
clients; SQLite schema 2 must not be opened by the old daemon.

---

# Connection status follow-up

The connection card omits the redundant Connected title. It shows min/max
acknowledged Ping/Pong RTT across the last ten *attempts* and a live `received:`
or `waiting:` millisecond counter; explicit state remains for connection failure.
Failed attempts take a slot but never fabricate an RTT. No server URL appears;
that belongs in Settings. The last reply age and samples survive automatic
reconnects to the same server, but changing settings resets them. Pong receipt
is timestamped on the network thread, not when UI events are drained.

Probes run every 2s with a separate 5s timeout and no session-list requests.
The visible counter redraws every 50ms; with the card hidden, at most three
threshold wakes update the dot while a ping is pending. Latest RTT/pending wait
is green through 250ms, yellow through 1000ms, orange through 3000ms and red
above; no reply yet isn't green, and a lost/unconfigured connection is red.
Late/unmatched pongs are ignored. The scripted socket and headless GPU tests
cover these states. `tau --screenshot PATH --connection-preview
[received|waiting|disconnected|unconfigured]` renders mocked cards without a
network account. Chat rows keep their last known worker state and unread dot;
the Tau connection dot remains solid.

The older acceptance notes below describe the original 20s diagnostic design.

# 0.7.0 integrated beta acceptance

The user explicitly authorized native integration, cleanup, remote `tau2`, a separate
beta daemon port and Windows/Android builds. Earlier release holds below are history,
not the current authorization. Stable service/data/routes remain separate.

## Current evidence

- Managed workspace all-target check passed; nextest **49/49 passed**, one Cargo job,
  wrapper-limited test concurrency. No new trivial UI/API-wrapper tests.
- Real frontend controller/transport ↔ native daemon ↔ gated local provider, tools,
  SQLite, upload/transfer, settings CAS, history/fork and restart acceptance passed.
- SIGKILL/WAL recovery preserves queue edits/deletes/control receipts and does not
  automatically repeat interrupted tools or billed compaction. Native title and
  cross-provider image/reference/export paths have deterministic provider coverage.
- Real desktop GPU render exercised the per-corner WGSL pipeline successfully.
- A private Xvfb/native-client/native-daemon run edited the actual settings UI:
  built-in/null → custom intentionally empty prompt saved correctly; another native
  client advanced the document revision; the stale UI save was rejected without
  overwriting the newer document. Desktop and narrow settings layouts were captured.
  That review caught and fixed a duplicate global notice overlay in this modal.
- All fixture processes were this task's own and were stopped. No production history
  was imported or changed; no paid provider completion was used for these checks.
- Release packaging/deployment evidence is recorded in `INTEGRATION.md` and
  `PACKAGING.md`, not inferred from debug binaries.

## Device acceptance still required

Physical Windows/DirectX/DPI and Android touch/IME/font acceptance remain device QA;
Linux/Vulkan or previous Wine checks do not establish those. In particular exercise
clipped hover/long-press boundaries, DST formatting, clipboard selection, scroll
anchors on older pages, cache proxy labels, model tile scrolling/landscape, rapid
send during model selection, and drafts/files across connection loss. Real provider
completions were not billed just to claim a live smoke test.

New native sections persist their first observation timestamps. Imported historical
Pi sections retain the timestamps actually available; no historical sub-block times
are fabricated. The variable-font/synthetic-style limitation recorded below remains.

## Implemented

- Independent top timestamps on each logical bubble, including Details, text,
  pending/queued sends and native sections. Local time, date + seconds, using
  source event milliseconds/RFC3339 (first contributing event for Details).
  Local pending creation times are saved once; old local records remain readable.
  Section times stay fixed while their content changes. Missing source
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
- No worker-TTL protocol was added. Native runtime idle eviction is independent;
  it releases idle runtimes with paused work without losing the durable queue.
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


## Shared editor / Sanscale migration — `tau2-sanscale-text-input`

Isolated worktree `/root/tau2-text-input`, based on the completed settings handoff
`446ac29`. SDK pinned to `8cc5afe833176a4fc71d1e5b8b97ad4952adfe40` in one workspace
dependency. The implementation task made no merge, deployment, production account
access or daemon restart. It was subsequently fast-forwarded through the maintained
frontend branch into `tau2`; see MERGE.md. Integration changed documentation only
and did not rerun the suite or convert pending device checks into passes.

Completed on the implementation retained in this branch:
- Workspace/all-targets check; **64 nextest tests passed**, including 13 new editor
  tests with real fonts and three actual App/SQLite/headless-GPU tests.
- Scoped frontend Clippy, all targets, `--no-deps -- -D warnings`, passed. Workspace
  dependency linting still hits the existing Markdown lints (previously recorded
  as `896a973b-1da6-4680-b894-c147eeb7d6fc`); no unrelated lint cleanup.
- A SQLite trigger catches redundant same-value draft writes: navigation/copy
  produce zero writes; cut writes once. Warm navigation produces zero shape,
  flow or block requests. Selection visibly changes GPU pixels; repeated frames
  are byte-identical. The actual new default-prompt settings UI exercises click
  geometry, clipboard, composition/commit/undo and independent scrolling.
- An observed debug run of 300 warm App Up/Down keys: p50 770ns, p95 800ns;
  one completed headless frame plus readback 2.61ms. These are NOT physical
  key-to-display or OS autorepeat latency measurements. Idle tick stays idle;
  no blink/polling/smoothing timer was added. Chad forwards keyboard events and
  the desktop adapter explicitly requests redraw after dirty input.

At the user's finish request, a repeat check encountered a cold shared build and
was stopped during dependency compilation. The late, unvalidated IME clause-highlight
polish/test was removed, retaining the implementation from the completed 64-test
run. Android cross-check stopped in dependency compilation (`regex-automata`),
before checking this frontend. Do not count either attempt as a pass. The attempted
private X11 input run supplied no completed result; no native GUI acceptance is claimed.

Remaining: physical Windows/DirectX/DPI and Android/native-IME acceptance, OS
composition/focus ordering, actual autorepeat-to-display measurements and held-drag
input on devices. Android's existing bridge sends full text, not native caret or
composition ranges. SDK intra-ligature caret granularity remains documented in 004.


## Beta 0.7.1 Windows delivery

At the user's request, built the release daemon and Windows x64 native app,
launcher and self-extracting installer sequentially from `6cacd15`. Verified the
compressed app payload matches the fresh executable; posted the approximately
9.4 MiB installer. Beta health reports 0.7.1/protocol 13 after its authorized
restart; stable's process/start/executable were unchanged. No backup or activity
check, as explicitly requested. Full checksums and deployment evidence are in
../INTEGRATION.md. No new Android package, test-suite rerun or physical GUI/input
acceptance is claimed by this delivery.
