# File viewer retention and Android text selection — October 2, 2026

User-requested source fixes for `tau2-integration` / `origin/tau2`, based on
`6dbcf02`. Implementation: `fb74424` (viewer), `173e88e` (selection/Android bridge).
Work was isolated in `fix/tau2-viewer-android-selection` under the host worktree
policy; the requested integration can fast-forward the tested source.

## Behavior and boundaries

- Chat/X no longer discard the opened file. Files restores the owning chat's
  buffer, selection, and vertical/horizontal reading position immediately, then
  renews the revision-checked read. Find/Browse restores the underlying buffer's
  scroll instead of resetting it; a picker preview is not the opened file.
- Four closed same-source chat views are retained in memory. Closing cancels file
  interest and gestures and releases GPU paint, search fields, filename snapshots
  and preview caches. Deleted chats and account/source changes discard retained
  views. This is not persistence across app restarts or unlimited background reads.
- Mobile shared text fields show draggable selection endpoints. Either end can
  extend, shrink or cross the other without moving the fixed endpoint. Long hold
  selects before finger-up, tolerates small finger jitter, gives haptic feedback,
  and permits hold-and-drag. Stationary edge dragging autoscrolls the field, not
  the chat/form. Cancellation/navigation stops the gesture.
- Handle geometry uses the same Sanscale layout as the text, including wraps,
  Unicode and DPI scaling. Selection changes do not write drafts or create undo
  edits. Existing editor IDs/revisions still reject obsolete IME snapshots.
- Android uses a floating contextual toolbar rather than a modal PopupMenu, so
  the first handle gesture reaches the native window. Its placement follows the
  Rust selection bounds without echoing same-revision text over composition.
  Revision/field changes dismiss stale actions. Password type is applied after
  single-line configuration so the password transformation and copy/cut restriction
  survive; the emulator regression caught the previous ordering problem.

No daemon, transport, protocol/schema, application version, live data, or stable
change. No beta deployment or release installers. Only the isolated Java bridge
fixture APK was built/installed, in its own `app.tau.selectiontest` emulator
package, and uninstalled afterwards.

## Validation

All Rust commands used managed `/usr/local/bin/cargo`, one build job. No Clippy or
Cargo built-in test runner.

- Both viewer regressions failed before the fix: reopen lost the path, and Browse
  reset a 300px reading position. Run `48151119-1996-48cd-be6b-cbde23a95abe`.
  The hold/drag regression also failed before the fix because selection stayed
  empty until release: `14afd786-012f-4583-be55-1a579400cfb8`.
- Frontend all-target compiler check: passed.
- `cargo nextest run --locked -p tau-frontend --lib --test remote_files
  --no-fail-fast`: **181/181 passed, zero skipped**, run
  `124ab6a5-86f7-4b19-bb7f-5e3985a31abb`. Includes real native remote-file behavior,
  navigation, IME/composition, scoped UI ownership, and editor regressions.
- Final focused **8/8 passed**, run `8a7bc657-c391-4f1b-9bec-b0ff9c89aedd`, after
  making the new reopen fixtures explicitly take the real FileOpen route. No
  production Rust behavior changed after the 181-case run.
- Tests inspect actual phone/2.5x GPU pixels for both handles, exercise both ends
  in composer and ordinary dialog inputs, cut/undo, Unicode/wrapped crossing and
  shrinking, stationary autoscroll, cancellation, stale callbacks, and zero
  selection-only SQLite draft writes.
- Android ARM64/API29 and Windows x64 MSVC frontend library compiler checks:
  passed. Java compiled against SDK35 and dexed for API29. Platform/dead-code
  warnings remain; no unrelated lint rewrites.
- Actual Android 36 x86_64 emulator bridge fixture: **passed** visible floating
  toolbar pixels over the NativeActivity surface, first outside down/up reaching the native input queue, Copy/Select All, obsolete
  InputConnection rejection, password copy/cut restrictions and detach cleanup.
  The fixture runs the production MainActivity with a tiny native input/window
  stub; it is **not** a full Android Rust/GPU/IME or physical-phone acceptance run.
- `git diff --check` and bridge script syntax check: passed.

Logs: `/tmp/tau2-viewer-selection-{check,tests,android,windows,final-focus}.log`,
`/tmp/tau2-selection-bridge.log`. Inspected phone/dialog snapshots are reproducible
with `TAU_SELECTION_PREVIEW_DIR=/tmp/tau2-selection-previews` and
`cargo nextest run --locked -p tau-frontend --lib -E 'test(app::mobile_input)'`.

Run the isolated platform check against an already booted, unlocked disposable
x86_64 emulator (never a real phone):

```sh
ANDROID_SERIAL=emulator-5566 frontend/android/tests/run-selection-bridge.sh
```

Physical-phone acceptance remains: real keyboard shown/hidden, drag each endpoint
on long wrapped text, rotation/keyboard resizing, and OEM clipboard/IME behavior.
A new client build is needed before these source fixes reach installed clients.
