# 004 — Migrate to Sanscale's cleaned public API

Priority: before completing the editor fixes. Status: merged via frontend into tau2; device acceptance pending.

`git ls-remote origin refs/heads/master` confirmed published Sanscale commit
`8cc5afe833176a4fc71d1e5b8b97ad4952adfe40` (API lifetimes and explicit caret geometry).
Tau 2 previously pinned `2be4f316c3870d8cd9ba6e12b5e10b2af385bf2c`. Reconfirm the remote
when starting; pin the chosen full commit, do not rely on a moving branch.

Read Sanscale `public-api.md`, `api-review.md`, `decisions.md`, its editor examples
and lifecycle/API-contract tests. Migrate renderer/emoji/transient handles as well
as the shared editor. Preserve retained text, clipping, selection and IME behavior.
Compendium has related in-flight edits; inspect them but do not modify that work.

## Difficulties / upstream feedback

Record each concrete API gap/repro here with commit, symptom,
minimal example, local impact and proposed upstream change. Do not claim an
upstream bug from a Tau byte-caret/controller bug. Avoid duplicating Sanscale's
caret geometry. Validate workspace plus real GPU render and device input paths.

Integration: `tau2-sanscale-text-input` through `04fd60c` was fast-forwarded into
`tau2-rust-frontend`, then `tau2`. The frontend branch is retained. Included in beta 0.7.1 daemon/Windows delivery;
physical acceptance and any upstream limitations below remain open.

## Migration handoff

`tau2-sanscale-text-input` pins the full verified upstream SHA above in one
workspace dependency for frontend and Markdown. Migrated fallible font-chain
registration, path-based font reads, immutable transform updates and typed caret
queries. Existing Markdown stale-handle recovery is retained; the editor validates
its cached transient handle before reuse. No local geometry interpolation or
private SDK access. Real GPU screenshots include native emoji, selection, clipping
and the new prompt settings. Completed checks/limits are in frontend/QA.md.

### Concrete upstream limitation: intra-ligature carets

On `8cc5afe833176a4fc71d1e5b8b97ad4952adfe40`, shape `office` with the bundled
DejaVuSans.ttf, default features and a wide wrap. Repeated `next_caret_stop` yields
`[0, 1, 2, 4, 5, 6]`: no stop between the letters of the `fi` ligature. This is the
SDK's documented shaping-cluster granularity, not a Tau word-boundary bug. It
limits ordinary per-grapheme Left/Right inside ligatures. Post-edit placement
inside such a cluster also needs upstream geometry scrutiny before declaring
complete conventional editor semantics.

Tau keeps deletion grapheme-based: the regression test deletes `office` one
letter at a time, even when the font combines letters. Do not silently duplicate
caret geometry to work around motion. Proposed upstream follow-up: consistent
grapheme-level ligature stops across hit-testing, motion and caret rectangles,
using font caret data where available and an explicit fallback policy.

Other integration failures were Tau bugs, now fixed: settings hit-testing used
16px despite drawing at 15px; cursor keys synchronously saved unchanged drafts;
hit-testing could reveal a pending keyboard caret before using the displayed
viewport. These are not attributed to Sanscale.
