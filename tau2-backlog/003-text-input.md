# 003 — Complete shared text input navigation

Priority: next editor work. Status: open. Depends on 004; coordinate 006/007.

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

Ownership: the user assigned editor work to another worktree. No editor or Sanscale
implementation was started by the settings worktree. Coordinate there before editing.
