# 045 — Play/Stop/run-through UX and recurring control-state debt

Status: **Open, high-priority product/control-state review.** Reported October 1,
2026. The immediate lockout fix is source-merged as `efd1476`; that does **not**
close the UX or architectural concerns. The user explicitly requested this backlog and
source commit/push/merge. No release or live-conversation mutation is authorized.

## User concerns — do not reduce this to one more race fix

- The UX "makes no sense": Play, Stop, pause, "run through here" and a pending
  queue action do not form an understandable set of controls.
- "Run through here" does not interrupt promptly enough. Its benefit over ordinary
  queued messages is not obvious, and its deferred behavior is not apparent.
- Stop used to interrupt the current work and let pending queued work play out.
  The reported flow now requires Stop followed by Play, and even that bricked a
  conversation with "Cancel the pending queue control first."
- The user reported **no discoverable way to cancel that pending action**. A
  conditional button existing in source is not evidence of a usable escape on
  the user's running client/device.
- This suggests technical debt, not merely another isolated edge case. The user
  says we have already looked at Play/pause "a million times": recurring attention
  without dependable basic controls has exhausted their patience and trust.
- Pausing for a genuine severe failure, such as disk full, is useful and should
  remain. Fixing normal control flow must not erase failure/recovery safeguards.

## Reproduced immediate defect and bounded fix

Sequence: active response → enqueue messages → run-through/pause is waiting →
Stop cancels the run → the old waiting control remains → Play is rejected.
The run can be gone while its boundary action still blocks execution.

[Recovery fix and validation](../docs/tau2-control-recovery.md): Stop atomically
retires the deferred action while preserving held messages; Play supersedes old
controls, including already-stuck chats and a stopped run ID from a stale frame.
Newer valid boundary intentions replace older ones; cleanup respects the last
explicit continuation/hold. A failed run settles its waiting action rather than
leaving it indefinitely pending. Receipt retries do not resurrect retired intent.

Small copy improvements name cancellation and make the run-through limit/pause
visible. They are **not** a UX redesign or proof of interactive-device acceptance.
Run-through still waits for the existing provider/tool-batch boundary; this fix
has not introduced mid-stream steering or faster checkpoints.

## Why this needs an actual ownership review

Control meaning is currently spread across:

- Durable `QueueState`: `paused`, optional `run_id`, and optional `QueueControl`
  with string `action`/`status`, boundary and frozen request/revision references.
- In-memory `AgentSession`: `running`, cancellation token, `resume_after_stop`,
  `needs_turn`, and task ownership/cleanup.
- Runtime `SessionStatus`/detail and the client's replicated queue, receipts and
  locally pending commands, arriving on different timelines.
- Separate runner/compaction, restoration/eviction, manager commands and UI
  predicates that interpret these combinations.

These facts have legitimate separate purposes, but a waiting intent with no
owner, a cancelled task that still reports running, and an accepted command whose
visible effect is deferred must have **one explicit contract**, not ad hoc guard
messages. Audit actual writers and transitions before deciding whether typed
states or a smaller desired-intent/actual-execution model removes invalid
combinations. Moving these fields into a new wrapper is not a fix.

Prior fixes are evidence of recurring ownership risk, not justification for
another speculative rewrite:

- [Resume during Stop cleanup](../docs/tau2-resume-during-stop.md): newer intent
  used to be overwritten by the old run's settlement.
- [Repeated single-task compaction pauses](../docs/tau2-single-task-compaction.md):
  a different cause repeatedly required recovery in a long task.
- This report: a boundary action outlived its run and became a prerequisite to
  the very Play action needed for recovery.

## Bounded follow-up deliverables

1. **Agree a plain-language behavior table with the user.** Define Play, Stop,
   pause-at-boundary, run-through and cancel in idle/running/stopping/error states.
   Address the user's historical Stop→drain expectation explicitly; do not silently
   decide it is obsolete or silently add automatic paid/tool continuation.
2. **Explain or simplify run-through.** Show which queued messages will run,
   where it will yield, why it is currently waiting, and an obvious interruption
   path. Decide whether it deserves a distinct action at all. Review safely
   taking steering input sooner, without replaying tools, fabricating completion
   or hiding uncertain effects. A renamed menu alone does not complete this.
3. **Consolidate transition ownership.** Inventory every writer, including
   capacity admission, provider streams, tools, automatic/manual compaction,
   cancelled-task cleanup, eviction/reopen, crash/restore and multiple clients.
   Name and remove conflicting predicates/transitions. Keep desired intent,
   acceptance receipts and actual stopped execution distinct where necessary.
4. **Keep the useful safety distinctions.** Storage/provider failures and unknown
   external effects remain visible and require appropriate explicit recovery.
   Looking at state, reconnecting, replaying a receipt or cancelling a run limit
   must not become an implicit permission to repeat paid work or tool effects.
5. **Validate the experience, not just another green suite.** Walk the reported
   sequence on desktop and phone, including a long model response/tool, rapid
   Stop→Play, queued prefixes/tails and a real visible error. Use a small set of
   behavioral regressions for demonstrated invariants; do not add another huge
   synthetic matrix and call the UX reviewed.

## Acceptance / closure

- Basic Play/Stop cannot require the user to understand or clear a hidden
  "pending control action". Every deferred action has an obvious override/escape.
- The difference between running, stopping, held work, deferred boundary intent
  and failure is visible and understandable without run IDs or queue internals.
- Queued messages are preserved and run exactly once under the agreed semantics;
  old receipts and old cleanup cannot override newer user intent.
- Long waits and run-through limitations are understandable, and any claimed
  interruption improvement is verified at the actual provider/tool boundary.
- Genuine disk-full/failure/restore pauses and uncertain-effect review survive.
- The ownership reduction and real-device walkthrough are recorded. **Do not
  close 045 merely because the immediate deadlock patch was merged.**
