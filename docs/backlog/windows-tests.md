# Windows test failures

Status: open. Recorded on `x86_64-pc-windows-msvc` at `4ddbf86`, using
Rust/Cargo `1.93.0-nightly` and Nextest `0.9.140`.

`cargo check --locked --offline --workspace --all-targets` passed. The Windows
Nextest run recorded four failures before the user cancelled it. The full suite
remains unverified. No baseline rerun or fixes have been made.

The failing source and tests are byte-for-byte unchanged from `tau2` at
`4495c88`. `Cargo.lock` and `.cargo/config.toml` are also unchanged. The only Rust
edits beyond relocation update two embedded-file paths.

## 1. Hidden-file index count

Test: `filesystem::tests::name_sync_prunes_dot_directories_but_keeps_dotfiles_and_explicit_browsing`

Location: `crates/code-viewer/src/filesystem.rs:220`.

Observed: 351 visible files; expected 350.

Investigate Windows path separators. The filesystem index exports native paths,
while hidden-path checks and test expectations use `/`. Confirm the extra entry
and keep indexed relative paths consistent across platforms.

## 2. Rename-index refresh

Test: `filesystem::tests::refreshed_filesystem_sync_sends_renames_and_removals_without_replaying_names`

Location: `crates/code-viewer/src/filesystem.rs:257`.

Observed: `roots.get_mut(&cwd).unwrap()` received `None`. The test panic also
poisoned the root-map lock used by the index worker.

Investigate the fixture's root key. `FileSystem` stores a canonical path; the
test looks it up using the original temporary path. Windows canonical paths can
include a different prefix. Use consistent root identity in the fixture.

## 3. Remote picker wake

Test: `app::code_view::tests::remote_picker_results_wake_and_repaint_without_input_or_polling_frames`

Location: `crates/frontend/src/app/code_view/tests.rs:285`.

Observed: `Picker did not wake the UI: Elapsed(())` after about 20 seconds.

Cause unconfirmed. Identify which wait failed, inspect the local daemon task's
startup result, then check connection and wake delivery. Do not increase the
timeout without finding the cause.

## 4. Remote-file transport

Test: `remote_files_stream_live_updates_search_parent_traversal_and_cancel_without_transcript_writes`

Location: `crates/frontend/tests/remote_files.rs:9`.

Observed: a 20-second timeout with `Cannot reach the daemon` and no notice.

Cause unconfirmed. Inspect the daemon task's startup result and the underlying
connection error. This test owns a loopback daemon; its generic Tailscale error
message does not establish a Tailnet problem.

## Recorded run

Run from Git Bash on Windows. These temporary profile settings forced a rebuild;
only the native Windows target was built.

```sh
CARGO_PROFILE_TEST_DEBUG=0 CARGO_PROFILE_TEST_INCREMENTAL=false \
  cargo nextest run --locked --offline --workspace --no-fail-fast \
  --build-jobs 2 --test-threads 2
```

Investigate only the four named tests first. Close each item after its cause is
confirmed and its Windows test passes. Check the full suite after the fixes.
