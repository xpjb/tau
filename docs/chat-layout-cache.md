# Transcript layout retention

Chat/topic navigation no longer discards Markdown documents or exact row heights.
Interaction owners, selection and pending gestures still detach immediately; only
source-bound layout data is reusable. Account/source-lineage changes clear both
caches. Returning to a chat revalidates row identities and content stamps, even if
a reloaded feed reused a numeric revision. Width/DPI changes invalidate heights.

Cold or invalidated rows start with provisional extents. The transcript measures
the saved reading anchor first, then the visible window and two-screen overscan,
repositioning after each measurement. Unvisited text is not parsed/shaped merely
to compute total scroll height. Scrollbar extents therefore refine as previously
unmeasured history enters the window. A visible individual message still uses
Barkdown's full-message layout. Tail-follow, disclosure headings and saved offsets
remain anchored during refinement; wheel targets adjust only after final layout.

Two independent pressure-based caches are bounded:

- Markdown documents: LRU, 8 MiB of source text or 4,096 documents. Current draw
  owners and selected text are protected, even if they alone exceed a limit;
  leaving that window makes them reclaimable. Collapsing Details is not eviction.
- Inactive height indexes: LRU, 32,768 rows including the active index, and at most
  128 inactive chats. The active index is not evicted if it alone exceeds the
  row budget. No cached entry contains live controls or pointer state.

These are logical source/entry budgets, not process-RSS limits. Sanscale separately
bounds its shaping caches; Barkdown recovers evicted shaped handles when painting.
The transcript can also recreate a document from the feed after document eviction,
without discarding still-valid cached row heights.

`tests/unit/app/transcript.rs` exercises cold expanded histories, warm chat/topic
returns (zero shaping, reflow or source reads), identical return pixels, background
edits, disclosure collapse, source fences, eviction, width/DPI changes and deep
saved anchors. Renderer/index tests cover pressure, LRU and selection protection.
Existing scrolling, download-navigation and hydration regressions cover the same
production path. Tests use isolated local stores and desktop/phone-sized headless
views; they are not device-specific latency measurements.
