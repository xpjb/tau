# 028 — One interaction scene and explicit gesture capture

Status: **Open; independent UI architecture topic, not in the first tool slice.**
Priority: **Separate pilot before broader input migration.**
Confidence: **7/10** in the value and appropriateness of this scope, not a probability of a LOC saving.
Net production LOC reduction estimate: **-150 to +250 lines**.

Recorded September 28, 2026. Evidence: the detailed `40a3698` investigation plus
relevant changes checked at `origin/tau2` = `cafef7f`. These are planning ranges,
not implemented or measured reductions. Positive means fewer lines; negative
means added code. Count changed production Rust across frontend, daemon and shared
crates; moving code is zero. Tests/docs/manifests are excluded unless explicitly
listed. See README for the confidence scale, baseline and overlap rules.

## Why / evidence

`frontend/src/app.rs` hit/area fields and `frame`, `hover`, `cursor`, `press`, `motion`,
`release`, `tick` long-press handling and `context_at`. Existing Markdown/editor
geometry is specialized and should be delegated to, not rebuilt.

Confidence rationale: Parallel hit registries and repeated overlay/gesture routing are
directly visible. Stable capture could simplify them, but a retained interaction model
itself needs substantial state and routing code.

## Proposed scope

Produce stable scoped interaction targets with layout geometry, clip/shape, stacking and
supported behaviors. Resolve press candidates, capture the recognized gesture, and
centralize overlay blocking/cancellation. Pilot a nested attachment surface, then
migrate the existing routing boundary only if the pilot proves the contract.

Out of scope: No new general GUI framework, renderer replacement, virtual DOM, or change
to deferred scroll-restoration architecture/tests in 012.

## Acceptance

- No click-through across overlays or rounded clips; nested controls retain precedence.
- Capture survives layout movement and cancels if its scoped target disappears; release
  does not retarget by stale coordinates.
- Preserve touch pan/pinch, scroll versus text selection, long press, keyboard focus,
  tooltips and independent ripple effects. Delegate text hit testing to existing
  specialists.

## Honest impact estimate

For the full routing-boundary migration, roughly 400–650 lines of scattered
registration/routing/cancellation could be replaced by 400–550 lines of
region/capture/policy code: -150 to +250 net. The initial single-surface pilot is likely
to add code while old routes remain; it cannot claim the full saving.

## Dependencies, overlap and current-source notes

Shares geometry/hit registration with 029, mechanics with 030 and owner types with
031/032. Count removed routing once. Existing hover/ripple requirements in 008/009
remain separate acceptance work.

cafef7f adds notice-click navigation and shared attachment control-panel geometry. Reuse
those and preserve their behavior; they are already implemented, not deletion credit.

No implementation, runtime/device certification or Rust test run is part of this
backlog entry. Work only with managed Cargo compiler checks and relevant nextest
coverage when implementation is authorized. Preserve existing deferred tasks.
