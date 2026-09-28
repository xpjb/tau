# Remote code viewer — integrated source, not deployed

On September 28, 2026, `feat/tau2-remote-code-viewer` (`ec962ba`, implementation
`823fa3e`) was merged into `tau2-integration`, which publishes to `origin/tau2`.
The newer transcript-prefetch and download-navigation work is retained. Shared
navigation closes/cancels the browser before rebinding a different chat/account,
and same-chat download notices also reveal the transcript without losing drafts.

Protocol **21** requires a matching daemon and client; there is no schema change.
**Do not deploy this source with the existing 0.7.8 / protocol-20 packages or apply
the older rollout command below to this HEAD.** A new versioned, matched
protocol-21 release is needed when explicitly requested. This merge did not build
release packages, deploy, restart services, or modify stable/master.

Merged-tree validation: managed workspace/all-target check and **278/278 nextest
tests** passed (`b7162a06-0904-4c1f-a1c6-96f752da6b56`, zero skipped). Windows x64
MSVC and Android ARM64 library checks passed; Android retains four existing
platform dead-code warnings. The added desktop/phone regression covers immediate
chat input and same-/cross-chat download navigation out of the browser.

See `docs/remote-code-viewer.md` for ownership, limits and validation, and
`frontend/qa/code-viewer/README.md` for reproducible headless previews. Physical-device
QA remains open. The feature worktree is `/root/tau2-remote-code-viewer`; the
isolated merge/validation worktree is `/root/tau2-remote-code-viewer-merge`.

---

# Current rollout hold — beta 0.7.8 packages attached, awaiting download confirmation

**Do not restart or deploy the daemon until the user explicitly confirms both
Windows and Android packages have been downloaded.** The user corrected the
merge destination to `tau2-integration`, not master. Local `tau2-integration`
tracks/publishes to `origin/tau2`. Stable/master must remain untouched.

- Feature `cdc33ca` (`fix/tau2-composer-thinking`, `/root/tau2-composer-thinking`)
  is merged as `af0e5e0`; release `b74af7e` is published to `origin/tau2`.
- `scripts/release-beta.sh --merge fix/tau2-composer-thinking --version 0.7.8 --push`
  completed successfully, **without `--deploy`**. All three binaries built
  sequentially; Windows and Android package verification passed. Protocol 20
  includes the previously integrated checkpoint-watch change `2517e9c`.
- Both packages were attached via Tau's `send_file`, Windows first, Android second:
  `/root/tau2-integration/dist/Tau-Beta-0.7.8-windows-x64.exe` and
  `/root/tau2-integration/dist/Tau-Beta-0.7.8-android-arm64-v8a.apk`.
  Attachment queuing is not confirmation of a completed user download.
- Beta remains **0.7.7 / protocol 19**, PID **1467375**. Stable PID **474496** is
  unchanged. Both services' active states, PIDs and monotonic start times matched
  before/after packaging. No production service was stopped, restarted or installed.
- New protocol-20 clients need the pending daemon update. Only after the user's
  explicit confirmation, run in `/root/tau2-integration`:
  `scripts/release-beta.sh --version 0.7.8 --push --deploy`.
  This reuses the completed package receipts; do not rebuild or rerun the suite.
- Three focused and fourteen related nextest tests passed; actual desktop/phone
  GPU screenshots were inspected. No physical-device acceptance is claimed.
  See `frontend/QA.md`; hashes and package facts are in `INTEGRATION.md`.


---

# Current Tau2 status — 2026-09-27 — beta 0.7.7 / protocol 19

This status supersedes the older prerelease notes below. Release commit `4b92d28`
and downloads merge `70b0444` are pushed to `origin/tau2`. The downloads change
restores Tau1-style text actions. Protocol 19 is unchanged.

- Beta is active at PID **1429355** and reports **0.7.7 / protocol 19**. Stable
  Tau remains active at PID **474496**; it was not restarted or changed.
- Before restart, the read-only check found 26 sessions, no running sessions,
  no queued prompts, one idle session and 25 sleeping sessions.
- The release workflow built the daemon, Windows x64 installer and Android ARM64
  APK; package checks passed. The five focused downloads tests passed on the
  feature branch. The merged-tree rerun timed out waiting for Cargo's shared lock;
  it did not reach the tests. No full suite or physical-device test was run.
- Windows x64 was verified and sent: `dist/Tau-Beta-0.7.7-windows-x64.exe`,
  SHA-256 `b4872c942783a128653264cdd7999a7da384689da4ab71ee5e584504a281c0e4`.
- Android ARM64 was verified and sent: `dist/Tau-Beta-0.7.7-android-arm64-v8a.apk`,
  versionCode **12**, SHA-256 `2fa3befe4f8cc96b212a84a61556a686853d9c30a580d0d60cbe7f3aa4a2a494`.
  No database backup was taken; the release script does not make one. Detailed
  release facts are in `INTEGRATION.md`. Physical Windows/Android acceptance remains open.
- The separate remove-pause work remains unmerged at WIP commit `d561c65` and was
  not included in this release.

---

# Mobile QA source update — September 26, 2026

Implemented on `fix/tau2-mobile-qa-20260926` in `/root/tau2-mobile-qa`, based on
local beta 0.7.5. See [mobile QA acceptance](docs/mobile-qa.md) for exact behavior,
commits, tests and logs. The changes cover mobile editing/keyboard insets, notice
tap capture, topic navigation, drawn arrows, system variable-font bold, ordinary
message retry/recovery and shared desktop/mobile connection health.

Sanscale font support is on `fix/android-variable-fonts` in
`/root/sanscale-mobile-fonts`; Tau pins `7bbe230` so newer unrelated API renames
do not enter this patch. No fonts are bundled in the Android/Windows application.

Automated acceptance is complete. **No packages, deployment or service restarts
were requested or performed.** Physical Android keyboard/Back/rotation/font/touch
acceptance remains with the user. Done still saves a draft, not a new send gesture.
Do not downgrade a client while it has pending `Checking` delivery records; older
clients do not recognize this new state. Older “Not sent” records have an explicit
same-ID retry, not an unsafe automatic migration of unknown rejection reasons.

---

# Release automation — use this instead of manual rollout steps

`/root/tau2/scripts/release-beta.sh` now owns the process. See
[docs/beta-release.md](docs/beta-release.md). It defaults to no tests, no push and
no deployment; the operator opts into those actions. Successful stages are reused,
Windows/Android build sequentially, and outputs are verified/checksummed. Use its
`delivery.json` for the two Tau file attachments, or an explicit real sender hook.
Do not replace those receipts with repeated ad-hoc builds, polling or hash commands.

Non-debug Windows release builds now use `/DEBUG:NONE`, removing the unused SDK
PDB dependency rather than hiding LNK4099. A tiny managed launcher build verified
that change (0.39 s, no warning). Seven isolated automation checks took 2.5 s; no
Rust suite, full package rebuild or redeployment was performed for this change.
Already delivered/running **0.7.4 / 18** remains unchanged. Pick a new version for
the next release; existing delivered artifacts without automation receipts are
not blindly adopted or overwritten.

---

# Integration release override — beta 0.7.4 / protocol 18

The user explicitly authorized pushing/merging the native rewrite and redeploying
beta, with **no new backup**, then sending Windows and Android packages. The
feature below is merged/pushed and deployed through `/root/tau2`, local `tau2-integration`
tracking `origin/tau2`; stable remains untouched. The historical no-deployment
restriction below is superseded for this authorized beta rollout only.

Reuse the completed merged check and **169/169 nextest result**
(`422f6a71-95bd-4ce0-88dd-f3f6d581ee83`) and release daemon build. **Do not rerun
unnecessary tests.** The user explicitly requested minimal rollout checks after
an opaque combined build/test command caused unacceptable delay. Build client
targets sequentially with one Cargo job/Rayon thread through the managed wrapper.
Release merge: `ca87e4d` (pushed to `origin/tau2`); daemon **0.7.4 / 18** is running.
Windows and Android 0.7.4 packages were built, verified and sent in that order.
Actual deployment/checksum facts are recorded in `INTEGRATION.md`.

---

# Tau2 native network rewrite — completed local implementation

## Location and safety

- **Branch:** `feat/tau2-block-sync`; **worktree:** `/root/tau2-block-sync`.
- Previous checkpoint: `0b08126`; use `git log -1 --oneline` for the completed local checkpoint.
- **Not pushed, merged or deployed.** Preserve `/root/tau2`, `/root/tau2-remove-pause` and unrelated worktrees.
- **Do not start/deploy beta or change/restart stable without explicit approval.** Read-only verification: `tau2-beta.service` inactive / PID 0; `tau.service` active / PID 474496.
- `/root/AGENTS.md`: **Clippy is banned in every form**. Use managed `/usr/local/bin/cargo`, nextest and rustdoc. No Cargo built-in test runner or wrapper bypass; retry shared-lock contention normally.

## User context and status

After the previous handoff explicitly acknowledged unfinished work, the user said **“sounds pretty based, go ahead and finish the full task.”** This continuation completed the remaining implementation and automated local validation. It did not alter any production service.

**Code/local automated gates are complete; deployment and real device/WAN certification are not authorized or claimed.** Read [docs/tau2-block-sync.md](docs/tau2-block-sync.md) for the current contract, limits, measured evidence and backup/restore/rollback procedure. The older protocol audit is historical, not an outstanding implementation checklist.

## Completed since `0b08126`

- Protocol **18**, source schema **5**, authored client schema **2**, replica schema **2**. Required durable Hello lineage binding; transactional source-change fences and persisted cross-handle replica epochs.
- Cold acceptance independent of history/provider preparation; context parsing outside locks, bounded/cancellable preparation, eight owned runs, two title jobs, 128-runtime admission and cold-project deletion without runtime inflation. Startup recovery is batched; portable export streams SQLite blobs.
- Private-context call-ID preservation and per-turn dangling-result repair. Native display pairing uses block parents/identities rather than reused provider IDs.
- Saved-action inspection/check/retry/forget UX, clear-replica and live diagnostic copy. Transactional alias recovery preserves distinct draft/file bundles. Missing-source authored work remains reachable and can be copied into an unsent new draft. File-save failure and explicit unreferenced-file removal are handled safely.
- Source/cache metadata and SQLite limits, bounded journal retention and whole-scope reset. Retained upload manifests/fork owners, periodic lease/orphan maintenance and conservative legacy-file handling. Detached finalization keeps ownership/capacity through cancellation. Outbox snapshots carry integrity digests and do not depend on later edits to originals.
- Keyset-paged sessions/projects, structural revision fencing, coalesced traversals and delayed status protection. Bounded receipt/submission cohorts; read-only request IDs no longer accumulate unnecessarily.
- Sparse cache projection, dirty plans, bounded previews/working sets, capped ordinary prefetch, explicit complete copy with aggregate metadata preflight, closed-child copy, full file captions and attachment cards independent of collapsed tools. Disposable account replicas/downloads have separate collection policies; authored files are not silently evicted.
- Independent bounded control writer/health lane, healthy-probe-based jittered backoff, native class priorities, dual-stack direct routes and diagnostic counters.
- Offline `taud --rotate-lineage DATABASE`: exclusive writer lease, preserved mutation ownership, no replay, restored-chat execution guards and explicit review that does not resume anything. No in-place downgrade to protocol 17/schema 4.

## Validation

Managed commands, all successful after implementation:

```sh
/usr/local/bin/cargo check --locked --workspace --all-targets
/usr/local/bin/cargo nextest run --locked --workspace --no-fail-fast
/usr/local/bin/cargo doc --locked --workspace --no-deps
git diff --check
```

**168 tests passed, 0 skipped, 14 binaries**; full nextest run `0737f711-2d72-4778-8040-6254e06351df`. Compiler/rustdoc are warning-free. Logs: `/tmp/tau2-finish-check.log`, `/tmp/tau2-finish-tests.log`, `/tmp/tau2-finish-doc.log`.

Coverage includes actual-process crash recovery, SQLite FULL and kernel ENOSPC, alias/source-fence rollback, post-rename cancellation, duplicate finish/fork retention/deletion, catalogue/status races, migrations/future-version rejection, sparse/large-copy limits, full metadata captions, cross-handle reset fencing, replica collection, IPv6 and 16-peer fanout. Actual headless GPU mobile/desktop tests passed; the inspected mobile restore dialog screenshot is `/tmp/tau2-restore-mobile.png`.

The genuine native shared-link regression runs taud plus two production Controllers and an unpaid local provider fixture through raw TCP/UDP proxies: **64 KiB/s per direction, 35 ms one-way delay, every 23rd UDP packet dropped**. It covers a 256 KiB upload, >40 KiB fenced code, 24 open tool disclosures versus a collapsed client, a 512 KiB cancel/resume download, concurrent rename, unread-file nonmaterialization and no provider replay.

Two recorded focused repeats: `f0b0fb1e-3a40-4418-bf3a-c0db38481990`, `7e53784c-b23b-4d21-9fd8-384d3151290d`; control receipts **207 / 203 ms**, worst control RTT **366 / 224 ms**, roughly 1.15 MB shaped UDP, one native connection per client, >=64 KiB resumed offset, zero integrity failures. Logs: `/tmp/tau2-native-final-1.log`, `/tmp/tau2-native-final-2.log`. The final full suite also reran this scenario.

**Important transport finding:** the loss test triggered a fatal assertion in pinned `iroh-quinn-proto 0.13.0` multi-datagram pacing. Supported segmentation offload is now disabled; the shaped regression passes without weakening assertions or removing loss. Rerun it before changing that workaround or transport version. Initial evidence: `/tmp/tau2-native-link-gso-crash.log`.

## What remains operational, not an unfinished cutover

- Obtain approval for target-device packaging, Pixel/Shlap/manual UI testing, real WAN/Tailscale reachability and sustained resource/throughput measurements. Headless rendering and loopback shaping are not those certifications or hard latency guarantees.
- Follow the documented offline backup + owned-tree restore + lineage rotation procedure. A failed/interrupted rotation must be repaired/rerun before serving. A matching schema-compatible rollback is required; do not downgrade upgraded state in place.
- Application/page quotas need physical WAL/temp headroom; layout still walks bounded retained metadata. Unknown legacy files and ambiguous paid outputs require deliberate inspection/archiving, not automatic deletion or regeneration.
- **No push, merge, deployment, beta start or stable service change is authorized by this completion.**
