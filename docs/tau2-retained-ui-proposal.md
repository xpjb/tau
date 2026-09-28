# Tau 2 retained UI: concrete replacement proposal

September 28, 2026. **Proposal, not an implemented refactor.** Source and counts
are pinned to `cafef7f7ad57e1c5bae6e3e9c89c66beebade80e` on
`tau2-integration`. The review branch is `review/tau2-retained-ui`.
Only this document is being added; no application behavior is changed.

## 1. Recommendation and what must get simpler

Use Burritofactory's basic shape: retained objects, a small `Widget` interface,
explicit parent-to-child calls, and generously injected mutable model/UI/services.
Keep concrete child fields wherever the composition is known. Keep one coherent
`visit_perframe` that can animate, measure, place children and paint.

**This is not a proposal to distribute the existing `App` over more files.** The
finished implementation must retire these mechanisms, not reproduce them behind
new names:

1. The application-wide, 73-variant `Action` enum and its central dispatcher.
   A button's owning component handles its activation next to its state.
2. Root-owned, parallel hit/action tables and separate interpretations of those
   tables in hover, press, release, keyboard focus and context-menu code.
3. Global scroll-lane selection and repeated switches between unrelated panes'
   offsets, extents, velocities and drag state.
4. Root-level modal/editor indexing and repeated overlay-precedence checks.
5. Long lists of unrelated interaction fields to reset when navigating. Retained
   subtree ownership and one binding/cancellation boundary replace those lists.

The model's real behavior, layout formulas and platform interop do not magically
vanish. Moving them is **relocation**, not a line-count saving.

The measured production scope is **6,100 physical lines**. The design budget is
**5,200–5,750 replacement production lines, including new infrastructure and small
interop changes**: a target of **350–900 fewer lines, roughly 6–15%**. These are
planning targets, not measured savings or a claim that the refactor is certain to
shrink. Section 9 gives the complete accounting and stop conditions.

## 2. Keep these things; do not build competing versions

- **Chad:** window/device events, surface lifecycle, update/frame entry points,
  desktop on-demand redraw, and the Android host.
- **Sanscale and `Renderer`:** shared text shaping/cache, caret/hit geometry,
  Markdown `Document`/`Preview`, image resources and layer submission. Tau already
  retains message documents; this is not an immediate-to-retained text rewrite.
- **`Editor`:** value, selection, undo, composition, scrolling and shaped view.
  Add a small `TextField` interaction wrapper; do not write another editor.
- **Controller, storage, transport, native blocks, downloads and domain rules.**
  Initially widgets receive the existing `Controller`. This proposal does not
  require the separate client-state/replication proposals to land first.
- **Current navigation guarantees:** controller selection remains authoritative;
  outgoing draft/anchor ownership and account/source validation remain intact.
  Selecting a chat must bind its composer before the next input, without a tick
  or frame in between. Preserve `app/navigation_tests.rs`.
- **Existing presentation and platform behavior:** responsive panes, touch,
  long-press, pinch, mouse selection, IME, copy/paste, file/export actions, menu
  keyboard controls, tooltips, scroll restoration and download-notice navigation.

The separate client-state/tool-projection work may simplify `rows`, tool grouping
and copy/content-interest semantics. **Do not claim those deletions here.** Consume
that projection if it has landed; otherwise keep the existing interpretation in
one adapter during this UI refactor. Do not create a competing transcript model.

Broad access is intentional. A widget may update the model, call services, change
its own interaction state, or request navigation. This proposal does not introduce
read-only view-model facades, capability slices, a universal command bus, a general
layout solver, or a new UI crate.

## 3. Ownership: a concrete target

Names below are proposed APIs, not existing or compiler-checked declarations.

```rust
struct App {
    root: RootWidget,
    model: Controller, // existing model; not a duplicate of Controller
    ui: UiState,
    services: Services,
}

struct UiContext<'a> {
    model: &'a mut Controller,
    ui: &'a mut UiState,
    services: &'a mut Services,
}

trait Widget {
    // true means consumed, NOT "the model/text changed".
    fn handle_event(&mut self, event: &UiEvent, cx: &mut UiContext<'_>) -> bool;
    fn visit_perframe(&mut self, frame: &mut Frame<'_>, cx: &mut UiContext<'_>);
}
```

`root`, `model`, `ui` and `services` are siblings so Rust can borrow them separately.
In particular, `UiState` must not also own the widget currently receiving
`&mut self`. This is a borrowing requirement, not an access-control doctrine.

The trait earns its place by enforcing the two entry points. It does not need a
`Vec<Box<dyn Widget>>` or a generic layout engine to justify itself. It also does
not enforce correct traversal order; implementation and tests must do that.

```text
RootWidget
├─ Workspace
│  ├─ Sidebar
│  │  ├─ ProjectTabs
│  │  ├─ ChatList                         keyed ChatRow widgets
│  │  └─ ConnectionIndicator
│  ├─ ChatPane
│  │  ├─ Transcript                      retained row/section interaction state
│  │  └─ Composer
│  │     ├─ TextField<existing Editor>
│  │     ├─ attachment controls
│  │     └─ QuickModelPicker
│  └─ AttachmentBrowser                  shares AttachmentCard with transcript
├─ OverlayHost                           one owner of overlay order and modality
│  └─ Overlay entries                    enum: dialog, menu, image viewer
├─ NoticeWidget
└─ TooltipHost                           existing Tooltip behavior/content
```

Workspace keeps the sidebar, chat and attachment browser as concrete fields. Its
layout decides which are visible: the attachment browser remains a side pane on
wide desktop windows and a replacement screen when narrow, as today. Hiding a
pane need not destroy its remembered state, but it removes its active input route.

`OverlayHost` owns a small ordered collection of an `Overlay` enum. Existing
flows choose explicit open, replace or close operations; merely introducing a
stack must not change Back behavior or make every settings transition a push.
Menu submenu history stays local to `Menu`, not a separate application scene.

Use concrete fields for fixed controls, `Vec<T>`/keyed collections for homogeneous
rows, and small enums for the closed dialog/overlay families. A dialog enum with
two delegation matches is fine, just as Burrito's `Scene` enum is fine. Do not
box every label, row or button. Use `Box<dyn Widget>` only if a real heterogeneous
collection benefits from it; no such collection is a prerequisite here.

### Shared state versus local state

| Owner | Owns | Must not become a second authority for |
|---|---|---|
| `Controller` | Existing chat/topic selection, drafts, persisted view preferences, operation and transfer state | Widget geometry or current pointer capture |
| `UiState` | Window metrics, active focus route, pointer/contact capture, structural requests, redraw/deadline requests | Editors, row contents, selected chat or a second widget tree |
| `Services` | Existing renderer/text service, platform action outbox, host wake access and export coordination | Independent copies of model data |
| `Workspace` binding | Which account/source/chat its retained views currently represent; pending reveal destination | Which chat the controller has selected |
| Each widget | Its controls, placed bounds/clip, animation, local scroll/selection gesture state | A copied editable chat/session model |
| `TextField` / `Composer` | Existing Editor and its interaction lifetime | A second persisted draft; accepted edits still update the existing model |

`pending_exports`, `export_targets`, `saving_downloads`, `export_errors` and source
fencing move together into the existing export workflow's coordinator. It can
outlive a card or viewer. Moving it to `Services` is not deleting it, and a hidden
widget must not be required to finish an OS save.

## 4. Event and frame mechanics

### 4.1 One explicit tree walk, not a new reflection framework

Keep manual delegation. Parents declare front-to-back child order and stop when
a child consumes an actionable event. Children handle their own controls before
the parent's fallback behavior. Painting uses the corresponding back-to-front
order. Do not maintain separate, independently ordered "hover children" and
"click children" lists.

`UiEvent` normalizes the existing desktop/Android inputs without losing contact
IDs, mouse buttons, modifiers, wheel units, composition or cancellation. It can
also represent a hover probe or timer notification; there is no need to put raw
Winit types throughout widgets.

Focus/capture stores a short route of retained owner IDs. Each parent forwards a
targeted event to the next concrete field or keyed child on that route. Pointer
hit traversal establishes the route; new-dialog focus can name it at construction,
without waiting for geometry. Detaching a subtree invalidates active routes
containing its owner ID. This stores paths for the few active targets, **not a
second editable widget tree** or a registry of mutable widget references.

For pointer targeting, use each widget's **last presented bounds and clip**.
A small shared placed-hit/control helper owns the rounded/rectangular containment
rule. Hover probes and button presses use that same rule and child order.

- A press uses its current coordinates against those bounds. It never trusts a
  previous frame's `hovered` boolean, which is the relevant Burrito pitfall.
- `ui.hot` identifies the current hover target and cursor/tooltip nomination.
  An old target stops being hot without every widget needing a separate global
  area-table lookup. Geometry changes trigger another hover probe through the
  same route; they do not trigger a second text/layout implementation.
- Empty modal backgrounds and disabled controls have explicit blocking behavior;
  "not activated" must not accidentally mean "click through to a lower layer".
- Captured motion/release goes to its owning scope and widget even outside its
  rectangle. Containers forward captured events before applying geometric gates;
  non-owning controls ignore them. Explicit child calls suffice initially: no
  mutable-reference registry, downcasts, or required third traversal trait.
- `ScrollView` may cancel an armed child press and take its gesture after the
  existing drag threshold. That is an explicit handoff, not a later search for
  whichever action happens to occupy the release coordinates.
- A removed/hidden target cancels its gesture. The remaining release is swallowed,
  not re-targeted at a newly exposed control. Window focus loss cancels contacts,
  selection drag and composition through the same ownership route.

There is no prohibition on a compact **local** hit index inside a complex row or
Markdown view. The change is removal of the application-wide semantic hit tables,
not a claim that storing rectangles or doing linear hit tests is bad.

### 4.2 Focus and overlay precedence

The root chooses the active input scope; each container chooses its focused child;
the editor owns caret and selection. Keep a stable widget identity in the focus
route, not `Option<Option<usize>>` or the position of a rendered hit entry.

For pointer events, the root walks the actually visible overlay/notice layers in
reverse paint order, then the workspace if the foreground layer permits it.
Preserve today's notice suppression on the full-page settings/project surfaces.
A modal blocks the background even when no foreground control handles the event.
A menu's outside-dismiss click is consumed rather than replayed behind the menu.

Keyboard/IME events go to the active focus scope and its focused leaf. Unhandled
shortcuts then unwind to the owning container/root. The focused editor gets the
opportunity to handle IME composition before Escape/Enter turn into navigation,
abort or send. Dialog-local Enter and Tab policies remain explicit.

A form knows its fields and tab order directly, including disabled/hidden fields;
it does not rediscover them from `Action::Focus` entries produced by painting.
Remembered focus in an inactive pane is fine; only one route is active. Opening
an overlay saves the prior route; closing restores it only if its target remains
valid. Pointer capture is separate from keyboard focus.

**Do not reuse `Editor::key`'s boolean as event consumption.** That result says
whether text changed; caret movement and composition handling can consume input
without changing text.

### 4.3 Local activations, broad injection

A reusable button handles press/release/keyboard interaction and exposes a local
activation. Its parent applies the feature behavior in the same event dispatch:

```rust
let handled = self.send.handle_event(event, cx);
if self.send.take_click() {
    let result = cx.model.send_prompt();
    cx.report(result);
}
if handled {
    return true;
}
```

`take_click` is drained immediately, not left waiting for the next paint. The same
pattern can expose an editor change to the composer, which writes the model draft.
A custom widget can instead implement its behavior directly. Do not introduce a
callback framework merely to shorten these few lines.

Small local enums remain useful for menu choices or parameterized confirmations.
They are not an excuse to move all 73 application actions into a new central enum.

### 4.4 Structural requests: keep a small buffer

Choose a small request buffer over returning navigation outcomes through every
ancestor. Requests can originate from input, an asynchronous completion or a
perframe visit. Ordinary model/service operations remain direct.

The buffer is limited to operations that require an ancestor/other subtree to
change after the current borrow has ended:

```text
Open(OverlaySpec)
Replace(OverlayId, OverlaySpec)
Close(OverlayId)
Back
Navigate(Chat | Project | NewChat)
RevealAttachment(DownloadTarget)
SetAttachmentPaneVisible(bool)
```

Navigation belongs here because the owner must coordinate the outgoing view,
controller selection and incoming view. This is not a general delayed model
command bus: sending a prompt, toggling a model preference or starting a download
does not need to travel through it.

`RootWidget::finish_dispatch` is the single owner-side application boundary:

1. Drain requests in FIFO order; do not `take` a vector and return midway through
   it, losing its tail as Burrito/newbase can do during a transition.
2. Use explicit overlay IDs, not an unqualified "pop whatever is now on top".
   Requests carry their originating scope/binding; stale requests cannot act on
   a replacement account/view. A compound replacement is one `Replace`, not a
   close followed by an open that depends on an already-destroyed origin.
3. Save/reconcile the outgoing view using its bound identity. If the controller
   has already changed account, never save old geometry through the new account.
4. Apply the controller navigation and rebind the affected retained subtree;
   preserve the existing draft/anchor/download-location rules.
5. Cancel invalid capture, reconcile focus/IME/tooltip owners and request redraw.

Run this boundary **after every input and after model/async update handling,
before the next input**. Also drain requests after `visit_perframe`, outside its
borrow. Newly opened pointer targets have no valid presented geometry until
painted, but their designated editor may receive keyboard input immediately.
An event that opened/closed something is not replayed into the replacement.

No scene-transition animation system is needed for Tau's first implementation.
If added later, it must not postpone model/editor rebinding or discard requests.

### 4.5 One perframe visit, Chad and Sanscale unchanged in role

`Frame` carries available bounds/clip, time, scale and draw layers, with access to
Chad's render context. The renderer/text service remains in `Services`; do not
simultaneously borrow it into `Frame` and again through `UiContext`.

A component's `visit_perframe` may read/write the model, advance animation,
measure text, place/visit children, paint and request another frame. There is no
mandatory measure/layout/update/paint split. The host submits the completed
layers and then finalizes structural requests outside traversal.

Keep visibility-driven content/preview demand where it is useful. Transcript and
attachment visits can submit bounded, deduplicated interests through the existing
controller/cache APIs. Do not duplicate fetching state in widgets or require
hidden rows to wake up to finish background work. Preserve the pinned baseline's
expanded prefetch margins and finite ongoing-chat catch-ups.

Chad's event path forwards normalized input; its update path keeps transport,
completion and nonvisual scheduling work; its frame path visits the tree and
submits layers. On-demand liveness is explicit: input/model updates invalidate
the view, hover/timers arm existing wake/redraw paths, and running animations
request their next frame. A paint must not be required to discover the first
reason to paint. Do not switch the application to continuous redraw as a shortcut.

Sanscale remains the owner of shaping, hit/caret geometry and text caches.
Widgets retain handles and Editor state, not a competing text layout cache.

## 5. Stable identity without a second model or UI engine

Assign a non-reused `WidgetId` to each retained interaction object. Key dynamic
rows by the existing stable row/item identity **scoped to account/source/chat and
surface**. A transcript card and an attachment-browser card are different widgets
viewing the same model attachment; progress comes from the same transfer record.
Do not use vector position or the address of a moved Rust value as identity.

On a dirty frame, initially run the existing row projection and reconcile by key.
Existing rows retain interaction state; new rows start unplaced; removed rows
cancel their focus/capture/tooltip ownership. There is no new reactive dependency
graph and no requirement to cache every derived model query. Growing text updates
the existing message document/handle rather than replacing its widget identity.

Keep current measured row placements and virtualization/content-interest bounds.
Retain interaction widgets for the realized window/overscan and active gesture
owners, not an unbounded heavyweight widget for every historical event. Offscreen
placement/anchor records may remain lightweight data. Account/source changes
invalidate the corresponding view binding and renderer keys; equal chat/entry
strings in a different source must not inherit old selection or gestures.

Native mobile editing also needs a target identity. At this baseline,
`PlatformAction::Edit`/`NativeEvent::Edit` carries text but no editor token
(`android.rs:197–215,343`, Java `MainActivity.java:48–96`). Carry the originating
editor token through that existing bridge and ignore results for a detached
editor, rather than applying late text to whatever currently has focus. This is
a small, counted bridge adaptation, not a replacement Android input system. If
the parallel inline-input work has landed first, preserve its path and apply the
same ownership rule instead of restoring the old fullscreen editor.

## 6. Exact replacement ledger

References below are to the pinned revision. Line spans describe the old source;
they are not assertions that every line in a span can simply be deleted.

| Existing implementation | Proposed owner/replacement | What is actually removed or streamlined |
|---|---|---|
| `app.rs:33–108` `Action`; `1556–2171` `activate/apply` | Each feature widget/dialog handles its controls locally; root handles only structural requests | Delete the global enum and dispatcher. Preserve controller calls, validation, error reporting and platform effects in the owning method. |
| `App.hits`, `chat_areas`, `project_areas`, `message_areas`, `detail_areas`, `info_areas`; frame resets `2199–2215` | Retained child/row/control bounds and clip; local Markdown geometry; `TooltipHost` | No root reconstruction/lookup of parallel semantic tables. No second focus list inferred from paint output. Local placement data remains. |
| `hover/cursor` `511–605`; `press/motion/release/cancel_pointer` `1066–1379` | Shared placed-hit/control behavior, pointer capture bookkeeping, and owning `ScrollView`, TextField, row or viewer | Remove repeated scene/target classification from every input entry point. Retain feature-specific gestures in their owners. |
| Editor helpers and keyboard switching `1380–1555`; `focus: Option<Option<usize>>` | Focus route, TextField and dialog/form-local policy | Remove composer-versus-modal-index selection from the root and paint-derived Tab order. Keep all Editor capabilities. |
| `Lane`; `scroll_value/set_scroll` and scroll dispatch `914–1047`; lane branches in `tick` | Owned `ScrollState`/`ScrollView` instances, parameterized by axis | Delete global lane switches. Reuse wheel filter, scrollbar and autoscroll mathematics. Transcript anchors are still transcript-specific. |
| `tick` `615–846` | Host model/completion pump plus widget-owned animation/status timing and redraw scheduling | Remove root polling of unrelated UI fields, not real transport/export/status work. Never make background completion depend on visibility. |
| `frame` `2172–2474` | Root/Workspace composition, overlay policy, viewer, attachment browser; existing layer submission | Remove the repeated top-level mode tests and global hit/reset lifecycle. Keep responsive layout and rendering. |
| `sidebar` `2475–2641`; `projects.rs` tabs/context/scroll | Sidebar, ProjectTabs, ChatList, ConnectionIndicator, shared Menu | Co-locate each row's click/context/hover rules; use the same menu and scrolling primitives. Topic operations retain their current controller APIs. |
| `rows` `2642–2890`; `details.rs`; tool interpretation within `chat` | Existing projection adapter feeding retained rows, or the separate typed tool projection when available | **Relocate/consume; no domain-reconciliation saving credited.** Do not re-derive native relationships inside new widgets. |
| `chat` `2891–3580`; `quick_models_*` `3581–3713`; `composer_status.rs` | Transcript/MessageRow, Composer and QuickModelPicker | Remove repeated global hit/action registration and unrelated state access; retain layout formulas, affordances, anchoring and content interests. |
| `notice_frame` `3714–3744`; `app/notices.rs` | NoticeWidget | Keep notice identity/deadlines and typed download destination; own body/close controls and consumption together. |
| `context_at` `3745–3842`; menu code in `projects.rs` | Shared Menu with local typed choice handlers | One implementation of selection, submenu, keyboard, scrolling and outside dismissal. Feature-specific menu contents remain data. |
| `icon_button` `3843–3898`; free `button` `4650–4693`; repeated field chrome | ButtonControl/Button and TextField/FormRows chrome | Share interaction and chrome, not only drawing. Parent code states label/style/placement and activation behavior; no `hits` parameter. |
| `usage_frame/info_frame` `3899–3950`; `tooltip.rs` and `tooltip/text.rs` | TooltipHost with owner-bound content and existing Tooltip | Remove separate root target lookup/pinning paths; reuse the rich text, timing, hover bridge and placement code. |
| Settings/model/daemon forms `3951–4577`; `modal_frame` `4578–4649`; project forms | Typed dialog widgets with shared field/button-row drawing and focus policy | Replace root `ModalKind` branches and repeated form plumbing. Keep field validation, connection handshake wait, daemon revision/save and recovery semantics. |
| `attachments.rs` card/export workflow; viewer branch in `frame/apply` | Shared AttachmentCard, ImageViewer, export coordinator | Card controls behave identically on both surfaces. Viewer owns zoom/pan/pinch; OS export completion survives widget removal. |
| `app/navigation.rs` and scattered navigation resets | Workspace binding/reveal logic called from the owner boundary | Keep one reconciliation route, replace manual unrelated-field resets with subtree rebinding/cancellation. Do not create another selected-chat field. |

### Destination of every existing Action variant

This inventory prevents removing a switch while losing one of its less visible
behaviors. A name below describes the old action, not a required new command type.

| Old variants | New activation owner |
|---|---|
| `Select`, `New`, `SelectProject` | ChatList/ProjectTabs request coordinated navigation |
| `NewProject`, `RenameProject`, `ProjectPrompt`, `DeleteProject`, `RemoveProject` | Topic dialog and its local confirmation/submit methods |
| `MoveMenu`, `ContextBack`, `MoveChat` | Menu/submenu state and local chat-menu handler |
| `RetryCreate`, `Rename`, `Delete`, `Clone`, `Sleep` | Chat row/menu and parameterized chat dialogs |
| `Settings`, `ModelSettings`, `DaemonSettings`, `CancelModal`, `Back` | Opener widgets request structural change; OverlayHost/Workspace own Back |
| `ResetModels`, `ToggleQuickModel`, `ChooseModel` | Model-preferences dialog and QuickModelPicker |
| `SettingsSection`, `SettingsField`, `SettingToggle`, `SettingReset`, `AgentSetting`, `AgentCommand`, `RefreshCatalog` | Daemon/model dialog and agent/model controls |
| `CopyDiagnostics`, `ClearReplica`, `CopyRecoveredDraft`, `ForgetRecovered`, `ReviewRestore` | Connection/recovery dialog methods |
| `Outbox`, `InspectControl`, `CheckControl`, `RetryControl`, `ForgetControl` | Saved-actions/control dialogs |
| `Focus`, `Confirm`, `Noop` | TextField/focus routing; dialog-local submit; explicit input blocker. These disappear as global actions. |
| `Send`, `Abort`, `Attach`, `RemoveFile`, `Suggest` | Composer and its concrete child controls |
| `Tail`, `History`, `Toggle`, `Copy`, `CopyDetails`, `CopySelection`, `Link`, `Fork` | Transcript/section/menu methods and local link confirmation |
| `Restore`, `Dismiss`, `RetryPending`, `Queue`, `EditQueue` | Message/queue row and queue-edit dialog |
| `Attachments`, `Attachment`, `SaveAttachment`, `UseSaved`, `CancelDownload`, `Zoom`, `Fit` | Attachment opener/card/viewer; export coordinator for async work |
| `Usage`, `Info`, `DismissNotice`, `OpenDownloadNotice` | Tooltip-owning controls and NoticeWidget; download destination uses the navigation boundary |

## 7. Concrete primitives, and what not to generalize

Build only these shared pieces initially:

- **PlacedHit + ButtonControl/Button:** stable target, bounds/clip, press/release,
  enabled/blocking behavior, activation, hover/cursor and existing visual feedback.
  A custom attachment/section control can reuse ButtonControl without adopting
  the label-button appearance, as Burrito's UpgradeCard does.
- **ScrollState/ScrollView:** axis, offset/extent, wheel easing, drag/inertia,
  scrollbar and optional middle-button autoscroll. Transcript's follow-tail,
  expansion pin and persisted anchor stay in Transcript; viewer zoom/pan stays in
  ImageViewer. Do not turn every coordinate into a universal scrolling policy.
- **TextField:** the existing Editor plus focus/IME/platform integration and
  shared field chrome. **FormRows** is a modest placement/drawing helper for
  repeated label/field/button rows, not a declarative form language. Dialogs own
  their validation and typed fields.
- **Menu:** options, local selection/submenu stack, scroll and keyboard handling.
- **OverlayHost:** order, input blocking, focus restoration and structural change.

Keep simple labels and decorative drawing as ordinary draw calls. Do not add
widgets solely to wrap one renderer call. Keep the useful existing `Tooltip`,
`Ripple`, text-content helpers and scrollbar mathematics rather than replacing
working algorithms for stylistic uniformity.

## 8. Migration with deletion gates

Do not begin with a full framework implementation. Land one representative
vertical slice, then expand only when it demonstrates actual reuse and deletion.
Implementation work belongs on fresh branches based on then-current integration;
reconcile concurrent UI changes before using these pinned line references.

### Stage 1 — prove the interface and owner boundary

Introduce sibling borrowing, the two-method trait, minimal input/focus/request
support, Button and TextField. Migrate the connection settings dialog and a topic
rename/prompt dialog as **one closed overlay subtree**, including their input and
submit behavior. These exercise reused chrome, IME, tab order, asynchronous
connection results, validation and structural replacement.

The legacy workspace may temporarily be one lower-priority adapter. Only that
adapter owns its old hit tables; a migrated overlay never registers old hits or
calls the old Action dispatcher. The root decides which path receives an event,
so an overlay event cannot run both systems.

**Deletion gate:** remove those dialogs' old drawing branches, Focus/Confirm
branches and hit registration as soon as their replacements work. Leave no dual
implementation or permanent "new UI" toggle. Demonstrate navigation/focus changes
before a frame and no click-through. This stage may increase total LOC while
shared primitives have only two consumers; record that cost rather than hiding it.

### Stage 2 — finish overlays and form/menu reuse

Move remaining settings/project/recovery/control dialogs, Menu, TooltipHost,
NoticeWidget and ImageViewer. Keep operation logic and user-visible flow intact.

**Deletion gate:** remove `Modal`, root modal/editor-index routing, root menu
keyboard/scroll branches, viewer input branches and separate tooltip target paths.
A legacy composer can participate through one explicit focus adapter temporarily;
do not retain the global modal-index scheme for it.

### Stage 3 — workspace, composer and owned scrolling

Move Sidebar/ProjectTabs/ChatList, Composer/QuickModelPicker and AttachmentBrowser.
Share AttachmentCard with the transcript path. Bind editors and gestures to their
source/view identity; adapt native edit tokens without replacing platform UX.

**Deletion gate:** remove the global Lane selector, pane scroll-value switches,
composer-versus-modal editor dispatch and migrated global Action variants. Keep
only a clearly identified legacy transcript adapter until Stage 4; it must use
owned ScrollState too, rather than keeping the whole Lane switch alive.

### Stage 4 — transcript and final removal

Move the existing row projection behind one adapter, reconcile retained
row/section interaction state, and move measured placements, anchors, expansion
pin, selection gestures, reveal destination and content interests to Transcript.
Preserve or consume the independently landed typed tool projection.

**Deletion gate:** remove the last `Hit { rect, action }` path, root area tables,
remaining Action dispatcher, legacy adapter and global reset lists. Check the
whole frontend diff for relocated code and new scaffolding. Publish actual counts
against the updated baseline and this budget, with tests counted separately.

A stage is not complete because its new widget files exist. It is complete when
its old implementation is gone and preserved behavior is covered.

## 9. Line-count impact: measured baseline and explicit budget

### Measured baseline

Physical source lines include comments and blank lines. Test-only modules and
inline test blocks are excluded; the two-line test-only import in attachments is
also excluded. This is a deliberately specified **replacement scope**, not an
assertion that the entire frontend is 6,100 lines.

| Source within `frontend/src` | Included production region | Lines |
|---|---|---:|
| `app.rs` | 1–4754, before test modules | 4,754 |
| `app/attachments.rs` | 1–477, excluding test-only import lines 2–3 | 475 |
| `app/projects.rs` | entire file | 271 |
| `app/navigation.rs` | entire file | 153 |
| `app/notices.rs` | 1–29 | 29 |
| `app/ripple.rs` | 1–61 | 61 |
| `app/composer_status.rs` | entire file | 25 |
| `scroll.rs` | entire file | 86 |
| `tooltip.rs` | entire file | 172 |
| `tooltip/text.rs` | entire file | 74 |
| **Total** | **6,064 nonblank lines** | **6,100** |

For orientation, some large, nonoverlapping spans **within `app.rs`** are:

| Span | Lines | Interpretation |
|---|---:|---|
| `activate/apply`, 1556–2171 | 616 | Dispatcher disappears; most actual feature operations must survive locally. |
| Pointer/editor/key path, 1066–1555 | 490 | Replaced by shared routing plus owning widget behavior, not zero lines. |
| Top-level frame, 2172–2474 | 303 | Becomes composition plus separate concrete views; real drawing survives. |
| Row projection, 2642–2890 | 249 | No semantic simplification credited to the UI proposal. |
| Chat rendering, 2891–3580 | 690 | Mix of transcript, composer and controls; moving all 690 is not saving 690. |
| Settings/modal rendering and field helpers, 3951–4649 | 699 | Repeated field/control plumbing is a reuse opportunity; validation and layout remain. |

Preserved implementations outside the budget's baseline include `editor.rs`
(511 total file lines), `render.rs` (976), `icons.rs` (293), `fonts.rs` (248),
`details.rs` (231), `daemon_settings.rs` (280), and the desktop/Android hosts.
Those file totals include their tests where present; **none is a deletion credit**.
Any net additions to these or other files for this UI work must still be charged
to the new-code budget. Moving code into Controller does not make it free.

### Proposed destination budgets — not measurements

These are allocations for the final implementation, **including carried-over
code**, not expected additions on top of all 6,100 existing lines. Boundaries
between files are not prescribed; each line gets counted once.

| Destination responsibility | Production LOC budget |
|---|---:|
| Thin App/Root/Workspace composition and navigation binding | 400–450 |
| Shared input/focus/capture, identity and structural-request handling | 400–460 |
| Button/control, ScrollView, TextField and form-chrome reuse | 600–670 |
| Sidebar, ProjectTabs, ChatList and connection presentation | 540–590 |
| Chat shell, Composer, QuickModelPicker/status | 520–580 |
| Transcript/rows/sections/anchors and retained projection adapter | 980–1,050 |
| AttachmentCard/browser, viewer and export workflow | 650–700 |
| Dialogs, overlay/menu policy, notices and existing tooltip implementation | 970–1,070 |
| Net renderer/platform-bridge adaptations outside the baseline | 40–60 |
| Retained formatting/Ripple/other small presentation helpers | 100–120 |
| **Total replacement production budget** | **5,200–5,750** |
| **Target change versus 6,100** | **−900 to −350** |

The reduction hypothesis is local activation instead of action serialization and
redispatch; one focus/gesture/scroll policy instead of repeated global switches;
and reused field/control/menu plumbing. The new cost is retained constructors,
identity/reconciliation, shared routing and the structural-request boundary.
Large domain handlers, text engines and real layout are mostly carried over.

The riskiest estimates are transcript reconciliation and dialog plumbing. If
explicit retained construction adds as much boilerplate as the old hit tables
removed, production size may be flat or even grow. This budget does **not** turn
that uncertainty into a promised saving. Re-estimate after Stage 1/2, using real
diffs, before committing to the transcript migration.

Allow approximately **350–650 additional test lines** for routing, lifetime and
no-frame-between-events regressions, beyond moving/adapting existing tests. With
the production target, the combined source/test delta could be **−550 to +300**.
This document itself is additional documentation, not production code or savings.

### Counting and acceptance rules

- At each stage report old code deleted, equivalent code relocated, replacement
  code added and genuinely new infrastructure separately. A rename or extraction
  earns zero simplification credit.
- Also report the net production change across **all touched frontend/interop
  files**, so costs cannot disappear outside the table. Report tests separately.
  Do not claim concurrent client-state/tool-projection reductions in this total.
- Use the same formatting/count convention before and after; report nonblank
  counts too. Do not minify code, remove useful comments or weaken tests to meet
  a physical-line target. Rust formatting cleanup is not part of this work.
- No production increase should be called a line-count simplification. If the
  final projection exceeds 5,750, explain the difference and explicitly revisit
  the target; if it exceeds the 6,100 baseline, stop before broad rollout and seek
  a decision on whether the architectural benefit justifies the extra code.
- Regardless of LOC, the five deletion criteria in section 1 are required.
  Conversely, deleting those mechanisms by losing features is not success.

The baseline can be reproduced without compiling anything:

```python
import subprocess

rev = "cafef7f7ad57e1c5bae6e3e9c89c66beebade80e"
regions = {
    "app.rs": 4754,
    "app/attachments.rs": 477,
    "app/projects.rs": 271,
    "app/navigation.rs": 153,
    "app/notices.rs": 29,
    "app/ripple.rs": 61,
    "app/composer_status.rs": 25,
    "scroll.rs": 86,
    "tooltip.rs": 172,
    "tooltip/text.rs": 74,
}
physical = nonblank = 0
for path, end in regions.items():
    text = subprocess.check_output(
        ["git", "show", f"{rev}:frontend/src/{path}"], text=True)
    lines = text.splitlines()[:end]
    if path == "app/attachments.rs":
        assert lines[1:3] == ["#[cfg(test)]", "use tau_transfer::TransferStatus;"]
        lines = lines[:1] + lines[3:]
    physical += len(lines)
    nonblank += sum(bool(line.strip()) for line in lines)
print(physical, nonblank)  # 6100 6064
```

## 10. Behavioral acceptance and evidence

Before removing each legacy path, carry forward its existing tests. In addition:

- Overlap: the visually foremost eligible control owns hover and activation;
  disabled/blocking overlays never leak a click. Pointer coordinates can change
  between a frame and a press without using stale hover state.
- Lifetime: press in A, close/navigate/reorder/remove A, release over B must not
  activate B. Inertia, capture, tooltip and IME cannot attach to a replacement row
  or a same-named chat from another account/source.
- Focus: composition consumes Enter/Escape appropriately, caret-only keys are
  consumed, modal Tab follows actual field order, and focus restoration validates
  the saved target. Native edit callbacks cannot overwrite a different editor.
- Navigation: select chat/project, create chat, or open a download notice and
  immediately send the next input **without any tick/frame**. Draft/anchor/source
  ownership must match `navigation_tests.rs`, including same-chat destinations.
- Requests: two valid requests are handled in order; no tail is silently lost;
  stale targeted close cannot close its replacement; a perframe-originated request
  waits until traversal returns but not until an arbitrary later input.
- Scrolling: wheel easing, scrollbar dragging, middle autoscroll, touch handoff,
  horizontal content, prepend/expansion anchoring and follow-tail remain intact.
  Test the attachment browser as both desktop side pane and phone screen.
- Content: retain bounded preview interests, ongoing-chat prefetch, incomplete
  history handling, incremental Markdown, shared transfer state and delayed OS
  export completion when the originating card is gone.
- Host: desktop redraw settles when idle and resumes for input, timers and async
  events; window focus loss cancels interactions; Android insets/touch/IME and
  viewer pinch remain functional. Keep representative desktop/phone render tests.

Implementation validation should use the managed `/usr/local/bin/cargo` compiler
checks, relevant `nextest` runs, and existing offscreen/platform checks where
available. Do not run Clippy or Cargo's built-in test runner. No Rust build, test,
application or deployment was run for this document; validation here is source
inspection, reproducible counting and document/diff checks only. The documented
counter returned `6100 6064`; a source comparison checked that the action inventory
covers all 73 variants exactly once, and the destination budget sums were checked.

### Reference implementation lessons used

- Burritofactory `e30b98d`: two-method trait, concrete retained children,
  ButtonControl reuse, broad injection and owner-applied scene requests. Its
  `Widget` trait is principally an interface contract, which is sufficient.
- Newbasebuilder `f1a28a4`: very similar copied UI family; not an independent
  shared framework to import. Preserve the useful pattern, not duplicate bugs.
- Realmwright `f3324ea`: retained trait-object containers, but event broadcasting
  without consumption does not supply the desired routing contract by itself.
- Compendium `410ba9b`: pane-local editing/scroll state, active focus/capture
  ownership, popup gating, direct disjoint borrowing and small owner-side effects.
  Its document-content `Widget` trait is a different abstraction, not the UI
  interface proposed here.

**Bottom line:** keep Burrito's straightforward programming model, preserve Tau's
working editor/render/model machinery, and earn the refactor by deleting global
routing and duplicated interaction policy. The trait and the number of files are
not the simplification; the explicit removals and measured replacement cost are.
