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
