# Shared Barkdown source integration — October 5, 2026

Tau beta now uses the separate `xpjb/barkdown` repository, full-revision pinned,
for Markdown, fenced-code syntax colors, bounded math and citation fallback.
Sanscale's common master pin includes prepared-cache residency and tab fixes.
Compendium shares those pins and code colors without changing its source editor.
[Validation and caveats](docs/text-repaint-stability.md#october-5-2026--shared-barkdown-and-additional-real-use-evidence).

This is source-only: no release number bump, client package, daemon deployment or
service restart. The integrated-updater proposal belongs to Compendium's RFC;
no self-update channel was published during this task.

---

# Unreleased Tau2 source — October 2, 2026

Client-first model selection and quiet file rows are merged as `f442792`, on top
of the shared-scroll integration. The combined source passes 311/311 selected
nextest cases plus native workspace/all-target, Android ARM64/API29 and Windows
MSVC checks. [Details](docs/client-first-models.md).

**Source protocol is now 23; the deployed beta below remains 22.** A matched
client/daemon release is required. No release build or deployment was performed
for this merge; application version/schema and stable remain unchanged.

---

# Beta 0.7.11 released — October 2, 2026

Matched daemon, Windows x64 installer and Android ARM64 APK are built from
`4c9a289` (native discovery correction `d9fae28`), published to `origin/tau2`,
and verified. Protocol remains **22**, Android versionCode **16**. Later branch
cleanup/release documentation commits do not change the built source inputs.

- **Beta deployed at 20:23 AEST:** `tau2-beta.service` PID `3209092`; health reports
  `0.7.11 / 22`, and the running executable matches the verified release binary.
  The authenticated pre-deploy status check found 94 sessions, none running
  (93 sleeping, 1 idle); after restart all 94 are sleeping. No agent work was
  active when deployment began. The existing beta URL/ports remain unchanged.
- **Stable untouched:** `master` remains `3818579`; stable PID `474496`, original
  start time and executable SHA-256 are unchanged. No live-data restore or manual
  DB edit, route takeover, transport replacement or paid operation was performed.
- Native transfer/link/pressure **26/26**, slow reads **2/2**, and final workspace/
  all-target compiler check pass. The new cold-discovery test fails before the
  correction, then passes with one attempt and a five-second established-peer
  timeout. [Exact evidence](docs/native-discovery-timeout.md).
- Sequential release completed with Windows embedded-payload checks and Android
  version/package/native-payload/alignment/pinned-signature checks. One Windows
  installer build-lock timeout was retained; resuming the existing script reused
  the daemon receipt and cached Windows app. No lock bypass or other job was killed.
- Windows then Android were submitted with `send_file`; both bridge calls returned
  **queued for chat delivery**. This is not confirmation of download, installation
  or physical-device acceptance. Artifacts and checksums are in `dist/`; resumable
  receipts/logs are in `dist/releases/0.7.11/`. Audit logs and starting Git bundle:
  `/root/tau-branch-rollup-20261002/`.
- **97 original local and 48 remote refs removed.** Only stable and Tau2 mainlines
  remain from the initial inventory. The clean, already-integrated Android
  checkout is detached at `1a8ee0c` and retained solely because the live adb server
  uses that directory. The temporary review worktree/ref is removed after this
  record lands. [Every original branch](docs/branch-rollup-20261002.md) is accounted for.
- Mog received only an adoption-gap note, `/root/mog/docs/TAU.md`, local commit
  `af01924`. Separate bugs 013/014/045 and physical-device QA remain open; passing
  these fixtures does not close the old seed-73 per-file finding.

---

# October 2 branch rollup — source integration and release preparation

The user requested accounting for every Tau/Tau2 branch, removing obsolete refs,
and releasing Tau2/beta. Stable/master stays unchanged. Both fresh model-catalog
and Android startup integrations are included.

- `64a9beb`: missing block-admission/repaint/native-recovery fixes.
- `4ebb24f`: missing new-chat chooser behavior adapted to retained UI.
- `15acf5a`: user-requested network-pressure harness and local diagnostics.
- The user explicitly rejected the old remove-pause experiment; its branch and
  clean unused worktree are removed, with Git recovery material retained.
- Every starting branch and deletion reason is listed in
  [the rollup inventory](docs/branch-rollup-20261002.md). A verified local bundle
  preserves the starting refs; no production data backup or restore was performed.

Four regressions fail on current pre-rollup `396d5ba` and pass in the candidate,
so new-chat and block recovery are not obsolete. The broad candidate run was
361/362; the unnecessary starter flag was removed, and its unchanged background
sync case passes. The initial focused run was 82/83: seed 29 exposed a cold native
connection timeout. That exact failed [report](docs/network-pressure/rollup-dodgy-29-failure.json)
is preserved, not replaced by a passing repeat.

The user then requested a local fix and beta release, with a Mog adoption note
instead of a transport replacement. The failed packet trace and new regression
identify Tau's five-second client idle limit racing Iroh discovery retry.
[The correction](docs/native-discovery-timeout.md) permits discovery within the
existing connect budget; the server still negotiates five-second established-peer
silence detection. Native transfer/link/pressure pass **26/26**, plus **2/2**
slow-read cases. Both UDP outage cases retain their five-second individual progress
guards; seed 29 connects once per client with no transport errors. Final workspace/
all-target checking passes. The original 013/014 findings and device QA stay open.
Mog's runtime is unchanged; its local documentation commit is `af01924`.

Release matched **0.7.11 / protocol 22** with the existing sequential release script.
The user has authorized proceeding; the earlier stress-finding hold is superseded.
Check live beta activity immediately before deployment and leave stable untouched.
At this source checkpoint no service has restarted or package been delivered.
The prior dated records below remain historical evidence, not current branch refs.

---

# Stop/Play queue-control recovery — source only, October 1, 2026

User-authorized source merge **`efd1476`** integrates `fix/tau2-control-recovery`
through `eab961b` (fix `6422e27`) into `origin/tau2` after `1209d85`; the existing
filename-index integration is preserved. Stop retires deferred boundary actions;
Play supersedes old/stuck actions without a separate Cancel prerequisite. Queue
contents, receipt/revision fences and genuine error/restore pauses remain safe.

Two failing-before lockout regressions now pass. All 66 daemon nextest cases pass
across disjoint selections; the retained-control desktop/phone-layout test passes.
Merged native daemon/frontend all-target compilation and the real-controller
chat/queue/restart end-to-end case pass. Exact IDs and commands:
[control recovery evidence](docs/tau2-control-recovery.md).

[045 UX/control-state debt](tau2-backlog/045-play-pause-control-ux-debt.md) remains
**open**: confusing run-through/Stop semantics, insufficient interruption,
undiscoverable cancellation, technical-debt concerns and repeated Play/pause
reviews are not closed by the immediate patch. No release/package/deployment,
version/protocol/schema bump, service restart, live data edits or physical-device
acceptance; the user's running conversation was not changed.

---

# Tau 2 integration / 0.7.0 beta

The user authorized native integration, cleanup, remote `tau2`, Windows/Android
packages, and an isolated beta daemon on another port. This is not a stable cutover.
Managed Cargo, one Cargo/Rayon thread, sequential release targets, and the wrapper's
nextest concurrency limit were used. Shared-lock exit 75 was deferred, not bypassed.

## Sources and storage ownership

- Frontend `70abf41`, preserved remotely at `tau2-rust-frontend`.
- Native backend `13097eb`, then the completed SQLite agent's `cddb8b7`.
- Shared-protocol/frontend/storage merge `4171a86`.
- Master's `ef16b27` / `3818579` image-generation behavior is carried natively, not
  by restoring the TypeScript extension. The image-only bridge never retries an
  ambiguous request; staging failures explicitly warn against automatic regeneration.

I prematurely started duplicate storage work instead of leaving it to the SQLite
agent. That was a coordination mistake. It is parked **locally only** at
`scratch/tau2-storage-reconciliation` (`4603a95`), outside release ancestry. The
completed agent's schema/store is the only implementation shipped. No competing
schema, writable JSONL mirror or alternate migration path was merged.

The canonical store retains WAL/FULL transactions, durable acceptance/queue/receipts,
indexed history, bounded hot transcripts, checkpoint/suffix replay, SQL prefix copies,
revision checks, explicit read-only Tau 1 import and real SIGKILL/WAL recovery.
Reconciliation adds durable queue-control/abort receipts, reusable model-selected
starters and private portable history export within that store. Run one daemon per
database; database revisions do not make competing tool-executing daemons safe.

## Integrated runtime

- Shared owned serde protocol **12**, including full settings CAS and native notices.
- Rust daemon/agent/system/project/provider/model settings UI. Exact prompt text,
  null/default versus custom empty, staged resets and conflict preservation.
- Target-bound model/thinking/compact/priority and session context actions.
- Native optional provider titles; first prompt line without extra billing by default.
- Native Codex images and cross-provider `generate_image`, verified file delivery,
  reference replay, cancellation and no automatic regeneration after failed staging.
- Last explicitly chosen default model; favorites never change it. Cache rings remain
  provider-cache estimates, not runtime-idle clocks. Native section observation times
  persist; old imported timestamps are not reconstructed.
- Removed Kotlin/Gradle client, UniFFI/bindgen transfer surface, JVM launcher/bootstrap,
  Python title helper, extension dialogs, title-only aliases and dead legacy tests.
- Windows download/export sync uses writable handles; beta installation IDs stay isolated.

## Validation

Final managed `cargo nextest run --locked --workspace --no-fail-fast`:
**49/49 passed**, 11 binaries, 17.363 seconds of tests. Workspace all-target check
also passed. No new trivial UI/API-wrapper tests were added.

Coverage includes actual frontend controller/transport → native router → local gated
provider/tools/uploads, settings CAS, transcript cuts, forks, restart, SQL rollback,
SIGKILL/WAL recovery and control reconciliation. Native titles, cross-provider images,
export, lost outbox after image generation, and read-only shared credential rotation
are exercised without paid provider completions.

Real desktop GPU rendering validated the per-corner shader. An isolated Xvfb native
client/daemon run saved an intentionally empty system prompt and rejected a stale
settings save after another client advanced the revision. Narrow-layout review caught
and fixed duplicate global notices over the daemon settings form. Owned fixture
processes were stopped; other Xvfb/preview processes were left alone.

Physical Windows/DirectX/DPI and Android touch/IME/font acceptance remains device QA.
See `frontend/QA.md`; previous Wine checks are not claimed as physical Windows QA.

## Packages

Built sequentially with managed Cargo; Android Java tools also used one processor.

- Windows x64 native installer: **9.38 MiB**, native app **30.69 MiB**. Archive contains
  exactly the fresh `app/Tau Beta.exe`; no JVM/JAR/fonts/PDBs or external CRT installer.
- Android ARM64 APK: **11.23 MiB**, extracted library **26.33 MiB**. Package
  `app.tau.rust`, versionCode **5**, versionName **0.7.0-beta**, not debuggable.
  V3 signature matches the existing beta certificate. CRC, compressed payload identity,
  ZIP alignment and 16 KiB ELF load alignment verified.

Packages and checksums are staged in the existing Tau outbox for user delivery.

## Isolated deployment

`tau2-beta.service` is enabled and running the final release daemon:

- `http://vibe:8789` → `127.0.0.1:8791`; private transfer UDP `127.0.0.1:8792`.
  This host uses Tailscale userspace networking, which forwards incoming tailnet
  traffic to loopback; no nonexistent local Tailscale interface is bound.
- Executable `/usr/local/lib/tau2-beta/taud`.
- Own database/settings/auth under `/var/lib/tau2-beta`, private files under
  `/root/.local/share/tau2-beta`. Database, settings, auth and environment are 0600.
- Reuses the existing client connection token. Settings/catalog imported read-only;
  beta conversation storage starts fresh. Stable history was not migrated.
- Actual authenticated protocol-12 hello/settings, create/snapshot/delete, anonymous
  WebSocket rejection, WAL/integrity/foreign-key checks passed on deployed beta.
- Stable PID **81**, start timestamp, executable hash and unit hash stayed unchanged.
  Existing Tailnet routes 4178, 8787 and 8788 were preserved; only 8789 was added.

Installed daemon SHA-256:
`4127f88bbe0434b49ad78d95f5daead9d238e02dabb02dac581bb093971c14d2`.

The imported catalog has 370 models and preserves the selected `gpt-6-astra` default.
It lists Luna/Sol as `gpt-5.6-*` and DeepSeek v4 variants, not the requested GPT-6
Luna/Sol or DeepSeek v4.1 favorite IDs. Those unavailable presets remain disabled;
no capabilities or silent model substitutions were invented. Favorites can be edited
against the actual catalog in client settings.

OpenRouter uses beta's own private static-key record. Until independent Codex device
login is approved, optional `TAU_CODEX_AUTH_SOURCE` reads stable's current credential
without copying, refreshing or modifying it. An own beta login takes precedence and
refreshes independently. If the primary access token expires, shared access reports
that it needs primary refresh or independent beta login. No paid live completion was
made just to claim provider acceptance. The initial device code expired unapproved;
a fresh approval code is supplied at delivery, not committed here.


## Settings and shared-editor integration (development only)

At the user's request, integrated the completed settings handoff `446ac29` and
shared-editor/Sanscale branch through `04fd60c` into `tau2-rust-frontend` first,
then into `tau2`. Both updates are fast-forwards; the maintained frontend branch
and clean feature worktrees remain available. Local `tau2-integration` tracks
`origin/tau2`, and `/root/tau2` is back on that integration branch.

No source changes beyond the already validated editor/settings implementation;
this handoff updates branch/status documentation only. Prior test evidence and
remaining SDK/device checks are in frontend/QA.md. Version 0.7.1/protocol 13 is
still not deployed. Stable `master`, running services and production data were
not changed. The archived storage-reconciliation scratch branch is untouched;
the completed agent implementation was already integrated in `4171a86`.


## Beta 0.7.1 deployment and Windows delivery — 2026-09-24

User authorized beta deployment and the Windows EXE, explicitly waiving activity
checks and backups. Built source `6cacd154ea91263cd8786460b8c06f167901ba52` with
managed Cargo, one build job and one Rayon thread; daemon and Windows targets
were built sequentially. No backup, active-session gate or paid provider request.

- Release daemon installed atomically by the beta-only installer. Restarted only
  `tau2-beta.service`; active PID **475435** at acceptance.
- `http://vibe:8789` remains the beta URL. Local `/v1/health` reports Tau **0.7.1**,
  **protocol 13**. The installed executable matches the freshly built release.
- Stable `tau.service` PID **474496**, start time and executable hash were unchanged.
  No stable data, executable, service or route replacement.
- Built and posted `Tau-Beta-0.7.1-windows-x64.exe` (about 9.4 MiB), preserving the
  existing isolated Tau Beta installation and local-work identities. The compressed
  app payload contains exactly `app/Tau Beta.exe`, hash-matched to the new native
  Windows release binary. No JVM, extra runtime installer or debug-symbol payload.
- Linker warnings concerned unavailable Microsoft static-library debug PDBs;
  all three Windows release build stages completed successfully.
- No new Android package, GUI/device acceptance, full test rerun or release claim
  beyond these builds and deployment checks. Existing pre-protocol-13 beta clients
  need a matched update. Physical input/IME/DPI and SDK ligature checks remain open.

Daemon SHA-256:
`b714a97211d8c2f477ac3ff2f4dc84a35efc63f4ce6b3c3214cf1093888ca590`

Windows installer SHA-256:
`80dce02691ee91431916d42529983b41b3efac30790d5710454064f51994200c`


## Topics (wire/storage: projects) / beta 0.7.2 deployment and Windows + Android delivery — 2026-09-24

User authorized deployment and both platform builds. The projects working tree was
versioned as **0.7.2 / protocol 14**; Android advanced to versionCode **7**. The feature
workspace compiler check and **69/69 nextest tests** passed before the version-only
release bump. Daemon, Windows app/installer and Android ARM64 release builds then
completed sequentially with the managed Cargo wrapper, one Cargo job and one Rayon
thread. Java tools used one active processor. No paid provider requests.

- Before migration: beta had **2 chats, 0 running**, no queued messages, schema 1.
  A consistent SQLite backup (including WAL state), settings/auth/environment, old
  binary and unit are private under
  `/var/lib/tau2-beta/backups/pre-0.7.2-20260924T111416Z`.
- Restarted only `tau2-beta.service`; acceptance PID **552203**. Installed executable
  byte-matches the new release. Local health and the actual Tailnet serve endpoint
  (`tailscale nc vibe 8789`, for this userspace-networked host) report **0.7.2 / 14**.
- Authenticated live acceptance created only owned UUID fixtures and verified project
  CRUD, unchanged captured prompts after edits, fresh-chat snapshots, moving with
  prompt replacement, transcript opening, both delete choices and complete cleanup.
  Anonymous WebSockets were rejected. No existing chat was opened, moved or deleted.
- Schema-2 integrity and foreign-key checks passed. Original session metadata differs
  only by General membership; both original chats' history, receipts and queues
  compare exactly with the pre-upgrade backup. General is the only project left
  after fixture cleanup.
- Stable `tau.service` PID **474496**, start timestamp, binary and unit hashes are
  unchanged. All existing Tailnet routes, including beta **http://vibe:8789**, match
  their pre-deployment state. Stable data/service was not migrated or restarted.
- Sent `Tau-Beta-0.7.2-windows-x64.exe` and
  `Tau-Beta-0.7.2-android-arm64-v8a.apk`. Package identities, payloads, certificate,
  alignment and sizes are documented in `frontend/PACKAGING.md`. No physical-device
  UI/IME/DPI acceptance is inferred from cross-builds. Older beta clients require a
  matched update for protocol 14.

At deployment, source was in the local projects worktree (no push/tag was
requested then). The validated source was subsequently committed and pushed to
`tau2` as `c2a2f5d`, without another build or deployment. Its release snapshot
and local validation records are retained under `target/projects-release-0.7.2/`.
Package checksums are in
`dist/Tau-Beta-0.7.2-SHA256SUMS.txt`.

SHA-256:

- Daemon: `1019b82f7f6e4d4f9d995ce1166bf753aabb398b346f249e8579b2af426b8008`
- Windows installer: `00ef15e7ca5cd4590f2df92593a719148b2ca7727c153ec877c9e9a0505dfe56`
- Android APK: `4e67786b8548a521a49652e3c260b1add4fdba96704945a965244255e5d8e723`

## Beta 0.7.3 / protocol 15 — 2026-09-25

The user requested merging `feat/tau2-finished-attention` to `tau2`, deploying the
beta daemon, then sending Windows followed by Android. The feature and the 0.7.3
version bump fast-forwarded to `origin/tau2` at `ebbbc54`. Local `tau2-integration`
tracks that remote because local `tau2/*` branches prevent a local branch named
`tau2`. No unrelated branch was merged; `feat/tau2-block-sync` remains separate.

The managed workspace check and nextest passed: **103/103 tests**, no skips. The
release daemon, Windows x64 installer and Android ARM64 APK built sequentially with
one Cargo job and one Rayon thread. Java packaging used one processor. Beta was
stopped before installation; schema 2, seven chats and an empty queue were checked.
The private backup at
`/var/backups/tau2-beta/pre-0.7.3-20260925T091912Z` contains the prior binary,
unit, environment, complete stopped SQLite state, settings/auth and beta files.
Checksums verified. The new daemon was installed atomically and only
`tau2-beta.service` was started. It reports **0.7.3 / 15** locally and through
`tailscale nc vibe 8789`. Its PID was **1064959** with zero restarts; database
integrity, foreign keys, schema and all six existing tables' rows matched the
backup after startup. An anonymous WebSocket received 401. Stable `tau.service`
kept PID **474496**, its binary hash and start time; its data and routes were not
changed. No paid provider request or production prompt was sent.

The Windows installer contains exactly the new `app/Tau Beta.exe`, byte-matched to
the fresh Windows build. It was sent first. The Android APK was sent second:
`app.tau.rust`, versionCode **8**, versionName **0.7.3-beta**, not debuggable;
v3 signature matches the preceding beta signing certificate, with verified ZIP,
native library and 16 KiB ELF/ZIP alignment. Artifact checksums are in
`dist/Tau-Beta-0.7.3-SHA256SUMS.txt`. Physical Windows/Android UI acceptance and
sustained slow-link behavior were not tested by this release. The known beta
network issue from `docs/tau2-protocol-audit.md` remains until a separate network
rewrite is reviewed and released.

SHA-256:

- Daemon: `1c1cb7baf8320ac1fed78f89d5d08265781cc15dada33e3ff8271edcf577754b`
- Windows installer: `82502237e5d21cafa76c57306f9693487ca73d790d2bbde9b9f5d74bf2910608`
- Android APK: `01ff14c3f7ff2f3d15bbc81d6bc0bc604194a4c877390771af98ab0a32741aaa`


## Beta 0.7.4 / protocol 18 — 2026-09-25

The user explicitly authorized stopping the flooding old beta, merging/pushing
`feat/tau2-block-sync`, redeploying, and delivering Windows then Android, with
**no new backup**. The rewrite is actually merged: `ca87e4d` has parents `25afcdc`
and `43e40a0`, and is pushed to `origin/tau2`; the feature branch is pushed too.
The existing finished/unread attention behavior is retained.

The merged compiler check passed and **169/169 tests** passed (run
`422f6a71-95bd-4ce0-88dd-f3f6d581ee83`). These results and the completed release
daemon were reused after the user requested no redundant tests. The suite itself
took 58.6 seconds; the earlier combined command also contained 71 seconds of test
compilation and a 5m31s release build. No suite was repeated during final delivery.
Client package builds run sequentially, one Cargo job and one Rayon thread,
registry-offline through the managed Cargo wrapper. No Clippy was run.

Beta was installed atomically and started at 20:14:35 AEST, PID **1078725**, with
zero restarts during rollout checks. Local HTTP and the Tailnet serve route report
**0.7.4 / protocol 18**. Anonymous WebSocket upgrade returns 401; an authenticated
read-only handshake returns the matching Hello, a nonempty lineage and the native
IPv4/IPv6 port offer. Source migration reached schema **5**, with zero queued rows.
No paid provider request or production prompt was sent by this rollout.

This host uses **userspace Tailscale**, not a kernel TUN interface. Native UDP is
bound privately to `127.0.0.1:8792` and `[::1]:8792`; tailscaled forwards incoming
Tailnet datagrams to loopback. The installer selects actual Tailnet addresses on
kernel-mode hosts instead, and never configures a public wildcard socket. The
optional `TAU_TRANSFER_BIND_V6` fixes the advertised IPv6 port independently.
No Tailscale or stable service restart was performed.

Stable `tau.service` retained PID **474496** and binary SHA-256
`23b45c6153281bcbc76c8c86108b89ff45fe63d96d461b3c9eb1c41b33ada230`.
Its data and routes were not changed. The separate slow Tau 1 download was not
claimed fixed: passive samples found chat traffic alongside file traffic, not a
proven root cause. Beta and build/test jobs were confirmed stopped during that
investigation; no traffic shaping or network configuration was installed by tests.

The Windows installer was delivered first. Its compressed payload contains exactly
`app/Tau Beta.exe`, verified against the fresh native build and embedded installer
bytes. Microsoft SDK missing-debug-PDB linker warnings do not affect the packaged
release executable. Physical client UI acceptance is not claimed.

Android ARM64 was delivered second: `app.tau.rust`, versionCode **9**,
versionName **0.7.4-beta**, not debuggable. Its v3 signing certificate matches
0.7.3; APK CRC, embedded native-library identity, ZIP alignment and 16 KiB ELF
LOAD alignment passed. Checksums are in `dist/Tau-Beta-0.7.4-SHA256SUMS.txt`.
No physical-device or sustained WAN benchmark was added during this rollout.

Daemon SHA-256: `8420508b9de56829f486f836242930fb3396f6cf7ea31e3c5a9666e045a99774`.
Windows SHA-256: `c9f77c687b11152103614b54096651e9f7c95eea5e98b054555d8c06c94f5374`.

Android SHA-256: `5ea4e7f5a4b199bcf783048be303f186331b1daac98f1ec6285428d2e3ebcb9d`.


## Beta 0.7.6 / protocol 19 — 2026-09-27

Mobile QA was already merged on `origin/tau2` at `c491518`. The Codex account
quota feature was merged from `fix/tau2-codex-plan-usage-20260926` as `ef0de95`;
`7c423ad` raises the wire version from 18 to 19 because the feature adds a new
client command and server response. Release commit `760754b` sets version 0.7.6
and was pushed to `origin/tau2`.

The seven focused quota tests passed after the protocol bump (run
`3f4e5cfd-aad6-42ba-b568-cb5afff23802`; 128 unrelated tests skipped). The full
suite was not rerun. Tests use local fixtures; no live Codex quota request or
physical-device acceptance was performed.

The managed release script built and verified the daemon, Windows x64 installer,
and Android ARM64 APK sequentially. It then installed beta only. The beta reports
**0.7.6 / protocol 19**, PID **1351456**, and runs the exact release daemon. Before
restart, a read-only check found 23 sessions, no running sessions, no queued
prompts, one idle session with persisted `needs_turn`, and 22 sleeping sessions.
No backup was taken. Stable Tau remains active at PID **474496**; the script's
stable identity check passed and stable was not restarted or changed.

The Windows installer was sent through Tau:
`dist/Tau-Beta-0.7.6-windows-x64.exe` (10,021,888 bytes).
Android was built and verified but **not sent**. The protocol bump requires matched
clients; older protocol-18 clients need the 0.7.6 client to reconnect.

SHA-256:

- Daemon: `13c2b572ceb95e3600dccb5fd5912c45e906c38a1adee36d67086e87ac49f643`
- Windows installer: `9e8c0fc9caac3a2d68eca5e417dbe26f7e5291d2a48243ed2d4c035e7acd6be0`
- Android APK: `8dd80130b5f7742d0f468112cc3c2bfc68782d2ca5eb5e3ceb314eacfd2d761a`

Android package `app.tau.rust` is versionCode **11**, versionName **0.7.6-beta**,
not debuggable; signing identity, embedded payload, APK checksums, alignment, and
16 KiB ELF alignment passed release verification. No physical Android test was run.


## Beta 0.7.7 / protocol 19 — 2026-09-27

Downloads merge `70b0444` restores Tau1-style text actions. Release commit
`4b92d28` updates the packages to **0.7.7** and is pushed to `origin/tau2`.
Protocol 19 is unchanged. The release also includes the merged alert/content and
connection-tooltip fixes.

The managed release workflow built and verified the daemon, Windows x64 installer
and Android ARM64 APK, then deployed beta only. Beta reports **0.7.7 / protocol 19**,
PID **1429355**, and runs the release daemon (SHA-256
`8e74861fbf46084156b00fabece5d1f6f3461c7231b405244052f4f5b0e2ede7`). Before
restart, a read-only check found 26 sessions, no running sessions, no queued
prompts, one idle session and 25 sleeping sessions. No backup was taken. Stable Tau
remains active at PID **474496** and was not restarted or changed.

Windows x64 was verified and sent:
`dist/Tau-Beta-0.7.7-windows-x64.exe` (SHA-256
`b4872c942783a128653264cdd7999a7da384689da4ab71ee5e584504a281c0e4`). Android
ARM64 was verified and sent: `dist/Tau-Beta-0.7.7-android-arm64-v8a.apk`, package
`app.tau.rust`, versionCode **12**, versionName **0.7.7-beta** (SHA-256
`2fa3befe4f8cc96b212a84a61556a686853d9c30a580d0d60cbe7f3aa4a2a494`). Both
package verifiers passed. No physical Windows or Android acceptance was run.

The five focused downloads tests passed on the feature branch. A merged-tree nextest
retry timed out waiting for the shared Cargo lock before the tests started; it was
not a test failure. No full suite was rerun for this release.


## Beta 0.7.8 / protocol 20 — packages attached, daemon held for download acknowledgement

The user prioritized the thinking-level indicator above the text box, corrected
the merge destination from master to `tau2-integration`, and explicitly required
Windows/Android downloads to complete before any daemon restart. Feature
`cdc33ca` was committed/pushed on `fix/tau2-composer-thinking`, merged as `af0e5e0`,
and versioned at release `b74af7e`, published to `origin/tau2`. Master is unchanged.
The release includes the already-integrated checkpoint-watch fix `2517e9c` and its
protocol-20 requirement. The optional `thinkingLevel` summary field adds no further
protocol or storage-schema version change.

The composer uses the saved per-chat level, including `off`, and reserves its
label before shortening a long model slug. The same value prefills the thinking
editor and persists offline. Catalogue revision fencing covers thinking changes;
legacy account caches read missing metadata as unknown, not a guessed default.
Three feature nextest tests and fourteen related activity/catalogue/offline-create
regressions passed on the feature source. Desktop, narrow-phone and scaled-phone
GPU shaping/rendering were exercised and screenshots inspected. No full suite,
physical Windows/Android acceptance or paid provider completion was performed.

The release automation ran **without `--deploy`**, with managed Cargo, one build
job/Rayon thread and sequential daemon/Windows/Android builds. Windows embedded
native app/launcher verification and Android package identity, signature, payload,
ZIP CRC/alignment and 16 KiB ELF alignment checks passed. Android remains
`app.tau.rust`, versionCode **13**, versionName **0.7.8-beta**, not debuggable, signed
with the existing beta identity. Receipts/logs and the ordered delivery manifest
are in `/root/tau2-integration/dist/releases/0.7.8/`.

Both `send_file` calls succeeded (queued attachments), Windows followed by Android:

- `Tau-Beta-0.7.8-windows-x64.exe` — SHA-256
  `a993066047e49611bdd154827983e6b43160fb14eaa857e69c81f181a40e9e72`.
- `Tau-Beta-0.7.8-android-arm64-v8a.apk` — SHA-256
  `cf6e895beb63969c0e7fd8656391382b595b6391cdd00a55e33e76470b7b63a7`.

**Download acknowledgement is still pending. Do not restart yet.** Read-only
before/after service checks matched: beta PID **1467375**, stable PID **474496**,
both active with unchanged monotonic start times. Beta health still reports
**0.7.7 / protocol 19**. No service install/stop/restart, backup, production data
change or stable deployment was done. The new clients need the pending matching
protocol-20 daemon; after explicit download confirmation only, resume with
`scripts/release-beta.sh --version 0.7.8 --push --deploy` from the integration
worktree. Documentation-only commits do not invalidate the verified build receipts.


## Retained UI and repeated-pause fix — integrated source, not deployed

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


## ZIP Extract QA fixes — source integration only

At the user's request, merged `fix/tau2-extract-behavior` (`a66fc3d`) into local
`tau2-integration` as **`3eb2b1d`**, publishing to `origin/tau2`. Integration was
clean and current at `6da3b4c`; the merge was conflict-free. The feature history
and worktree remain preserved. This is not a stable/master merge or a deployment.

- Removed Extract's remaining hover/context tooltip.
- A shared scoped claim disables Extract in chat and Attachments until extraction
  and opening finish, including repeated clicks before repaint. Failure permits
  retry; old-source completions cannot clear a different job.
- Restored Tau1's single-root ZIP handling: publish/open the actual root, not a
  redundant archive-named outer folder. Loose roots and collision suffixes remain.
- Merge and feature tip have the same tree
  `9d2efa90520c74b61da2b4059989832fae6eac5c`; compared with tested implementation
  `637a5b0`, only Markdown notes differ. Reused the **159/159 frontend library
  nextest** result (`00ff05dc-6818-4807-900f-708d9966934d`), native all-target check,
  Windows MSVC check and Android ARM64 check. No new test/rebuild claim.
- No packages, service restarts, production data writes, version/protocol/schema
  changes or physical-device acceptance. Existing matched-release/deployment
  requirements and holds remain. See `docs/zip-extraction.md` for full evidence.


## Local fuzzy picker — source integration only

At the user's request, merged `fix/tau2-fzf-picker` (`7963270`) into
`tau2-integration` as **`4501584`**, publishing to `origin/tau2`. Integration was
clean and freshly fetched at `c13c670` (0.7.9); the merge was conflict-free. Merge
preparation used `/root/tau2-fzf-merge` on `merge/tau2-fzf-picker`, leaving the
feature branch and worktree intact.

- Local fzf-style matching with background revisioned name sync; no query RPC.
- Highlighted results, keyboard navigation and selected-file preview underneath.
- Hidden-file toggle, explicit counts/partial-index status and no 100-result cap.
- Distinct working Chat and X controls; no idle redraw loop from prefetch.
- Merge tree **`3a911f9ffbf6043d1e501010d7543b0728b6b7e5`** exactly equals the tested feature tip.
  Reused the final **25/25 focused nextest** result
  (`adf430d5-373b-418f-a174-f1acf4b3d186`), including all three corrected idle
  regressions from the earlier 325/328 broad run, and the native all-target /
  Windows MSVC / Android ARM64 compiler checks. Only integration documentation
  changes follow the identical-tree merge. No new test or build claim.
- **No deployment, service restart, release packages, provider calls, production
  data writes or stable/master changes.** Running beta remains unchanged. Protocol
  **22** requires a future matched client/daemon release; version stays 0.7.9 in
  this source-only integration, with no schema change. Physical-device QA remains
  open. See `docs/remote-code-viewer.md` for feature limits and validation details.


## Filename-index filtering — source integration, October 1, 2026

At the user's request, merged `fix/tau2-file-index` tip `a395f27` into
`tau2-integration` / `origin/tau2` as **`67e0dd8`**, on top of freshly fetched
`40ac1b0`. The worktree was clean and the merge conflict-free. The independent
wheel-scroll wake fix already in the integration branch is preserved, as are the
filename feature branch/worktree and their three implementation/audit commits.

- Recursive fuzzy indexing prunes dot-directory descendants before descent.
  Dotfiles remain toggleable; hidden directories and ignored files stay browsable,
  and explicit Here roots remain supported. Query edits are still entirely local.
- Actual development-tree audit: 93,946 eligible names, same 93,484 visible files,
  no scan limit, 0.73 MiB compressed initial filename payload, and 10.02 MiB client
  name/record heap lower bound. These are one host's aggregate measurements, not a
  total-RAM, network-throughput or universal drive-coverage guarantee.
- Every filename feature source/test/dependency file exactly matches `a395f27`.
  Reused its managed all-target compiler check and **21/21 focused nextest** run
  `cc38e41f-c970-4556-ac24-0a9d932bcf28`; no tests, builds or audits repeated and no
  new merged-tree test result claimed. `git diff --check` and local doc links pass.
- A wake-driven native/headless picker regression passes; no filename-specific
  missed repaint was reproduced. The separately owned Windows socket/loading
  diagnosis and physical-device acceptance remain open.
- **No deployment, service restart, release packages, production storage writes,
  provider calls, stable/master changes or version/protocol/schema bump.** Source
  remains 0.7.10 / protocol 22. See `docs/file-name-index.md` for evidence and the
  bounded scaling follow-ups.


## Markdown URL underlines — source integration, October 1, 2026

At the user's request, merged `fix/tau2-link-underline` (`6395a36`) into
`origin/tau2` as **`4ca14ca`**, based on freshly fetched `4afd665`. The merge was
conflict-free and prepared in `/root/tau2-link-underline-merge` on its own branch;
the feature branch/worktree remain preserved.

- Link underlines now sit 0.1 em below each shaped line's measured baseline,
  instead of at a fixed line-top offset that could cross the glyphs. Link color,
  one-pixel thickness, span widths, activation and strikethrough are unchanged.
- The real-font regression failed before the fix: the underline was 0.0237 em
  above the baseline at 12 px. It now covers wrapped/styled links, explicit line
  breaks, headings, quotes, table cells, font sizes and translated/scrolled scenes.
- Merged-tree managed Markdown/frontend all-target compiler check passed, with
  existing dead-code warnings. Focused nextest **30/30 passed**, with 162 unrelated
  frontend tests filtered out (`300eb6e1-ccca-4b9d-b629-8124075b1068`). This includes
  all 27 Markdown tests and real frontend GPU pixel checks at 12, 16 and 32 px.
- **Source only:** no deployment, service restart, packages, production data writes,
  stable/master changes or version/protocol/schema bump. Headless rendering is
  not physical Windows/Android acceptance; existing rollout holds remain intact.


## Text-cache residency — source integration, October 1, 2026

Approved as a residency/bookkeeping improvement for the next iteration cycle,
**not as a confirmed fix for the user's periodic blank/text-missing frame**.
Merged `fix/tau2-text-frame-flash` (`7f961d7`) on top of freshly fetched
`b76386c` as **`1350fd2`**, in `/root/tau2-text-residency-merge` on its own merge
branch. Existing filename, scroll, controls and URL-underline integrations are
preserved. Original feature history/worktrees remain available.

- Sanscale now counts valid cached preparation as block-cache use, without
  re-shaping or adding GPU uploads. The real-capacity pixel reproduction is
  protected; cold eviction, pool bounds and stale-input rejection remain intact.
- The dependency fix is also integrated into Sanscale master via `48b5234`,
  published at **`8a290e9`**. Its merged library/example/integration nextest passed
  **98/98**, including ignored GPU cases, and strict private-item rustdoc passed.
  Tau retains the compatible **`4325844`** pin rather than importing unrelated
  upstream public-API naming changes.
- Merged native frontend/Markdown all-target check passed with existing warnings.
  Their library nextest passed **195/195, zero skipped**
  (`45f0d8b2-0263-4ec4-abb8-de0399f8fab6`). This includes a new ordinary heartbeat
  repaint/pixel check which also passes with the old dependency: it is additional
  coverage, **not** proof of a repaired periodic timer bug.
- Preparation gains a small age-store cost. One release smoke with 16,384 layouts
  measured 0.812 ms before / 0.824 ms after for CPU preparation; this is a bounded
  host observation, not a general performance guarantee.
- **The periodic-blink QA remains OPEN.** The user expects actual testing in the
  next iteration; neither a real-session cache sweep nor a platform cause has
  been established. See `docs/text-repaint-stability.md` for the reproduction,
  explicit limits, merged-tree evidence and remaining diagnosis.
- **Source only:** no deployment, service restart, release packages, production
  data writes, stable/master change in Tau, or version/protocol/schema bump.


## Model catalog / manually entered GPT-6.1 Sol — source integration, October 2, 2026

At the user's request, merged `fix/tau2-model-catalog-refresh` (`59c87a2`) into
`origin/tau2` as **`d528e56`**, from freshly fetched `8cceeb8`, in the independent
`/root/tau2-model-catalog-refresh-merge` worktree. The merge is conflict-free and
its tree exactly matches the tested feature tree. Existing feature history and
worktree are preserved.

- The user's manually entered quick model remains a direct exact-ID selection.
  Updated Codex catalog compatibility gating discovers GPT-6.1 Sol; cache age,
  missing IDs and old client-version stamps trigger bounded asynchronous
  revalidation on use without expiring last-good same-identity limits.
- Refreshed capacities propagate to retained warm/sleeping token usage with new
  revisions, preserving token counts, run state and idle deadlines. Cold chats
  are not warmed, and catalog GETs never execute a model turn.
- Reused the feature's managed native daemon/frontend all-target compiler check
  and **13 distinct passing targeted cases** across the documented runs. Both
  original regressions fail before the fix; the actual-controller manual-model
  case passes with a gated local catalog. No tests/builds were rerun for this
  identical-tree merge and no new merged-tree test result is claimed. Diff and
  new documentation links pass. See [exact evidence](docs/model-catalog-refresh.md).
- **Source only:** no deployment, service restart, packages, production
  cache/settings/auth write, live completion request, stable/master change or
  package-version/protocol/database-schema bump. Beta remains **0.7.10 / 22**;
  physical Windows/Android acceptance and existing rollout holds are unchanged.


## Android startup lock contention — source integration, October 2, 2026

At the user's request, merge **`1321f65`** integrates
`fix/tau2-android-startup-lock` through `1a8ee0c` into `origin/tau2`, from freshly
fetched `d3de9ad`, in `/root/tau2-android-startup-merge` on the separate branch
`merge/tau2-android-startup-lock`. The intervening catalog/retained-usage fix is
preserved. Only HANDOFF/QA prepends conflicted; both histories are retained.

- Current authored/replica opens no longer repeat initialization writes.
  Optional read-LRU and local-body-reuse work cannot block first-frame rendering
  or report a busy-cache popup for already-saved pending intent. The short
  directory lease/GC handoff waits normally instead of failing `try_lock`.
  Live leases/quota, actual-write timeout/durability, migration rollback and
  authored work remain protected; Android startup stage logs retain diagnosis.
- Startup source/tests exactly match the accepted feature; catalog source/test
  files remain unchanged. Fresh merged native daemon/frontend all-target check
  and **12/12 targeted nextest cases** pass, including real phone-size first-frame
  pixels/uncertain intent under a held writer and the actual-controller catalog
  regression (`a8923dc4-52c5-4251-b184-4ff75b449ae3`). Reuse the feature's 210-case
  suite and unchanged frontend Android/Windows/rustdoc checks; no fresh full
  merged-suite or platform run is claimed. See [evidence](docs/android-startup-lock.md).
- **Source only:** no release packages, deployment, service restart, live data or
  credential access, stable/master change or version/protocol/schema bump.
  The user's exact physical-phone recurrence remains unconfirmed; no device was
  attached. No Clippy or built-in Cargo test runner. Other QA/release holds remain.


## File viewer retention / Android selection — October 2, 2026

User-requested source fixes for `tau2-integration` / `origin/tau2`, prepared from
`6dbcf02` in the required independent worktree. The tested tip `06fe7c0` was
fast-forwarded into `tau2-integration` and pushed to `origin/tau2`, without another
build/test run for the identical source tree. `fb74424` retains recent per-chat
file buffers and Find/Browse reading position; `173e88e` adds mobile selection
handles, hold-and-drag/autoscroll and a nonmodal Android clipboard toolbar.
Account/source/editor generation boundaries, drafts and unchanged desktop mouse
editing remain protected. See [exact behavior and evidence](docs/viewer-and-touch-selection.md).

Frontend all-target and Android/Windows library checks pass, frontend library plus
real native remote-file nextest **181/181 pass**, and final strengthened-fixture
focus **8/8 passes**. An isolated Android 36 emulator runs the production Java
bridge with a minimal native window/input stub: visible toolbar pixels, first
touch pass-through, Copy/Select All, stale callbacks, password protection and
detach cleanup pass. This is not full Android client or physical-phone acceptance.

No release installers, service restart/deployment, production data write, stable
change, or version/protocol/schema bump. Installed 0.7.11 clients are unchanged.


## Codex recovery / beta 0.7.12 — October 5, 2026

Merged `fix/tau2-codex-signin` (`e5eb159`) as `996112c`, released source `344fa25`
published to `origin/tau2`. Shared Codex renewal now joins deployed Pi's existing
credential lock and writes back to the shared file, without making a beta copy.
Native device sign-in is only a fallback; error metadata wraps and copies with
partial output. No added runtime/dependency/service. Protocol 24, same DB schema.

Managed all-target daemon/frontend check, 8 focused auth/UI cases, 18 existing form
cases, 9 final shared-auth cases, and one actual deployed-Pi cross-runtime lease
case pass; the final Copy guard's focused render/action case passes too. See
[implementation and validation](docs/codex-signin.md). No physical Windows or live
browser sign-in certification.

The existing release script built and verified daemon, Windows and Android
sequentially; receipts under `dist/releases/0.7.12/`. Windows was queued through
`send_file` with instructions to wait before installing. Android remains unsent.
**Deployment held:** one active beta chat at 09:40–09:44 UTC. Running beta remains
0.7.11 / protocol 22, PID 3209092. Stable PID 474496 unchanged. Resume the same
release with `--push --deploy` once idle or explicitly authorized to interrupt.
