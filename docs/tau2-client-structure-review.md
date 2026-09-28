# Tau 2 Rust client: size and abstraction review

Reviewed September 28, 2026, against `origin/tau2` at `40a3698` (0.7.8).
This is a source audit and proposal, not an implementation or release change.
Locations below refer to that source revision.

## Bottom line

The client is not 20,000 lines of UI implementation. About 30% of that number
is tests; much of the remainder is the offline client, storage, native content
sync, platform integration, and deliberately custom rendering/input behavior.
There is one shared desktop/Android UI, not two separately implemented screens.

There is nevertheless a clear abstraction gap between the drawing primitives
and the application. `app.rs` contains too much layout, interaction, and feature
state in one owner. Two modest abstractions look worthwhile: a short-lived
UI/layout context and shared scroll-axis mechanics. Feature-owned state would
make those useful beyond merely shortening calls or moving methods into files.
No evidence from this audit justifies a new general-purpose GUI framework.

## What the line count actually includes

These are physical lines, including comments and whitespace, not executable
statements, binary size, or runtime cost. Formatting varies substantially.

| Scope | Lines |
| --- | ---: |
| `frontend/src/**/*.rs`, including unit tests | 18,512 |
| `frontend/tests/**/*.rs` | 1,854 |
| Source plus tests | **20,366** |
| Test-only portions of the above | **6,123** |
| Non-test source | **14,243** |
| `frontend/build.rs` (additional) | 23 |
| All tracked frontend Rust | **20,389** |

Both mutually exclusive platform adapters are counted. Shared workspace crates
such as `markdown`, `protocol`, `blocks`, and `transfer`, dependency sources, and
the Android Java bridge are not included in this frontend Rust count.

A responsibility-oriented breakdown of the 14,243 non-test lines:

| Responsibility | Lines | Files |
| --- | ---: | --- |
| Application UI, layout, gestures, actions, dialogs | 5,671 | `app.rs` and non-test `app/*` |
| Drawing, editing, text presentation, UI primitives | 2,579 | `render`, `editor`, `fonts`, `icons`, `details`, `keyboard`, `scroll`, `tooltip` and `tooltip/text` |
| State, storage, sync, transport, settings/status models | 4,687 | `controller`, `store`, `feed`, `blocks/*`, `transport*`, `connection`, `daemon_settings`, `models`, `cache_ttl`, `codex_usage`, `clock` |
| Platform adapters, file export, startup, offline demo | 1,306 | `desktop`, `android`, `downloads`, `disk`, `lib`, `main`, `demo` |

Not all of the first row is drawing code: it also includes application policy
such as local-message/queue/history presentation, modal confirmation, and
account-bound download completion.

### Counting method

The gross count is reproducible with:

```sh
git ls-files -z 'frontend/*.rs' | xargs -0 wc -l
```

For the test split, count all `frontend/tests` files and source files named
`tests.rs` or `*_tests.rs`. In other source files, exclude the suffix starting at
the first top-level test-gated module (`cfg(test)` or `cfg(all(test, ...))`).
All such modules are at the end of their files at the reviewed revision.
Also exclude the test-only import at `app/attachments.rs:2–3` and the nine
physical lines of test-only helpers at `blocks.rs:94–95`, `116–117`, `126–127`,
`136`, and `237–238`. This gives 4,269 test-only lines under `src`, plus 1,854
integration-test lines. Test attributes/module declarations are included in the
test count; intervening blank lines follow their surrounding section.

## What is already sensibly shared

- **One editor:** `editor.rs:41` owns text, selection, undo, composition and its
  viewport; composer and dialogs already reuse it. `app.rs:1409–1584` routes
  focused editing through that component. Do not invent a second field editor.
- **One attachment presentation:** `app/attachments.rs:65–182,323–498` computes
  display state and draws the card used by both the transcript and attachments
  pane (`app.rs:2330,3378`). This is not duplicated UI needing another renderer.
- **Existing primitives:** `render.rs:87–180` provides layers, clipping and
  interaction coloring; `app.rs:4686–4728` already has a shared text button.
  `Tooltip`, `Wheel`, and `Scrollbar` are also existing abstractions to extend.
- **Shared rendering state:** `render.rs:183–209,481–502` retains and updates
  Markdown documents/layouts rather than creating a new parser for every label.
- **A real client/data boundary:** controller, feed, persistent local work, native
  replica, and transport have different jobs. Durable author intent and a
  disposable remote-content cache are not interchangeable copies of state.

## Where the structure is doing unnecessary work

### 1. UI context + small layout helpers — highest-leverage abstraction

`app.rs` is 4,840 lines (4,789 non-test). Its `App` has 80 direct fields.
`chat` occupies lines 2929–3617; `apply`, 1589–2209; `tick`, 614–880.
This concentrates unrelated responsibilities even where there is no literal
copy-paste.

The file has 139 `Rect::new` calls, 23 calls to the existing free `button`, and
47 ordinary/clipped label calls. The repeated pattern is manual scaled geometry,
style arguments, renderer/layer plumbing, clipping, and hit registration.
Settings and modal forms repeat `Editor::draw` followed by a focus hit
(`app.rs:4041–4084,4530–4545,4654–4667`; `app/projects.rs:233–245`).

**Proposal:** extend the existing helpers into a short-lived `Ui` context
borrowing the renderer, one layer, hit sink, scale, clip, and theme. Add small
row/column or rectangle-splitting helpers and named label/button styles.
For example, conceptually:

```rust
let rect = column.take(40.0);
ui.button(rect, "Save", Action::Confirm, ButtonStyle::Primary);
ui.field(next_rect, &mut editor, field_id, field_style);
```

This is a proposed API shape, not existing or compiled code. Drawing and hit
registration should share geometry; clipping must preserve layout coordinates
while intersecting the interactive area. `field` delegates to the existing
`Editor`, not a new input implementation. Preserve layer order, disabled-control
behavior, focus visibility, and modal input blocking.

Keep this context separate from `App` and `Controller`: borrow disjoint fields
for a short render pass and keep using typed `Action` values. Do not replace
explicit actions with an untyped callback/event bus, or borrow the entire app
inside every widget. Start with a couple of forms before generalizing.

This would remove repeated decisions, not just repeated argument lists. A
helper wrapping `Rect::new` alone would mostly hide the current complexity.

### 2. Scroll-axis state — the clearest literal runtime duplication

Transcript, attachment pane and topic tabs independently track offset, maximum
and velocity (`app.rs:276–328`). `tick` repeats the same clamped inertial step
and exponential decay three times (`app.rs:828–852`); `motion` repeats drag
velocity estimation (`app.rs:1283–1306`). `scroll_value`/`set_scroll` dispatch
across the same separate state (`app.rs:946–966`).

**Proposal:** a small `ScrollAxis { offset, max, velocity }` with shared clamp,
drag, stop and inertial-step operations, reusing the existing wheel/scrollbar
math. Enable inertia only for lanes that already have it; topic tabs are
horizontal, whereas attachment/transcript scrolling here is vertical.

Crucially, **mechanics are not restoration policy**. Transcript follow-tail,
session-bound measured layout, stable row anchors, paging and expansion pins
belong in a transcript-specific owner above the generic axis. Text-editor
scrolling is in em-space and follows the caret; do not force it through a
pixel-based transcript policy. Menu keyboard reveal is also a separate policy.

See backlog `012-chat-scroll-position-audit.md`: restoration is explicitly
provisional and the user deferred its redesign and associated test removal.
This review changes neither. A mechanics extraction must not be presented as
resolving that deeper ownership question.

### 3. Give features state ownership, not just separate filenames

`app/projects.rs` and `app/attachments.rs` mostly add more `impl App` methods
using `super::*`. Those are useful file splits but still operate on the same
large object. The attachments pane layout and image viewer remain inside
`frame`; settings transactions, connection state, selection changes, timers and
scrolling meet in `tick`.

Natural owners include transcript layout/anchor state, attachment/export jobs,
settings dialog state, and overlays/focus. Their render/event methods should
consume narrow inputs and emit actions; `App` should coordinate them. Avoid a
component trait hierarchy until multiple real components need the same
interface. In particular, extracting `chat()` to another `impl App` file is not
by itself a reduction in coupling or total code.

For the transcript, the existing `rows()` projection (`app.rs:2680–2928`) is a
promising boundary from domain state to display rows. Preserve stable message
identity and local authored text while receipts, queue entries and canonical
bodies arrive independently. This is correctness logic, not boilerplate to
compress into a generic list widget.

### 4. Smaller, concrete cleanup candidates

- **Download acquisition:** `Action::Attachment` and `Action::SaveAttachment`
  (`app.rs:2137–2181`) repeat transfer startup, error capture, completion checks,
  and pending-export bookkeeping. A shared acquisition operation with explicit
  preview/save intent could remove that repetition. Do not merge the semantics:
  image preview is cache-only; file activation/export may save, and completion
  must stay bound to account, lineage and the requested path. A typed export
  job could also replace some coordinated maps, after auditing cancellation.
- **Test fixtures:** editor, project, icon and download interaction tests have
  separate versions of temporary store + headless GPU + offline demo setup.
  Share a per-test harness with explicit sizing/seeding options, not global
  mutable app/GPU state. Reduce scaffolding without deleting useful assertions.
- **Old transcript adapters:** `controller.rs:1373–1433` still handles
  snapshot/update/page variants; `feed.rs:150–263` retains old update/page
  algorithms. The protocol explicitly marks these variants `#[serde(skip)]`
  (`protocol/src/lib.rs:212–231`), and actual history uses native blocks
  (`controller.rs:870–881`). They are not a second live network path.
  Demos and tests still inject them. Migrate those callers to an explicit
  in-process fixture/view API before retiring obsolete adapters. **Do not delete
  `Feed::snapshot` wholesale:** native full projection and startup previews
  still use it. This is a modest cleanup opportunity, not thousands of dead lines.

## Recommended sequence and limits

1. Pilot `Ui` + layout/style helpers on two existing dialog/form renderers.
   Require a net reduction in repeated behavior and unchanged geometry/hits.
2. Consolidate scroll-axis mechanics separately, keeping existing behavior and
   keeping transcript restoration policy explicit.
3. Move cohesive feature state out of `App` as those boundaries become clear;
   separately retire legacy fixture adapters and duplicate test setup.

For an implementation, use focused existing render/input/download tests through
managed Cargo/nextest and compiler checks appropriate to the changed paths.
Do not turn this source audit into a release gate, a blanket test campaign, or
an excuse for unrelated rewrites. Preserve the existing deferred-scroll scope.

No runtime profiling was performed. Existing Markdown/layout reuse should be
preserved; a cleaner UI API does not by itself prove faster frames. Likewise,
there is no measured basis for promising a particular percentage reduction.
Judge success by fewer places to change a behavior and clearer state ownership,
not by making a 20k-line counter read 10k.

## Work performed

Fetched `origin/tau2`, created the dedicated `review/tau2-client-structure`
branch/worktree, counted the tracked source and test sections, and traced UI,
scrolling, editing, attachment, controller and native/legacy transcript paths.
Only this review document is changed. No application code, dependencies,
protocols, data, services or release artifacts were changed. No Rust build,
behavioral test run, deployment, or device acceptance was performed or needed
for this documentation-only review.
