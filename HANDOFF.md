# Android startup locking — branch source only, October 2, 2026

User reports near-instant normal launch versus recognizable >1-second stalls,
possibly a DB lock failure. On `fix/tau2-android-startup-lock` in
`/root/tau2-android-startup-lock`, based on `8cceeb8`, reproduction `7d71909`
and fixes `dd2a831` / `b5937b2` remove current-schema initialization writes, keep
busy optional read-LRU/local-body-reuse updates off the first frame, and serialize the short replica-admin
handoff without failing `try_lock`. Live leases/quota, real write timeout,
durability, migration rollback and authored work remain protected. Android stage
logs and error chains distinguish future recurrences from GPU/lifecycle waits.

Native/Android ARM64/API29/Windows MSVC compiler checks and frontend rustdoc pass.
**210/210 frontend nextest cases pass**, including real phone-size first-frame
pixels/draft/uncertain intent under a held WAL writer, migrations and concurrent
replica quota/GC.
No Clippy, built-in test runner, package, deployment or production-data change.
**Not merged or released; no attached Android device and no claim that the
user's physical-phone recurrence is closed.** See
[causes, diagnostics, exact evidence and remaining acceptance](docs/android-startup-lock.md).

---

# Stop/Play lockout — integrated source, not deployed, October 1, 2026

User requested fixing the stuck run-through → Stop → Play flow, then recording
that the UX makes no sense, suggests technical debt, and Play/pause has already
been reviewed repeatedly, followed by commit/push/merge. Implementation
**`6422e27`** on `fix/tau2-control-recovery` retires controls on Stop, makes Play
supersede stale intent, preserves queue data and applies last-intent precedence
through cancelled-task cleanup. Genuine failure/restore pauses remain.

Both lockout reproductions fail before the patch; **66/66 daemon nextest cases**
pass after it across disjoint selections, including WAL/disk-full/recovery.
The retained frontend control regression passes, including phone-sized visible
Cancel/Play. See [exact evidence](docs/tau2-control-recovery.md).
[045](tau2-backlog/045-play-pause-control-ux-debt.md) is **open**, not fixed by this
patch: run-through still waits for the existing boundary; systemic UX and control
ownership need review. **`efd1476`** merges feature `eab961b` into `origin/tau2`
from `1209d85`, preserving the integrated filename index. Merged native daemon/
frontend all-target check and the actual-controller queue end-to-end case pass
(`7963b209-8413-40c7-abac-3ccc415995b3`); previous tests are reused. No deployment,
release packages, version/protocol/schema change, service restart, live data edit
or physical-device acceptance. The reported running conversation is untouched.

---

# Wheel-scroll startup fix — integrated source, not released, October 1, 2026

At the user's request, `fix/tau2-undock-scroll-latency` (`ff92cf8`, fix `ec2d764`)
is merged as **`e7e0294`** into `tau2-integration`, publishing to `origin/tau2`.
Wheel input now wakes on-demand rendering before the easing animation starts.
The merge exactly matches the validated feature tree; reuse the **197/197 frontend
nextest** result and native/Windows compiler checks. Only status notes follow;
no redundant build or test run. See [frontend QA](frontend/QA.md) for the failing-before
regression and scope. No package, version bump, deployment, service restart or
stable/master change. Physical Windows latency/DPI acceptance remains open.

---

# Simplification and single-task compaction — integrated source, not deployed

At the user's request, practical closeout is merged into `tau2-integration`,
which publishes to `origin/tau2`:

- **`73a617f`** merges `feat/tau2-simplification` through `7be1ac1`.
- **`1cb2282`** merges the tested `fix/tau2-compaction-pause` through `1f75f4d`
  (daemon fix `302e25f` plus visible failure reasons). The unrelated old
  `tau2/qa-remove-pause` WIP is preserved, not included.

The named legacy UI/model paths are removed. The final bounded pass deleted
three unused hit-test entry points and their unused import, without dropping
features or safety tests. The pause fix supports repeated compaction inside one
long task without replaying tools; genuine failure/abort/recovery pauses remain.
Header/sidebar integration preserves direct update/control ownership and shows
actual failure reasons. The simplification lifetime suite and the added
pause-reason regression were kept; retired baseline cases were not resurrected.

Fresh merged-tree validation: workspace/all-target check; **328/328 nextest tests
across 19 binaries**, zero skipped (`0c6b4074-76b5-4b77-92d0-cde1b3ca931c`, 101.816s);
Windows MSVC and Android ARM64/API29 library checks; workspace rustdoc. No Clippy
or built-in Cargo test runner. Physical device acceptance remains open.

Remaining work is triaged in `tau2-backlog/044-closeout-and-followups.md`: 013
queue-content lifetime, 014 outage diagnosis, device QA, optional justified
pruning, and the **unmet** size target. Simplification alone saved **1,570 raw /
564 normalized** lines after outside charges. Including the independent pause
fix, combined savings are **1,430 / 323**. The user accepted source integration
with this debt recorded; this is not a claim of meeting the 5,000-line goal.
See `docs/tau2-simplification-closeout.md` and the completion-size ledger.

No deployment, release package, version/protocol/schema bump, service restart,
settings/live-data change or physical Windows/Android QA occurred. Prior release
holds remain. Feature branches/worktrees and unrelated WIP are preserved.

---

# Local fuzzy picker — integrated source, not deployed

At the user's request, `fix/tau2-fzf-picker` (`7963270`) was merged into
`tau2-integration` as **`4501584`**, publishing to `origin/tau2`. It implements local
fzf-style matching,
background revisioned name sync, highlighted results + selected-file preview,
hidden-file toggles, complete match counts, and working distinct Chat/X controls.
See `docs/remote-code-viewer.md` for ownership, limits, validation and handoff.
Protocol **22** requires a future matched client/daemon release; source only, no
deployment/packages/restarts. The running 0.7.9 beta is untouched. The merge is
conflict-free and exactly matches the tested feature tree; prior validation is
reused, with no redundant rebuild or test rerun.

Final focused nextest: **25/25 passed**, including the three idle regressions found
and fixed during the broad run (325/328 before that fix). Windows/Android library
checks pass. Physical-device QA remains open. Continue using managed Cargo only;
Clippy and the built-in test runner remain prohibited.

---

# ZIP Extract QA fix — integrated source, not deployed

At the user's request, `fix/tau2-extract-behavior` (`a66fc3d`) was merged into
`tau2-integration` as **`3eb2b1d`**. Integration publishes to `origin/tau2`.
Commits `f3d3c95` and `637a5b0` restore Tau1-style single-root ZIP extraction,
remove Extract's tooltip, and guard/disable it across both card surfaces until
extraction plus opening finish. The feature branch/worktree remain preserved.

The conflict-free merge has exactly the feature branch's tree; only Markdown
status notes change afterward. Reused the existing managed frontend all-target,
Windows MSVC and Android ARM64 compiler checks and **159/159 frontend library
nextest** result, zero skipped; no redundant rebuild or test rerun. Offline GPU
screenshots were inspected during feature QA; no physical Windows acceptance is
claimed. See `docs/zip-extraction.md` for exact evidence and re-extraction semantics.

**No deployment, package build, service restart, version/protocol/schema change,
or stable/master change.** Existing release/deployment holds below remain.

---

# Retained UI and repeated-pause fix — integrated source, not deployed

The user reviewed the retained UI's +3,050 same-format production-line increase
and explicitly requested the merge. Source integration accepts that tradeoff; it
is not a claim that the size-reduction target was met or that physical-device QA
has passed.

- Retained UI `186ad23` merged as **`7ab80f3`**. All four migration stages are in;
  old global actions/hits/lanes/modal indices are removed. Android edits inline in
  the existing window using the adapted `46618b9` InputConnection implementation.
- Independent resume-after-stop fix `0b7c2ae` merged separately as **`b52f80e`**.
  An accepted Resume is not overwritten by a cancelled run's cleanup; later stops
  still win. Provider failures are not automatically retried.
- Validated in `/root/tau2-retained-ui-merge` on `merge/tau2-retained-ui`, based on
  freshly fetched `origin/tau2` at `415aeff`. Both merges were conflict-free.
- Merged-tree managed workspace/all-target check and **308/308 nextest tests**
  passed, zero skipped (`ac0efec3-a0dc-4d7b-b238-3e371e7d693d`, 108.550s).
  Windows MSVC, Android aarch64/API-29, SDK-35 Java and workspace rustdoc passed.
- **No deployment, service restart, version bump, release package or stable/master
  change.** Running beta behavior is unchanged. Protocol 21 still requires a
  versioned, matched daemon/client release when explicitly requested; Java and
  Rust inline-editor components must ship together. Do not use older rollout
  instructions against this source.
- Physical Android keyboard/IME/touch/pinch and interactive Windows QA remain
  open. Automated/headless checks do not prove those experiences.

See `docs/tau2-retained-ui-implementation.md` for the complete ownership, deletion,
validation and reproducible size ledger; `docs/tau2-resume-during-stop.md` records
that bug's failing-before/passing-after evidence. Feature/worktree histories,
including the unrelated `tau2-remove-pause` WIP, remain preserved.

---

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
