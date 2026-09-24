# Tau 2 backlog

User reports from September 23, 2026. Keep one issue per file. Record facts,
acceptance checks, related work and remaining device checks; do not mark an item
complete just because code compiles.

| Item | Priority | Status |
| --- | --- | --- |
| 001 Daemon settings and hierarchical prompts | First | Beta daemon/Windows delivered |
| 002 Direct model selection without a catalog gate | With 001 | Beta daemon/Windows delivered |
| 003 Text input keyboard navigation | Editor worktree | Merged via frontend into tau2; device QA pending |
| 004 Sanscale API migration | Editor worktree | Pinned/migrated; ligature limitation recorded |
| 005 Highlight colour | Visual | Open |
| 006 Caret response | Editor worktree | Hot path fixed/measured; physical latency pending |
| 007 Text input scrolling | Editor worktree | Merged via frontend into tau2; device QA pending |
| 008 Click-origin ripple | Visual | Open |
| 009 Nested tool hover feedback | Visual | Open |
| 010 Account usage remaining | Context hover | Open |
| 011 Title generation model and prompt | With 001 | Beta daemon/Windows delivered |
| 012 Chat scroll-position architecture and test removal | Later | Open; quick fix provisional, audit deferred |

Settings and the composer must reuse the existing shared editor. Do not create
another text controller. See 004 for the pinned source and migration difficulties.
Work stays on Tau 2. Do not replace stable Tau or restart a daemon hosting an active
conversation to deploy its own update. New settings fields require matched clients.

## Scope correction

Project overrides were not requested. Do not treat their existing implementation
as an accepted requirement. Item 001 has exactly two levels:
**model override → default system prompt**. There is no provider, project or
built-in fallback layer. Empty default/override text is intentional.

## Settings handoff — implementation `15d2a69`

User requested a stop after important settings work and assigned the editor to
another worktree. No editor/Sanscale code was changed here.

- Settings schema 2, protocol 13, client/daemon version 0.7.1 (unshipped).
- Exactly model override → saved default prompt. Default and model overrides can
  both be empty. No project/provider/built-in prompt fallback and no append layer.
- Settings → Daemon settings → Prompts edits the default and any exact model ID.
  A missing `agent.modelSystemPrompts` key inherits; a string, including `""`, overrides.
- Optional `daemon.titleModel`; unset follows the chat model. Titles remain native.
- Model IDs no longer require membership in metadata. Provider errors reach the
  chat. Unknown capacity stays unknown; no threshold-based auto-compaction then.
- Workspace check and all **51 nextest tests passed**. Scoped daemon/frontend/
  protocol Clippy with warnings denied passed. Native debug client/daemon build
  passed. The full workspace Clippy gate remains blocked by existing untouched
  Markdown lints; flag `896a973b-1da6-4680-b894-c147eeb7d6fc` records this.
- Real settings-controller/daemon/local-provider tests cover empty/default/custom
  prompts, both provider formats, model switches, compaction, restart, settings
  conflicts, direct IDs, provider rejections, title models and draft preservation.
- No new GUI/device acceptance or release packages. Do not claim those passed.
- No production settings, daemon restart, history import or paid model request.
  The existing settings reader preserves old default text without rewriting files
  on load. The live Astra override still needs `""` set at the matched beta update.

The code commit is on `tau2/daemon-settings-prompts`, based on `tau2-integration`.
Stable `master` and other worktrees are unchanged. Keep the paired daemon/client
protocol change together when integrating with editor work. Deploy only after an
idle, coordinated beta update; do not interrupt the active conversation.


## Beta 0.7.1 delivery

The integrated settings and shared-editor code was deployed to beta and delivered
as the matched Windows x64 installer on 2026-09-24. Android was not rebuilt for
this request. Device acceptance, physical latency and documented SDK limitations
remain pending; deployment does not close those checks. See ../INTEGRATION.md.
