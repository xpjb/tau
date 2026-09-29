# 041 — Transcript owns its actual children; delete the row-description layer

Status: **Selected after 037, 039 and 040.** This is a bounded rewrite of transcript
UI assembly/ownership, not networking, persistence, Markdown or shaping. Absorbs
the UI part of 016/031, cleanup 035 and reopened 012.

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
