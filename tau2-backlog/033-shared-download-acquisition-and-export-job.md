# 033 — Shared download acquisition and explicit export-job intent

Kind: **Feature-level implementation candidate.**
Design parent: **031**; network receiver mechanics are separately 026.

Status: **Open; preserve newer DownloadTarget/navigation work.**
Priority: **Small independent cleanup if download flow is edited.**
Confidence: **8/10** in the value and appropriateness of this scope, not a probability of a LOC saving.
Net production LOC reduction estimate: **-25 to +50 lines**.

Recorded September 28, 2026. Evidence: the detailed `40a3698` investigation plus
relevant changes checked at `origin/tau2` = `cafef7f`. These are planning ranges,
not implemented or measured reductions. Positive means fewer lines; negative
means added code. Count changed production Rust across frontend, daemon and shared
crates; moving code is zero. Tests/docs/manifests are excluded unless explicitly
listed. See README for the confidence scale, baseline and overlap rules.

## Why / evidence

`frontend/src/app.rs` Action::Attachment/Action::SaveAttachment and
`app/attachments.rs::{completed_exports, export_target, begin_save}`. Current
`notice::DownloadTarget` already binds identity/lineage/chat/entry.

Confidence rationale: Attachment activation/save still repeat
acquisition/error/completion plumbing. Preview and export semantics differ, and upstream
already removed some source-target duplication, so the remaining saving is modest.

## Proposed scope

Share acquisition/error/completion setup with an explicit preview/save intent. If it
genuinely simplifies the remaining coordinated maps, use a typed export-job state
referencing DownloadTarget. Keep platform save/open/extract actions and
source/path-bound completion checks explicit.

Out of scope: No unified transfer protocol, export destination redesign, shared card
renderer replacement or claim to fix the distinct outage issue in 014.

## Acceptance

- Image preview stays cache-only and cannot trigger an export. File activation/save
  follows its existing user intent.
- Old, failed, cancelled or wrong-account/source/path completion cannot start a save.
  Preserve retry and completed-file verification.
- Keep downloadable-notice navigation, source fencing and current tests for save-once
  and account changes.

## Honest impact estimate

Roughly 35–80 remaining acquisition/bookkeeping lines can be replaced with 30–60
helper/job lines: -25 to +50 net. Do not count already-shared
DownloadTarget.matches_source or card geometry as new deletion credit.

## Dependencies, overlap and current-source notes

Job ownership can sit under 019/031 but contributes no extra savings there. Network
receive mechanics are separately 026; source/copy verification remains necessary.

cafef7f already introduces DownloadTarget and shared navigation/geometry, reducing the
opportunity compared with the original audit. The two action acquisition branches
remain.

No implementation, runtime/device certification or Rust test run is part of this
backlog entry. Work only with managed Cargo compiler checks and relevant nextest
coverage when implementation is authorized. Preserve existing deferred tasks.
