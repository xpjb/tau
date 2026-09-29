//! Directory/code surface, shared by desktop and Android. The renderer owns only
//! visible line paint; the worker owns parsing, and selections use source line IDs.
use super::*;
mod paint;
use sanscale::{Align, BlockKey, Draw, PaintHandle, PaintSpan, ParagraphKey, ParagraphSource, Style};
use std::{borrow::Cow, sync::Arc};
use tau_code_viewer::{Document, Selection};
use tau_protocol::files::*;
use ui::controls::{Controls, TextField};
use ui::controls::ButtonStyle;
use ui::scroll::ScrollState;
use ui::{Context, Event as InputEvent, Frame};

pub(super) struct View {
    pub(super) id: ui::Id,
    identity: String,
    lineage: Option<String>,
    session: String,
    generation: u64,
    subscribed: bool,
    seen: Option<Arc<crate::file_client::Update>>,
    pub(super) path: Option<String>,
    directory: Option<String>,
    parent: Option<String>,
    search_root: Option<String>,
    operation: FileOperation,
    pub(super) search: Option<TextField>,
    index_root: Option<String>,
    index_seen: Option<Arc<crate::file_index::Update>>,
    matcher: crate::file_index::Matcher,
    matches: Option<Arc<crate::file_index::Matches>>,
    match_generation: u64,
    retained_row: Option<(String, f32)>,
    highlighter: tau_code_viewer::finder::Finder,
    show_hidden: bool,
    search_from_chat: bool,
    preview: Option<Arc<Document>>,
    preview_target: Option<String>,
    preview_error: Option<String>,
    preview_loading: bool,
    preview_scroll: ScrollState,
    preview_horizontal: ScrollState,
    preview_viewport: Rect,
    preview_paints: HashMap<u64, (Vec<PaintSpan>, Option<PaintHandle>)>,
    previews: std::collections::VecDeque<(String, Arc<Document>)>,
    last_click: Option<(String, Instant)>,
    entries: Vec<FileEntry>,
    next: Option<String>,
    pages: Vec<Option<String>>,
    row: usize,
    pub(super) document: Option<Arc<Document>>,
    pub(super) selection: Option<Selection>,
    /// The exact generated reference and its byte position. Never global replace
    /// user prose or other references when a live file moves selected lines.
    pub(super) reference: Option<(usize, String)>,
    reference_sync: Option<bool>,
    pub(super) drag_anchor: Option<u64>,
    pub(super) scroll: ScrollState,
    horizontal: ScrollState,
    viewport: Rect,
    line_height: f32,
    gutter: f32,
    cursor: usize,
    loading: bool,
    pub(super) error: Option<String>,
    status: String,
    paints: HashMap<u64, (Vec<PaintSpan>, Option<PaintHandle>)>,
}
impl View {
    fn new(model: &Controller, session: String) -> Self {
        let id = ui::Id::new();
        Self {
            id,
            identity: model.identity.clone(),
            lineage: model.account.source_lineage.clone(),
            session,
            generation: 0,
            subscribed: false,
            seen: None,
            path: None,
            directory: None,
            parent: None,
            search_root: None,
            operation: FileOperation::List { after: None },
            search: None,
            index_root: None,
            index_seen: None,
            matcher: crate::file_index::Matcher::new(model.file_wake()),
            matches: None,
            match_generation: 0,
            retained_row: None,
            highlighter: tau_code_viewer::finder::Finder::new(""),
            show_hidden: false,
            search_from_chat: false,
            preview: None,
            preview_target: None,
            preview_error: None,
            preview_loading: false,
            preview_scroll: ScrollState::new(id, false),
            preview_horizontal: ScrollState::new(id, true),
            preview_viewport: Rect::new(0., 0., 0., 0.),
            preview_paints: HashMap::new(),
            previews: Default::default(),
            last_click: None,
            entries: vec![],
            next: None,
            pages: vec![None],
            row: 0,
            document: None,
            selection: None,
            reference: None,
            reference_sync: None,
            drag_anchor: None,
            scroll: ScrollState::new(id, false),
            horizontal: ScrollState::new(id, true),
            viewport: Rect::new(0., 0., 0., 0.),
            line_height: 24.,
            gutter: 52.,
            cursor: 0,
            loading: true,
            error: None,
            status: String::new(),
            paints: HashMap::new(),
        }
    }
    fn len(&self) -> usize {
        if self.search.is_some() { self.matches.as_ref().filter(|m| m.generation == self.match_generation).map_or(0, |m| m.rows.len()) }
        else { self.entries.iter().filter(|e| self.show_hidden || !e.name.starts_with('.')).count() }
    }
    fn entry(&self, row: usize) -> Option<FileEntry> {
        if self.search.is_some() {
            let matches = self.matches.as_ref().filter(|m| m.generation == self.match_generation)?;
            let index = *matches.rows.get(row)?;
            let entry = &matches.index.entries[index];
            Some(FileEntry { path: matches.index.absolute(index), name: entry.path.rsplit('/').next()?.into(), directory: false, symlink: entry.symlink })
        } else { self.entries.iter().filter(|e| self.show_hidden || !e.name.starts_with('.')).nth(row).cloned() }
    }
    fn search_status(&self) -> String {
        let Some(update) = &self.index_seen else { return "Syncing file names…".into(); };
        let Some(index) = &update.index else { return update.error.clone().unwrap_or_else(|| "Syncing file names…".into()); };
        let eligible = if self.show_hidden { index.entries.len() } else { index.visible };
        let matching = self.matches.as_ref().is_none_or(|m| m.generation != self.match_generation);
        let state = if update.error.is_some() { " · cached, sync unavailable" }
            else if update.indexing { " · indexing…" } else if update.limited { " · PARTIAL index; narrow with Here" }
            else if matching { " · matching…" } else { "" };
        format!("{} / {} files{} · .gitignore{}", self.len(), eligible, state, if self.show_hidden { "" } else { " · hidden off" })
    }
    fn remember_preview(&mut self, path: String, doc: Arc<Document>) {
        self.previews.retain(|(p, _)| p != &path);
        self.previews.push_front((path, doc));
        while self.previews.len() > 8 || self.previews.iter().map(|(_, d)| d.text.len()).sum::<usize>() > 8*1024*1024 {
            self.previews.pop_back();
        }
    }
    pub(super) fn sent(&mut self) {
        self.selection = None;
        self.reference = None;
        self.reference_sync = None;
    }
    fn line_at(&self, point: Vec2) -> Option<usize> {
        let doc = self.document.as_ref()?;
        if self.search.is_some() || doc.lines.is_empty() {
            return None;
        }
        Some(((point.y - self.viewport.y + self.scroll.value).max(0.) / self.line_height) as usize)
            .map(|n| n.min(doc.lines.len() - 1))
    }
    fn clear_paint(&mut self, renderer: &mut Renderer) {
        for (_, (_, paint)) in self.paints.drain().chain(self.preview_paints.drain()) {
            if let Some(paint) = paint {
                renderer.text.drop_paint(paint);
            }
        }
    }
}
fn parent(path: &str) -> Option<String> {
    let path = path.trim_end_matches(['/', '\\']);
    let at = path.rfind(['/', '\\'])?;
    Some(if at == 0 { "/".into() } else { path[..at].into() })
}
fn display_path(path: &str) -> String {
    path.chars().map(|c| if c.is_control() { '�' } else { c }).collect()
}
struct Source<'a>(&'a str);
impl ParagraphSource for Source<'_> {
    fn paragraph_text(&self, _: usize, _: ParagraphKey) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed(self.0))
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) enum Choice {
    Files,
    FileClose,
    FileChat,
    FileHidden,
    FileSelect(String),
    FileAccept,
    FileOpen(String, bool),
    FileUp,
    FileFindHere,
    FileFind,
    FileClear,
    FileCopy,
    FilePage(bool),
}
pub(in crate::app) struct CodeBrowser {
    pub view: Option<View>,
    pub controls: Controls<Choice>,
    surface: ui::controls::Control,
    pub pointer: Option<Pointer>,
    pub composer_bottom: Option<f32>,
}
impl CodeBrowser {
    pub fn new() -> Self {
        let id = ui::Id::new();
        Self {
            view: None,
            controls: Controls::new(id),
            surface: ui::controls::Control::new(id, false),
            pointer: None,
            composer_bottom: None,
        }
    }
    fn cancel_pointer(&mut self, cx: &mut Context<'_>) {
        self.pointer = None;
        if cx.ui.capture.is_some_and(|c| {
            c.target.scope == self.controls.id || self.view.as_ref().is_some_and(|v| v.id == c.target.scope)
        }) {
            cx.ui.capture = None;
        }
        if let Some(code) = &mut self.view {
            code.drag_anchor = None;
            code.scroll.stop();
            code.horizontal.stop();
            code.preview_scroll.stop();
            code.preview_horizontal.stop();
        }
    }
    fn activate(&mut self, action: Choice, cx: &mut Context<'_>) {
        let result = self.code_action(action, cx);
        cx.report(result);
    }

    pub(super) fn close_code(&mut self, cx: &mut Context<'_>) {
        if self.view.is_some() {
            self.cancel_pointer(cx);
        }
        if let Some(mut view) = self.view.take() {
            cx.ui.detach(view.id);
            cx.ui.search = None;
            self.controls.begin();
            self.surface.rect = None;
            view.clear_paint(&mut cx.services.renderer);
            let _ = cx.model.view_files(None);
            cx.ui.focus = None;
        }
    }
    fn code_request(&mut self, cx: &mut Context<'_>) {
        let Some(code) = &mut self.view else {
            return;
        };
        let preview = code.search.is_some();
        let previous = if preview { code.preview.clone() } else { code.document.clone() };
        let path = if preview {
            let Some(path) = code.preview_target.clone() else { return; };
            Some(path)
        } else { code.path.clone() };
        let operation = if preview || matches!(code.operation, FileOperation::Open {..}) {
            FileOperation::Open { revision: previous.as_ref().map(|d| d.revision.clone()) }
        } else { code.operation.clone() };
        let request = FileRequest { session_id: code.session.clone(), path, operation };
        code.seen = None;
        if preview { code.preview_loading = true; } else { code.loading = true; }
        match cx.model.view_files_preview(Some(request), previous, preview) {
            Ok(generation) => {
                code.generation = generation;
                code.subscribed = true;
                if preview { code.preview_error = None; } else { code.error = None; }
            }
            Err(error) => {
                if preview { code.preview_error = Some(error.to_string()); code.preview_loading = false; }
                else { code.error = Some(error.to_string()); code.loading = false; }
                code.subscribed = false;
            }
        }
        cx.ui.dirty = true;
    }
    pub(super) fn code_action(&mut self, action: Choice, cx: &mut Context<'_>) -> Result<()> {
        match action {
            Choice::Files => {
                if self.view.is_some() {
                    self.close_code(cx);
                    return Ok(());
                }
                let Some(session) = cx.model.account.selected.clone() else {
                    return Ok(());
                };
                cx.model.save_chat(&session)?;
                self.cancel_pointer(cx);
                cx.ui.focus = None;
                self.view = Some(View::new(cx.model, session));
                self.code_request(cx);
            }
            Choice::FileClose | Choice::FileChat => {
                self.close_code(cx);
            }
            Choice::FileHidden => {
                if let Some(code) = &mut self.view {
                    code.show_hidden = !code.show_hidden;
                    code.scroll.value = 0.; code.row = 0;
                }
                self.code_query(cx);
            }
            Choice::FileSelect(path) => {
                let Some(code) = &mut self.view else { return Ok(()); };
                let double = code.last_click.as_ref().is_some_and(|(p, at)| p == &path && at.elapsed().as_millis() < 350);
                code.last_click = Some((path.clone(), Instant::now()));
                if let Some(matches) = &code.matches && matches.generation == code.match_generation
                    && let Some(row) = matches.rows.iter().position(|&i| matches.index.absolute(i) == path) {
                    code.row = row;
                }
                if double { self.code_action(Choice::FileOpen(path, false), cx)?; }
                else { self.select_preview(cx); }
            }
            Choice::FileAccept => {
                if let Some(entry) = self.view.as_ref().and_then(|c| c.entry(c.row)) {
                    self.code_action(Choice::FileOpen(entry.path, entry.directory), cx)?;
                }
            }
            Choice::FileOpen(path, directory) => {
                self.cancel_pointer(cx);
                let Some(code) = &mut self.view else {
                    return Ok(());
                };
                code.clear_paint(&mut cx.services.renderer);
                cx.ui.detach(code.id);
                cx.ui.search = None;
                let cached = code.previews.iter().find(|(p, _)| p == &path).map(|(_, doc)| doc.clone());
                code.path = Some(path);
                code.operation = if directory {
                    FileOperation::List { after: None }
                } else {
                    FileOperation::Open { revision: None }
                };
                code.search = None;
                code.match_generation = code.matcher.cancel();
                code.document = if directory { None } else { cached };
                code.preview = None;
                code.preview_target = None;
                code.selection = None;
                code.reference = None;
                code.reference_sync = None;
                code.drag_anchor = None;
                code.entries.clear();
                code.pages = vec![None];
                code.next = None;
                code.scroll.value = 0.;
                code.horizontal.value = 0.;
                code.row = 0;
                code.cursor = 0;
                cx.ui.focus = None;
                self.code_request(cx);
            }
            Choice::FileUp => {
                if let Some(path) = self.view.as_ref().and_then(|c| {
                    if c.document.is_some() { c.path.as_deref().and_then(parent) } else { c.parent.clone() }
                }) {
                    self.code_action(Choice::FileOpen(path, true), cx)?;
                }
            }
            Choice::FileFindHere => {
                if let Some(code) = &mut self.view && code.search.is_some() {
                    code.index_root = code.directory.clone();
                    code.path = code.index_root.clone();
                    code.index_seen = None;
                    code.matches = None;
                    code.match_generation = code.matcher.cancel();
                    code.retained_row = None;
                    code.row = 0; code.scroll.value = 0.;
                    self.cancel_preview(cx);
                }
            }
            Choice::FileFind => {
                self.cancel_pointer(cx);
                let from_chat = self.view.is_none();
                if from_chat { self.code_action(Choice::Files, cx)?; }
                let Some(code) = &mut self.view else { return Ok(()); };
                if code.search.is_some() { code.search_from_chat = false; self.code_back(cx); return Ok(()); }
                code.search_from_chat = from_chat;
                code.clear_paint(&mut cx.services.renderer);
                code.search = Some(TextField::new(code.id, "", Editor::line(String::new())));
                let field = code.search.as_mut().unwrap();
                field.size = 16.;
                field.placeholder = "Find files…".into();
                code.path = code.search_root.clone();
                if code.index_root.take().is_some() { code.index_seen = None; code.matches = None; }
                code.loading = false; code.error = None;
                code.row = 0; code.scroll.value = 0.;
                cx.ui.focus = Some(field.control.target);
                cx.ui.search = cx.ui.focus;
                self.cancel_preview(cx);
                self.code_query(cx);
                if cx.ui.mobile && let Some(field) = self.view.as_ref().and_then(|c| c.search.as_ref()) {
                    cx.focus_native(field.control.target, field.editor.native_id());
                }
            }
            Choice::FileClear => {
                if let Some(code) = &mut self.view {
                    code.selection = None;
                    code.reference = None;
                    code.reference_sync = None;
                    code.drag_anchor = None;
                }
                cx.ui.focus = None;
            }
            Choice::FileCopy => {
                if let Some(code) = &self.view
                    && let (Some(doc), Some(selection)) = (&code.document, &code.selection)
                    && let Some(text) = doc.selected_text(selection)
                {
                    cx.services.platform.push(PlatformAction::Copy(text));
                }
            }
            Choice::FilePage(next) => {
                let Some(code) = &mut self.view else {
                    return Ok(());
                };
                if next {
                    if let Some(next) = code.next.clone() {
                        code.pages.push(Some(next));
                    } else {
                        return Ok(());
                    }
                } else if code.pages.len() > 1 {
                    code.pages.pop();
                } else {
                    return Ok(());
                }
                code.operation = FileOperation::List { after: code.pages.last().cloned().flatten() };
                code.scroll.value = 0.;
                code.row = 0;
                code.entries.clear();
                self.code_request(cx);
            }
        }
        cx.ui.dirty = true;
        Ok(())
    }
    pub(super) fn code_back(&mut self, cx: &mut Context<'_>) {
        self.cancel_pointer(cx);
        if self.view.as_ref().is_some_and(|c| c.search.is_some() && c.search_from_chat) {
            self.close_code(cx); return;
        }
        if let Some(code) = &mut self.view
            && code.search.take().is_some()
        {
            cx.ui.detach(code.id);
            cx.ui.search = None;
            code.preview = None; code.preview_target = None;
            code.match_generation = code.matcher.cancel();
            code.clear_paint(&mut cx.services.renderer);
            code.path = code.document.as_ref().map(|d| d.path.clone()).or_else(|| code.directory.clone());
            code.operation = if code.document.is_some() {
                FileOperation::Open { revision: None }
            } else {
                FileOperation::List { after: None }
            };
            code.entries.clear();
            code.pages = vec![None];
            code.next = None;
            code.scroll.value = 0.;
            code.row = 0;
            cx.ui.focus = None;
            self.code_request(cx);
        } else if self.view.as_ref().is_some_and(|c| c.document.is_some()) {
            let result = self.code_action(Choice::FileUp, cx);
            cx.report(result);
        } else {
            self.close_code(cx);
        }
    }
    fn cancel_preview(&mut self, cx: &mut Context<'_>) {
        if let Some(code) = &mut self.view {
            if code.subscribed { let _ = cx.model.view_files(None); }
            code.subscribed = false;
            code.preview = None; code.preview_target = None; code.preview_error = None; code.preview_loading = false;
        }
    }
    fn select_preview(&mut self, cx: &mut Context<'_>) {
        let Some(code) = &mut self.view else { return; };
        if code.search.is_none() { return; }
        let path = code.entry(code.row).map(|e| e.path);
        if code.preview_target == path { return; }
        code.clear_paint(&mut cx.services.renderer);
        code.preview_scroll.value = 0.; code.preview_horizontal.value = 0.;
        if let Some(path) = path {
            code.preview = code.previews.iter().find(|(p, _)| p == &path).map(|(_, d)| d.clone());
            code.preview_target = Some(path);
            if cx.ui.window_focused && cx.ui.visible && !cx.ui.covered { self.code_request(cx); }
        } else { self.cancel_preview(cx); }
        cx.ui.dirty = true;
    }
    pub(super) fn code_query(&mut self, cx: &mut Context<'_>) {
        let Some(code) = &mut self.view else { return; };
        let Some(search) = &code.search else { return; };
        let query: String = search.editor.value.chars().take(256).collect();
        code.highlighter.query(&query);
        let index = code.index_seen.as_ref().and_then(|u| u.index.clone());
        code.match_generation = if let Some(index) = index { code.matcher.query(index, query, code.show_hidden) }
            else { code.matcher.cancel() };
        code.scroll.value = 0.; code.row = 0; code.retained_row = None;
        self.cancel_preview(cx);
        cx.ui.dirty = true;
    }
    /// Persist once per completed gesture/live revision, not on every pointer
    /// motion. Keep a user's draft and only replace our own still-intact marker.
    pub(super) fn code_reference(&mut self, remove: bool, cx: &mut Context<'_>) {
        let Some(code) = &mut self.view else {
            return;
        };
        if cx.model.account.selected.as_ref() != Some(&code.session) || cx.model.identity != code.identity {
            return;
        }
        if !remove && code.error.is_some() {
            return;
        }
        let next = if remove {
            None
        } else {
            code.document.as_ref().zip(code.selection.as_ref()).and_then(|(d, s)| s.reference(d)).map(|r| {
                let fence = "`".repeat(r.split(|c| c != '`').map(str::len).max().unwrap_or(0) + 1);
                format!("{fence}{r}{fence}\n")
            })
        };
        let Some(chat) = cx.model.chats.get(&code.session) else {
            return;
        };
        let mut draft = chat.local.draft.clone();
        let old = code.reference.take();
        if let Some((start, old)) = old {
            let start = if draft.get(start..start + old.len()) == Some(old.as_str()) {
                Some(start)
            } else {
                let mut matches = draft.match_indices(&old);
                let first = matches.next().map(|(at, _)| at);
                if matches.next().is_none() { first } else { None }
            };
            if let Some(start) = start {
                let replacement = next.as_deref().unwrap_or("");
                draft.replace_range(start..start + old.len(), replacement);
                code.reference = next.map(|r| (start, r));
            }
            // A marker the user edited/removed is no longer ours to rewrite.
        } else if let Some(reference) = next {
            if !draft.is_empty() && !draft.ends_with("\n\n") {
                draft.push_str(if draft.ends_with('\n') { "\n" } else { "\n\n" });
            }
            let start = draft.len();
            draft.push_str(&reference);
            code.reference = Some((start, reference));
        }
        // Rebinding the inline editor fences all native snapshots of older markers.
        if draft != cx.model.chats[&code.session].local.draft {
            match cx.model.draft(draft.clone()) {
                Ok(()) => {}
                Err(error) => cx.model.report_error(error),
            }
        }
    }
    pub(super) fn code_tick(&mut self, dt: f32, cx: &mut Context<'_>) {
        let active = cx.ui.window_focused && cx.ui.visible && !cx.ui.covered;
        let plan = if active {
            cx.model.account.selected.clone().map(|session| (session, self.view.as_ref().and_then(|c| c.index_root.clone())))
        } else { None };
        if let Err(error) = cx.model.sync_file_index(plan) { cx.report(Err(error)); }
        let Some(code) = &self.view else {
            return;
        };
        if code.identity != cx.model.identity
            || code.session != cx.model.account.selected.as_deref().unwrap_or("")
            || code.lineage != cx.model.account.source_lineage
        {
            self.close_code(cx);
            cx.ui.dirty = true;
            return;
        }
        let active = cx.ui.window_focused && cx.ui.visible && !cx.ui.covered;
        if !active && code.subscribed {
            let _ = cx.model.view_files(None);
            self.view.as_mut().unwrap().subscribed = false;
        } else if active
            && (!code.subscribed || code.generation != cx.model.viewer_generation())
            && cx.model.epoch.is_some()
            && (code.search.is_none() || code.preview_target.is_some())
        {
            self.code_request(cx);
        }
        if let Some(update) = cx.model.file_index.clone()
            && self.view.as_ref().is_some_and(|c| update.session == c.session
                && update.lineage == c.lineage.as_deref().unwrap_or("")
                && c.index_seen.as_ref().is_none_or(|old| !Arc::ptr_eq(old, &update))) {
            let code = self.view.as_mut().unwrap();
            let changed = code.index_seen.as_ref().and_then(|u| u.index.as_ref()).zip(update.index.as_ref())
                .is_none_or(|(a, b)| !Arc::ptr_eq(a, b));
            if let Some(index) = &update.index {
                if code.search_root.is_none() && code.index_root.is_none() { code.search_root = Some(index.root.clone()); }
                if code.directory.is_none() && code.document.is_none() { code.directory = Some(index.root.clone()); }
                if code.search.is_some() { code.path = Some(index.root.clone()); }
            }
            code.index_seen = Some(update);
            if changed && code.search.is_some() {
                let selected = code.entry(code.row).map(|e| (e.path, code.scroll.value));
                self.code_query(cx);
                self.view.as_mut().unwrap().retained_row = selected;
            }
            cx.ui.dirty = true;
        }
        if let Some(code) = &mut self.view
            && let Some(matches) = code.matcher.take()
            && code.search.is_some() && matches.generation == code.match_generation {
            if let Some((path, scroll)) = code.retained_row.take()
                && let Some(row) = matches.rows.iter().position(|&i| matches.index.absolute(i) == path) {
                code.row = row; code.scroll.value = scroll;
            }
            code.matches = Some(matches);
            code.row = code.row.min(code.len().saturating_sub(1));
            self.select_preview(cx);
            cx.ui.dirty = true;
        }
        let update = cx.model.file_update.clone();
        if let Some(update) = update
            && self.view.as_ref().is_some_and(|c| {
                update.generation == c.generation
                    && update.session == c.session
                    && update.lineage == c.lineage.as_deref().unwrap_or("")
                    && c.seen.as_ref().is_none_or(|old| !Arc::ptr_eq(old, &update))
            })
        {
            let code = self.view.as_mut().unwrap();
            code.seen = Some(update.clone());
            code.loading = false;
            let previewing = code.search.is_some();
            if previewing {
                code.preview_loading = false;
                match &update.response {
                    Err(error) => code.preview_error = Some(error.clone()),
                    Ok(FileReply::Text {..} | FileReply::Unchanged {..}) => {
                        code.preview_error = None;
                        if let Some(doc) = &update.document {
                            if code.preview.as_ref().is_some_and(|old| old.namespace != doc.namespace) { code.clear_paint(&mut cx.services.renderer); }
                            code.preview = Some(doc.clone());
                            if let Some(path) = code.preview_target.clone() { code.remember_preview(path, doc.clone()); }
                        }
                    }
                    _ => {}
                }
            }
            let mut invalidated = false;
            let mut moved = false;
            if !previewing { match &update.response {
                Err(error) => {
                    code.error = Some(error.clone());
                    code.status = "Preview unavailable · retrying".into();
                    code.drag_anchor = None;
                }
                Ok(reply) => {
                    code.error = None;
                    match reply {
                        FileReply::Directory { path, parent, entries, next } => {
                            if code.search_root.is_none() {
                                code.search_root = Some(path.clone());
                            }
                            code.path = Some(path.clone());
                            code.directory = Some(path.clone());
                            code.parent = parent.clone();
                            code.entries = entries.clone();
                            code.next = next.clone();
                            code.status = "Read only · ignored files remain browsable".into();
                        }
                        FileReply::Index { .. } => {},
                        FileReply::Text { path, .. } | FileReply::Unchanged { path, .. } => {
                            if let Some(doc) = &update.document {
                                let cursor =
                                    code.document.as_ref().and_then(|d| d.lines.get(code.cursor)).map(|l| l.id);
                                if let Some(cursor) = cursor
                                    && let Some(line) = doc.position(cursor)
                                {
                                    code.cursor = line;
                                }
                                let top = code
                                    .document
                                    .as_ref()
                                    .and_then(|d| d.lines.get((code.scroll.value / code.line_height) as usize))
                                    .map(|l| l.id);
                                if let Some(top) = top
                                    && let Some(line) = doc.position(top)
                                {
                                    code.scroll.value =
                                        line as f32 * code.line_height + code.scroll.value % code.line_height;
                                }
                                if let Some(selection) = &code.selection {
                                    if selection.range(doc).is_none() {
                                        invalidated = true;
                                        code.selection = None;
                                        code.drag_anchor = None;
                                    } else {
                                        moved = true;
                                    }
                                }
                                if code.document.as_ref().is_some_and(|old| old.namespace != doc.namespace) {
                                    code.clear_paint(&mut cx.services.renderer);
                                }
                                code.document = Some(doc.clone());
                                code.cursor = code.cursor.min(doc.lines.len() - 1);
                            }
                            code.path = Some(path.clone());
                            code.directory = parent(path);
                            code.status = if invalidated {
                                "Selected text changed · reselect to comment (draft kept)"
                            } else {
                                "Read only · live · select line numbers to comment"
                            }
                            .into();
                        }
                    }
                }
            }
            }
            code.row = code.row.min(code.len().saturating_sub(1));
            if invalidated {
                self.view.as_mut().unwrap().reference_sync = Some(true);
            } else if moved && self.view.as_ref().is_some_and(|c| c.reference.is_some()) {
                self.view.as_mut().unwrap().reference_sync = Some(false);
            }
            cx.ui.dirty = true;
        }
        if !cx.ui.composing
            && let Some(remove) = self.view.as_mut().and_then(|c| c.reference_sync.take())
        {
            self.code_reference(remove, cx);
            cx.ui.dirty = true;
        }
        // Hold in the code body promotes scrolling to line-range selection. A
        // gutter press selects immediately, and dragging back shrinks the range.
        if active
            && let Some(p) = &self.pointer
            && p.touch
            && !p.dragged
            && p.started.elapsed().as_millis() >= 450
            && self.view.as_ref().is_some_and(|c| {
                c.error.is_none()
                    && c.search.is_none()
                    && c.document.is_some()
                    && contains(c.viewport, p.start)
                    && c.drag_anchor.is_none()
            })
        {
            let point = p.start;
            self.code_begin(point, cx);
            cx.services.platform.push(PlatformAction::Haptic);
        }
        if let Some(code) = &mut self.view
            && code.drag_anchor.is_some()
            && let Some(p) = &self.pointer
        {
            let margin = 16. * cx.ui.scale;
            let delta = if p.last.y < code.viewport.y + margin {
                p.last.y - code.viewport.y - margin
            } else {
                (p.last.y - code.viewport.y - code.viewport.height + margin).max(0.)
            };
            let next = (code.scroll.value + delta.clamp(-90. * cx.ui.scale, 90. * cx.ui.scale) * 12. * dt.min(0.05))
                .clamp(0., code.scroll.max);
            if next != code.scroll.value {
                code.scroll.value = next;
                let point = p.last;
                self.code_extend(point, cx);
                cx.ui.dirty = true;
            }
        }
    }
    fn code_begin(&mut self, point: Vec2, cx: &mut Context<'_>) {
        let Some(code) = &mut self.view else {
            return;
        };
        let Some(line) = code.line_at(point) else {
            return;
        };
        let doc = code.document.as_ref().unwrap();
        code.cursor = line;
        code.selection = Some(Selection::new(doc, line, line));
        code.drag_anchor = Some(doc.lines[line].id);
        code.scroll.stop();
        code.horizontal.stop();
        if let Some(c) = &mut cx.ui.capture {
            c.target = self.surface.target;
            c.claimed = true;
        }
        cx.ui.focus = None;
        cx.ui.dirty = true;
    }
    fn code_extend(&mut self, point: Vec2, cx: &mut Context<'_>) {
        let Some(code) = &mut self.view else {
            return;
        };
        let Some(end) = code.line_at(point) else {
            return;
        };
        let doc = code.document.as_ref().unwrap();
        let Some(anchor) = code.drag_anchor.and_then(|id| doc.position(id)) else {
            return;
        };
        code.selection = Some(Selection::new(doc, anchor, end));
        code.cursor = end;
        cx.ui.dirty = true;
    }
    pub(super) fn code_press(&mut self, id: u64, point: Vec2, touch: bool, cx: &mut Context<'_>) -> bool {
        if cx.ui.covered {
            return false;
        }
        let Some(code) = &self.view else {
            return false;
        };
        if code.error.is_some()
            || code.document.is_none()
            || code.search.is_some()
            || !contains(code.viewport, point)
            || touch && point.x > code.viewport.x + code.gutter
        {
            return false;
        }
        self.pointer = Some(Pointer {
            id,
            start: point,
            last: point,
            at: Instant::now(),
            started: Instant::now(),
            dragged: false,
            touch,
        });
        cx.ui.capture = Some(ui::Capture {
            target: self.surface.target,
            pointer: id,
            start: point,
            point,
            touch,
            dragged: false,
            claimed: true,
            started: Instant::now(),
        });
        self.code_begin(point, cx);
        true
    }
    pub(super) fn code_motion(&mut self, id: u64, point: Vec2, cx: &mut Context<'_>) -> bool {
        if cx.ui.covered {
            return false;
        }
        let (Some(code), Some(p)) = (&mut self.view, &mut self.pointer) else {
            return false;
        };
        if p.id != id || !contains(code.viewport, p.start) {
            return false;
        }
        let selecting = code.drag_anchor.is_some();
        p.dragged |= (point.x - p.start.x).abs() + (point.y - p.start.y).abs() > 7. * cx.ui.scale;

        p.last = point;
        p.at = Instant::now();
        if selecting {
            self.code_extend(point, cx);
        }
        cx.ui.dirty = true;
        true
    }
    pub(super) fn code_release(&mut self, id: u64, point: Vec2, cx: &mut Context<'_>) -> bool {
        if self.view.as_ref().is_none_or(|c| c.drag_anchor.is_none())
            || self.pointer.as_ref().is_none_or(|p| p.id != id)
        {
            return false;
        }
        self.code_extend(point, cx);
        self.view.as_mut().unwrap().drag_anchor = None;
        self.pointer = None;
        cx.ui.capture = None;
        self.code_reference(false, cx);
        if !cx.ui.mobile {
            cx.ui.focus = cx.ui.composer;
        }
        cx.ui.dirty = true;
        true
    }
    pub(super) fn code_wheel(&mut self, amount: f32, horizontal: bool, point: Vec2, cx: &mut Context<'_>) -> bool {
        let Some(code) = &mut self.view else {
            return false;
        };
        if code.search.is_some() && contains(code.preview_viewport, point) {
            let scroll = if horizontal { &mut code.preview_horizontal } else { &mut code.preview_scroll };
            scroll.value = (scroll.value + amount).clamp(0., scroll.max);
            cx.ui.dirty = true; return true;
        }
        if !contains(code.viewport, point) {
            return false;
        }
        if horizontal {
            code.horizontal.value = (code.horizontal.value + amount).clamp(0., code.horizontal.max);
        } else {
            code.scroll.value = (code.scroll.value + amount).clamp(0., code.scroll.max);
        }
        cx.ui.dirty = true;
        true
    }
    pub(super) fn code_key(&mut self, key: &str, ctrl: bool, shift: bool, cx: &mut Context<'_>) -> bool {
        if ctrl && matches!(key, "Space" | " ") {
            self.activate(Choice::FileFind, cx);
            return true;
        }
        let Some(code) = &mut self.view else {
            return false;
        };
        if key == "Escape" {
            if code.search.is_some() {
                self.code_back(cx);
            } else if code.selection.is_some() {
                self.activate(Choice::FileClear, cx);
            } else {
                self.code_back(cx);
            }
            cx.ui.dirty = true;
            return true;
        }
        if cx.ui.composer.is_some() && cx.ui.focus == cx.ui.composer {
            return false;
        }
        if ctrl && key.eq_ignore_ascii_case("c") && code.search.is_none() {
            self.activate(Choice::FileCopy, cx);
            return true;
        }
        if code.document.is_none() || code.search.is_some() {
            let key = if code.search.is_some() {
                if ctrl && matches!(key, "n" | "N" | "j" | "J") || key == "Tab" && !shift { "ArrowDown" }
                else if ctrl && matches!(key, "p" | "P" | "k" | "K") || key == "Tab" && shift { "ArrowUp" }
                else { key }
            } else { key };
            let row_height = if code.search.is_some() { 36. } else { 44. } * cx.ui.scale;
            let page = (code.viewport.height / row_height).floor().max(1.) as usize;
            match key {
                "ArrowUp" | "k" if code.search.is_none() || key == "ArrowUp" => code.row = code.row.saturating_sub(1),
                "ArrowDown" | "j" if code.search.is_none() || key == "ArrowDown" => {
                    code.row = (code.row + 1).min(code.len().saturating_sub(1))
                }
                "PageUp" => code.row = code.row.saturating_sub(page),
                "PageDown" => code.row = (code.row + page).min(code.len().saturating_sub(1)),
                "Home" if code.search.is_none() || ctrl => code.row = 0,
                "End" if code.search.is_none() || ctrl => code.row = code.len().saturating_sub(1),
                "Enter" | "l" if code.search.is_none() || key == "Enter" => {
                    if let Some(entry) = code.entry(code.row) {
                        let action = Choice::FileOpen(entry.path.clone(), entry.directory);
                        self.activate(action, cx);
                    }
                    return true;
                }
                "Backspace" | "h" if code.search.is_none() => {
                    self.activate(Choice::FileUp, cx);
                    return true;
                }
                _ => return false,
            }
            let top = code.row as f32 * row_height;
            if top < code.scroll.value {
                code.scroll.value = top;
            } else if top + row_height > code.scroll.value + code.viewport.height {
                code.scroll.value = (top + row_height - code.viewport.height).min(code.scroll.max);
            }
            self.select_preview(cx);
        } else {
            if code.error.is_some() && (shift || key == "v") {
                return true;
            }
            let doc = code.document.as_ref().unwrap();
            let old = code.cursor;
            let page = (code.viewport.height / code.line_height).floor().max(1.) as usize;
            match key {
                "ArrowUp" | "k" => code.cursor = old.saturating_sub(1),
                "ArrowDown" | "j" => code.cursor = (old + 1).min(doc.lines.len() - 1),
                "PageUp" => code.cursor = old.saturating_sub(page),
                "PageDown" => code.cursor = (old + page).min(doc.lines.len() - 1),
                "Home" => code.cursor = 0,
                "End" => code.cursor = doc.lines.len() - 1,
                "ArrowLeft" => {
                    code.horizontal.value = (code.horizontal.value - 48. * cx.ui.scale).max(0.);
                    cx.ui.dirty = true;
                    return true;
                }
                "ArrowRight" => {
                    code.horizontal.value = (code.horizontal.value + 48. * cx.ui.scale).min(code.horizontal.max);
                    cx.ui.dirty = true;
                    return true;
                }
                "Backspace" | "h" => {
                    self.activate(Choice::FileUp, cx);
                    return true;
                }
                "v" => {
                    code.selection = Some(Selection::new(doc, old, old));
                    self.code_reference(false, cx);
                    cx.ui.dirty = true;
                    return true;
                }
                _ => return false,
            }
            if shift {
                let anchor = code.selection.as_ref().and_then(|s| doc.position(s.anchor)).unwrap_or(old);
                code.selection = Some(Selection::new(doc, anchor, code.cursor));
            }
            let top = code.cursor as f32 * code.line_height;
            if top < code.scroll.value {
                code.scroll.value = top;
            } else if top + code.line_height > code.scroll.value + code.viewport.height {
                code.scroll.value = (top + code.line_height - code.viewport.height).min(code.scroll.max);
            }
            if shift {
                self.code_reference(false, cx);
            }
        }
        cx.ui.dirty = true;
        true
    }
}

impl Widget for CodeBrowser {
    fn handle_event(&mut self, event: &InputEvent<'_>, cx: &mut Context<'_>) -> bool {
        if matches!(event, InputEvent::Cancel) {
            self.cancel_pointer(cx);
            return false;
        }
        if let InputEvent::Text(text) = event
            && cx.ui.focus.is_none()
            && self.code_key(text, false, false, cx)
        {
            return true;
        }
        if let InputEvent::Key { key, ctrl, shift } = *event
            && !cx.ui.composing
            && self.code_key(key, ctrl, shift, cx)
        {
            return true;
        }
        let Some(code) = &self.view else {
            return false;
        };
        if code.identity != cx.model.identity
            || code.lineage != cx.model.account.source_lineage
            || Some(&code.session) != cx.model.account.selected.as_ref()
        {
            self.close_code(cx);
            return false;
        }
        if matches!(event, InputEvent::Back) {
            self.code_back(cx);
            return true;
        }
        let Some(code) = &mut self.view else {
            return false;
        };
        if let Some(field) = &mut code.search {
            let old = field.editor.value.clone();
            let handled = field.handle_event(event, cx);
            let changed = old != field.editor.value;
            if changed {
                self.code_query(cx);
            }
            if handled {
                return true;
            }
        }
        if self.view.as_mut().is_some_and(|v| v.search.is_some() && v.preview_scroll.bar_event(event, cx)) { return true; }
        if self.view.as_mut().is_some_and(|v| v.scroll.bar_event(event, cx)) {
            return true;
        }
        let (child, choice) = self.controls.event(event, cx);
        if let Some(choice) = choice {
            self.activate(choice, cx);
            return true;
        }
        match *event {
            InputEvent::Down { pointer, point, touch } => {
                if !child && self.code_press(pointer, point, touch, cx) {
                    return true;
                }
                if self.view.as_ref().is_some_and(|v| contains(v.viewport, point)) {
                    self.pointer = Some(Pointer {
                        id: pointer,
                        start: point,
                        last: point,
                        at: Instant::now(),
                        started: Instant::now(),
                        dragged: false,
                        touch,
                    });
                }
            }
            InputEvent::Move { pointer, point } => {
                if self.view.as_ref().is_some_and(|v| v.drag_anchor.is_some()) && self.code_motion(pointer, point, cx) {
                    return true;
                }
                if let Some(p) = &mut self.pointer
                    && p.id == pointer
                {
                    p.dragged |= (point.x - p.start.x).abs() + (point.y - p.start.y).abs() > 7. * cx.ui.scale;
                    p.last = point;
                }
            }
            InputEvent::Up { pointer, point } => {
                if self.code_release(pointer, point, cx) {
                    return true;
                }
                if self.pointer.as_ref().is_some_and(|p| p.id == pointer) {
                    self.pointer = None;
                }
            }
            InputEvent::Wheel { amount, horizontal, point } => {
                return self.code_wheel(amount, horizontal, point, cx);
            }
            _ => {}
        }
        let Some(code) = &mut self.view else {
            return child;
        };
        if code.search.is_some() {
            let preview_handled = code.preview_scroll.event(event, child, cx);
            if preview_handled { return true; }
        }
        let horizontal = match event {
            InputEvent::Move { point, .. } => {
                self.pointer.as_ref().is_some_and(|p| (point.x - p.start.x).abs() > 1.5 * (point.y - p.start.y).abs())
            }
            _ => false,
        };
        if horizontal {
            let h = code.horizontal.event(event, child, cx);
            code.scroll.event(event, h, cx)
        } else {
            let v = code.scroll.event(event, child, cx);
            if !matches!(event, InputEvent::Move { .. }) { code.horizontal.event(event, v, cx) } else { v }
        }
    }
    fn visit_perframe(&mut self, frame: &mut Frame<'_>, cx: &mut Context<'_>) {
        if self.view.is_none() {
            return;
        }

        self.controls.begin();
        let b = frame.bounds;
        let layer = &mut *frame.layer;
        let s = cx.ui.scale;
        let mut code = self.view.take().unwrap();
        let comments = code.search.is_none()
            && code.document.is_some()
            && (code.selection.is_some() || cx.ui.composer.is_some() && cx.ui.focus == cx.ui.composer);
        let bottom = self.composer_bottom.unwrap_or(b.y + b.height);
        layer.rect(Rect::new(b.x, b.y, b.width, 124. * s), color(0x0e141b));
        self.controls.button(
            cx,
            layer,
            Rect::new(b.x + 8. * s, b.y + 6. * s, 64. * s, 40. * s),
            "‹ Chat",
            Choice::FileChat,
            ButtonStyle::Tonal,
            frame.clip,
        );
        if code.search.is_some() {
            self.controls.button(
                cx,
                layer,
                Rect::new(b.x + 76. * s, b.y + 6. * s, 44. * s, 40. * s),
                "Here",
                Choice::FileFindHere,
                ButtonStyle::Tonal,
                frame.clip,
            );
        } else if code.parent.is_some() || code.document.is_some() {
            self.controls.button(
                cx,
                layer,
                Rect::new(b.x + 76. * s, b.y + 6. * s, 44. * s, 40. * s),
                "Up",
                Choice::FileUp,
                ButtonStyle::Tonal,
                frame.clip,
            );
        }
        cx.services.renderer.label(
            layer,
            if code.search.is_some() { "Find files" } else { "Files" },
            Rect::new(b.x + 130. * s, b.y + 15. * s, (b.width - 240. * s).max(1.), 24. * s),
            16. * s,
            color(0xe5eaf0),
            true,
        );
        self.controls.button(
            cx,
            layer,
            Rect::new(b.x + b.width - 104. * s, b.y + 6. * s, 60. * s, 40. * s),
            if code.search.is_some() { "Browse" } else { "Find" },
            Choice::FileFind,
            if code.search.is_some() { ButtonStyle::Primary } else { ButtonStyle::Tonal },
            frame.clip,
        );
        self.controls.icon(
            cx,
            layer,
            Rect::new(b.x + b.width - 44. * s, b.y + 6. * s, 40. * s, 40. * s),
            Icon::Close,
            18.,
            Choice::FileClose,
            ButtonStyle::Quiet,
            true,
            frame.clip,
        );
        cx.services.renderer.label(
            layer,
            &display_path(code.path.as_deref().unwrap_or("Chat working directory")),
            Rect::new(b.x + 14. * s, b.y + 54. * s, b.width - 28. * s, 28. * s),
            13. * s,
            color(0xb7c2ce),
            false,
        );
        let top = if let Some(search) = &mut code.search {
            let rect = Rect::new(b.x + 12. * s, b.y + 88. * s, b.width - 24. * s, 48. * s);
            search.visit_perframe(&mut Frame { layer, bounds: rect, clip: frame.clip }, cx);
            cx.services.renderer.label(
                layer,
                &code.search_status(),
                Rect::new(b.x + 14. * s, b.y + 139. * s, b.width - 28. * s, 18. * s),
                11. * s,
                color(0x82909f),
                false,
            );
            self.controls.button(
                cx,
                layer,
                Rect::new(b.x + 12. * s, b.y + 162. * s, 128. * s, 32. * s),
                if code.show_hidden { "✓ Show hidden" } else { "Show hidden" },
                Choice::FileHidden,
                if code.show_hidden { ButtonStyle::Primary } else { ButtonStyle::Tonal },
                frame.clip,
            );
            cx.services.renderer.label(layer, if cx.ui.mobile { "Tap to preview · Open" } else { "↑↓ / Ctrl-N/P · Enter open · Esc back" },
                Rect::new(b.x + 150.*s, b.y + 170.*s, (b.width - 164.*s).max(1.), 20.*s), 11.*s, color(0x82909f), false);
            b.y + 200. * s
        } else {
            let status = code.error.as_deref().unwrap_or(if code.loading { "Loading…" } else { &code.status });
            let label = code
                .error
                .is_none()
                .then(|| {
                    code.selection
                        .as_ref()
                        .zip(code.document.as_ref())
                        .and_then(|(selection, doc)| selection.range(doc))
                        .map(|r| format!("Lines {}–{}", r.start + 1, r.end))
                })
                .flatten();
            cx.services.renderer.label(
                layer,
                label.as_deref().unwrap_or(status),
                Rect::new(
                    b.x + 14. * s,
                    b.y + 94. * s,
                    (b.width - if comments || code.document.is_none() { 158. * s } else { 28. * s }).max(1.),
                    24. * s,
                ),
                12. * s,
                color(if code.error.is_some() { 0xf2a6a6 } else { 0x82909f }),
                false,
            );
            if comments {
                self.controls.button(
                    cx,
                    layer,
                    Rect::new(b.x + b.width - 140. * s, b.y + 84. * s, 62. * s, 40. * s),
                    "Copy",
                    Choice::FileCopy,
                    ButtonStyle::Tonal,
                    frame.clip,
                );
                self.controls.button(
                    cx,
                    layer,
                    Rect::new(b.x + b.width - 74. * s, b.y + 84. * s, 62. * s, 40. * s),
                    "Clear",
                    Choice::FileClear,
                    ButtonStyle::Tonal,
                    frame.clip,
                );
            }
            if code.document.is_none() {
                self.controls.button(
                    cx,
                    layer,
                    Rect::new(b.x + b.width - 140. * s, b.y + 84. * s, 128. * s, 32. * s),
                    if code.show_hidden { "✓ Show hidden" } else { "Show hidden" },
                    Choice::FileHidden,
                    if code.show_hidden { ButtonStyle::Primary } else { ButtonStyle::Tonal },
                    frame.clip,
                );
            }
            b.y + 124. * s
        };
        let paging = code.search.is_none() && code.document.is_none() && (code.next.is_some() || code.pages.len() > 1);
        let available = (bottom - top - if paging { 48. * s } else { 0. }).max(1.);
        let row_height = if code.search.is_some() { 36.*s } else { 44.*s };
        let list_height = if code.search.is_some() { (available * 0.45).min(8.*row_height).max(1.) } else { available };
        code.viewport = Rect::new(b.x, top, b.width, list_height);
        let viewport = code.viewport;
        layer.rect(viewport, color(0x0b1118));
        layer.rect(Rect::new(b.x, top - s, b.width, s), color(0x2a3541));
        if let Some(doc) = &code.document
            && code.search.is_none()
        {
            let selection = code.selection.as_ref().and_then(|s| s.range(doc));
            (code.line_height, code.gutter) = paint::document(doc, selection, &mut code.paints, &mut code.scroll, &mut code.horizontal, viewport, layer, cx);
        } else {
            code.scroll.max = (code.len() as f32 * row_height - viewport.height).max(0.);
            code.scroll.value = code.scroll.value.clamp(0., code.scroll.max);
            let first = (code.scroll.value / row_height) as usize;
            let end = (first + (viewport.height / row_height).ceil() as usize + 1).min(code.len());
            for i in first..end {
                let Some(entry) = code.entry(i) else { continue; };
                let y = viewport.y + i as f32 * row_height - code.scroll.value;
                let rect = Rect::new(viewport.x + 8. * s, y, viewport.width - 20. * s, row_height);
                let hit = crate::render::intersect(rect, viewport);
                if i == code.row {
                    layer.clipped_rect(rect, color(0x172330), viewport);
                }
                let name = if code.search.is_some() {
                    entry
                        .path
                        .strip_prefix(code.path.as_deref().unwrap_or(""))
                        .unwrap_or(&entry.path)
                        .trim_start_matches('/')
                } else {
                    &entry.name
                };
                let label = format!(
                    "{} {}{}",
                    if entry.directory { "▸" } else { "·" },
                    display_path(name),
                    if entry.directory { "/" } else { "" }
                );
                if code.search.is_some() {
                    let name = name.to_owned();
                    paint::matched_label(&mut code, i, &name, Rect::new(rect.x + 10.*s, y + 9.*s, rect.width - 20.*s, row_height), viewport, layer, cx);
                } else { cx.services.renderer.clipped_label(
                    layer,
                    &label,
                    Rect::new(rect.x + 10. * s, y + 11. * s, rect.width - 20. * s, 24. * s),
                    14. * s,
                    color(if entry.directory { 0x8bd6ff } else { 0xd8dee9 }),
                    false,
                    viewport,
                );
                }
                if hit.height > 0. {
                    let action = if code.search.is_some() { Choice::FileSelect(entry.path.clone()) } else { Choice::FileOpen(entry.path.clone(), entry.directory) };
                    self.controls.place(action, rect, viewport, false).control.highlight(layer, cx.ui, false);
                }
            }
            code.paints.retain(|id, (_, paint)| {
                let keep = (first..end).contains(&(*id as usize));
                if !keep && let Some(p) = paint.take() { cx.services.renderer.text.drop_paint(p); }
                keep
            });
            if code.len() == 0 {
                let text = code.error.as_deref().unwrap_or(if code.loading {
                    "Loading remote files…"
                } else if code.search.is_some() && code.index_seen.as_ref().is_none_or(|u| u.index.is_none() || u.indexing) {
                    "Syncing file names…"
                } else if code.search.is_some() && code.matches.as_ref().is_none_or(|m| m.generation != code.match_generation) {
                    "Matching…"
                } else if code.search.is_some() {
                    "No matching files"
                } else {
                    "No visible entries on this page"
                });
                cx.services.renderer.label(
                    layer,
                    text,
                    Rect::new(viewport.x + 24. * s, viewport.y + 32. * s, viewport.width - 48. * s, 100. * s),
                    14. * s,
                    color(0x82909f),
                    false,
                );
            }
        }
        code.preview_viewport = Rect::new(0., 0., 0., 0.);
        if code.search.is_some() {
            let header_y = viewport.y + viewport.height;
            layer.rect(Rect::new(b.x, header_y, b.width, 36.*s), color(0x111923));
            let selected = code.entry(code.row);
            let title = selected.as_ref().map(|e| e.path.strip_prefix(code.path.as_deref().unwrap_or("")).unwrap_or(&e.path).trim_start_matches('/')).unwrap_or("Preview");
            cx.services.renderer.label(layer, title, Rect::new(b.x + 14.*s, header_y + 9.*s, (b.width - 100.*s).max(1.), 24.*s), 12.*s, color(0xb7c2ce), false);
            if selected.is_some() {
                self.controls.button(
                    cx,
                    layer,
                    Rect::new(b.x + b.width - 76. * s, header_y + 2. * s, 64. * s, 32. * s),
                    "Open",
                    Choice::FileAccept,
                    ButtonStyle::Tonal,
                    frame.clip,
                );
            }
            let preview = Rect::new(b.x, header_y + 36.*s, b.width, (bottom - header_y - 36.*s).max(1.));
            code.preview_viewport = preview;
            layer.rect(preview, color(0x0b1118));
            if let Some(doc) = &code.preview {
                paint::document(doc, None, &mut code.preview_paints, &mut code.preview_scroll, &mut code.preview_horizontal, preview, layer, cx);
            }
            if code.preview.is_none() || code.preview_error.is_some() {
                let text = code.preview_error.as_deref().unwrap_or(if code.preview_loading { "Loading preview…" } else { "Select a file to preview" });
                layer.rect(Rect::new(preview.x, preview.y, preview.width, 50.*s), color(0x0b1118));
                cx.services.renderer.clipped_label(layer, text, Rect::new(preview.x + 14.*s, preview.y + 14.*s, (preview.width - 28.*s).max(1.), 36.*s), 13.*s, color(0x82909f), false, preview);
            }
            code.preview_scroll.rect = preview; code.preview_horizontal.rect = preview;
            code.preview_scroll.paint(layer, cx);
        }
        if paging {
            let y = bottom - 44. * s;
            if code.pages.len() > 1 {
                self.controls.button(
                    cx,
                    layer,
                    Rect::new(b.x + 12. * s, y, 88. * s, 40. * s),
                    "Previous",
                    Choice::FilePage(false),
                    ButtonStyle::Tonal,
                    frame.clip,
                );
            }
            if code.next.is_some() {
                self.controls.button(
                    cx,
                    layer,
                    Rect::new(b.x + b.width - 100. * s, y, 88. * s, 40. * s),
                    "Next",
                    Choice::FilePage(true),
                    ButtonStyle::Tonal,
                    frame.clip,
                );
            }
        }
        code.scroll.rect = viewport;
        code.horizontal.rect = viewport;
        self.surface.rect = Some(viewport);
        self.surface.clip = frame.clip;
        code.scroll.paint(layer, cx);
        self.view = Some(code);
        self.controls.finish(cx);
    }
}

#[cfg(all(test, not(target_os = "android")))]
mod tests;
