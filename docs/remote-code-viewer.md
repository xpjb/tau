# Remote files and local fuzzy picker

Read-only files, launched with the folder button beside View attachments. Directory
list and code buffer share one surface on desktop and Android. The chat's cwd is
the initial location, **not** a sandbox. Up and explicit symlink traversal may leave
it. There are no filesystem write/save, rename, delete or execute actions.

## Interaction

- Tap a directory/file, use Up to browse parents, or use arrows/j/k and Enter.
  Dot-prefixed files and directories are hidden by default. **Show hidden** toggles
  them in both directory browsing and the picker; it does not disable ignore rules.
- Ctrl+Space / Find opens a local fuzzy **file** picker over the chat's initial cwd.
  Nucleo supplies subsequence scoring, path/word-boundary ranking, smart case,
  accent normalization, independent whitespace-separated terms (in either order),
  and exact/prefix/suffix/negative terms. Matching characters are coloured. Names
  include directory components, not file contents. **Here** narrows the picker to
  the currently browsed folder (also useful for folders outside cwd).
- Up/Down, Ctrl-N/P, Ctrl-J/K, Tab/Shift-Tab, Page Up/Down and Ctrl-Home/End move the
  selection without editing the query. All matches are scrollable, not just 100.
  The count shows matches / eligible indexed files, with explicit indexing,
  partial-index and cached/error states. Ignored files and untraversed symlink
  directories are not represented as complete filesystem coverage.
- The selected file has a syntax-coloured, independently scrollable preview below
  the results. Single click/tap selects; Enter, **Open**, or double-click opens it.
  Rapid movement coalesces into a 75 ms delayed preview interest; cached previews
  appear immediately, and revision checks avoid retransmitting unchanged bodies.
  The preview is separate from the opened buffer, selection and saved chat draft.
- **Browse** returns to the directory/code surface. Escape returns directly to chat
  when the picker was opened from chat, or to the prior browser otherwise. **Chat**
  and **X** have distinct retained control identities and both close the browser.
- Desktop: click/drag lines; Shift+arrows extend a selection, `v` starts one, and
  Ctrl+C / Copy copies the original selected text. Horizontal wheel/arrows pan.
- Android: gutter tap/drag selects, or hold code for a haptic acknowledgement then
  drag; dragging back shrinks the range. Ordinary body swipes scroll or pan.
- The shared chat composer appears for a selection and inserts a quoted
  `/absolute/path:start-end` reference into the **saved current-chat draft**. It
  preserves existing prose/attachments and never sends automatically. Clear hides
  the composer but keeps the draft. Returning to chat exposes the ordinary draft.
- Live insertions above unchanged selected text move its reference. An edit within
  a selected run invalidates the selection and removes only the generated marker.
  Active comment editing remains visible so input is not lost, but Send is disabled
  until current lines are selected again. IME composition is not cancelled by a
  live reference update; Android native-editor snapshots are rebased as needed.
- Escape closes search, clears selection, returns from code to its directory, or
  closes the browser. The Chat / close buttons return directly to the conversation.

## Ownership and transport

- `tau-code-viewer`: stable line identities, bounded patience diff, selection
  reconciliation, Tree-sitter paint, fuzzy path ranking, and an optional filesystem
  service. Parsing/diffing and filesystem work run off the render thread.
- The daemon owns one index worker, initialized ahead of time over its cwd. Indexes
  are shared by canonical root, not chat, and refreshed every ten seconds. They
  respect `.gitignore`/ignore rules, including when no repository is initialized.
  Explicit directory browsing still exposes ignored files. Symlink directories
  are not recursively followed; Git's internal `.git` directory is skipped.
- Up to four roots are indexed. Each scan is limited to 200,000 **files**, 24 MiB of
  conservatively estimated JSON records (within the 32 MiB response budget), or
  15 seconds. Visible paths are scanned before hidden paths, so hidden caches
  cannot consume the visible-file budget first. Unsupported paths/read failures
  and scan limits mark the index partial. Ordinary directories use 256-entry pages.
- The client warms names for the foreground chat before the picker is opened.
  Index sync is independent of preview interests and query edits. An unchanged
  revision transfers no names; a known previous revision sends additions/removals,
  and an unknown/evicted revision sends a snapshot. The client applies each update
  atomically off the UI thread, retaining verified cached names on temporary loss.
  Chat/root/source changes fence publication; a new scope must be confirmed before
  cached names are reused. Names are memory-only, not transcript or disk-cache data.
- Ranking runs on a reusable, latest-only worker with cancellation during scans.
  Rendering shapes only visible result/preview rows. Preview caching is bounded to
  eight documents / 8 MiB of source text, in addition to the open buffer.
- The native protocol is **22** (matching client/daemon required; no schema change).
  File RPCs share the existing authenticated Iroh endpoint/connection, 16 KiB
  chunks, 64 KiB byte credit, compression, chunk hashes, whole-response hash, and
  stream-local cancellation. Name sync uses lower-priority background admission;
  directory reads and previews retain foreground admission. No HTTP file path, attachment
  staging, transcript database write, provider call, or separate native endpoint.
- Before any filesystem access the daemon verifies the chat still exists. Cwd
  resolution is centralized in the manager; today all chats use the daemon cwd.
- The frontend owns independent coalesced name-sync and viewer interests with
  bounded result mailboxes. Chat, account/source lineage and request generations
  fence stale results. Navigation cancels old interests; backgrounding/occlusion
  and modals suspend them. Closing the browser releases preview payloads while
  ordinary foreground-chat name warming continues. Reopening needs no query RPC.
- Open files/directories refresh once per second after completion of a read. Files
  are published to the UI only as complete verified snapshots; unchanged files send
  no body. Equal source lines keep their Sanscale paragraph identities even across
  separated edits. Only visible lines are shaped and own paint handles.

## First-cut limits

UTF-8 regular text files up to 4 MiB / 100,000 newline characters; no binary/device/
pipe preview. Paths are UTF-8, at most 2,048 bytes, and cannot contain control
characters. Read errors are inline and retried, not substituted with empty files.
A single displayed line is limited to 16 KiB plus an explicit truncation label;
Copy retains the original text. Syntax colour currently covers Rust, Python,
JavaScript/JSX and JSON up to 512 KiB; other languages/larger files remain plain text.

Android reuses the branch's existing OS text-editor bridge for search/comments;
this feature does not merge the separate mobile-inline-input work. Haptics respect
Android's system setting. This is not a full Vim, editor, LSP or content-grep tool.

## Validation

Real native tests exercise directory paging, ignored vs explicitly opened files,
parent traversal, symlinks, binary/size rejection, live atomic replacement, stable
line references, native byte credit/auth/revocation/cancellation, coalesced results,
and no provider/transcript side effects. GPU tests exercise desktop, 360dp phone,
and 2.5x phone rendering, real hit regions, drag/undrag, touch hold/haptic dispatch,
search/editor isolation, draft preservation, source/chat fencing, and IME updates.

Reproduce screenshots with `TAU_CODE_PREVIEW_DIR=/tmp/code-previews` and managed
`cargo nextest run -p tau-frontend -E 'test(code_view::tests)'` (the environment
variable is optional). All Rust commands use `/usr/local/bin/cargo`; no Clippy or
Cargo built-in test runner. Final run IDs/results are recorded below at handoff.

The local-picker update is **not deployed**. Physical Windows/Android input, native keyboard,
IME, haptic feedback and weak-device GPU acceptance remain device QA; successful
cross-compilation or headless phone layouts do not establish those results.

### Feature-branch handoff — September 28, 2026

Implementation is committed at `823fa3e` on `feat/tau2-remote-code-viewer`.
Managed workspace all-target compiler check passed. Final full workspace nextest
run `9017d8a4-3311-4c4a-9a4b-07b90897f386`: **257/257 passed, zero skipped**
(89.004 seconds of tests). Earlier complete runs also passed (255 tests before the
additional authentication/cancellation and IME regressions; then 257).

Windows x64 MSVC and Android ARM64 library compiler checks passed, as did Android
SDK Java compilation and the new crate's all-feature rustdoc build. Android reports
four cfg/platform dead-code warnings in existing shared frontend code; they are
not correctness or release gates. No lint-driven cleanup was performed.

Inspected actual headless desktop/code/search and phone/comment frames; reproducible
fixtures and representative PNGs are in `frontend/qa/code-viewer/`. Physical-device
acceptance remains open. At the feature-branch handoff, no release packages, merge
into `tau2`, deployment, production-data migration, service restart or paid provider
request had been performed.


### Integration merge — September 28, 2026

Merged `ec962ba` into `tau2-integration` / `origin/tau2`, on top of `cafef7f`.
Conflict resolution preserves the newer multi-chat prefetch plans, native stream
reservations, shared UI navigation and both sets of transport regressions. Chat
or account changes cancel the old file interest before another input event;
explicit same-chat navigation, including download notices, also exits the browser.
Existing drafts remain with their owning chats.

Merged-tree checks, all through `/usr/local/bin/cargo`:

- `check --locked --workspace --all-targets`: passed.
- `nextest run --locked --workspace --no-fail-fast`: **278/278 passed**, zero
  skipped, run `b7162a06-0904-4c1f-a1c6-96f752da6b56` (92.215 seconds of tests).
  Includes the new desktop/phone navigation regression, existing real-daemon
  remote-file and multi-chat prefetch scenarios, and the full transport suite.
- `xwin check --locked --target x86_64-pc-windows-msvc -p tau-frontend --lib`:
  passed.
- `check --locked --target aarch64-linux-android -p tau-frontend --lib`, using the
  existing API-29 NDK compiler: passed with four existing platform-only warnings.
- `git diff --check`: passed. Logs: `/tmp/tau-files-merge-{check,tests,windows,android}.log`.

No release packages, deployment or service restart. The older 0.7.8 downloads use
protocol 20 and cannot be paired with this protocol-21 source; a new matched
client/daemon release is needed. Physical-device QA remains open.


## Local-picker QA handoff

Branch `fix/tau2-fzf-picker`, worktree `/root/tau2-fzf-picker`, based on release
`c13c670` (0.7.9 / protocol 21). Implementation commits `9a3c3f9` and `41cc562` are
pushed; the follow-up fixes idle prefetch invalidation and updates native tests.
**At the feature handoff: no merge, release build, installer delivery, deployment
or service restart.**
The running 0.7.9 beta is unchanged; this protocol-22 branch needs a matched future
client/daemon release. No Clippy or Cargo built-in test runner was used.

Validation, with managed Cargo and one build job:

- Workspace all-target compiler check passed (`/tmp/tau-fzf-check.log`).
- Full workspace nextest run `c6fe4db1-08b8-4c32-8368-670868461031`: **325/328 passed**.
  Three existing idle-render regressions caught an unconditional dirty flag in the
  new prefetch hook. That hook was corrected to invalidate only on errors/data.
- Final focused nextest run `adf430d5-373b-418f-a174-f1acf4b3d186`: **25/25 passed**,
  including all three previously failing tests, all code-viewer/index tests, real
  native prefetch/delta/preview-cancellation tests, background stream admission,
  and the actual-daemon filesystem integration. The entire suite was not repeated
  after the one-line idle fix (`/tmp/tau-fzf-final-tests.log`).
- Windows x64 MSVC and Android ARM64 library compiler checks passed
  (`/tmp/tau-fzf-{windows,android}-check.log`). Existing platform/dead-code warnings
  are not release gates; no unrelated lint rewrites were made.
- Headless UI checks cover desktop, 360dp phone and 2.5x phone, 240 results (no
  top-100 cutoff), highlighted loose/reordered queries, syntax previews, cached
  preview reuse, independent preview scrolling, hidden path components, partial
  counts, root/source fencing, draft preservation, keyboard traversal and actual
  Chat/X/Browse/Open/Show-hidden hit regions. A worker test ranks 50,000 names and
  cancels obsolete queries. Physical-device input/IME acceptance remains open.

Inspected frames are reproducible using `TAU_CODE_PREVIEW_DIR` with the focused
code-view tests. Representative local-picker frames are in
`frontend/qa/code-viewer/fzf-{desktop,phone,phone-2x}.png`.


### Approved source integration

Merged `7963270` into `tau2-integration` / `origin/tau2` as **`4501584`**, on top of
`c13c670`. The merge was conflict-free and its tree exactly matches the tested
feature tip (`3a911f9ffbf6043d1e501010d7543b0728b6b7e5`). Prior compiler checks and the final
25/25 focused nextest result are reused; no test or package rebuild was repeated.
Only integration notes differ after the merge. No deployment or service restart;
protocol 22 still requires a future matched daemon/client release.
