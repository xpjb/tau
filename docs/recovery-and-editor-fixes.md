# Compaction, stream recovery and desktop selection

October 7, 2026. Changes target the `tau2` mainline, not stable `master`.

## Compaction deadlock

The affected chat's retained context began with two assistant tool calls that
had no recorded results, followed by many ordinary completed exchanges. Its
fork contained the same interrupted prefix. Read-only inspection found 96 and
95 retained entries respectively, but zero eligible compaction boundaries.

Commit `302e25f` introduced a pending-call set that blocked every subsequent
boundary until all earlier calls acquired results. Unlike normal request
projection, it did not account for missing results after interruption/forking.
`history::messages` already supplies explicit unknown-outcome results, warning
against automatically repeating external or paid effects.

Compaction now measures spans between calls and **stored** results. Real results
remain with their call, including parallel batches and ambiguous legacy aliases.
Calls without results no longer invalidate the rest of history; the existing
unknown-outcome repair also runs on compaction input. No history migration,
manual database edit or tool reexecution is necessary. Native checkpoints and
text summaries use the same boundary calculation.

Regression coverage includes the formerly failing leading-orphan case, repeated
call IDs, real/ambiguous result boundaries, automatic compaction after reload,
manual compaction of an existing fork, both checkpoint types and no tool replay.

## Transient stream errors

The stream path previously retried only numeric `500` and three normalized Codex
error names. Other recoverable upstream failures became generic terminal errors.
Additionally, retry safety required a Codex checkpoint scope, preventing even
ordinary chat-completions disconnects from retrying. Compaction had no owner for
retrying stream-body failures at all.

Both APIs now classify transient status/code/type values, preserve redacted
upstream diagnostics (including terminal events without an SSE separator), and
retry within the existing configured backoff/budget. Chat commits checkpoints
before retrying and remains running; a retry explanation appears in Details,
not an error banner. Compaction/summary/search requests retry their disposable
streams. Stop and compaction timeouts still cancel backoff. Permanent errors,
quota exhaustion and known started native image generation do not auto-retry.

Tests cover both APIs, checkpoint continuation, partial tools never executing,
exact-once subsequent tool effects, retry details/redaction, permanent failures,
compaction retries, no-progress budgets, Stop and paid-image safeguards.

## Desktop editor

Desktop presses were always routed to single-caret hit testing. Sanscale exposed
word/paragraph selection, but Tau never counted clicks or invoked those APIs.

The shared composer/settings/dialog field now recognizes double/triple clicks,
selects Unicode words or complete hard-break-delimited paragraphs across soft
wraps, and drags in the selected unit. Shift-click extends selection. Click
sequences are time/distance/DPI bounded, tied to the editor's identity and reset
by intervening input, navigation, pointer cancellation or dragging. Secret fields
select as one word. Touch long-press/selection remains on its existing path.

Real-font tests cover selection geometry, Unicode, wrapping, masking, drag
reversal and undo. Headless App tests drive actual composer and settings fields,
including clipboard and non-mutating selection. Windows cross-compilation is a
compiler check, not a claim of interactive testing on a native Windows host.

Release versions and protocol/schema are unchanged. Installed binaries and
running services have not been changed; deployment is a separate operator action.

## Validation

- Managed nextest workspace run on the final code: 423/425 passed; the two native
  network stress failures both passed isolated reruns without code, deadline or
  assertion changes. They also passed the earlier 423-test full run. The affected
  cases are `native_blocks_resume_after_udp_outage_while_websocket_stays_healthy`
  and `blackhole_recovers_original_intents_and_bulk_without_duplicate_effects`;
  their intermittent native transport timeout/recovery behavior is logged for
  separate follow-up. All compaction, provider recovery and editor tests passed.
- `cargo xwin check --locked --target x86_64-pc-windows-msvc -p tau-frontend
  --all-targets` passed through the managed Cargo wrapper.
- A temporary read-only diagnostic invoked the new Rust boundary calculation on
  the actual affected database records: the original chat can compact 95 of its
  96 retained entries, and the existing fork can compact 87 of 95. The diagnostic
  was removed; no user history or database writes belong to the regression suite.
- No Clippy, built-in test runner, live provider requests, deployment or service
  restart was performed. Tests use isolated fixture accounts and local providers.
