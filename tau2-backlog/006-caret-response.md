# 006 — Check caret response speed

Priority: editor. Status: hot path fixed/measured; physical latency QA pending. Related: 003/004.

The user suspects slow caret movement but is uncertain. Measure key-repeat input,
redraw/wake latency, shaping work and blink reset in the composer and prompt editor.
Compare the Sanscale editor example; preserve on-demand idle rendering. Do not add
caret smoothing or a polling timer to mask missed redraws.

Acceptance: distinguish key repeat delay from draw latency, verify immediate caret
feedback and solid caret after input, record measurements and remaining device QA.

Integration: `tau2-sanscale-text-input` through `04fd60c` was fast-forwarded into
`tau2-rust-frontend`, then `tau2`. The frontend branch is retained. Included in beta 0.7.1 daemon/Windows delivery;
physical acceptance and any upstream limitations below remain open.

## Editor handoff

Cursor-only keys no longer save unchanged drafts to synchronous SQLite. Cached
layout/placed-caret queries avoid reshaping on warm motion; a SQLite trigger and
SDK work counters verify both. App input dirties an on-demand frame immediately;
idle remains idle and the existing solid caret is retained. Measured App-key and
completed headless-frame timings, with explicit OS/display exclusions, are in
frontend/QA.md. Physical repeat-to-display latency is still unmeasured.
