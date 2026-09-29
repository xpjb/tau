# 041 — Transcript owns its actual children; delete the row-description layer

Status: **Source ownership complete; size and physical acceptance remain open.**
This is a bounded rewrite of transcript UI assembly/ownership, not networking,
persistence, Markdown or shaping. Absorbs the UI part of 016/031, cleanup 035 and
reopened 012.

## Replacement

Transcript owns ordered keyed message/detail/attachment children and viewport /
reading policy. A message owns its text view and actual interactive descendants.
Reconcile model changes directly into those owners. Fixed field data can be
constructed literally; trivial constructors are not a required abstraction.

- Remove `app/projection.rs` and `projection::rows`. Remove `app.rs::Row` as a
  rich intermediate UI description, `MessageRow.row`, whole-row cloning and the
  description → retained wrapper → Part reconstruction loop.
- Do not move that builder into Transcript or rename it `View`, `NodeSpec`, etc.
  Model identities/body references from 039–040 are not another widget tree.
- Keep a lightweight order/height index where needed for virtualization. Bound
  live children to the viewport/overscan plus active interaction owners. Streaming
  updates affected text/children; hover must not rebuild all descriptions or
  remeasure every loaded body. Resize can legitimately invalidate widths/heights.
- Rendering/text services retain Markdown documents, shaping caches, selection
  geometry and cross-message text order. Do not rewrite the parser or invent a
  widget for each glyph/paragraph with no independent interaction.

## Reading position: bounded 012 review

Audit the owner, then consolidate follow-tail, anchor key/offset, prepend/expansion
adjustment, chat-switch save/restore and download-jump policy in Transcript's
source/chat-bound reading state. Persist only meaningful view preference; empty
loading frames cannot overwrite an established anchor, and a release cannot save
old layout under a new chat. Layout placement is disposable, not another authority.

`frontend/src/app/scroll_tests.rs` was deleted under 042 during test-adapter
retirement, rather than porting its 199-line old scenario. Still preserve necessary
navigation/anchor guarantees through
small assertions on the new owner and existing download-navigation coverage;
do not recreate the whole deleted GPU scenario. No new scroll database/schema.
Use 036's post-layout selection correction; do not resurrect an App dispatcher.

## Retire adapter-only legacy paths

Port needed demo inputs to the same native record/model path used by real data.
With 042, delete genuinely obsolete `Feed::update/page`, their controller handlers
and skipped-wire `TranscriptUpdate/Page` variants/types after a full caller audit.
Delete the old fixture-only algorithm tests rather than simulate a dead protocol.
`Feed::snapshot`/native full startup/sparse updates have live responsibilities:
replace those consumers before removal, never blanket-delete by name. Provider
history/export formats and shared Event types are not automatically obsolete.

## Done / checks

Ordinary text, streamed text, nested Details/tool output, pending→confirmed messages
and attachments all use the direct owned path, with no feature-flagged predecessor.
Reorder/prepend keeps identities; source replacement cancels old controls. Selecting,
copying, expanding, paging, following the tail and download jumps still work.
Check bounded child counts and that hover/idle do not trigger full model assembly
or reshaping; no claimed speedup without measurement.

Keep a compact native-seeded transcript integration scenario and model/renderer
checks at their real boundaries. Delete rich-Row/part-key assumptions and redundant
GPU matrices under 042. Publish the removed path inventory and net counts; if the
change is another adapter or only moves code, it is not complete.

## Rich projection removed — September 29, 2026

Deleted `app/projection.rs`, `app.rs::Row`, `details::Line`, `Tools::lines`,
`MessageRow.row`, whole-row cloning and Part reconstruction. Complete Copy is now
an explicit formatting function, not the surviving UI builder. Feed supplies IDs
and body ownership; Transcript groups only child identities and retains a lightweight
height/text-key index. Real message, Details disclosure, thinking and tool owners
are independently virtualized (a huge Details group no longer retains all its
controls). Tools directly own Input/Output disclosures/text controls and use native
children; text under an offscreen heading still requests its native root.

Renderer Markdown/shaping/selection documents remain the text owner. Model/width
changes invalidate measurements by content/availability/disclosure signature;
hover/idle does not reassemble or remeasure loaded bodies. A counter on the actual
renderer measurement call verifies zero idle measurement and bounded changed-text
work in the existing retained-identity case. A compact native handoff smoke checks
the actual control, literal authored source and canonical interest, not another
transition matrix. Selection/order, prepend, navigation and source-cancellation
coverage remain. Persisted Details keys still resolve before rebasing an anchor.

Queue-edit logical assertions now live in the existing restart/recovery test;
removed their extra GPU/Row assertion. Removed obsolete Line assertions rather
than porting them to the new controls. The real-daemon integration exposed a fixture
race: its `queue.len()==1` wait sometimes selected the previous synchronizing `hold`
row, then tried to edit that deleted request. The wait now requires the new original
request ID, complete body and no removal barrier, like the real action eligibility.
No production revision fence was weakened to make this pass.

Fresh **195/195 frontend tests**, zero skipped, pass
(`e0984339-948c-45be-afcc-b7f96f072832`), including native two-client/daemon and the
new actual handoff. Compiler checks pass. The source deletion is real, but the
replacement costs **214 normalized lines** (raw saves 224); it does not meet the
simplification gate. Legacy fixture update/page retirement, event-boundary cleanup,
remaining pruning and final cross-platform/physical acceptance remain open.

## Legacy adapter retirement — September 29, 2026

Deleted `Feed::update`, `Feed::page`, the legacy full-snapshot overlap algorithm,
all three skipped-wire Transcript variants/controller handlers and the shared
`TranscriptSnapshot` type. Provider `HistoryPage`, `TranscriptChange`, `TextDelta`
and Event remain because daemon execution/history uses them. The native View is
flat; native full-window replacement and sparse installation share the existing
Feed, with duplicate identity/order checks and unchanged root/queue cursor barriers.
The native decoder also checks metadata identity against its actual header.

Offline preview now commits ordinary native headers/bodies/directories and uses
that same native decoder/model boundary. It explicitly refuses a connected
controller. No second legacy protocol or test action bus replaces the deleted
paths. The quota network test supplies session metadata rather than injecting
an offline transcript into its connected cache. Navigation loading tests clear the
actual replica and install cumulative native windows; real cache/history tests
still exercise actual native paging/reset semantics. Thinking now has one compact
native streaming/Markdown check; the obsolete delta/page algorithm test and its
helpers are deleted. Manual long-body fixtures now publish native content rather
than being silently overwritten by real viewport hydration.

Fresh frontend **194/194**, zero skipped, pass
(`ebfa166d-d4e6-424b-8633-4095a65ef5ca`). After the final identity-validation check,
frontend/daemon/protocol all-target compilation and 35 native contract tests pass
(`db018803-8bd7-49de-83aa-acc90d494588`). Net cross-workspace saving: **462 raw /
326 normalized**, including deletion of 30 shared protocol code lines. Event
boundary cleanup, broader pruning and the final size/device gates remain open.


Synthetic lifecycle follow-up is now closed in 037: update/cancellation no longer
travel through input dispatch. Fresh frontend validation is recorded there. The
remaining reading-position, size and physical-device gates are not closed by it.


## Reading policy consolidated — September 29, 2026

The existing source/chat-bound Transcript now owns one private live reading
choice: a position (follow-tail or row anchor), a disclosure anchor, or a pending
download entry. LocalChat.position is a checkpoint, not another live controller restored
and rewritten on every paint. The existing serialized preference schema is intact.

Deleted `placed_session`, the expansion-position map and separate expansion pin,
standalone jump validation/location traversal, Workspace's old-chat save policy
and save facade, its manual scroll/layout/history resets, and timer-owned motion
cancellation. The source-bound owner checkpoints before rebinding; old geometry
cannot be stored under the newly selected account/chat. Save retains an unresolved
anchor and its intended offset across empty, partial and temporarily clamped
layouts. Explicit scroll/tail/jump transitions replace the live choice. Successful
Send requests tail explicitly rather than relying on a paint-time model reread.

Disclosure positions come from the actual retained row on demand. No global
heading registry is rebuilt; a pinned disclosure can retain its one owner through
a temporarily tiny viewport. If the control disappears, reading falls back to its
row checkpoint, rather than seeking older pages for a dead control key. Checkpoints serialize a row identity, not a transient
control key. Geometry-dependent offsets use the measurement's scale, including a
save between a DPI change and its first reflow. Existing selection/autoscroll,
wheel interpolation, overscan, paging/error guards and source fences remain.

The native cache's existing saved-anchor hydration now also recognizes direct
Tool/Thinking row keys by comparison with known native headers. This preserves
cold offline body hydration without parsing provider IDs, changing schemas or
adding another replica path.

### Evidence and validation

The [baseline regression patch](../docs/reviews/retained-ui/reading-regressions.patch)
applies to `bdfbec3`. Run the named tests with managed Cargo/nextest in an isolated
worktree; no live data or OS file actions are needed:

- Existing same-chat download navigation: **83 measurement calls** before the
  change, versus the retained-window bound **<35** afterward on desktop, phone and
  scaled-phone headless scenarios. Failing-before run
  `6addc27b-746d-49a8-9669-42ece78f3a53`.
- One checkpoint case reuses the navigation fixture for late pointer release,
  cold/partial windows (including an anchor-only clamped window), prepend, DPI and
  pinned expansion during streaming. Before the change an interim page replaced
  saved `demo-event-29` with `demo-event-50`
  (`33f8ad8e-48a6-4c47-8eca-39afd29b5f5d`). Existing selection/copy and native-interest
  tests remain; the retired scroll_tests.rs scenario was not resurrected.
- The existing disk hydration/recency test failed on the saved tool-row key before
  the cache match was fixed (`50e64137-8d9d-4416-ae83-82055c6681db`). It now checks
  text, Details, Tool and Thinking keys while retaining the byte/eviction assertions.

Final fresh workspace all-target check and **325/325 workspace tests across 19
binaries**, zero skipped, pass (`0f842d26-834d-433a-b9e7-f9a173b04277`, 100.265s).
Windows MSVC and Android ARM64/API29 library checks and frontend rustdoc pass.
No Clippy/built-in Cargo test runner, Java changes, physical-device QA, integration
merge, package build, deployment, restart or live-data change.

Net production: **−17 raw / +12 normalized**; tests **+80 / +107**. The necessary
replacement/checks make this slice grow **53 / 106** overall. This closes the
reading-ownership review, not the whole-refactor reduction target. The shared size
ledger records the remaining gap; 013/014 and physical acceptance stay separate.
