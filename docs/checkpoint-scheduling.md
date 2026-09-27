# Checkpoint-based native watch scheduling

Status: **implemented and locally tested; not deployed or packaged**.
Branch: `fix/tau2-checkpoint-sync`, based on `origin/tau2` at `a3704e9`.

## Decision

Keep the unified block transport, verified cache and existing reserved stream
classes. Remove the client's wall-clock cancellation used to rotate live watches.
Scheduling now relinquishes a stream at an explicit protocol checkpoint, rather
than cancelling an arbitrary partially received response and reconstructing it.

This is the smallest justified foundational simplification identified in the
review. Changing message identities or consolidating durable operation state
would affect persistence, queue/history projection and recovery semantics well
beyond this demonstrated problem. This change does not declare those designs
sound or claim to solve them.

The previous five-second client lease could expire before a metadata page's
closing `Page` frame arrived. The accumulated records were then discarded, and
the next request used the same previously committed cursor. A sufficiently slow
but completing response could repeatedly make no durable progress. Similarly,
a content frame slower than the scheduling lease could repeatedly lose its
uncommitted prefix. A scheduling policy was acting as a response deadline.

## Wire contract

This branch requires **control protocol 20 and native ALPN `tau/blocks/2`**.
The baseline used control protocol 19 and `tau/blocks/1`. Both client and daemon
must be updated together. Do not silently use the new client with an old server
that would ignore the scheduling request and never yield. Existing database
schemas, source lineage, block identities and body versions are unchanged.

- `Watch.scheduled = true` opts into cooperative stream rotation. Ordinary bulk
  downloads, immutable control descriptors and unscheduled watches retain their
  existing finite/continuous behaviour.
- The server retains the five-second **scheduling quantum**, but tests its expiry
  only after a complete bounded round of requested feed pages or a complete body
  header/range. It never splits a metadata page to meet that quantum.
- All members of a metadata batch get a page opportunity before yielding. A slow
  first feed must not force every renewal to restart ahead of the later feeds.
  Existing limits remain: at most 16 feeds per batch and 32 records per page,
  with bounded frames and application credit.
- `Yield` terminates that stream, not its interest. It follows the checkpoint in
  the same ordered stream. The client commits preceding pages/ranges, releases
  its permits, then reopens from its committed cursors and verified prefixes.
  Renewals use the existing class semaphore queues; no new scheduler queue or
  connection is introduced.
- An idle scheduled watch also sends `Yield` when its quantum expires. Idle
  subscriptions therefore do not permanently occupy the metadata slots.
- `End` retains its meaning: a finite history request or completed/sealed body
  is complete. It must not be reopened solely for fairness.
- The frontend rejects `End` or `Yield` if a metadata page remains unfinished.
  A cooperative yield is counted as a completed stream, not a cancellation or
  integrity failure.

The quantum is **not a hard bound on response duration**: a slow checkpoint may
take longer. Existing transport/credit IO timeouts remain unchanged. User
cancellation, interest removal, source changes and replica-reset fences can still
cancel obsolete work immediately. This change does not add automatic replay of
controls or paid work, and does not restart a healthy native connection.

A complete scheduling round is not a transactionally atomic multi-feed snapshot.
The feeds still have their existing independent cursors and cache transactions.
This change establishes a progress-preserving scheduling boundary, not a new
cross-feed consistency guarantee.

## Validation

All runs used the managed Cargo wrapper and nextest, with offline dependencies.
No Clippy, built-in Cargo test runner, live service, device deployment or external
provider was used.

1. **Red regression on the original implementation:** a real QUIC server with a
   six-second metadata backend read returned to the frontend at about five
   seconds without committing the requested metadata round. Nextest run
   `7174961a-0be6-4193-b73f-6780e1f91ce8` failed the checkpoint assertion as expected.
2. **Two new frontend regressions passed**, run
   `8fed34b8-b8fb-4741-bf8d-2d456c7397c1`:
   - A six-second metadata read commits both the first and later batched feeds
     before yielding, without fetching their bodies.
   - A six-second first chunk commits before yielding; renewal requests exactly
     the verified offset, completes with the expected bytes and retains one
     native connection. Neither path records scheduling as cancelled IO.
3. **All 13 transfer-library tests passed**, run
   `91b2b96a-7509-4c1f-9edc-2329274471c2`. New tests cover idle-slot release/FIFO
   admission and finite history/content completion. Existing coverage includes
   credit bounds, reserved classes, authorization, compression, resumption,
   IPv6 and 16-peer fanout.
4. **41 focused frontend regressions passed**, run
   `42e180a4-1eb7-4e28-8fd9-81dfc17b9c66`: block-cache/scheduler tests, recovery,
   control probes, and the existing real-daemon/two-controller impaired-link
   workflow with a local unpaid provider fixture. The latter exercises shared
   TCP/UDP shaping, upload, cancel/resume, concurrent control and content hashes.

Total: **56 passing targeted tests**, plus the intentionally failing pre-fix
regression. No entire workspace/GPU suite or release build was run.

## Limits and follow-up

This does not close backlog 013 (queued-content lifetime) or 014 (the unexplained
post-outage per-file progress gap). It does not unify application delivery states,
change file-download retry policy, or certify arbitrary WAN/device behaviour.
Those require their own evidence and design decisions, not claims based on this
scheduling fix. No merge, push, deployment or package delivery is implied.
