# 027 — One native sync library with storage/transport modules

Status: **Open; optional packaging decision, not a LOC-reduction priority.**
Priority: **Only alongside a justified API boundary change.**
Confidence: **6/10** in the value and appropriateness of this scope, not a probability of a LOC saving.
Net production LOC reduction estimate: **0 lines**.

Recorded September 28, 2026. Evidence: the detailed `40a3698` investigation plus
relevant changes checked at `origin/tau2` = `cafef7f`. These are planning ranges,
not implemented or measured reductions. Positive means fewer lines; negative
means added code. Count changed production Rust across frontend, daemon and shared
crates; moving code is zero. Tests/docs/manifests are excluded unless explicitly
listed. See README for the confidence scale, baseline and overlap rules.

## Why / evidence

`blocks/Cargo.toml`, `transfer/Cargo.toml`, `daemon/Cargo.toml`, `frontend/Cargo.toml`
and the two crate roots. tau-transfer predates tau-blocks; current storage and network
roles are distinct even if packaged together.

Confidence rationale: Both applications depend on both crates and transfer already
depends on blocks. A unified API is plausible, but separate crates currently enforce a
dependency boundary and allow storage-only builds without the networking stack.

## Proposed scope

If consolidation helps the public API, place durable storage and transport in separate
modules of one sync crate while preserving caller-owned transactions and policy
boundaries. Migrate imports/manifests/tests mechanically; avoid a second facade
retaining both entire public APIs indefinitely.

Out of scope: No new generic replication engine or receiver implementation credited to
this item. Storage and network responsibilities remain separate internally.

## Acceptance

- Both daemon and client use the consolidated library without wire/schema or runtime
  behavior changes.
- Storage operations remain composable in the caller transaction; network waits do not
  become part of DB critical sections.
- Retain relevant storage/transport behavioral tests and measure build/dependency
  consequences rather than assuming packaging speeds anything up.

## Honest impact estimate

Expected algorithmic/production LOC reduction: approximately 0. Most code and tests
simply move. Imports/re-exports may fluctuate by a few lines; manifest/doc deletion is
not production Rust savings. Any useful receive-coordination reduction belongs to 026,
not this rename/merge.

## Dependencies, overlap and current-source notes

026 may motivate a unified API; 022/023 may inform its eventual boundaries. None of
their reductions can be counted again here.

At cafef7f the dependency direction is still tau-transfer → tau-blocks → tau-protocol,
with both application consumers using both. No third storage-only application consumer
was found in workspace manifests.

No implementation, runtime/device certification or Rust test run is part of this
backlog entry. Work only with managed Cargo compiler checks and relevant nextest
coverage when implementation is authorized. Preserve existing deferred tasks.
