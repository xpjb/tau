# 038 — Finish form/chrome reuse, then remove the repeated plumbing

Status: **Selected after 037.** Absorbs the unfinished part of 029/032, and only
concrete remaining duplication from 033. Typed dialog lifetimes already exist;
do not rebuild them to fit another architecture diagram.

## Implementation slices

1. Migrate Connection and one async Topic form to the common controls. Share only
   demonstrated field-row/label, footer/button and inline-feedback placement.
   Delete their repeated placement/registration code in the same commit.
2. Migrate model/daemon settings and operation dialogs, then menu/notice/tooltip
   chrome. Keep table-driven daemon settings and feature-specific validation/data.
   Remove old helpers and repeated layout/event wrappers after their last caller.
3. Finish remaining workspace/composer/code-browser/attachment chrome consumers.
   Do not rewrite code-document/selection/picker algorithms. If preview/save
   acquisition still duplicates setup, share that setup with explicit intent;
   do not introduce an export-job framework merely to replace map declarations.

## Boundaries and deletion targets

Audit `ui/{dialogs,settings,operations,menu,notice,tooltips}.rs`, repeated
`TextField` placement, and migrated callers in header/composer/code/cards. The
prior overlay/form destination bucket was 2,640 physical production lines;
**that is an inspection area, not 2,640 deletable lines**.

- One control place/paint/event contract, no retained button followed by a second
  ad-hoc draw call. No list-building just to reconstruct fixed owned fields.
- Replace recurring row/footer calculations with a small concrete helper only
  when it deletes code in real consumers. Struct literals are fine; boilerplate
  constructors and one-call forwarding wrappers need no preservation.
- Keep small local action enums and owner-applied structural requests. Do not
  replace them with callbacks borrowing the whole App or a universal action bus.
- Preserve the existing Editor and same-window Android input. A closed dialog
  invalidates callbacks, not the durable operation it submitted.

## Done / tests

The migrated forms use common controls, geometry and compositing, with fewer
independent placement/dispatch decisions. Tab/Enter/IME, error draft preservation,
wrong-request completion and replacement/source fences still work. Modals block
background input; download notices do not evict an unsaved form. Preview stays
cache-only; Save/Extract remain explicit, source-bound and single-flight.

Retain one representative async form and one real editor integration scenario;
other forms need distinct validation cases, not copies of the same lifecycle
suite. Delete exact layout/glyph assumptions and the legacy-selector assertions
identified in 042. Review actual compact/narrow/scaled layouts visually.

Report deleted symbols/callers and net production/test counts. If the two-form
pilot only adds helpers around unchanged code, revise it before migrating more.
No global style overhaul, new form schema, data migration or release work.
