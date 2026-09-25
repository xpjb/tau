# One-command beta releases

Run from the **`tau2-integration` worktree**, not stable/master:

```sh
scripts/release-beta.sh --plan --version 0.7.5 --push --deploy
scripts/release-beta.sh --version 0.7.5 --push --deploy
```

Add `--merge feat/name` to merge and push a local feature first. Choose the next
unused version; the script updates both Rust packages, their lockfile entries,
and Android versionName/versionCode, then commits just those changes. Merge
conflicts, dirty worktrees, concurrent checkout changes and non-fast-forward
pushes stop the rollout. Nothing is force-pushed or auto-stashed.

## What it does without agent babysitting

1. Serializes beta releases with a per-user lock; uses managed Cargo, one build
   job/Rayon thread, one Java processor, and registry-offline builds by default.
2. Reuses successful source/toolchain-keyed stages and checks artifact hashes.
   Documentation-only commits do not invalidate builds/checks. `--online` is an
   explicit opt-in when dependencies need downloading.
3. Builds the daemon, Windows installer and Android ARM64 APK **sequentially**.
4. Verifies the embedded Windows app and launcher, APK package/version/debug flag,
   pinned signing identity, native payload, ZIP CRC/alignment and ELF alignment.
   A missing or wrong Android signing key fails before compilation; no replacement
   key is silently generated.
5. Writes immutable release artifacts, checksums, logs, receipts and an ordered
   delivery manifest. Existing delivered artifacts without receipts are not
   adopted as freshly built or overwritten; choose a new version.
6. With `--push`, atomically publishes the release branch (and merged feature).
   With `--deploy`, requires published HEAD, installs **beta only**, checks the
   running executable and version/protocol, and verifies stable's identity stayed
   unchanged. A matching completed deployment is not restarted on resume.
7. Emits timed progress itself every 30 seconds. Failure prints a short log tail;
   all full logs remain under `dist/releases/VERSION/`. SIGINT/SIGTERM stop only
   this invocation's tagged managed build envelopes, never another Cargo job.

**No compiler check or test suite runs implicitly.** Add `--check` and/or `--test`
only when appropriate; their successful receipts are reused for unchanged build
inputs. Tests use nextest, never Cargo's built-in runner. Clippy remains banned.
No backups are taken. Restore/downgrade is a separate explicitly approved operation.
The script does not certify device UI, WAN performance or application correctness
merely because packaging and health checks passed.

Rerun the **same command** after a failed stage: already successful stages are
reused. An immutable completed release with changed inputs or corrupted final
artifacts is rejected; use a new version instead of silently replacing it.
Uncommitted work must be reconciled first. A cancelled deployment may leave beta
stopped; resume deliberately rather than automatically rolling back a migrated DB.

## Delivery, without pretending files were sent

`dist/releases/VERSION/delivery.json` lists Windows then Android, ready to attach.
In a Tau agent session, call the two `send_file` tools on those paths—no manual
build commands, polling, checksum calculations or log interpretation are needed.
The current host has no standalone Tau attachment CLI, so manifest generation is
**not** reported as delivery.

If a real delivery adapter exists, pass:

```sh
scripts/release-beta.sh --version 0.7.5 --push --deploy \
  --send-command /absolute/path/to/send-file-adapter
```

The executable receives one absolute file path and must return zero only after
successful delivery. Windows is sent before Android; successful sends are
receipted. Interrupted or failed sends remain uncertain and are not automatically
repeated. Reconcile with the adapter, then use `--retry-delivery` only if explicitly
resending is appropriate. No shell expression is evaluated as an adapter.

## The Microsoft SDK warning

`LNK4099` reported absent Microsoft CRT/vendor PDB debug-symbol files, not missing
runtime libraries. Previously those warnings were accepted. Windows build scripts
now request `/DEBUG:NONE` **only for non-debug release builds**, so the linker does
not try to construct unused PDB debug data. This removes the dependency; it is not
`/IGNORE:4099` or a blanket warning suppression. Debug/symbol-enabled builds are
unchanged. Android retains its unstripped library for symbolication.

The change was checked with a targeted managed Windows launcher build (0.39 s,
no warning), not another app-suite run or redeployment. Already delivered 0.7.4
packages and running services were not replaced for this build-time fix.

## Lightweight maintenance checks

```sh
bash -n scripts/release-beta.sh scripts/build-windows-sfx.sh \
  scripts/install-daemon.sh frontend/android/build.sh
python3 -B scripts/test-beta-release.py
scripts/release-beta.sh --plan
```

These exercise only automation/temporary fixtures. They do not build applications,
run the Rust suite, shape traffic or touch services. The pinned public Android
certificate digest is `deploy/android-signing.sha256`; it is not a private key.
