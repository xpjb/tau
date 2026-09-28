# 017 — One logical message through pending, queue and history

Status: **Open; design follow-up, not a storage-lifetime fix.**
Priority: **Within 016, after 015.**
Confidence: **7/10** in the value and appropriateness of this scope, not a probability of a LOC saving.
Net production LOC reduction estimate: **-50 to +100 lines**.

Recorded September 28, 2026. Evidence: the detailed `40a3698` investigation plus
relevant changes checked at `origin/tau2` = `cafef7f`. These are planning ranges,
not implemented or measured reductions. Positive means fewer lines; negative
means added code. Count changed production Rust across frontend, daemon and shared
crates; moving code is zero. Tests/docs/manifests are excluded unless explicitly
listed. See README for the confidence scale, baseline and overlap rules.

## Why / evidence

`frontend/src/app.rs::rows`, `frontend/src/feed.rs::{native_view, queue_transitions}`,
`frontend/src/store.rs::LocalChat::reconcile_complete`, and
`frontend/src/blocks.rs::snapshot_inner` maintain different observations of the same
authored message.

Confidence rationale: Display precedence is repeatedly reconstructed by request
identity. A single message model is plausible, but aliasing, asynchronous handoff and
conservative retirement add essential state.

## Proposed scope

Give an authored message a source/chat-scoped logical identity, with references to local
intent, queue observation and canonical history observation. Centralize
display-body/status/placement queries and queue-to-history cursor barriers. Keep
operation retirement evidence separate from presentation precedence.

Out of scope: No automatic retry-policy change, operation-store consolidation or daemon
body-identity rewrite. In particular this does not close 013: a stable client display ID
does not make a removed server body readable.

## Acceptance

- One displayed message through delayed acknowledgements, queued/history overlap,
  removals arriving first, aliases and reconnect.
- No loss of authored text on receipt/header arrival alone. Reuse local bytes only after
  the existing source/length/digest checks.
- Edits preserve expected revisions, source changes fence old work, and copy/edit
  actions target the same logical item.

## Honest impact estimate

Budget 150–220 lines of row-precedence and handoff coordination replaced by 120–200
lines of message records, indexes and selectors: -50 to +100 net production lines.
Necessary formatting, delivery evidence and recovery checks are not counted as
deletable.

## Dependencies, overlap and current-source notes

Included in 016. Shares operation references with 021 but does not count the
durable-store migration. Existing 013 remains open/deferred under its own acceptance
conditions.

The pending/queue/history representations and conservative body-retirement rules remain
in cafef7f. Preserve recent-chat state and navigation-owned selection changes.

No implementation, runtime/device certification or Rust test run is part of this
backlog entry. Work only with managed Cargo compiler checks and relevant nextest
coverage when implementation is authorized. Preserve existing deferred tasks.
