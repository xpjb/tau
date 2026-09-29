# 038 — Finish form/chrome reuse, then remove the repeated plumbing

Status: **Source-complete; physical device acceptance remains open.** Absorbs the unfinished part of 029/032, and only
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

## Implementation — September 29, 2026

Connection and async Topic now share labelled-field placement and row/stack button
geometry; removed Connection's separate floating-label shaping/cutout path. The
same helpers serve operation dialogs. Models, daemon settings and Connection tools
share page/title/footer geometry. Feedback is measured and reserved above footers,
not painted over fields. Compact Topic forms use rows instead of a tall button
stack and share scarce field space when the IME is up. Existing Editor, validation,
request IDs, source lifetimes and owner-applied structural requests remain intact.

Removed Form's primary/destructive boolean pair in favor of explicit ButtonStyle,
fixed-field Vec reconstruction in event dispatch, and the operation dialog's
choice-list clone followed by lookup. Model suggestions use the existing typed
Controls rather than another retain/find/dispatch loop. Hidden model-search and
boolean-setting fields are excluded from focus/target traversal. Busy actions stay
visible but disabled; the common control consumes them without activation.

Visual inspection caught a real feedback gap: deliberately submitting a form
offline was classified like background transport noise, so no error was visible.
Form submissions now report their errors inline; background transport behavior is
unchanged. The compact real-editor scenario asserts the offline error and both
preserved drafts, then cancels through the actual button. Desktop/phone Connection,
Topic and compact error renders were inspected. No physical device QA is claimed.

Menu/notice/tooltip and remaining chrome were audited: their shared control,
clip/feedback and compositor migration was already done in 036–037. Distinct
popup deadlines, submenu placement, code selection and export acquisition remain
with their existing owners; no new chrome/form/export framework was warranted.

Deleted the obsolete legacy-selector assertion, a duplicate form replacement
scenario, and the daemon "completion" test that never submitted a request (its
fresh form had no saving token, so it could not exercise the claimed fence).
Distinct async Topic completion, actual IME/paste, settings parsing, validation,
export/source and Extract single-flight coverage survives.

Validation: the two-form pilot passed 19 targeted checks. Final frontend
all-target compiler check and **196/196 frontend tests across 10 binaries**, zero
skipped (`8831524a-f4a0-4ac8-8688-0e0b53e27c56`, 77.279s). Net reduction from 037:
production **55 raw / 67 same-format**, tests **14 / 24**, total **69 / 91**.
The pilot's normalized saving was only 27 total lines; measured reuse, not the
much larger raw call-formatting delta, justified migration. Rewritten form files
were formatted before final counting. Cumulative reduction is **313 / 289**;
this is deliberately not represented as satisfying the 5,000-line gate.


## Attachment registry/routing deletion — September 29, 2026

Deleted the whole `CardDeck` registry, its compound source/session/surface key,
separate seen-set lifecycle, event dispatcher and copied hint traversal. The
source-bound AttachmentBrowser owns its actual mounted cards directly. Input goes
to those cards; hints use their existing Controls; unmounted cards detach by their
actual geometry. Source/session changes still discard the entire bound pane.

Deleted the per-button destination envelopes: Acquire/Save/UseSaved/Cancel read
the mounted card's existing DownloadTarget and file metadata. There is no second
chat/entry/name copy to route. A changed native file remounts the card's controls,
so a held gesture cannot activate replacement metadata; an unchanged repaint
retains its control identity. Existing capture coverage now checks both cases.
No save/export/extract state machine, acquisition policy or feature was removed.

Fresh all-target frontend check and **194/194 frontend tests**, zero skipped, pass
(`ba5ce64f-5ed7-4656-b4b9-66680fa06c98`). Windows MSVC and Android ARM64/API29 library
checks pass. Physical device QA is still open. Net production saving is **64 raw /
64 normalized**; retained test changes cost **11 / 25**, for **53 / 39** overall.
This is a modest complete-layer deletion, not a claim to have closed the size gap.
