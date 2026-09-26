# Tau2 mobile QA — September 26, 2026

Worktree `/root/tau2-mobile-qa`, branch `fix/tau2-mobile-qa-20260926`, based on
local beta 0.7.5 (`f9d105a`). No protocol change, daemon change, deployment, package
build or service restart. Stable Tau 1 is unchanged. Connection-dot changes apply
to Tau2 on both desktop and Android.

## Changes

- Opaque full-screen native editor instead of a floating dialog over the chat.
  Keep the existing Done/save-draft meaning; no new implicit send or keyboard-send
  gesture. Handle IME/system-bar insets, keep Back in the editor, and drain queued
  native edits before pointer/key actions and suspension.
- Notice body and dismiss button consume the press. A release after the notice
  disappears cannot reach Stop. The same rule prevents notice long-press menus.
- Mobile/narrow topic selection chooses the list, not the remembered chat. Desktop
  keeps last-chat restoration. Hidden mobile chats retain unread status.
- Draw disclosure chevrons through the existing icon renderer.
- Android discovery uses NDK font matching, TTC face index and all variation axes.
  Sanscale `map_font_with_variations` applies the same instance to shaping, metrics
  and outlines; normalized coordinates participate in font identity. Tau pins
  `7bbe230` to retain its existing Sanscale API names. The library's newer master
  names remain outside this Tau change. No application font bundle.
- The dot stays green at stable 250–500 ms (up to 800 ms). Its ten-probe window
  retains missed attempts and warns on a range over 400 ms. Native QUIC loss
  deltas warn for 20 seconds; unchanged cumulative counters do not extend that
  warning. Pending probes are not counted as dropped packets. Keep orange above
  1 second and red above 3 seconds/disconnection.
- Ordinary saved messages retry transport failures with a two-second backoff.
  Socket loss/restart enters `Checking`, not a terminal “Not sent” state. Missing
  receipts permit resubmission with the same ID; accepted receipts settle without
  replay. Recover pending work in other chats on connection. One unacknowledged
  message per chat preserves order; retain the existing four-send global limit.
- Invalid local attachments and explicit server rejections stay blocked, with an
  explicit same-ID retry action. Older rejected/unconfirmed records require that
  action because they do not record whether automatic retry was safe. Source
  lineage fences and uncertain controls still require review. Active sends offer
  Copy text, not Restore draft, to avoid creating another send while retrying.

No new network protocol, background timer, font-discovery service or retry worker.
The existing heartbeat events drive backoff; existing SQLite records and receipts
own the saved intent. The new delivery state separates automatic receipt checking
from the old manual-review state. Old clients cannot read this new enum value;
do not downgrade a client with pending `Checking` records.

## Acceptance

- 14 focused native GPU/UI and connection tests passed:
  `/tmp/tau2-mobile-ui-tests.log`, run `1cc4cdc3-aaa8-4d4d-a7c2-f27ea9d38ff2`.
  Includes actual pointer dismissal over Stop, mobile topic/read behavior, and
  health windows. No mock of the renderer or local store.
- 20 recovery/health/safety tests passed:
  `/tmp/tau2-mobile-recovery-tests.log`, run `ed7be80f-106f-42bf-ab9d-8704c8bfc94d`.
  Real WebSocket peers exercise lost accepted acknowledgement, missing receipt,
  ordered same-ID retry after restart with another chat selected, and existing
  source/alias/storage recovery contracts. No paid provider call.
- Four focused safety tests passed after final transaction/backoff review:
  `/tmp/tau2-mobile-safety-tests.log`, run `0f55d1dc-3403-4cf5-bea7-59376c89cb8d`.
  Includes injected SQLite commit failure, temporary failure versus invalid
  attachment, explicit same-ID retry, and stale callbacks after lineage change.
- Sanscale variable-font GPU test passed:
  `/tmp/tau2-font-tests.log`, run `64b444f6-b566-48f8-97d0-38dd5e864f1f`.
  Real Roboto variable test subset verifies different advances and heavier bold
  pixels, instance deduplication, and unchanged regular pixels after using bold.
  The merged Sanscale API test selection also passed 5 tests (7 skipped by the
  repository's normal selection), run `c26e39c0-0d94-4c35-89c3-849195bd4661`;
  `/tmp/tau2-font-api-tests.log`.
- Managed Cargo frontend check with all host targets passed. Android ARM64 library
  check passed, including final native event ordering. Logs:
  `/tmp/tau2-mobile-check.log`, `/tmp/tau2-mobile-android-check.log`.
  Android reports five dead-code warnings in desktop input helpers; no errors.
- Java bridge compiled against Android API 35 with minimum API use guarded.
  `javac` reports deprecated API use, no errors. `git diff --check` passed.
- No Clippy, Cargo built-in test runner, automated formatter or full app suite.

## Device acceptance still needed

No Android device is attached. This pass does not certify physical IME animation,
Back behavior, rotation, OEM font matching or touch behavior. Verify on the user's
phone after a separately authorized client release: type a long draft, show/hide
keyboard, Done then immediately Send, Back without sending, switch topics, dismiss
notices over Stop, and inspect regular/bold/italic text and disclosure arrows.
