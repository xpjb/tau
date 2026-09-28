# Client navigation review and download notices — September 28, 2026

Branch: `fix/tau2-download-notification-navigation`, based on `40a3698`.
Implementation: `eb46065`. This is a source change, not a beta deployment.

## Findings in the existing app

The ownership split is useful, but the UI navigation lifecycle was fragmented:

- `Controller.account` owns selected chat/topic, current topic membership,
  per-topic resume choices, read state and persistence. `blocks` owns cached
  history and native content requests. Neither needs screen coordinates.
- `App` owns the composer, visible panes, pointer state and measured transcript
  layout. Previously the action handler changed the controller's selection, but
  only the next `tick()` rebound the composer and discarded the old layout.
  That left the selected chat and the displayed editor temporarily inconsistent.
- This was reproduced on unchanged `40a3698`: after selecting chat two, the
  controller selected two but the composer still contained demo's draft. Input
  handling writes the composer's value to the selected chat, so input arriving
  before that tick could write the old draft into the new chat. The retained
  regression now covers chat, topic and new-chat transitions before another tick.
- Selection reconciliation used only the chat ID. Changing accounts with the
  same selected ID could retain the old account's measured layout. Reconciliation
  now includes account identity and never saves that old layout through the new
  account's controller.
- Notices were strings with a dismiss-only hit action. The existing save target
  already contained the durable account/source/chat/entry identity needed for
  navigation; it did not need a filename lookup or another saved download index.
- Attachment text registered separate hover/tap targets for the name/status and
  caption. These redundant information targets are removed; action descriptions
  remain attached to their own buttons on each surface.

`App` is still a large event/render coordinator, not a fully separated collection
of screens. In particular, history demand and image-preview demand originate in
layout/rendering, and Back/overlay precedence remains in `App`. Those are existing
couplings, not an architecture-wide cleanup claimed by this change. The useful
controller/cache/view ownership boundaries are retained.

## Updated navigation path

`app/navigation.rs` owns UI selection reconciliation and the pending attachment
location. Explicit chat/topic actions and server-driven selection changes use
that reconciliation. The controller remains the single authority for selection;
the navigation state records which account/chat the current editor/layout belongs
to, rather than maintaining a second selection.

Successful OS saves publish a typed `Notice` containing text and a scoped
`DownloadTarget`. Ordinary messages construct notices without a destination.
Replacing a notice therefore replaces its action as well. Popup identity includes
its destination, so equal displayed paths for different files do not share a
stale deadline/action.

Clicking the notice body uses the ordinary chat navigation route and current topic
membership, closes covering panes, and resolves the attachment entry against actual
transcript rows. The destination centers the shared attachment control panel, even
when a long message or image precedes it. It is not an event-ID guess or a saved
pixel offset. The close button only dismisses, and both popup actions consume the
pointer gesture before underlying controls can receive its release.

A pending destination survives empty/loading frames and older metadata pages.
The existing history cursor/loading/attempt gate pages toward it. Intermediate
pages do not overwrite the saved scroll anchor. Manual transcript scrolling,
changing selection/account/source, or navigating away cancels the pending jump;
a confirmed missing widget produces an ordinary notice. Rendering a cached target
also works offline. There is no protocol or persistence-schema change.

## Validation

All Rust commands used `/usr/local/bin/cargo`:

- `check --locked -p tau-frontend --all-targets`: passed.
- `nextest run --locked -p tau-frontend --lib`: **106/106 passed**, run
  `6ce34156-631b-4290-a041-e570b5d055b7`.
- Two relevant `end_to_end` tests: **2/2 passed**, run
  `4a276bad-7d56-4f5d-b056-69388cb9ead3`: real native file authorization/verified
  offline cache, and multi-client topics/drafts/selection recovery.
- `xwin check --locked -p tau-frontend --lib --target x86_64-pc-windows-msvc`:
  passed. An earlier plain cross-check lacked the MSVC tools; the established
  managed xwin setup resolved that build-environment failure.
- Android ARM64 library compiler check with the existing API-29 NDK compiler:
  passed, with four pre-existing unused desktop-helper warnings.
- `git diff --check`: passed.

The new native UI tests exercise desktop, 360dp phone and 2.5× phone geometry,
identical filenames with distinct entry/event IDs, current-topic moves, same-chat
jumps, draft/anchor preservation, overlay closure, click-through protection,
loading and multiple history pages, cancellation, missing/stale destinations,
notice replacement, ordinary navigation before the next input, and account changes.
The frontend suite also covers the existing 32-case download render matrix,
button tooltips/touch holds, topics, scroll restoration, editor and viewer controls.

Desktop and phone GPU screenshots were inspected in
`/tmp/tau2-download-navigation-previews/`. Set `TAU_NAVIGATION_PREVIEW_DIR` when
running `app::navigation_tests` to reproduce them. The loading-page UI fixture uses
controlled history pages; the existing native cache test separately verifies
attachment paging/loading completion. These are not physical-device results.

The baseline reproduction intentionally failed once on a temporary detached
worktree, which was then removed. Its log is
`/tmp/tau2-download-navigation-baseline.log`; the retained fixed regression passes
in the frontend suite. Other logs use `/tmp/tau2-download-navigation-*`.

No packages were built, services restarted, or integration/stable branches changed.
