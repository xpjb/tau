# 003 — Complete shared text input navigation

Priority: next editor work. Status: merged via frontend into tau2; device acceptance pending. Depends on 004; coordinate 006/007.

Reported in the running Tau 2 composer: Left/Right inconsistently stop at line
starts or fail to cross lines; Ctrl+Left/Right, Ctrl+Backspace and Up/Down do not
work. The old `App::key` handled only byte/grapheme Left/Right, Home/End and simple
delete. The old editor stored a byte cursor but no visual-line affinity or goal column.

Use Sanscale's placed Caret, Motion and measured layout as in `examples/editor.rs`
and `examples/code-editor.rs`. Inspect Compendium's controller without overwriting
its independent uncommitted work. Reuse one Editor for composer/settings/dialogs.

Acceptance: hard/soft line boundaries in both directions, Up/Down goal column,
word motion/deletion, selection extension/collapse, Home/End and document edges,
Unicode/emoji/ligatures, undo/redo, IME and clipboard. Typing a settings prompt must
behave like typing a message. Inspect Chad's key translation before blaming shaping.
Record Sanscale obstacles in 004 rather than adding silent local workarounds.

Integration: `tau2-sanscale-text-input` through `04fd60c` was fast-forwarded into
`tau2-rust-frontend`, then `tau2`. The frontend branch is retained. Included in beta 0.7.1 daemon/Windows delivery;
physical acceptance and any upstream limitations below remain open.

## Editor handoff

Implemented in `tau2-sanscale-text-input`, based on settings handoff `446ac29`.
The existing Editor is shared by composer, default/model/title prompts, credentials
and other dialogs; there is no second controller. Typed caret/goal state, word and
page motion, selection/collapse, grapheme deletion, bounded undo/redo and IME
projection are covered by real-font and actual App/GPU tests. Desktop/Android
hardware-key adapters share named-key/shortcut translation. See frontend/QA.md
for completed checks and remaining physical input QA; see 004 for ligature limits.
