# Tau 2 client: duplicated meaning, not just duplicated drawing code

Reviewed September 28, 2026 against `origin/tau2` at `40a3698` (0.7.8).
This second pass replaces the first review's ranking of a UI context and scroll
helper as the main opportunities. Those would help locally, but neither
addresses the most consequential duplication. This remains an architecture
proposal, not a runtime change or an instruction to deploy.

## Diagnosis

The interesting problem is not the raw 20k-line count, or simply that `app.rs`
is large. The client repeatedly reconstructs the same domain and interaction
meaning in different representations. Some boundaries discard information that
the next consumer then has to recover using side tables, heuristics or strings.

Two stronger abstractions are justified by the existing code:

1. **A typed transcript projection** preserving item identity, tool hierarchy,
   body references and availability, shared by display, copying and content demand.
2. **A layout-derived interaction scene with explicit gesture ownership** so
   drawing, hit testing, overlays and pointer capture refer to the same targets.

These are different from a general UI framework, a new authoritative client
transcript database, or merely splitting `impl App` across files.

## 1. Preserve the transcript's meaning once

### The current round trip

```text
native block headers / parent relationships / verified cached bytes
  -> blocks::View: flat TranscriptSnapshot<Event> + side tables
  -> Feed: events + queue + lengths + incomplete IDs + tool states
  -> App::rows + Tools::lines: reconstructed groups and display keys
  -> layout: scan events again and decode display keys to native IDs
  -> Controller::viewport
  -> Cache::plan_visible: interpret disclosure/body requirements again
```

The issue is not that these stages exist. Decoding, presentation, layout and
network scheduling legitimately differ. The issue is that they independently
interpret **which logical section owns which content**.

Concrete evidence at the reviewed revision:

| Decision | Repeated implementations |
| --- | --- |
| Which events are visible and form a Details group? | `app.rs:2680–2739`; `details.rs:213–231` (`open_items`, used for the planner's no-viewport fallback) |
| Which input/output sections are expanded and require bodies? | `details.rs:168–198`; `blocks.rs:275–297` |
| Which native tool owns a displayed line? | `blocks.rs:341–346` rewrites the event call ID from native parents; `details.rs:38–60,123–146` rebuilds pairing; `app.rs:3188–3210` reconstructs it again from keys |
| What belongs in a copied Details group? | `details.rs:64–109`; `blocks.rs:225–232,355–387` reconstruct and traverse membership for fetching/completeness |
| Is authored text represented by pending work, a queue row or canonical history? | `app.rs:2697–2701,2741–2799,2802–2926`; retirement is separately checked in `store.rs:191–205` and `blocks.rs:178–189` |

Some checks in this table must remain distinct. In particular, display membership
is not proof of durable delivery, and a copy's completeness checks are not the
same as painting a bounded preview. Share **identity and membership**, not every
consumer's policy.

`BlockHeader` already has a parent and content metadata. Yet `native_event`
encodes its parent identity into `Event.tool_call_id`, and `Feed` needs parallel
maps to retain body length, missing content and tool state. This is evidence of
an awkward event-adapter boundary, not a need to invent another protocol.

### An actual contract mismatch

`details.rs:177–190` produces body keys such as:

```text
tool:native-tool-42:Input:text
```

`app.rs:3203–3209` recovers a native interest by removing `tool:`, optionally
`:body`, then `:Input`, `:Output` or `:Error`. It never removes `:text`.
The heading key resolves; the corresponding body key does not.

A source-level Python reproduction of those exact transformations confirmed this
for Input, Output and Error. A long body inside the viewport/overscan while its
heading is outside it may therefore fail to contribute an interest. **The
GUI/network manifestation has not been reproduced.** The source mismatch is
recorded separately as flag `87d54bdf-7c32-4eab-b7ca-ce6409140806`.

Adding one more suffix rule would patch this case. Carrying the source reference
alongside the display key removes the fragile inverse mapping altogether.

### Proposed boundary: a typed, derived transcript projection

A presentation item should carry, explicitly:

- A stable logical identity scoped to account/source/session. An authored
  message keeps that identity across pending, queued and saved representations.
- Its semantic kind and section membership: message, reasoning, tool, input,
  output, attachment. Use native parent identity, not provider call IDs that
  can repeat across turns.
- References to its native bodies and metadata, including lazy/unknown children.
- Available preview data, separately from expected size, cache coverage and
  sealing/finality. A fully cached body can still have a truncated display preview.
- Presentation state derived from local disclosure preferences and authoritative
  metadata, rather than embedding UI meaning in string keys.

This is a **derived read model**, not a third mutable transcript store. Local
intents and the verified replica remain separate authorities with different
lifetimes. Cached projection/index entries are disposable and keyed by the
relevant revisions; do not eagerly materialize an entire conversation tree.

Consumers use the same item/section membership for different purposes:

```text
replica metadata + available bodies + local intents
                    |
             transcript projection
              ↙         ↓         ↘
       display/layout  copy scope  stable logical targets
              |           |
       visible sources   complete-content demand
              \           /
          existing bounded native scheduler
```

The scheduler still owns authorization, cache limits, priorities, resumable
transfers and admission. It should receive explicit source demands, not know how
the UI names its Input/Output rows or stores disclosure keys. Preview and Copy
remain distinct demands; Copy must retain sealing, integrity and size checks.
A no-renderer bootstrap can select its bounded recent window without duplicating
the screen's grouping algorithm.

Likewise, normalize pending/queue/history **display precedence** once by request
identity. Do not let `rows()` independently decide message continuity at paint
time. Keep conservative outbox-retirement evidence separate: an accepted header
or receipt does not mean canonical bytes can replace the user's authored text.

Typed body availability would also stop passing literal `"Loading…"` as if it
were authored `Event.text` (`blocks.rs:194–195,208–209`, `feed.rs:35`). The UI
currently partly reverses this encoding (`app.rs:2788–2791`). Placeholder text
belongs at the presentation boundary, not in source content.

### What this should replace, not merely wrap

- The duplicate visibility/group scans in `rows` and `open_items`.
- `chat`'s `tool_roots` rescan and display-key decoding.
- Independent disclosure-key interpretation in native fetch planning.
- Repeated reconstruction of section membership for Copy.
- Parallel per-item availability lookups spread across projection consumers.

Native decoding, sparse cache updates, memory budgets and delivery evidence do
not disappear. Do not put one new object in front of all the old algorithms and
call that consolidation.

## 2. Make interaction ownership explicit

### The current coordination cost

Rendering populates `hits`, `message_areas`, `detail_areas`, `chat_areas`,
`project_areas`, `info_areas`, scrollbars and several standalone rectangles.
Markdown separately retains text/link hit geometry. These records are reset and
rebuilt in `frame` (`app.rs:2210–2256`).

Input then searches and prioritizes these representations separately in:

- hover and cursor selection (`app.rs:510–604`),
- press, movement and release (`1097–1388`),
- long-press handling inside `tick` (`853–869`),
- context-menu dispatch (`3780–3875`).

Those paths repeatedly ask whether a modal/viewer/menu is open and whether an
interaction belongs to a control, text, a tooltip, a scroll pane or the image.
Gesture ownership is distributed among `pointer`, `pinch`, `scroll_drag`,
`selecting`, `field_selection`, and unrelated screen-state checks. For example,
`release` searches current hit rectangles using both endpoint coordinates rather
than an explicit pressed-control identity.

The missing abstraction is not just `button()`. It is **which stable target owns
this interaction, under which overlay and clipping rules?**

### Proposed boundary: one interaction scene + gesture state

Layout should produce interactive regions with stable target IDs, geometry,
clip/shape, stacking order and supported behavior. Drawing and event routing
consume those same region records. Existing Markdown/editor geometry remains
the specialist text hit tester for its region; do not replace it with coarse
rectangles or duplicate font/caret logic.

Resolve a press to a target and gesture candidates. As movement/time resolves a
touch into tap, drag or long press, one explicit gesture state owns the sequence:
control press, text selection, scrolling or viewer pan/pinch. Keep independent
visual effects such as a released ripple separate from exclusive gesture state.

The owner remains stable while layout moves. If its scoped target disappears,
cancel rather than silently retargeting the action under the pointer. Controls
still validate release position and cancellation. Overlay routing decides once
whether input is blocked, dismissed, consumed or eligible to reach a lower layer.
A read-only tooltip can block clicks without becoming a command handler.

This can retain immediate-mode drawing. It needs neither a virtual DOM nor a
trait-object widget hierarchy: a layout-produced region list, typed IDs, an
explicit overlay/input boundary and scoped capture state are sufficient to pilot.
Do not force all overlapping scene regions into one global exclusive enum;
only gesture ownership is exclusive.

### What this should replace

- Repeated hit-priority and overlay-blocking policies in each event handler.
- Parallel geometry registration for one logical interactive target.
- Re-identification of a pressed target from whatever occupies its old coordinates.
- Scattered gesture cancellation and long-press dispatch logic.

A reusable `ScrollAxis` becomes a useful mechanism underneath this, not the
headline architecture fix. Transcript anchor restoration is still a separate,
explicitly deferred design question (backlog 012); this review does not change
it or remove its tests.

## Other real duplication, lower priority

- **Delivery recovery policy:** `store.rs:305–313` and
  `controller.rs:1143–1153` independently implement Sending/Preparing ->
  Checking versus Unconfirmed, and rejection of an unconfirmed model selection.
  Prompt-vs-control classification is also repeated in `not_sent` and receipt
  handling. A shared typed recovery transition is more valuable than a generic
  persistence wrapper. Do not collapse all operations into identical automatic
  retry behavior: ordinary messages and potentially effectful controls differ.
  Consolidating the three durable-intent containers would require a separate
  persistence/migration design, not a casual UI refactor.
- **Dialog lifecycle:** modal kind, anonymous indexed editor fields, loading and
  saving flags, submit dispatch and completion handling are distributed across
  `apply`, `tick` and special renderers. Typed dialog state could own these, but
  the existing shared `Editor` and table-driven daemon settings should remain.
- **Test scaffolding and download acquisition:** still valid smaller cleanups
  from the first pass. They are not the central explanation for complexity.

## A concrete first refactor, not a new architecture all at once

1. Give every generated detail line an explicit native interest/source reference.
   Preserve existing string keys for persisted disclosure/layout identity during
   this slice. Delete the inverse-key parser and `tool_roots` rescan. Check a tool
   body whose heading is outside overscan, including orphan result cases.
2. Extract one semantic grouping/projection path consumed by display and demand
   construction. Move membership decisions out of the native scheduler while
   retaining its limits and distinct preview/copy policies. Port native fixtures
   alongside this; do not preserve a second legacy renderer solely for tests.
3. Pilot layout-derived interaction regions and captured targets on one nested
   surface, such as an attachment card inside a message. Preserve rounded clips,
   control precedence, long-press behavior and no click-through before expanding.

Use focused compiler checks and existing nextest behavior coverage if these
changes are implemented. Verify negative behavior too: closed tools do not
fetch bodies, previewing does not export an image, and disconnected controls
are not automatically executed. Do not turn this proposal into a full release
campaign or claim that abstractions alone prove faster frames.

The success criteria are fewer independent definitions of the same rule, removed
inverse mappings, and fewer coordinated state transitions. There is no measured
basis yet for a percentage LOC saving. Moving methods or adding a wrapper while
retaining all the independent interpretations would not meet those criteria.

## Size appendix — unchanged measurements

Physical lines include comments and whitespace, not executable statements or
runtime cost. The count alone neither condemns nor vindicates the architecture.

| Scope | Lines |
| --- | ---: |
| `frontend/src/**/*.rs`, including unit tests | 18,512 |
| `frontend/tests/**/*.rs` | 1,854 |
| Source plus tests | 20,366 |
| Test-only portions | 6,123 |
| Non-test source | 14,243 |
| `frontend/build.rs` (additional) | 23 |
| All tracked frontend Rust | 20,389 |

The 14,243 non-test lines comprise: application UI/layout/dispatch 5,671;
rendering/editor/text/UI primitives 2,579; state/storage/sync/settings/status
models 4,687; platform adapters/export/startup/demo 1,306. Both mutually exclusive
platform adapters are counted; shared workspace/dependency sources and the Java
bridge are excluded. `app.rs` itself is 4,840 lines (4,789 non-test), with 80
direct state fields.

Gross count: `git ls-files -z 'frontend/*.rs' | xargs -0 wc -l`.
Test split: all `frontend/tests`, all source files named `tests.rs` or
`*_tests.rs`, and the suffix starting at the first top-level test-gated module
in every other source file. At the reviewed revision these modules are all at
file ends. Also count the test-only import at `app/attachments.rs:2–3` and nine
test-helper lines at `blocks.rs:94–95,116–117,126–127,136,237–238`. This produces
4,269 test lines under `src` plus 1,854 integration-test lines.

## Scope and validation

Work remains on the dedicated `review/tau2-client-structure` branch/worktree.
The first pass counted files and inspected feature boundaries; the second traced
semantic ownership through replica projection, display, copy, content demand,
input routing and recovery. Counts were checked with Python; the key-contract
mismatch was reproduced independently from the source transformations. That is
not a Rust test, device test or demonstrated end-to-end UI failure.

Only this review document changes. No application source, dependencies, protocol,
persisted user data, services or release artifacts have been changed. No Rust
build, behavioral suite, runtime profiling, deployment or device acceptance was
performed for the review.
