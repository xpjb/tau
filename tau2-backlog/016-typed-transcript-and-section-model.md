# 016 — Typed transcript items and shared section membership

Kind: **Parent proposal — not a separate implementation task.**
Children: **015, 017, 018**. Related cleanup: 035. Design context: 019 / 022.

Status: **Parent proposal; implement through selected child scopes, not as an additional combined rewrite.**
Priority: **015 first; select subsequent transcript slices after its evidence.**
Confidence: **7/10** in the value and appropriateness of this scope, not a probability of a LOC saving.
Net production LOC reduction estimate: **-100 to +300 lines**.

Recorded September 28, 2026. Evidence: the detailed `40a3698` investigation plus
relevant changes checked at `origin/tau2` = `cafef7f`. These are planning ranges,
not implemented or measured reductions. Positive means fewer lines; negative
means added code. Count changed production Rust across frontend, daemon and shared
crates; moving code is zero. Tests/docs/manifests are excluded unless explicitly
listed. See README for the confidence scale, baseline and overlap rules.

## Why / evidence

`frontend/src/blocks.rs::{View, snapshot_inner, plan_visible}`,
`frontend/src/feed.rs::{Feed, native_view}`, `frontend/src/app.rs::rows`, and
`frontend/src/details.rs::{Tools, open_items}`. Rendering and the no-viewport planner
separately filter events and form Details groups.

Confidence rationale: The native-header → flat-event → reconstructed-section round trip
is well evidenced. The full replacement needs substantial indexing and compatibility
code; net shrinkage is uncertain.

## Proposed scope

Replace the native transcript adapter boundary with typed items, scoped identities and
one derived section-membership index. Extend beyond tools to
messages/reasoning/attachments and represent pending/queue/history observations and
content availability through 017/018. Display, copying and content demand share
membership while retaining their distinct policies. Rebuild only affected loaded
projections.

Out of scope: Not a new authoritative transcript database or eager full-history graph.
No general operation-store migration or change to deferred scroll-restoration policy.

## Acceptance

- Native display and no-renderer bootstrap do not maintain competing visibility/grouping
  algorithms. Section source references are explicit.
- Preserve prepend-stable disclosure/anchor keys, bounded loading, sparse updates, copy
  completeness and source/version fencing.
- Remove the superseded native interpretations instead of wrapping them. Preserve live
  startup/full-preview behavior and migrate demo inputs to the same semantic contract.

## Reference estimate — not an additional implementation budget

This scenario is retained for design comparison. Do not add it to child-task
estimates or schedule the parent/design record as another implementation.

Umbrella budget: approximately 450–650 lines of adapter/grouping/reconciliation code
replaced, with 350–550 lines of model, indexes and compatibility code introduced. Net:
-100 to +300. This is an inclusive scenario for 015, 017 and 018, not an additional
saving after them. It excludes the separately identified non-wire legacy-adapter
deletion in 035.

## Dependencies, overlap and current-source notes

Umbrella covering 015/017/018. Also overlaps the domain-record portion of 022 and the
transcript owner in 031. Re-estimate the remainder after each child lands; never sum the
umbrella and children.

cafef7f adds retained previews and multi-chat prefetch around the same adapter boundary.
Preserve those behaviors; the old audit line numbers are not current implementation
locations.

No implementation, runtime/device certification or Rust test run is part of this
backlog entry. Work only with managed Cargo compiler checks and relevant nextest
coverage when implementation is authorized. Preserve existing deferred tasks.
