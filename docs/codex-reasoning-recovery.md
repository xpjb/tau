# Codex reasoning recovery and automatic continuation

October 5, 2026 (UTC); retry coverage updated October 7, 2026.

## Contract

Completed encrypted reasoning survives a failed or aborted Codex response. On a
transient upstream body disconnect, idle timeout, premature EOF, or server-error
stream event, the agent commits the recoverable state and continues automatically.
The chat stays running and the recovered failure has no user-facing error banner.
A redacted retry diagnostic is retained in expandable Details.

Recovery accepts only completed reasoning items with a nonempty ID, encrypted
content and a valid summary array. Provider, API, exact model, account and endpoint
must match. Item IDs are deduplicated against successful history, earlier recovered
items and branch-local rejection records. Failed prose and unfinished tool calls
remain display history only.

The capture belongs to one provider request, independently of the display-update
channel. It is populated while decoding completed items, so a later error in the
same transport chunk or cancellation before display delivery preserves the item.
The existing failed assistant entry stores `tauReasoningRecovery`; the normal
SQLite history owns durability, restart, clones and forks. The display projection
excludes its ciphertext and account provenance. Chat-completions adapters omit the
private recovery envelope entirely.

Explicit Stop cancels both the provider stream and retry backoff. Resume/new
steering reuses eligible checkpoints in chronology, before later user instructions.
Native compaction receives recovered reasoning as part of its input.

## Retry policy

The agent's existing `retry.enabled`, `maxRetries`, and `baseDelayMs` settings apply.
Consecutive failures without new checkpoints consume the retry budget and use
bounded exponential backoff. A genuinely new completed checkpoint resets that
budget, allowing a long reasoning run to keep making progress across recurring
upstream cutoffs. Re-emitting an input checkpoint is not new progress.

Known native image-generation activity blocks automatic continuation because its
paid effects may already have occurred. Invalid JSON, invalid completed responses,
authentication failures and other non-transient errors retain the ordinary error
path. Recognized encrypted-reasoning rejection durably suppresses those recovered
IDs on a later continuation. Local tool execution still follows successful whole-
response validation; interrupted calls do not execute.

The shared retry classifier also covers chat-completions streams and disposable
compaction/summary/search requests. See `recovery-and-editor-fixes.md` for the
compaction deadlock, retry-classification and editor regressions.

## Implementation

Paths below are relative to `crates/daemon/src/`.

- `agent/provider/recovery.rs`: eligibility, provenance, deduplication, capture,
  rejection markers and safe-continuation state.
- `agent/provider/codex.rs`: capture at item completion, separate from display.
- `agent/provider/mod.rs`: request projection and typed transient stream errors.
- `agent/history.rs`: retain only private recovery from failed/aborted messages.
- `agent/mod.rs`: commit before retry; cancellation and bounded retry ownership.

## Regression evidence

The initial isolated hotfix passed **84/84** daemon nextest tests. Its regression
cases live under `crates/daemon/tests/unit/agent/` in the consolidated tree and cover repeated failures
with progress, bounded no-progress retries, exact-once local tool effects, abort,
backoff cancellation, restart, cloning, new steering, model/account/provider/API/
endpoint changes, durable rejection, native compaction and same-chunk errors.

A separate black-box fixture forces the exact reqwest `Model stream disconnected`
path by truncating an HTTP response body after a completed encrypted reasoning
item. It runs the actual executable in an isolated loopback-only network namespace
with synthetic credentials. Original deployed binary loses the checkpoint; the
fixed binary must automatically continue with it exactly once and finish the chat.
Scratch evidence and fixture scripts live under `/tmp/tau2-astra-recovery-investigation/`.

## Compatibility

The initial reproduction used deployed source `4c9a289` (beta 0.7.11, protocol 22).
Checkpoint recovery itself is an additive private history field and requires no
database-schema change. The consolidated `tau2` release also includes the native
network refactor and Codex sign-in: it uses protocol 26 and requires matching
Windows/Android clients. Deliver those packages before the approved service restart.
