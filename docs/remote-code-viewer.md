# Remote code viewer (development)

Read-only files, launched beside View attachments. Directory list and code buffer
share one surface on desktop and Android. CWD is the initial location, not a
sandbox. Parent navigation and explicit symlink traversal are supported.

## Ownership and boundaries

- `tau-code-viewer`: stable line/paragraph identities, selection reconciliation,
  syntax paint, fuzzy path ranking, and an optional daemon filesystem service.
- The daemon owns shared, bounded `.gitignore`-aware indexes. It validates the chat
  before filesystem access. File content is ephemeral, never inserted into the
  transcript database or attachment store merely because it was viewed.
- The existing authenticated native connection carries finite, credited,
  hash-verified filesystem responses. Navigating/closing cancels only the viewer's
  stream. Requests and results are generation/chat/source scoped.
- Live refresh runs only for the open viewer. Unchanged lines retain identity;
  references are recomputed after insertions above a selection. If selected text
  changes, selection is invalidated instead of silently targeting other code.
- The existing composer appears only for a line selection. A reference is inserted
  into its saved draft, never automatically sent. Other draft text is preserved.
- Ctrl+Space / an on-screen Find action opens path search. No content search or
  editing in this first cut. Gutter tap/drag and touch hold/drag select line ranges.

Compiler, nextest, real headless layout/input and cross-target checks will be
recorded here. This branch is not a deployment or physical-device acceptance.
