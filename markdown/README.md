# tau-markdown

A companion component extracted from the Markdown editor example in
[xpjb/sanscale](https://github.com/xpjb/sanscale), revision
`2be4f316c3870d8cd9ba6e12b5e10b2af385bf2c`:

- `examples/markdown-editor/markdown/*` → `src/markdown/*`
- `examples/markdown-editor/preview.rs` → `src/preview.rs`

The parser owns a Rope, stable line/cell identities, incremental changes and
projected source mappings. The Sanscale view owns layout/paint snapshots and
visible scenes. Neither component owns a window, network connection, link
activation or image loading. Fonts are consumer-supplied. It is extracted here
so Tau's build is pinned/self-contained and the backend agent need not coordinate
a second repository change. It can be moved upstream into a companion crate later.

Local additions: passive authored-link destination metadata, projected selection
rectangles and plain-text copy, with a source-map/link-edit regression. Activation
and allowed URL schemes belong to the app. The upstream CPU/differential tests
were retained; adapter tests use Tau's bundled, licensed deterministic fonts
instead of system font discovery. No demo editor/fontdb/window dependency remains.

See [the preserved upstream design/dialect notes](src/markdown/README.md) for
supported syntax, incremental work bounds and remaining limitations. Statements
there about being "example-local" describe its origin, not this crate's packaging.
This is still a young API, not a standards-conformance claim.

```rust,ignore
let mut doc = tau_markdown::Document::new("");
let mut view = tau_markdown::Preview::new(unique_namespace);
doc.append(chunk)?;
view.sync(&doc, &mut text, faces, theme, width, font_size);
let scene = view.scene(&mut text, &doc, viewport, scroll);
// Draw scene.under, scene.draws, scene.over using your own pass and clipping.
view.release(&mut text); // before discarding a retained view
```

MIT OR Apache-2.0, inherited from Sanscale; both license texts are included.
