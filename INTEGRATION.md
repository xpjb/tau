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
