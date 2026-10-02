# Remote code viewer previews

These are **actual headless GPU frames** of the shared Rust UI, not design mockups
or physical-device screenshots. Fixtures are synthetic and offline; no real chat,
filesystem content, token or provider is used.

- `desktop.png`: directory/code surface without a composer, syntax and line gutter.
- `phone-comment.png`: 360dp phone, gutter range selection and the existing composer.
- `search.png`: shared editor and fuzzy path results.

Additional directory, code, comment and 2.5x phone frames are produced by:

```sh
TAU_CODE_PREVIEW_DIR=/tmp/tau-code-previews \
  /usr/local/bin/cargo nextest run --locked -p tau-frontend \
  -E 'test(code_view::tests)'
```

The tests also exercise touch hold/haptic dispatch, drag/undrag, navigation,
composer visibility, live selection/reference reconciliation, IME composition,
source/chat fencing and preservation of newer saved draft text.

See `docs/remote-code-viewer.md` for ownership, bounds and remaining device QA.

## Local fuzzy picker (protocol 22, not deployed)

- `fzf-desktop.png`, `fzf-phone.png`, `fzf-phone-2x.png`: 240 matching paths,
  independent out-of-order fuzzy terms, matched-character colour, hidden toggle,
  and selected-file syntax preview below the list.
- Reproduce with `TAU_CODE_PREVIEW_DIR=/tmp/code-previews` and managed
  `cargo nextest run -p tau-frontend -E 'test(code_view::tests)'`.
  The fixture writes `desktop-fzf.png`, `phone-fzf.png`, `phone-2x-fzf.png`.
- These are rendered headless UI fixtures, not physical-device acceptance or
  evidence of a deployment. See the current `docs/remote-code-viewer.md` handoff.
