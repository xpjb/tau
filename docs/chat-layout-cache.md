# Transcript history window and layout retention

SQLite is the sole durable local conversation history. The live `Feed` and
transcript hold one bounded display window instead of mirroring every cached header:

- A cold open projects the newest 256 root events.
- A saved non-tail position projects at most 256 roots around its stable row ID.
- Moving within the middle of the window changes only visible body hydration.
- Entering the first or last quarter replaces the far side from SQLite while the
  existing row anchor keeps the same content under the viewport.
- Older network history is requested only after the display window reaches the
  oldest root already present in SQLite.
- Returning to latest replaces an old window directly from SQLite. The existing
  down-arrow remains visible whenever newer cached roots exist.

The `Feed` has no separate scrollback LRU or retained-root merge path. It owns the
current window, queue state, and local intent overlays. Decoded body text is reused
within that window until the 8 MiB pressure limit; pressure clears nonvisible text
in one pass. Moving the window drops remote events, bodies, message ownership, and
active row geometry that are no longer in that window. Durable history and verified
body chunks remain available in the bounded SQLite cache.

Within the current window, cold or invalidated rows start with provisional extents.
The transcript measures the saved reading anchor first, then the visible window and
two-screen overscan. Unvisited text is not parsed or shaped merely to compute the
scrollbar. Page Up, Page Down, Home, and End navigate the transcript when the
composer is not focused; End returns to the real latest window.

Layout reuse across chat/topic navigation remains source-bound. Interaction owners,
selection, and pending gestures detach immediately. Width or DPI changes invalidate
heights. Account or source-lineage changes clear retained layouts.

Two pressure-based presentation caches remain bounded:

- Markdown documents: LRU, 8 MiB of source text or 4,096 documents. Current draw
  owners and selected text are protected.
- Inactive height indexes: LRU, 32,768 rows across at most 128 inactive chats.
  The active index is naturally bounded by the 256-root history window.

These are logical source and entry budgets, not process-RSS limits. Tests cover the
bounded tail, old-anchor restoration, cached two-way window movement, jump to
latest, body limits, page keys, source fences, and layout reuse.
