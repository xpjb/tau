//! Real native layout, hit regions, pointer/keyboard gestures and local storage.
use super::*;
use chad::{Config, HeadlessCtx};
use std::sync::Arc;

struct Harness { app: App, ctx: HeadlessCtx, _root: tempfile::TempDir }
impl Harness {
    fn new(size: (u32,u32), mobile: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        let ctx = HeadlessCtx::new(&Config { size, device_limits: crate::desktop::limits(), ..Default::default() }).unwrap();
        let mut app = App::new(&ctx, Store::open(root.path().into()).unwrap(), Arc::new(|| {}), mobile).unwrap();
        app.back(); crate::demo::populate(&mut app.controller).unwrap();
        app.controller.account.projects.extend((1..=24).map(|i| Project { id:format!("p{i}"), name:format!("Topic {i}"), prompt:format!("Exact prompt {i}\n"),revision:i }));
        app.controller.account.projects[1].name = "Tau development".into();
        app.controller.account.projects[2].name = "Research".into();
        app.controller.account.sessions[2].project_id = "p24".into();
        app.resize(size,1.,Vec2::new(0.,0.)); app.tick(0.); app.root.workspace.show_chats = mobile;
        Self { app,ctx,_root:root }
    }
    fn frame(&mut self) { self.app.tick(0.); self.app.frame(&self.ctx,self.ctx.view()); }
    fn projects(&self) -> Vec<(Rect, String)> {
        self.app.root.workspace.sidebar.projects.controls.placed().filter_map(|(a,r)|
            if let ui::sidebar::TopicChoice::Select(id)=a {Some((r,id.clone()))} else {None}).collect()
    }
    fn chats(&self) -> Vec<(Rect, String)> {
        self.app.root.workspace.sidebar.controls.placed().filter_map(|(a,r)|
            if let ui::sidebar::Choice::Select(id)=a {Some((r,id.clone()))} else {None}).collect()
    }
    fn click(&mut self,r:Rect) {
        let p = Vec2::new(r.x+r.width/2.,r.y+r.height/2.);
        self.app.press(1,p,self.app.ui.mobile); self.app.release(1,p); self.frame();
    }
    fn click_dialog(&mut self, label: &str) {
        let r = self.app.root.dialog.as_ref().unwrap().button(label).expect("visible retained control");
        let p = Vec2::new(r.x + r.width / 2., r.y + r.height / 2.);
        self.app.press(1, p, self.app.ui.mobile); self.app.release(1, p); self.frame();
    }
    fn dump(&self, file: &str) {
        if let Some(root) = std::env::var_os("TAU_PROJECT_DUMP_DIR") {
            std::fs::create_dir_all(&root).unwrap();
            let size = self.ctx.size();
            image::save_buffer(PathBuf::from(root).join(file),&self.ctx.read_rgba8().unwrap(),size.0,size.1,image::ColorType::Rgba8).unwrap();
        }
    }
}
#[test]
fn project_tabs_gestures_unread_nested_menus_and_confirmation_use_actual_native_ui() {
    let size = (360, 720);
    let mut h = Harness::new(size,true); h.frame();
    assert!(h.app.controller.project_unread("p24"));
    assert_eq!(h.chats().iter().filter(|(r,_)| contains(h.app.root.workspace.sidebar.scroll.rect, Vec2::new(r.x+1.,r.y+1.))).count(),2,"Only General's chats appear");
    let bounds = h.app.root.workspace.sidebar.projects.scroll.rect;
    let point = Vec2::new(bounds.x + bounds.width / 2., bounds.y + bounds.height / 2.);
    h.app.wheel(300.,false,point);
    assert!(h.app.root.workspace.sidebar.projects.scroll.wheel.is_some());
    h.app.root.workspace.sidebar.projects.scroll.wheel.as_mut().unwrap().1 = Instant::now()-std::time::Duration::from_secs(1);
    h.frame(); assert!(h.app.root.workspace.sidebar.projects.scroll.value>0.); assert_eq!(h.app.root.workspace.sidebar.scroll.value,0.);
    h.app.wheel(100.,true,point); assert!(h.app.root.workspace.sidebar.projects.scroll.wheel.is_some());
    h.app.press(4,point,true); h.app.motion(4,Vec2::new(point.x - 65., point.y));
    assert!(h.app.ui.capture.as_ref().unwrap().dragged); h.app.release(4,Vec2::new(point.x - 65., point.y));
    assert!(h.app.root.menu.is_none(),"Swiping is not a long press or a tab selection");
    h.app.root.workspace.sidebar.projects.scroll.velocity=0.; h.app.with_ui(|root, cx| root.workspace.navigate_project("p24", cx)).unwrap(); h.frame();
    assert_eq!(h.app.controller.account.selected.as_deref(),None,"Mobile waits for a chat choice");
    assert!(h.app.controller.project_unread("p24"), "Only a visible chat is read");
    assert!(h.app.root.workspace.show_chats, "Mobile topic switches keep the list open");
    assert_eq!(h.chats().iter().filter(|(r,_)| contains(h.app.root.workspace.sidebar.scroll.rect,Vec2::new(r.x+1.,r.y+1.))).map(|(_,id)| id.as_str()).collect::<Vec<_>>(),vec!["three"]);
    assert!(h.projects().iter().any(|(_,id)| id=="p24"),"Selected tab auto-reveals");
    h.click(h.chats().iter().find(|(_,id)|id=="three").unwrap().0);
    assert!(!h.app.controller.project_unread("p24"));
    h.app.with_ui(|root, cx| root.workspace.navigate_project("general", cx)).unwrap(); h.frame();
    assert_eq!(h.app.controller.account.selected.as_deref(),None);
    h.app.controller.select("demo").unwrap(); h.app.tick(0.); h.app.root.workspace.show_chats=true; h.frame();
    let target = h.chats().iter().find(|(_,id)| id=="two").unwrap().0;
    let point=Vec2::new(target.x+60.,target.y+20.);
    h.app.press(2,point,true); h.app.ui.capture.as_mut().unwrap().started=Instant::now()-std::time::Duration::from_millis(500);
    h.frame(); assert!(h.app.ui.capture.is_none(),"Hold opens before lift");
    h.app.release(2,point);
    assert_eq!(h.app.controller.account.selected.as_deref(),Some("demo"));
    assert_eq!(h.app.root.menu.as_ref().unwrap().chat.as_deref(),Some("two"));
    let r = h.app.root.menu.as_ref().unwrap().button(|a| matches!(a,ui::MenuChoice::MoveMenu(id) if id=="two")).unwrap();
    let point = Vec2::new(r.x+r.width/2., r.y+r.height/2.); h.app.press(1,point,true); h.app.release(1,point); h.frame();
    assert!(h.app.root.menu.as_ref().unwrap().parent.is_some());
    h.dump("projects-phone-menu.png");
    for _ in 0..24 { h.app.key("ArrowDown",false,false); }
    h.frame();
    assert!(h.app.root.menu.as_ref().unwrap().scroll.value>0.);
    assert!(h.app.root.menu.as_ref().unwrap().button(|a| matches!(a,ui::MenuChoice::MoveChat(chat,project) if chat=="two" && project=="p24")).is_some_and(|r| r.height > 0.),"Every topic is reachable in the clipped submenu");
    let menu=h.app.root.menu.as_ref().unwrap();
    assert!(menu.rect.y>=0. && menu.rect.y+menu.rect.height<=size.1 as f32);
    h.app.key("ArrowLeft",false,false); h.frame(); assert!(h.app.root.menu.as_ref().unwrap().parent.is_none());
    h.app.key("Escape",false,false); h.frame();
    let general_tab=h.projects().iter().find(|(_,id)| id=="general").unwrap().0;
    let point=Vec2::new(general_tab.x+20.,general_tab.y+20.);
    h.app.context_at(point); h.frame();
    assert_eq!(h.app.root.menu.as_ref().unwrap().options.len(),1,"General is permanent but its prompt is editable");
    h.app.key("Escape",false,false);
    h.app.root.workspace.sidebar.projects.scroll.value=h.app.root.workspace.sidebar.projects.scroll.max; h.frame();
    let new=h.app.root.workspace.sidebar.projects.controls.placed().find(|(a,_)|matches!(a,ui::sidebar::TopicChoice::New)).unwrap().1;
    h.click(new);
    assert_eq!(h.app.root.dialog.as_ref().unwrap().topic_key().unwrap().0, "new");
    h.app.input("gypqj New work"); // descenders in the compact Name editor
    h.app.ui.focus=Some(h.app.root.dialog.as_ref().unwrap().fields()[1].control.target); h.app.input("  Exact\ncontext\n"); h.frame();
    assert_eq!(h.app.root.dialog.as_ref().unwrap().fields()[1].editor.value,"  Exact\ncontext\n");
    h.dump("projects-phone-new.png");
    h.click_dialog("Cancel");
    h.app.open_ui(ui::DialogSpec::Topic(ui::TopicEdit::Rename("p1".into()))).unwrap(); h.frame();
    h.app.back();
    h.app.open_ui(ui::DialogSpec::Topic(ui::TopicEdit::Delete("p24".into()))).unwrap(); h.frame();
    h.click_dialog("Continue…");
    assert_eq!(h.app.root.dialog.as_ref().unwrap().topic_key(), Some(("delete-choice", "p24")));
    assert_eq!(h.app.root.dialog.as_ref().unwrap().buttons().len(),3);
    h.dump("projects-phone-delete.png");
    h.click_dialog("Cancel");
    assert_eq!(h.app.controller.account.projects.len(),25);
    assert_eq!(h.app.controller.account.sessions.len(),3,"Cancellation changes nothing");
    h.app.root.workspace.sidebar.projects.scroll.value=0.; h.app.with_ui(|root, cx| root.workspace.navigate_project("general", cx)).unwrap(); h.frame();
    h.dump("projects-phone-tabs.png");
}

#[test]
fn selected_topic_stays_visible_when_its_chat_bumps_from_the_far_right() {
    for (size, mobile) in [((1000, 800), false), ((360, 720), true)] {
        let mut h = Harness::new(size, mobile);
        let mut sessions = h.app.controller.account.sessions.clone();
        sessions[2].updated_at_ms = 1;
        for i in 1..24 {
            let mut chat = sessions[2].clone();
            chat.id = format!("topic-{i}-chat");
            chat.project_id = format!("p{i}");
            chat.updated_at_ms = 2 + i;
            sessions.push(chat);
        }
        h.app.controller.message(ServerMessage::Sessions { sessions }).unwrap();
        h.app.with_ui(|root, cx| root.workspace.navigate_project("p24", cx)).unwrap();
        h.frame();
        assert!(h.app.root.workspace.sidebar.projects.scroll.value > 0.);
        assert!(h.projects().iter().any(|(_, id)| id == "p24"));
        h.app.with_ui(|root, cx| root.workspace.navigate_chat("three", cx)).unwrap();
        h.app.tick(0.);
        if mobile { h.app.back(); }
        h.frame();
        h.app.controller.draft("bump this topic".into()).unwrap();
        h.frame();
        assert_eq!(h.app.controller.account.projects[1].id, "p24");
        assert!(h.projects().iter().any(|(_, id)| id == "p24"), "the selected tab remains visible after moving left (mobile={mobile}, scroll={}, tabs={:?})", h.app.root.workspace.sidebar.projects.scroll.value, h.projects().iter().map(|(_, id)| id.as_str()).collect::<Vec<_>>());
    }
}
