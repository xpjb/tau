# Native Tau frontend — 0.7.0 beta

Chad + Sanscale + incremental Markdown, speaking shared protocol 12 directly to the
native Rust/SQLite daemon. Integrated on `tau2`; `tau2-rust-frontend` preserves the
pre-integration snapshot. There is no preview protocol bridge in the release path.

Windows uses the native per-user **Tau Beta installer**, not a bare EXE/ZIP or JVM.
Android uses `app.tau.rust`, a thin Java OS bridge, and a signed ARM64 APK; the same
beta key/package identity permits updates. Stable Tau remains separately installed.

```sh
# Managed Cargo, one job/rayon thread; run builds sequentially.
cargo check --locked --workspace --all-targets
cargo nextest run --locked --workspace
scripts/build-windows-sfx.sh
ANDROID_ABI=arm64-v8a frontend/android/build.sh
```

Desktop development can seed connection settings with `TAU2_SERVER` and `TAU2_TOKEN`;
`TAU2_DATA_DIR` isolates local data. `target/debug/tau --screenshot PATH [--phone]`
renders the explicitly offline demo through the real GPU shader path.

## UI and ownership

- Exact saved/live replacement and history cuts; local drafts survive restart.
- Unknown sends remain visible and are never automatically replayed.
- Per-section first-observed timestamps; Details copies only its own content.
- Estimated provider cache TTL rings, independent of runtime idle policy.
- Actual heartbeat RTT/endpoint health; contextual, target-bound chat controls.
- New chats use the last explicit model choice. Quick favorites never set that default.
- Settings → Daemon / agent settings edits the full revisioned document. Save uses
  CAS, keeps edits on conflict, distinguishes built-in/null from custom empty prompts,
  and never exposes provider secrets.
- Native model/thinking/compact/priority commands; no extension dialog protocol.
- Verified native transfers, attachment viewing/export and native clipboard/IME.

Default quick favorites: Codex `gpt-6-luna`, `gpt-6-sol`, `gpt-6-astra`, and OpenRouter
`deepseek/deepseek-v4.1-flash`. They are checked against the real catalog; unavailable
entries are disabled, not invented capabilities or fallback model choices.

[QA](QA.md) · [Packaging](PACKAGING.md) · [Architecture](../ARCHITECTURE.md) ·
[Daemon/storage](../docs/tau2-agent.md)


## Shared text input

Composer and settings/dialog fields use the same editor. Arrows follow Sanscale's
visual lines; Up/Down retain a goal column. Ctrl+arrows and Ctrl+Backspace/Delete
use word boundaries; Home/End use visual lines (Ctrl selects document edges),
PageUp/Down use the field viewport, and Shift extends selection. Undo/redo,
clipboard and desktop IME share the same edit path. Backspace/Delete remove one
Unicode grapheme, not a whole font ligature.

Wheel over a field scrolls that field, independently of the transcript. Shift+wheel
scrolls horizontally; single-line fields also scroll horizontally. Drag selection
autoscrolls at field edges. Navigation/editing reveals the caret; idle rendering
does not reset manual scrolling. Android keeps its native EditText/IME bridge.

Migration evidence and remaining SDK/device limitations: `QA.md` and
`../tau2-backlog/004-sanscale-migration.md`.
