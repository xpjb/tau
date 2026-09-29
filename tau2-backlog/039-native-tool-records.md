# 039 — Share native tool meaning and use it directly

Status: **In progress: native parent membership replaces provider-ID rewriting; direct retained tool/body-reference migration remains.** Replaces old 015 and the tool part
of 016/018/022. Integrate its actual tool-widget consumer with 037; no standalone
intermediate-description module. The original state proposal remains historical.

## Scope and owner

Native verified records remain the client authority for remote facts. Shared
protocol/model types or accessors define public tool identity, parent/input/result
relationships, execution state and body references once. Daemon publication and
client display/copy/interests use that contract with **unchanged wire encoding**.
Use existing crates; no replica framework, new database or crate merger.

Bodies carry source/scope/ID/version and explicit availability/finality, separate
from a bounded preview. Unknown children are not a completed empty child list.
Actual metadata decoding/validation is useful work; rebuilding a rich widget
specification from it every redraw is not.

## Bounded implementation

1. Carry native body/owner references through the live tool path; remove the
   reverse `tool:...:Input:text`/Output/Error display-key parser and `tool_roots`
   rescan in Transcript. Preserve existing persisted disclosure keys as UI keys,
   never as a decoder for native identity.
2. Migrate one complete tool path: daemon publication → committed native state →
   retained disclosure/text children → content interest and complete Copy.
   Delete native parent → provider-call-ID → re-paired tool interpretation for
   that path. Display/copy/demand share membership, not identical fetching policy.
3. Move remaining native tool cases to that same path; delete superseded native
   branches in `details::Tools`, `blocks::{snapshot_inner,native_event,plan_visible,
   copy_ready,copy_complete}` as applicable. Keep ordinary-message compatibility
   only until 040–041; do not add a permanent tool adapter for each consumer.

A compact model-owned relationship index or accessor is allowed where needed;
no second editable tool tree and no `ToolProjection`/renamed description tree
sitting between records and retained widgets. Presentation nesting is owned by
those widgets. Retain text shaping/Markdown caches rather than per-glyph widgets.

## Acceptance and deletion gate

Use a small native-record case table for partial children, repeated provider call
IDs, orphan/multiple results, errors/interruption and overflow metadata. Validate
the previously source-only mismatch with a visible body whose heading is outside
overscan: the correct native body is requested. Closed tools do not fetch hidden
output; complete Copy retains sealing, integrity and aggregate byte limits.

Preserve foreground/background budgets, warm scrollback, sparse native updates,
source/version fencing and prepend-stable disclosure keys. No metadata/body/full
history materialization merely to construct the UI. No body placeholder injected
as authored text for migrated tools.

Count replacements across daemon, protocol/shared crates and frontend. Moving
meaning into shared code earns no LOC credit by itself. This slice may have little
net saving; its purpose is to remove repeated interpretation, not pretend to fund
the 5,000-line target. Run targeted native/copy/UI cases using managed nextest.

## First deletion — September 29, 2026

Removed Transcript's `tool_roots` rescan and inverse disclosure-key parser.
The existing detail lines carry their owning remote row directly, so a text child
can request its owner when its heading is above overscan. Keys remain persisted
UI expansion keys, never an ID decoder. This is a narrow deletion in the existing
path, **not** a new ToolProjection or an assertion that the old projection is done.

The real retained viewport case uses a native root/input directory and one verified
body chunk. It scrolls into the body, derives the actual native watch plan from
Transcript's interests, and fetches/verifies the remaining native content. It failed
before the fix (`68921602-657d-459f-999b-02bad85b9854`) and passes afterward, along
with 41 other native/lifetime checks (`0a907926-1377-43ed-8022-2872ef8cecd3`). The
existing cache fixture is reused, rather than copying its database/range setup.
This case seeds native records; it does **not** claim the remaining daemon-to-widget
shared metadata migration is complete.

At this checkpoint, shared metadata/body references, direct native membership and
the provider-ID rewrite/re-pairing deletion were still required (see the next slice).
The rich Row/projection pipeline remains explicitly scheduled for 040–041.

## Native membership and public metadata contract — September 29, 2026

Deleted `native_event`, its provider-call-ID overwrite and the overflow-metadata
restoration of that fabricated ID. The existing bounded View/Feed carry a compact
native parent index. Display and complete Copy group results by that relationship,
not by provider IDs; orphans remain independent roots. `details::Tools` no longer
builds a provider-call set or repairs orphan groups. Bootstrap disclosure grouping
now consumes actual roots, not flattened children followed by re-pairing. Persisted
`tool:<native-id>` keys are unchanged. This is a native relationship index, not a
new editable tool tree or intermediate widget specification.

Shared `ToolState`, textual `ToolBody` classification and input addressing live
in the existing protocol module. Daemon publication/recovery and client demand /
copy readiness use them with unchanged JSON encoding. Metadata/attachment payloads
are not classified as text results. The existing real two-client daemon/network
case opens tools by native IDs, no longer relying on overwritten provider IDs.

The native table now covers repeated provider IDs, multiple mixed success/error
results, an orphan, unknown child coverage, displayed section membership, closed
body demand and complete Copy. It revealed a real mismatch: UI presents one Error
section containing all results, but demand independently applied Output/Error keys
per child. Demand now uses the displayed aggregate section and aggregate length.
The narrow assertion failed before (`9b031b31-d1c3-407d-be97-dbd771d3e122`) and
passes after (`c1c8467f-218e-4642-a3dd-ad2eb560576a`). Provider-ID preservation
also fails at `6171d42` (`b3a75666-4b7e-4721-aa23-ebc2517d80f0`). The temporary
regression worktree was removed.

A daemon-side case exercises actual publication for reused IDs, orphan results,
overflow metadata, success/failure and interrupted recovery using the shared
accessors. Together with the actual viewport-interest case and the real impaired
link/two-client test, it covers both endpoints without exposing private daemon
modules just for frontend tests.

Validation: frontend/daemon/protocol all-target compiler checks and **254/254 tests
across 14 binaries, zero skipped** (`5a638867-c1b4-4e49-af40-fa007a6b1034`).
Frontend production shrinks **21 raw / 13 same-format** but shared production grows
**41 / 69**; frontend tests grow **35 / 42** and daemon tests **36 / 37**.
**Charged net growth is 91 / 135**, not a saving. Cumulative reduction after all
outside charges is **314 / 265**; the 5,000-line gate remains very far open.

Still open: explicit source/scope/version and availability/finality body references,
removal of authored-text loading placeholders, and actual retained tool children
instead of the remaining `Line`/Row/Part reconstruction. Those must be integrated
with 040–041, not declared done because parent membership now works. The existing
text/Markdown caches, complete-Copy integrity/budget gates and sparse updates remain.
