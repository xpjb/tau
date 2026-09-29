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

## Deletion ledger so far

- **036:** deleted `icon_controls_tests.rs` and composer-centering assertions;
  kept actual Settings/Tail actions in compact occlusion/selection scenarios.
- **037 controls:** deleted `hover_tests.rs`'s separate GPU setup, section-key
  assertions and ripple adapter accessors. One actual nested attachment case
  verifies child-only pixels/capture/action and parent blank-space ownership.
- **037 controls:** collapsed `render_download_state_matrix`'s all-state × size ×
  hover/press/tooltip gallery into one narrow-card fit check; removed exact target
  height, one-draw and text-only implementation assertions. Existing download
  action/failure/source and Extract single-flight tests remain. Their required
  shared fixture is not yet retired.
- **037 controls:** existing clip/control case also checks a disabled foreground
  Form consumes without an action. Ripple timing still checks elapsed-time fade
  and idle behavior, no longer an obsolete string-key check.
- `test_ui.rs` remains, minus the obsolete ripple/section forwarders. It is **not
  deleted yet**; the rest of 042 and its 2,000-line test allocation are still open.

### 037 ownership retirement

Deleted the synthetic ancestor-path test and path-vector assertion. Strengthened
actual nested capture coverage to require cancellation before any frame. Added
one consumed-Tick/sibling-motion regression (both fixes reproduced against the
previous commit). Deleted the gallery-only caption/tooltip coordinate test and
Root's separate test-card ownership/dispatch tree, along with its App paint
adapter. Existing Save/retry/restart/source and Extract single-flight checks now
paint and dispatch through the actual attachment browser. Net tests: **45 raw /
77 same-format lines removed**; all 198 surviving frontend tests pass.

### 038 form retirement

Removed the old selector-registration assertion and a duplicate model/operation
replacement case. Removed the daemon completion test that supplied an arbitrary
result to a new form with no submitting token; it never exercised completion
ownership. The actual async Topic request/source checks and editor callback cases
remain. Extended the existing real-editor scene with compact offline feedback and
draft preservation instead of adding another form matrix. Net tests: **14 raw /
24 same-format removed**. The full surviving frontend suite is 196/196 green.

### Independent project/status pruning and synthetic Submit retirement

Deleted duplicate GPU project/chat activity matrices, fixed separator/row/gap
assertions, and the desktop copy of the long gesture/menu scenario. Kept actual
scroll-axis, swipe-vs-hold, clipped menu navigation, unread and selected-tab reveal
checks. Moved topic resume/restart/deleted-or-moved membership assertions to the
existing controller-only `tests/activity.rs`; no GPU is needed for persistence.
Collapsed composer status's eight-level/three-size glyph matrix to one narrow
isolated render: extents fit, and changing thinking alone visibly changes the row.
Existing thinking-level model tests retain semantic coverage.

Deleted the production `Event::Submit` variant and all five handlers: only tests
emitted it. The two useful callers now click the actual Refresh / Forget locally
controls. Deleted `FixtureChoice::Confirm`, its dispatch, and the unused agent
settings fixture and composer forwarding method. `test_ui.rs` still exists;
its remaining retirement and the direct transcript rewrite remain open.

Validation: all-target frontend compiler check, then **195/195 frontend tests
across 10 binaries, zero skipped** (`c5568c42-b06e-49e2-b482-eb19fa8b5115`).
Net reduction from `55c11f0`: production **5 / 5**, tests **105 / 171**, total
**110 / 176** raw / same-format. Cumulative reduction is **405 / 400**, not the
5,000-line acceptance gate. No physical device check is claimed.
