# 005 — Reduce the yellow/strong highlight

**September 29 triage:** visual acceptance is part of [037](037-retained-interaction-ownership.md), alongside single-owner hover/press and ripple. Do not solve compounded parent/child highlights with palette changes alone.

Priority: visual. Status: open.

The user reports a highlight that is too yellow and too strong relative to its
base colour. Check both text selection and hover/pressed surfaces in the running
UI; identify the exact affected layer rather than changing every palette entry.
Compare Tau 1. Use a subdued tint derived from the underlying surface, preserve
readable text and keyboard focus, and check sRGB-to-linear handling.

Acceptance: screenshots at rest, hovered, pressed and selected on dark/light
message surfaces; user visual approval. No automated panel-layout tests.
