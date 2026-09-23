# 006 — Check caret response speed

Priority: editor. Status: needs measurement. Related: 003/004.

The user suspects slow caret movement but is uncertain. Measure key-repeat input,
redraw/wake latency, shaping work and blink reset in the composer and prompt editor.
Compare the Sanscale editor example; preserve on-demand idle rendering. Do not add
caret smoothing or a polling timer to mask missed redraws.

Acceptance: distinguish key repeat delay from draw latency, verify immediate caret
feedback and solid caret after input, record measurements and remaining device QA.

Ownership: the user assigned editor work to another worktree. No editor or Sanscale
implementation was started by the settings worktree. Coordinate there before editing.
