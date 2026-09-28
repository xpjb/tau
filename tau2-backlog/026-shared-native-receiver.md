# 026 — One native receive/verify/commit state machine

Status: **Open; useful bounded follow-up, not the selected first change.**
Priority: **After 015, or independently if receive behavior changes.**
Confidence: **8/10** in the value and appropriateness of this scope, not a probability of a LOC saving.
Net production LOC reduction estimate: **-50 to +50 lines**.

Recorded September 28, 2026. Evidence: the detailed `40a3698` investigation plus
relevant changes checked at `origin/tau2` = `cafef7f`. These are planning ranges,
not implemented or measured reductions. Positive means fewer lines; negative
means added code. Count changed production Rust across frontend, daemon and shared
crates; moving code is zero. Tests/docs/manifests are excluded unless explicitly
listed. See README for the confidence scale, baseline and overlap rules.

## Why / evidence

`frontend/src/blocks.rs::watch_once`, `frontend/src/blocks/files.rs::Downloads::fetch`,
`frontend/src/blocks/uploads.rs::Downloads::descriptor`, and
`transfer/src/blocks.rs::{Header, Watcher}`.

Confidence rationale: Three receive loops duplicate frame validation and cache
installation. Sharing them centralizes invariants, but keeping endpoint-specific
policies and a storage sink may consume most of the deleted lines.

## Proposed scope

Extract shared page/header/range assembly and validation behind a committed-storage sink
in the existing shared code. Migrate transcript watches, file receives and descriptor
receives. Keep UI notices/progress, scheduling, file export and final descriptor
decoding/digest rules at their host boundary.

Out of scope: No transport replacement, crate merger, retry/timeout policy rewrite,
operation execution or attempt to close the unrelated deferred outage-cause
investigation in 014.

## Acceptance

- Preserve source/epoch checks inside storage transactions, frame bounds, requested
  IDs/versions, file contiguous-offset checks and descriptor whole-body integrity.
- Publish resumable state only after durable commit. Byte credit is flow control, not
  proof of persisted state; permit bounded staging of metadata records before a page
  checkpoint.
- Handle End versus Yield, cancellation, failed commits, batch scopes, restart and late
  frames without false completion. Preserve final export fencing.

## Honest impact estimate

Approximately 100–150 lines of repeated receive-loop mechanics could be replaced, with
100–150 lines of common parser/sink code and endpoint policies: -50 to +50 net. Moving a
single loop out of frontend is zero cross-workspace reduction. Do not remove independent
integrity checks merely to improve the count.

## Dependencies, overlap and current-source notes

026 supplies the actual receive coordinator discussed under 027; packaging cannot claim
the same saving. It can support 023 but does not itself add multi-authority hosting.

cafef7f added batched background feeds and finite background body catch-ups to
watch_once. Include those modes and per-feed notice scopes in the extraction; do not
revert scheduling improvements.

No implementation, runtime/device certification or Rust test run is part of this
backlog entry. Work only with managed Cargo compiler checks and relevant nextest
coverage when implementation is authorized. Preserve existing deferred tasks.
