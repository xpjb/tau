# Windows test follow-up

The native `x86_64-pc-windows-msvc` run at `4ddbf86` recorded four failures
before cancellation. This cleanup investigates those failures, rather than
increasing timeouts or deleting useful coverage. The full native Windows suite
still needs a Windows host.

## Filesystem index: reproduced and fixed

Both historical failures reproduced using Windows-target nextest under Wine:

- `name_sync_prunes_dot_directories_but_keeps_dotfiles_and_explicit_browsing`
  counted 351 visible files instead of 350. Relative index names used native
  backslashes, but hidden-name checks and matching use `/`. Index names now use
  wire separators; absolute daemon paths retain their native syntax.
- `refreshed_filesystem_sync_sends_renames_and_removals_without_replaying_names`
  looked up a noncanonical temporary path in a canonical-keyed cache and
  poisoned its mutex. The fixture now uses the canonical key.

`PathIndex::absolute` also preserves verbatim Windows root semantics when
joining a wire-relative name, regardless of the client's OS. A regression covers
Unix, drive-letter verbatim, and verbatim UNC roots.

All ten code-viewer tests passed under Wine after these changes. Wine leaves
service processes alive: nextest reports two leaky test processes and the
managed systemd envelope exits nonzero on cleanup. This is not a clean native
Windows validation result, and the wrapper was not bypassed or modified.

## Remote picker and remote files: startup causes fixed; native rerun needed

The original 20-second timeouts concealed daemon startup errors. Both fixtures
now race their initial readiness wait against the daemon task and report its
actual result immediately.

- Default settings specify `/bin/bash`, which is not an absolute Windows path.
  These read-only loopback fixtures never execute a shell; on Windows they now
  provide a native absolute placeholder (`current_exe`) for validation.
- Daemon database/settings/attachment publication attempted ordinary directory
  opens for `sync_all` on Windows. Directory sync is now Unix-only, matching the
  existing frontend and export paths. File sync and SQLite durability remain.

Under Wine, both fixtures now reach Iroh startup, where the platform rejects
socket configuration with OS error 10045. They no longer report a misleading
Tailscale/UI-wake timeout. Native Windows is required to validate these remaining
transport cases; they are retained, not ignored or deleted.

## Executed checks

- Managed Linux nextest: 30/30 selected code-viewer, settings, attachment,
  remote-file and remote-picker tests passed.
- Managed xwin environment plus Windows-target nextest under Wine: both original
  index failures reproduced; subsequently 10/10 code-viewer test bodies passed.
- The two Windows-target daemon-backed fixtures are blocked by Wine's socket
  implementation after the startup fixes above.

Use `/usr/local/bin/cargo` for host Rust work. No Clippy, built-in Cargo test
runner, wrapper override, deployment, or credentialed/provider test was used.
