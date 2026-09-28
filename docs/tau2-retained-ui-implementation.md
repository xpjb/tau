# Retained UI implementation checkpoint

**Stage 1 mechanics are implemented; the whole proposal and Android UX are not finished.** Work is on
`feat/tau2-retained-ui`, based on integration / `origin/tau2` at `415aeff`.
Source checkpoint: `4642461a4444903af5d82c8987785710109620c0`.
The earlier ownership-only commit is `6d950ca` (121 frontend unit tests passed).
No integration merge, deployment, service restart or release package was performed.

## UX decisions override mechanical compatibility

The user rejected carrying forward Android's separate editing window. Existing
behavior is not automatically a requirement: preserve data integrity and intended
capabilities, but evaluate the interaction itself before protecting it with tests.
The target is **the actual field edited in place, in the same window, with the
system IME**, not a second screen or Done/save-draft step.

The source checkpoint below still contains the old window adapter with added
callback fences. **That is a migration intermediate, not approved Android UX.**
Its window-specific behavior must be replaced, not treated as a compatibility gate.

Read-only review found the relevant implementation on
`fix/tau2-mobile-inline-input` at `46618b9`, based on `40a3698`. It keeps Rust drawing
and hit-testing the field while a transparent same-window Android Editable /
InputConnection supplies IME services, with editor-ID/revision-fenced snapshots.
Its source and handoff were inspected; its reported tests were not independently
rerun in this review. It remains unmerged and physical-device acceptance is open.
Adapt that work to retained TextFields and the newer CodeBrowser rather than
blindly importing its older App ownership/index routing.

The same principle applies elsewhere. Whether a download notification should
navigate away from an unsaved dialog needs a UX decision; an uncommitted patch
restoring the old notification priority was set aside rather than silently
cementing that behavior. The data/source-navigation guarantees remain requirements.

## Implemented boundary

`App` owns four siblings: `controller`, `root`, `ui` and `services`.
`ui::Context` lends broad mutable Controller/UiState/Services access without
borrowing the root through itself. `Widget` enforces `handle_event` and
`visit_perframe`; composition is concrete, with a closed dialog enum.

The first closed subtree comprises connection settings and **all topic dialogs**
(new, rename, prompt, delete and delete-mode choice), not just their visuals:

- Named fields wrap the existing Editor. Shared Control/Button/TextField/Form
  code handles current-coordinate hits, rounded button shapes, inherited clips,
  capture, text selection, composition, field Tab order and native editing.
  Changed text and consumed input are not conflated.
- Root routing gives the retained dialog exclusive input, including its backdrop.
  Captured release never re-targets a replacement. Paint and input share the
  placed clip; Renderer only gains a small clipping helper, not another renderer.
- Local handlers perform validation and model/service effects directly. FIFO
  owner-applied requests perform structural changes outside traversal, after
  input/model updates and frame visits. A stale close cannot close a new dialog;
  an error does not discard the remaining queue.
- Topic completion matches the request and account/source on its dialog instance.
  Connection completion belongs to the submitting instance/identity; failures
  preserve edits. Underlying durable operations remain Controller-owned.
- Composer focus is restored only if its old account/source/chat binding remains
  valid. The code browser pauses its interest immediately when covered, without
  discarding its existing View/Document/Selection or introducing another composer.
- Android edit and clipboard callbacks now carry originating tokens. Coalescing
  only combines snapshots for the same token. Blur/resize cancels gestures, **not**
  the native editing window's live session. Detachment, focus changes, source
  changes and legacy navigation invalidate the appropriate token. Android also
  reports window focus into UiState. Java and Rust must be built together.

Connection's old `Modal.options` declared diagnostics, saved actions and cache
clearing, but its special drawing path never painted them. These are accessible
through **More…** rather than silently discarded. The phone introduction uses its
measured height so it no longer overlaps the floating URL label.

## Deletions, not a second implementation

Removed the old connection renderer, project-dialog renderer, project
submit/completion methods, root `connecting` / `saving_project` fields and all six
corresponding `ModalKind` variants. Migrated dialogs do not emit legacy hit records,
use modal field indices, import `Action`, or dispatch their buttons through it.

The three global variants `RemoveProject`, `CopyDiagnostics` and `ClearReplica`
are gone (**82 -> 79**). Remaining openers are temporary legacy-to-retained entry
points. `ui_owner.rs` contains a narrow, explicitly temporary structural bridge to
unmigrated model/daemon/catalogue/saved-actions dialogs, not a general effect bus.

## Still to do (proposal stages 2–4)

- Remaining dialogs, Menu, TooltipHost, NoticeWidget and ImageViewer.
- The nested attachment-card/scroll-parent pilot **before** expanding workspace
  pointer routing. Current scope/leaf targets prove a closed dialog, not arbitrary
  deep-tree routing or touch-scroll takeover. Do not represent that gate as passed.
- Sidebar, composer, quick models, attachment browser, CodeBrowser ownership and
  owned scrolling; then transcript reconciliation and final adapter removal.
- Delete global Action/Hit tables, Modal/index focus routing, Lane and reset lists
  after their real replacements. These still exist only for the legacy workspace.

There is no new model/replication framework or competing ClientState. `Controller`,
`details.rs`, native protocol and shared tool semantics are unchanged. The other
agent's bounded tool-projection work can use this branch's existing model APIs;
transcript migration should consume that projection rather than recreate it.
Backlog 012 remains deferred: only mechanical ownership-path updates were made to
its existing tests, with no anchor redesign, added restoration tests or deletions.

## Actual cost at this checkpoint

These are **physical production lines**, including comments/blanks, excluding
solely test code. This is the proposal's replacement scope, not the entire app.

| Source, under `frontend/src` | Integration `415aeff` | `4642461` |
|---|---:|---:|
| `app.rs` | 4,815 | 4,560 |
| `app/projects.rs` | 271 | 143 |
| `app/navigation.rs` | 159 | 161 |
| `app/code_view.rs` | 452 | 451 |
| Unchanged attachment/notices/ripple/composer-status/scroll/tooltip scope | 923 | 923 |
| `app/ui_owner.rs` | 0 | 84 |
| `app/ui/mod.rs` | 0 | 167 |
| `app/ui/controls.rs` | 0 | 207 |
| `app/ui/dialogs.rs` | 0 | 334 |
| **Scope subtotal** | **6,620** | **7,030** |

Charge another **14 net production lines** outside that scope: Renderer +10,
Editor -1 (existing full-value replacement available to the shared adapter),
desktop +1, Android Rust +4, Java zero net. Effective charged total: **7,044**,
**+424 production lines**, not a saving. Nonblank: 6,582 -> 6,987 including those
adaptations (**+405**). Reused primitives currently have only two dialog families.

Test-only source grows **292 lines**: +263 in test files and +29 in dialog test
helpers/module declaration. Total frontend source growth is **716 physical lines**,
matching `git diff 415aeff 4642461 --numstat -- frontend`. Documentation is separate.
Moved business logic is not counted as eliminated semantics. This pilot neither
proves nor revises the final reduction target; re-budget before broad migration.

Reproduce the scope subtotal from the immutable source checkpoint:

```python
import subprocess
rev = "4642461"
regions = {
    "app.rs": [(1, 4560)], "app/attachments.rs": [(1, 1), (4, 477)],
    "app/projects.rs": [(1, 143)], "app/navigation.rs": [(1, 161)],
    "app/notices.rs": [(1, 29)], "app/ripple.rs": [(1, 61)],
    "app/composer_status.rs": [(1, 25)], "scroll.rs": [(1, 87)],
    "tooltip.rs": [(1, 172)], "tooltip/text.rs": [(1, 74)],
    "app/code_view.rs": [(1, 451)], "app/ui_owner.rs": [(1, 84)],
    "app/ui/mod.rs": [(1, 167)], "app/ui/controls.rs": [(1, 207)],
    "app/ui/dialogs.rs": [(1, 31), (59, 361)],
}
lines = []
for path, spans in regions.items():
    source = subprocess.check_output(
        ["git", "show", f"{rev}:frontend/src/{path}"], text=True).splitlines()
    for first, last in spans:
        lines.extend(source[first - 1:last])
print(len(lines), sum(bool(line.strip()) for line in lines))  # 7030 6973
```

## Validation

All Rust commands use managed `/usr/local/bin/cargo`.

- Workspace compiler check, all targets: passed.
- Workspace nextest suite: **290/290 passed**, zero skipped, run
  `305445b0-2283-4212-9e6c-538e47ac82dc` (98.592 seconds of tests).
- Windows MSVC `cargo xwin check`, Android aarch64/API-29 compiler check and Java
  SDK-35 compilation: passed. Android reports five platform-only dead-code
  warnings (four pre-existing, plus unused wheel/preedit event construction);
  Java reports its existing deprecated-API note. Neither is a release gate here.
- Twelve added regressions cover frame-independent focus/replacement, opaque
  input scopes, captured and clipped hits, IME priority, native/clipboard lifetimes
  including window blur and navigation round trips, FIFO/stale requests, async
  completion ownership, idle settling and code-browser subscription lifetime.
- Existing GUI assertions were adapted to retained controls, not removed. The
  new async UI tests explicitly inject completion state; the full workspace also
  runs its existing real-daemon/native transport suites. They are not a claim of
  physical-device coverage.
- Real GPU headless desktop/phone previews: connection, tools and topic prompt.
  Set `TAU_RETAINED_UI_PREVIEW_DIR` when running the new dialog tests to reproduce.

Physical Android keyboard/device QA and interactive Windows QA remain open.
The inline-input branch was not merged. Its editor-ID/revision fence must replace
the temporary window adapter while retaining source and widget-lifetime safety.

## Inline-input integration (subsequent checkpoint)

Adapted the existing `46618b9` implementation, not a second phone editor. Java's
same-window InputConnection now edits both retained fields and the shared chat /
code composer. Field IDs, Rust-edit revisions, account/source/chat bindings and
clipboard tokens reject detached callbacks. Send retains the active keyboard;
keyboard insets retain the buffer. Compact connection fields remain above the IME.
Code-reference changes rebind the editor and fence old snapshots instead of
rewriting an old fullscreen editor's eventual returned text. Composition still
defers live-reference updates; user prose is preserved.

Validation: 21 focused nextest cases passed (existing code-view tests plus adapted
inline-input and retained-dialog tests), Android Rust check and SDK-35 Java compile
passed. This remains host/headless validation, not physical-device acceptance.
Stages 2–4 are actively continuing; this checkpoint is not the end of the task.

## Remaining dialogs

Model preferences/suggestions, staged daemon settings, catalogue refresh, chat
rename/delete, agent controls, queue edits, link confirmation, recovery and saved
intent inspection now own retained forms. `Modal`, `ModalKind`, root daemon-draft /
save-completion state, modal editor indices and the retained-to-legacy dialog bridge
are deleted. Captured queue/agent destinations are explicit, not whichever chat is
selected when Submit runs. Replacement requests validate their originating owner.

Deliberate UX choice: download notifications do not navigate away from an open
form. Their typed destination remains intact; close the form to follow it. The
navigation regression now checks that boundary and still verifies the real target,
outgoing draft, source/topic binding and measured destination.

Validation: frontend suite initially 170/172; fixed the offline settings fixture
and revised notification precedence, then both regressions passed. All 13 retained
dialog tests passed, including two new replacement/completion lifetime tests.

## Overlay ownership checkpoint

ImageViewer owns pan/pinch/background gestures and buttons; Menu owns its captured
controls, keyboard selection, submenu and scrolling; NoticeWidget owns popup
controls/lifetime; TooltipHost owns tooltip presentation and dismissal. Menu choices
are local to menus, including transcript context choices. The old context-menu
renderer, root menu keyboard/scroll dispatcher and root viewer gesture/render paths
are removed. Download/export lifetime state now belongs to Services, not the legacy
workspace. Legacy workspace hover anchors are still an adapter until that subtree
is migrated; the attachment/scroll pilot and stages 3–4 are still in progress.

After disk space was restored, the managed all-target frontend check completed.
27 focused nextest cases passed, including actual menu/submenu, tooltip, image
viewer, retained-dialog and download rendering/interaction tests. A preceding
14-case run covered notice/navigation/inline-input/viewer paths. Physical-device
acceptance remains separate from headless checks.

## Nested routing and sidebar checkpoint

AttachmentCard now owns its actual clipped, retained controls, shared by transcript
and attachment browser. AttachmentBrowser owns scrolling, paging interest and card
reconciliation. A real headless nested-input pilot verifies child-button capture,
scroll-parent takeover without activation, pointer-ID isolation and cancellation
when a captured card disappears. This is exercised through App's real event entry
points, not a flat target-routing simulation.

Sidebar and project tabs now own retained controls, context/hold behavior and their
own ScrollStates. The corresponding root lane switches, parallel project hit
areas, inertial state and old project renderer have been removed. Tests inspect
placed retained controls through test-only selectors; those snapshots are never
used to dispatch input. Retained children are reordered in paint order while
keeping their identities (two ordering regressions caught by project tests were
fixed and all five project tests rerun successfully).

Checks: managed frontend all-target compilation; 6 attachment presentation/action
cases; 13 nested/navigation/attachment cases; 15 initial passing sidebar/control/
scroll cases plus the two corrected ordering cases in the five-case rerun.
Composer, code browser and transcript ownership still require completion.

## Shared composer and code-browser checkpoint

Composer now owns its retained TextField, controls and slash completion; quick
models own retained choices. CodeBrowser owns its toolbar, search TextField,
per-view ScrollStates, source-bound subscription and line-selection gestures.
The existing Document/Selection/reference-update algorithms remain in place.
There is still one composer shared with code comments, not a second editor.

Field-index routing and the legacy native-input destination variant are deleted.
IME and clipboard destinations are now retained Targets with source/chat lifetime
checks. The root resolves the focused dialog/composer/search field; buffer changes
rebind the IME revision without reopening the keyboard. New-form focus is installed
after old-form teardown, and return-focus identity is captured before construction.

All 26 focused composer, code-browser, editor, inline-mobile and retained-dialog
nextest cases passed after the migration. Workspace/transcript orchestration and
removal of the remaining global Action/hit adapter are the final implementation
stage, followed by full and cross-platform validation.

## Final ownership migration (source checkpoint)

Workspace/ChatPane/Header/Transcript/MessageRow now own the remaining live UI.
The production LegacyWorkspace, global Action/Hit/area tables, Lane switches and
root navigation reset lists are removed. The existing row projection is one
adapter, not a new domain model. Retained row controls are bounded to the existing
overscan plus captured owners; lightweight placements and anchor formulas remain.
Test selectors are explicitly test-only FixtureChoice/PlacedControl and fixture
setup delegates to real owners, not a second production dispatcher.

Active focus/capture/native paths include concrete ancestors and are reconciled
before the next event as well as after structural changes. Six added regressions
cover streaming/reorder identity, source replacement, hidden content before paint,
code-search IME insets, nested selection capture, ancestor detachment and visible
form errors. Existing viewer coverage now checks jitter, pinch over toolbar and
window cancellation. No backlog-012 restoration tests were added or removed.

Managed all-target workspace compilation passed. Full workspace nextest passed
**305/305**, zero skipped (`9634ee5e-ecd8-49db-a453-0cccaa7d077f`, 105.837s).
Windows MSVC compiler check and SDK-35 Java compilation passed. Android compiler
validation and the final all-file cost ledger follow this checkpoint. New owner
code is expanded into readable formatting; the cost comparison must normalize
both revisions, not credit previously condensed source as a simplification.
