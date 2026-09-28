# Tau 2 backlog

User reports and QA findings since September 23, 2026. Keep one issue per file.
Record facts, acceptance checks, related work and remaining device checks; do not
mark an item complete just because code compiles.

| Item | Priority | Status |
| --- | --- | --- |
| 001 Daemon settings and hierarchical prompts | First | Beta daemon/Windows delivered |
| 002 Direct model selection without a catalog gate | With 001 | Beta daemon/Windows delivered |
| 003 Text input keyboard navigation | Editor worktree | Merged via frontend into tau2; device QA pending |
| 004 Sanscale API migration | Editor worktree | Pinned/migrated; ligature limitation recorded |
| 005 Highlight colour | Visual | Open |
| 006 Caret response | Editor worktree | Hot path fixed/measured; physical latency pending |
| 007 Text input scrolling | Editor worktree | Merged via frontend into tau2; device QA pending |
| 008 Click-origin ripple | Visual | Open |
| 009 Nested tool hover feedback | Visual | Open |
| 010 Account usage remaining | Context hover | Open |
| 011 Title generation model and prompt | With 001 | Beta daemon/Windows delivered |
| 012 Chat scroll-position architecture and test removal | Later | Open; quick fix provisional, audit deferred |
| [013 Queue-to-history content-reference lifetime](013-queued-message-content-lifetime.md) | Later | Open; deferred, proportionate fix to be scoped |
| [014 Native data recovery after an outage](014-native-data-recovery-after-outage.md) | Later | Open; deferred, cause of per-file progress gap unresolved |

## Client/state/replication investigation — September 28, 2026

The records below form related initiatives, not 21 independent projects.
**015 is the recommended first implementation.** 016 is its transcript parent;
019, 022 and 031 are design context, not separately schedulable refactors.
Other entries are implementation candidates, supporting cleanup or explicitly
conditional alternatives. Recording them is not implementation authorization.
Existing IDs are retained so references remain valid.

### How to read the estimates

- **Confidence /10** is confidence that the proposed scope is a worthwhile and
  appropriate improvement, conditional on its acceptance checks. 8–9: concrete
  repeated code/contract evidence; 6–7: plausible boundary with integration risk;
  4–5: architectural hypothesis needing a pilot or a real consumer. It is not
  confidence that the best-case LOC number will be achieved.
- **Net reduction = old production Rust removed minus new production Rust added**
  across frontend, daemon and shared crates, including retained compatibility and
  migration code. These are rough physical-line planning ranges at the existing
  source style, not a measured patch or a statistical confidence interval.
  **Positive = fewer lines; negative = growth.** No credit for directory moves,
  minification, deleting safety checks, or moving application code into a shared
  crate. Tests/docs/manifests/dependencies are not production LOC.
- Test-only savings are shown separately for **034**. Most runtime refactors will
  add tests; no reduction in total repository size is promised.
- **Do not sum these estimates.** 016 is an inclusive umbrella for 015/017/018; 019/031
  are ownership-only organization; 020 can be absorbed into 021; 022–026 share
  primitives; 028/029/032 overlap UI coordination. Every entry states its overlap.
  Re-estimate remaining work after earlier changes land.
- Confidence in the usefulness of a boundary can be high even when its LOC range
  crosses zero. In particular, **015 is first for correctness/semantic ownership,
  not because it promises the largest line-count reduction**. 035 is a clearer
  pure production-code deletion candidate; 034 only reduces test scaffolding.

### Parent/design records — context, not extra projects

These retain the architectural reasoning and original estimate scenarios. Do not
schedule them in addition to their implementation slices or add their estimates
to child estimates. The 022 rename/queue-edit pilot is a future scoping example,
not another approved task.

| Record | Role | Confidence | Reference LOC scenario, not an extra budget |
| --- | --- | --- | --- |
| [016 Full transcript/section model](016-typed-transcript-and-section-model.md) | Transcript parent: 015, 017, 018; related cleanup 035 | 7/10 | -100 to +300 |
| [019 Cohesive ClientState ownership](019-cohesive-client-state-ownership.md) | Client-state design context for state/replication work | 6/10 | -150 to 0 |
| [022 Shared model / safe prediction](022-shared-public-model-and-safe-prediction.md) | Shared-state/prediction design context for state/replication work | 5/10 | -250 to +100 |
| [031 Feature-owned App state](031-feature-owned-app-state.md) | UI feature-ownership design context for 028–033 | 6/10 | -120 to 0 |

### Initiative A — state and replication

Design context: **019 / 022**. Grouping here expresses shared responsibilities,
not a requirement to implement every entry or a blanket prerequisite chain.

#### Transcript — parent 016

**Start with 015.** 017 and 018 are later slices of the same model. 035 is related
legacy cleanup, not a prerequisite to 015 and not included in 016's LOC scenario.

| Task | Confidence | Estimated net LOC reduction |
| --- | --- | --- |
| [015 Shared tool projection](015-shared-tool-transcript-projection.md) | 8/10 | -50 to +150 |
| [017 Logical message lifecycle](017-logical-message-lifecycle.md) | 7/10 | -50 to +100 |
| [018 Typed content availability](018-typed-content-availability.md) | 7/10 | -80 to +20 |
| [035 Legacy transcript adapters](035-retire-legacy-transcript-fixture-adapters.md) | 8/10 | +80 to +180 |

#### Operation lifecycle

020 is a small extraction that 021 can absorb. 021 is a real local-store migration,
not merely a parent heading. 025 concerns authoritative outcome observation; it
is related work, not automatically part of that migration.

| Task | Confidence | Estimated net LOC reduction |
| --- | --- | --- |
| [020 Shared recovery policy](020-shared-delivery-recovery-policy.md) | 9/10 | -10 to +25 |
| [021 Durable operation registry](021-unified-durable-operation-registry.md) | 6/10 | -200 to +100 |
| [025 Common outcome observation](025-common-operation-outcome-observation.md) | 5/10 | -100 to +80 |

#### Catalogue synchronization

A separate collection-level candidate within the same shared-state direction.

| Task | Confidence | Estimated net LOC reduction |
| --- | --- | --- |
| [024 Common catalogue replication](024-catalogue-on-shared-replication.md) | 5/10 | -150 to +100 |

#### Supporting synchronization infrastructure

026 is the concrete receiver extraction. 027 is optional packaging, not another
algorithmic win. 023 remains conditional on a concrete additional host/use case;
there is no recommendation to build a general replica engine now.

| Task | Confidence | Estimated net LOC reduction |
| --- | --- | --- |
| [026 Shared native receiver](026-shared-native-receiver.md) | 8/10 | -50 to +50 |
| [027 Merge blocks/transfer packaging](027-unify-blocks-transfer-library.md) | 6/10 | 0 |
| [023 Publisher / receiver roles](023-direction-independent-replica-roles.md) | 4/10 | -400 to 0 |

### Initiative B — UI ownership and interaction

Design context: **031**. 028 is the interaction/capture work. The other tasks are
related helpers or feature-level slices, not mandatory stages of a framework
rewrite. Their overlapping registration/lifecycle code must be counted once.

| Task | Confidence | Estimated net LOC reduction |
| --- | --- | --- |
| [028 Interaction scene / capture](028-interaction-scene-and-gesture-capture.md) | 7/10 | -150 to +250 |
| [029 UI context / layout helpers](029-small-ui-layout-context.md) | 6/10 | -50 to +100 |
| [030 Scroll-axis mechanics](030-shared-scroll-axis-mechanics.md) | 8/10 | 0 to +35 |
| [032 Typed dialog lifecycle](032-typed-dialog-lifecycle.md) | 7/10 | -50 to +100 |
| [033 Download acquisition / export job](033-shared-download-acquisition-and-export-job.md) | 8/10 | -25 to +50 |

### Supporting test tooling

034 is independent test scaffolding cleanup, with no production-code saving.

| Task | Confidence | Estimated net LOC reduction |
| --- | --- | --- |
| [034 Headless test fixtures](034-shared-headless-ui-test-fixtures.md) | 9/10 | 0 production; +100 to +250 tests |

### Evidence and topic coverage

Detailed audit baseline: `40a3698`. Before writing these entries, fetched Tau 2
and reviewed relevant changes through `cafef7f`. New navigation/DownloadTarget
ownership and transcript warm-cache/background-prefetch work are preserved and
not counted as future savings. Old review line numbers refer to the audit
baseline; backlog entries primarily name symbols/files. No Rust build, tests or
application changes were made for this backlog-only pass.

- Original UI context, scrolling, feature owners, acquisition, fixtures and
  legacy adapters: **029–035**.
- Deeper transcript/state investigation: **015–021**, including typed content,
  message continuity and the separate small recovery-policy extraction.
- Retained interaction/gesture ownership and typed dialog lifecycle: **028/032**.
- Shared client/server state, Quake-inspired safe prediction and reusable
  publisher/receiver roles: **022/023**.
- Common catalogue/outcome observation, native receiving and library packaging:
  **024–027**.

Existing **012** remains the separate explicitly deferred scroll-restoration
architecture/test-removal issue; 030 is mechanics only. Existing **013** remains
server content-reference lifetime; 017 does not solve it just by introducing a
client identity. Existing **014** remains the unresolved outage-progress finding;
026 is not a proven recovery fix. Those issues are not duplicated or closed here.
The completed LOC inventory and already-shared Editor/renderer/attachment code
are evidence, not invented new implementation tickets.

Source proposals: `docs/tau2-client-structure-review.md`,
`docs/tau2-client-state-proposal.md`, `docs/tau2-shared-replication-proposal.md`.
The first-pass smaller ideas are also preserved in commit `23bef63` of the review
branch. The new entries include them without restoring their original ranking.

Settings and the composer must reuse the existing shared editor. Do not create
another text controller. See 004 for the pinned source and migration difficulties.
Work stays on Tau 2. Do not replace stable Tau or restart a daemon hosting an active
conversation to deploy its own update. New settings fields require matched clients.

## Scope correction

Project overrides were not requested. Do not treat their existing implementation
as an accepted requirement. Item 001 has exactly two levels:
**model override → default system prompt**. There is no provider, project or
built-in fallback layer. Empty default/override text is intentional.

## Settings handoff — implementation `15d2a69`

User requested a stop after important settings work and assigned the editor to
another worktree. No editor/Sanscale code was changed here.

- Settings schema 2, protocol 13, client/daemon version 0.7.1 (unshipped).
- Exactly model override → saved default prompt. Default and model overrides can
  both be empty. No project/provider/built-in prompt fallback and no append layer.
- Settings → Daemon settings → Prompts edits the default and any exact model ID.
  A missing `agent.modelSystemPrompts` key inherits; a string, including `""`, overrides.
- Optional `daemon.titleModel`; unset follows the chat model. Titles remain native.
- Model IDs no longer require membership in metadata. Provider errors reach the
  chat. Unknown capacity stays unknown; no threshold-based auto-compaction then.
- Workspace check and all **51 nextest tests passed**. Scoped daemon/frontend/
  protocol Clippy with warnings denied passed. Native debug client/daemon build
  passed. The full workspace Clippy gate remains blocked by existing untouched
  Markdown lints; flag `896a973b-1da6-4680-b894-c147eeb7d6fc` records this.
- Real settings-controller/daemon/local-provider tests cover empty/default/custom
  prompts, both provider formats, model switches, compaction, restart, settings
  conflicts, direct IDs, provider rejections, title models and draft preservation.
- No new GUI/device acceptance or release packages. Do not claim those passed.
- No production settings, daemon restart, history import or paid model request.
  The existing settings reader preserves old default text without rewriting files
  on load. The live Astra override still needs `""` set at the matched beta update.

The code commit is on `tau2/daemon-settings-prompts`, based on `tau2-integration`.
Stable `master` and other worktrees are unchanged. Keep the paired daemon/client
protocol change together when integrating with editor work. Deploy only after an
idle, coordinated beta update; do not interrupt the active conversation.


## Beta 0.7.1 delivery

The integrated settings and shared-editor code was deployed to beta and delivered
as the matched Windows x64 installer on 2026-09-24. Android was not rebuilt for
this request. Device acceptance, physical latency and documented SDK limitations
remain pending; deployment does not close those checks. See ../INTEGRATION.md.
