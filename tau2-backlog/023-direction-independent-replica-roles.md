# 023 — Reusable publisher/receiver roles for replication

Kind: **Conditional proposal — not currently recommended as standalone work.**
Group: **State and replication / infrastructure**; requires a concrete host beyond the existing roles.

Status: **Open, conditional; no standalone generalization recommended now.**
Priority: **Only with a concrete additional host/use case.**
Confidence: **4/10** in the value and appropriateness of this scope, not a probability of a LOC saving.
Net production LOC reduction estimate: **-400 to 0 lines**.

Recorded September 28, 2026. Evidence: the detailed `40a3698` investigation plus
relevant changes checked at `origin/tau2` = `cafef7f`. These are planning ranges,
not implemented or measured reductions. Positive means fewer lines; negative
means added code. Count changed production Rust across frontend, daemon and shared
crates; moving code is zero. Tests/docs/manifests are excluded unless explicitly
listed. See README for the confidence scale, baseline and overlap rules.

## Why / evidence

`transfer/src/blocks.rs::{Backend, Client, Acceptor}`, `blocks/src/lib.rs::{feed,
cache_page, cache_lineage}` and the bidirectional-flow discussion in
`docs/tau2-shared-replication-proposal.md`. `cache_lineage` binds one cache to one
source; alternating authorities through it clears prior state.

Confidence rationale: Role reuse is a sound direction, but the current
one-daemon/many-client flow already shares source/cache/transport primitives. No
second-authority product requirement or measured missing generic engine was established.

## Proposed scope

When an actual host needs both roles, reuse publisher and committed-receiver interfaces
with explicit authority, scoped identities, interests and durable progress. Prove
client→server→client and server→client→server handling without interpreting returned
state as a new command. Require an identified consumer before adding new abstraction
layers.

Out of scope: No current mandate for peer-to-peer listeners, multiwriter merge/CRDTs, a
new wire protocol or general distributed database. This records an opened design topic,
not an approved feature.

## Acceptance

- Forwarded facts keep identity/revision; accepted intents produce distinct
  authoritative outcomes. State replay never schedules execution.
- Keep metadata progress separate from verified body coverage, and respect
  reset/tombstone rules and interest withdrawal versus deletion.
- If multiple authorities become a requirement, isolate stores/namespaces and progress
  explicitly; do not silently reuse the current single-lineage cache.

## Honest impact estimate

No independently duplicated production algorithm was identified for this generalization.
Budget zero saving and a possible 0–400 added lines for role/authority adapters: -400 to
0 reduction. A truly multi-authority feature would need a separate estimate, not be
hidden in this range.

## Dependencies, overlap and current-source notes

Do not claim movement of existing blocks/transfer code as a reduction. Shared-receiver
work is separately 026; shared domain transitions are 022; packaging is 027.

Current cafef7f still has one authenticated daemon lineage per cache. Background
multi-chat synchronization is not multi-authority replication.

No implementation, runtime/device certification or Rust test run is part of this
backlog entry. Work only with managed Cargo compiler checks and relevant nextest
coverage when implementation is authorized. Preserve existing deferred tasks.
