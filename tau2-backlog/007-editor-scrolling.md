# 007 — Repair text box scrolling

Priority: editor. Status: open. Related: 003/004.

The user reports bad scrolling in the composer and recalls fixes in Compendium or
Sanscale examples. Current `Editor::text_origin` recomputes the entire offset from
the caret every frame; the editor has no independent viewport scroll state.

Compare prior implementations. Keep the caret visible after edits/navigation without
snapping away from user scrolling. Wheel, selection drag/autoscroll, click hit tests,
soft wrapping, resize and long settings prompts must use the same content origin
and visible clip. Avoid inheriting transcript scrolling state. Verify desktop and
Android IME/touch paths; log Sanscale difficulties under 004.
