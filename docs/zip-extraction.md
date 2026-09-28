# ZIP Extract QA fix — source only

Branch/worktree: `fix/tau2-extract-behavior`, `/root/tau2-extract-behavior`, based on
`origin/tau2` at `6da3b4c`. Implementation: `f3d3c95` (folder handling), `637a5b0`
(single-flight UI and tooltip removal). At the user's request, feature tip
`a66fc3d` was merged as **`3eb2b1d`** into `tau2-integration`, which publishes to
`origin/tau2`. Not deployed; no version, protocol, persisted schema, package,
service or stable/master change.

## Behavior

- **Extract has no hover or context tooltip.** The prior removal covered the
  filename/status/caption, not the Extract action. Other buttons are unchanged.
- The first activation claims the account/source/chat/file target **before**
  queueing its platform action. Further clicks, including a stale second card in
  the other surface before repaint, cannot start another worker. Both transcript
  and Attachments show disabled **Extracting…** until extraction **and opening**
  finish. Navigating away/back does not lose the claim.
- Success, extraction/open failure, missing ZIP and worker-start failure release
  the claim. Failures can be retried; a completion for an old account/source does
  not clear another job or display its error in the new source. This is an
  in-flight guard, not permanent deduplication: a deliberate Extract after
  completion may create a new collision-suffixed copy, without replacing edits.
- Like Tau1's `windows/launcher/src/main.rs`, a ZIP with one top-level directory
  publishes that directory directly under Downloads/Tau and opens **that** folder.
  For example, `download-name.zip` containing `project/src/main.rs` becomes
  `project/src/main.rs`, not `download-name/project/src/main.rs`. Only the redundant
  wrapper is removed; `src` and other meaningful directories remain intact.
- Loose files/multiple roots stay together in an archive-named folder. Existing
  file/folder names get numbered suffixes. Full validation/extraction still occurs
  in temporary staging before publication; the guard removes staging on errors
  and after a successful inner-root move. Empty ZIPs are rejected; a valid empty
  directory can be extracted.
- Existing ZIP protections remain: 50 MB archive, 2,048 entries, 512 MiB expanded,
  safe paths, no symlinks/unsupported types, case-colliding duplicate rejection,
  bounded copies and checked lengths. Open/Show still act on the saved ZIP.
  Android still has no desktop Extract/Show actions.

## Evidence

Before the fixes, the real UI regression queued **four** workers for four clicks,
and the no-tooltip regression found the unwanted hover target (nextest run
`428abbbf-c117-4b13-a4ed-2f3940e4a9c8`). The root-folder regression returned
`download-name` instead of `project` (`11714438-b599-4c48-8452-841e98e54927`).

Final managed validation on `637a5b0`:

- `cargo check --locked -p tau-frontend --all-targets`: passed.
- `cargo nextest run --locked -p tau-frontend --lib --no-fail-fast`: **159/159**,
  zero skipped, 40.472s (`00ff05dc-6818-4807-900f-708d9966934d`). Includes implicit/
  explicit roots, loose roots, collisions/user edits, cleanup, unsafe archives,
  rapid clicks before repaint and after dispatch, busy/retry/success, real shared
  chat/sidebar controls, navigation, and account/source completion fencing.
- `cargo xwin check --locked -p tau-frontend --lib --target x86_64-pc-windows-msvc`:
  passed.
- Android aarch64/API-29 frontend library check: passed, with the existing NDK
  compiler/archiver environment. No Java changes or package build required.

Integration validation: `3eb2b1d` and feature tip `a66fc3d` have the same Git tree
(`9d2efa90520c74b61da2b4059989832fae6eac5c`). Compared with tested source `637a5b0`,
only Markdown documentation differs, including the post-merge status notes. The
existing checks above are therefore reused, not represented as new merged-tree
runs; no redundant suite or cross-build was performed.

All commands used `/usr/local/bin/cargo`, `CARGO_BUILD_JOBS=1`,
`RAYON_NUM_THREADS=1`. No Clippy or Cargo built-in test runner was invoked.
Compiler warnings were not treated as unrelated rewrite work.

The real GPU render matrix now includes **33** cases at four desktop/phone sizes
and scales, with Extract's absent tooltip explicitly asserted. Inspected the
normal hovered Extract, narrow busy card and simultaneous busy chat/sidebar
screenshots in `/tmp/tau2-extract-previews`. Reproduce with
`TAU_DOWNLOAD_PREVIEW_DIR=/tmp/tau2-extract-previews` and nextest filter
`test(download_render_tests) | test(download_interaction_tests)`.

These tests are offline, use temporary files and do not launch an OS file manager.
Physical Windows shell/Explorer acceptance is still open; automated checks are
not a claim of device QA or a deployed beta fix.
