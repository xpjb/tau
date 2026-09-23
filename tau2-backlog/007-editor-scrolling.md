# 007 — Repair text box scrolling

Priority: editor. Status: merged via frontend into tau2; device acceptance pending. Related: 003/004.

The user reports bad scrolling in the composer and recalls fixes in Compendium or
Sanscale examples. The old `Editor::text_origin` recomputed the entire offset from
the caret every frame; the editor had no independent viewport scroll state.

Compare prior implementations. Keep the caret visible after edits/navigation without
snapping away from user scrolling. Wheel, selection drag/autoscroll, click hit tests,
soft wrapping, resize and long settings prompts must use the same content origin
and visible clip. Avoid inheriting transcript scrolling state. Verify desktop and
Android IME/touch paths; log Sanscale difficulties under 004.

Integration: `tau2-sanscale-text-input` through `04fd60c` was fast-forwarded into
`tau2-rust-frontend`, then `tau2`. The frontend branch is retained. Included in beta 0.7.1 daemon/Windows delivery;
physical acceptance and any upstream limitations below remain open.

## Editor handoff

Shared Editor now owns its viewport independently of caret/transcript state.
Wheel/Shift+wheel, single-line horizontal scrolling and edge-drag autoscroll use
the same remembered font size, clipping and origin as drawing/hit-testing. Resize,
edit and navigation reveal the caret; idle frames retain manual scrolling. The
actual prompt settings App/GPU test checks scroll ownership, stable idle pixels
and click geometry without a premature pending-caret reveal. Physical held-drag,
touch/native EditText and DPI acceptance remain pending; see frontend/QA.md.
