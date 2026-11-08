# Codex renewal and sign-in recovery

The beta's old shared-credential branch rejected an expired/rejected access token
without attempting renewal. In the reported incident, beta failed at 09:03–09:05
UTC on October 5, 2026; stable updated the shared credential file at 09:07. A
subsequent read-only beta quota request succeeded. Beta had **no independent
Codex record**. This was automatic OAuth renewal, not a user browser sign-in.
The old error did not distinguish expiration from a provider-rejected token.

## Small shared-refresh path

`AuthStore` now participates in the **existing** stable Pi lock: an empty
`auth.json.lock` directory with an mtime heartbeat, and 30-second stale recovery.
The contract was checked against the deployed Pi `02b081c`, not a different
installed version. No Node/Pi executable or new service is used by beta.

Credential reads and read/refresh/write operations hold the lease. After acquiring
it, beta rereads the current record, reuses another process's renewal, and writes
rotated credentials back to the selected file before release. Other provider
records are preserved. It never copies the shared record into beta's file. The
rotation task survives cancellation of the requesting model operation. Lease
identity checks fence lost owners; heartbeat and release cannot update/remove a
replacement owner. Nonempty directories and symlinks are not recursively removed.
An existing independent beta login still takes precedence.

## Only genuine login failure needs a browser

Missing/revoked OAuth credentials or persistent Codex rejection produce a typed
sign-in-required error. Routine expiry now refreshes instead. Transient OAuth
network/server/lock failures do not create a sign-in-required prompt.

The authenticated control connection offers a short-lived, coalesced device flow.
A selected chat's authentication failure opens **Sign in to Codex**, also reachable
from Settings and the error's link/context menu. The dialog displays the official
`https://auth.openai.com/codex/device` page, a copyable code, approval/expiry/error
status, cancellation, and explicit Resume. It only opens the fixed official URL.
Provider access/refresh tokens and device secrets remain on the daemon. The code
and state are transient, never added to the transcript or durable request outbox.
No sign-in waits on the credential lock for human approval; credentials are reread
and merged only at approval. Cancelling/obsolete attempts cannot overwrite a later
login. Source-bound dialogs and request IDs reject stale completions.

Completing browser authorization writes a separate beta OAuth session, for the
account the user chooses. This is an optional fallback, not required for sharing
or routine renewal. No paid/model/tool operation is replayed by signing in.

## Error visibility and Copy

`errorMessage` used to be rendered only in a single-line heading, while Copy read
only the (often empty) body. Errors are now wrapped transcript content with a short
heading, and both immediate and complete-body Copy include the error plus any
partial answer. Saved local failure details are included too. Optimistic local
message bodies remain copyable before remote body replication. Desktop clipboard
initialization failures are retried/reported rather than silently ignored.

## Validation

- Managed daemon/frontend all-target compiler check passed on the integrated fix.
- Initial focused auth/control/UI nextest: **8/8**; existing connection/dialog
  cases: **18/18**. Desktop and phone-sized real-render previews verified the full
  URL, code and instructions; actual context-menu Copy includes the error.
- Final shared-refresh/auth nextest: **9/9**. Two independent AuthStores rotate a
  stale shared credential exactly once against local OAuth fixtures, preserve
  other providers, and make no beta copy. Invalid grant preserves the old record
  and requests sign-in. Expiry/cancel/source fences are covered.
- **1/1 deployed-Pi interoperability test passed** in both directions, including
  holding beta's lease beyond Pi's synchronous ten-second stale threshold.
  It used the deployed `proper-lockfile` module only in an isolated temporary
  fixture; no production credential was refreshed or modified by testing.
- All provider-turn/auth-flow tests use local fixtures. The one live quota read
  was diagnostic and did not perform a completion, renewal or sign-in.
- No Clippy or Cargo built-in test runner. No physical Windows/browser approval
  claim; successful package compilation is recorded separately at rollout.

Logs and rendered previews: `/tmp/tau2-codex-signin/`. Protocol **24** requires
matched beta clients/daemon. No database schema change or stable executable change.
