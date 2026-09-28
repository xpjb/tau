# 030 — Shared scroll-axis mechanics without restoration redesign

Status: **Open; mechanics-only extraction, 012 remains deferred.**
Priority: **Small independent cleanup when touching these paths.**
Confidence: **8/10** in the value and appropriateness of this scope, not a probability of a LOC saving.
Net production LOC reduction estimate: **0 to +35 lines**.

Recorded September 28, 2026. Evidence: the detailed `40a3698` investigation plus
relevant changes checked at `origin/tau2` = `cafef7f`. These are planning ranges,
not implemented or measured reductions. Positive means fewer lines; negative
means added code. Count changed production Rust across frontend, daemon and shared
crates; moving code is zero. Tests/docs/manifests are excluded unless explicitly
listed. See README for the confidence scale, baseline and overlap rules.

## Why / evidence

`frontend/src/app.rs::{tick, motion, scroll_value, set_scroll}` and
transcript/attachment/topic scroll fields, with existing `frontend/src/scroll.rs`
wheel/scrollbar math.

Confidence rationale: Three lanes repeat offset/max/velocity state, inertia decay and
drag estimation. This is concrete duplication, but it is only a few dozen physical
lines.

## Proposed scope

Introduce a small ScrollAxis owning offset, maximum and velocity with shared
clamp/drag/stop/inertial-step operations. Use it only for lanes that already share these
mechanics; retain horizontal/vertical interpretation and lane-specific policies.

Out of scope: No transcript anchor/follow-tail redesign, history/paging policy change,
or closure of 012. This is not the broader interaction scene.

## Acceptance

- Preserve bounds, decay, drag velocity and wheel/scrollbar behavior for all migrated lanes.
- Do not add inertia to lanes that lack it. Keep editor em-space/caret scrolling and
  menu keyboard reveal separate.
- Preserve navigation/download-jump interaction and existing restoration behavior; no
  new scroll architecture tests or deletion of deferred tests in this task.

## Honest impact estimate

About 45–75 lines of repeated mechanics/field coordination can be replaced by 40–45
shared/helper lines: approximately 0 to +35 net after rounding conservatively.
Scope-specific follow-tail/anchor code remains; moving that into the axis would not be a
legitimate saving.

## Dependencies, overlap and current-source notes

May become a mechanism under 028/031. Count mechanical deletion only once. Existing 007
concerns editor scrolling; existing 012 owns the explicitly deferred restoration audit.

cafef7f adds navigation-owned download jumps and changes paging thresholds. Preserve
those behaviors; the extraction must not reinstate the old thresholds.

No implementation, runtime/device certification or Rust test run is part of this
backlog entry. Work only with managed Cargo compiler checks and relevant nextest
coverage when implementation is authorized. Preserve existing deferred tasks.
