# Completion size baseline

Measured September 29, 2026 from tracked source at
`33f7d6f6b45533e081e5021b0c3f69b1e045b6f9`. This is the **whole frontend**, unlike
the earlier retained-rewrite/touched-files ledger. All Rust includes build.rs,
unit and integration tests, and both platform hosts. Nothing was reformatted in
tracked source. This measurement is not a deletion forecast.

| Rust code lines (tokei, excludes comments/blanks) | Raw | Same format |
| --- | ---: | ---: |
| Production | 19,174 | 21,584 |
| Test-only, including helpers/module declarations | 8,551 | 11,434 |
| **Total** | **27,725** | **33,018** |
| **Required final ceiling before outside-code charges** | **22,725** | **28,018** |

For context, raw all-language frontend code is 28,662. Do not move Rust logic into
Java, generated files or another crate and call that a reduction. Charge added
replacement code outside frontend and report interop changes. Scope expansion is
not a way to count unrelated existing code deletions toward this requirement.

Useful **physical**, comments/blanks-included inspection areas:

- All frontend Rust: 28,707 = 19,777 production + 8,930 tests.
- App/UI (`src/app.rs` and `src/app/**`): 10,399 production + 4,744 tests.
- `app/projection.rs`: 307; `app/test_ui.rs`: 345; `app/scroll_tests.rs`: 199.
  These are gross removals, not net savings: their necessary replacements count.
- The older rewrite ledger's 2,640-line form/overlay and 1,324-line transcript
  buckets include real behavior; those entire buckets are not deletable overhead.

Working allocation: roughly 3,000 production + 2,000 test lines net. This is not
supported by a sum of independently proven opportunities yet. The model backlog's
small/overlapping estimates cannot close that gap on paper. Measure after each
closed slice and revise a growing design; do not declare completion below target.

## Reproduce

Tools used: `tokei 12.1.2`; `rustfmt 1.10.0-nightly (ad3d0bc141 2026-07-31)`.
Fixed formatting: edition 2024, width 120, Max small heuristics, skip_children.
Reuse the existing frontend-local cfg(test) scanner in
`scripts/retained-ui-size.py`. Its module-suffix/item assumptions must be checked
again if future test organization changes; it is not a general Rust parser.
The test count is total minus production, not a filename-only guess.

Run from the repository root, passing the same baseline or a future candidate:

```sh
python3 - 33f7d6f <<'PY'
import json, pathlib, runpy, subprocess, sys, tempfile
m = runpy.run_path('scripts/retained-ui-size.py')
rev = sys.argv[1]
paths = [p for p in m['git']('ls-tree', '-r', '--name-only', rev, 'frontend').splitlines()
         if p.endswith('.rs')]
with tempfile.TemporaryDirectory() as tmp:
    root = pathlib.Path(tmp)
    for path in paths:
        text = m['source'](rev, path)
        prod = '' if m['test_only'](path) else m['production'](text)
        values = {'all': text, 'production': prod,
                  'normalized_all': m['formatted'](path, text),
                  'normalized_production': m['formatted'](path, prod)}
        for name, value in values.items():
            dest = root / name / path
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_text(value)
    result = {}
    for name in ['all', 'production', 'normalized_all', 'normalized_production']:
        data = json.loads(subprocess.check_output(
            ['tokei', str(root / name), '--output', 'json'], text=True))
        result[name] = data['Rust']['code']
    print(json.dumps(result, indent=2))
PY
```

Also run the existing change ledger for the candidate:
`python3 scripts/retained-ui-size.py 33f7d6f <candidate>`.
Its normalized **production** deltas complement the whole-tree count above;
its test column is physical, not normalized, and it does not count untouched
non-UI files. Do not substitute that number for the new whole-frontend baseline.

For any non-frontend implementation changes, compare all tracked source in the
changed crates/hosts at both revisions, using the same formatter/counter. Show
production/tests separately with reviewed test boundaries, and subtract net growth
from the frontend saving. Moving 300 lines out and adding 300 elsewhere earns zero.
Include new crates/languages/helpers and deleted callers; no off-tree accounting.
Record formatter/counter versions and retain the same versions for both sides.

## Per-slice ledger

After 036–038, the first 039 interest fix and the independent 042 pruning below,
raw frontend code is **27,320**; same-format code is **32,618**. Production is
**19,103 / 21,547** and tests are **8,217 / 11,071** (raw / same-format).
Net reduction is **405 / 400**; remaining gap is **4,595 / 4,600**. Production
saving is only **71 / 37** so far: the later architectural deletions must deliver
substantially more. These are intermediate results, not completion.
Deltas below use reduction-positive values and show raw / same-format:

| Slice / commit | Deleted path | Replacement cost | Net production reduction | Net test reduction | Outside charge | Cumulative / remaining |
| --- | --- | --- | ---: | ---: | ---: | --- |
| 036 | Unordered surface batching; Move-only selection; icon/centering tests | Ordered surface boundaries + three behavior checks | −44 / −46 | +44 / −24 | 0 | Reduction 0 / −70; gap 5,000 / 5,070 |
| 037 controls | Paint/register bypasses; renderer input state; per-row feedback; Debug keys; redundant hover/gallery tests | One retained Button/Control feedback path, explicit styles/cursor, typed equality, consumed/action pair, compact checks | +137 / +129 | +166 / +185 | 0 | Reduction 303 / 244; gap 4,697 / 4,756 |
| 037 composition/lifetime | Root descendant routes, cached paths/routing IDs, separate fixture card tree and redundant path/gallery tests | Exact owner/leaf validation, local layout/order, explicit lifecycle broadcast, two before/after checks | −104 / −123 | +45 / +77 | 0 | Reduction 244 / 198; gap 4,756 / 4,802 |
| 038 forms | Floating-label path; repeated field/footer/page placement; fixed-field/choice Vec rebuilding; bespoke suggestion controls; redundant form tests | Shared concrete placement helpers, compact IME layout and explicit form-action feedback | +55 / +67 | +14 / +24 | 0 | Reduction 313 / 289; gap 4,687 / 4,711 |
| 039 viewport ownership (`55c11f0`) | Reverse display-key parser and tool-root rescan | Direct owner on existing detail lines; real viewport/native-body regression using shared fixture | +22 / +5 | −40 / −70 | 0 | Reduction 295 / 224; gap 4,705 / 4,776 |
| 042 independent pruning / Submit retirement | Project activity/layout matrix, repeated GPU persistence setup, status glyph matrix, test-only Submit event and handlers | Controller persistence check, one isolated status render, real pointer submissions | +5 / +5 | +105 / +171 | 0 | Reduction 405 / 400; gap 4,595 / 4,600 |

Keep raw and same-format totals alongside the ledger. A deletion shared by two
slices is counted once. Documentation and this measurement recipe are not runtime
savings. The baseline measurement was documentation-only; implementation checks
are recorded with each owning backlog item. The protocol equality derive changes
neither raw (442) nor same-format (527) code lines in `protocol/src/lib.rs`.
