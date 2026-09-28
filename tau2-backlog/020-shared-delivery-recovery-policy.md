# 020 — One typed delivery-recovery transition and retry classification

Status: **Open; small independent cleanup with safety-sensitive acceptance.**
Priority: **Small follow-up; not prerequisite to 015.**
Confidence: **9/10** in the value and appropriateness of this scope, not a probability of a LOC saving.
Net production LOC reduction estimate: **-10 to +25 lines**.

Recorded September 28, 2026. Evidence: the detailed `40a3698` investigation plus
relevant changes checked at `origin/tau2` = `cafef7f`. These are planning ranges,
not implemented or measured reductions. Positive means fewer lines; negative
means added code. Count changed production Rust across frontend, daemon and shared
crates; moving code is zero. Tests/docs/manifests are excluded unless explicitly
listed. See README for the confidence scale, baseline and overlap rules.

## Why / evidence

`frontend/src/store.rs::load_chat`, `frontend/src/controller.rs::{network_event,
not_sent}` and receipt handling repeatedly classify ordinary versus slash-prefixed
prompts and transition Sending/Preparing, Checking, Unconfirmed and WaitingForModel.

Confidence rationale: The duplicated transition/classification branches are directly
visible. The extraction is small; reason-specific messages and distinct operation
policies must remain.

## Proposed scope

Classify recovery policy once from the typed intent and reuse a pure recovery transition
for restart, disconnect and send failure. Parameterize context/reason rather than
homogenizing user-visible diagnostics. Preserve the current persisted format.

Out of scope: No unified operation database, schema migration, slash-command redesign or
new retry behavior.

## Acceptance

- Ordinary messages check the original receipt; uncertain effectful controls are not
  automatically rerun.
- Unconfirmed model selection remains rejected/reviewable as appropriate. Waiting
  intents are not silently submitted after a source fence.
- Exercise restart/disconnect/not-sent tables for the same operation classes, including
  caller-specific details.

## Honest impact estimate

Roughly 25–45 repeated production lines can be replaced with 20–35 lines of
classification/transition helpers: -10 to +25 net. The safety benefit is one definition
of retry policy, not hundreds of saved lines.

## Dependencies, overlap and current-source notes

The policy would become part of 021/022 if those land; do not credit it again. Existing
command-specific execution/receipt handlers remain necessary.

The same store/controller classifications remain at cafef7f under shifted line numbers.

No implementation, runtime/device certification or Rust test run is part of this
backlog entry. Work only with managed Cargo compiler checks and relevant nextest
coverage when implementation is authorized. Preserve existing deferred tasks.
