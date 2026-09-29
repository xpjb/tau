# 040 — One message through local, queued and confirmed state

Status: **Selected, bounded model slice.** Replaces old 017 and the message portion
of 018/022. Reuse 039's explicit body-reference contract. No operation-registry,
storage-schema, catalogue or generic replication migration.

## Change

Reconcile confirmed observations with existing durable local intent when model
updates arrive, not inside painting. Use source/chat/request identity for one
logical authored message; ordered collections hold IDs, not independent rich
copies. Put this in the existing chat/feed owner or its direct replacement, not
another owner layered over unchanged Feed plus new message/row models.

1. Centralize body/status/placement choice currently spread across
   `app/projection.rs`, `Feed::native_view` queue transitions and local pending
   lookups. A receipt, queue header, history header and verified body are different
   evidence. Preserve queue-removal/root-cursor barriers.
2. Switch pending/queued/confirmed message consumers, including UI action targets,
   to that state. Remove their competing precedence and duplicate-row suppression
   branches as they switch. 041 must consume identities/body state, not Vec<Row>.
3. Represent missing/partial/preview-limited/growing/complete body state explicitly
   for these consumers. Stop injecting/reversing `Loading…` as source text; a real
   message with that text stays literal authored content. Do not duplicate bodies.

The existing durable outbox remains the owner of authored input and immutable
request payloads. Rendering eligibility is not outbox-retirement evidence; retain
necessary delivery/content checks. State installation/replay has **no executor
side effects**. Pure value transformations may be shared with authority handling;
a tentative client edit is not permission or an authoritative revision.

## Acceptance / retained coverage

Use the real model/recovery coverage for delayed/lost acknowledgements, restart,
queue/history overlap and reversed arrival, pending queue edits/rejections, aliases,
source changes and header-only content. Assert one logical message, original text
until safe handoff, stable action/anchor identity and original operation IDs.
Preserve expected revisions and authenticated local-byte reuse checks. Restart or
replay must never duplicate paid requests, tools or filesystem effects.

Move the useful logical assertions out of the GPU-heavy
`own_message_keeps_one_display_identity_through_receipt_queue_and_header_only_history`
test into this model boundary; delete its old Row assertions and redundant fixture.
Keep one actual transcript handoff smoke check in 041, not the entire matrix twice.

## Explicit exclusions / stop condition

013 remains the separate **daemon content-reference lifetime** problem. This
client reconciliation does not make a deleted queued body readable. Schedule its
own deterministic reproduction/proportionate fix without blocking UI work.

020's small recovery-classification duplication may be removed if this code is
touched; do not change retry policy or make a registry migration a prerequisite.
021/023–027 stay deferred/shelved as recorded in the triage map.

Done means one model rule is used by real consumers and the superseded branches
are gone. A new facade that delegates to all the old reconciliations fails the
slice. Report net cross-workspace production/test cost, including any adapters.

## Model ownership installed — September 29, 2026

Feed now owns an ordered identity index referring to verified events, queue items
and durable intent, with one explicit body owner. It contains IDs, not cloned text,
menus, titles or a second widget tree. Controller mutation/persistence and native
update boundaries reconcile that index. Painting no longer chooses local/queue/
history precedence or suppresses duplicates. The existing outbox still retires only
against content/receipt evidence; root-cursor queue-removal barriers are unchanged.
Source reset/alias/local restoration paths rebuild the same index without execution.

The transitional projection consumes those identities; 041 must still remove its
presentation objects. The old own-message GPU/Row matrix is deleted. One real native
cache case checks accepted intent, queue headers, overlapping canonical headers,
complete verified content and stable original request/display identity. Existing
recovery, pending queue control and real two-client tests remain. A retained identity
fixture that previously violated native immutable ordering now uses a valid prepend.

Fresh frontend all-target check and **194/194 frontend tests**, zero skipped, pass
(`f8e5d0c6-58c5-4f2f-86ad-5b8481a7ee78`). The slice removes 101 raw but adds 78
normalized frontend lines: production −107 raw / +88 normalized; tests +6 / −10.
No added shared code in this slice. This is model ownership, not a claimed large
simplification; direct consumers and their competing description path are next.
