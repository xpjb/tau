# Tau 2 backlog

User reports from September 23, 2026. Keep one issue per file. Record facts,
acceptance checks, related work and remaining device checks; do not mark an item
complete just because code compiles.

| Item | Priority | Status |
| --- | --- | --- |
| 001 Daemon settings and hierarchical prompts | First | In progress; two-level scope confirmed |
| 002 Direct model selection without a catalog gate | Next; shares 001 settings | In progress |
| 003 Text input keyboard navigation | Next editor work | Open |
| 004 Sanscale API migration | Before 003/006/007 | Open; upstream verified |
| 005 Highlight colour | Visual | Open |
| 006 Caret response | Editor | Needs measurement |
| 007 Text input scrolling | Editor | Open |
| 008 Click-origin ripple | Visual | Open |
| 009 Nested tool hover feedback | Visual | Open |
| 010 Account usage remaining | Context hover | Open |
| 011 Title generation model and prompt | With 001 | In progress |

Settings and the composer must reuse the existing shared editor. Do not create
another text controller. See 004 for the pinned source and migration difficulties.
Work stays on Tau 2. Do not replace stable Tau or restart a daemon hosting an active
conversation to deploy its own update. New settings fields require matched clients.

## Scope correction

Project overrides were not requested. Do not treat their existing implementation
as an accepted requirement. Item 001 has exactly two levels:
**model override → default system prompt**. There is no provider, project or
built-in fallback layer. Empty default/override text is intentional.
