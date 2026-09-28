# 015 — One shared typed tool-transcript projection

Kind: **Implementation slice — recommended first.**
Parent: **016 (transcript)**; design context 019 / 022.

Status: **Open; recommended first implementation, not started.**
Priority: **First from this investigation.**
Confidence: **8/10** in the value and appropriateness of this scope, not a probability of a LOC saving.
Net production LOC reduction estimate: **-50 to +150 lines**.

Recorded September 28, 2026. Evidence: the detailed `40a3698` investigation plus
relevant changes checked at `origin/tau2` = `cafef7f`. These are planning ranges,
not implemented or measured reductions. Positive means fewer lines; negative
means added code. Count changed production Rust across frontend, daemon and shared
crates; moving code is zero. Tests/docs/manifests are excluded unless explicitly
listed. See README for the confidence scale, baseline and overlap rules.

## Why / evidence

`frontend/src/details.rs::{Tools, Line}`, `frontend/src/app.rs::chat`,
`frontend/src/blocks.rs::{snapshot_inner, plan_visible, copy_ready, copy_complete,
native_event}`, and `daemon/src/blocks.rs::event_inner`. The producer emits
`tool:<id>:Input:text`; the inverse interest parser does not strip `:text`. The source
mismatch is established; its GUI/network manifestation was not reproduced.

Related source-mismatch finding: `87d54bdf-7c32-4eab-b7ca-ce6409140806`.

Confidence rationale: Three consumers reconstruct the same native relationships, and an
actual display-key/source-ID contract mismatch is established. Partial children and
existing presentation rules make the replacement larger than a simple ID fix.

## Proposed scope

Define shared typed tool metadata/relationship helpers and a derived tool projection
with native identity, execution state, input/result body references and child-discovery
completeness. Preserve the current wire encoding. Have daemon publication use the shared
contract and have native display, tool-copy membership and tool-body interests consume
the same projection. Carry explicit source references on rendered sections; preserve
existing disclosure keys.

Out of scope: Ordinary-message reconciliation, outer Details-group redesign,
persistence/wire migration, crate merger, receiver extraction or UI framework.

## Acceptance

- Delete the obsolete native tool pairing/membership paths, tool-root rescan and inverse
  display-key parser as consumers switch.
- Cover partial children, repeated provider IDs, orphan/multiple results,
  interruption/errors and overflow metadata. A visible body must retain the correct
  interest when its heading is off-screen.
- Keep body bytes in the existing bounded cache, preserve complete-copy verification and
  current foreground/background interest policy. Closed tools must not acquire
  unrequested bodies.

## Honest impact estimate

Budget roughly 200–300 retired/replaced production lines across the tool-related
portions of details, planner, copy and publication code, offset by 150–250 lines of
shared types, projection and adapters. Net: -50 to +150. Much of the 231-line details
module is still necessary presentation; it is not all deletable. Regression tests will
add code and are excluded.

## Dependencies, overlap and current-source notes

Contained in the broader 016 projection estimate and overlaps the tool-contract part of
022. Do not add those savings. The smaller explicit-reference/parser fix is the first
substep here, not another separately credited item.

The key producer and inverse parser still exist at cafef7f. Preserve its wider overscan,
warm previews and background catch-up work; none of that code is credited as removable.

No implementation, runtime/device certification or Rust test run is part of this
backlog entry. Work only with managed Cargo compiler checks and relevant nextest
coverage when implementation is authorized. Preserve existing deferred tasks.
