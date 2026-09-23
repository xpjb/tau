# 001 — Daemon settings and hierarchical system prompts

Priority: first. Status: in progress.

## Report and confirmed facts

Astra must use an explicitly empty replacement prompt. Tau 1 has a zero-byte
`model-prompts/openai-codex/gpt-6-astra.md`. The native importer missed that file.
Tau 2 has revisioned JSON settings, separate from SQLite conversations. Its existing
settings UI already shares `frontend/src/editor.rs` with the composer. Global and
project prompts exist in the code, but project overrides were not requested.
Their presence is not a requirement. Model prompt overrides are the priority.

## Native design

- One settings document and the existing compare-and-swap save. No Pi worker,
  extension, live file mirror or new settings database.
- Replacement fallback: exact model → daemon default → built-in.
- Provider-level defaults need user confirmation before adding that layer.
- No project concept, project settings section or project prompt override layer.
  The earlier plan carried forward an unwanted implementation detail; that scope
  decision is withdrawn before runtime implementation.
- `null` inherits; `""` is an intentional replacement that stops fallback. Never
  trim prompt text or treat an empty replacement as missing.
- Any existing append text and filesystem instructions remain separate from the
  replacement fallback; do not invent a project settings system for them. Tools
  remain tools.
- Model targets use the existing settings form and shared editor,
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
empty versus null, model switching and daemon-default fallback. Check that
Astra's empty override removes replacement boilerplate but keeps tools.
Test Codex and Chat Completions. Check prompt lengths and
invalid targets. Show the target and fallback order in the actual menu.

Leave editor/Sanscale work in 003–007. Leave active production services unchanged.
