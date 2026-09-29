# 037 — One retained control and child-traversal contract

Status: **In progress. Control migration implemented; composition/lifetime slice next.**
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
