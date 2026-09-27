# WebSocket acquisition and local message continuity

Branch: `fix/tau2-connection-message-continuity`, based on `tau2-integration`
(`origin/tau2`) at `c491518`. These changes are not deployed or packaged.

## Agreed connection model

The ordinary indicator measures only the control WebSocket, not QUIC. There are
two operating states: a usable WebSocket, or no WebSocket / acquiring one.
Unconfigured settings and terminal errors are explicit non-operating exceptions.

Both operating states use actual attempt-start timestamps:

- With a socket: last ping age, either its remaining reply deadline or the next
  ping countdown, and acknowledged RTT statistics. An unanswered ping is not an
  RTT sample. Old RTT history is labeled as belonging to a previous socket until
  this one answers; it is never shown as a current pong.
- Without a socket: acquisition attempt number and age, either its remaining
  deadline or the next permitted attempt countdown, and the last failure reason.
  A reason is historical context, not a competing "retrying" state.

Timing policy:

| Operation | Policy |
| --- | --- |
| Acquire socket | One **5 s** deadline across DNS, TCP, TLS, WebSocket upgrade and Tau hello |
| Failed acquisition | At most one in flight; minimum **1 s between starts**, not after failure |
| Existing socket | Ping every **2 s**, without overlapping probes |
| Missing matching pong | Discard socket at **5 s**, acquire a replacement |

A 10 ms refusal waits the remaining 990 ms. A 2 s failure or 5 s timeout retries
immediately. There is no exponential backoff, jitter, or additional hello timeout.
A pong taking longer than the normal cadence permits the next probe immediately.
RTT/deadlines include the bounded WebSocket writer queue, as before.

The hover refreshes its clocks while visible; hidden indicators wake only for
relevant color boundaries or real events. Repeated failed-acquisition events are
coalesced within each no-socket episode, so sleeping UIs do not accumulate a retry
backlog; disconnect boundaries between successful sockets are retained for intent
recovery. Content transport counters remain in
copied diagnostics, and content failures keep their existing specific reporting.
A WebSocket pong does not certify QUIC health or provider progress.

## Message ownership and body reuse

1. Save the user's intent in authored SQLite before network effects, as before.
2. Acceptance changes delivery status; it is not permission to discard the local
   display copy. A replicated header alone also does not establish body readiness.
3. User and queued-text headers carry `meta.bodyHash`: BLAKE3 of the exact sealed
   body. The client can seed its existing verified block cache from locally held
   text only when source lineage, chat, request ID, length and digest match.
4. Matching bytes are committed before projection and body-interest scheduling.
   The production scheduler does not open a body stream for a complete sealed
   block. Changed, unknown or evicted content follows normal verified fetching.
5. Retire the outbox copy only after a complete canonical queue representation or
   a cached, displayable user-message representation exists. Viewport lag must
   not retire the overlay before the replacement can be drawn. Large messages
   still use bounded previews; Copy can use their complete cached bytes.

Pending, queued and canonical user rows share one request-based display identity.
Native block-interest IDs are separate, so preserving the row key does not break
viewport fetching. A still-needed authored overlay remains copyable and is not
presented as verified canonical content. Explicit rejections, uncertainty, source
restore fences and original-ID receipt recovery retain their existing semantics.
Handled slash commands still finish without inventing a transcript message.

### Queue-to-history ordering

Root and queue directories can arrive in either order. If a queue tombstone beats
the root cursor, retain that row as **Synchronizing message** until the root catches
up; do not allow stale edit/delete/run-prefix actions. Once the canonical user row
exists it wins the overlap, so there is never a second queue row for that request.
Incomplete retained rows stay incomplete and cannot prematurely retire an intent.

The queue root carries a small membership/revision hash. Membership changes thus
advance the root directory even if the queue-state body itself is unchanged. This
allows both consumption and explicit deletion to converge without a blank interval
or an indefinitely retained ghost row.

### Storage and compatibility

Reusable input candidates live in the disposable account replica, independently
of queue block IDs, so deleting a queue block does not delete the only reusable
copy before its history header arrives. Retention is capped at **256 candidates /
32 MiB** per account replica, inside its existing database page cap. Candidate
hashes are reverified before adoption. Source-lineage changes and Clear replica
cache clear them; neither deletes authored outbox work. Failure to save an optional
candidate does not prevent sending a durably saved intent.

The metadata and replica table/index additions are additive: control protocol 18,
daemon schema 5, authored schema 2 and replica schema 2 are unchanged. Old peers
without body hashes fall back to fetching; upgraded clients still retain their
local overlay until canonical content arrives. Old historical source blocks need
not be rewritten just to add hashes. Use an updated daemon and client for the
normal zero-download local-message path.

## Validation

All Rust commands used the managed Cargo wrapper; no Clippy or built-in Cargo test
runner was used. No live services, accounts or external provider calls were used.

- Compiler check: frontend and daemon passed.
- Focused transport/recovery/cache/GPU run: 48/49 passed; its protocol-mismatch
  regression exposed a moved lineage check. Restoring version-check precedence
  fixed it, and its focused rerun passed.
- Actual-daemon/controller E2E, impaired-link TCP/QUIC, source fencing, and a
  production content-scheduler regression: **10/10 passed**, nextest
  `f3d6fa02-f300-4bb1-9ef4-72ff103bef0a`.
- Queue handoff, E2E, GPU message continuity and source header checks after the
  directory-ordering change: **6/6 passed**, nextest
  `ec6767a2-4682-4c2e-86af-e27c667426a6`.
- Final frontend library/GPU/cache/controller tests plus source header regression:
  **74/74 passed**, nextest `c28d27ca-7a4d-4c4c-8e44-1643d4371ff6`.
- Expanded queue-ordering checks (incomplete handoff and explicit deletion) and
  root-membership revision check: **2/2 passed**, nextest
  `0b8e82ba-61fd-4966-b273-25edb9207028`.
- Final transport/recovery checks including 1,000 coalesced acquisition failures
  during UI sleep: **19/19 passed**, nextest
  `99a25684-dc69-4d85-9054-308cc8784d6f`.

The live native scheduler test counts backend reads: it fetches an unknown remote
body while issuing **zero body-read requests** for the known authored input.
Other regressions cover shared acquisition/hello deadlines, fixed attempt spacing,
immediate recovery after heartbeat timeout, receipt-before-body ordering, restart,
UTF-8/large previews, scope/digest/lineage mismatch, corrupt candidates, queue-first
handoffs, stable rendered identity, and stale viewport interests.

Physical-device behavior and arbitrary WAN/mobile reconnection performance have
not been certified. There is no deployment, package build, merge or push in this
change.
