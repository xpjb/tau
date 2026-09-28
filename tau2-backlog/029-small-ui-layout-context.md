# 029 — Small UI/layout context over existing drawing primitives

Kind: **Small UI-helper implementation candidate.**
Design parent: **031**; overlaps registration/lifecycle work in 028 / 032.

Status: **Open; lower-priority two-form pilot.**
Priority: **After semantic/interaction boundaries are clear.**
Confidence: **6/10** in the value and appropriateness of this scope, not a probability of a LOC saving.
Net production LOC reduction estimate: **-50 to +100 lines**.

Recorded September 28, 2026. Evidence: the detailed `40a3698` investigation plus
relevant changes checked at `origin/tau2` = `cafef7f`. These are planning ranges,
not implemented or measured reductions. Positive means fewer lines; negative
means added code. Count changed production Rust across frontend, daemon and shared
crates; moving code is zero. Tests/docs/manifests are excluded unless explicitly
listed. See README for the confidence scale, baseline and overlap rules.

## Why / evidence

`frontend/src/app.rs` free button helper, settings/modal renderers and repeated Editor
drawing/focus-hit registration; `frontend/src/app/projects.rs` form layout;
`frontend/src/render.rs` layers/clipping and existing shared Editor.

Confidence rationale: Repeated style, scale, geometry and hit registration are visible,
but many calls are already compact helpers. Another wrapper can easily add code without
removing a decision.

## Proposed scope

Pilot a short-lived Ui context on two forms, borrowing renderer/layer/scale/clip/theme
and the relevant hit sink. Add only recurring row/column splitting and named control
styles. Delegate field editing to the current Editor, with drawing and hit geometry
derived together.

Out of scope: No new editor, global widget hierarchy or blanket app-wide styling
rewrite. No frame-time improvement claimed without profiling.

## Acceptance

- Preserve actual drawn/clipped geometry, focus, disabled controls, layer order and
  modal blocking at desktop/mobile sizes.
- Delete repeated field/button registration decisions in the pilot, not just replace
  Rect::new with another spelling.
- Keep typed Actions and narrow borrows; reject an untyped callback bus or whole-App
  borrow in every widget.

## Honest impact estimate

For two representative forms, 60–140 old helper/call-site lines replaced by 40–110
context/helper/call-site lines gives -50 to +100 net. The original counts of Rect/label
calls are not equivalent to deletable lines; no extrapolation to every screen is
justified.

## Dependencies, overlap and current-source notes

Hit-registration savings overlap 028. Typed fields/lifecycle overlap 032; do not credit
their boilerplate twice. The first audit ranked this too highly; it is not the selected
first implementation.

Use cafef7f shared attachment geometry and existing primitives as the starting point. No
future saving is credited for already-shared Editor or attachment rendering.

No implementation, runtime/device certification or Rust test run is part of this
backlog entry. Work only with managed Cargo compiler checks and relevant nextest
coverage when implementation is authorized. Preserve existing deferred tasks.
