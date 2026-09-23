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
- Exactly two levels: model override → default system prompt. No provider or
  built-in fallback layer. The default is ordinary saved text, including empty text.
- No project concept, project settings section or project prompt override layer.
  The earlier plan carried forward an unwanted implementation detail; that scope
  decision is withdrawn before runtime implementation.
- A missing model override inherits; `""` is an intentional override. Never
  trim prompt text or treat an empty replacement as missing.
- No separate append-prompt setting. Existing nonempty global append text can be
  included in the saved default text. AGENTS.md loading and working-directory
  context stay separate from prompt selection. Tools remain tools.
- Model targets use the existing settings form and shared editor,
  with an inherit/custom toggle and a multiline prompt field. Advanced
  JSON still exposes every native runtime setting. Host bind addresses, filesystem
  locations and credentials remain deployment/security configuration, not prompts.
- Models are optional metadata/prompt overrides, not an allowlist (002). A missing
  context window stays unknown; no made-up capacity or automatic compaction limit.
- Set Astra’s native model override to `""` when the matched beta update is applied.
  No new migration system. Native operation never reads the Pi prompt directory.

## Acceptance

Exercise the real settings wire, stale-save protection, restart, provider payloads,
empty versus null, model switching and daemon-default fallback. Check that
Astra's empty override removes replacement boilerplate but keeps tools.
Test Codex and Chat Completions. Check prompt lengths and
invalid targets. Show the target and fallback order in the actual menu.

Leave editor/Sanscale work in 003–007. Leave active production services unchanged.
