# Completion size baseline

Measured September 29, 2026 from tracked source at
`33f7d6f6b45533e081e5021b0c3f69b1e045b6f9`. This is the **whole frontend**, unlike
the earlier retained-rewrite/touched-files ledger. All Rust includes build.rs,
unit and integration tests, and both platform hosts. Nothing was reformatted in
tracked source. This measurement is not a deletion forecast.

| Rust code lines (tokei, excludes comments/blanks) | Raw | Same format |
| --- | ---: | ---: |
| Production | 19,194 | 21,620 |
| Test-only, including helpers/module declarations | 8,531 | 11,398 |
| **Total** | **27,725** | **33,018** |
| **Required final ceiling before outside-code charges** | **22,725** | **28,018** |

For context, raw all-language frontend code is 28,662. Do not move Rust logic into
Java, generated files or another crate and call that a reduction. Charge added
replacement code outside frontend and report interop changes. Scope expansion is
not a way to count unrelated existing code deletions toward this requirement.

Useful **physical**, comments/blanks-included inspection areas:

- All frontend Rust: 28,707 = 19,805 production + 8,902 tests.
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
Reuse the frontend-local cfg(test) scanner in
`scripts/retained-ui-size.py`. Its item-boundary assumptions must be checked
again if future test organization changes; it is not a general Rust parser.
The test count is total minus production, not a filename-only guess. The classifier
now removes each cfg(test) item individually: production export handling follows
an inline test module in app/attachments.rs. This corrects the old suffix shortcut,
reclassifying 20 raw / 36 normalized frontend lines and 33 / 53 daemon/protocol
lines at **both** baseline and candidate. Whole-tree totals and net savings are
unchanged; the absolute production/test splits below are corrected. Inline and
external test-module examples were checked against this rule.

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

At the bounded simplification closeout, before the independent pause fix,
frontend Rust is **26,093 raw / 32,357 same-format**: production **18,587 / 21,851**,
tests **7,506 / 10,506**. Outside growth remains **62 / 97** (production **26 / 60**,
tests **36 / 37**). The **charged reduction is 1,570 / 564**, with **3,430 / 4,436
remaining**. Charged production is **581 raw smaller / 291 same-format larger**.
The 5,000-line and normalized production-simplification goals are **not met**.
The user authorized a practical source closeout/integration with that shortfall
backlogged, rather than another speculative rewrite to meet the counter.
Deltas below use reduction-positive values and show raw / same-format:

| Slice / commit | Deleted path | Replacement cost | Net production reduction | Net test reduction | Outside charge | Cumulative / remaining |
| --- | --- | --- | ---: | ---: | ---: | --- |
| 036 | Unordered surface batching; Move-only selection; icon/centering tests | Ordered surface boundaries + three behavior checks | −44 / −46 | +44 / −24 | 0 | Reduction 0 / −70; gap 5,000 / 5,070 |
| 037 controls | Paint/register bypasses; renderer input state; per-row feedback; Debug keys; redundant hover/gallery tests | One retained Button/Control feedback path, explicit styles/cursor, typed equality, consumed/action pair, compact checks | +137 / +129 | +166 / +185 | 0 | Reduction 303 / 244; gap 4,697 / 4,756 |
| 037 composition/lifetime | Root descendant routes, cached paths/routing IDs, separate fixture card tree and redundant path/gallery tests | Exact owner/leaf validation, local layout/order, explicit lifecycle broadcast, two before/after checks | −104 / −123 | +45 / +77 | 0 | Reduction 244 / 198; gap 4,756 / 4,802 |
| 038 forms | Floating-label path; repeated field/footer/page placement; fixed-field/choice Vec rebuilding; bespoke suggestion controls; redundant form tests | Shared concrete placement helpers, compact IME layout and explicit form-action feedback | +55 / +67 | +14 / +24 | 0 | Reduction 313 / 289; gap 4,687 / 4,711 |
| 039 viewport ownership (`55c11f0`) | Reverse display-key parser and tool-root rescan | Direct owner on existing detail lines; real viewport/native-body regression using shared fixture | +22 / +5 | −40 / −70 | 0 | Reduction 295 / 224; gap 4,705 / 4,776 |
| 042 independent pruning / Submit retirement | Project activity/layout matrix, repeated GPU persistence setup, status glyph matrix, test-only Submit event and handlers | Controller persistence check, one isolated status render, real pointer submissions | +5 / +5 | +105 / +171 | 0 | Reduction 405 / 400; gap 4,595 / 4,600 |
| 039 native parent membership / metadata | Provider-ID overwrite and restore; provider pairing/orphan repair; child-by-child output disclosure mismatch | Native parent index, shared metadata accessors, direct grouping and expanded native/publisher cases | +21 / +13 frontend | −35 / −42 frontend | +77 / +106 (production 41 / 69; tests 36 / 37) | Charged reduction 314 / 265; gap 4,686 / 4,735 |
| 042 global adapter retirement | `test_ui.rs`, FixtureChoice/global selector/action conversions and App forwarders; old `scroll_tests.rs`; control/browser coordinate assumptions | Small owner-local selectors, actual pointer/cancellation paths, compact control state case and real stale-source navigation calls | +8 / +8 | +552 / +464 | 0 new; cumulative charge 77 / 106 | Charged reduction 874 / 737; gap 4,126 / 4,263 |

| 039 explicit body state | Length-only map; injected/reversed authored placeholders; obsolete attachment Row test | Shared body identity, separate preview availability and native literal/version regression | −15 / −32 frontend | +4 / −6 frontend | +15 / +21 production; cumulative 92 / 127 | Charged reduction 848 / 678; gap 4,152 / 4,322 |

| 040 model message ownership | Paint-time reconciliation, own-message GPU/Row matrix | ID-only Feed ownership, Controller update hooks and native handoff test; transitional presentation consumes it | +107 / −88 | −6 / +10 | 0 new | Charged reduction 949 / 600; gap 4,051 / 4,400 |

| 041 direct retained transcript | projection.rs, rich Row, Line, Tools::lines, Part reconstruction and per-frame body measurement | Concrete native tool/message owners, ID/height cache and actual handoff/measurement checks | +209 / −215 | +15 / +1 | 0 new (Hash derives have zero line delta) | Charged reduction 1,173 / 386; gap 3,827 / 4,614 |

| 041 legacy adapter retirement | Feed update/page/legacy overlap, skipped Transcript wire variants/type, old delta/page tests | Native full/sparse installation, offline native preview and compact thinking check | +180 / +62 frontend | +252 / +234 | −30 / −30 production; cumulative 62 / 97 | Charged reduction 1,635 / 712; gap 3,365 / 4,288 |

| 037 synthetic lifecycle retirement | Tick/Cancel event variants, broadcast classifier and lifecycle input handlers | Direct child updates, actual-owner cancellation and held-control checks | −71 / −86 | 0 / 0 | 0 new | Charged reduction 1,564 / 626; gap 3,436 / 4,374 |

| 038 attachment registry/routing retirement | CardDeck registry/seen-set/dispatcher/hints and per-button destination envelopes | Actual browser-owned cards; local choices use their existing target; metadata replacement remounts controls | +64 / +64 | −11 / −25 | 0 new | Charged reduction 1,617 / 665; gap 3,383 / 4,335 |

| 041 / 012 reading-policy consolidation | External layout reset/save policy, placed_session, expansion-position map/pin and per-paint preference round trip | One private reading choice on Transcript, scoped checkpoints and actual-owner anchor lookup; native bookmark hydration and bounded regressions | +17 / −12 | −80 / −107 | 0 new | Charged reduction 1,554 / 546; gap 3,446 / 4,454 |

This reading slice is an ownership/correctness change, **not a net size win**:
production is 17 raw lines smaller but 12 normalized lines larger; added/updated
coverage costs 80 / 107. Total growth is 63 / 119. No tests were deleted to offset
that cost, and no second reading coordinator or persistence schema was introduced.

| Bounded closeout (`24803c1`) | Unused Editor/Controls containment entry points and unordered Renderer::hit_text lookup | None; actual retained hit owners and ordered text selection remain | +16 / +18 | 0 / 0 | 0 new | Charged reduction 1,570 / 564; gap 3,430 / 4,436 |

The report now excludes Git-classified binary assets, without dropping textual
Java/host changes. Its historical 415aeff guard is **6,625 physical / 6,582 nonblank**:
the corrected per-item cfg(test) scanner preserves five blank separators that the
old suffix shortcut removed (four in app.rs, one in notices.rs). Nonblank source
is identical. This fixes the previously flagged PNG/UTF-8 report failure; the
Rust-only completion counts are unaffected.

The outside split also now classifies the daemon's cfg(test) agent_test family,
protocol_audit_test and transcript_legacy_test correctly. Their real entrypoints
were checked in lib.rs/transcript.rs. Before the pause merge this reclassifies
**1,624 raw / 2,937 normalized** from production to tests at **both** revisions;
whole-tree totals and all existing net deltas are unchanged. It prevents charging
the pause fix's new agent-test module as runtime code.

Outside-code measurement compares every tracked Rust source in `daemon` and
`protocol` with the same counter/formatter and reviewed test boundaries in the
changed files. Baseline combined totals: **8,454 / 13,425**, production
**5,856 / 8,883**. Pre-pause candidate: **8,516 / 13,522**, production **5,882 / 8,943**.
Only `daemon/src/blocks.rs` and `protocol/src/blocks.rs` add code; legacy types/variants
were deleted from `protocol/src/lib.rs` and `protocol/src/transcript.rs`; the earlier
`protocol/src/lib.rs` equality derive still has zero code-line delta. No other
crate, language, generated code or runtime host acquired replacement logic.

Keep raw and same-format totals alongside the ledger. A deletion shared by two
slices is counted once. Documentation and this measurement recipe are not runtime
savings. The baseline measurement was documentation-only; implementation checks
are recorded with each owning backlog item. The protocol equality derive changes
neither raw (442) nor same-format (527) code lines in `protocol/src/lib.rs`.
