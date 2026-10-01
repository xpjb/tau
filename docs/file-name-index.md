# Filename index: size, policy and sync discovery

September 30, 2026 (UTC). Branch `fix/tau2-file-index`, worktree
`/root/tau2-file-index`, based on `origin/tau2` at `003de72`.
Audit tool: `cdb9b96`; dot-directory pruning and repaint regressions: `36dc9c8`.
**Source-only, pushed; not merged or deployed.** No version/protocol/schema change.

## Scope and conclusion

The user reported a Windows picker stuck on “Syncing file names” and clarified
that another worktree owns the wider data-socket/loading investigation. This work
covers index feasibility, dot-directory exclusion and the filename UI's wake /
repaint boundary. It does **not** diagnose or fix the reported Windows hang, alter
native transport/recovery, or close [backlog 014](../tau2-backlog/014-native-data-recovery-after-outage.md).

A whole development root can have a quite manageable **names-only** index after
excluding hidden directory trees and respecting project ignores. On this host,
the previous hidden traversal hit the 200,000-file cap. Pruning dot-directories
retained exactly the same 93,484 visible files, plus 462 ordinary dotfiles, without
hitting a limit: about **10 MiB of client name/record storage**, **9.7 MiB of JSON**,
and **0.73 MiB of compressed filename payload** for the initial snapshot.

This is not evidence that an arbitrary drive has fewer than 200,000 eligible
files, nor a measurement of Windows network throughput or total application RAM.

## What is already implemented

Sources: `code-viewer/src/filesystem.rs`, `code-viewer/src/finder.rs`,
`frontend/src/file_index.rs`, `protocol/src/files.rs`, `transfer/src/files.rs`.

- Queries are local Nucleo matching. Typing does not issue a search RPC.
- One daemon worker indexes canonical roots ahead of Find. A foreground client
  warms names independently of previews; chat/root/source changes fence results.
- A root has a sorted name snapshot and a hash revision. An unchanged revision
  sends **zero names**; the one retained previous revision receives added/changed
  names and removed names. Unknown/evicted revisions receive a complete snapshot.
  This is revision/delta deduplication, **not** a reusable chunk manifest or an
  unbounded change journal. Disconnecting across several changed revisions can
  therefore require another full snapshot.
- Names use the existing authenticated native connection, background admission,
  independent 16 KiB zstd level-1 chunks, 64 KiB byte credit, chunk/response hashes,
  and stream-local cancellation. There is no second endpoint or search per query.
- Both sides' names are memory-only. Client restart requires a new snapshot.
  Updates become visible atomically after the whole response has been verified.
  A cancelled initial snapshot has no persisted filename-chunk resume checkpoint.
- The daemon can retain four roots and their previous revisions. It rescans each
  retained root every ten seconds, even if it is no longer the current picker
  root. The frontend retains one cached scope; other holders can temporarily keep
  old snapshots alive while matching/rendering.
- Per scan: 200,000 files, 24 MiB conservative JSON-record budget, and a cooperative
  15-second traversal budget. Unsupported paths, read failures or truncation mark
  the index partial. A finite response is capped at 32 MiB. Limits are not a promise
  of complete drive coverage; symlink directories and ignored paths are omitted.

## Implemented exclusion policy

The previous two-pass walk visited visible paths first, then all hidden paths
except `.git`. That avoided hidden files displacing visible files, but still
walked hidden cache trees and spent memory/time/bandwidth on them even when the
picker's Show hidden toggle was off.

There is now **one traversal**. Dot-prefixed directory descendants are rejected
before descent at every depth, independently of `.gitignore` negations. `.git`
entries remain excluded. Dotfiles such as `.env` remain indexed but are hidden in
picker results until Show hidden is enabled. Ignore rules still apply, including
outside initialized Git repositories, and symlink directories are not followed.

Directory browsing and explicit file opening are unchanged. Show hidden can still
expose hidden directories in the browser, but does not start indexing them.
**Here** is an explicit scope change: a user may browse into a hidden directory
and choose it as a root; its own dot-directory descendants are still pruned.

Generated **non-dot** directories are a separate issue. Existing `.gitignore` and
`.ignore` rules are the current control, not a guessed global blacklist. For a
user-owned development root, optional `.ignore` directory patterns could include:

```gitignore
**/target/
**/node_modules/
**/build/
**/dist/
```

Do not install those automatically: some projects intentionally keep relevant
files there. Explicit browsing remains available regardless of index ignores.

## Read-only measurements

The audit requests the actual production filesystem service, applies the reply
with the actual client `PathIndex`, then serializes/compresses it using the same
chunk size, compression level and raw-fallback rule as native filename transfer.
It reads names/metadata/ignore files, not ordinary file contents, prints aggregate sizes,
and does not connect to a daemon, stage filenames, or touch production storage.

Commands, through the managed Cargo wrapper with one build/Rayon job:

```sh
CARGO_BUILD_JOBS=1 RAYON_NUM_THREADS=1 /usr/local/bin/cargo run --offline --locked \
  -p tau-code-viewer --features filesystem --example index_size -- /root
```

The first measurement used `cdb9b96` (old traversal); the second used the pruning
change. `/root` is this host's live development tree, not a controlled immutable
fixture or the user's Windows drive. Git/ignore rules were enabled for both runs.
The audit executable does not create/remove project files; Cargo writes its normal
build outputs. Source edits between runs are outside the audit itself.

| Measurement | Previous two-pass walk | Prune dot-directories |
| --- | ---: | ---: |
| Indexed files | 200,000 | 93,946 |
| Visible files | 93,484 | 93,484 |
| Limit reached | Yes | No |
| Scan + polling to completed snapshot | 2.813 s | 1.257 s |
| UTF-8 relative-path bytes | 17,515,470 | 7,502,663 |
| Client vector/string heap lower bound | 23,915,470 B / 22.81 MiB | 10,508,935 B / 10.02 MiB |
| Serialized index reply | 23,110,451 B / 22.04 MiB | 10,131,686 B / 9.66 MiB |
| Compressed chunk payload | 1,701,970 B / 1.62 MiB | 768,199 B / 0.73 MiB |
| 16 KiB source chunks | 1,411 | 619 |

Logs: `/tmp/tau-index-before.log`, `/tmp/tau-index-after.log`. Compressed payload
**excludes** Tau headers, credit messages, QUIC/IP overhead and retransmission;
it is not a packet capture or end-to-end goodput claim. The heap figure includes
client vector capacity and allocated string capacities on this 64-bit host, but
not allocator rounding/metadata, daemon copies, retained revisions, JSON buffers,
transient delta maps, matching results, previews, or total process RSS. Those make
actual peak RAM higher. Scan timings are single warm-host samples, not guarantees.

The tool also times scoring `main src` over every returned path. The new debug
build scored 93,946 paths in 832 ms; that is **unoptimized scoring only**, not the
full latest-only matcher/ranking pipeline or release/device performance acceptance.
Do not use it as a claim that large picker queries are instantaneous.

### How it scales beyond this tree

The measured average relative path is about 80 bytes. At that path length, a
client record is 32 bytes on this host, giving roughly 112 bytes/file before the
extra allocations above. JSON averages approximately 108 bytes/file here.

| Eligible files | Approximate client names/records lower bound | Approximate raw JSON |
| --- | ---: | ---: |
| 100,000 | 11.2 MB / 10.7 MiB | 10.8 MB / 10.3 MiB |
| 200,000 | 22.4 MB / 21.4 MiB | 21.6 MB / 20.6 MiB |
| 1,000,000 | 112 MB / 107 MiB | 108 MB / 103 MiB |

These are arithmetic extrapolations, **not measured inventories**. Compression
varies with path structure, and long/deep names hit the byte cap before the file
cap. A million-file flat snapshot is not supported by the present protocol limits.
Thus “index the dev drive” is practical for this filtered tree, but cannot be
promised generically by simply raising the cap or transferring everything again.

## Repaint result, and its limits

The publication path is:

1. `file_index::Service` publishes a verified snapshot/update and calls `Wake`.
2. `Controller::poll` accepts generation/session/source-fenced updates and marks
   the model changed.
3. `RootWidget::update` runs `code_tick`, even with the browser closed. The picker
   accepts names, submits local matching, and marks the UI dirty.
4. Matcher completion has its own wake; `code_tick` accepts its current generation
   and marks the UI dirty. Desktop `App::tick` then requests a redraw.

New real-daemon/native-client/headless-App regression opens an empty picker with
an exact filename query, then advances **only on actual wake events** and paints
only when `App::tick` requests it. Names, the sole non-hidden-directory match,
highlights and the preview arrive without further input or forced polling frames.
Existing native name-sync and large-matcher tests now also assert their callbacks
wake an idle consumer. All pass; no filename-specific missing repaint was
reproduced and no speculative continuous-redraw workaround was added.

This is not physical Windows/winit/DirectX or Android acceptance. It cannot rule
out a Windows event-loop issue, a stalled data connection, or a result that never
completes. The initial “Syncing file names” label also lacks byte/phase progress;
it does not distinguish acquisition, initial scan and response transfer. Keep the
other worktree's wait-boundary/recovery diagnosis independent of this result.

## Bounded follow-ups (not implemented here)

1. Keep the existing limits and partial-state UI. Expose an intentional root /
   ignore-policy control before assuming every drive is a suitable default root.
   Names-only prefetch is already in place; do not reintroduce per-keystroke RPCs.
2. Avoid rewalking four idle large roots forever. Coalesced filesystem invalidation
   plus bounded reconciliation could replace the ten-second full rescan, with a
   full fallback on missed events / changed ignore files. Measure first; do not
   replace a scan with unbounded recursive watchers.
3. For inventories beyond the current cap, prototype directory/size-bounded,
   revisioned shards and reuse the existing verified native transport. Reuse
   unchanged shards, select a cheaper snapshot when a delta costs more, and
   checkpoint initial sync. This is names-specific work, not a new universal
   state-replication framework or alternate data socket.
4. If cold-start download is actually a problem, persist **verified** names per
   account/source lineage/canonical root/policy version with a byte quota and
   explicit stale/error state. Do not publish another source's paths or treat an
   incomplete transfer as complete coverage. Current memory-only caching remains
   deliberately unchanged.
5. Measure release scoring/ranking and transient/steady heap on the target Windows
   client and the weakest supported phone before raising path/RAM limits. Index
   size and network dedup do not by themselves guarantee quick local matching.

## Validation / handoff

- Managed `check --offline --locked -p tau-code-viewer -p tau-frontend --all-targets
  --features tau-code-viewer/filesystem`: passed, 1m52s; existing platform/dead-code
  warnings retained (`/tmp/tau-index-check.log`).
- Focused managed nextest: **21/21 passed**, 185 unrelated frontend/code-viewer
  tests excluded, run `cc38e41f-c970-4556-ac24-0a9d932bcf28`, 6.628s of tests after
  48.91s compilation (`/tmp/tau-index-tests.log`). Selection:

  ```sh
  /usr/local/bin/cargo nextest run --offline --locked \
    -p tau-code-viewer -p tau-frontend \
    -E 'package(tau-code-viewer) | test(file_index::tests) | test(code_view::tests) | binary(remote_files)'
  ```

  Covers the new directory-pruning / explicit-browsing / Here policy, stable
  revisions and rename/removal deltas, native names/preview lifecycle, large
  latest-only matching, desktop/phone UI fixtures, and the wake-driven repaint.
- Both actual `/root` audits completed. `git diff --check` passed.
- No Clippy, Cargo built-in test runner, full workspace suite, platform/package
  rebuild, service restart, merge/deployment, production DB write, or paid provider
  request. Windows hang/recovery and physical-device acceptance remain open.
