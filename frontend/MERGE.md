# Independent backend merge seam

Base: `025c2b4` (`master`). Frontend worktree: `/root/tau-rust-frontend`, branch
`tau2/rust-frontend`. No Pi implementation/lifecycle behavior was redesigned here.
The backend conversion is expected to be merged later, not cherry-picked into this
branch while it is in progress.

## Main overlap: wire types

`tau-protocol` now owns the existing **protocol 10** contract. There is no intentional
wire/version change. It is a small Serde-only crate, not a dependency on taud:

- `ClientRequest`, `ClientCommand`, `QueueOperation`, `ServerMessage`, session/model,
  extension, upload/crash, event, snapshot/page/delta and queue data types.
- Both serialization directions are available. Omitted change vectors default to
  empty. `Hello.daemon_version` / `TitlePrompt.default_prompt` are owned Strings,
  rather than `&'static str`, so a client can deserialize them.
- Attachment source paths are still skipped by Serde, as before.

**Merge the backend's authoritative contract into this crate**, then adapt the
client match arms. Do not restore a second Rust copy in the daemon. If the backend
changes the wire semantics, bump the shared version; the preview rejects a mismatch.
The untouched Kotlin client remains a protocol-10 reference, not a second Tau 2
protocol owner.

## Mechanical daemon changes to preserve or discard appropriately

- `daemon/src/protocol.rs`: re-exports the crate. `ResponseError` and
  `ContextUsagePi` keep Pi-specific error/context conversion out of the wire crate.
  A native backend can remove those adapters rather than preserving Pi concepts.
- `state.rs`: re-exports `SessionModel`.
- `transcript.rs`: re-exports wire data; `EventPi` / `QueueStatePi` hold old Pi
  parsing methods. The local `TranscriptChange` **wrapper** owns `wire`, `head`
  and `bumps_chat`, with Deref for existing projection code. `head` and activity
  bump hints never became shared client state. The backend may replace this whole
  projection layer with its own implementation.
- `manager.rs`: imports the old parsing traits; two outgoing update constructors
  move `change.wire` into the message.
- `server.rs`: imports `ResponseError`; two static string literals become `.into()`.
- One transcript test borrows `interrupted.queue` rather than moving through Deref.

These files may conflict with the backend rewrite. Resolve toward the backend's
logic and retain only the shared-contract boundary—not the transitional shims.

## Acknowledgements

The preview does not anticipate a guarantee the old daemon cannot make. Prompts
and controls are persisted before transmission, with stable request IDs. Socket
loss preserves uncertain actions; reconnect supplies pending IDs to OpenSession.
A snapshot/update's delivered IDs (or queue membership) reconcile local work.
Nothing automatically resends on restart, reconnect, or a stale socket epoch.

After native taud durably acknowledges a request, simplify
`frontend/src/store.rs::Delivery` and `controller.rs` response/reconciliation.
Define what the ack guarantees (journaled request vs execution started), when an
ID can be forgotten, and how reconnect discovers accepted-but-not-yet-executed
work. Don't replace uncertainty with optimistic "delivered" on a socket write.
There is no new retry/idempotency assumption hidden in this branch.

## Transfers and build files

The existing Iroh implementation is reused directly. UniFFI is behind an `ffi`
feature, **default-on for the legacy build**, off for `tau-frontend`; `bindgen`
enables it explicitly. A later legacy removal can delete that surface entirely.
No grant/QUIC protocol changes were made.

Merge workspace/package manifests first, then resolve/regenerate `Cargo.lock`
with Cargo. Don't take one branch's lockfile wholesale over the other's new
backend dependencies. `windows/` remains excluded. `build-windows-sfx.sh --beta`
now packages the native frontend through the existing installer; no flag retains
the Kotlin path. `windows/channel.rs` selects isolated install/Start Menu names,
and `tau-beta-launcher` dispatches a native version instead of Java. Preserve this
parallel channel until an explicit stable migration. Both use the static-CRT
target setting; regenerate the separate Windows lockfile only if its dependencies
change. These packaging edits do not depend on the backend rewrite.

## Tests at merge

`frontend/tests/end_to_end.rs` currently uses `taud::Config` and the daemon's
existing `tests/fixtures/pi.py`, **only as a test dependency**. Replace its fixture
setup with the backend's deterministic native agent/provider fixture when Pi is
removed; keep the controller/transport-level scenarios. Production frontend code
has no taud/Pi dependency. The transport fault peer, grant/QUIC test, ordering
checks, and Markdown tests are independent of Pi.

Run `cargo nextest run --workspace`, rebuild both Android ABIs and Windows, then
repeat device send/reconnect/restart acceptance. No production service/config or
existing installed client was changed by this branch.
