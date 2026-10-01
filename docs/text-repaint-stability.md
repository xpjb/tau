# One-frame text dropout under cache pressure

October 1, 2026 (UTC). Source-only on `fix/tau2-text-frame-flash`, worktree
`/root/tau2-text-frame-flash`, based on `origin/tau2` at `003de72`.
**Pushed, not merged or deployed.** No version, protocol or schema change.

## Reproduced condition

The pinned Sanscale `7bbe230` tracks a block's last use only in shaping.
Tau deliberately retains unchanged Markdown/editor layout handles: the visible
scene checks that they still measure correctly and reuses them without a new
shape request. `Renderer::draw` prepares those handles on every repaint, but
preparation did not renew their block-cache age.

At the real **131,072-block** bound, allocating another block sweeps the oldest
entries down to roughly three quarters of the bound. Recently prepared message
and composer layouts therefore remained as old as their original shaping. A
late status label could sweep them after their `Draw` handles had already been
collected for the current paint. `prepare` skips the stale handles, while Tau
still clears/draws the background and the new label. On the next paint,
Markdown's scene and the editor detect the dead layouts and re-shape them.

The regression uses the actual Tau `Renderer`, Markdown `Preview`, shared
`Editor`, platform fonts and headless GPU pixels. It fills the production block
pool using distinct block keys over one shared tiny paragraph, paints a warm
frame with **zero shape requests**, then allocates a footer label during the
next paint. Before the fix:

- The warm frame is identical to the reference.
- The sweep frame loses all message/composer ink: **2,470 differing pixels on
  this host**, with the background and new footer still visible.
- The following frame returns to the reference without a source edit.

Reproduction commit: `b29462a`; failing run
`711a77c6-0659-4f2f-92b2-3176039fa2d3`.
Log: `/tmp/tau2-repaint-reproduction.log`; inspected PNGs:
`/tmp/tau2-repaint-before/{reference,sweep}.png`.
`TAU_REPAINT_DUMP_DIR` enables the same optional captures in the test.

This is a demonstrated condition matching a one-frame text blink, **not proof
that the user's observed periodic flicker reaches this capacity or has this
cause**. The old Compendium vertex-arena corruption fix (`e958f9b`, Sanscale
`3d3c14d`) is already included in the existing pin; batches own separate buffers.
There is no periodic frontend `TextService::clear` in this path.

## Fix

Sanscale commit **`4325844c649dde6e2f1812a489ebca8d9b2ddadd`**, pushed on
`fix/tau2-prepared-text-residency`, renews valid prepared blocks' age using the
same access clock as shaping, including geometry hits. Tau's workspace pin and
lockfile now reference that commit. The dependency patch is based directly on
the previous pin, without importing unrelated upstream API changes.

No extra shaping, layout revision changes, atlas/vertex uploads, cache-bound
increase, frame/lease API or continuous repaint workaround was added. Cold
layouts remain evictable. Invalid block/paint inputs remain incomplete; preparing
an already-evicted handle cannot restore its text. Measurement and recording a
retained batch alone still do not renew residency.

## Validation

All Rust commands used the managed Cargo wrapper and behavioral nextest runner.

- Tau frontend/Markdown all-target compiler check, offline/locked: passed;
  existing dead-code/platform warnings remain. `/tmp/tau2-repaint-check.log`.
- Frontend and Markdown **library** nextest: **190/190 passed, zero skipped**,
  run `6d92c8a9-cfaa-4214-b495-13c1723b1f2a`;
  `/tmp/tau2-repaint-suite.log`.
- Both consumer pixel regressions with explicit Vulkan: **2/2 passed**, run
  `a16d0650-83a3-4433-8e92-68c70ae361e1`;
  `/tmp/tau2-repaint-vulkan.log`. The cache-sweep frame now matches the unchanged
  message/composer reference. The other regression captures **32 queued
  repaints across 64 text surfaces**, with shape/image/text pipeline switches
  and actual new-glyph atlas uploads, without intermediate polling/readback;
  every captured frame matches its reference.
- Sanscale library/integration nextest, perf counters enabled and including
  normally ignored GPU cases: **64/64 passed, zero skipped**, run
  `0855c23c-a0e5-4e3d-a06e-6b2288b54f36`;
  `/tmp/sanscale-residency-suite.log`. The added lifecycle case checks warm
  pixels, unchanged retained-batch liveness, cold eviction and stale-input
  rejection at the real capacity.
- Sanscale private-item rustdoc with all features and `-D warnings`: passed;
  `/tmp/sanscale-residency-doc.log`. Public declarations are unchanged.
- `git diff --check`: passed. No unrelated dependency versions/edges changed.

Optional OpenGL validation could not initialize the host device: both cases
failed inside `HeadlessCtx::new` with **“Parent device is lost”**, before any
Tau rendering, including a surfaceless/software retry. Logs:
`/tmp/tau2-repaint-gl.log`, `/tmp/tau2-repaint-gl-surfaceless.log`. Do not count
these attempts as GL/GLES acceptance. Physical Windows/DirectX and Android
confirmation of the reported flicker remains open.

No Clippy, Cargo built-in test runner, production data/service changes, paid
provider calls, package build, release or deployment. Stable/master, Compendium,
Sanscale master and the integration worktree were left unchanged.
