# 034 — Shared per-test headless UI fixtures

Kind: **Independent supporting cleanup — test-only.**
Group: **Test tooling**; does not depend on a runtime architecture migration.

Status: **Open; test-only maintenance task.**
Priority: **Low risk, independent of runtime architecture.**
Confidence: **9/10** in the value and appropriateness of this scope, not a probability of a LOC saving.
Net production LOC reduction estimate: **0 lines**.
Test-only LOC reduction estimate: **+100 to +250**.

Recorded September 28, 2026. Evidence: the detailed `40a3698` investigation plus
relevant changes checked at `origin/tau2` = `cafef7f`. These are planning ranges,
not implemented or measured reductions. Positive means fewer lines; negative
means added code. Count changed production Rust across frontend, daemon and shared
crates; moving code is zero. Tests/docs/manifests are excluded unless explicitly
listed. See README for the confidence scale, baseline and overlap rules.

## Why / evidence

`frontend/src/app/{editor_tests,project_tests,icon_controls_tests,download_interaction_tests,viewer_tests,composer_status_tests,connection_tests,tooltip_tests}.rs`
and similar headless setup sites. Existing specialized download fixtures should be
reused, not reimplemented.

Confidence rationale: Multiple test modules visibly repeat temporary store, real
GPU/context, App construction and demo seeding. A per-test harness can remove
scaffolding without removing assertions.

## Proposed scope

Create a test-only fixture with explicit dimensions/mobile mode/seeding options and
owned temporary directory/App/headless context. Keep specialized scenario setup in each
test. Migrate repeated generic setup/frame/input helpers without a global mutable GPU or
app singleton.

Out of scope: No production runtime abstraction, screenshot/coverage deletion or global
shared mutable fixture.

## Acceptance

- Keep all meaningful assertions, source-binding scenarios and desktop/mobile coverage.
  Preserve per-test isolation and deterministic ownership.
- Tests run through the managed nextest path and retain real layout/fonts/storage
  behavior; no mock replacement merely to make tests faster.
- Leave the explicitly deferred scroll tests alone until 012 is taken up. Do not count
  deleting tests as simplification.

## Honest impact estimate

Production reduction is exactly 0 by scope. Test-only estimate: +100 to +250 lines,
based on roughly 180–300 repeated fixture/helper lines replaced by a 50–80-line common
harness and calls. This is scaffolding reduction only; new assertions remain welcome.

## Dependencies, overlap and current-source notes

Porting legacy fixture inputs under 035 is a different change. If both touch setup,
count shared harness deletion once. Never add test-only savings to the production
totals.

cafef7f adds navigation tests and expands download coverage. Reuse their established
behavior and do not delete those new assertions to meet the estimate.

No implementation, runtime/device certification or Rust test run is part of this
backlog entry. Work only with managed Cargo compiler checks and relevant nextest
coverage when implementation is authorized. Preserve existing deferred tasks.
