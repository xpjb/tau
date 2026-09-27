# 013 — Queue-to-history content-reference lifetime

Status: **Open; deferred at the user's request on September 27, 2026.** Record the
finding, not an approved redesign or an implementation task for this session.
Treat the reported edge case proportionately; production frequency is not known.

## Established problem

Queue metadata advertises `queued:<request-id>` separately from its body bytes.
When the daemon consumes that item, `daemon/src/blocks.rs::queue` removes its
block; the history projection uses an event-derived block ID. A client can
therefore receive valid queue metadata, request its body, and encounter `Unknown
block` because consumption happened before the read reached the server. The
queue-removal metadata has not necessarily reached the client yet.

This is a content-reference lifetime problem during an ordinary state transition,
not evidence of a lost message. The pressure workload uses two already-connected
clients sending messages; reopening on another device is not required. Whether a
particular request could have been avoided using already-known text affects how
the bug is exposed, not whether the lifetime problem exists. Do not block a fix on
more authorship diagnostics or on reproducing the same scheduling-sensitive seed.

## Evidence and related work

- [Default seed-91 failure](../docs/network-pressure/recovery-default-91-content-failure.json):
  `Content sync: Unknown block` for a queued ID, while the local header was still
  present (version 1, length 37, sealed). The associated workspace run was
  **232/233**, with this content error as the failure. The report recorded completed
  recovery transfers and one canonical record per authored message.
- [Dodgy seed-173 failure](../docs/network-pressure/dodgy-173-failure.json): two
  unexpected `Content sync: Unknown block` alerts. Passing repeats do not close it.
- These immutable reports were retained in `bcc541b` on
  `fix/tau2-network-pressure`. The real controller/daemon/local-provider harness is
  `frontend/tests/network_pressure.rs` on that unmerged QA branch.
- The earlier complete, already-held-body delayed-plan fix was merged into `tau2`
  at `bdc1722`. Preserve local text reuse and that regression; it does not establish
  that every in-flight content lifetime case is covered. The original failed
  report did not establish the author or cached byte count of its particular fetch.

## Scope decision still needed

Stable content identity across promotion is a candidate, not a settled decision:
queued versus committed-to-history can be state/placement metadata about the same
message, while unchanged content remains readable under the same identity. Actual
edits still need correct versioning; merely reusing an ID is insufficient if its
body is deleted during the handoff.

Compare a small compatible identity/lifetime simplification with a narrowly scoped
fix that confirms obsolete interests against fresh metadata. Do not undertake a
broad storage, protocol or history refactor solely to address this edge case.
Never convert arbitrary missing advertised content into success or merely hide
its error. Do not add speculative delays or re-download text the client holds.

## Acceptance when revisited

- Add a deterministic regression: deliver queued metadata, consume/promote the
  message, then issue or finish the old body request before removal metadata
  arrives. Verify normal handoff without a spurious popup or stranded interest.
- Preserve available local text and exactly one canonical message; no control
  replay, loss or unnecessary fetch of already-held bytes.
- Genuine missing content that is still advertised must remain an error. Edits
  must not let an old version or offset adopt different bytes as the same content.
- Run relevant targeted and pressure tests after choosing a proportionate fix;
  passing fuzz repeats alone are not proof of correction.
