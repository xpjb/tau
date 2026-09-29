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
