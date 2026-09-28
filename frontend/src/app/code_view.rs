//! Directory/code surface, shared by desktop and Android. The renderer owns only
//! visible line paint; the worker owns parsing, and selections use source line IDs.
use super::*;
use std::{borrow::Cow, sync::Arc};
use tau_code_viewer::{Document, Selection};
use tau_protocol::files::*;
use sanscale::{Align, BlockKey, Draw, PaintHandle, PaintSpan, ParagraphKey, ParagraphSource, Style};

pub(super) const SEARCH_FIELD: usize = usize::MAX;
pub(super) struct View {
    identity: String, lineage: Option<String>, session: String,
    generation: u64, subscribed: bool, seen: Option<Arc<crate::file_client::Update>>,
    pub(super) path: Option<String>, directory: Option<String>, parent: Option<String>, search_root: Option<String>,
    operation: FileOperation, pub(super) search: Option<Editor>,
    entries: Vec<FileEntry>, next: Option<String>, pages: Vec<Option<String>>, row: usize,
    pub(super) document: Option<Arc<Document>>, pub(super) selection: Option<Selection>,
    /// The exact generated reference and its byte position. Never global replace
    /// user prose or other references when a live file moves selected lines.
    pub(super) reference: Option<(usize, String)>,
    reference_sync: Option<bool>,
    pub(super) drag_anchor: Option<u64>,
    pub(super) scroll: f32, pub(super) max_scroll: f32, horizontal: f32, max_horizontal: f32,
    viewport: Rect, line_height: f32, gutter: f32,
    cursor: usize, loading: bool, pub(super) error: Option<String>, status: String,
    paints: HashMap<u64, (Vec<PaintSpan>, Option<PaintHandle>)>,
}
impl View {
    fn new(app: &App, session: String) -> Self {
        Self { identity: app.controller.identity.clone(), lineage: app.controller.account.source_lineage.clone(), session,
            generation: 0, subscribed: false, seen: None, path: None, directory: None, parent: None, search_root: None,
            operation: FileOperation::List { after: None }, search: None,
            entries: vec![], next: None, pages: vec![None], row: 0, document: None, selection: None, reference: None, reference_sync: None, drag_anchor: None,
            scroll: 0., max_scroll: 0., horizontal: 0., max_horizontal: 0., viewport: Rect::new(0.,0.,0.,0.), line_height: 24., gutter: 52.,
            cursor: 0, loading: true, error: None, status: String::new(), paints: HashMap::new() }
    }
    pub(super) fn sent(&mut self) {self.selection=None;self.reference=None;self.reference_sync=None;}
    fn line_at(&self, point: Vec2) -> Option<usize> {
        let doc = self.document.as_ref()?;
        if self.search.is_some() || doc.lines.is_empty() { return None; }
        Some(((point.y-self.viewport.y+self.scroll).max(0.)/self.line_height) as usize).map(|n| n.min(doc.lines.len()-1))
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
impl App {
    pub(super) fn close_code(&mut self) {
        if self.root.legacy.code.is_some() {self.cancel_pointer();}
        if let Some(mut view) = self.root.legacy.code.take() {
            view.clear_paint(&mut self.services.renderer);
            let _ = self.controller.view_files(None);
            self.root.legacy.focus = None;
        }
    }
    fn code_request(&mut self) {
        let Some(code)=&mut self.root.legacy.code else { return; };
        let request=FileRequest { session_id:code.session.clone(), path:code.path.clone(), operation:code.operation.clone() };
        code.seen=None; code.loading=true;
        let previous=matches!(code.operation,FileOperation::Open {..}).then(||code.document.clone()).flatten();
        match self.controller.view_files_document(Some(request),previous) {
            Ok(generation)=>{code.generation=generation;code.subscribed=true;code.error=None;}
            Err(error)=>{code.error=Some(error.to_string());code.loading=false;code.subscribed=false;}
        }
        self.ui.dirty=true;
    }
    pub(super) fn code_action(&mut self, action: Action) -> Result<()> {
        match action {
            Action::Files => {
                if self.root.legacy.code.is_some() { self.close_code(); return Ok(()); }
                let Some(session)=self.controller.account.selected.clone() else { return Ok(()); };
                self.save()?; self.cancel_pointer(); self.root.legacy.focus=None; self.root.legacy.show_attachments=false; self.root.legacy.show_chats=false;
                self.root.legacy.code=Some(View::new(self,session)); self.code_request();
            }
            Action::FileClose => { self.close_code(); }
            Action::FileOpen(path, directory) => {
                self.cancel_pointer();
                let Some(code)=&mut self.root.legacy.code else { return Ok(()); };
                code.clear_paint(&mut self.services.renderer);
                code.path=Some(path); code.operation=if directory {FileOperation::List {after:None}} else {FileOperation::Open {revision:None}};
                code.search=None;code.document=None;code.selection=None;code.reference=None;code.reference_sync=None;code.drag_anchor=None;
                code.entries.clear();code.pages=vec![None];code.next=None;code.scroll=0.;code.horizontal=0.;code.row=0;code.cursor=0;
                self.root.legacy.focus=None;self.code_request();
            }
            Action::FileUp => {
                if let Some(path)=self.root.legacy.code.as_ref().and_then(|c| if c.document.is_some() {c.path.as_deref().and_then(parent)} else {c.parent.clone()}) {
                    self.code_action(Action::FileOpen(path,true))?;
                }
            }
            Action::FileFindHere => {
                if let Some(code)=&mut self.root.legacy.code && let Some(search)=&code.search {
                    code.path=code.directory.clone();code.operation=FileOperation::Search {query:search.value.clone()};
                    code.entries.clear();code.scroll=0.;code.row=0;self.code_request();
                }
            }
            Action::FileFind => {
                self.cancel_pointer();
                if self.root.legacy.code.is_none() { self.code_action(Action::Files)?; }
                let Some(code)=&mut self.root.legacy.code else { return Ok(()); };
                if code.search.is_some() { self.code_back(); return Ok(()); }
                code.search=Some(Editor::new(String::new()));
                code.search.as_mut().unwrap().single_line=true;
                code.path=code.search_root.clone();
                code.operation=FileOperation::Search {query:String::new()};code.entries.clear();code.row=0;code.scroll=0.;
                self.root.legacy.focus=Some(Some(SEARCH_FIELD));self.code_request();
                if self.ui.mobile { self.apply(Action::Focus(Some(SEARCH_FIELD)))?; }
            }
            Action::FileClear => { if let Some(code)=&mut self.root.legacy.code {code.selection=None;code.reference=None;code.reference_sync=None;code.drag_anchor=None;}self.root.legacy.focus=None; }
            Action::FileCopy => {
                if let Some(code)=&self.root.legacy.code && let (Some(doc),Some(selection))=(&code.document,&code.selection)
                    && let Some(text)=doc.selected_text(selection) {self.services.platform.push(PlatformAction::Copy(text));}
            }
            Action::FilePage(next) => {
                let Some(code)=&mut self.root.legacy.code else {return Ok(());};
                if next {if let Some(next)=code.next.clone() {code.pages.push(Some(next));}else{return Ok(());}}
                else if code.pages.len()>1 {code.pages.pop();}else{return Ok(());}
                code.operation=FileOperation::List {after:code.pages.last().cloned().flatten()};code.scroll=0.;code.row=0;code.entries.clear();self.code_request();
            }
            _ => {}
        }
        self.ui.dirty=true;Ok(())
    }
    pub(super) fn code_back(&mut self) {
        self.cancel_pointer();
        if let Some(code)=&mut self.root.legacy.code && code.search.take().is_some() {
            code.path=code.document.as_ref().map(|d|d.path.clone()).or_else(||code.directory.clone());
            code.operation=if code.document.is_some(){FileOperation::Open {revision:None}}else{FileOperation::List {after:None}};
            code.entries.clear();code.pages=vec![None];code.next=None;code.scroll=0.;code.row=0;self.root.legacy.focus=None;self.code_request();
        } else if self.root.legacy.code.as_ref().is_some_and(|c|c.document.is_some()) {
            let result=self.code_action(Action::FileUp);self.report(result);
        } else {self.close_code();}
    }
    pub(super) fn code_query(&mut self) {
        if let Some(code)=&mut self.root.legacy.code && let Some(search)=&code.search {
            code.operation=FileOperation::Search {query:search.value.chars().take(128).collect()};
            code.entries.clear();code.scroll=0.;code.row=0;self.code_request();
        }
    }
    /// Persist once per completed gesture/live revision, not on every pointer
    /// motion. Keep a user's draft and only replace our own still-intact marker.
    pub(super) fn code_reference(&mut self, remove: bool) {
        let Some(code)=&mut self.root.legacy.code else {return;};
        if self.controller.account.selected.as_ref()!=Some(&code.session) || self.controller.identity!=code.identity {return;}
        if !remove && code.error.is_some() {return;}
        let next=if remove {None} else {code.document.as_ref().zip(code.selection.as_ref()).and_then(|(d,s)|s.reference(d)).map(|r| {let fence="`".repeat(r.split(|c|c!='`').map(str::len).max().unwrap_or(0)+1);format!("{fence}{r}{fence}\n")})};
        let Some(chat)=self.controller.chats.get(&code.session) else {return;};
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
        if draft!=self.root.legacy.composer.value || draft!=self.controller.chats[&code.session].local.draft {
            match self.controller.draft(draft.clone()) {
                Ok(())=>self.replace_composer(draft),
                Err(error)=>self.controller.report_error(error),
            }
        }
    }
    pub(super) fn code_tick(&mut self, dt: f32) {
        let Some(code)=&self.root.legacy.code else {return;};
        if code.identity!=self.controller.identity || code.session!=self.controller.account.selected.as_deref().unwrap_or("") || code.lineage!=self.controller.account.source_lineage {
            self.close_code();self.ui.dirty=true;return;
        }
        let active=self.ui.window_focused && self.root.dialog.is_none() && self.root.legacy.viewer.is_none();
        if !active && code.subscribed {
            let _=self.controller.view_files(None);self.root.legacy.code.as_mut().unwrap().subscribed=false;
        } else if active && (!code.subscribed || code.generation!=self.controller.viewer_generation()) && self.controller.epoch.is_some() {self.code_request();}
        let update=self.controller.file_update.clone();
        if let Some(update)=update && self.root.legacy.code.as_ref().is_some_and(|c|update.generation==c.generation && update.session==c.session && update.lineage==c.lineage.as_deref().unwrap_or("") && c.seen.as_ref().is_none_or(|old|!Arc::ptr_eq(old,&update))) {
            let code=self.root.legacy.code.as_mut().unwrap();code.seen=Some(update.clone());code.loading=false;
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
                                let top=code.document.as_ref().and_then(|d|d.lines.get((code.scroll/code.line_height) as usize)).map(|l|l.id);
                                if let Some(top)=top && let Some(line)=doc.position(top) {code.scroll=line as f32*code.line_height+code.scroll%code.line_height;}
                                if let Some(selection)=&code.selection {
                                    if selection.range(doc).is_none() {invalidated=true;code.selection=None;code.drag_anchor=None;}
                                    else {moved=true;}
                                }
                                if code.document.as_ref().is_some_and(|old|old.namespace!=doc.namespace) {code.clear_paint(&mut self.services.renderer);}
                                code.document=Some(doc.clone());code.cursor=code.cursor.min(doc.lines.len()-1);
                            }
                            code.path=Some(path.clone());code.directory=parent(path);
                            code.status=if invalidated {"Selected text changed · reselect to comment (draft kept)"} else {"Read only · live · select line numbers to comment"}.into();
                        }
                    }
                }
            }
            code.row=code.row.min(code.entries.len().saturating_sub(1));
            if invalidated {self.root.legacy.code.as_mut().unwrap().reference_sync=Some(true);}
            else if moved && self.root.legacy.code.as_ref().is_some_and(|c|c.reference.is_some()) {self.root.legacy.code.as_mut().unwrap().reference_sync=Some(false);}
            self.ui.dirty=true;
        }
        if !self.composing() && let Some(remove)=self.root.legacy.code.as_mut().and_then(|c|c.reference_sync.take()) {self.code_reference(remove);self.ui.dirty=true;}
        // Hold in the code body promotes scrolling to line-range selection. A
        // gutter press selects immediately, and dragging back shrinks the range.
        if active && let Some(p)=&self.root.legacy.pointer && p.touch && !p.dragged && p.started.elapsed().as_millis()>=450
            && self.root.legacy.code.as_ref().is_some_and(|c|c.error.is_none() && c.search.is_none() && c.document.is_some() && contains(c.viewport,p.start) && c.drag_anchor.is_none()) {
            let point=p.start;self.code_begin(point);self.services.platform.push(PlatformAction::Haptic);
        }
        if let Some(code)=&mut self.root.legacy.code && code.drag_anchor.is_some() && let Some(p)=&self.root.legacy.pointer {
            let margin=16.*self.ui.scale;
            let delta=if p.last.y<code.viewport.y+margin {p.last.y-code.viewport.y-margin}
                else {(p.last.y-code.viewport.y-code.viewport.height+margin).max(0.)};
            let next=(code.scroll+delta.clamp(-90.*self.ui.scale,90.*self.ui.scale)*12.*dt.min(0.05)).clamp(0.,code.max_scroll);
            if next!=code.scroll {code.scroll=next;let point=p.last;self.code_extend(point);self.ui.dirty=true;}
        }
    }
    fn code_begin(&mut self, point: Vec2) {
        let Some(code)=&mut self.root.legacy.code else{return;};
        let Some(line)=code.line_at(point) else{return;};let doc=code.document.as_ref().unwrap();
        code.cursor=line;code.selection=Some(Selection::new(doc,line,line));code.drag_anchor=Some(doc.lines[line].id);
        self.root.legacy.focus=None;self.ui.dirty=true;
    }
    fn code_extend(&mut self, point: Vec2) {
        let Some(code)=&mut self.root.legacy.code else{return;};
        let Some(end)=code.line_at(point) else{return;};let doc=code.document.as_ref().unwrap();
        let Some(anchor)=code.drag_anchor.and_then(|id|doc.position(id)) else{return;};
        code.selection=Some(Selection::new(doc,anchor,end));code.cursor=end;self.ui.dirty=true;
    }
    pub(super) fn code_press(&mut self, id:u64, point:Vec2, touch:bool)->bool {
        if self.root.dialog.is_some() || self.root.legacy.viewer.is_some() || self.root.legacy.context_menu.is_some() {return false;}
        let Some(code)=&self.root.legacy.code else{return false;};
        if code.error.is_some() || code.document.is_none() || code.search.is_some() || !contains(code.viewport,point) || touch && point.x>code.viewport.x+code.gutter {return false;}
        self.root.legacy.pointer=Some(Pointer {id,start:point,last:point,at:Instant::now(),started:Instant::now(),dragged:false,touch});
        self.code_begin(point);true
    }
    pub(super) fn code_motion(&mut self,id:u64,point:Vec2)->bool {
        if self.root.dialog.is_some() || self.root.legacy.viewer.is_some() || self.root.legacy.context_menu.is_some() || self.root.legacy.scroll_drag.is_some() {return false;}
        let (Some(code),Some(p))=(&mut self.root.legacy.code,&mut self.root.legacy.pointer) else{return false;};
        if p.id!=id || !contains(code.viewport,p.start) {return false;}
        let selecting=code.drag_anchor.is_some();
        p.dragged|=(point.x-p.start.x).abs()+(point.y-p.start.y).abs()>7.*self.ui.scale;
        if !selecting && p.dragged {
            if code.document.is_some() && code.search.is_none() && (point.x-p.start.x).abs()>1.5*(point.y-p.start.y).abs() {
                code.horizontal=(code.horizontal+ p.last.x-point.x).clamp(0.,code.max_horizontal);
            } else {code.scroll=(code.scroll+p.last.y-point.y).clamp(0.,code.max_scroll);}
        }
        p.last=point;p.at=Instant::now();if selecting {self.code_extend(point);}self.ui.dirty=true;true
    }
    pub(super) fn code_release(&mut self,id:u64,point:Vec2)->bool {
        if self.root.legacy.code.as_ref().is_none_or(|c|c.drag_anchor.is_none()) || self.root.legacy.pointer.as_ref().is_none_or(|p|p.id!=id) {return false;}
        self.code_extend(point);self.root.legacy.code.as_mut().unwrap().drag_anchor=None;self.root.legacy.pointer=None;
        self.code_reference(false);
        if !self.ui.mobile {self.root.legacy.focus=Some(None);}self.ui.dirty=true;true
    }
    #[cfg(not(target_os="android"))]
    pub(super) fn code_wheel(&mut self,amount:f32,horizontal:bool,point:Vec2)->bool {
        let Some(code)=&mut self.root.legacy.code else{return false;};if !contains(code.viewport,point){return false;}
        if horizontal {code.horizontal=(code.horizontal+amount).clamp(0.,code.max_horizontal);}
        else {code.scroll=(code.scroll+amount).clamp(0.,code.max_scroll);}
        self.ui.dirty=true;true
    }
    pub(super) fn code_key(&mut self,key:&str,ctrl:bool,shift:bool)->bool {
        if ctrl && matches!(key,"Space"|" ") {self.activate(Action::FileFind);return true;}
        let Some(code)=&mut self.root.legacy.code else{return false;};
        if key=="Escape" {
            if code.search.is_some() {self.code_back();}
            else if code.selection.is_some() {self.activate(Action::FileClear);}
            else {self.code_back();}self.ui.dirty=true;return true;
        }
        if self.root.legacy.focus==Some(None) {return false;}
        if ctrl && key.eq_ignore_ascii_case("c") && code.search.is_none() {self.activate(Action::FileCopy);return true;}
        if code.document.is_none() || code.search.is_some() {
            match key {
                "ArrowUp"|"k" if code.search.is_none() || key=="ArrowUp"=>code.row=code.row.saturating_sub(1),
                "ArrowDown"|"j" if code.search.is_none() || key=="ArrowDown"=>code.row=(code.row+1).min(code.entries.len().saturating_sub(1)),
                "Enter"|"l" if code.search.is_none() || key=="Enter"=>{if let Some(entry)=code.entries.get(code.row) {let action=Action::FileOpen(entry.path.clone(),entry.directory);self.activate(action);}return true;}
                "Backspace"|"h" if code.search.is_none()=>{self.activate(Action::FileUp);return true;}
                _=>return false,
            }
            let top=code.row as f32*44.*self.ui.scale;
            if top<code.scroll {code.scroll=top;}else if top+44.*self.ui.scale>code.scroll+code.viewport.height {code.scroll=(top+44.*self.ui.scale-code.viewport.height).min(code.max_scroll);}
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
                "ArrowLeft"=>{code.horizontal=(code.horizontal-48.*self.ui.scale).max(0.);self.ui.dirty=true;return true;}
                "ArrowRight"=>{code.horizontal=(code.horizontal+48.*self.ui.scale).min(code.max_horizontal);self.ui.dirty=true;return true;}
                "Backspace"|"h"=>{self.activate(Action::FileUp);return true;}
                "v"=>{code.selection=Some(Selection::new(doc,old,old));self.code_reference(false);self.ui.dirty=true;return true;}
                _=>return false,
            }
            if shift {let anchor=code.selection.as_ref().and_then(|s|doc.position(s.anchor)).unwrap_or(old);code.selection=Some(Selection::new(doc,anchor,code.cursor));}
            let top=code.cursor as f32*code.line_height;
            if top<code.scroll {code.scroll=top;}else if top+code.line_height>code.scroll+code.viewport.height {code.scroll=(top+code.line_height-code.viewport.height).min(code.max_scroll);}
            if shift {self.code_reference(false);}
        }
        self.ui.dirty=true;true
    }
    pub(super) fn code_frame(&mut self,ctx:&impl RenderContext,body:&mut Layer,chrome:&mut Layer,b:Rect) {
        let s=self.ui.scale;let mut code=self.root.legacy.code.take().unwrap();
        let comments=code.search.is_none() && code.document.is_some() && (code.selection.is_some() || self.root.legacy.focus==Some(None));
        let bottom=if comments {self.composer_layout(b,&code.session).4} else {b.y+b.height};
        chrome.rect(Rect::new(b.x,b.y,b.width,124.*s),color(0x0e141b));
        button(&mut self.services.renderer,chrome,&mut self.root.legacy.hits,Rect::new(b.x+8.*s,b.y+6.*s,64.*s,40.*s),"‹ Chat",Action::FileClose,s,false);
        if code.search.is_some() {
            button(&mut self.services.renderer,chrome,&mut self.root.legacy.hits,Rect::new(b.x+76.*s,b.y+6.*s,44.*s,40.*s),"Here",Action::FileFindHere,s,false);
        } else if code.parent.is_some() || code.document.is_some() {
            button(&mut self.services.renderer,chrome,&mut self.root.legacy.hits,Rect::new(b.x+76.*s,b.y+6.*s,44.*s,40.*s),"Up",Action::FileUp,s,false);
        }
        self.services.renderer.label(chrome,"Files",Rect::new(b.x+130.*s,b.y+15.*s,(b.width-240.*s).max(1.),24.*s),16.*s,color(0xe5eaf0),true);
        button(&mut self.services.renderer,chrome,&mut self.root.legacy.hits,Rect::new(b.x+b.width-104.*s,b.y+6.*s,60.*s,40.*s),if code.search.is_some(){"Done"}else{"Find"},Action::FileFind,s,code.search.is_some());
        self.icon_button(ctx,chrome,Rect::new(b.x+b.width-44.*s,b.y+6.*s,40.*s,40.*s),Icon::Close,18.,Action::FileClose,false,true);
        self.services.renderer.label(chrome,&display_path(code.path.as_deref().unwrap_or("Chat working directory")),Rect::new(b.x+14.*s,b.y+54.*s,b.width-28.*s,28.*s),13.*s,color(0xb7c2ce),false);
        let top=if let Some(search)=&mut code.search {
            let rect=Rect::new(b.x+12.*s,b.y+88.*s,b.width-24.*s,48.*s);
            search.draw(&mut self.services.renderer,chrome,rect,16.*s,self.root.legacy.focus==Some(Some(SEARCH_FIELD)),false,"Fuzzy find paths…",true);
            self.root.legacy.hits.push(Hit {rect,action:Action::Focus(Some(SEARCH_FIELD))});
            self.services.renderer.label(chrome,code.error.as_deref().unwrap_or(if code.loading {"Searching…"} else {&code.status}),Rect::new(b.x+14.*s,b.y+139.*s,b.width-28.*s,18.*s),11.*s,color(0x82909f),false);
            b.y+160.*s
        } else {
            let status=code.error.as_deref().unwrap_or(if code.loading {"Loading…"} else {&code.status});
            let label=code.error.is_none().then(||code.selection.as_ref().zip(code.document.as_ref()).and_then(|(selection,doc)|selection.range(doc)).map(|r|format!("Lines {}–{}",r.start+1,r.end))).flatten();
            self.services.renderer.label(chrome,label.as_deref().unwrap_or(status),Rect::new(b.x+14.*s,b.y+94.*s,(b.width-if comments{158.*s}else{28.*s}).max(1.),24.*s),12.*s,color(if code.error.is_some(){0xf2a6a6}else{0x82909f}),false);
            if comments {
                button(&mut self.services.renderer,chrome,&mut self.root.legacy.hits,Rect::new(b.x+b.width-140.*s,b.y+84.*s,62.*s,40.*s),"Copy",Action::FileCopy,s,false);
                button(&mut self.services.renderer,chrome,&mut self.root.legacy.hits,Rect::new(b.x+b.width-74.*s,b.y+84.*s,62.*s,40.*s),"Clear",Action::FileClear,s,false);
            }
            b.y+124.*s
        };
        let paging=code.search.is_none() && code.document.is_none() && (code.next.is_some() || code.pages.len()>1);
        code.viewport=Rect::new(b.x,top,b.width,(bottom-top-if paging{48.*s}else{0.}).max(1.));
        let viewport=code.viewport;
        body.rect(viewport,color(0x0b1118));chrome.rect(Rect::new(b.x,top-s,b.width,s),color(0x2a3541));
        if let Some(doc)=&code.document && code.search.is_none() {
            code.line_height=if self.ui.mobile {28.}else{24.}*s;
            code.gutter=(44.+doc.lines.len().to_string().len().saturating_sub(3) as f32*8.)*s;
            code.max_scroll=(doc.lines.len() as f32*code.line_height+16.*s-viewport.height).max(0.);
            code.scroll=code.scroll.clamp(0.,code.max_scroll);
            let first=(code.scroll/code.line_height).floor() as usize;
            let end=(first+(viewport.height/code.line_height).ceil() as usize+2).min(doc.lines.len());
            let selection=code.selection.as_ref().and_then(|s|s.range(doc));
            let mut visible=HashSet::new();let mut widest=viewport.width;
            body.rect(Rect::new(viewport.x,viewport.y,code.gutter,viewport.height),color(0x111923));
            for i in first..end {
                let line=&doc.lines[i];visible.insert(line.id);
                let y=viewport.y+i as f32*code.line_height-code.scroll;
                if selection.as_ref().is_some_and(|r|r.contains(&i)) {body.clipped_rect(Rect::new(viewport.x,y,viewport.width,code.line_height),color(0x213c57),viewport);}
                self.services.renderer.clipped_label(body,&(i+1).to_string(),Rect::new(viewport.x+8.*s,y+5.*s,code.gutter-14.*s,code.line_height),12.*s,color(if selection.as_ref().is_some_and(|r|r.contains(&i)){0x8bd6ff}else{0x6c7d90}),false,viewport);
                // Cap a single pathological/minified line's shaping work while
                // retaining its original full text for selection and copying.
                let mut stop=line.text.len().min(16*1024);while !line.text.is_char_boundary(stop){stop-=1;}
                let mut text=line.text[..stop].replace('\t',"    ");
                if stop<line.text.len() {text.push_str(" … [long line preview limited; Copy keeps full text]");}
                let tabs=line.text[..stop].contains('\t');
                let byte=|n:usize|{let n=n.min(stop);n+if tabs {line.text[..n].bytes().filter(|&b|b==b'\t').count()*3} else {0}};
                let spans=line.paint.iter().filter(|p|p.range.start<stop).map(|p|PaintSpan {range:byte(p.range.start)..byte(p.range.end),color:color(p.color)}).collect::<Vec<_>>();
                let cached=code.paints.entry(line.id).or_insert_with(||(vec![],None));
                if cached.0!=spans {if let Some(p)=cached.1.take(){self.services.renderer.text.drop_paint(p);}cached.1=if spans.is_empty(){None}else{self.services.renderer.text.register_paint(&spans).ok()};cached.0=spans;}
                let namespace=0x5441_5546_0000_0000|doc.namespace;
                let key=ParagraphKey {namespace,slot:line.id as u32,generation:1};
                let style=Style {chain:self.services.renderer.faces.mono[0],wrap_em:None,align:Align::Left,line_spacing:1.2};
                let key_id=0xe000_0000_0000_0000|(doc.namespace<<32)|line.id;
                if let Some(block)=self.services.renderer.text.shape(BlockKey(key_id),&style,&[key],&Source(&text)) {
                    let size=14.*s;
                    widest=widest.max(self.services.renderer.text.measure(block).width_em()*size+code.gutter+24.*s);
                    body.draws.push(Draw {block,at:Vec2::new(viewport.x+code.gutter+8.*s-code.horizontal,y+4.*s),size,color:color(0xd8dee9),clip:Some(Rect::new(viewport.x+code.gutter,viewport.y,viewport.width-code.gutter,viewport.height)),paint:cached.1});
                }
            }
            code.max_horizontal=(widest-viewport.width).max(code.horizontal);
            code.paints.retain(|id,(_,paint)|{let keep=visible.contains(id);if !keep && let Some(p)=paint.take(){self.services.renderer.text.drop_paint(p);}keep});
        } else {
            code.max_scroll=(code.entries.len() as f32*44.*s-viewport.height).max(0.);code.scroll=code.scroll.clamp(0.,code.max_scroll);
            let first=(code.scroll/(44.*s)) as usize;let end=(first+(viewport.height/(44.*s)).ceil() as usize+1).min(code.entries.len());
            for i in first..end {
                let entry=&code.entries[i];let y=viewport.y+i as f32*44.*s-code.scroll;
                let rect=Rect::new(viewport.x+8.*s,y,viewport.width-20.*s,44.*s);let hit=crate::render::intersect(rect,viewport);
                if i==code.row {body.clipped_rect(rect,color(0x172330),viewport);}
                let name=if code.search.is_some(){entry.path.strip_prefix(code.path.as_deref().unwrap_or("")).unwrap_or(&entry.path).trim_start_matches('/')}else{&entry.name};
                let label=format!("{} {}{}",if entry.directory{"▸"}else{"·"},display_path(name),if entry.directory{"/"}else{""});
                self.services.renderer.clipped_label(body,&label,Rect::new(rect.x+10.*s,y+11.*s,rect.width-20.*s,24.*s),14.*s,color(if entry.directory{0x8bd6ff}else{0xd8dee9}),false,viewport);
                if hit.height>0. {self.root.legacy.hits.push(Hit {rect:hit,action:Action::FileOpen(entry.path.clone(),entry.directory)});}
            }
            if code.entries.is_empty() {
                let text=code.error.as_deref().unwrap_or(if code.loading{"Loading remote files…"}else if code.search.is_some(){"No matching paths"}else{"Empty directory"});
                self.services.renderer.label(body,text,Rect::new(viewport.x+24.*s,viewport.y+32.*s,viewport.width-48.*s,100.*s),14.*s,color(0x82909f),false);
            }
        }
        if paging {
            let y=bottom-44.*s;
            if code.pages.len()>1 {button(&mut self.services.renderer,chrome,&mut self.root.legacy.hits,Rect::new(b.x+12.*s,y,88.*s,40.*s),"Previous",Action::FilePage(false),s,false);}
            if code.next.is_some() {button(&mut self.services.renderer,chrome,&mut self.root.legacy.hits,Rect::new(b.x+b.width-100.*s,y,88.*s,40.*s),"Next",Action::FilePage(true),s,false);}
        }
        let session=code.session.clone();self.root.legacy.code=Some(code);self.scrollbar(chrome,Lane::Files,viewport);
        if comments {self.draw_composer(ctx,chrome,b,&session,true);}
    }
}

#[cfg(all(test, not(target_os="android")))]
mod tests;
