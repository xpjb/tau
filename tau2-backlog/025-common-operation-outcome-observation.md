# 025 — Common observation of authoritative operation outcomes

Status: **Open; separate from local registry migration and effect execution.**
Priority: **Later; justify a small end-to-end outcome slice first.**
Confidence: **5/10** in the value and appropriateness of this scope, not a probability of a LOC saving.
Net production LOC reduction estimate: **-100 to +80 lines**.

Recorded September 28, 2026. Evidence: the detailed `40a3698` investigation plus
relevant changes checked at `origin/tau2` = `cafef7f`. These are planning ranges,
not implemented or measured reductions. Positive means fewer lines; negative
means added code. Count changed production Rust across frontend, daemon and shared
crates; moving code is zero. Tests/docs/manifests are excluded unless explicitly
listed. See README for the confidence scale, baseline and overlap rules.

## Why / evidence

`daemon/src/operations.rs::{reserve_operation, operation_outcome, operation_receipt,
recover_operations}`, controller Response/Operation/receipt handling, and protocol
operation summaries. Prompt/queue receipts already have a separate durable lifecycle.

Confidence rationale: Response/receipt/state reconciliation is scattered, but these are
not interchangeable acknowledgements. A shared outcome contract can help; publishing all
outcomes as replicated records is still an unproven simplification.

## Proposed scope

Give operation observations common typed IDs and explicit
accepted/completed/failed/uncertain semantics. Start with one existing operation kind.
Evaluate reuse of bounded record observation instead of bespoke polling only if it
deletes handlers; preserve immutable reservations and the original request identity.

Out of scope: No universal transaction spanning filesystem/provider effects; no collapse
of all prompt, queue and generic-control policies into identical retry behavior.

## Acceptance

- A lost response followed by reconnect recovers the original outcome without
  reexecuting the command.
- Accepted, terminal outcome and installed corresponding state remain distinct; no
  premature local-body or overlay retirement.
- After interruption, external effects may remain uncertain. Do not expire durable
  operation ownership with disposable descriptor/upload leases.

## Honest impact estimate

Roughly 100–180 lines of outcome-observation glue may be replaced by 100–200 lines of
common records/adapters: -100 to +80 net. Server reservation, outcome retention and
uncertain-effect recovery are necessary and not deletion candidates.

## Dependencies, overlap and current-source notes

Overlaps the observation side of 021/022 and may share collection infrastructure with
024. The estimate excludes the local-store migration; coordinated implementations must
be re-estimated together.

Generic operation source code is unchanged between 40a3698 and cafef7f. Controller line
numbers moved; its receipt/response ownership rules still apply.

No implementation, runtime/device certification or Rust test run is part of this
backlog entry. Work only with managed Cargo compiler checks and relevant nextest
coverage when implementation is authorized. Preserve existing deferred tasks.
