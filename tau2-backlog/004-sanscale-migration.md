# 004 — Migrate to Sanscale's cleaned public API

Priority: before completing the editor fixes. Status: open; source verified.

`git ls-remote origin refs/heads/master` confirmed published Sanscale commit
`8cc5afe833176a4fc71d1e5b8b97ad4952adfe40` (API lifetimes and explicit caret geometry).
Tau 2 currently pins `2be4f316c3870d8cd9ba6e12b5e10b2af385bf2c`. Reconfirm the remote
when starting; pin the chosen full commit, do not rely on a moving branch.

Read Sanscale `public-api.md`, `api-review.md`, `decisions.md`, its editor examples
and lifecycle/API-contract tests. Migrate renderer/emoji/transient handles as well
as the shared editor. Preserve retained text, clipping, selection and IME behavior.
Compendium has related in-flight edits; inspect them but do not modify that work.

## Difficulties / upstream feedback

Not attempted yet. Record each concrete API gap/repro here with commit, symptom,
minimal example, local impact and proposed upstream change. Do not claim an
upstream bug from a Tau byte-caret/controller bug. Avoid duplicating Sanscale's
caret geometry. Validate workspace plus real GPU render and device input paths.
