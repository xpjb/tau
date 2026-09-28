# 032 — Typed dialog state and one submit/completion lifecycle

Status: **Open; separate UI-state cleanup.**
Priority: **After a representative dialog is selected.**
Confidence: **7/10** in the value and appropriateness of this scope, not a probability of a LOC saving.
Net production LOC reduction estimate: **-50 to +100 lines**.

Recorded September 28, 2026. Evidence: the detailed `40a3698` investigation plus
relevant changes checked at `origin/tau2` = `cafef7f`. These are planning ranges,
not implemented or measured reductions. Positive means fewer lines; negative
means added code. Count changed production Rust across frontend, daemon and shared
crates; moving code is zero. Tests/docs/manifests are excluded unless explicitly
listed. See README for the confidence scale, baseline and overlap rules.

## Why / evidence

`frontend/src/app.rs::{Modal, ModalKind, apply, tick}` and dialog renderers,
`frontend/src/app/projects.rs`, and the existing table-driven daemon-settings draft.

Confidence rationale: Modal kind, indexed fields, wait/save flags and submit/completion
branches are spread across rendering, apply and tick. A typed lifecycle can remove
coordination, but named variants/fields add declarations.

## Proposed scope

Give dialogs explicit editing/submitting/completed/error state and typed field
ownership, reusing Editor. Bring submit/result correlation and close/cancel handling
under that owner. Start with a representative async form, then replace the duplicated
lifecycle only where the same contract holds.

Out of scope: No new text controller, visual redesign, generic form framework or changes
to settings precedence/scope.

## Acceptance

- Late or wrong-request results cannot mutate a replacement dialog or another account/chat.
- Preserve validation, draft edits after errors, cancel/close behavior, focus and
  keyboard submission.
- Keep daemon settings table-driven and reuse the shared editor; remove superseded
  scattered wait/save bookkeeping.

## Honest impact estimate

Approximately 100–200 coordination/lifecycle lines could be replaced by 100–150
typed-state/dispatch lines: -50 to +100 net for the migrated lifecycle. A one-dialog
pilot may initially grow while compatibility remains.

## Dependencies, overlap and current-source notes

UI helper boilerplate overlaps 029; owner types overlap 031; overlay routing overlaps
028. Exclude their savings when reporting this change.

cafef7f selection/notice navigation must remain the authority for navigation. Dialog
completion should use that path rather than restore older duplicated navigation code.

No implementation, runtime/device certification or Rust test run is part of this
backlog entry. Work only with managed Cargo compiler checks and relevant nextest
coverage when implementation is authorized. Preserve existing deferred tasks.
