# Stop/Play lockout — bounded source fix, October 1, 2026

User requested fixing the stuck run-through → Stop → Play flow, then recording
that the UX makes no sense, suggests technical debt, and Play/pause has already
been reviewed repeatedly, followed by commit/push/merge. Implementation
**`6422e27`** on `fix/tau2-control-recovery` retires controls on Stop, makes Play
supersede stale intent, preserves queue data and applies last-intent precedence
through cancelled-task cleanup. Genuine failure/restore pauses remain.

Both lockout reproductions fail before the patch; **66/66 daemon nextest cases**
pass after it across disjoint selections, including WAL/disk-full/recovery.
The retained frontend control regression passes, including phone-sized visible
Cancel/Play. See [exact evidence](docs/tau2-control-recovery.md).
[045](tau2-backlog/045-play-pause-control-ux-debt.md) is **open**, not fixed by this
patch: run-through still waits for the existing boundary; systemic UX and control
ownership need review. Source merge is authorized; release/deployment, service
restarts, live data changes and physical-device acceptance are not part of this
change. Final integration/check evidence will follow the source merge.

---

# Stop/Play queue-control recovery — October 1, 2026

The immediate lockout fix `6422e27` makes Stop retire deferred controls and Play
supersede stale ones; genuine error/restore pauses remain. Menu/header/cancel
copy now names pending run-through and its pause/limit. The retained-control
regression covers desktop state and visible named cancellation/Play at phone size
(`55f8fb11-4fbb-4bdb-a970-021f6b75a95e`). See
[reproduction and validation](../docs/tau2-control-recovery.md).
[045 UX/control-state debt](../tau2-backlog/045-play-pause-control-ux-debt.md)
remains open: this is not faster mid-stream steering or real-device UX acceptance.
No package or deployment was performed.

---

# Tau 2 backlog

## New high-priority control/UX report — October 1, 2026

[045 Play/Pause/Stop and run-through UX/debt](045-play-pause-control-ux-debt.md)
records the user's confusing/undiscoverable controls, insufficient interruption,
bricked conversation, historical Stop→drain expectation and repeated Play/pause
fixes. The immediate [Stop/Play lockout patch](../docs/tau2-control-recovery.md)
is source-complete; **045 remains open** for product semantics, transition
ownership and interactive acceptance. Do not close it with another race patch.

## Current disposition: bounded source closeout

Triaged **September 29, 2026**, against freshly fetched `origin/tau2` at
`33f7d6f`. This supersedes the earlier implementation ordering, not the recorded
bug evidence. Implementation is on `feat/tau2-simplification`; **036 is source-complete**.
037's control/composition/lifetime and explicit update/cancellation paths are in;
synthetic lifecycle events are gone. 038's form reuse is implemented. 039–040 use
native body/tool identity and reconcile authored messages outside painting. 041
has deleted the rich projection/Row/Part pipeline and the legacy transcript fixture
protocol. The redundant attachment-card registry and addressed button routes are
also gone. 042 has retired the global test action adapter, test-only Submit and
old scroll scenario, alongside independent matrix pruning.
Transcript reading-position ownership is now consolidated; external layout resets
and the expansion registry are gone. The user has now authorized local/justified
deletions, backlogging the remaining work and merging both this source and the
independent compaction/pause fix into `tau2-integration`. Those source merges are
complete (`73a617f`, `1cb2282`); [merged acceptance](../docs/tau2-simplification-closeout.md)
passes 328/328 workspace tests and compiler/platform/rustdoc checks. The 5,000-line target is
**not achieved**; physical acceptance is **not complete**. These are explicit
open/deferred items, not reasons to start another speculative rewrite before the
authorized source merge. See [044 closeout/follow-ups](044-closeout-and-followups.md).

### Deletion-first execution

The user's follow-up prioritizes removing **entire redundant subsystems**, not
agonizing over individual lines. Choose each slice by naming the competing owner,
adapter or algorithm that will disappear, its actual replacement owner and all
remaining callers. Delete the obsolete path end-to-end in that slice; don't leave
a facade, dormant branch or test-only reconstruction behind. Preserve product
features and safety contracts. Helpers, formatting and test-setup consolidation
are not standalone simplification targets merely because they can shave lines.
Keep measuring net cost, but don't let the counter drive local code golfing.

The selected ownership, input/paint and transcript boundaries are now implemented
and their named predecessors removed. A bounded final pass deleted three unused
hit-test entry points; no further large safe local deletion was demonstrated.
Do not add another UI or replication framework to chase the remaining count.

### Ordered, bounded slices

| Item | Deliverable / deletion | Dependency |
| --- | --- | --- |
| [036 Correct drawing and selection](036-drawing-and-selection-correctness.md) | Ordered compositing; selection follows scrolled text | Source-complete; device QA open |
| [037 One control and child-traversal contract](037-retained-interaction-ownership.md) | Delete paint/register bypasses and root-maintained descendant routing | Source-complete; device QA open |
| [038 Finish forms and chrome](038-forms-and-chrome.md) | Delete repeated field/footer/layout and migrated control plumbing | Source-complete; device QA open |
| [039 Native tool records, used directly](039-native-tool-records.md) | Delete native tool re-pairing and display-key parsing | Source-complete through 041; device QA open |
| [040 One message through local/queue/history state](040-message-state-reconciliation.md) | Move reconciliation out of paint; delete competing precedence rules | Source-complete through 041; 013 remains separate |
| [041 Transcript owns its children](041-direct-retained-transcript.md) | Projection/wrapper and competing reading-policy paths deleted; 012 source review closed | Source ownership complete; size/device gates open |
| [042 Remove low-value tests and compatibility scaffolding](042-test-retirement.md) | Delete brittle assertions, obsolete tests and `test_ui.rs`; smaller behavioral suite | Targeted retirements complete; optional further pruning in 044 |
| [043 Completion audit](043-simplification-acceptance.md) | Measured reduction, actual behavioral/device acceptance, no legacy path left | Source closeout authorized with unmet size/device goals recorded |

The numbered slice descriptions and intermediate check results remain historical
evidence. Their stale “next” work is superseded by the implementation follow-ups
and the current disposition in 044. No dormant dual implementation is accepted.

### Size and test policy

- The original, **unmet and now deferred**, reduction target is **at least 5,000 fewer frontend Rust code lines** than
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
The implemented work concerned shared **meaning and reconciliation**, not a missing
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
| [012 Scroll ownership](012-chat-scroll-position-audit.md) | Source ownership complete in 041; old scroll tests deleted; physical acceptance open |
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
