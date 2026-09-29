# 042 — Delete brittle/redundant tests and their compatibility code

Status: **Selected; starts immediately and accompanies every slice.** Supersedes
old 034's “preserve all assertions” policy. No coverage-percentage or test-count
quota. Fewer tests with stronger guarantees is the intended result.

## Concrete disposition at `33f7d6f`

Physical file sizes below are **gross inspection areas**, not additive savings.
Names without a directory are under `frontend/src/app/`.

| Existing test/code | Action | Surviving guarantee / owner |
| --- | --- | --- |
| `icon_controls_tests.rs` (121 lines) | Delete glyph-alpha, exact 40×40, margin, y=16 and tail-button-placement assertions; remove standalone layout tests, fold only needed actions into a small control smoke case. | Settings opens; Tail appears when relevant and follows latest, 037/041 |
| `editor_tests::composer_placeholder_and_single_line_caret_are_vertically_centered` | Delete centering/layout test; do not retune its tolerances after form redesign. | Actual editing/IME/scroll/clipboard tests remain; placement gets visual review |
| `project_tests.rs` (265) | Delete exact x=299 / y=140..175 separator, 34/40px row and gap assumptions. Collapse duplicated desktop/mobile scenarios and controller ordering assertions already covered in `tests/activity.rs`. | Keep one UI ordering/reveal/navigation smoke and meaningful clipping/gesture assertions; model persistence at its owner |
| `composer_status_tests.rs` (52) | Drop eight-level × three-size layout matrix, draw-count=2 and fixed 11px gap. | One narrow-layout visibility check + existing `tests/thinking_level.rs` state behavior; no second matrix |
| `download_render_tests.rs` (185) | Reduce all-state × size × hover/press gallery assertions; remove one-draw/text-only/exact-height implementation contracts when chrome changes. Keep existing helper data only while useful callers exist. | One compact fit/clip scene; real Save/Open/Extract intent, retry and source safety in interaction/model tests |
| `scroll_tests.rs` (199) | **Delete in 041**, including module declaration; no mechanical port of old body. | Necessary chat-bound reading-state assertions belong to new owner/existing navigation case, 012/041 |
| `test_ui.rs` (345) | **Delete after useful callers migrate.** `FixtureChoice`, global `placed_controls`, action conversion switches and App forwarding methods preserve the old interface solely for tests. | Access actual retained owners/controls; a small local selector is fine, no replacement global action/selector registry |
| Dialog test assertion “Migrated fields/buttons never register legacy actions” | Delete obsolete migration assertion, not merely rename it. | A real focus/dispatch test, 037/038 |
| `ui/dialogs/tests.rs` (274), `ui/lifetime_tests.rs` (239), `ui/attachments_tests.rs` (157), `hover_tests.rs` (127) | Consolidate overlapping setup/scenarios; keep only distinct behavior. Replace weak whole-frame “pixels differ” evidence for opacity with 036's actual occlusion assertion. | One shared-control/capture table, one real nested control+scroll scene, one form callback/replacement case, small source/lifetime checks |
| `control_tests::own_message_keeps_one_display_identity_through_receipt_queue_and_header_only_history` | Move logical assertions to 040's model boundary, delete GPU and rich-Row scaffolding. | One message and retained authored text through handoff; one integration smoke, not duplicate transition matrices |
| `tests/recovery::retained_history_delta_gap_and_stale_page_are_transactional` | Retire with legacy update/page implementation in 041. Do not preserve a fake legacy protocol for the test. | Audit existing native feed/reset/version tests for these live guarantees; port only a missing native invariant |
| Repeated `Harness`, headless/store/demo setup and screenshot dump functions across app tests | Delete unused setup after pruning; share surviving minimal setup without a global mutable singleton or configurable testing framework. | Real rendering/fonts/storage where required, per-test isolation |

## What is not blanket-deleted

Coordinates are legitimate **inputs** for hit testing and pixel sampling. Keep
clip/rounded-boundary, occlusion, frontmost consumption and selection-copy
assertions; don't confuse them with a demand that a button forever sit at x=150.
A scenario's title or old passing status does not establish that its assertions
protect the intended guarantee: inspect the actual assertion.

Keep useful native cache/version/source fencing, authored-draft durability,
immutable operation/recovery, verified-byte/integrity/budget, wrong-account export,
Extract single-flight, stale IME/paste callback, keyboard/Unicode/editor and
real two-client/daemon coverage. They may be shortened/consolidated where
redundant, but do not delete distinct safety behavior just to hit a number.

The audit's six diagnostic probes are not six mandatory new permanent tests.
Fold the essential occlusion, shared feedback and scroll-selection guarantees
into the smaller surviving suite. Delete probe scaffolding from runtime worktrees.

## Commit/deletion discipline

- Start with independent cosmetic assertions. Delete changed layout-only tests
  rather than repeatedly fixing their expected coordinates. Remove now-unused
  setup, cfg(test) exports and module declarations in the same slice.
- For each changed test module, record **deleted / consolidated / kept** with one
  line naming the useful guarantee and where it now lives. This is a short change
  ledger, not a requirement to preserve each assertion or add a new test per row.
- Model-only scenarios do not need GPU setup; common control behavior need not be
  retested for every form. Retain a few actual App/GPU integrations to catch
  mismatches that isolated unit tests miss.
- Run the remaining affected nextest cases and relevant compiler checks. A failure
  must be classified: real regression, obsolete expectation, or fixture defect.
  Never delete a unique failing safety assertion merely to produce green output.

Working allocation: 2,000 net test code lines toward the overall target, measured
separately from production and including replacement tests/helpers. Current total
frontend test code is 8,551 raw / 11,434 same-format lines; app/UI test code occupies
4,744 physical lines. These inventories do not prove a particular deletion yield.
