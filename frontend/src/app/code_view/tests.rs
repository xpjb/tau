use super::*;
use chad::{Config, HeadlessCtx};
struct Harness {app:App,ctx:HeadlessCtx,_root:tempfile::TempDir}
impl Harness {
    fn new(size:(u32,u32),scale:f32,mobile:bool)->Self {
        let root=tempfile::tempdir().unwrap();
        let ctx=HeadlessCtx::new(&Config {size,device_limits:crate::desktop::limits(),..Default::default()}).unwrap();
        let mut app=App::new(&ctx,Store::open(root.path().into()).unwrap(),Arc::new(||{}),mobile).unwrap();
        app.back();crate::demo::populate(&mut app.controller).unwrap();app.resize(size,scale,Vec2::new(0.,0.));app.tick(0.);app.root.legacy.show_chats=false;app.frame(&ctx,ctx.view());
        Self {app,ctx,_root:root}
    }
    fn frame(&mut self) {self.app.tick(0.);self.app.frame(&self.ctx,self.ctx.view());}
    fn click(&mut self,predicate:impl Fn(&Action)->bool) {
        let r=self.app.root.legacy.hits.iter().find(|h|predicate(&h.action)).unwrap().rect;
        let p=Vec2::new(r.x+r.width/2.,r.y+r.height/2.);self.app.press(1,p,self.app.ui.mobile);self.app.release(1,p);self.frame();
    }
    fn update(&mut self,reply:FileReply,document:Option<Arc<Document>>) {
        let view=self.app.root.legacy.code.as_ref().unwrap();
        self.app.controller.file_update=Some(Arc::new(crate::file_client::Update {generation:view.generation,session:view.session.clone(),lineage:view.lineage.clone().unwrap_or_default(),response:Ok(reply),document}));self.frame();
    }
    fn text(&mut self,text:&str) {
        let old=self.app.root.legacy.code.as_ref().unwrap().document.clone();
        let doc=Arc::new(Document::replace(old.as_deref(),"/workspace/src/main.rs".into(),blake3::hash(text.as_bytes()).to_hex().to_string(),text.into()));
        self.update(FileReply::Text {path:doc.path.clone(),revision:doc.revision.clone(),text:String::new()},Some(doc));
    }
    fn line(&self,n:usize,gutter:bool)->Vec2 {
        let c=self.app.root.legacy.code.as_ref().unwrap();Vec2::new(c.viewport.x+if gutter{20.}else{120.}*self.app.ui.scale,c.viewport.y+n as f32*c.line_height+0.5*c.line_height-c.scroll)
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
        let attachment=h.app.root.legacy.hits.iter().find(|h|matches!(h.action,Action::Attachments)).unwrap().rect;
        let files=h.app.root.legacy.hits.iter().find(|h|matches!(h.action,Action::Files)).unwrap().rect;
        assert!(files.x<attachment.x && (attachment.x-files.x)<=48.*scale);
        h.app.controller.draft("Existing draft".into()).unwrap();h.frame();h.click(|a|matches!(a,Action::Files));
        h.update(FileReply::Directory {path:"/workspace".into(),parent:Some("/".into()),entries:vec![FileEntry {path:"/workspace/src".into(),name:"src".into(),directory:true,symlink:false},FileEntry {path:"/workspace/README.md".into(),name:"README.md".into(),directory:false,symlink:false}],next:None},None);
        assert!(!h.app.root.legacy.hits.iter().any(|h|matches!(h.action,Action::Focus(None))));h.dump(&format!("{name}-directory.png"));
        h.app.apply(Action::FileOpen("/workspace/src/main.rs".into(),false)).unwrap();h.text(&source());h.dump(&format!("{name}-code.png"));
        assert!(!h.app.root.legacy.hits.iter().any(|h|matches!(h.action,Action::Focus(None))));
        let start=h.line(1,true);let end=h.line(3,true);
        h.app.press(9,start,mobile);h.app.motion(9,end);h.app.release(9,end);h.frame();
        assert!(h.app.root.legacy.hits.iter().any(|h|matches!(h.action,Action::Focus(None))));
        assert_eq!(h.app.controller.selected().unwrap().local.draft,"Existing draft\n\n`/workspace/src/main.rs:2-4`\n");
        assert!(h.app.controller.selected().unwrap().local.pending.is_empty(),"Selection must not send");
        h.dump(&format!("{name}-comment.png"));
        h.click(|a|matches!(a,Action::FileCopy));
        assert!(h.app.actions().iter().any(|a|matches!(a,PlatformAction::Copy(t) if t=="fn main() {\n    let answer = 42;\n    println!(\"{answer}\");")));
        h.app.root.legacy.focus=Some(None);h.app.input("Why 42?");h.frame();
        h.text(&format!("// inserted above\n{}",source()));
        assert_eq!(h.app.controller.selected().unwrap().local.draft,"Existing draft\n\n`/workspace/src/main.rs:3-5`\nWhy 42?");
        assert_eq!(h.app.code_native_value("Existing draft\n\n`/workspace/src/main.rs:2-4`\nStill typing".into()), "Existing draft\n\n`/workspace/src/main.rs:3-5`\nStill typing");
        h.text(&format!("// inserted above\n{}",source().replace("let answer = 42;","let answer = 43;")));
        assert!(h.app.root.legacy.code.as_ref().unwrap().selection.is_none());
        assert!(!h.app.controller.selected().unwrap().local.draft.contains("main.rs:"));
        assert!(h.app.controller.selected().unwrap().local.draft.ends_with("Why 42?"));
        assert!(h.app.root.legacy.hits.iter().any(|h|matches!(h.action,Action::Focus(None))), "An active comment edit is not hidden/lost by a live replacement");
        assert!(h.app.apply(Action::Send).is_err());
        assert!(!h.app.code_native_value("Existing draft\n\n`/workspace/src/main.rs:2-4`\nStill typing".into()).contains("main.rs:"));
        h.click(|a|matches!(a,Action::FileClear));h.frame();
        assert!(!h.app.root.legacy.hits.iter().any(|h|matches!(h.action,Action::Focus(None))));
        h.click(|a|matches!(a,Action::FileClose));assert!(h.app.root.legacy.code.is_none());
        assert!(h.app.root.legacy.hits.iter().any(|h|matches!(h.action,Action::Focus(None))));
    }
}
#[test]
fn touch_hold_haptics_drag_undrag_scroll_and_chat_switch_are_scoped() {
    let mut h=Harness::new((360,720),1.,true);h.click(|a|matches!(a,Action::Files));h.text(&source());
    let start=h.line(1,false);h.app.press(4,start,true);
    h.app.root.legacy.pointer.as_mut().unwrap().started=Instant::now()-std::time::Duration::from_millis(500);h.frame();
    assert!(h.app.actions().iter().any(|a|matches!(a,PlatformAction::Haptic)));
    let end=h.line(4,false);h.app.motion(4,end);h.frame();
    let doc=h.app.root.legacy.code.as_ref().unwrap().document.as_ref().unwrap();assert_eq!(h.app.root.legacy.code.as_ref().unwrap().selection.as_ref().unwrap().range(doc),Some(1..5));
    let end=h.line(2,false);h.app.motion(4,end);h.app.release(4,end);h.frame();
    assert!(h.app.controller.selected().unwrap().local.draft.contains("main.rs:2-3"));
    h.click(|a|matches!(a,Action::FileClear));
    let start=h.line(5,false);h.app.press(6,start,true);h.app.motion(6,Vec2::new(start.x,start.y-90.));h.app.release(6,Vec2::new(start.x,start.y-90.));h.frame();
    assert!(h.app.root.legacy.code.as_ref().unwrap().scroll>0.);assert!(h.app.root.legacy.code.as_ref().unwrap().selection.is_none());
    let old_draft=h.app.controller.selected().unwrap().local.draft.clone();
    h.app.controller.select("two").unwrap();h.frame();assert!(h.app.root.legacy.code.is_none());
    assert_eq!(h.app.controller.chats["demo"].local.draft,old_draft);
    assert!(!h.app.controller.selected().unwrap().local.draft.contains("main.rs:"));
}
#[test]
fn telescope_reuses_real_editor_and_cancels_without_losing_code_selection() {
    let mut h=Harness::new((1000,800),1.,false);h.click(|a|matches!(a,Action::Files));h.text(&source());
    let start=h.line(1,true);h.app.press(1,start,false);h.app.release(1,start);h.frame();
    let before=h.app.controller.selected().unwrap().local.draft.clone();
    h.app.key("Space",true,false);h.frame();assert!(h.app.root.legacy.code.as_ref().unwrap().search.is_some());
    h.app.input("srcmain");h.frame();assert_eq!(h.app.root.legacy.code.as_ref().unwrap().search.as_ref().unwrap().value,"srcmain");
    assert_eq!(h.app.controller.selected().unwrap().local.draft,before,"Search text is not a chat draft");
    h.update(FileReply::Search {path:"/workspace/src".into(),entries:vec![FileEntry {path:"/workspace/src/main.rs".into(),name:"main.rs".into(),directory:false,symlink:false}],indexing:false,limited:false},None);
    h.dump("desktop-search.png");h.app.key("Escape",false,false);h.frame();h.text(&source());
    assert!(h.app.root.legacy.code.as_ref().unwrap().selection.is_some());assert_eq!(h.app.controller.selected().unwrap().local.draft,before);
    // Rendering the same actual code frame does not reshape source paragraphs.
    sanscale::profiling::reset_work_counters();h.frame();
    assert_eq!(sanscale::profiling::work_counters().source_reads,0);
    h.app.controller.file_update=Some(Arc::new(crate::file_client::Update {generation:99_999,session:"other".into(),lineage:"other".into(),response:Err("old response".into()),document:None}));h.frame();
    assert!(h.app.root.legacy.code.as_ref().unwrap().error.is_none());
}

#[test]
fn live_references_preserve_ime_composition_and_newer_saved_draft_text() {
    let mut h=Harness::new((1000,800),1.,false);h.click(|a|matches!(a,Action::Files));h.text(&source());
    let start=h.line(1,true);h.app.press(1,start,false);h.app.release(1,start);h.frame();
    h.app.preedit("入力".into(),None);assert!(h.app.composing());
    h.text(&format!("// inserted\n{}",source()));
    assert!(h.app.composing(),"A live line-number update must not cancel the IME");
    assert!(h.app.root.legacy.composer.value.contains("main.rs:2-2"));
    h.app.input("入力");h.frame();
    assert!(h.app.root.legacy.composer.value.contains("main.rs:3-3"));assert!(h.app.root.legacy.composer.value.ends_with("入力"));
    let draft=format!("Newer saved prefix\n{}",h.app.controller.selected().unwrap().local.draft);
    h.app.controller.draft(draft).unwrap();
    h.text(&format!("// inserted twice\n// inserted\n{}",source()));
    assert!(h.app.root.legacy.composer.value.starts_with("Newer saved prefix\n"));assert!(h.app.root.legacy.composer.value.contains("main.rs:4-4"));
    h.app.preedit(" still typing".into(),None);
    h.text(&format!("// inserted twice\n// inserted\n{}",source().replace("fn main()", "fn renamed()")));
    assert!(h.app.composing());h.app.input(" still typing");h.frame();
    assert!(h.app.root.legacy.composer.value.ends_with("入力 still typing"));assert!(!h.app.root.legacy.composer.value.contains("main.rs:"));
    assert!(h.app.apply(Action::Send).is_err());
}
