# 012 — Revisit chat scroll-position ownership and delete the old tests

Status: **Reopened for the bounded [041 transcript slice](041-direct-retained-transcript.md), September 29, 2026.** The earlier session-specific deferment is superseded by the user's request to triage and finish simplification. No implementation is part of this planning pass.

Review source/chat-bound ownership of reading position, persistence, switching, empty/loading frames, prepend/expansion and layout identity. Preserve useful reading behavior, not the provisional mechanism. Consolidate policy into the actual Transcript owner rather than introduce another state store. Fix stationary-pointer selection immediately in 036; it does not wait for this work.

## Statement to preserve verbatim

```text
I found the cause: after switching chats, the pointer-release save could write the old chat’s layout into the newly selected chat’s scroll position. I’ve added a check tying each measured layout to its chat, saved the old chat before switching, and prevented an empty loading frame from erasing an anchor. The switch regression passes on desktop and narrow layouts; I’m running the full suite now.
```

The user requested deletion of `frontend/src/app/scroll_tests.rs`. **Delete that file and its module declaration in 041; do not mechanically update it again.** The current request expressly allows pruning brittle/redundant tests. Keep only small, useful reading-state assertions at the new owner/existing navigation boundary, not a rewritten copy of the old GPU scenario. The quick fix and its passing tests were never architecture acceptance.
