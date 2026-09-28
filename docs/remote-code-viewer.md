# Remote code viewer — development branch

Read-only files, launched with the folder button beside View attachments. Directory
list and code buffer share one surface on desktop and Android. The chat's cwd is
the initial location, **not** a sandbox. Up and explicit symlink traversal may leave
it. There are no filesystem write/save, rename, delete or execute actions.

## Interaction

- Tap a directory/file, use Up to browse parents, or use arrows/j/k and Enter.
- Ctrl+Space / Find opens a fuzzy path picker over the chat's initial cwd. Names
  include directory components, not file contents. **Here** narrows the picker to
  the currently browsed folder (also useful for folders outside cwd).
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
  respect `.gitignore`, including when no repository is initialized. Explicit
  directory browsing still exposes ignored files. Symlink directories are not
  recursively followed by the index. Git's internal `.git` directory is skipped.
- Up to four roots are indexed; each scan is limited to 200,000 entries, 32 MiB of
  name/path strings, or 15 seconds. Partial/indexing status is explicit; Here can
  narrow the root. Shallow paths are indexed first so project/folder names remain
  discoverable even if a huge cache subtree exhausts the deeper scan. Ordinary directories use 256-entry pages, search returns 100
  ranked paths, and neither response requires an unbounded tree on the client.
- The native protocol is **21** (matching client/daemon required; no schema change).
  File RPCs share the existing authenticated Iroh endpoint/connection, foreground
  admission budget, 16 KiB chunks, 64 KiB byte credit, compression, chunk hashes,
  whole-response hash, and stream-local cancellation. No HTTP file path, attachment
  staging, transcript database write, provider call, or separate native endpoint.
- Before any filesystem access the daemon verifies the chat still exists. Cwd
  resolution is centralized in the manager; today all chats use the daemon cwd.
- The frontend owns one coalesced viewer interest and bounded result mailbox. Chat,
  account/source lineage and request generations fence stale results. Navigation,
  closing and backgrounding cancel the stream; foregrounding and reconnecting
  resubscribe. Closing also releases the last file payload held by the mailbox.
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

This branch is **not deployed**. Physical Windows/Android input, native keyboard,
IME, haptic feedback and weak-device GPU acceptance remain device QA; successful
cross-compilation or headless phone layouts do not establish those results.
