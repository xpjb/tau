//! Directory/code surface, shared by desktop and Android. The renderer owns only
//! visible line paint; the worker owns parsing, and selections use source line IDs.
use super::*;
use ui::{Context, Frame, Event as InputEvent};
use ui::controls::{Controls,TextField};
use ui::scroll::ScrollState;
use ui::controls::{button as retained_button,icon_button as retained_icon};
use std::{borrow::Cow, sync::Arc};
use tau_code_viewer::{Document, Selection};
use tau_protocol::files::*;
use sanscale::{Align, BlockKey, Draw, PaintHandle, PaintSpan, ParagraphKey, ParagraphSource, Style};

#[cfg(test)]
pub(super) const SEARCH_FIELD: usize = usize::MAX;
pub(super) struct View {
    id:ui::Id, identity: String, lineage: Option<String>, session: String,
    generation: u64, subscribed: bool, seen: Option<Arc<crate::file_client::Update>>,
    pub(super) path: Option<String>, directory: Option<String>, parent: Option<String>, search_root: Option<String>,
    operation: FileOperation, pub(super) search: Option<TextField>,
    entries: Vec<FileEntry>, next: Option<String>, pages: Vec<Option<String>>, row: usize,
    pub(super) document: Option<Arc<Document>>, pub(super) selection: Option<Selection>,
    /// The exact generated reference and its byte position. Never global replace
    /// user prose or other references when a live file moves selected lines.
    pub(super) reference: Option<(usize, String)>,
    reference_sync: Option<bool>,
    pub(super) drag_anchor: Option<u64>,
    pub(super) scroll: ScrollState, horizontal: ScrollState,
    viewport: Rect, line_height: f32, gutter: f32,
    cursor: usize, loading: bool, pub(super) error: Option<String>, status: String,
    paints: HashMap<u64, (Vec<PaintSpan>, Option<PaintHandle>)>,
}
impl View {
    fn new(model: &Controller, session: String) -> Self {
        let id=ui::Id::new();
        Self { id, identity: model.identity.clone(), lineage: model.account.source_lineage.clone(), session,
            generation: 0, subscribed: false, seen: None, path: None, directory: None, parent: None, search_root: None,
            operation: FileOperation::List { after: None }, search: None,
            entries: vec![], next: None, pages: vec![None], row: 0, document: None, selection: None, reference: None, reference_sync: None, drag_anchor: None,
            scroll: ScrollState::new(id,false), horizontal: ScrollState::new(id,true), viewport: Rect::new(0.,0.,0.,0.), line_height: 24., gutter: 52.,
            cursor: 0, loading: true, error: None, status: String::new(), paints: HashMap::new() }
    }
    pub(super) fn sent(&mut self) {self.selection=None;self.reference=None;self.reference_sync=None;}
    fn line_at(&self, point: Vec2) -> Option<usize> {
        let doc = self.document.as_ref()?;
        if self.search.is_some() || doc.lines.is_empty() { return None; }
        Some(((point.y-self.viewport.y+self.scroll.value).max(0.)/self.line_height) as usize).map(|n| n.min(doc.lines.len()-1))
    }
    fn clear_paint(&mut self, renderer: &mut Renderer) {
        for (_, (_, paint)) in self.paints.drain() { if let Some(paint)=paint { renderer.text.drop_paint(paint); } }
    }
}
fn parent(path: &str) -> Option<String> {
    let path = path.trim_end_matches(['/', '\\']);
    let at = path.rfind(['/', '\\'])?;
    Some(if at == 0 { "/".into() } else { path[..at].into() })
}
fn display_path(path: &str) -> String { path.chars().map(|c| if c.is_control() { '�' } else { c }).collect() }
struct Source<'a>(&'a str);
impl ParagraphSource for Source<'_> {
    fn paragraph_text(&self, _: usize, _: ParagraphKey) -> Option<Cow<'_, str>> { Some(Cow::Borrowed(self.0)) }
}
#[derive(Clone,Debug)]
pub(in crate::app) enum Choice {Files,FileClose,FileOpen(String,bool),FileUp,FileFindHere,FileFind,FileClear,FileCopy,FilePage(bool)}
pub(in crate::app) struct CodeBrowser {pub view:Option<View>,pub controls:Controls<Choice>,surface:ui::controls::Control,pub pointer:Option<Pointer>,pub composer_bottom:Option<f32>}
impl CodeBrowser {
    pub fn new()->Self {let id=ui::Id::new();Self {view:None,controls:Controls::new(id),surface:ui::controls::Control::new(id,false),pointer:None,composer_bottom:None}}
    fn cancel_pointer(&mut self,cx:&mut Context<'_>){self.pointer=None;cx.ui.detach(self.controls.id);if let Some(code)=&mut self.view {code.drag_anchor=None;code.scroll.stop();code.horizontal.stop();cx.ui.detach(code.id);}}
    fn activate(&mut self,action:Choice,cx:&mut Context<'_>){let result=self.code_action(action,cx);cx.report(result);}

    pub(super) fn close_code(&mut self,cx:&mut Context<'_>) {
        if self.view.is_some() {self.cancel_pointer(cx);}
        if let Some(mut view) = self.view.take() {
            cx.ui.detach(view.id);cx.ui.search=None;self.controls.begin();self.surface.rect=None;
            view.clear_paint(&mut cx.services.renderer);
            let _ = cx.model.view_files(None);
            cx.ui.focus = None;
        }
    }
    fn code_request(&mut self,cx:&mut Context<'_>) {
        let Some(code)=&mut self.view else { return; };
        let request=FileRequest { session_id:code.session.clone(), path:code.path.clone(), operation:code.operation.clone() };
        code.seen=None; code.loading=true;
        let previous=matches!(code.operation,FileOperation::Open {..}).then(||code.document.clone()).flatten();
        match cx.model.view_files_document(Some(request),previous) {
            Ok(generation)=>{code.generation=generation;code.subscribed=true;code.error=None;}
            Err(error)=>{code.error=Some(error.to_string());code.loading=false;code.subscribed=false;}
        }
        cx.ui.dirty=true;
    }
    pub(super) fn code_action(&mut self, action: Choice,cx:&mut Context<'_>) -> Result<()> {
        match action {
            Choice::Files => {
                if self.view.is_some() { self.close_code(cx); return Ok(()); }
                let Some(session)=cx.model.account.selected.clone() else { return Ok(()); };
                cx.model.save_chat(&session)?; self.cancel_pointer(cx); cx.ui.focus=None;
                self.view=Some(View::new(cx.model,session)); self.code_request(cx);
            }
            Choice::FileClose => { self.close_code(cx); }
            Choice::FileOpen(path, directory) => {
                self.cancel_pointer(cx);
                let Some(code)=&mut self.view else { return Ok(()); };
                code.clear_paint(&mut cx.services.renderer);
                code.path=Some(path); code.operation=if directory {FileOperation::List {after:None}} else {FileOperation::Open {revision:None}};
                code.search=None;code.document=None;code.selection=None;code.reference=None;code.reference_sync=None;code.drag_anchor=None;
                code.entries.clear();code.pages=vec![None];code.next=None;code.scroll.value=0.;code.horizontal.value=0.;code.row=0;code.cursor=0;
                cx.ui.focus=None;self.code_request(cx);
            }
            Choice::FileUp => {
                if let Some(path)=self.view.as_ref().and_then(|c| if c.document.is_some() {c.path.as_deref().and_then(parent)} else {c.parent.clone()}) {
                    self.code_action(Choice::FileOpen(path,true),cx)?;
                }
            }
            Choice::FileFindHere => {
                if let Some(code)=&mut self.view && let Some(search)=&code.search {
                    code.path=code.directory.clone();code.operation=FileOperation::Search {query:search.editor.value.clone()};
                    code.entries.clear();code.scroll.value=0.;code.row=0;self.code_request(cx);
                }
            }
            Choice::FileFind => {
                self.cancel_pointer(cx);
                if self.view.is_none() { self.code_action(Choice::Files,cx)?; }
                let Some(code)=&mut self.view else { return Ok(()); };
                if code.search.is_some() { self.code_back(cx); return Ok(()); }
                code.search=Some(TextField::new(code.id,"",Editor::line(String::new())));
                let field=code.search.as_mut().unwrap();field.size=16.;field.placeholder="Fuzzy find paths…".into();
                code.path=code.search_root.clone();
                code.operation=FileOperation::Search {query:String::new()};code.entries.clear();code.row=0;code.scroll.value=0.;
                cx.ui.focus=Some(code.search.as_ref().unwrap().control.target);cx.ui.search=cx.ui.focus;self.code_request(cx);
                if cx.ui.mobile && let Some(field)=self.view.as_ref().and_then(|c|c.search.as_ref()) {cx.focus_native(field.control.target,field.editor.native_id());}
            }
            Choice::FileClear => { if let Some(code)=&mut self.view {code.selection=None;code.reference=None;code.reference_sync=None;code.drag_anchor=None;}cx.ui.focus=None; }
            Choice::FileCopy => {
                if let Some(code)=&self.view && let (Some(doc),Some(selection))=(&code.document,&code.selection)
                    && let Some(text)=doc.selected_text(selection) {cx.services.platform.push(PlatformAction::Copy(text));}
            }
            Choice::FilePage(next) => {
                let Some(code)=&mut self.view else {return Ok(());};
                if next {if let Some(next)=code.next.clone() {code.pages.push(Some(next));}else{return Ok(());}}
                else if code.pages.len()>1 {code.pages.pop();}else{return Ok(());}
                code.operation=FileOperation::List {after:code.pages.last().cloned().flatten()};code.scroll.value=0.;code.row=0;code.entries.clear();self.code_request(cx);
            }
            _ => {}
        }
        cx.ui.dirty=true;Ok(())
    }
    pub(super) fn code_back(&mut self,cx:&mut Context<'_>) {
        self.cancel_pointer(cx);
        if let Some(code)=&mut self.view && code.search.take().is_some() {
            cx.ui.detach(code.id);cx.ui.search=None;
            code.path=code.document.as_ref().map(|d|d.path.clone()).or_else(||code.directory.clone());
            code.operation=if code.document.is_some(){FileOperation::Open {revision:None}}else{FileOperation::List {after:None}};
            code.entries.clear();code.pages=vec![None];code.next=None;code.scroll.value=0.;code.row=0;cx.ui.focus=None;self.code_request(cx);
        } else if self.view.as_ref().is_some_and(|c|c.document.is_some()) {
            let result=self.code_action(Choice::FileUp,cx);cx.report(result);
        } else {self.close_code(cx);}
    }
    pub(super) fn code_query(&mut self,cx:&mut Context<'_>) {
        if let Some(code)=&mut self.view && let Some(search)=&code.search {
            code.operation=FileOperation::Search {query:search.editor.value.chars().take(128).collect()};
            code.entries.clear();code.scroll.value=0.;code.row=0;self.code_request(cx);
        }
    }
    /// Persist once per completed gesture/live revision, not on every pointer
    /// motion. Keep a user's draft and only replace our own still-intact marker.
    pub(super) fn code_reference(&mut self, remove: bool,cx:&mut Context<'_>) {
        let Some(code)=&mut self.view else {return;};
        if cx.model.account.selected.as_ref()!=Some(&code.session) || cx.model.identity!=code.identity {return;}
        if !remove && code.error.is_some() {return;}
        let next=if remove {None} else {code.document.as_ref().zip(code.selection.as_ref()).and_then(|(d,s)|s.reference(d)).map(|r| {let fence="`".repeat(r.split(|c|c!='`').map(str::len).max().unwrap_or(0)+1);format!("{fence}{r}{fence}\n")})};
        let Some(chat)=cx.model.chats.get(&code.session) else {return;};
        let mut draft=chat.local.draft.clone();
        let old=code.reference.take();
        if let Some((start, old))=old {
            let start=if draft.get(start..start+old.len())==Some(old.as_str()) {Some(start)} else {
                let mut matches=draft.match_indices(&old);
                let first=matches.next().map(|(at,_)|at);
                if matches.next().is_none() {first} else {None}
            };
            if let Some(start)=start {
                let replacement=next.as_deref().unwrap_or("");draft.replace_range(start..start+old.len(),replacement);
                code.reference=next.map(|r|(start,r));
            }
            // A marker the user edited/removed is no longer ours to rewrite.
        } else if let Some(reference)=next {
            if !draft.is_empty() && !draft.ends_with("\n\n") {draft.push_str(if draft.ends_with('\n') {"\n"} else {"\n\n"});}
            let start=draft.len();draft.push_str(&reference);code.reference=Some((start,reference));
        }
        // Rebinding the inline editor fences all native snapshots of older markers.
        if draft!=cx.model.chats[&code.session].local.draft {
            match cx.model.draft(draft.clone()) {
                Ok(())=>{},
                Err(error)=>cx.model.report_error(error),
            }
        }
    }
    pub(super) fn code_tick(&mut self, dt: f32,cx:&mut Context<'_>) {
        let Some(code)=&self.view else {return;};
        if code.identity!=cx.model.identity || code.session!=cx.model.account.selected.as_deref().unwrap_or("") || code.lineage!=cx.model.account.source_lineage {
            self.close_code(cx);cx.ui.dirty=true;return;
        }
        let active=cx.ui.window_focused && !cx.ui.covered;
        if !active && code.subscribed {
            let _=cx.model.view_files(None);self.view.as_mut().unwrap().subscribed=false;
        } else if active && (!code.subscribed || code.generation!=cx.model.viewer_generation()) && cx.model.epoch.is_some() {self.code_request(cx);}
        let update=cx.model.file_update.clone();
        if let Some(update)=update && self.view.as_ref().is_some_and(|c|update.generation==c.generation && update.session==c.session && update.lineage==c.lineage.as_deref().unwrap_or("") && c.seen.as_ref().is_none_or(|old|!Arc::ptr_eq(old,&update))) {
            let code=self.view.as_mut().unwrap();code.seen=Some(update.clone());code.loading=false;
            let mut invalidated=false;let mut moved=false;
            match &update.response {
                Err(error)=>{code.error=Some(error.clone());code.status="Preview unavailable · retrying".into();code.drag_anchor=None;}
                Ok(reply)=>{
                    code.error=None;
                    match reply {
                        FileReply::Directory {path,parent,entries,next}=>{
                            if code.search_root.is_none() {code.search_root=Some(path.clone());}
                            code.path=Some(path.clone());code.directory=Some(path.clone());code.parent=parent.clone();code.entries=entries.clone();code.next=next.clone();
                            code.status="Read only · ignored files remain browsable".into();
                        }
                        FileReply::Search {path,entries,indexing,limited}=>{
                            if code.search_root.is_none() {code.search_root=Some(path.clone());}
                            code.path=Some(path.clone());code.entries=entries.clone();code.status=if *indexing {"Indexing paths…"} else if *limited {"Partial index · Here searches this folder"} else {"Fuzzy paths · respects .gitignore"}.into();
                        }
                        FileReply::Text {path,..} | FileReply::Unchanged {path,..}=>{
                            if let Some(doc)=&update.document {
                                let cursor=code.document.as_ref().and_then(|d|d.lines.get(code.cursor)).map(|l|l.id);
                                if let Some(cursor)=cursor && let Some(line)=doc.position(cursor) {code.cursor=line;}
                                let top=code.document.as_ref().and_then(|d|d.lines.get((code.scroll.value/code.line_height) as usize)).map(|l|l.id);
                                if let Some(top)=top && let Some(line)=doc.position(top) {code.scroll.value=line as f32*code.line_height+code.scroll.value%code.line_height;}
                                if let Some(selection)=&code.selection {
                                    if selection.range(doc).is_none() {invalidated=true;code.selection=None;code.drag_anchor=None;}
                                    else {moved=true;}
                                }
                                if code.document.as_ref().is_some_and(|old|old.namespace!=doc.namespace) {code.clear_paint(&mut cx.services.renderer);}
                                code.document=Some(doc.clone());code.cursor=code.cursor.min(doc.lines.len()-1);
                            }
                            code.path=Some(path.clone());code.directory=parent(path);
                            code.status=if invalidated {"Selected text changed · reselect to comment (draft kept)"} else {"Read only · live · select line numbers to comment"}.into();
                        }
                    }
                }
            }
            code.row=code.row.min(code.entries.len().saturating_sub(1));
            if invalidated {self.view.as_mut().unwrap().reference_sync=Some(true);}
            else if moved && self.view.as_ref().is_some_and(|c|c.reference.is_some()) {self.view.as_mut().unwrap().reference_sync=Some(false);}
            cx.ui.dirty=true;
        }
        if !cx.ui.composing && let Some(remove)=self.view.as_mut().and_then(|c|c.reference_sync.take()) {self.code_reference(remove,cx);cx.ui.dirty=true;}
        // Hold in the code body promotes scrolling to line-range selection. A
        // gutter press selects immediately, and dragging back shrinks the range.
        if active && let Some(p)=&self.pointer && p.touch && !p.dragged && p.started.elapsed().as_millis()>=450
            && self.view.as_ref().is_some_and(|c|c.error.is_none() && c.search.is_none() && c.document.is_some() && contains(c.viewport,p.start) && c.drag_anchor.is_none()) {
            let point=p.start;self.code_begin(point,cx);cx.services.platform.push(PlatformAction::Haptic);
        }
        if let Some(code)=&mut self.view && code.drag_anchor.is_some() && let Some(p)=&self.pointer {
            let margin=16.*cx.ui.scale;
            let delta=if p.last.y<code.viewport.y+margin {p.last.y-code.viewport.y-margin}
                else {(p.last.y-code.viewport.y-code.viewport.height+margin).max(0.)};
            let next=(code.scroll.value+delta.clamp(-90.*cx.ui.scale,90.*cx.ui.scale)*12.*dt.min(0.05)).clamp(0.,code.scroll.max);
            if next!=code.scroll.value {code.scroll.value=next;let point=p.last;self.code_extend(point,cx);cx.ui.dirty=true;}
        }
    }
    fn code_begin(&mut self, point: Vec2,cx:&mut Context<'_>) {
        let Some(code)=&mut self.view else{return;};
        let Some(line)=code.line_at(point) else{return;};let doc=code.document.as_ref().unwrap();
        code.cursor=line;code.selection=Some(Selection::new(doc,line,line));code.drag_anchor=Some(doc.lines[line].id);
        code.scroll.stop();code.horizontal.stop();if let Some(c)=&mut cx.ui.capture {c.target=self.surface.target;c.claimed=true;}
        cx.ui.focus=None;cx.ui.dirty=true;
    }
    fn code_extend(&mut self, point: Vec2,cx:&mut Context<'_>) {
        let Some(code)=&mut self.view else{return;};
        let Some(end)=code.line_at(point) else{return;};let doc=code.document.as_ref().unwrap();
        let Some(anchor)=code.drag_anchor.and_then(|id|doc.position(id)) else{return;};
        code.selection=Some(Selection::new(doc,anchor,end));code.cursor=end;cx.ui.dirty=true;
    }
    pub(super) fn code_press(&mut self, id:u64, point:Vec2, touch:bool,cx:&mut Context<'_>)->bool {
        if cx.ui.covered {return false;}
        let Some(code)=&self.view else{return false;};
        if code.error.is_some() || code.document.is_none() || code.search.is_some() || !contains(code.viewport,point) || touch && point.x>code.viewport.x+code.gutter {return false;}
        self.pointer=Some(Pointer {id,start:point,last:point,at:Instant::now(),started:Instant::now(),dragged:false,touch});
        cx.ui.capture=Some(ui::Capture {target:self.surface.target,pointer:id,start:point,point,touch,dragged:false,claimed:true,started:Instant::now()});self.code_begin(point,cx);true
    }
    pub(super) fn code_motion(&mut self,id:u64,point:Vec2,cx:&mut Context<'_>)->bool {
        if cx.ui.covered {return false;}
        let (Some(code),Some(p))=(&mut self.view,&mut self.pointer) else{return false;};
        if p.id!=id || !contains(code.viewport,p.start) {return false;}
        let selecting=code.drag_anchor.is_some();
        p.dragged|=(point.x-p.start.x).abs()+(point.y-p.start.y).abs()>7.*cx.ui.scale;

        p.last=point;p.at=Instant::now();if selecting {self.code_extend(point,cx);}cx.ui.dirty=true;true
    }
    pub(super) fn code_release(&mut self,id:u64,point:Vec2,cx:&mut Context<'_>)->bool {
        if self.view.as_ref().is_none_or(|c|c.drag_anchor.is_none()) || self.pointer.as_ref().is_none_or(|p|p.id!=id) {return false;}
        self.code_extend(point,cx);self.view.as_mut().unwrap().drag_anchor=None;self.pointer=None;
        cx.ui.capture=None;self.code_reference(false,cx);
        if !cx.ui.mobile {cx.ui.focus=cx.ui.composer;}cx.ui.dirty=true;true
    }
    pub(super) fn code_wheel(&mut self,amount:f32,horizontal:bool,point:Vec2,cx:&mut Context<'_>)->bool {
        let Some(code)=&mut self.view else{return false;};if !contains(code.viewport,point){return false;}
        if horizontal {code.horizontal.value=(code.horizontal.value+amount).clamp(0.,code.horizontal.max);}
        else {code.scroll.value=(code.scroll.value+amount).clamp(0.,code.scroll.max);}
        cx.ui.dirty=true;true
    }
    pub(super) fn code_key(&mut self,key:&str,ctrl:bool,shift:bool,cx:&mut Context<'_>)->bool {
        if ctrl && matches!(key,"Space"|" ") {self.activate(Choice::FileFind,cx);return true;}
        let Some(code)=&mut self.view else{return false;};
        if key=="Escape" {
            if code.search.is_some() {self.code_back(cx);}
            else if code.selection.is_some() {self.activate(Choice::FileClear,cx);}
            else {self.code_back(cx);}cx.ui.dirty=true;return true;
        }
        if cx.ui.composer.is_some() && cx.ui.focus==cx.ui.composer {return false;}
        if ctrl && key.eq_ignore_ascii_case("c") && code.search.is_none() {self.activate(Choice::FileCopy,cx);return true;}
        if code.document.is_none() || code.search.is_some() {
            match key {
                "ArrowUp"|"k" if code.search.is_none() || key=="ArrowUp"=>code.row=code.row.saturating_sub(1),
                "ArrowDown"|"j" if code.search.is_none() || key=="ArrowDown"=>code.row=(code.row+1).min(code.entries.len().saturating_sub(1)),
                "Enter"|"l" if code.search.is_none() || key=="Enter"=>{if let Some(entry)=code.entries.get(code.row) {let action=Choice::FileOpen(entry.path.clone(),entry.directory);self.activate(action,cx);}return true;}
                "Backspace"|"h" if code.search.is_none()=>{self.activate(Choice::FileUp,cx);return true;}
                _=>return false,
            }
            let top=code.row as f32*44.*cx.ui.scale;
            if top<code.scroll.value {code.scroll.value=top;}else if top+44.*cx.ui.scale>code.scroll.value+code.viewport.height {code.scroll.value=(top+44.*cx.ui.scale-code.viewport.height).min(code.scroll.max);}
        } else {
            if code.error.is_some() && (shift || key=="v") {return true;}
            let doc=code.document.as_ref().unwrap();let old=code.cursor;
            let page=(code.viewport.height/code.line_height).floor().max(1.) as usize;
            match key {
                "ArrowUp"|"k"=>code.cursor=old.saturating_sub(1),
                "ArrowDown"|"j"=>code.cursor=(old+1).min(doc.lines.len()-1),
                "PageUp"=>code.cursor=old.saturating_sub(page),
                "PageDown"=>code.cursor=(old+page).min(doc.lines.len()-1),
                "Home"=>code.cursor=0,
                "End"=>code.cursor=doc.lines.len()-1,
                "ArrowLeft"=>{code.horizontal.value=(code.horizontal.value-48.*cx.ui.scale).max(0.);cx.ui.dirty=true;return true;}
                "ArrowRight"=>{code.horizontal.value=(code.horizontal.value+48.*cx.ui.scale).min(code.horizontal.max);cx.ui.dirty=true;return true;}
                "Backspace"|"h"=>{self.activate(Choice::FileUp,cx);return true;}
                "v"=>{code.selection=Some(Selection::new(doc,old,old));self.code_reference(false,cx);cx.ui.dirty=true;return true;}
                _=>return false,
            }
            if shift {let anchor=code.selection.as_ref().and_then(|s|doc.position(s.anchor)).unwrap_or(old);code.selection=Some(Selection::new(doc,anchor,code.cursor));}
            let top=code.cursor as f32*code.line_height;
            if top<code.scroll.value {code.scroll.value=top;}else if top+code.line_height>code.scroll.value+code.viewport.height {code.scroll.value=(top+code.line_height-code.viewport.height).min(code.scroll.max);}
            if shift {self.code_reference(false,cx);}
        }
        cx.ui.dirty=true;true
    }

}

impl Widget for CodeBrowser {
    fn handle_event(&mut self,event:&InputEvent<'_>,cx:&mut Context<'_>)->bool{
        if matches!(event,InputEvent::Cancel){self.cancel_pointer(cx);return false;}
        if let InputEvent::Text(text)=event && cx.ui.focus.is_none() && self.code_key(text,false,false,cx){return true;}
        if let InputEvent::Key {key,ctrl,shift}=*event && !cx.ui.composing && self.code_key(key,ctrl,shift,cx){return true;}
        let Some(code)=&self.view else{return false;};
        if code.identity!=cx.model.identity || code.lineage!=cx.model.account.source_lineage || Some(&code.session)!=cx.model.account.selected.as_ref(){self.close_code(cx);return false;}
        if matches!(event,InputEvent::Back){self.code_back(cx);return true;}
        if let InputEvent::Tick(dt)=*event {self.code_tick(dt,cx);}
        let Some(code)=&mut self.view else{return false;};
        if let Some(field)=&mut code.search {let old=field.editor.value.clone();let handled=field.handle_event(event,cx);let changed=old!=field.editor.value;if changed{self.code_query(cx);}if handled{return true;}}
        if self.view.as_mut().is_some_and(|v|v.scroll.bar_event(event,cx)){return true;}
        let (child,choice)=self.controls.event(event,cx);
        if let Some(choice)=choice {self.activate(choice,cx);return true;}
        match *event {
            InputEvent::Down {pointer,point,touch}=>{
                if !child && self.code_press(pointer,point,touch,cx){return true;}
                if self.view.as_ref().is_some_and(|v|contains(v.viewport,point)){self.pointer=Some(Pointer {id:pointer,start:point,last:point,at:Instant::now(),started:Instant::now(),dragged:false,touch});}
            }
            InputEvent::Move {pointer,point}=>{
                if self.view.as_ref().is_some_and(|v|v.drag_anchor.is_some()) && self.code_motion(pointer,point,cx){return true;}
                if let Some(p)=&mut self.pointer && p.id==pointer {p.dragged|=(point.x-p.start.x).abs()+(point.y-p.start.y).abs()>7.*cx.ui.scale;p.last=point;}
            }
            InputEvent::Up {pointer,point}=>{if self.code_release(pointer,point,cx){return true;}if self.pointer.as_ref().is_some_and(|p|p.id==pointer){self.pointer=None;}}
            InputEvent::Wheel {amount,horizontal,point}=>{return self.code_wheel(amount,horizontal,point,cx);}
            _=>{}
        }
        let Some(code)=&mut self.view else{return child;};
        let horizontal=match event {InputEvent::Move {point,..}=>self.pointer.as_ref().is_some_and(|p|(point.x-p.start.x).abs()>1.5*(point.y-p.start.y).abs()),_=>false};
        if horizontal {let h=code.horizontal.event(event,child,cx);code.scroll.event(event,h,cx)}
        else {let v=code.scroll.event(event,child,cx);if !matches!(event,InputEvent::Move {..}){code.horizontal.event(event,v,cx)}else{v}}
    }
    fn visit_perframe(&mut self,frame:&mut Frame<'_>,cx:&mut Context<'_>){
        if self.view.is_none(){return;}

        self.controls.begin();let b=frame.bounds;let layer=&mut *frame.layer;let s=cx.ui.scale;let mut code=self.view.take().unwrap();
        let comments=code.search.is_none() && code.document.is_some() && (code.selection.is_some() || cx.ui.composer.is_some() && cx.ui.focus==cx.ui.composer);
        let bottom=self.composer_bottom.unwrap_or(b.y+b.height);
        layer.rect(Rect::new(b.x,b.y,b.width,124.*s),color(0x0e141b));
        retained_button(&mut cx.services.renderer,layer,&mut self.controls,Rect::new(b.x+8.*s,b.y+6.*s,64.*s,40.*s),"‹ Chat",Choice::FileClose,s,false);
        if code.search.is_some() {
            retained_button(&mut cx.services.renderer,layer,&mut self.controls,Rect::new(b.x+76.*s,b.y+6.*s,44.*s,40.*s),"Here",Choice::FileFindHere,s,false);
        } else if code.parent.is_some() || code.document.is_some() {
            retained_button(&mut cx.services.renderer,layer,&mut self.controls,Rect::new(b.x+76.*s,b.y+6.*s,44.*s,40.*s),"Up",Choice::FileUp,s,false);
        }
        cx.services.renderer.label(layer,"Files",Rect::new(b.x+130.*s,b.y+15.*s,(b.width-240.*s).max(1.),24.*s),16.*s,color(0xe5eaf0),true);
        retained_button(&mut cx.services.renderer,layer,&mut self.controls,Rect::new(b.x+b.width-104.*s,b.y+6.*s,60.*s,40.*s),if code.search.is_some(){"Done"}else{"Find"},Choice::FileFind,s,code.search.is_some());
        retained_icon(cx,&mut self.controls,layer,Rect::new(b.x+b.width-44.*s,b.y+6.*s,40.*s,40.*s),Icon::Close,18.,Choice::FileClose,false,true);
        cx.services.renderer.label(layer,&display_path(code.path.as_deref().unwrap_or("Chat working directory")),Rect::new(b.x+14.*s,b.y+54.*s,b.width-28.*s,28.*s),13.*s,color(0xb7c2ce),false);
        let top=if let Some(search)=&mut code.search {
            let rect=Rect::new(b.x+12.*s,b.y+88.*s,b.width-24.*s,48.*s);
            search.visit_perframe(&mut Frame {layer,bounds:rect,clip:frame.clip},cx);
            cx.services.renderer.label(layer,code.error.as_deref().unwrap_or(if code.loading {"Searching…"} else {&code.status}),Rect::new(b.x+14.*s,b.y+139.*s,b.width-28.*s,18.*s),11.*s,color(0x82909f),false);
            b.y+160.*s
        } else {
            let status=code.error.as_deref().unwrap_or(if code.loading {"Loading…"} else {&code.status});
            let label=code.error.is_none().then(||code.selection.as_ref().zip(code.document.as_ref()).and_then(|(selection,doc)|selection.range(doc)).map(|r|format!("Lines {}–{}",r.start+1,r.end))).flatten();
            cx.services.renderer.label(layer,label.as_deref().unwrap_or(status),Rect::new(b.x+14.*s,b.y+94.*s,(b.width-if comments{158.*s}else{28.*s}).max(1.),24.*s),12.*s,color(if code.error.is_some(){0xf2a6a6}else{0x82909f}),false);
            if comments {
                retained_button(&mut cx.services.renderer,layer,&mut self.controls,Rect::new(b.x+b.width-140.*s,b.y+84.*s,62.*s,40.*s),"Copy",Choice::FileCopy,s,false);
                retained_button(&mut cx.services.renderer,layer,&mut self.controls,Rect::new(b.x+b.width-74.*s,b.y+84.*s,62.*s,40.*s),"Clear",Choice::FileClear,s,false);
            }
            b.y+124.*s
        };
        let paging=code.search.is_none() && code.document.is_none() && (code.next.is_some() || code.pages.len()>1);
        code.viewport=Rect::new(b.x,top,b.width,(bottom-top-if paging{48.*s}else{0.}).max(1.));
        let viewport=code.viewport;
        layer.rect(viewport,color(0x0b1118));layer.rect(Rect::new(b.x,top-s,b.width,s),color(0x2a3541));
        if let Some(doc)=&code.document && code.search.is_none() {
            code.line_height=if cx.ui.mobile {28.}else{24.}*s;
            code.gutter=(44.+doc.lines.len().to_string().len().saturating_sub(3) as f32*8.)*s;
            code.scroll.max=(doc.lines.len() as f32*code.line_height+16.*s-viewport.height).max(0.);
            code.scroll.value=code.scroll.value.clamp(0.,code.scroll.max);
            let first=(code.scroll.value/code.line_height).floor() as usize;
            let end=(first+(viewport.height/code.line_height).ceil() as usize+2).min(doc.lines.len());
            let selection=code.selection.as_ref().and_then(|s|s.range(doc));
            let mut visible=HashSet::new();let mut widest=viewport.width;
            layer.rect(Rect::new(viewport.x,viewport.y,code.gutter,viewport.height),color(0x111923));
            for i in first..end {
                let line=&doc.lines[i];visible.insert(line.id);
                let y=viewport.y+i as f32*code.line_height-code.scroll.value;
                if selection.as_ref().is_some_and(|r|r.contains(&i)) {layer.clipped_rect(Rect::new(viewport.x,y,viewport.width,code.line_height),color(0x213c57),viewport);}
                cx.services.renderer.clipped_label(layer,&(i+1).to_string(),Rect::new(viewport.x+8.*s,y+5.*s,code.gutter-14.*s,code.line_height),12.*s,color(if selection.as_ref().is_some_and(|r|r.contains(&i)){0x8bd6ff}else{0x6c7d90}),false,viewport);
                // Cap a single pathological/minified line's shaping work while
                // retaining its original full text for selection and copying.
                let mut stop=line.text.len().min(16*1024);while !line.text.is_char_boundary(stop){stop-=1;}
                let mut text=line.text[..stop].replace('\t',"    ");
                if stop<line.text.len() {text.push_str(" … [long line preview limited; Copy keeps full text]");}
                let tabs=line.text[..stop].contains('\t');
                let byte=|n:usize|{let n=n.min(stop);n+if tabs {line.text[..n].bytes().filter(|&b|b==b'\t').count()*3} else {0}};
                let spans=line.paint.iter().filter(|p|p.range.start<stop).map(|p|PaintSpan {range:byte(p.range.start)..byte(p.range.end),color:color(p.color)}).collect::<Vec<_>>();
                let cached=code.paints.entry(line.id).or_insert_with(||(vec![],None));
                if cached.0!=spans {if let Some(p)=cached.1.take(){cx.services.renderer.text.drop_paint(p);}cached.1=if spans.is_empty(){None}else{cx.services.renderer.text.register_paint(&spans).ok()};cached.0=spans;}
                let namespace=0x5441_5546_0000_0000|doc.namespace;
                let key=ParagraphKey {namespace,slot:line.id as u32,generation:1};
                let style=Style {chain:cx.services.renderer.faces.mono[0],wrap_em:None,align:Align::Left,line_spacing:1.2};
                let key_id=0xe000_0000_0000_0000|(doc.namespace<<32)|line.id;
                if let Some(block)=cx.services.renderer.text.shape(BlockKey(key_id),&style,&[key],&Source(&text)) {
                    let size=14.*s;
                    widest=widest.max(cx.services.renderer.text.measure(block).width_em()*size+code.gutter+24.*s);
                    layer.draws.push(Draw {block,at:Vec2::new(viewport.x+code.gutter+8.*s-code.horizontal.value,y+4.*s),size,color:color(0xd8dee9),clip:Some(Rect::new(viewport.x+code.gutter,viewport.y,viewport.width-code.gutter,viewport.height)),paint:cached.1});
                }
            }
            code.horizontal.max=(widest-viewport.width).max(code.horizontal.value);
            code.paints.retain(|id,(_,paint)|{let keep=visible.contains(id);if !keep && let Some(p)=paint.take(){cx.services.renderer.text.drop_paint(p);}keep});
        } else {
            code.scroll.max=(code.entries.len() as f32*44.*s-viewport.height).max(0.);code.scroll.value=code.scroll.value.clamp(0.,code.scroll.max);
            let first=(code.scroll.value/(44.*s)) as usize;let end=(first+(viewport.height/(44.*s)).ceil() as usize+1).min(code.entries.len());
            for i in first..end {
                let entry=&code.entries[i];let y=viewport.y+i as f32*44.*s-code.scroll.value;
                let rect=Rect::new(viewport.x+8.*s,y,viewport.width-20.*s,44.*s);let hit=crate::render::intersect(rect,viewport);
                if i==code.row {layer.clipped_rect(rect,color(0x172330),viewport);}
                let name=if code.search.is_some(){entry.path.strip_prefix(code.path.as_deref().unwrap_or("")).unwrap_or(&entry.path).trim_start_matches('/')}else{&entry.name};
                let label=format!("{} {}{}",if entry.directory{"▸"}else{"·"},display_path(name),if entry.directory{"/"}else{""});
                cx.services.renderer.clipped_label(layer,&label,Rect::new(rect.x+10.*s,y+11.*s,rect.width-20.*s,24.*s),14.*s,color(if entry.directory{0x8bd6ff}else{0xd8dee9}),false,viewport);
                if hit.height>0. {self.controls.place(Choice::FileOpen(entry.path.clone(),entry.directory),rect,viewport,false);}
            }
            if code.entries.is_empty() {
                let text=code.error.as_deref().unwrap_or(if code.loading{"Loading remote files…"}else if code.search.is_some(){"No matching paths"}else{"Empty directory"});
                cx.services.renderer.label(layer,text,Rect::new(viewport.x+24.*s,viewport.y+32.*s,viewport.width-48.*s,100.*s),14.*s,color(0x82909f),false);
            }
        }
        if paging {
            let y=bottom-44.*s;
            if code.pages.len()>1 {retained_button(&mut cx.services.renderer,layer,&mut self.controls,Rect::new(b.x+12.*s,y,88.*s,40.*s),"Previous",Choice::FilePage(false),s,false);}
            if code.next.is_some() {retained_button(&mut cx.services.renderer,layer,&mut self.controls,Rect::new(b.x+b.width-100.*s,y,88.*s,40.*s),"Next",Choice::FilePage(true),s,false);}
        }
        code.scroll.rect=viewport;code.horizontal.rect=viewport;self.surface.rect=Some(viewport);self.surface.clip=frame.clip;
        code.scroll.paint(layer,cx);self.view=Some(code);self.controls.finish(cx);
        }
}

#[cfg(all(test, not(target_os="android")))]
mod tests;
