# Transcript prefetch and warm views — September 28, 2026

Source change on `fix/tau2-transcript-cache`; not packaged or deployed.

## Diagnosis

Tau 2 already has an account/daemon-identity-scoped SQLite replica under
`blocks/`. Verified content survives client restarts, with a 512 MiB CAS quota
and a 1 GiB SQLite page budget. This is an on-demand cache, not a complete offline
mirror of every conversation. Drafts, pending intents and saved downloads have
separate ownership and are not evicted with it.

The visible gaps were also caused above the disk layer: selecting another chat
discarded its view; a viewport change explicitly reprojected departing rows as
`Loading…`; only 32 preview groups were retained; history began loading just
180dp from the edge. Disk eviction recorded writes, not actual reads. Only the
selected chat had native transcript interests, so other ongoing replies waited
for selection even though their status was available.

## Behavior

- Keep at most four native chat views/interests warm. The selected chat is first;
  ongoing chats are next (recently used ones preferred), then recently opened
  chats, with catalogue activity as a cold-start fallback. Remember the last eight
  selections per account across restarts. Starter, provisional and missing-source
  chats are excluded from background selection.
- Background root/queue watches receive live updates. Fetch at most eight ordinary
  text/attachment-caption roots from the latest 32 headers, with an 8 MiB body
  admission budget per background chat. Do not page its root history or inherit
  open tool/Details preferences. Non-image files, images, thinking and hidden tool
  bodies are not automatically downloaded by these background interests.
- Background metadata and bodies use bulk admission and low QUIC priority, leaving
  the selected chat's two metadata/four foreground stream slots reserved. The
  server no longer promotes a bulk text watch back to foreground priority.
  The same connection, checkpoint yielding and verified-prefix resumption apply.
- The selected chat also keeps this small tail fetching while reading scrollback.
  Visible and copy interests take precedence; overlapping body IDs are deduplicated.
- Background fetching never selects a chat, marks it read, advances activity,
  creates a runtime, or submits a provider/tool action. It runs while the client
  process and connection are available, not as a new OS background service.
- Keep already-loaded scrollback across viewport changes and metadata/history
  refreshes. Each warm view retains up to 128 groups within the existing 8 MiB
  preview-byte bound and 256 KiB per-group bound. Current viewport hydration has
  priority over retained offscreen content. Replacements, tombstones, source
  changes and full resets still invalidate old data.
- Cold views leave memory, not disk. Reopening hydrates around the saved anchor,
  including offline/restart. Local drafts/intents survive view eviction.
- Fetch history two viewport heights ahead of its boundary, and request bodies
  with two-screen overscan. Existing in-flight/cursor guards prevent duplicate
  page requests; history still uses the existing scroll anchor.
- Navigation, initial hydration and Copy refresh disk-read recency in one batch.
  Stream reprojection reuses write recency rather than adding UI-thread fsyncs
  per chunk. Merely looking up metadata does not protect unread content. No disk
  quota increase, cache TTL change, polling timer, wire-shape or schema change.

## Validation

Managed Cargo workspace/all-target compiler check passed on the final source.
An initial focused nextest run passed
**65/65** tests (`9410d8d9-0a4b-4aff-a7e4-e2187c2b2215`), covering native cache and
transport, quotas, queue/body reuse, reset fencing and existing GPU scroll tests.
An additional **7/7** passed (`52355a49-8314-4ee5-9fe6-6cc5481afc21`) covering
retained controller views, disk-only anchor restoration, bounded/persistent warm
selection and disclosure planning. Its actual daemon/controller/local-provider
scenario receives a **live streaming prefix and completed reply in an unselected
chat**, preserves unread state, reconnects a restarted client, receives another
unselected reply and reopens both transcripts after the daemon is stopped.
Provider calls are explicitly gated local fixtures, never paid requests.

A final **15/15** passed (`a67bb5a5-b8bf-4dca-a37b-0b765f2d71b3`), including the
selected-chat tail while reading scrollback, the streaming/restart scenario,
existing recovery tests and the real two-client loss/delay/bandwidth regression.
A further **6/6** passed (`3053dc28-2aee-4494-bc9d-64e0f63b05a3`) after restricting
recency writes to actual navigation/hydration/copy rather than every streamed
projection. It checks read-based eviction, both memory bounds, retained views,
offline anchors and zero extra recency writes during ordinary live projection.
These are focused runs with overlapping coverage, not a repeated full suite.

One earlier test invocation timed out on the managed build lock without running
checks; it was not bypassed. No Clippy or built-in Cargo test runner was invoked.
Physical-device responsiveness, suspended-app fetching and arbitrary WAN behavior
are not certified. No daemon restart, package delivery or production data change.
