# 037 — One retained control and child-traversal contract

Status: **Selected.** Depends on 036's paint-order contract. Absorbs unfinished
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
