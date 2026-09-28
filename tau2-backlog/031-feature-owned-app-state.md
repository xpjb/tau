# 031 — Feature-owned App state rather than more impl App files

Kind: **Design context — not a separately schedulable App rewrite.**
Initiative: **UI ownership and interaction**; implemented through justified slices such as 028–033.

Status: **Design context; preserve existing navigation ownership, do not schedule a blanket App rewrite.**
Priority: **Reference for UI slices; no independent implementation slot.**
Confidence: **6/10** in the value and appropriateness of this scope, not a probability of a LOC saving.
Net production LOC reduction estimate: **-120 to 0 lines**.

Recorded September 28, 2026. Evidence: the detailed `40a3698` investigation plus
relevant changes checked at `origin/tau2` = `cafef7f`. These are planning ranges,
not implemented or measured reductions. Positive means fewer lines; negative
means added code. Count changed production Rust across frontend, daemon and shared
crates; moving code is zero. Tests/docs/manifests are excluded unless explicitly
listed. See README for the confidence scale, baseline and overlap rules.

## Why / evidence

`frontend/src/app.rs::App`, `app/attachments.rs`, `app/projects.rs` and current
`app/navigation.rs`. Candidate owners include transcript layout,
attachment/export/viewer state and modal/focus state.

Confidence rationale: App owns many unrelated fields and split files still operate
through impl App. Narrower ownership can reduce coupling, but struct/method relocation
is not a LOC reduction.

## Proposed scope

Move coherent remaining state and lifecycle into feature-owned structures with narrow
inputs and typed actions. App coordinates them. Use existing Navigation as established
ownership rather than inventing a parallel selection owner. Introduce common component
interfaces only if multiple real consumers need them.

Out of scope: No blanket component framework, domain ClientState rewrite or semantic
credit for other backlog entries.

## Acceptance

- A feature cannot silently mutate unrelated App state through super::*-wide access;
  lifecycle/reset ownership is explicit.
- Preserve account/chat binding, async completion, viewer/export cancellation and
  existing navigation behavior.
- Demonstrate reduced cross-feature coordination, not merely a shorter app.rs. Preserve
  renderer and Editor reuse.

## Reference estimate — not an additional implementation budget

This scenario is retained for design comparison. Do not add it to child-task
estimates or schedule the parent/design record as another implementation.

For ownership-only reorganization, expect 0–120 added production lines (-120 to 0
reduction) from owner types/interfaces/constructors. Existing behavior mostly moves. Any
real algorithm deletion belongs to its corresponding semantic item, not this
packaging-of-fields task.

## Dependencies, overlap and current-source notes

Related to 028/030/032/033; exclude their algorithmic savings. 019 is domain/client-host
ownership, while this item concerns App UI feature state.

cafef7f already has Navigation and centralized selection/download navigation. Mark that
portion as existing work and focus only on remaining feature coupling.

No implementation, runtime/device certification or Rust test run is part of this
backlog entry. Work only with managed Cargo compiler checks and relevant nextest
coverage when implementation is authorized. Preserve existing deferred tasks.
