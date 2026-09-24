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
