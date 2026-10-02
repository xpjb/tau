# Model catalog / manually entered GPT-6.1 Sol — October 2, 2026

Status: **fixed on `fix/tau2-model-catalog-refresh`, source only**. Not merged,
packaged or deployed. Worktree: `/root/tau2-model-catalog-refresh`.

The user manually entered `openai-codex/gpt-6.1-sol` in Quick Model and reported
that its token/context display still had not acquired catalog metadata. Manual
entries already reach the provider without an allowlist; that behavior is kept.

## Confirmed causes

1. The beta's saved Codex catalog was fetched September 25, 2026 at
   **09:33:02.174 UTC**. `begin` treated an authenticated matching cache as fresh
   indefinitely, and `restore` ignored its persisted fetch time. Selecting a
   missing exact ID could not cause a new lookup.
2. The authenticated Codex endpoint gates models by `client_version`. Two
   read-only GETs on October 2 returned nine models for Tau's **0.156.1**, without
   `gpt-6.1-sol`, versus ten for **0.160.0**, including that exact ID with
   **272,000** context tokens. The official Codex latest-stable release response
   identified `rust-v0.160.0`, published October 1 at **20:19:13 UTC**. No release
   date for GPT-6.1 Sol itself is inferred from that client release.
3. Retained runtime usage could keep an old capacity after a successful refresh;
   the session list preferred that snapshot over the refreshed catalog.

## Fix and bounds

- Use verified Codex catalog client version **0.160.0**. Save an optional
  `codexClientVersion` with the private cache; older/missing version stamps cause
  revalidation, not loss of otherwise valid same-identity limits. Cache schema
  stays 1; old caches are readable without a database migration.
- Revalidate on listing, viewing, exact-model selection and turn boundaries when
  the cache is at least one hour old or the requested exact ID is missing. This
  is **freshness**, not expiration: keep the last good same-endpoint/identity
  limits during a GET or transient failure. No inactive-provider polling loop.
- Coalesce in-flight requests per provider. Automatic retries/misses share a
  60-second cooldown after a GET. Manual refresh bypasses cooldown, not an active
  request. Existing two-request concurrency, timeout, size and provider bounds
  remain. Unknown capacities are never guessed or borrowed from another ID.
- Recompute warm and sleeping usage while preserving reported tokens, run status,
  detail and idle deadline. Changed state gets a newer revision so delayed pages
  cannot replace the refreshed capacity. Cold chats are not loaded to refresh
  metadata; their lists/read-only state derive limits from the current catalog.
- The Codex client version is still a maintained compatibility pin. A future
  provider-side version gate requires verifying/updating that pin; cache TTL
  alone cannot override server-side model eligibility.

## Validation

Before implementation, both the old-cache and retained-usage regressions failed:
`/tmp/tau2-catalog-regression-before.log`, nextest run
`cb5067ae-8ae3-4e1d-ab09-a37bc8667e8f`.

Managed native daemon/frontend all-target compiler check passed with existing
frontend dead-code warnings. Targeted nextest covered cache age, fresh exact-ID
misses, legacy client-version stamps, future timestamps, identity/endpoint
scoping, cooldown/coalescing, manual refresh and failure retention; authenticated
Codex/Chat Completions request paths; warm/sleeping revisions; stale-cache restart
without a new model turn or runtime; and the existing controller queue/settings
end-to-end flow and unknown-capacity tooltip.

- Seven initial daemon cases passed (`87079b47-3561-443c-bfe1-b00caf34995a`).
- Combined run `c969597b-6f0b-422d-bf2b-c892cc6a8b6a` passed 12 cases. Its new
  controller fixture initially used the wrong serialized API spelling; that
  fixture was corrected to `chat_completions` and only that case rerun.
- The corrected real-controller test passed in 0.305 seconds, run
  `e1e244da-a0d2-40f1-8573-526601cc4cef`: a manually saved quick model absent from
  a fresh cache reaches the local provider unchanged; its draft survives choice;
  reported tokens remain visible while the catalog GET is gated; releasing the
  GET updates the capacity without another prompt or manual refresh. Exactly one
  catalog GET and one scripted local turn occur. **13 distinct selected cases
  pass**, not a claim of a single green full-suite run.

Logs: `/tmp/tau2-catalog-validation.log`, `/tmp/tau2-catalog-controller.log`.
No Clippy, Cargo built-in test runner, unrelated lint rewrites or full suite.

## Rollout limits

No beta/stable restart, production cache/settings/auth write, credential
copy/refresh, live completion request, client packaging, version/protocol/database
schema bump or physical Windows/Android acceptance. Live diagnosis used only the
read-only catalog/release requests above. Existing beta **0.7.10 / protocol 22**
is unchanged. This daemon fix must be integrated and deployed before the user's
running beta gains the new behavior; existing matched clients can consume it.
