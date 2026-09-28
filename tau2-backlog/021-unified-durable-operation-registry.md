# 021 — One durable operation registry for local intent

Kind: **Implementation task with a storage migration.**
Group: **State and replication / operations**; related tasks 020 and 025, not an umbrella authorizing both.

Status: **Open; requires its own migration/recovery design before implementation.**
Priority: **Later; separate from 015.**
Confidence: **6/10** in the value and appropriateness of this scope, not a probability of a LOC saving.
Net production LOC reduction estimate: **-200 to +100 lines**.

Recorded September 28, 2026. Evidence: the detailed `40a3698` investigation plus
relevant changes checked at `origin/tau2` = `cafef7f`. These are planning ranges,
not implemented or measured reductions. Positive means fewer lines; negative
means added code. Count changed production Rust across frontend, daemon and shared
crates; moving code is zero. Tests/docs/manifests are excluded unless explicitly
listed. See README for the confidence scale, baseline and overlap rules.

## Why / evidence

`Account.pending_create`, `Account.pending_controls`, `LocalChat.pending`, controller
model-selection waiters and request/receipt queues. See
`frontend/src/{store,controller}.rs` and the operations section of
`docs/tau2-client-state-proposal.md`.

Confidence rationale: Three durable intent containers produce repeated lifecycle
bookkeeping. A consolidation can help, but migration, immutable submission and different
recovery semantics are real costs.

## Proposed scope

Create one registry keyed by original operation ID with source binding, typed
intent/target, authored references, dependencies, preparation/frozen submission,
delivery/outcome and explicit recovery policy. Per-chat lists become indexes. Design a
versioned transactional migration preserving all existing IDs and exact prepared
payloads.

Out of scope: No universal automatic-retry rule or claim of exactly-once external
execution. Do not replace daemon effect reservations or change wire outcomes as part of
this local-store migration.

## Acceptance

- Interrupt migration and restart without lost drafts/files, changed intent IDs or
  duplicated effects. Reject unsupported future schemas safely.
- Persist before network submission; preserve create aliases, attachment preparation,
  model dependencies and unresolved outcomes.
- Clearing replica data cannot clear the registry; changing source cannot automatically
  retarget old work.

## Honest impact estimate

Allow 250–400 lines of duplicated registry/reconciliation bookkeeping to be replaced,
but 300–450 lines of common operations, migration and compatibility logic may be needed:
-200 to +100 net. Retained migration code is counted, not wished away.

## Dependencies, overlap and current-source notes

Includes the 020 policy if not already extracted. Overlaps 017 references and the local
half of 025; those pieces must be priced once. 019 supplies organization, not extra
savings.

All three durable containers still exist at cafef7f. New prefetch state is unrelated and
is excluded from the removal budget.

No implementation, runtime/device certification or Rust test run is part of this
backlog entry. Work only with managed Cargo compiler checks and relevant nextest
coverage when implementation is authorized. Preserve existing deferred tasks.
