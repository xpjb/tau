# 001 — Daemon settings and hierarchical system prompts

Priority: first. Status: in progress.

## Report and confirmed facts

Astra must use an explicitly empty replacement prompt. Tau 1 has a zero-byte
`model-prompts/openai-codex/gpt-6-astra.md`. The native importer missed that file.
Tau 2 has revisioned JSON settings, separate from SQLite conversations. Its existing
settings UI already shares `frontend/src/editor.rs` with the composer. Global and
project prompts exist; provider/model prompt overrides do not.

## Native design

- One settings document and the existing compare-and-swap save. No Pi worker,
  extension, live file mirror or new settings database.
- Replacement fallback: exact model → nearest matching project with a replacement
  → provider → global → built-in. This preserves Tau 1's model-first rule.
- `null` inherits; `""` is an intentional replacement that stops fallback. Never
  trim prompt text or treat an empty replacement as missing.
- Append global, provider, matching projects from outer to inner, then model text.
  AGENTS.md and the working directory remain separate context. Tools remain tools.
- Provider/model/project targets use the existing settings form and shared editor,
  with an inherit/custom toggle and a multiline replacement/append field. Advanced
  JSON still exposes every native runtime setting. Host bind addresses, filesystem
  locations and credentials remain deployment/security configuration, not prompts.
- Models are optional metadata/prompt overrides, not an allowlist (002). A missing
  context window stays unknown; no made-up capacity or automatic compaction limit.
- The existing explicit, first-run Tau 1 importer may read old prompts once. Native
  operation never reads the Pi directory. Existing installations need an explicit
  settings repair during an idle, matched-client beta upgrade.

## Acceptance

Exercise the real settings wire, stale-save protection, restart, provider payloads,
empty versus null, model switching, nested projects and parent fallback. Check that
Astra's empty override removes replacement boilerplate but keeps tools and chosen
append/project context. Test Codex and Chat Completions. Check prompt lengths and
invalid targets. Show the target and fallback order in the actual menu.

Leave editor/Sanscale work in 003–007. Leave active production services unchanged.
