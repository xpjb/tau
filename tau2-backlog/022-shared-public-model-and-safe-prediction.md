# 022 — Shared public state semantics and safe local prediction

Kind: **Design context — not a separately schedulable world-model rewrite.**
Initiative: **State and replication**; the pilot described below needs its own bounded scope before selection.

Status: **Design context; any future pilot needs a separately bounded implementation decision.**
Priority: **Reference for shared contracts; no independent implementation slot.**
Confidence: **5/10** in the value and appropriateness of this scope, not a probability of a LOC saving.
Net production LOC reduction estimate: **-250 to +100 lines**.

Recorded September 28, 2026. Evidence: the detailed `40a3698` investigation plus
relevant changes checked at `origin/tau2` = `cafef7f`. These are planning ranges,
not implemented or measured reductions. Positive means fewer lines; negative
means added code. Count changed production Rust across frontend, daemon and shared
crates; moving code is zero. Tests/docs/manifests are excluded unless explicitly
listed. See README for the confidence scale, baseline and overlap rules.

## Why / evidence

`daemon/src/blocks.rs`, `frontend/src/blocks.rs`,
`protocol/src/{blocks,transcript,lib}.rs`, `daemon/src/operations.rs`, and
`docs/tau2-shared-replication-proposal.md`. The Quake III comparison motivated shared
behavior, confirmed state plus pending intents, baselines and interest selection; it
does not justify new game-style transport.

Confidence rationale: Both ends are Rust and currently mirror public metadata
conventions. Sharing pure transitions is plausible, but most agent execution is neither
predictable nor safely replayable, and much storage/transport code is already shared.

## Proposed scope

Beyond the tool contract, pilot shared public records and pure transformations for a
small pair of existing operations such as rename and queued-text edit. The daemon
validates authority/current revision and commits; a client may derive a marked tentative
view from confirmed state and unresolved intent. Keep committed-state replay separate
from effect execution.

Out of scope: No arbitrary object replication, full daemon-state serialization, fixed
tick, packet codec replacement or whole-application reducer. No immediate
operation-store migration.

## Acceptance

- The same immutable operation has one identity through client A, daemon and client B.
  Only A has its pending overlay.
- Resolve conflicting edits and lost acknowledgements without replaying agent/tool
  effects or treating a prediction as permission.
- Public projection excludes provider-private state; partial client knowledge and body
  availability stay explicit. Reject or clearly mark unsupported predictions.

## Reference estimate — not an additional implementation budget

This scenario is retained for design comparison. Do not add it to child-task
estimates or schedule the parent/design record as another implementation.

For the additional pilot beyond 015, perhaps 100–250 lines of mirrored interpretation
can be replaced by 150–350 lines of shared contracts, pure transitions and host
adapters: -250 to +100 net. A full shared-world rewrite is not scoped tightly enough to
promise a reduction; this estimate must not be extrapolated to it.

## Dependencies, overlap and current-source notes

Exclude tool savings already counted in 015. Broader transcript changes overlap 016,
recovery policy 020, and local operation ownership 021. Count shared primitives once.

Existing tau-protocol/tau-blocks/tau-transfer sharing remains substantial at cafef7f.
New navigation and prefetch code are not evidence of duplicated server simulation.

No implementation, runtime/device certification or Rust test run is part of this
backlog entry. Work only with managed Cargo compiler checks and relevant nextest
coverage when implementation is authorized. Preserve existing deferred tasks.
