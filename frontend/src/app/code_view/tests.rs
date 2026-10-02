use super::*;
use chad::{Config, HeadlessCtx};
struct Harness {app:App,ctx:HeadlessCtx,_root:tempfile::TempDir}
impl Harness {
    fn new(size:(u32,u32),scale:f32,mobile:bool)->Self {
        let root=tempfile::tempdir().unwrap();
        let ctx=HeadlessCtx::new(&Config {size,device_limits:crate::desktop::limits(),..Default::default()}).unwrap();
        let mut app=App::new(&ctx,Store::open(root.path().into()).unwrap(),Arc::new(||{}),mobile).unwrap();
        app.back();crate::demo::populate(&mut app.controller).unwrap();app.resize(size,scale,Vec2::new(0.,0.));app.tick(0.);app.root.workspace.show_chats=false;app.frame(&ctx,ctx.view());
        Self {app,ctx,_root:root}
    }
    fn frame(&mut self) {self.app.tick(0.);self.app.frame(&self.ctx,self.ctx.view());}
    fn files(&mut self) {
        let r=self.app.root.workspace.chat.header.controls.placed().find(|(a,_)|matches!(a,ui::header::Choice::Files)).unwrap().1;
        self.click_rect(r);
    }
    fn click(&mut self,predicate:impl Fn(&Choice)->bool) {
        let r=self.app.root.workspace.chat.code.controls.placed().find(|(a,_)|predicate(a)).unwrap().1;
        self.click_rect(r);
    }
    fn click_rect(&mut self,r:Rect) {
        let p=Vec2::new(r.x+r.width/2.,r.y+r.height/2.);self.app.press(1,p,self.app.ui.mobile);self.app.release(1,p);self.frame();
    }
    fn update(&mut self,reply:FileReply,document:Option<Arc<Document>>) {
        let view=self.app.root.workspace.chat.code.view.as_ref().unwrap();
        self.app.controller.file_update=Some(Arc::new(crate::file_client::Update {generation:view.generation,session:view.session.clone(),lineage:view.lineage.clone().unwrap_or_default(),response:Ok(reply),document}));self.frame();
    }
    fn text(&mut self,text:&str) {
        let old=self.app.root.workspace.chat.code.view.as_ref().unwrap().document.clone();
        let doc=Arc::new(Document::replace(old.as_deref(),"/workspace/src/main.rs".into(),blake3::hash(text.as_bytes()).to_hex().to_string(),text.into()));
        self.update(FileReply::Text {path:doc.path.clone(),revision:doc.revision.clone(),text:String::new()},Some(doc));
    }
    fn index(&mut self, paths: &[&str], limited: bool) {
        let reply = FileReply::Index { path: "/workspace".into(), revision: "a".repeat(64), base: None,
            entries: paths.iter().map(|p| IndexedPath { path: (*p).into(), symlink: false }).collect(), removed: vec![], indexing: false, limited };
        let index = Arc::new(tau_code_viewer::finder::PathIndex::apply(None, &reply).unwrap());
        self.app.controller.file_index = Some(Arc::new(crate::file_index::Update { generation:0,
            session: self.app.controller.account.selected.clone().unwrap(), lineage: self.app.controller.account.source_lineage.clone().unwrap_or_default(),
            index: Some(index), indexing:false,limited,error:None }));
        self.frame();
    }
    fn matched(&mut self) {
        let end = Instant::now() + std::time::Duration::from_secs(5);
        loop {
            self.frame();
            let code = self.app.root.workspace.chat.code.view.as_ref().unwrap();
            if code.matches.as_ref().is_some_and(|m| m.generation == code.match_generation) { return; }
            assert!(Instant::now() < end, "local matching stalled");
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
    fn preview(&mut self, text: &str) {
        let view = self.app.root.workspace.chat.code.view.as_ref().unwrap();
        let doc = Arc::new(Document::replace(view.preview.as_deref(), view.preview_target.clone().unwrap(), blake3::hash(text.as_bytes()).to_hex().to_string(), text.into()));
        self.update(FileReply::Text {path:doc.path.clone(), revision:doc.revision.clone(),text:String::new()}, Some(doc));
    }
    fn line(&self,n:usize,gutter:bool)->Vec2 {
        let c=self.app.root.workspace.chat.code.view.as_ref().unwrap();Vec2::new(c.viewport.x+if gutter{20.}else{120.}*self.app.ui.scale,c.viewport.y+n as f32*c.line_height+0.5*c.line_height-c.scroll.value)
    }
    fn dump(&self,name:&str) {
        if let Some(root)=std::env::var_os("TAU_CODE_PREVIEW_DIR") {std::fs::create_dir_all(&root).unwrap();let size=self.ctx.size();image::save_buffer(PathBuf::from(root).join(name),&self.ctx.read_rgba8().unwrap(),size.0,size.1,image::ColorType::Rgba8).unwrap();}
    }
}
fn source()->String {format!("// Remote code, café 🦀\nfn main() {{\n    let answer = 42;\n    println!(\"{{answer}}\");\n}}\n\n{}",(0..100).map(|n|format!("// stable paragraph {n}\n")).collect::<String>())}
#[test]
fn directory_code_gutter_selection_and_current_composer_work_at_desktop_and_phone_sizes() {
    for (name,size,scale,mobile) in [("desktop",(1100,800),1.,false),("phone",(360,720),1.,true),("phone-2x",(900,1800),2.5,true)] {
        let mut h=Harness::new(size,scale,mobile);
        h.app.controller.draft("Existing draft".into()).unwrap();h.frame();h.files();
        h.update(FileReply::Directory {path:"/workspace".into(),parent:Some("/".into()),entries:vec![FileEntry {path:"/workspace/src".into(),name:"src".into(),directory:true,symlink:false},FileEntry {path:"/workspace/README.md".into(),name:"README.md".into(),directory:false,symlink:false}],next:None},None);
        assert!(h.app.root.workspace.chat.composer.field.control.rect.is_none());h.dump(&format!("{name}-directory.png"));
        h.app.with_ui(|root, cx| root.workspace.chat.code.code_action(Choice::FileOpen("/workspace/src/main.rs".into(),false), cx)).unwrap();h.text(&source());h.dump(&format!("{name}-code.png"));
        assert!(h.app.root.workspace.chat.composer.field.control.rect.is_none());
        let start=h.line(1,true);let end=h.line(3,true);
        h.app.press(9,start,mobile);h.app.motion(9,end);h.app.release(9,end);h.frame();
        assert!(h.app.root.workspace.chat.composer.field.control.rect.is_some());
        assert_eq!(h.app.controller.selected().unwrap().local.draft,"Existing draft\n\n`/workspace/src/main.rs:2-4`\n");
        assert!(h.app.controller.selected().unwrap().local.pending.is_empty(),"Selection must not send");
        h.dump(&format!("{name}-comment.png"));
        h.click(|a|matches!(a,Choice::FileCopy));
        assert!(h.app.actions().iter().any(|a|matches!(a,PlatformAction::Copy(t) if t=="fn main() {\n    let answer = 42;\n    println!(\"{answer}\");")));
        h.app.ui.focus=Some(h.app.root.workspace.chat.composer.field.control.target);h.app.input("Why 42?");h.frame();
        let old_editor = h.app.root.workspace.chat.composer.field.editor.native_id();
        h.text(&format!("// inserted above\n{}",source()));
        assert_eq!(h.app.controller.selected().unwrap().local.draft,"Existing draft\n\n`/workspace/src/main.rs:3-5`\nWhy 42?");
        assert_ne!(old_editor, h.app.root.workspace.chat.composer.field.editor.native_id(), "Live reference updates fence old native snapshots");
        h.text(&format!("// inserted above\n{}",source().replace("let answer = 42;","let answer = 43;")));
        assert!(h.app.root.workspace.chat.code.view.as_ref().unwrap().selection.is_none());
        assert!(!h.app.controller.selected().unwrap().local.draft.contains("main.rs:"));
        assert!(h.app.controller.selected().unwrap().local.draft.ends_with("Why 42?"));
        assert!(h.app.root.workspace.chat.composer.field.control.rect.is_some(), "An active comment edit is not hidden/lost by a live replacement");
        assert!(h.app.with_ui(|root, cx| root.workspace.send(cx)).is_err());
        h.click(|a|matches!(a,Choice::FileClear));h.frame();
        assert!(h.app.root.workspace.chat.composer.field.control.rect.is_none());
        h.click(|a|matches!(a,Choice::FileClose));assert!(h.app.root.workspace.chat.code.view.is_none());
        assert!(h.app.root.workspace.chat.composer.field.control.rect.is_some());
    }
}
#[test]
fn touch_hold_haptics_drag_undrag_scroll_and_chat_switch_are_scoped() {
    let mut h=Harness::new((360,720),1.,true);h.files();h.text(&source());
    let start=h.line(1,false);h.app.press(4,start,true);
    h.app.root.workspace.chat.code.pointer.as_mut().unwrap().started=Instant::now()-std::time::Duration::from_millis(500);h.frame();
    assert!(h.app.actions().iter().any(|a|matches!(a,PlatformAction::Haptic)));
    let end=h.line(4,false);h.app.motion(4,end);h.frame();
    let doc=h.app.root.workspace.chat.code.view.as_ref().unwrap().document.as_ref().unwrap();assert_eq!(h.app.root.workspace.chat.code.view.as_ref().unwrap().selection.as_ref().unwrap().range(doc),Some(1..5));
    let end=h.line(2,false);h.app.motion(4,end);h.app.release(4,end);h.frame();
    assert!(h.app.controller.selected().unwrap().local.draft.contains("main.rs:2-3"));
    h.click(|a|matches!(a,Choice::FileClear));
    let start=h.line(5,false);h.app.press(6,start,true);h.app.motion(6,Vec2::new(start.x,start.y-90.));h.app.release(6,Vec2::new(start.x,start.y-90.));h.frame();
    assert!(h.app.root.workspace.chat.code.view.as_ref().unwrap().scroll.value>0.);assert!(h.app.root.workspace.chat.code.view.as_ref().unwrap().selection.is_none());
    let old_draft=h.app.controller.selected().unwrap().local.draft.clone();
    h.app.controller.select("two").unwrap();h.frame();assert!(h.app.root.workspace.chat.code.view.is_none());
    assert_eq!(h.app.controller.chats["demo"].local.draft,old_draft);
    assert!(!h.app.controller.selected().unwrap().local.draft.contains("main.rs:"));
}
#[test]
fn telescope_reuses_real_editor_and_cancels_without_losing_code_selection() {
    let mut h=Harness::new((1000,800),1.,false);h.files();h.text(&source());
    let start=h.line(1,true);h.app.press(1,start,false);h.app.release(1,start);h.frame();
    let before=h.app.controller.selected().unwrap().local.draft.clone();
    h.app.key("Space",true,false);h.frame();assert!(h.app.root.workspace.chat.code.view.as_ref().unwrap().search.is_some());
    h.app.input("srcmain");h.frame();assert_eq!(h.app.root.workspace.chat.code.view.as_ref().unwrap().search.as_ref().unwrap().editor.value,"srcmain");
    assert_eq!(h.app.controller.selected().unwrap().local.draft,before,"Search text is not a chat draft");
    h.index(&["src/main.rs"], false);h.matched();h.preview(&source());
    h.dump("desktop-search.png");h.app.key("Escape",false,false);h.frame();h.text(&source());
    assert!(h.app.root.workspace.chat.code.view.as_ref().unwrap().selection.is_some());assert_eq!(h.app.controller.selected().unwrap().local.draft,before);
    // Rendering the same actual code frame does not reshape source paragraphs.
    sanscale::profiling::reset_work_counters();h.frame();
    assert_eq!(sanscale::profiling::work_counters().source_reads,0);
    h.app.controller.file_update=Some(Arc::new(crate::file_client::Update {generation:99_999,session:"other".into(),lineage:"other".into(),response:Err("old response".into()),document:None}));h.frame();
    assert!(h.app.root.workspace.chat.code.view.as_ref().unwrap().error.is_none());
}

#[test]
fn live_references_preserve_ime_composition_and_newer_saved_draft_text() {
    let mut h=Harness::new((1000,800),1.,false);h.files();h.text(&source());
    let start=h.line(1,true);h.app.press(1,start,false);h.app.release(1,start);h.frame();
    h.app.preedit("入力".into(),None);assert!(h.app.composing());
    h.text(&format!("// inserted\n{}",source()));
    assert!(h.app.composing(),"A live line-number update must not cancel the IME");
    assert!(h.app.root.workspace.chat.composer.field.editor.value.contains("main.rs:2-2"));
    h.app.input("入力");h.frame();
    assert!(h.app.root.workspace.chat.composer.field.editor.value.contains("main.rs:3-3"));assert!(h.app.root.workspace.chat.composer.field.editor.value.ends_with("入力"));
    let draft=format!("Newer saved prefix\n{}",h.app.controller.selected().unwrap().local.draft);
    h.app.controller.draft(draft).unwrap();
    h.text(&format!("// inserted twice\n// inserted\n{}",source()));
    assert!(h.app.root.workspace.chat.composer.field.editor.value.starts_with("Newer saved prefix\n"));assert!(h.app.root.workspace.chat.composer.field.editor.value.contains("main.rs:4-4"));
    h.app.preedit(" still typing".into(),None);
    h.text(&format!("// inserted twice\n// inserted\n{}",source().replace("fn main()", "fn renamed()")));
    assert!(h.app.composing());h.app.input(" still typing");h.frame();
    assert!(h.app.root.workspace.chat.composer.field.editor.value.ends_with("入力 still typing"));assert!(!h.app.root.workspace.chat.composer.field.editor.value.contains("main.rs:"));
    assert!(h.app.with_ui(|root, cx| root.workspace.send(cx)).is_err());
}

#[test]
fn retained_modal_pauses_file_interest_before_the_next_frame_without_destroying_the_view() {
    let mut h = Harness::new((1000, 800), 1., false);
    h.app.with_ui(|root, cx| root.workspace.files(cx)).unwrap();
    // Model a previously submitted interest. The native worker is tested by the
    // remote-files integration tests; this checks the UI's ownership boundary.
    h.app.root.workspace.chat.code.view.as_mut().unwrap().subscribed = true;
    let generation = h.app.controller.viewer_generation();
    h.app.open_ui(ui::DialogSpec::Connection).unwrap();
    assert!(!h.app.root.workspace.chat.code.view.as_ref().unwrap().subscribed);
    assert!(h.app.controller.viewer_generation() > generation, "Opening a modal cancels the old interest before another event/frame");
    h.app.back();
    assert!(h.app.root.workspace.chat.code.view.is_some(), "Closing the dialog reveals the same code viewport");
    assert!(!h.app.root.workspace.chat.code.view.as_ref().unwrap().subscribed, "Offline does not invent a replacement connection");
}

#[test]
fn local_picker_matches_all_files_previews_highlights_and_never_edits_the_draft() {
    for (name,size,scale,mobile) in [("desktop",(1100,800),1.,false),("phone",(360,720),1.,true),("phone-2x",(900,1800),2.5,true)] {
        let mut h=Harness::new(size,scale,mobile);
        h.app.controller.draft("Keep my draft".into()).unwrap();
        // Names arrive while still in chat, ahead of opening Find.
        let files=(0..240).map(|i|format!("frontend/src/app/file_{i:03}.rs")).chain([".private/file.rs".into(),"src/.nested/file.rs".into(),"src/.file.rs".into()]).collect::<Vec<_>>();
        h.index(&files.iter().map(String::as_str).collect::<Vec<_>>(),false);
        let warmed=h.app.controller.file_index.clone().unwrap();
        h.app.key("Space",true,false);h.matched();
        assert_eq!(h.app.root.workspace.chat.code.view.as_ref().unwrap().len(),240,"no top-100 cutoff");
        h.app.input("flrs fnt");h.matched();
        let code=h.app.root.workspace.chat.code.view.as_ref().unwrap();
        assert_eq!(code.len(),240,"whitespace terms match independently, in either order");
        assert!(Arc::ptr_eq(h.app.controller.file_index.as_ref().unwrap(),&warmed),"typing doesn't replace or request an index");
        assert!(code.search_status().starts_with("240 / 240 files"));
        assert_eq!(h.app.controller.selected().unwrap().local.draft,"Keep my draft");
        h.preview(&source());
        let code=h.app.root.workspace.chat.code.view.as_ref().unwrap();
        assert!(code.document.is_none(),"preview is separate from the opened buffer");
        assert!(code.preview.as_ref().unwrap().lines.iter().any(|l|!l.paint.is_empty()),"syntax paint");
        assert!(code.paints.values().any(|(spans,_)|!spans.is_empty()),"matched characters are highlighted");
        assert!(code.preview_viewport.y>=code.viewport.y+code.viewport.height);
        h.dump(&format!("{name}-fzf.png"));
        h.app.key("n",true,false);h.frame();assert_eq!(h.app.root.workspace.chat.code.view.as_ref().unwrap().row,1);
        assert!(h.app.root.workspace.chat.code.view.as_ref().unwrap().preview.is_none(),"never show old content under a new filename");
        h.preview("fn selected() {}\n");
        h.app.key("p",true,false);h.frame();assert_eq!(h.app.root.workspace.chat.code.view.as_ref().unwrap().preview.as_ref().unwrap().text,source(),"revisited preview is immediate");
        h.app.key("Tab",false,false);h.frame();assert_eq!(h.app.root.workspace.chat.code.view.as_ref().unwrap().row,1);
        h.app.key("Tab",false,true);h.frame();assert_eq!(h.app.root.workspace.chat.code.view.as_ref().unwrap().row,0);
        h.app.key("End",true,false);h.frame();assert_eq!(h.app.root.workspace.chat.code.view.as_ref().unwrap().row,239);
        assert!(h.app.root.workspace.chat.code.view.as_ref().unwrap().scroll.value>0.);
        h.app.key("Home",true,false);h.frame();
        h.click(|a|matches!(a,Choice::FileAccept));
        assert!(h.app.root.workspace.chat.code.view.as_ref().unwrap().search.is_none());
        assert_eq!(h.app.root.workspace.chat.code.view.as_ref().unwrap().document.as_ref().unwrap().text,source(),"Enter/Open reuses parsed preview");
        h.click(|a|matches!(a,Choice::FileChat));
        assert!(h.app.root.workspace.chat.code.view.is_none());
        assert_eq!(h.app.controller.selected().unwrap().local.draft,"Keep my draft");
    }
}
#[test]
fn hidden_toggle_partial_counts_mouse_preview_and_every_exit_have_real_distinct_hit_targets() {
    for mobile in [false,true] {
        let mut h=Harness::new(if mobile {(360,720)} else {(1000,800)},1.,mobile);
        h.index(&["src/main.rs","src/map.rs",".env",".config/options.rs","src/.nested/inside.rs"],true);
        h.app.key("Space",true,false);h.matched();
        assert_eq!(h.app.root.workspace.chat.code.view.as_ref().unwrap().len(),2);
        assert!(h.app.root.workspace.chat.code.view.as_ref().unwrap().search_status().contains("PARTIAL"));
        h.click(|a|matches!(a,Choice::FileHidden));h.matched();
        assert_eq!(h.app.root.workspace.chat.code.view.as_ref().unwrap().len(),5);
        h.click(|a|matches!(a,Choice::FileSelect(p) if p.ends_with("options.rs")));
        assert!(h.app.root.workspace.chat.code.view.as_ref().unwrap().search.is_some(),"single click previews, doesn't exit search");
        assert!(h.app.root.workspace.chat.code.view.as_ref().unwrap().preview_target.as_ref().unwrap().ends_with("options.rs"));
        h.preview(&source());
        let viewport=h.app.root.workspace.chat.code.view.as_ref().unwrap().preview_viewport;
        let start=Vec2::new(viewport.x+120.,viewport.y+120.);h.app.press(17,start,true);h.app.motion(17,Vec2::new(start.x,start.y-60.));h.app.release(17,Vec2::new(start.x,start.y-60.));h.frame();
        assert!(h.app.root.workspace.chat.code.view.as_ref().unwrap().preview_scroll.value>0.);
        assert_eq!(h.app.root.workspace.chat.code.view.as_ref().unwrap().scroll.value,0.,"preview scroll doesn't move result list");
        h.click(|a|matches!(a,Choice::FileHidden));h.matched();assert_eq!(h.app.root.workspace.chat.code.view.as_ref().unwrap().len(),2);
        h.app.input("cannotmatch");h.matched();assert_eq!(h.app.root.workspace.chat.code.view.as_ref().unwrap().len(),0);
        assert!(h.app.root.workspace.chat.code.view.as_ref().unwrap().preview_target.is_none());
        h.app.key("Enter",false,false);h.frame();assert!(h.app.root.workspace.chat.code.view.as_ref().unwrap().search.is_some());
        h.app.key("Escape",false,false);h.frame();assert!(h.app.root.workspace.chat.code.view.is_none(),"Esc from chat-launched picker returns directly to chat");
        for exit in [Choice::FileChat,Choice::FileClose] {
            h.app.key("Space",true,false);h.matched();
            h.click(|a|matches!((&exit,a),(Choice::FileChat,Choice::FileChat)|(Choice::FileClose,Choice::FileClose)));
            assert!(h.app.root.workspace.chat.code.view.is_none());
        }
        h.files();
        h.update(FileReply::Directory {path:"/workspace".into(),parent:Some("/".into()),entries:vec![FileEntry{path:"/workspace/.config".into(),name:".config".into(),directory:true,symlink:false},FileEntry{path:"/workspace/src".into(),name:"src".into(),directory:true,symlink:false}],next:None},None);
        assert_eq!(h.app.root.workspace.chat.code.view.as_ref().unwrap().len(),1);
        h.click(|a|matches!(a,Choice::FileHidden));assert_eq!(h.app.root.workspace.chat.code.view.as_ref().unwrap().len(),2);
        h.click(|a|matches!(a,Choice::FileFind));h.matched();
        h.click(|a|matches!(a,Choice::FileFind));
        assert!(h.app.root.workspace.chat.code.view.as_ref().unwrap().search.is_none(),"Browse explicitly returns to directory");
        h.click(|a|matches!(a,Choice::FileChat));assert!(h.app.root.workspace.chat.code.view.is_none());
    }
}

#[test]
fn background_index_refresh_preserves_selection_and_old_scope_results_cannot_resurface() {
    let mut h=Harness::new((1000,800),1.,false);
    h.index(&["src/a.rs","src/b.rs","src/c.rs"],false);
    h.app.key("Space",true,false);h.matched();
    h.app.key("ArrowDown",false,false);h.frame();h.preview(&source());
    let selected=h.app.root.workspace.chat.code.view.as_ref().unwrap().preview_target.clone();
    h.index(&["src/a.rs","src/b.rs","src/c.rs","src/added.rs"],false);h.matched();
    assert_eq!(h.app.root.workspace.chat.code.view.as_ref().unwrap().preview_target,selected);
    assert!(h.app.root.workspace.chat.code.view.as_ref().unwrap().preview.is_some());
    // Root changes clear the list immediately and do not show the old root on error.
    h.app.root.workspace.chat.code.view.as_mut().unwrap().directory=Some("/elsewhere".into());
    h.click(|a|matches!(a,Choice::FileFindHere));h.frame();
    assert_eq!(h.app.root.workspace.chat.code.view.as_ref().unwrap().len(),0);
    assert!(h.app.root.workspace.chat.code.view.as_ref().unwrap().preview.is_none());
    h.index(&["fresh.rs"],false);
    h.app.open_ui(ui::DialogSpec::Connection).unwrap();h.frame();
    h.matched();
    assert!(!h.app.root.workspace.chat.code.view.as_ref().unwrap().subscribed,"late local matching cannot reopen a preview under a modal");
    h.app.back();h.frame();
    let old=h.app.controller.file_index.clone();
    h.app.controller.select("two").unwrap();h.frame();assert!(h.app.root.workspace.chat.code.view.is_none());
    h.app.controller.file_index=old;
    h.app.key("Space",true,false);h.frame();
    assert!(h.app.root.workspace.chat.code.view.as_ref().unwrap().index_seen.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remote_picker_results_wake_and_repaint_without_input_or_polling_frames() {
    use std::time::Duration;
    async fn until(app: &mut App, ctx: &HeadlessCtx, wakes: &mut tokio::sync::mpsc::UnboundedReceiver<()>, done: impl Fn(&App) -> bool) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        while !done(app) {
            tokio::time::timeout_at(deadline, wakes.recv()).await.expect("Picker did not wake the UI").expect("Wake channel closed");
            let indexed = app.root.workspace.chat.code.view.as_ref().is_some_and(|v| v.index_seen.as_ref().is_some_and(|u| u.index.is_some()));
            let matched = app.root.workspace.chat.code.view.as_ref().is_some_and(|v| v.len() > 0);
            let repaint = app.tick(0.);
            let code = app.root.workspace.chat.code.view.as_ref();
            if (!indexed && code.is_some_and(|v| v.index_seen.as_ref().is_some_and(|u| u.index.is_some())))
                || (!matched && code.is_some_and(|v| v.len() > 0)) {
                assert!(repaint, "Background names/matches must invalidate an idle picker");
            }
            // Model desktop OnDemand: no frames unless an update requests one.
            if repaint { app.frame(ctx, ctx.view()); }
        }
    }
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path().join("work"); std::fs::create_dir_all(cwd.join("src")).unwrap();
    std::fs::write(cwd.join("src/needle.rs"), "fn needle() {}\n").unwrap();
    std::fs::create_dir(cwd.join(".cache")).unwrap();
    std::fs::write(cwd.join(".cache/needle.rs"), "not a fuzzy result").unwrap();
    let tcp = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
    let udp = std::net::UdpSocket::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
    let settings_path = root.path().join("settings.json");
    let mut settings = tau_protocol::settings::Settings::default();
    settings.agent.load_agents_files = false; settings.daemon.generate_titles = false; settings.daemon.idle_timeout_seconds = 0;
    std::fs::write(&settings_path, serde_json::to_vec(&settings).unwrap()).unwrap();
    let daemon = tokio::spawn(taud::run(taud::Config {
        bind: tcp, transfer_bind: match udp { std::net::SocketAddr::V4(a) => a, _ => unreachable!() }, transfer_bind_v6: None,
        token: Arc::from("picker-wake-fixture"), settings_path, import_pi_dir: None, codex_auth_source: None, cwd,
        database_path: root.path().join("daemon.sqlite3"), telemetry_path: root.path().join("crashes.jsonl"),
        attachment_root: root.path().join("outbox"), upload_root: root.path().join("uploads"),
    }));
    let store = Store::open(root.path().join("client")).unwrap();
    store.put("", "settings", &crate::store::Settings { server_url: format!("http://{tcp}"), token: "picker-wake-fixture".into() }).unwrap();
    let ctx = HeadlessCtx::new(&Config { size: (1000, 800), device_limits: crate::desktop::limits(), ..Default::default() }).unwrap();
    let (wake, mut wakes) = tokio::sync::mpsc::unbounded_channel();
    let mut app = App::new(&ctx, store, Arc::new(move || { let _ = wake.send(()); }), false).unwrap();
    app.resize(ctx.size(), 1., Vec2::new(0., 0.));
    // Suppress name warming until the picker opens, without timing races against
    // a fast loopback response. Control/session creation still runs normally.
    app.ui.window_focused = false;
    until(&mut app, &ctx, &mut wakes, |a| a.controller.epoch.is_some() && a.controller.account.source_lineage.is_some()).await;
    app.controller.new_chat().unwrap();
    until(&mut app, &ctx, &mut wakes, |a| a.controller.account.selected.as_ref().is_some_and(|id| a.controller.account.sessions.iter().any(|s| &s.id == id))).await;
    assert!(app.controller.file_index.is_none());
    app.ui.window_focused = true;
    app.key("Space", true, false); app.input("needle.rs");
    assert!(app.tick(0.)); app.frame(&ctx, ctx.view());
    assert_eq!(app.root.workspace.chat.code.view.as_ref().unwrap().search_status(), "Syncing file names…");
    // No input, sleeps, or forced poll/frame loop from here: both names and
    // matcher completion must make it through wake -> tick -> repaint.
    until(&mut app, &ctx, &mut wakes, |a| a.root.workspace.chat.code.view.as_ref().is_some_and(|v| v.len() == 1 && v.preview.is_some())).await;
    let code = app.root.workspace.chat.code.view.as_ref().unwrap();
    assert!(code.search_status().starts_with("1 / 1 files"));
    assert_eq!(code.preview.as_ref().unwrap().text, "fn needle() {}\n");
    assert!(code.paints.values().any(|(spans, _)| !spans.is_empty()), "Results actually painted after the background wake");
    drop(app); daemon.abort(); let _ = daemon.await;
}

#[test]
fn reopening_files_restores_each_chats_buffer_and_reading_position() {
    for mobile in [false, true] {
        let mut h = Harness::new(if mobile { (360,720) } else { (1000,800) }, 1., mobile);
        h.files();
        h.app.with_ui(|root, cx| root.workspace.chat.code.code_action(Choice::FileOpen("/workspace/src/main.rs".into(), false), cx)).unwrap();
        h.text(&source());
        let point = h.line(3, true);
        h.app.press(3, point, mobile); h.app.release(3, point); h.frame();
        h.app.root.workspace.chat.code.view.as_mut().unwrap().scroll.value = 240.;
        h.frame();
        let original = h.app.root.workspace.chat.code.view.as_ref().unwrap().document.clone().unwrap();
        let scroll = h.app.root.workspace.chat.code.view.as_ref().unwrap().scroll.value;
        let draft = h.app.controller.selected().unwrap().local.draft.clone();
        for exit in [Choice::FileChat, Choice::FileClose] {
            let generation = h.app.controller.viewer_generation();
            h.click(|a| a == &exit);
            assert!(h.app.root.workspace.chat.code.view.is_none());
            assert!(h.app.controller.viewer_generation() > generation, "Hidden buffers do not keep a live file interest");
            h.files();
            let view = h.app.root.workspace.chat.code.view.as_ref().unwrap();
            assert_eq!(view.path.as_deref(), Some("/workspace/src/main.rs"));
            assert!(matches!(view.operation, FileOperation::Open { .. }), "Revalidate the file, not its parent directory");
            assert!(Arc::ptr_eq(view.document.as_ref().unwrap(), &original), "Reopening is immediate, not a blank directory reload");
            assert_eq!(view.scroll.value, scroll);
            assert_eq!(view.selection.as_ref().unwrap().range(&original), Some(3..4));
            assert_eq!(h.app.controller.selected().unwrap().local.draft, draft);
        }
        h.app.with_ui(|root, cx| root.workspace.navigate_chat("two", cx)).unwrap(); h.frame();
        h.files();
        assert!(h.app.root.workspace.chat.code.view.as_ref().unwrap().document.is_none(), "Another chat must not inherit this buffer");
        h.app.with_ui(|root, cx| root.workspace.navigate_chat("demo", cx)).unwrap(); h.frame();
        h.files();
        assert!(Arc::ptr_eq(h.app.root.workspace.chat.code.view.as_ref().unwrap().document.as_ref().unwrap(), &original));
        // A reset source must not recover an old source's file contents or line selection.
        h.app.controller.account.source_lineage = Some("replacement-source".into()); h.frame();
        assert!(h.app.root.workspace.chat.code.view.is_none());
        h.files();
        assert!(h.app.root.workspace.chat.code.view.as_ref().unwrap().document.is_none());
    }
}

#[test]
fn closing_the_picker_keeps_the_open_file_not_its_preview_and_restores_scroll() {
    let mut h = Harness::new((1000,800), 1., false);
    h.files();
    h.app.with_ui(|root, cx| root.workspace.chat.code.code_action(Choice::FileOpen("/workspace/src/main.rs".into(), false), cx)).unwrap();
    h.text(&source());
    h.app.root.workspace.chat.code.view.as_mut().unwrap().scroll.value = 300.; h.frame();
    let scroll = h.app.root.workspace.chat.code.view.as_ref().unwrap().scroll.value;
    for close_browser in [false, true] {
        h.click(|a| matches!(a, Choice::FileFind));
        h.index(&["other.rs"], false); h.matched(); h.preview("not the opened file\n");
        if close_browser {
            h.click(|a| matches!(a, Choice::FileChat)); h.files();
        } else {
            h.click(|a| matches!(a, Choice::FileFind));
        }
        let code = h.app.root.workspace.chat.code.view.as_ref().unwrap();
        assert!(code.search.is_none());
        assert_eq!(code.path.as_deref(), Some("/workspace/src/main.rs"));
        assert_eq!(code.document.as_ref().unwrap().text, source());
        assert_eq!(code.scroll.value, scroll);
    }
}

#[test]
fn code_view_wheel_uses_the_shared_smooth_scroll_owner() {
    let mut h = Harness::new((1000, 800), 1., false);
    h.files();
    h.text(&"A line long enough to scroll horizontally as well.\n".repeat(200));
    let view = h.app.root.workspace.chat.code.view.as_ref().unwrap();
    let point = Vec2::new(view.viewport.x + 80., view.viewport.y + 100.);
    let before = view.scroll.value;
    h.app.wheel(96., false, point);
    let view = h.app.root.workspace.chat.code.view.as_ref().unwrap();
    assert!(view.scroll.motion.wheel.is_some(), "Files must use the same smooth wheel scrolling as the transcript");
    assert_eq!(view.scroll.value, before, "A wheel notch is animated, not a different immediate jump in Files");
    assert!(h.app.needs_redraw(), "Wheel input starts the on-demand animation");
    h.app.scroll(24., false, point, true);
    let view = h.app.root.workspace.chat.code.view.as_ref().unwrap();
    assert_eq!(view.scroll.value, before + 24.);
    assert!(!view.scroll.motion.active(), "Native touchpad input must not receive a second easing/momentum tail");
}

#[test]
fn file_and_picker_preview_horizontal_touch_flings_share_physics_and_cancel() {
    for preview in [false, true] {
        let mut h = Harness::new((360, 720), 1., true);
        let text = format!("{}\n", "a wide source line ".repeat(20)).repeat(30);
        if preview {
            h.index(&["src/long.rs"], false);
            h.app.key("Space", true, false);
            h.matched();
            h.preview(&text);
        } else { h.files(); h.text(&text); }
        let view = h.app.root.workspace.chat.code.view.as_ref().unwrap();
        let viewport = if preview { view.preview_viewport } else { view.viewport };
        let start = Vec2::new(viewport.x + viewport.width / 2., viewport.y + viewport.height / 2.);
        h.app.press(61, start, true);
        let mut end = start;
        for i in 1..=3 {
            std::thread::sleep(std::time::Duration::from_millis(16));
            end.x = start.x - i as f32 * 24.;
            h.app.motion(61, end);
        }
        h.app.release(61, end);
        let view = h.app.root.workspace.chat.code.view.as_ref().unwrap();
        let axis = if preview { &view.preview_horizontal } else { &view.horizontal };
        let released = axis.value;
        assert!(axis.motion.active(), "preview={preview}");
        h.frame(); // Layout must not discard a newly released fling.
        assert!(h.app.tick(1. / 60.));
        h.app.frame(&h.ctx, h.ctx.view());
        let view = h.app.root.workspace.chat.code.view.as_ref().unwrap();
        assert!((if preview { view.preview_horizontal.value } else { view.horizontal.value }) > released);
        assert_eq!(view.scroll.value, 0.);
        assert_eq!(view.preview_scroll.value, 0.);
        h.app.cancel_pointer();
        let view = h.app.root.workspace.chat.code.view.as_ref().unwrap();
        assert!(!view.horizontal.motion.active() && !view.preview_horizontal.motion.active());
        assert_eq!(h.app.controller.selected().unwrap().local.draft, "");
    }
}

#[test]
fn explorer_rows_are_bare_monospace_names_and_long_names_stay_on_one_line() {
    let mut h = Harness::new((360,720), 1., true);
    let rect = Rect::new(18., 100., 320., 24.);
    for (name, directory, expected) in [("README.md", false, "README.md"), ("src", true, "src/")] {
        let mut layer = Layer::default();
        h.app.with_ui(|_, cx| paint::entry_label(name, directory, rect, rect, &mut layer, cx));
        assert_eq!(layer.draws.len(), 1);
        let draw = &layer.draws[0];
        let style = Style { chain: h.app.services.renderer.faces.mono[0], wrap_em: None, align: Align::Left, line_spacing: 1. };
        let reference = h.app.services.renderer.text.shape_transient(expected, &style).unwrap();
        assert_eq!(h.app.services.renderer.text.measure(draw.block).width_em(), h.app.services.renderer.text.measure(reference).width_em(),
            "Neither a prefix glyph nor an icon column belongs in a file row");
        assert_eq!(draw.at.x, rect.x);
    }
    for (name, directory) in [(format!("{}.rs", "long-file-name-".repeat(30)), false), ("cafe\u{301}-directory-".repeat(30), true)] {
        let mut layer = Layer::default();
        h.app.with_ui(|_, cx| paint::entry_label(&name, directory, rect, rect, &mut layer, cx));
        let draw = &layer.draws[0]; let size = h.app.services.renderer.text.measure(draw.block);
        assert!(size.width_em() * draw.size <= rect.width);
        assert!(size.height_em() * draw.size <= rect.height, "Long names ellipsize, never wrap into the next row");
    }
}
