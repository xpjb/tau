# Tau / Tau2 branch rollup — October 2, 2026

## Scope and safety

Account for every local and live remote branch, retain useful Tau2 work, remove
obsolete refs, and release matched Tau2/beta clients and daemon. Stable `master`
and `tau.service` stay unchanged. No production data backup/restore, model call,
formatter, Clippy or built-in Cargo test runner is part of this work.

Start: `tau2-integration` and live `origin/tau2` at `396d5ba`; stable local/remote
`master` at `3818579`. Live default branch is still `master`. All six initial
worktrees were clean and there were no stashes. Inventory: **99 local and 50
remote branch refs, 101 distinct names**. Only eight distinct branches had commits
outside Tau2 ancestry; legacy master itself is already an ancestor of Tau2.

Before deletion, all refs and their objects were saved in the verified 8.5 MiB
Git bundle `/root/tau-branch-rollup-20261002/before.bundle`. That directory also
contains the exact ref lists, deletion transactions, remote lease-guard logs,
baseline regression patch and validation logs. This is Git recovery material,
not a backup or modification of live Tau data. Tags are unchanged.

## Earlier agent's changes

- Both fresh integrations are present: model-catalog `d528e56`/`d3de9ad` and
  Android startup `1321f65`/`0d40a67`.
- `396d5ba` has exactly the same tree as `0d40a67`. The earlier three `ours`
  review merges added ancestry, not source changes. Review tips remain reachable:
  `24b8323` (client structure), `8091d92` (retained proposal), `90953ef` (UI audit).
- Current proposal/audit files retain later updates. The old 015–035 proposals
  already have a disposition in `tau2-backlog/triage.md`; restoring their old
  files would revive superseded plans, not recover a missing implementation.
- The unintended remote `tau` branch is absent. Neither stable master nor the
  current Tau2 source needs a rollback based on the verified refs and trees.
- The previous ten-worktree removal cannot be reconstructed folder-by-folder
  from the remaining worktree registry. The cleanup script keeps branch refs,
  checks tracked/untracked cleanliness, and can delete ignored build outputs.
  Current branch ancestry/content was checked independently, not inferred from
  that script's remote-reachability rule.

## Missing versus obsolete work

- **Block loading/recovery is still needed.** Against current `396d5ba`, the
  admission regression loads 30 of 60 visible bodies; disk hydration fails to
  schedule a repaint; the extended UDP outage fails its five-second per-body
  progress guard with a healthy control socket (about 72 ms RTT). Its source fix
  is merged into the review branch at `64a9beb`, with both handoff histories kept.
- **New-chat chooser is still needed.** The current real-App regression fails
  before confirmation at `quick_start`; the old fix was not carried into retained
  UI. `4ebb24f` ports it into existing Composer/Header/Sidebar, keeps the current
  thinking label and genuine failure/run/pause states, and preserves the existing
  provisional starter flag/background-sync contract. No old App or snapshot
  adapter was restored.
- **Network pressure harness: include**, per the user's October 2 call. Reuse
  one private impaired-link fixture for both suites, preserving UDP-only outages
  with a separate fixture flag and full TCP/UDP blackholes. Adapt Notice text to
  the current typed API. Keep opt-in local DB/admission/content diagnostics and
  all original failure reports; do not suppress a content failure or weaken a
  progress assertion to make the suite green.
- **Remove-pause experiment: discard**, explicitly directed by the user. It
  removes Play/pause/run-through and changes recovery semantics. Preserve current
  Stop/Play, failure safeguards and open UX task 045; do not merge the experiment.
- **Inline input:** native editor code was adapted at `afde400` and integrated
  through `7ab80f3`. Java differences are later haptic/paste fencing improvements;
  the old app-routing code is obsolete.
- **Accepted-message UI:** change plus exact revert has an empty net tree diff
  from `c1f0554`; nothing to carry forward.
- **Legacy minimal scroll hotfix:** `git cherry` identifies both code/test patches
  as already applied. The only other change is obsolete Android versionCode 37.
- **Storage reconciliation:** explicitly recorded duplicate SQLite draft, replaced
  by the canonical `cddb8b7` implementation integrated at `4171a86`.

## Validation record

All Rust commands use `/usr/local/bin/cargo`, one Cargo/Rayon job and nextest.

- Current-mainline negative control: only the regression tests were added to a
  detached `396d5ba` checkout. **4/4 fail** as described above; run
  `88f6ea00-7ee5-48d0-9420-c4328f74fdb0`, 23.593 seconds. This is evidence that the
  fixes are missing from today's code, not just a branch-ancestry guess.
- Initial candidate workspace/all-target check passed. Full candidate nextest:
  **361/362 passed**, zero skipped, run `4cee6437-60cd-4f54-b198-aa4e3161da7f`,
  124.962 seconds. All four negative-control cases pass here. The old branch's
  unnecessary provisional `starter=true` change broke the background-sync case;
  that flag change was removed rather than weakening the existing assertion.
- The pressure harness's first compiler pass found the old String-style Notice
  formatting. It now reads the typed notice's text; no production API was changed
  to accommodate an obsolete fixture.
- Combined workspace/all-target compilation passes. Final focused nextest:
  **82/83 passed**, 282 filtered out, run `5e1485e8-dee2-4702-87ee-715777dd9486`,
  115.613 seconds. All daemon cases, native outages, both new-chat regressions and
  the unchanged background-sync case pass. Pressure normal/recovery pass; dodgy
  seed 29 fails its strict no-transport-errors guard after a failed native
  connection attempt. Correct files and message identities survive. The exact
  report is retained at `docs/network-pressure/rollup-dodgy-29-failure.json`.
  No assertion is weakened and no passing rerun replaces the failed result.
- Installer preparation is authorized; deployment is awaiting the user's call on
  this stress finding. Physical-device acceptance and bugs 013/014/045 remain open.

## Cleanup progress

Deleted **45 remote refs** in one atomic push, each guarded by its exact inspected
tip. Deleted **93 local refs** in one expected-tip transaction. Stable and Tau2
mainline refs were excluded. Removed the clean unused Android merge checkout and
the explicitly rejected remove-pause checkout. The Android feature checkout is
still used as the working directory of the host adb server, so its local branch
and worktree remain; its tip is already integrated. The three remaining feature
refs are retained until their combined source acceptance and publication.

## Complete starting branch inventory

Tips below are the pre-cleanup snapshot. “Ancestor” was verified against
`396d5ba`, including remote-only and differently advanced local/remote refs.

| Branch | Local tip | Remote tip | Disposition |
| --- | --- | --- | --- |
| `docs/0.5.8-deployment` | `3797303a1985` | — | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `docs/deployed-0.5.10` | `4d09826e412f` | — | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `docs/tau2-network-backlog` | `a3704e9af6f0` | `a3704e9af6f0` | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `feat/flag-it` | `8d0807bcf555` | — | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `feat/generate-image` | `3818579b7430` | — | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `feat/rust-transfers` | `30c471a1b9b3` | — | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `feat/starter-chat` | `ee56775a5977` | — | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `feat/tau2-attachments-view` | `d0597557cc66` | `d0597557cc66` | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `feat/tau2-block-sync` | `43e40a0d9efe` | `43e40a0d9efe` | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `feat/tau2-finished-attention` | `ebbbc54f9a39` | — | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `feat/tau2-remote-code-viewer` | `ec962ba92330` | `ec962ba92330` | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `feat/tau2-retained-ui` | `186ad2373fa1` | `186ad2373fa1` | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `feat/tau2-simplification` | `7be1ac1fdbf4` | `7be1ac1fdbf4` | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `fix/copy-message` | `cd695e51f050` | — | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `fix/dialog-enter` | `4dafea610c1f` | — | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `fix/evict-transcript-on-idle-sleep` | `07bea2446077` | — | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `fix/history-loading` | `24b9da0ced60` | — | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `fix/image-viewer-updates` | `588a80b0d8aa` | `588a80b0d8aa` | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `fix/image-zoom` | `0ea6abb4b7fd` | — | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `fix/markdown-tables` | `8566b0539c84` | `8566b0539c84` | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `fix/model-failure` | `dd2c2286b960` | — | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `fix/new-chat` | `f66b8a19dbb6` | — | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `fix/qa-batch` | `050e3ca9f29b` | — | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `fix/quiet-reconnect` | `86b66c47f600` | — | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `fix/recovery-order` | `5d8ed1e18484` | — | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `fix/remember-downloads` | `d68d73a9b805` | — | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `fix/selection-crash` | `e9eb849bc8dc` | — | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `fix/selection-drag` | `8f9bbcab64d9` | — | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `fix/skip-malformed-history` | `b5d14786e012` | — | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `fix/stream-scroll-anchor` | `bd944a9f15d4` | — | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `fix/tau2-accepted-message-ui` | `4b198ab0cc8e` | `4b198ab0cc8e` | Delete: exact change and revert; empty net tree diff from c1f0554 |
| `fix/tau2-alerts` | `1b0c1447c5aa` | `1b0c1447c5aa` | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `fix/tau2-android-startup-lock` | `1a8ee0c1e833` | `1a8ee0c1e833` | Already merged; delete remote ref, keep local ref while adb process uses its worktree |
| `fix/tau2-block-loading-stability` | `edfb629cb456` | `edfb629cb456` | Needed: current-mainline regressions fail; source merge 64a9beb |
| `fix/tau2-cache-ttl` | `eb7540470dde` | — | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `fix/tau2-checkpoint-sync` | `2517e9cd0eed` | — | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `fix/tau2-codex-plan-usage-20260926` | `c3694a765f5b` | `c3694a765f5b` | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `fix/tau2-compaction-pause` | `1f75f4dd0c62` | `1f75f4dd0c62` | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `fix/tau2-composer-thinking` | `cdc33ca0da70` | `cdc33ca0da70` | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `fix/tau2-connection-message-continuity` | `1f6179e4b4d4` | — | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `fix/tau2-context-usage` | `c010915f6f83` | `c010915f6f83` | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `fix/tau2-control-recovery` | `eab961b128d9` | `eab961b128d9` | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `fix/tau2-conversation-bumping` | `9c14bda5c5ca` | — | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `fix/tau2-download-notification-navigation` | `b0bcfe31efbd` | `b0bcfe31efbd` | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `fix/tau2-download-ui` | `7b74bceb736c` | `7b74bceb736c` | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `fix/tau2-extract-behavior` | `a66fc3d254fb` | `a66fc3d254fb` | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `fix/tau2-file-index` | `a395f279a144` | `a395f279a144` | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `fix/tau2-fzf-picker` | `7963270d4c4f` | `7963270d4c4f` | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `fix/tau2-immediate-intent` | `5c5ef4dd062c` | `5c5ef4dd062c` | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `fix/tau2-inline-downloads` | `2b35fd511fdc` | `2b35fd511fdc` | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `fix/tau2-link-underline` | `6395a362a291` | `6395a362a291` | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `fix/tau2-mobile-inline-input` | `46618b914250` | `46618b914250` | Delete: adapted in afde400 and integrated through 7ab80f3; native editor files retained |
| `fix/tau2-mobile-qa-20260926` | `c4915184513d` | `c4915184513d` | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `fix/tau2-model-catalog-refresh` | `59c87a21427a` | `59c87a21427a` | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `fix/tau2-network-pressure` | `9502cdcfb08d` | `9502cdcfb08d` | Merge harness and opt-in diagnostics, per October 2 user decision; preserve known failures |
| `fix/tau2-new-chat-status` | `5e70fbed90cf` | `5e70fbed90cf` | Needed: current-mainline regression fails; adapted source merge 4ebb24f |
| `fix/tau2-qa-session-20260925` | `d2bef686ee85` | `d2bef686ee85` | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `fix/tau2-queue-edit-ack` | `6d3066878730` | — | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `fix/tau2-realtime-connection` | `1a663fca9d4b` | `1a663fca9d4b` | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `fix/tau2-resume-during-stop` | `0b7c2ae9e0d9` | `0b7c2ae9e0d9` | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `fix/tau2-soft-breaks` | `cc5084cdd9b5` | — | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `fix/tau2-text-frame-flash` | `7f961d7bcf70` | `7f961d7bcf70` | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `fix/tau2-tooltips` | `3bbb52b0b8f9` | — | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `fix/tau2-topic-bumping` | `01cace745dda` | `01cace745dda` | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `fix/tau2-transcript-cache` | `254785690fec` | `254785690fec` | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `fix/tau2-undock-scroll-latency` | `ff92cf891943` | `ff92cf891943` | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `fix/title-prompt` | `2ba569bdb891` | `2ba569bdb891` | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `fix/unified-send-file` | `9a38a50f894a` | — | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `fix/unnamed-chats` | `f7c8a388d2e0` | — | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `fix/unread-running` | `6188a6c11689` | — | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `fix/windows-download-finish` | `d304e5e308b2` | — | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `investigate/selection-crash` | `4dafea610c1f` | — | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `master` | `3818579b7430` | `3818579b7430` | Keep stable master unchanged |
| `merge/tau2-android-startup-lock` | `0d40a67b0995` | `0d40a67b0995` | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `merge/tau2-control-recovery` | `4afd665d1a3e` | `4afd665d1a3e` | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `merge/tau2-fzf-picker` | `33f7d6f6b455` | — | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `merge/tau2-link-underline` | `b76386c44a38` | — | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `merge/tau2-model-catalog-refresh` | `d3de9ad7b610` | — | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `merge/tau2-remote-code-viewer` | `415aeff6d275` | — | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `merge/tau2-retained-ui` | `6da3b4cedf0b` | `6da3b4cedf0b` | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `merge/tau2-text-residency` | `8cceeb815751` | `8cceeb815751` | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `plan/tau2-simplification-completion` | `6ce8c7915b65` | `6ce8c7915b65` | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `refactor/flat-events` | `b1a06ddd619d` | — | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `refactor/session-and-attachment-lifecycle` | `74f0ceb7c1fd` | — | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `refactor/transcript-domain-and-ws-heartbeat` | `fe0ef1b54202` | — | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `release/0.5.11-r2` | `b0f87e67d738` | — | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `release/0.5.12` | `9239a2f81d9c` | `dbe9fc801d8b` | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `release/0.5.13` | `563bf7c36053` | — | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `release/0.5.14-scroll-hotfix` | `025c2b4e728e` | — | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `release/0.5.14-scroll-hotfix-minimal` | `0d1c9b01bc93` | — | Delete: both code/test patches already applied; only obsolete Android versionCode 37 remains |
| `scratch/tau2-storage-reconciliation` | `4603a95db733` | — | Delete: duplicate SQLite draft; canonical cddb8b7 integrated at 4171a86 |
| `tau2` | — | `396d5ba63ac6` | Keep remote Tau2 mainline |
| `tau2-integration` | `396d5ba63ac6` | — | Keep local Tau2 mainline |
| `tau2-qa-thinking-stream` | `84071ab49032` | `84071ab49032` | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `tau2-qa-topics` | `73e64c307a1e` | `73e64c307a1e` | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `tau2-rust-frontend` | — | `060bcd9dc16c` | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `tau2-sanscale-text-input` | `04fd60c5accd` | `04fd60c5accd` | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `tau2/daemon-settings-prompts` | `446ac299ac45` | — | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `tau2/integrated-agent` | `cddb8b7bfa91` | — | Delete: ancestor of 396d5ba; all commits already in Tau2 |
| `tau2/qa-remove-pause` | `d561c6507f46` | — | Delete: user rejected experiment on October 2; preserve current Stop/Play and open UX task 045 |
| `tau2/rust-frontend` | `060bcd9dc16c` | — | Delete: ancestor of 396d5ba; all commits already in Tau2 |
