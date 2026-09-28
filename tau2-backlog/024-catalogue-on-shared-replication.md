# 024 — Reuse record replication for chat/topic catalogues

Kind: **Collection-level implementation candidate.**
Group: **State and replication / catalogue**; design context 019 / 022.

Status: **Open; protocol/migration design required, not started.**
Priority: **Later, after shared semantic contracts prove useful.**
Confidence: **5/10** in the value and appropriateness of this scope, not a probability of a LOC saving.
Net production LOC reduction estimate: **-150 to +100 lines**.

Recorded September 28, 2026. Evidence: the detailed `40a3698` investigation plus
relevant changes checked at `origin/tau2` = `cafef7f`. These are planning ranges,
not implemented or measured reductions. Positive means fewer lines; negative
means added code. Count changed production Rust across frontend, daemon and shared
crates; moving code is zero. Tests/docs/manifests are excluded unless explicitly
listed. See README for the confidence scale, baseline and overlap rules.

## Why / evidence

`daemon/src/listing.rs::list_page`, `frontend/src/controller.rs`
SessionPage/ProjectPage/SessionState handling and `Catalog`/`state_versions`, plus
`protocol/src/lib.rs` list/page messages.

Confidence rationale: Catalogue paging and status merging are an actual parallel
synchronization system. Reusing feeds might remove it, but runtime status clocks and
bounded catalogue traversal do not disappear automatically.

## Proposed scope

Evaluate publishing typed chat/topic summaries through the existing bounded
record-feed/reset mechanisms instead of separate list-page reconciliation. Model
structural membership and runtime status revisions explicitly. Keep large prompts and
histories out of headers and cold list queries. Plan matched protocol/schema changes if
replacing the live wire path.

Out of scope: No model-provider catalogue redesign, local-navigation redesign or loss of
per-record/epoch distinctions by replacing all clocks with one number.

## Acceptance

- Delayed pages never overwrite newer status, including status arriving before
  membership. Hot activity does not indefinitely restart traversal.
- Bound pages/metadata, never instantiate cold runtimes or read transcript/provider
  history just to list chats.
- Preserve selected/missing-local-work chats, deletions, restart epochs and current
  background-sync scheduling inputs. Remove replaced catalogue handlers only after the
  new path covers them.

## Honest impact estimate

About 150–250 lines of bespoke list/page/staging/stale-status handling are candidates,
offset by 150–300 lines of public record projection, migration and integration: -150 to
+100 net. The 45-line daemon listing file alone cannot finance a large generic
framework.

## Dependencies, overlap and current-source notes

Shares generic collection primitives with 025 and domain types with 022. Re-estimate
after those exist; do not charge or credit the common scaffolding twice. 019 is only the
client ownership wrapper.

Catalogue handlers remain at cafef7f; SessionState now also triggers background
synchronization. Preserve that connection rather than deleting it as obsolete status
plumbing.

No implementation, runtime/device certification or Rust test run is part of this
backlog entry. Work only with managed Cargo compiler checks and relevant nextest
coverage when implementation is authorized. Preserve existing deferred tasks.
