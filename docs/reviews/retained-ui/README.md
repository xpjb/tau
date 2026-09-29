# Retained UI rewrite review — September 29, 2026

## Verdict

**A real ownership rewrite, but not a simplification, and not a complete interaction/rendering rewrite.** Keep the retained owners, stable IDs, source/IME fences and shared composer. Do not start another wholesale rewrite. Finish the common interaction and compositing boundaries before adding more per-feature scaffolding.

There are two independently reproduced regressions: broken overlay compositing and selection/autoscroll coupling. The reported folder-button styling and child/parent feedback problems also reproduce, but those were carried over from the old implementation. A larger source tree did not purchase the promised uniform control semantics.

This is a review, not a product fix or release. No production changes, deployment, live-account access or physical-device acceptance are included.

## Scope and attribution

- Before the rewrite: integration `415aeff6d27554ca0d594fd305eaaa9090e4bb45`.
- Completed rewrite: `186ad2373fa14a7936547ab52e299d661d5bcf0b` (merged as `7ab80f3`).
- Current reviewed integration: `33f7d6f6b45533e081e5021b0c3f69b1e045b6f9`, freshly fetched at review start.
- `cafef7f` is the earlier, approximately 20k-Rust-code-line frontend, before the remote code browser.

Both regressions pass equivalent real-App/GPU probes at `415aeff`, fail at `186ad23`, and still fail at `33f7d6f`. Thus they are not attributable to the subsequent fzf picker or ZIP fixes. File/line references below are to `33f7d6f` unless a different revision is explicit.

## Findings, in priority order

### 1. [P1] One render layer destroys overlay z-order — introduced regression

**Locations:** `frontend/src/app.rs:199–215`; `frontend/src/app/ui/mod.rs:243–284`; `frontend/src/render.rs:885–913`.

`App::frame` now creates one `Layer`, and Root appends workspace, tooltips, notice/menu/viewer and dialog drawing into it. The unchanged renderer does **not** preserve that interleaving: within each layer it renders all shapes, then all images/icons, then all text. Consequently a later opaque dialog rectangle cannot cover earlier workspace text or icons. Menus, tooltips and floating controls have the same compositing hazard.

The old App submitted `[main, body, chrome, overlay]`. Deleting those layers without changing the renderer's ordering contract is a behavior change, not merely ownership relocation.

**Reproduction:** populate the offline demo and open Connection. The attached `opaque-dialog.png` is the actual current headless rendering: transcript text runs through the URL/token fields, and sidebar/header text and icons remain visible. The probe finds 521 non-background pixels in a sidebar-label region that should be fully covered. The same assertion passes on the pre-rewrite App.

**Required correction:** restore ordered compositing layers/batches at the component/overlay boundaries, or use an ordered draw-command representation. Preserve the existing renderer if separate layers suffice. Input opacity does not establish visual opacity. Add assertions over occluded content, not merely that a dialog emitted some pixels.

### 2. [P2] Autoscrolling a text selection no longer advances its endpoint — introduced regression

**Locations:** `frontend/src/app/ui/transcript.rs:551–566,593–612`; `frontend/src/app.rs:199–218`.

Pointer Move extends the selection. Tick moves the viewport during a held drag, but neither the subsequent frame nor release updates the selection from the newly laid-out text under the stationary pointer. The old App did this after laying out the frame (`415aeff:frontend/src/app.rs:2508–2516`).

**Reproduction:** drag toward the bottom edge of a long message and hold still. After 16 ticks/frames the viewport exposes later text, but the selected endpoint stays at byte 595 while the pointer is over byte 805. Equivalent pre-rewrite behavior passes. Copying the selection consequently omits the text that scrolled under the held pointer.

**Required correction:** let Transcript reconcile its selection against the newly placed text after scroll/layout changes. Keep this local; do not restore the monolithic App dispatcher. This is selection mechanics, not the deferred scroll-position restoration redesign.

### 3. [P2] Retained input ownership does not own hover/press feedback — carried-over architectural gap

**Locations:** `frontend/src/app/ui/transcript.rs:499–523`; `frontend/src/app/ui/message_row.rs:101–106,146–150,324–333`; `frontend/src/app/ui/controls.rs:118–144,494–557`.

The input side correctly captures the attachment button and short-circuits its row's activation handler. But Transcript calls `row.press_visual(point)` **before asking children to consume Down**. Separately, MessageRow's painting checks the global pointer position and always allows its attachment row to highlight. It never consults the retained hot/captured target to suppress the parent.

**Reproduction:** hover Open on a saved attachment in the transcript. `ui.hot` is the button, yet an unrelated point on its parent changes from RGBA `[22,78,99,255]` to `[46,87,105,255]`. Pressing the button gives it capture but also creates the parent's ripple. These are independent failing assertions.

**Important limit:** the primary-click probe emits exactly one `UseDownload(Open)` platform action and opens no parent menu/dialog. This verifies the reported visual/parent-press leakage, **not** a general claim that two business actions run on every click. The exact other panel/control, if any, that fires a second business handler still needs its own reproduction.

The old App also created a ripple through `section_at(point)` after finding a child hit, and painted hover using raw coordinates. This is an unfulfilled correction goal, not a newly introduced defect.

**Required correction:** resolve the frontmost interaction owner first, use that same owner for feedback and cursor/tooltip state, and let scroll ancestors explicitly observe a gesture candidate without firing their own activation feedback. Reuse this policy across ordinary buttons, attachment controls, row/tile controls and floating controls.

### 4. [P3] Folder styling is selected by icon identity, not button intent — carried-over gap

**Locations:** `frontend/src/app/ui/controls.rs:509–557`, especially `521–523`; `frontend/src/app/ui/header.rs:139–193`.

The shared icon helper makes only Stop, Play and Attachments tonal at rest. Folder is absent, so its background exists only while hovered. Header passes both Folder and Attachments as enabled, nonprimary peer actions, but they receive different chrome. A GPU probe confirms idle pixels `[14,20,27,255]` versus `[24,33,43,255]`.

The same whitelist exists in the pre-rewrite `App::icon_button`; moving it preserved the inconsistency. A retained struct alone cannot correct it.

**Required correction:** a small explicit button-style choice (for example primary/tonal/quiet) shared by text and icon content. Choose the intended style at the call site. Do not grow the icon whitelist, and do not assume every icon in the application must have identical prominence.

## What actually grew

`tokei` counts below are from clean Git archives, running `tokei . --output json` inside each archived `frontend`. They include tests. Rust code excludes comments/blanks; it is not the production-only UI-scope metric.

| Revision | Rust code lines | All-language code lines |
|---|---:|---:|
| `cafef7f`, before code browser | 20,731 | 21,535 |
| `415aeff`, immediately before retained rewrite | 21,562 | 22,368 |
| `186ad23`, completed retained rewrite | 26,773 | 27,703 |
| `33f7d6f`, current reviewed integration | 27,725 | 28,662 |

The approximately 20k → 27k Rust-code increase therefore splits into **+831 before the rewrite, +5,211 in the rewrite, and +952 afterward**. The total is +6,994, not all attributable to this patch.

The existing audit script independently reproduces the rewrite's production/test ledger:

| Rewrite delta, `415aeff` → `186ad23` | Lines |
|---|---:|
| Production physical source, all charged frontend/interop changes | **+4,059** |
| Test-only physical source | **+1,418** |
| Total physical frontend source delta | **+5,477** |
| Production with both versions formatted identically | **+3,050** |
| Production nonblank, identical formatting | **+3,007** |

So the user's +3k core note is accurate. Formatting explains about 1,009 of the raw production-line increase, **not** the remaining 3,050. The code-browser production file alone is +851 physical versus +141 same-format lines. Neither tests nor formatting explain away the core increase.

The implementation ledger already admits the missed reduction target. Its effective UI-scope total is 10,679 physical production lines, versus a 6,620 baseline, including other touched production deltas. Those are **not** whole-frontend production totals. The much larger frontend `tokei` count also includes transport, replication, persistence, rendering, hosts and tests.

Reproduce the detailed existing ledger:

```sh
python3 scripts/retained-ui-size.py 415aeff 186ad23
```

## Avoidable costs and architectural incompleteness

These are refactoring opportunities supported by source inspection, not independently additive LOC savings promises.

1. **Two ways to paint a button, only one partially owner-aware.** `Button::visit_perframe` sets target-filtered press state, but free `button`/`icon_button` functions draw first and then register with `Controls`. Attachment actions, sidebar rows and quick-model tiles have further hand-painted variants. Even Button's hover uses geometric containment rather than `ui.hot`. We pay for retained identity/reconciliation *and* keep much of the old draw-and-register mechanism. Consolidating this boundary is both a correctness fix and the strongest simplification opportunity.
2. **Form input was shared; form layout mostly was not.** `Form` (`controls.rs:341–405`) provides buttons, tabbing and event dispatch, not the proposed modest form-row/layout helper. Dialogs independently repeat sizing, labels, field placement, footer/button placement, feedback, construction and local choice dispatch. The destination ledger puts dialogs/menu/notices/tooltips at **2,640 production lines**, versus **910** for common controls/scroll/routes. Those are destination sizes, not amounts that can all be deleted. Small field-row/footer/feedback helpers are more defensible than a form schema or another UI framework. Keep genuine validation and async completion ownership local.
3. **The tree's visibility and ancestry are manually described several times.** `RootWidget::active_route` (`routes.rs:6–109`) reaches through workspace, pane, code view, composer, rows and cards to reconstruct paths. Workspace/ChatPane separately reproduce visibility in event traversal and frame traversal (`workspace.rs:245–415`); timers have their own knowledge. `active_route` recognizes scopes, not leaf membership, so individual owners must also call detach correctly. This is a hand-maintained shadow traversal, not a second allocated widget registry, but it still creates coupling and growth pressure. Co-locate each owner's active-child/route policy with its composition rather than extending another root switch for each new feature.
4. **Collection and event contracts are still split.** `Form::event` returns `Option<A>`, losing consumed-without-activation, whereas `Controls::event` returns `(bool, Option<A>)`. For example AttachmentBrowser handles a form choice and otherwise continues into cards/scroll (`attachments.rs:528–539`), even if the form consumed Down/Hover. `Controls` also keys identity with `format!("{slot}:{choice:?}")` and repeatedly searches/removes/reinserts a Vec (`controls.rs:432–447`). Prefer one modest consumed/action result and typed stable keys. Do not expand a `Debug` serialization convention into an identity framework. No performance regression is asserted from the linear searches without a benchmark.

There is real retained state: stable IDs survive row reorder/streaming, editors retain state, controls retain clips/capture identity, widgets are source-bound, and transcript interaction owners are virtualized. The global production Action/Hit/Lane/Modal machinery was removed. The problem is **incomplete common semantics underneath those owners**, not that the files merely moved or that retained UI inherently must shrink.

The implementation deliberately retains full projection/measurement on dirty frames (`transcript.rs:234–290`) and reconciles row/part collections afterward (`396–435`, `message_row.rs:44–76`). That is not inherently incorrect and does not justify a reactive engine. It does mean retained identity should not be described as incremental layout or a demonstrated performance win. Preserve the existing text/document caches; avoid adding another model.

## Recommended sequence

1. Fix compositing and the selection regression with the failing probes as gates.
2. Unify control interaction/feedback and explicit visual styles; migrate one nested attachment row and one floating control first. Remove their bypass paths as they migrate.
3. Tighten consumed/action and child-route/visibility helpers; then extract only demonstrably repeated form layout.
4. Re-run the same-format production audit after each slice, charging all touched files and separating tests. Require actual deletions/reuse before broad migration. No credible measured basis currently supports promising all 3,050 production lines back.

## Review validation and artifacts

- `regressions.patch`: six opt-in real-App/headless-GPU tests with desired-behavior assertions. It applies to both `186ad23` and `33f7d6f`; five fail and the single-business-action control passes on each. The styling assertion expresses the requested peer-style consistency, not a historical style guarantee.
- `baseline-regressions.patch`: the two equivalent pre-rewrite compositing/selection probes adapted only for the old App ownership/API; both pass on `415aeff`.
- `opaque-dialog.png`: actual current rendering, inspected during review; not a mockup.

Probes are supplied as patches **outside frontend**, rather than leaving deliberately failing tests in the ordinary suite. They are additional review artifacts, not production simplification credit. Apply only in a disposable worktree:

```sh
git apply /path/to/docs/reviews/retained-ui/regressions.patch
TAU_REVIEW_PREVIEW_DIR=/tmp/tau-review \
  /usr/local/bin/cargo nextest run --locked -p tau-frontend --lib \
  --no-fail-fast -E 'test(app::ui::review_tests::)'
```

For the baseline patch use a `415aeff` worktree and filter `test(app::review_tests::)`.

Current six-probe run: `f854534d-c2de-4b07-b6f8-2edcfa3337af`. Completed-rewrite six-probe run: `2e25e46b-32b9-4d84-8515-196510fa4f64`. Pre-rewrite two-probe run: `ea738f66-9081-45c1-9dae-d69801ac47b6`. Full current frontend-library validation is recorded in the follow-up below.

All compilation/test commands use managed `/usr/local/bin/cargo` and nextest. No Clippy, built-in Cargo test runner, physical Android/Windows QA, packaging or deployment was performed.
