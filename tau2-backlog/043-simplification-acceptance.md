# 043 — Completion means correct behavior and measured removal

Status: **Practical source closeout/integration authorized; original size and physical gates remain unmet.** The user explicitly requested local/justified deletions, remaining work triaged into backlog, and integration including the pause fix. This supersedes holding the source merge for the original 5,000-line goal, not its accounting or the open device checks. See [044](044-closeout-and-followups.md). The criteria below remain the original acceptance contract, not a claim that all passed.

## Source/deletion gates

- `app/projection.rs`, rich Row → wrapper assembly, inverse display-key decoding,
  paint/register control bypasses, root descendant switches and the test-only
  global UI compatibility layer are absent, not renamed or hidden behind flags.
- One native semantic contract, one message reconciliation owner and actual
  retained UI owners. Necessary bounded ID/height/body indexes are not independent
  editable models. No second renderer/editor/replica framework.
- No legacy update/page protocol-shaped adapter retained solely for tests/demo;
  live native startup/sparse/history behavior still works.
- No unrelated behavior removed to manufacture savings. Preserve current file
  picker, code references, saved/export/extract actions, drafts, source safety,
  on-demand rendering and bounded background sync.

## Size gate

Use the [pinned baseline and method](../docs/reviews/retained-ui/completion-size.md).
All tracked frontend Rust, production and tests: **27,725 → ≤22,725 raw code lines**.
Under the fixed formatter: **33,018 → ≤28,018**, before charging added code outside
frontend. Both must hold. Count all touched shared/daemon/platform code and any
new test/helper/generated replacement; relocation/minification is no reduction.

Report production and tests independently. Working allocation is roughly 3,000
production + 2,000 tests; the split is a planning budget, not an audited forecast
or license to achieve the entire reduction by deleting tests. Show the final
production reduction and explain changed ownership, not just a passing total.

After each slice publish: old path deleted; replacement added; net production/test
code delta; outside-frontend charge; cumulative reduction; gap to 5,000. Use one
ledger and count each deletion once. If the required reduction is unsupported or
not reached, say **not finished** and revise the next slice. Do not widen into
operation-store/transport migrations solely to chase the count or quietly waive it.

## Focused behavioral acceptance

Use surviving tests, not another exhaustive UI test campaign:

- Mixed-content opaque overlays and nested/floating controls render in paint order.
- One target owns visual/logical feedback; capture, clip, disabled blocking,
  scroll takeover, hide/detach and pointer isolation are correct.
- Stationary-pointer autoscroll extends selected/copied text after layout.
- Streaming/prepend/expansion/navigation and pending→confirmed handoff preserve
  identity, authored text and reading position with bounded live children/content.
- Shared controls/forms remain responsive and settle idle. IME/paste/async results
  cannot write a replaced owner; replay/reconnect never duplicate effects.
- Existing native integration/recovery tests cover actual transport/model contracts,
  not the retired fixture adapters. 013/014 remain separately reported until fixed.

Use managed Cargo compiler checks and relevant nextest suites per slice. At the
end run workspace all-target checks and the surviving full nextest suite; cross-check
Windows/Android and Java when affected, with relevant rustdoc checks. No Clippy or
Cargo built-in test runner. Do not report historical green runs as fresh validation.

Physical Windows/DPI and Android touch/IME checks remain necessary: overlays,
folder/background/hover/ripple, shared editing, nested scrolling, selection and
code/attachment navigation. Reuse existing device QA rather than adding a test for
every coordinate. Without a device, report source completion separately and keep
physical acceptance open. Packaging, deployment and service restarts need separate
authorization; this task does not grant it.
