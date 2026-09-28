# 018 — Explicit content availability instead of placeholder source text

Status: **Open; model-quality improvement, not guaranteed code reduction.**
Priority: **Alongside the relevant 016 consumers.**
Confidence: **7/10** in the value and appropriateness of this scope, not a probability of a LOC saving.
Net production LOC reduction estimate: **-80 to +20 lines**.

Recorded September 28, 2026. Evidence: the detailed `40a3698` investigation plus
relevant changes checked at `origin/tau2` = `cafef7f`. These are planning ranges,
not implemented or measured reductions. Positive means fewer lines; negative
means added code. Count changed production Rust across frontend, daemon and shared
crates; moving code is zero. Tests/docs/manifests are excluded unless explicitly
listed. See README for the confidence scale, baseline and overlap rules.

## Why / evidence

`frontend/src/blocks.rs::{View, snapshot_inner}` and
`frontend/src/feed.rs::drop_preview` synthesize `Loading…`; `App::rows` partly reverses
it. Lengths, incomplete IDs and execution states are separate maps. Cache residency and
in-memory preview size are already distinct facts.

Confidence rationale: Placeholder text and parallel maps demonstrably obscure the
difference between missing, partial and authored content. Explicit states are clearer
but require more declarations and transitions.

## Proposed scope

Introduce a bounded content-state facade over existing local/authored storage and the
verified cache: scoped body reference, advertised length/version, verified coverage,
preview handle, growing/sealed state and acquisition outcome. Let the UI generate
loading/truncation presentation without modifying source text.

Out of scope: No replacement blob store, transport change, whole-body materialization or
stronger delivery claim based only on metadata.

## Acceptance

- Distinguish empty, unavailable, partial, preview-limited, growing and complete
  content, including real authored text equal to a loading label.
- A complete on-disk body may have a truncated preview. Cache eviction cannot discard
  authored work or logical identity.
- Preserve copy finality/hash/aggregate-size checks, source fencing and existing
  cache/prefetch budgets.

## Honest impact estimate

Only about 40–100 lines of placeholder/map coordination are plausible deletion
candidates; expect 80–120 lines of explicit state and accessors. Net: -80 to +20. This
is mainly a correctness/clarity improvement, not a credible large LOC saving.

## Dependencies, overlap and current-source notes

Included in 016 and partly needed by 015. Count shared availability types once. Do not
also credit them as a new saving in 019/022.

cafef7f still synthesizes loading text and keeps the side maps; its retained-preview
logic makes preserving residency/preview distinctions especially important.

No implementation, runtime/device certification or Rust test run is part of this
backlog entry. Work only with managed Cargo compiler checks and relevant nextest
coverage when implementation is authorized. Preserve existing deferred tasks.
