# 036 — Restore correct drawing and selection

Status: **Ready, first.** Two small independent fix commits, not another renderer
or transcript rewrite. Evidence: [reproduced audit](../docs/reviews/retained-ui/README.md).

## A. Ordered compositing

At `33f7d6f`, `App::frame` gives Root one `Layer`. Root appends workspace and
opaque overlays, but `Renderer::draw` groups **shapes → images → text per Layer**.
Earlier text therefore paints over a later dialog background.

**Important:** the existing renderer already draws multiple Layers inside **one
GPU render pass** (`render.rs`, the loop inside `begin_render_pass`). One pass is
not the bug. Scissoring clips bounds; it does not repair this ordering.

- Restore explicit ordered layer/batch boundaries in the existing renderer/API.
  Let the owner inserting an overlapping surface establish its compositing scope.
  Start with workspace/chrome/overlays; include floating controls and nested menus.
- Preserve painter order across shapes, images/icons and text. Within a batch,
  type grouping is allowed only where equivalent. No global type sort across
  surfaces, speculative render graph, per-widget offscreen texture, or blanket
  one-GPU-pass-per-widget policy.
- Define the small paint-order contract consumed by 037's frontmost-first input.
  Do not repair only Connection or confuse input blocking with visual opacity.

**Done:** actual opaque dialogs, menus, viewer chrome and floating controls cover
lower content correctly. Use one compact rendered occlusion case with mixed
text/image/shape content and a transparent/clipped boundary; inspect desktop and
phone screenshots. Adapt the audit probe into the surviving coverage, not a
second full fixture/golden-coordinate suite. An assertion that “pixels changed”
or “a dialog emitted draws” is insufficient.

## B. Selection follows newly placed text

`Transcript::handle_event` extends selection on pointer Move; Tick scrolls under
a stationary held pointer without extending it after the new text layout.

- Reconcile the held selection endpoint against the renderer's new placed text
  after scroll/layout, before copying/releasing. Keep this in Transcript, not App.
- Preserve cross-message selection, horizontal offset, clipping and pointer ownership.
- Keep one compact behavior regression: hold at an edge, advance scroll without
  another pointer move, and verify the selected/copied text advances. Do not freeze
  byte 595/805 or a particular font/layout; those were diagnostic observations.

**Done:** the existing audit reproduction passes, with new code reusable in 041.
This does not require or close backlog 012's restoration review.

## Cost / verification

Some correction code may be necessary. Charge it to the total reduction budget;
correctness cannot be traded for a smaller file. Retire the one-unordered-Layer
assumption and the Move-only selection update, not existing text/shape services.
Use managed compiler checks and targeted nextest for changed paths. No deployment.
