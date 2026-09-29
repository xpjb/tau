# Tau 2 backlog

## Current priority: finish the simplification

Triaged **September 29, 2026**, against freshly fetched `origin/tau2` at
`33f7d6f`. This supersedes the earlier implementation ordering, not the recorded
bug evidence. Implementation is on `feat/tau2-simplification`; **036 is source-complete**.
037's control/composition/lifetime and 038's form reuse are implemented.
039 is in progress: viewport-key decoding and provider-ID rewriting/pairing are gone;
explicit body state and direct retained tool children remain.
042 has retired test-only Submit, the global test action adapter and the old scroll
scenario, alongside independent matrix pruning. No replacement test registry was added.
Remaining slices and the 5,000-line gate are not complete.

The retained rewrite established ownership but did not finish shared input/paint
semantics or simplify the transcript. Finish those boundaries and remove their
predecessors; do not add another UI or replication framework.

### Ordered, bounded slices

| Item | Deliverable / deletion | Dependency |
| --- | --- | --- |
| [036 Correct drawing and selection](036-drawing-and-selection-correctness.md) | Ordered compositing; selection follows scrolled text | Source-complete; device QA open |
| [037 One control and child-traversal contract](037-retained-interaction-ownership.md) | Delete paint/register bypasses and root-maintained descendant routing | Source-complete; device QA open |
| [038 Finish forms and chrome](038-forms-and-chrome.md) | Delete repeated field/footer/layout and migrated control plumbing | Source-complete; device QA open |
| [039 Native tool records, used directly](039-native-tool-records.md) | Delete native tool re-pairing and display-key parsing | Tool-widget consumer uses 037 |
| [040 One message through local/queue/history state](040-message-state-reconciliation.md) | Move reconciliation out of paint; delete competing precedence rules | Reuse 039 body-reference contract |
| [041 Transcript owns its children](041-direct-retained-transcript.md) | Delete `app/projection.rs`, rich `Row` descriptions and wrapper reconciliation; revisit 012 | 037, 039, 040 |
| [042 Remove low-value tests and compatibility scaffolding](042-test-retirement.md) | Delete brittle assertions, obsolete tests and `test_ui.rs`; smaller behavioral suite | Start now; finish with each owning slice |
| [043 Completion audit](043-simplification-acceptance.md) | Measured reduction, actual behavioral/device acceptance, no legacy path left | All above |

036 is not held hostage by the model work. 038 and the model slices can proceed
independently after the shared control contract. 042 starts by removing independent
layout/glyph assertions; delete owner-specific adapters when their last useful
consumer migrates. No prolonged dual implementation earns a completed slice.

### Size and test policy

- Completion requires **at least 5,000 fewer frontend Rust code lines** than
  `33f7d6f`, including tests: **27,725 → at most 22,725**. Also require a 5,000-line
  reduction under identical formatting, charging growth elsewhere in the workspace.
  [Baseline and measurement](../docs/reviews/retained-ui/completion-size.md).
- Working allocation: **3,000 production + 2,000 test code lines removed net**.
  These are targets, not a claim that the current inventory proves those savings.
  Production must genuinely shrink; deleting only tests is not completion.
  Every replacement and new regression is charged. No minification, code relocation,
  language switching, feature removal or generated-code hiding to satisfy the count.
- Delete tests that merely freeze incidental coordinates, spacing, glyph pixels,
  adapter shapes or retired implementation details. Do **not** keep updating them
  after a redesign. Do not replace each deleted test with another test.
- Keep a small set of tests for observable guarantees: correct occlusion/input
  ownership, selection/copy, authored-data preservation, source/lifetime fences,
  no duplicate effects and bounded native content. A coordinate used to exercise
  a hit/clip boundary is not automatically a brittle layout requirement.
- Each slice reports production/test deltas, deleted paths and remaining gap. If a
  pilot adds a wrapper rather than removing a path, stop and revise it before broad
  migration. A green suite or a new owner struct is not architecture acceptance.

### State/sync direction

The remembered shared-state proposal exists on `review/tau2-client-structure` at
`24b8323`; it was **not implemented**. Existing `tau-blocks` / `tau-transfer` already
share revisioned records, delta/reset handling and verified content transfer.
The unfinished part here is shared **meaning and reconciliation**, not a missing
transport engine. 039–040 use confirmed records plus durable local intents, then
feed retained children directly. State replay must never execute tools or paid
requests. Do not serialize the whole daemon, add a fixed tick, or introduce a
universal diff/replica layer.

[Complete disposition of earlier items 015–035](triage.md) includes the deferred
operation-store/catalogue/receiver proposals. Their speculative, overlapping LOC
estimates are retired; they do not finance the 5,000-line target.

## Existing reports

| Item | Current disposition |
| --- | --- |
| [001 Prompts](001-daemon-settings-prompts.md), [002 Direct models](002-direct-model-selection.md), [011 Titles](011-title-generation-settings.md) | Delivered in beta 0.7.1 daemon/Windows; historical requirements, not new implementation |
| [003 Navigation](003-text-input.md), [004 Sanscale](004-sanscale-migration.md), [006 Caret](006-caret-response.md), [007 Editor scroll](007-editor-scrolling.md) | Implemented; outstanding device/upstream checks retained, relevant device acceptance in 043 |
| [005 Highlight](005-highlight-colour.md), [008 Ripple](008-click-ripple.md), [009 Nested hover](009-tool-hover.md) | Open visual acceptance, absorbed into 037; some mechanics exist, not blanket closure |
| [010 Account usage](010-usage-remaining.md) | Shipped beta 0.7.6; live provider/device acceptance remains separate |
| [012 Scroll ownership](012-chat-scroll-position-audit.md) | Reopened for bounded 041; delete the explicitly unwanted scroll tests |
| [013 Queue content lifetime](013-queued-message-content-lifetime.md) | Independent model-side follow-up; deterministic reproduction and proportionate fix, not solved by 040 |
| [014 Outage recovery](014-native-data-recovery-after-outage.md) | Deferred diagnosis; preserve failing evidence, no guessed timeout/receiver rewrite |

## Working rules

Use a branch/worktree, small commits and pushes. For Rust use managed
`/usr/local/bin/cargo`, compiler checks and nextest; **no Clippy and no built-in
Cargo test runner**. Documentation-only triage needs no Rust build.

Reuse the existing Editor, Markdown/Sanscale renderer and verified content cache.
Constructor helpers must establish an invariant or perform real setup; do not wrap
plain struct literals or introduce types/modules merely to name an intermediate
step. Small feature-local choices and owner-applied structural requests are fine;
a second global action bus, widget registry or editable state copy is not.

Work stays on Tau2. No stable-Tau changes, live data migration, service restart,
packaging or deployment is authorized by this backlog. Physical acceptance and
release authorization remain explicit. Settings still have exactly **model override
→ default prompt**, with intentional empty strings and no provider/project fallback.
Historical delivery records are in `INTEGRATION.md` and Git history; the old settings
handoff is not a current release gate.
