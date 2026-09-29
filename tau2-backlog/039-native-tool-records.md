# 039 — Share native tool meaning and use it directly

Status: **Selected model/UI vertical slice.** Replaces old 015 and the tool part
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
