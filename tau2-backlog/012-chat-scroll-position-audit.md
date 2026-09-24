# 012 — Revisit chat scroll-position architecture (deferred)

Status: **Open. Do not take this on during the current session.** The user considers both the pre-existing scroll/selection flow and the quick fix in `frontend/src/app.rs` likely poor design. Do not treat the quick fix or the passing checks as architecture acceptance. Re-examine ownership, layout identity, persistence, switching, empty/loading frames, paging, and whether scroll restoration belongs in this UI flow at all before deciding on a replacement.

## Statement to preserve verbatim

```text
I found the cause: after switching chats, the pointer-release save could write the old chat’s layout into the newly selected chat’s scroll position. I’ve added a check tying each measured layout to its chat, saved the old chat before switching, and prevented an empty loading frame from erasing an anchor. The switch regression passes on desktop and narrow layouts; I’m running the full suite now.
```

The user requested that the newly added `frontend/src/app/scroll_tests.rs` tests be **deleted later**, not during this session. Remove them when revisiting this architecture; do not pile on further tests now. The quick fix is provisional, not a reason to close this backlog item.
