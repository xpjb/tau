# 037 — One retained control and child-traversal contract

Status: **Source-complete. Physical interaction acceptance remains open.**
Depends on 036's paint-order contract. Absorbs unfinished
005/008/009/028 and residual 030/031 work. Keep the concrete Widget tree and broad
sibling-borrowed Context/Services; do not add another framework.

## Closed slices

1. **One nested attachment row + header button.** Resolve the frontmost target
   using its placed bounds, inherited clip and shape; use that owner for hover,
   press/ripple, focus, cursor, tooltip and activation. Children consume before
   parents activate. Scroll ancestors may observe a candidate and explicitly
   cancel/take over; observation is not a parent click or highlight.
2. **Migrate every control consumer.** Text/icon buttons, attachment actions,
   sidebar/topic rows, quick-model tiles, floating controls and scrollbars must
   use the same boundary. Explicit primary/tonal/quiet intent replaces the icon
   whitelist: Folder gets its intended idle surface. Keep visual colour/ripple
   tuning in this migration, not another parallel effects implementation.
3. **Composition owns traversal and lifetime.** Each parent defines active child
   order/visibility locally for layout/paint, reverse-order input and active-target
   validation. Remove Root's hand-maintained descendant paths and duplicated
   responsive visibility policy. No second registry, generated shadow tree, or
   reflection machinery. Moving the same switch into another file is not enough.

Parent-provided bounds and inherited clip are sufficient; keep the existing
window-coordinate convention unless a specific local transform earns its cost.
Container-local responsive decisions must not be repeated in root route code.

## Required deletions

- Free `button` / `icon_button` paint-first/register-later bypasses in
  `ui/controls.rs`, plus special raw-pointer hover/press painting in migrated owners.
- Transcript's speculative `press_visual` before child dispatch, and parent
  `surface_highlight` based only on pointer containment.
- `Controls` identity derived from `format!("{slot}:{choice:?}")`; use actual
  stable domain keys/owned controls without a second identity serialization scheme.
- The split `Form::event` / `Controls::event` consumption semantics. Consumed without
  activation must remain consumed, including disabled foreground controls.
- Root-owned descendant knowledge in `ui/routes.rs`; redundant per-feature hit,
  visual and lifetime registration. Retain only minimal active target/path state.

## Acceptance

A child hover/press affects only its intended surface and dispatches one action;
parent blank space still works. Rounded/clipped/disabled controls block or consume
as intended. Capture follows its owner across sibling bounds, rejects another
pointer, survives reflow, and cancels on hide/detach/source replacement **before
another frame**. Claimed selection is not stolen by scrolling. Keyboard/IME/paste
and late async completions retain their current source/instance fences.

Input consumption must not short-circuit lifecycle updates: split Tick/update from
consumable input, or make their traversal explicitly different. On-demand drawing
stays on-demand; elapsed time/deadlines, not frame counts, drive effects. A redraw
visits layout/paint; it does not imply a continuous loop. Hidden-owner cancellation,
notice deadlines and durable operations must not depend on being painted.

Use the surviving nested-control/lifetime cases in 042, not a test for every
button's coordinates. Inspect subdued highlights/ripples at desktop/phone scales;
005/008/009 remain visually unaccepted until that check. Publish net production
and test deltas after the pilot and migration; do not expand a growing wrapper.

## Control checkpoint — September 29, 2026

The nested-control pilot and control-consumer migration use the existing
`Control` for placed geometry, capture, hot target, cursor and ripple ownership.
Text/icon buttons both visit the retained Button; attachment actions, sidebar
rows/topic tabs, quick models and other custom surfaces use the same owned
feedback. Scroll chrome reads its routed owner, not raw pointer containment.
Folder and its header peers request an explicit tonal style.

Deleted the free paint-then-register button/icon helpers, icon-based background
whitelist, renderer `Interaction`/raw-pointer colour helpers, transcript-wide
speculative ripple dispatch, per-row ripple lifecycle loops and Debug-serialized
control keys. Dynamic identity compares typed action values plus an optional
local slot. The only protocol change derives equality for QueueOperation; wire
and storage are unchanged and its raw/normalized code-line delta is zero.
Form and Controls now both return consumed separately from activated. Viewer and
attachment-browser parents respect consumed-without-action. Parent frame clips
are passed to the common button visit. Tooltip cards block underlying hover.

Validation: frontend all-target compiler check; **165/165 frontend library tests**,
zero skipped (`ccede3fe-331d-4e40-b5a9-427f16e6c7f1`), then 5/5 targeted checks after
the final title-feedback/disabled-ripple changes. The six historical diagnostic
probes now all pass (`78fef44f-089d-45f4-a3ae-0474bbcdda7c`, 8/8 including two
surviving checks). The temporary probe's parent-ripple observation was adjusted
only to its new control owner; the probe module was removed afterwards. Desktop
attachment idle/hover renders were inspected: only the child highlights, and
Folder now has an idle surface. Physical device/interaction acceptance stays open.

This checkpoint is **not all of 037**: parent-local active-child order, removal of
`ui/routes.rs`'s descendant knowledge, leaf/hide validation before another frame,
and non-consumable lifecycle/Tick traversal are the next closed slice. Do not
start the forms/model rewrite by declaring those remaining contracts done.

Net checkpoint reduction relative to 036: production **137 raw / 129 same-format**,
tests **166 raw / 185 same-format**. Cumulative reduction from the original
baseline: **303 raw / 244 same-format**, including 036's correction costs. No new
paint or input framework was added. See the whole-tree size ledger for the gap.

Expanded checkpoint validation at `5db17a2`: **all 199 frontend tests across 10
binaries passed, zero skipped**, run `090280fc-d0f9-45f3-919c-1111246b1c4f` (76.032s).
This includes the frontend integration binaries, not only App/library checks.
The narrow saved-ZIP render was also inspected. The Extract single-flight test
now finds its action by visible semantic label rather than an incidental slot
number; both extraction tests passed after that locator cleanup
(`9a3d28cf-467e-42ef-8772-bdfa2b379481`). Neither cleanup changes the size totals.
`origin/tau2` was re-fetched and remains `33f7d6f`. No integration merge, deployment,
full-workspace acceptance or physical Android/Windows QA is claimed.

## Composition/lifetime slice — September 29, 2026

Removed `ui/routes.rs`, all cached ancestor paths (including native/clipboard
paths), and routing-only Root/Workspace/Chat IDs. `Widget::owns` queries the real
retained owner and exact leaf, with source-record liveness for transcript rows and
attachment cards. Container-local workspace layout governs paint, input and
visibility; Chat's child order and Root's modal choice are also shared. Captured
movement/release traverses only its owner, not whichever opaque sibling it crosses.
There is no second tree, registry or serialized identity. Shared child visits
inherit one clip. Lost focus cancels preedit; abandoned captures also stop their
selection/scroll candidates.

Tick/Cancel traversal is explicitly broadcast rather than consumable input.
Workspace, overlays, sibling controls and menu ancestors all receive their updates;
hidden notices suspend deadlines, and hidden tooltip updates do not demand paints.
Durable operations and file-interest reconciliation remain outside paint.

Strengthened the existing attachment lifetime check to remove its source **without
a frame**, and added one real long-press/sibling-wheel regression. Both fail on
`18764eb` (`e0c86321-74f0-45ee-94de-7c20f5ca1edd`) and pass after the fix
(`3ddfacda-de8d-47ee-8422-bb0decb42088`). Removed the route-vector representation
assertion and synthetic ancestor-path test, not the actual source/IME/capture
checks. Removed Root's test-only card subtree and gallery paint/dispatch adapter;
Save/Extract checks now use the real attachment browser. The redundant
coordinate-based caption/tooltip check is gone.

Validation: all-target frontend compiler check and **198/198 frontend tests across
10 binaries, zero skipped** (`fce6f5eb-a8ac-4458-8c94-9f96a33286a6`, 76.644s).
Later import-only cleanup also compiler-checked. No device acceptance is claimed.

This correctness slice costs **104 raw / 123 same-format production lines**, while
removing **45 / 77 test lines**: total growth **59 / 46**, not a saving. The old
routing machinery is deleted, rather than wrapped, but exact ownership and
lifecycle checks cost more than its incomplete scope-only switch. Do not conceal
that cost: cumulative saving is now **244 / 198**, and the 5,000-line gate remains
open. 038 must remove actual repeated placement code; 039–041 still own removal
of the legacy projection/model pipeline.

## Event-boundary triage — September 29, 2026

User clarification: widget events should mostly be a thin adaptation of real
winit/platform input, not a general lifecycle/message bus. A synthetic variant
must have a specific need; traversal convenience or historical existence is not
sufficient. Do not introduce a new event framework to remove the old one.

| Current exception | Disposition / owner |
| --- | --- |
| `Submit` (only test emitters) | **Removed immediately under 042**, including production handlers and fixture selector; real controls exercise the two useful callers. |
| `Tick` | Existing broadcast is a correctness repair, not an architectural endorsement. **037 follow-up with 041 ownership work, before 043:** use an explicit non-consuming update call if it removes the synthetic dispatch path; preserve on-demand drawing, elapsed timing, independent siblings and completion/source checks. No new scheduler/registry. |
| `Cancel` | **037 follow-up with 041 ownership work, before 043:** consolidate actual owner cleanup, replacing the mixture of synthetic delivery and direct resets where unnecessary. Platform focus loss/touch cancellation/suspend and app-owned hide/source/navigation changes must still abandon interactions without activation. Review broad resize cancellation separately from true owner loss; no fabricated pointer-up or generic lifecycle bus. |
| `Hover` replay after layout | **Retain for now:** a stationary pointer must retarget when geometry/content moves. Reconcile against actual placed owners; this is not invented mouse movement or a new gesture vocabulary. |
| `Back` / targeted `Paste` | **Retain:** Android back and clipboard completion are real platform actions, not literal winit variants. Keep field/source/token fencing on clipboard completion. |

No offscreen-gesture signal exists: desktop cursor-leaving clears hover. The
cancellation entry points already live at the platform/App/owner boundaries;
there is no requirement to retransmit every reason as a new synthetic event.
