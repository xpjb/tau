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

- **No chat-count cutoff.** Subscribe to every running chat, plus chats viewed or
  active in the last 24 hours (including activity from other clients). Keep a
  settling run eligible while its final body/queue catches up. Old inactive
  archives are not subscribed merely because they exist. Recency persists per
  account without an eight-chat list limit. Starter, provisional and
  missing-source chats are excluded from background selection.
- Background root/queue watches receive live updates. Fetch at most eight ordinary
  text/attachment-caption roots from the latest 32 headers, with an 8 MiB body
  admission budget per background chat. Do not page its root history or inherit
  open tool/Details preferences. Non-image files, images, thinking and hidden tool
  bodies are not automatically downloaded by these background interests.
- Batch background root/queue feeds **across chats**, within 16-request/encoded
  size limits, rather than keeping a separate stream occupied per chat.
  Background metadata and bodies share six bulk slots and low QUIC priority,
  leaving the selected chat's two metadata/four foreground slots reserved.
  The server no longer promotes bulk text back to foreground priority. The same
  connection, checkpoint yielding and verified-prefix resumption apply. Background
  bodies are finite catch-ups: once current bytes are cached they release their
  slot, even during a live turn. Batched metadata announces the next append;
  idle body watches do not hold slots for five seconds per chat.
- Keep decoded UI views independently of network subscriptions: a 32 MiB accounted
  preview/header/queue working set, with the selected view protected and other
  views evicted least-recently-used. Small chats can all fit; there is no four-view
  cutoff. This is conservative view accounting, not an exact process-RSS bound.
  Background chats need no UI Feed at all to advance headers/bodies on disk.
- Replan only changed background scopes. Their content chunks do not redraw the
  open conversation for every token. Remember successful prefetch versions so
  disk eviction cannot cause an endless idle redownload loop. Replacements,
  source/cache resets and explicit viewing/Copy still fetch missing bytes.
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

## Follow-up: remove chat-count limits

The first implementation's four-chat/eight-recent limits were rejected in QA.
They are removed rather than merely raised. The follow-up focused nextest run
passed **6/6** tests (`e1e5dda5-5776-4d45-a72c-eb1b3ad237bb`):

- 13 recent selections plus other never-opened running chats all participate;
  missing/starter/cold chats are excluded and a settling run remains eligible.
- 37 chats' 74 root/queue feeds fit in five bounded cross-chat batches, with no
  skipped feed or duplicate request.
- Memory pressure evicts only decoded views, not disk data or subscriptions;
  prefetch completion avoids cache-eviction/redownload loops.
- The actual daemon and two production controllers synchronize **12 recently
  active chats and six simultaneous live replies**, with only the selected chat
  materialized as a UI Feed on the observing client. After restarting that client,
  all six final replies arrive without selection or unread changes. All 12 chats
  then reopen offline and remain in RAM because their small views fit the byte
  budget. Provider calls are gated local fixtures only.

The broader regression run passed 38 checks, including the impaired-link test;
its many-chat check exposed an intrusive test probe repeatedly reopening the
writable cache and contending with sync. The probe now inspects SQLite read-only.
Both real-daemon scenarios then passed (`c3cf786a-3ca9-4bb3-a577-937332490841`).
The finite-catch-up follow-up passed **6/6** checks
(`cc46b926-352b-4f09-9faa-14e0281df890`), including both real-daemon scenarios,
batch/eviction regressions, the existing delayed-plan reuse regression, and an
actual QUIC test proving that an unsealed background body ends promptly, releases
its bulk slot, and resumes an appended suffix from its verified offset.

The remaining validation below records the earlier implementation's focused
coverage; it is not evidence that a four-chat limit remains.

## Initial validation

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

## Integration validation — September 28, 2026

The merge into `tau2-integration` preserves download-notification navigation's
explicit history paging alongside the two-viewport ordinary prefetch threshold.
Both unreleased QA entries are retained.

Managed Cargo nextest passed **8/8** focused merged-tree tests
(`0c7510e0-c8cf-4ca0-9f40-357fc69bd21f`): five navigation regressions, saved-scroll
restoration, and both real-daemon background-fetch scenarios, including 12 chats
and six simultaneous live replies. The workspace/all-target compiler check and
`git diff --check` also passed. Two earlier attempts timed out at the shared build
lock before running tests; the lock was not bypassed. No packages were built and
no services were deployed or restarted.
