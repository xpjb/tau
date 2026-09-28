# 019 — Cohesive ClientState ownership around the shared model

Kind: **Design context — not a separately schedulable refactor.**
Initiative: **State and replication**; used by transcript, operation and catalogue tasks.

Status: **Design context; not a standalone implementation or LOC-reduction project.**
Priority: **Reference when defining state owners; no independent implementation slot.**
Confidence: **6/10** in the value and appropriateness of this scope, not a probability of a LOC saving.
Net production LOC reduction estimate: **-150 to 0 lines**.

Recorded September 28, 2026. Evidence: the detailed `40a3698` investigation plus
relevant changes checked at `origin/tau2` = `cafef7f`. These are planning ranges,
not implemented or measured reductions. Positive means fewer lines; negative
means added code. Count changed production Rust across frontend, daemon and shared
crates; moving code is zero. Tests/docs/manifests are excluded unless explicitly
listed. See README for the confidence scale, baseline and overlap rules.

## Why / evidence

`frontend/src/controller.rs::{Controller, Chat}`, `frontend/src/store.rs::{Account,
LocalChat}`, `frontend/src/feed.rs::Feed`, plus `docs/tau2-client-state-proposal.md`.
Catalog, navigation, local intent, content and connection concerns currently meet in the
controller.

Confidence rationale: The current state is spread across Account, LocalChat, Feed and
Controller bookkeeping. Ownership would be clearer, but relocating fields into named
owners does not itself delete behavior.

## Proposed scope

Organize the client host into catalog, loaded chats, operation references, bounded
content access, transfers, navigation/settings and source/connection ownership.
Lists/indexes hold references into owners. Keep sockets, SQLite handles and platform
effects in services; keep durable authored data separate from disposable replica state.

Out of scope: This entry credits only ownership/reorganization. Transcript, operation,
transfer and UI algorithms have separate entries; it is not permission to implement all
of them together.

## Acceptance

- Each datum has an explicit owner; feature transitions use narrow interfaces rather
  than coordinated copies.
- Do not load all chats/history to instantiate ClientState. Preserve source identity,
  recovery, migration compatibility and bounded working sets.
- Adopt already-landed navigation and recent-chat ownership rather than replacing them
  to match a diagram.

## Reference estimate — not an additional implementation budget

This scenario is retained for design comparison. Do not add it to child-task
estimates or schedule the parent/design record as another implementation.

Expected production impact of organization alone: 0–150 added lines, or -150 to 0
reduction, for owner types and interfaces. Moving Controller fields/methods between
modules counts as zero. No child-project savings are included.

## Dependencies, overlap and current-source notes

Umbrella context for 016–018, 021, 024/025 and 033; their behavior changes must not be
attributed again here. Distinct from App UI feature ownership in 031.

cafef7f introduces `app/navigation.rs`, `DownloadTarget`, recent-chat timestamps and
background-plan ownership. These are existing work, not future savings credited to this
proposal.

No implementation, runtime/device certification or Rust test run is part of this
backlog entry. Work only with managed Cargo compiler checks and relevant nextest
coverage when implementation is authorized. Preserve existing deferred tasks.
