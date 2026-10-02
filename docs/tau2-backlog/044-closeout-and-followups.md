# 044 — Practical source closeout and remaining work

The user authorized a bounded final deletion pass, triage of the remaining work,
and merging the simplification **and** the tested compaction/pause fix into
`tau2-integration`. This is source acceptance with recorded debt, not achievement
of the original 5,000-line target or authorization to package/deploy/restart.
Integration publishes to `origin/tau2`; stable/master remains untouched. Source is
now merged as `73a617f` and `1cb2282`; [final merged-tree acceptance](../docs/tau2-simplification-closeout.md)
passes 328/328 tests plus compiler/platform/rustdoc checks. No deployment occurred.

## What closed locally

- The named 036–041 ownership/migration paths and 042 compatibility/matrix
  retirements are implemented. Old projection/Row/Part assembly, legacy fixture
  update/page adapters, synthetic lifecycle events, root shadow routing, card
  registry/destination envelopes and competing reading bookkeeping are absent.
- Final bounded deletion: unused `Editor::contains`, `Controls::contains` and the
  unordered `Renderer::hit_text` path. Real control bounds and ordered text
  selection remain. No tests or product features were removed in this last pass.
- The historical size report no longer tries to decode changed binary assets as
  UTF-8. Textual Java/host changes remain counted. Reviewed daemon test-module
  families are classified as tests; no runtime code is relocated for the count.
- Native IME adapters are **not** dead merely because desktop builds warn about
  them. Both the actual Android bridge and retained editing tests use them.
- Form versus dynamic Controls, attachment display mapping, code-browser gesture
  state, verified cache indexes and source/request fences still do real work.
  No large safe local deletion was demonstrated; replacing them now would be a
  new feature-sensitive rewrite, not a justified closeout deletion.

Before the independent pause merge, charged reduction is **1,570 raw / 564
same-format**, leaving **3,430 / 4,436** of the original target. Production is
**581 raw smaller / 291 normalized larger**. [The size ledger](../docs/reviews/retained-ui/completion-size.md)
records all replacement/test/shared-code costs. The pause fix is separate useful
work and its additions must be shown separately, not attributed as deletion savings.

## Remaining-work disposition

| Work | Disposition | Concrete next action / stop condition |
| --- | --- | --- |
| [013 queued-content lifetime](013-queued-message-content-lifetime.md) | Open, independent reliability bug; not fixed by stable client display identity or the compaction fix | Reproduce metadata → consumption/promotion → delayed old body read deterministically. Compare bounded reference retention with authoritative obsolete-interest reconciliation; choose the smaller correct fix. Preserve body integrity and do not hide a still-advertised missing body. |
| [014 stalled native transfer recovery](014-native-data-recovery-after-outage.md) | Deferred diagnosis; preserved failure evidence still applies | Capture the failing file's wait boundary (acquisition/credits/receive/commit/export). Do not claim a shared receiver or larger timeout fixes an unidentified cause. |
| Physical Windows/Android acceptance, [043](043-simplification-acceptance.md) | Open; compiler/headless results are not device QA | Windows/DPI and Android IME/touch: opaque overlays, nested hover/ripple/disabled blocking, editing, stationary selection, scrolling, code/files and attachment navigation. Reuse device QA rather than new coordinate matrices. |
| 5,000-line reduction / further structural deletion | Deferred, explicitly unmet; no proven inventory covers the gap | Only restart a slice with a named obsolete owner/algorithm, all callers and a credible net removal. No feature removal, relocation, minification or broad operation-store/transport migration to satisfy a number. |
| Further model/transcript test consolidation | Optional, subordinate to real duplication | Delete only redundant implementation-shape coverage after identifying the retained observable guarantee. Keep native body/cursor/source/no-replay and real daemon/two-client coverage. No blanket test-count target or test-only global action adapter. |
| Earlier operation registry, catalogue and receiver proposals | Still deferred/shelved per [triage](triage.md) | Require an independent concrete need and scope decision. Their withdrawn LOC estimates do not finance this closeout. |
| Source integration and release | Both source merges complete; deployment is not authorized | Merged-tree acceptance is recorded in the closeout document. The unrelated old `tau2/qa-remove-pause` WIP is preserved. Build/release matched clients/daemon only on separate authorization; do not treat source integration as device acceptance. |

The final local deletion check passed frontend all-target compilation and 22
focused editor/control/IME/lifetime nextest cases (`33404561-5ba1-4e68-804d-9d4bfa1a3686`).
The preceding simplification runtime passed 325/325 workspace tests. Neither is
substituted for fresh merged-tree validation; integration's handoff records that
result and the pause scenarios together. No Clippy or built-in Cargo test runner.


Combined accounting including the independent pause fix is **1,430 raw / 323
normalized lines smaller** after all outside charges; its additional runtime and
regression coverage is recorded separately in the ledger. Neither this figure nor
the green merged suite closes 013/014 or marks the original size target achieved.
