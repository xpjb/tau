# 008 — Restore click-origin glow/ripple

**September 29 triage:** ripple mechanics already exist. Finish shared control ownership, cancellation, clipping and visual acceptance under [037](037-retained-interaction-ownership.md); do not implement a second ripple path.

Priority: visual. Status: open.

Tau 1 had an extra circular glow expanding from the point clicked. Restore that
feedback in Tau 2, rather than a uniform pressed tint alone. Inspect the old native
client effect in the Tau 1 branch; do not import its UI/runtime framework.

Clip to the actual control/rounded surface, keep the click origin through reflow,
keep intensity restrained (005), and stop frame requests when the effect finishes.
Check click, touch, canceled presses, disabled controls and nested controls.
