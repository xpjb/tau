# Repeated pauses during a long single task

Source fix on `fix/tau2-compaction-pause`, based on integration `33f7d6f`.
Daemon fix: `302e25f`.
This is independent of the unfinished simplification work and the older
`tau2/qa-remove-pause` WIP. Those worktrees are unchanged.

## Diagnosis

Read-only beta inspection found repeated Resume receipts and context checkpoints
in an active long-running chat, but no recent saved assistant provider-stream
errors explaining those resumes. Most recent checkpoints retained only one user
message. The beta health endpoint reports 0.7.9 / protocol 21; no service or live
configuration/database was changed.

The actual daemon path reproduces a deterministic failure: when a single user
request runs enough tools to need compaction, the old implementation refuses to
compact without a second user message (`Not enough completed turns to compact
safely`). Run settlement then pauses the queue. Sending another user message can
supply that artificial boundary, but the problem returns during the same long
task or after its next checkpoint. Resume alone cannot create that boundary.

This is a proven bug consistent with the observed metadata, not proof that every
reported pause had this cause. Errors before provider generation previously left
only transient session status, not a saved assistant error or a run-failure log.

## Change and safety

- Prefer whole user-turn boundaries when the retained suffix fits the configured
  token target. Otherwise compact between complete assistant/tool exchanges,
  including multiple checkpoints within one user task.
- Never split a multi-call batch from its results. Native call identities remain
  distinct; legacy aliases settle a call only when unambiguous. Interrupted
  assistant fragments do not become executable calls.
- Reuse the existing retained-entry cursor and checkpoint format. Retained
  boundaries need not be user entries; cold context loading already supports
  any persisted entry ID. No schema, protocol or provider API change.
- Preserve the original durable history and tool outcomes. This is context
  reduction, not tool replay or an automatic retry of uncertain provider effects.
  Abort/restart/restore and genuine provider-failure safety pauses remain.
- A lone oversized input or an indivisible oversized exchange still fails
  explicitly; the existing one-compaction-without-progress guard remains.
- Log bounded run-failure reasons, including compaction/preparation failures, so
  the next unexpected pause can be diagnosed without reading authored content.

## Validation

Managed Cargo only; no Clippy or built-in Cargo test runner.

- Failing-before regression: `0e768e62-76e7-49ce-b9a4-00e6b24c595b`.
  The real agent/provider/socket path stops with the exact error above after two
  tool calls from one prompt. Initial fixture attempts lacked an authoritative
  model catalog and did not exercise compaction; they are not regression proof.
- Passing targeted run: `aad4424b-5d68-4617-88e2-aefe7e75a88b`, 2/2.
- Full daemon nextest: `00e783ea-a3dc-4295-b2bb-b6c302a23c12`, **60/60**, zero skipped.
  The new scenario completes two checkpoints in one task, for native checkpoints
  and text summaries, with exactly-once shell side effects, paired call/result
  replay and cold-context reconstruction. Boundary checks cover parallel calls,
  ambiguous legacy IDs, retention preference and insufficient history. Existing
  interrupted-stream, abort/resume, restore, crash and paid-effect tests survive.

## Visible reasons, not a generic pause label

The header now prioritizes a failed run over the queue's paused flag, displays
its daemon error in red, and opens the full reason in the existing notice popup
when the title/status is clicked or tapped. This read-only action cannot Resume,
submit a prompt or repeat an external effect. Running states take precedence over
held-queue state and expose details such as `Compacting context`. The sidebar
also keeps Error/Working distinct from an idle paused queue.

The shared desktop/phone interaction regression verifies opening the full reason
and refusing a stale failure detail after the run starts again. No new event bus,
dialog type, protocol field or cached status copy was introduced.

Final combined validation:

- Workspace/all-target compiler check: passed.
- Full workspace nextest: **331/331**, zero skipped,
  `65d4aa38-7d24-4e2d-b910-ad50351b2113` (103.409s).
- Focused desktop/phone header interaction: 1/1,
  `ed6057f8-3ac1-48ef-954b-9351480bef9f`.
- Windows x64 MSVC (`cargo xwin check`) and Android ARM64/API-29 library compiler
  checks: passed. Existing unrelated dead-code warnings remain; no lint rewrites.
- `git diff --check`: passed. No physical Windows/Android acceptance claimed.

No merge, deployment, service restart, client package or physical-device QA is
part of this source fix. The running beta is unchanged until a separately
approved matched release; integration is protocol 22, unlike the live beta.
