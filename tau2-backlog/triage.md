# Earlier backlog disposition — September 29, 2026

**Closeout note:** the dated planning map below is historical. 036–042's selected
source work is implemented; [044](044-closeout-and-followups.md) is the current
remaining-work disposition. Earlier speculative rewrites are not reactivated by
the unachieved size target.

This is a disposition map, **not another task queue**. Active work is 036–043 in
[README](README.md). Items 001–014 remain in their original files. Items 015–035
were found only on `origin/review/tau2-client-structure` at `24b8323`; none were
silently implemented by creating those documents. All remote backlog paths were
inventoried. Their original evidence remains available in Git rather than copying
another 2,490 lines of superseded proposals into the active backlog.

## Earlier state/UI review

| Old ID / topic | Decision now | Owner / reactivation condition |
| --- | --- | --- |
| 015 Shared tool transcript interpretation | **Replace the proposed intermediate representation.** Preserve explicit native identity/children/body contracts. | 039; direct consumers, no new rich UI description layer |
| 016 Full transcript/section model | **Superseded as a parent architecture.** No second editable transcript or mandatory section-object graph. | 039–041; necessary model semantics and UI ownership are separate |
| 017 Logical message lifecycle | **Selected, bounded.** One source/chat/request identity and reconciliation owner. | 040; operation persistence format stays intact |
| 018 Content availability | **Absorb.** Explicit body/version/availability at the consumers that need it. | 039–040; no new blob store or standalone facade project |
| 019 Cohesive ClientState | **Not a task.** Use actual owners; moving fields into a named mega-struct is not simplification. | Revisit only for a demonstrated competing owner not fixed by 040 |
| 020 Recovery classification | **Small follow-up, not a prerequisite.** Preserve different replay policies. | 040 may remove duplicated classification if touched; otherwise leave it, do not add an extraction project |
| 021 Durable operation registry | **Deferred.** Real local-store migration, uncertain shrinkage. | Separate approval after a concrete duplicated lifecycle remains; preserve IDs/frozen payloads/migration recovery |
| 022 Shared public state / safe prediction | **Direction adopted narrowly**, not a whole-world rewrite. | 039–040 share record meaning and reconcile local intents; rename/queue-edit prediction pilot deferred until needed |
| 023 Direction-independent replica roles | **Shelved.** No additional host/authority requirement. | Only revive with a real second consumer; no general peer/CRDT engine |
| 024 Catalogue on shared replication | **Deferred.** Separate matched protocol/schema change. | Revisit if catalogue correctness/duplication justifies replacing its paging/status algorithm |
| 025 Common outcome observation | **Deferred.** Receipts, effects and installed state are not interchangeable. | One concrete operation kind with duplicate outcome handling must justify a bounded pilot |
| 026 Shared native receiver | **Deferred, not an outage fix.** Moving the receive loop is no saving. | A receive-path change must demonstrate net removal of repeated verify/commit logic |
| 027 Merge blocks/transfer crates | **Dropped from simplification plan.** Packaging alone removes no algorithm. | Only with a separately justified dependency/API change |
| 028 Interaction scene / capture | **Partly landed, correctness unfinished.** Do not build another scene alongside retained children. | 037; delete root shadow traversal and unify visual/input ownership |
| 029 UI/layout context | **Superseded extraction proposal.** Context/Frame already exist; finish modest reuse. | 038; no second context or form framework |
| 030 Scroll-axis mechanics | **Landed in substance as ScrollState.** No second extraction. | 037/041 trim only actual duplication; preserve lane-specific policy |
| 031 Feature-owned App state | **Ownership landed; simplification not complete.** No blanket App rewrite. | 037–038/041; direct owners replace residual registration and wrapper layers |
| 032 Typed dialog lifecycle | **Landed in substance.** Keep instance/source/request fences; reduce remaining plumbing. | 038; no second dialog state machine |
| 033 Download acquisition/export intent | **Partly addressed by Services/Transfers and retained cards.** Not an independent savings project. | 038 only if duplicated setup remains; preserve preview/save/extract distinctions and single-flight work |
| 034 Shared test fixtures | **Replace policy.** Delete low-value tests first, share only surviving setup. | 042; previous “keep all assertions / do not count deleted tests” restriction is superseded |
| 035 Non-wire transcript fixture adapters | **Selected cleanup, coupled to real consumers.** | 041–042; delete old update/page algorithms and skipped-wire variants once callers are retired; native startup/sparse updates stay |

Old confidence scores and signed LOC ranges are **withdrawn as planning budgets**.
They overlap, predate retained ownership, and mostly predict small or uncertain
savings. A model rewrite cannot be sold as a proven source of thousands of lines.

## Boundaries between the selected work and real sync bugs

- **012:** the old session-specific deferment is lifted for 041's reading-position
  ownership review and requested test deletion. 036 fixes stationary-pointer
  selection immediately; it does not wait for restoration redesign.
- **013:** a client-side stable message ID does not keep a deleted daemon body
  readable. Keep this separate: reproduce queued metadata → promotion → delayed
  old body request; compare bounded content lifetime/identity with authoritative
  obsolete-interest reconciliation, then select the smaller correct fix. No hiding
  a missing body that remains advertised. This does not gate drawing/UI completion.
- **014:** the failed per-file recovery-progress observation is still unexplained.
  Instrument the actual failing wait before choosing a fix. A shared receiver or
  longer timeout is not evidence of recovery. Separate reliability work, not
  deletion budget and not silently marked fixed by shared-state semantics.

## What the shared-state idea buys here

Current code already has durable feed cursors, `BlockRecord::Put/Remove`, reset
pages, source lineage and verified body coverage. Yet `blocks::View` flattens
native parent relationships into Events; Feed holds those plus side maps;
`app/projection.rs` rebuilds rich rows; retained rows rebuild parts. Copy/fetch
interpret relationships again. That is the redundant path to remove.

039 shares the public metadata interpretation used by daemon publication and
client consumers, retaining the current wire representation. 040 reconciles
confirmed observations and pending authored work once outside painting. 041
owns the actual text/disclosure/attachment widgets. Small ordered ID/height
indexes for a bounded viewport are useful; another rich description tree is not.

A receiver may know metadata without bodies, or queue removal before history
arrival. Keep that evidence and bounded working set. Shared state means shared
contracts and safe reconciliation, not identical heaps or replayable effects.

## Evidence and superseded records

- Fresh integration/source baseline: `33f7d6f6b45533e081e5021b0c3f69b1e045b6f9`.
- Earlier review, proposals and all old 015–035 records: `24b8323` (source audit
  began at `40a3698`; follow-up at `cafef7f`). For example:
  `git show 24b8323:docs/tau2-shared-replication-proposal.md`.
- [Retained rewrite audit and reproduced bugs](../docs/reviews/retained-ui/README.md).
  Its six diagnostic probes are evidence, not a mandate to add six permanent tests.
- `docs/tau2-retained-ui-{proposal,implementation}.md` remain historical ownership
  and integration records. Their old preserve-adapters/test-deferral guidance does
  not override this completion plan.

This pass changes planning/documentation only. It does not rerun the historical
Rust suites, implement a state model, delete runtime tests, or claim device QA.

Validation: all changed Markdown local links and the 001–043 disposition/index
were checked; the published whole-frontend size recipe reproduced all four counts
exactly; the retained-review regression patch still passes `git apply --check`.
`git diff --check` passed. Frontend, daemon, shared crates, dependencies and scripts
are unchanged from `33f7d6f`. No Rust build/test run or device check was needed for
this documentation-only change.
