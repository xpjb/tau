# 003 — Complete shared text input navigation

Priority: next editor work. Status: implemented on feature branch; merge/device acceptance pending. Depends on 004; coordinate 006/007.

Reported in the running Tau 2 composer: Left/Right inconsistently stop at line
starts or fail to cross lines; Ctrl+Left/Right, Ctrl+Backspace and Up/Down do not
work. Current `App::key` handles only byte/grapheme Left/Right, Home/End and simple
delete. The editor stores a byte cursor but no visual-line affinity or goal column.

Use Sanscale's placed Caret, Motion and measured layout as in `examples/editor.rs`
and `examples/code-editor.rs`. Inspect Compendium's controller without overwriting
its independent uncommitted work. Reuse one Editor for composer/settings/dialogs.

Acceptance: hard/soft line boundaries in both directions, Up/Down goal column,
word motion/deletion, selection extension/collapse, Home/End and document edges,
Unicode/emoji/ligatures, undo/redo, IME and clipboard. Typing a settings prompt must
behave like typing a message. Inspect Chad's key translation before blaming shaping.
Record Sanscale obstacles in 004 rather than adding silent local workarounds.

Ownership: `tau2-sanscale-text-input` in `/root/tau2-text-input`; leave the settings
worktree untouched. This branch is not merged or deployed.

## Editor handoff

Implemented in `tau2-sanscale-text-input`, based on settings handoff `446ac29`.
The existing Editor is shared by composer, default/model/title prompts, credentials
and other dialogs; there is no second controller. Typed caret/goal state, word and
page motion, selection/collapse, grapheme deletion, bounded undo/redo and IME
projection are covered by real-font and actual App/GPU tests. Desktop/Android
hardware-key adapters share named-key/shortcut translation. See frontend/QA.md
for completed checks and remaining physical input QA; see 004 for ligature limits.
