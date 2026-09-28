# 035 — Retire non-wire transcript update/page fixture adapters

Status: **Open; migrate remaining demo/test callers before deletion.**
Priority: **Independent cleanup; relatively credible LOC saving.**
Confidence: **8/10** in the value and appropriateness of this scope, not a probability of a LOC saving.
Net production LOC reduction estimate: **+80 to +180 lines**.

Recorded September 28, 2026. Evidence: the detailed `40a3698` investigation plus
relevant changes checked at `origin/tau2` = `cafef7f`. These are planning ranges,
not implemented or measured reductions. Positive means fewer lines; negative
means added code. Count changed production Rust across frontend, daemon and shared
crates; moving code is zero. Tests/docs/manifests are excluded unless explicitly
listed. See README for the confidence scale, baseline and overlap rules.

## Why / evidence

`frontend/src/feed.rs::{update, page}` occupy about 114 lines at the audited revision;
controller TranscriptUpdate/TranscriptPage handlers and protocol variants marked
#[serde(skip)] add more. Demos/tests still inject them. `Feed::snapshot` also serves
real native full projections/startup and cannot simply be deleted.

Confidence rationale: Obsolete update/page algorithms and skipped-wire message variants
are identifiable. Some similarly named snapshot paths are still live, so selective
retirement is essential.

## Proposed scope

Move demo/test injection to an explicit in-process fixture/view API compatible with the
native projection, then delete genuinely unused old update/page algorithms,
corresponding control-message-shaped adapters and orphaned types. Audit all callers;
preserve shared Event/provider-history types and live snapshot behavior.

Out of scope: No deletion of Feed::snapshot wholesale, no claim these variants form a
second live wire protocol, and no provider-history/export format rewrite.

## Acceptance

- Search/caller audit demonstrates removed variants are no longer used; fixtures still
  cover live native rendering semantics.
- Native full startup previews, sparse patches and older-history requests retain
  behavior. Do not keep a parallel legacy renderer only for fixtures.
- Run relevant native projection, demo/render and controller tests without deleting
  safety assertions.

## Honest impact estimate

About 180–230 production lines of old algorithms/handlers/variants can plausibly be
retired, offset by 50–100 lines of explicit fixture/view entry points and remaining
production demo adaptation: +80 to +180 net. Test changes are separate and unestimated
here; no thousands-of-lines claim is supported.

## Dependencies, overlap and current-source notes

Some bridging may be replaced by 016, so re-estimate after a transcript migration and
count each adapter deletion once. Generic headless fixture setup is 034, not credited
here.

The legacy update/page methods and skipped-wire variants remain at cafef7f; its
warm-preview/full-view behavior reinforces the need to preserve the live snapshot path.

No implementation, runtime/device certification or Rust test run is part of this
backlog entry. Work only with managed Cargo compiler checks and relevant nextest
coverage when implementation is authorized. Preserve existing deferred tasks.
