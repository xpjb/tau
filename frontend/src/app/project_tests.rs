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
        app.resize(size,1.,Vec2::new(0.,0.)); app.tick(0.); app.root.legacy.show_chats = mobile;
        Self { app,ctx,_root:root }
    }
    fn frame(&mut self) { self.app.tick(0.); self.app.frame(&self.ctx,self.ctx.view()); }
    fn click(&mut self, pred: impl Fn(&Action)->bool) {
        let r = self.app.test_hits().iter().find(|h| pred(&h.action)).expect("visible action").rect;
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
    for (size,mobile) in [((1000,800),false),((360,720),true)] {
        let mut h = Harness::new(size,mobile); h.frame();
        if !mobile {
            // When the tab strip overflows, its clipped shapes/text and hit
            // targets must stop before the sidebar/chat separator at x=299.
            let pixels = h.ctx.read_rgba8().unwrap();
            let at = |y: usize| &pixels[(y*size.0 as usize+299)*4..(y*size.0 as usize+299)*4+3];
            for y in 140..175 {
                assert_eq!(at(y), at(130), "tab strip painted over the separator at y={y}");
            }
            assert!(h.app.test_projects().iter().all(|(r,_)| r.x+r.width<=299.));
        }
        assert!(h.app.controller.project_unread("p24"));
        assert_eq!(h.app.test_chats().iter().filter(|(r,_)| contains(h.app.root.sidebar.scroll.rect, Vec2::new(r.x+1.,r.y+1.))).count(),2,"Only General's chats appear");
        assert!(!h.app.test_hits().iter().any(|hit| matches!(hit.action,Action::NewProject)),"The plus belongs at the scrolling end");
        let point = Vec2::new(100.,160.);
        h.app.wheel(300.,false,point);
        assert!(h.app.root.sidebar.projects.scroll.wheel.is_some());
        h.app.root.sidebar.projects.scroll.wheel.as_mut().unwrap().1 = Instant::now()-std::time::Duration::from_secs(1);
        h.frame(); assert!(h.app.root.sidebar.projects.scroll.value>0.); assert_eq!(h.app.root.sidebar.scroll.value,0.);
        h.app.wheel(100.,true,point); assert!(h.app.root.sidebar.projects.scroll.wheel.is_some());
        h.app.press(4,point,true); h.app.motion(4,Vec2::new(35.,161.));
        assert!(h.app.ui.capture.as_ref().unwrap().dragged); h.app.release(4,Vec2::new(35.,161.));
        assert!(h.app.root.menu.is_none(),"Swiping is not a long press or a tab selection");
        h.app.root.sidebar.projects.scroll.velocity=0.; h.app.apply(Action::SelectProject("p24".into())).unwrap(); h.frame();
        assert_eq!(h.app.controller.account.selected.as_deref(),if mobile { None } else { Some("three") },"Only desktop resumes the last chat");
        assert_eq!(h.app.controller.project_unread("p24"), mobile, "Only a visible chat is read");
        assert_eq!(h.app.root.legacy.show_chats, mobile, "Mobile topic switches keep the list open");
        assert_eq!(h.app.test_chats().iter().filter(|(r,_)| contains(h.app.root.sidebar.scroll.rect,Vec2::new(r.x+1.,r.y+1.))).map(|(_,id)| id.as_str()).collect::<Vec<_>>(),vec!["three"]);
        assert!(h.app.test_projects().iter().any(|(_,id)| id=="p24"),"Selected tab auto-reveals");
        h.click(|a| matches!(a,Action::Select(id) if id=="three"));
        assert!(!h.app.controller.project_unread("p24"));
        h.app.apply(Action::SelectProject("general".into())).unwrap(); h.frame();
        assert_eq!(h.app.controller.account.selected.as_deref(),if mobile { None } else { Some("demo") });
        h.app.controller.select("demo").unwrap(); h.app.tick(0.); h.app.root.legacy.show_chats=mobile; h.frame();
        let target = h.app.test_chats().iter().find(|(_,id)| id=="two").unwrap().0;
        let point=Vec2::new(target.x+60.,target.y+20.);
        if mobile {
            h.app.press(2,point,true); h.app.ui.capture.as_mut().unwrap().started=Instant::now()-std::time::Duration::from_millis(500);
            h.frame(); assert!(h.app.ui.capture.is_none(),"Hold opens before lift");
            h.app.release(2,point);
        } else { h.app.context_at(point); h.frame(); }
        assert_eq!(h.app.controller.account.selected.as_deref(),Some("demo"));
        assert_eq!(h.app.root.menu.as_ref().unwrap().chat.as_deref(),Some("two"));
        let r = h.app.root.menu.as_ref().unwrap().button(|a| matches!(a,ui::MenuChoice::MoveMenu(id) if id=="two")).unwrap();
        let point = Vec2::new(r.x+r.width/2., r.y+r.height/2.); h.app.press(1,point,mobile); h.app.release(1,point); h.frame();
        assert!(h.app.root.menu.as_ref().unwrap().parent.is_some());
        h.dump(if mobile {"projects-phone-menu.png"} else {"projects-desktop-menu.png"});
        for _ in 0..24 { h.app.key("ArrowDown",false,false); }
        h.frame();
        assert!(h.app.root.menu.as_ref().unwrap().scroll.value>0.);
        assert!(h.app.root.menu.as_ref().unwrap().button(|a| matches!(a,ui::MenuChoice::MoveChat(chat,project) if chat=="two" && project=="p24")).is_some_and(|r| r.height > 0.),"Every topic is reachable in the clipped submenu");
        assert!(h.app.test_hits().iter().all(|hit| hit.rect.y>=0. && hit.rect.y+hit.rect.height<=size.1 as f32));
        h.app.key("ArrowLeft",false,false); h.frame(); assert!(h.app.root.menu.as_ref().unwrap().parent.is_none());
        h.app.key("Escape",false,false); h.frame();
        let general_tab=h.app.test_projects().iter().find(|(_,id)| id=="general").unwrap().0;
        let point=Vec2::new(general_tab.x+20.,general_tab.y+20.);
        h.app.context_at(point); h.frame();
        assert_eq!(h.app.root.menu.as_ref().unwrap().options.len(),1,"General is permanent but its prompt is editable");
        h.app.key("Escape",false,false);
        h.app.root.sidebar.projects.scroll.value=h.app.root.sidebar.projects.scroll.max; h.frame();
        h.click(|a| matches!(a,Action::NewProject));
        assert_eq!(h.app.root.dialog.as_ref().unwrap().topic_key().unwrap().0, "new");
        h.app.input("gypqj New work"); // descenders in the compact Name editor
        h.app.ui.focus=Some(h.app.root.dialog.as_ref().unwrap().fields()[1].control.target); h.app.input("  Exact\ncontext\n"); h.frame();
        assert_eq!(h.app.root.dialog.as_ref().unwrap().fields()[1].editor.value,"  Exact\ncontext\n");
        h.dump(if mobile {"projects-phone-new.png"} else {"projects-desktop-new.png"});
        h.click_dialog("Cancel");
        h.app.apply(Action::RenameProject("p1".into())).unwrap(); h.frame();
        assert_eq!(h.app.root.dialog.as_ref().unwrap().fields()[0].control.rect.unwrap().height, 40., "Rename topic uses the same compact Name field");
        h.app.apply(Action::CancelModal).unwrap();
        h.app.apply(Action::DeleteProject("p24".into())).unwrap(); h.frame();
        h.click_dialog("Continue…");
        assert_eq!(h.app.root.dialog.as_ref().unwrap().topic_key(), Some(("delete-choice", "p24")));
        assert_eq!(h.app.root.dialog.as_ref().unwrap().buttons().len(),3);
        h.dump(if mobile {"projects-phone-delete.png"} else {"projects-desktop-delete.png"});
        h.click_dialog("Cancel");
        assert_eq!(h.app.controller.account.projects.len(),25);
        assert_eq!(h.app.controller.account.sessions.len(),3,"Cancellation changes nothing");
        h.app.root.sidebar.projects.scroll.value=0.; h.app.apply(Action::SelectProject("general".into())).unwrap(); h.frame();
        h.dump(if mobile {"projects-phone-tabs.png"} else {"projects-desktop-tabs.png"});
    }
}

#[test]
fn topics_restore_the_last_open_chat_across_switches_restart_and_membership_changes() {
    for (size, mobile) in [((1000, 800), false), ((360, 720), true)] {
        let mut h = Harness::new(size, mobile);
        let mut older = h.app.controller.account.sessions[2].clone();
        older.id = "older".into();
        older.title = "Earlier topic chat".into();
        older.updated_at_ms = 2;
        h.app.controller.account.sessions.push(older);
        h.frame();
        let general = h.app.test_projects().iter().find(|(_, id)| id == "general").unwrap().0;
        assert_eq!(general.height, 34.);
        assert_eq!(h.app.root.sidebar.scroll.rect.y, general.y + general.height + 8.);

        // Upgrading an old account with only a global selected chat seeds its
        // General resume target on the first topic switch.
        assert!(h.app.controller.account.last_chat_by_project.is_empty());
        h.app.apply(Action::SelectProject("p24".into())).unwrap();
        h.frame();
        assert_eq!(h.app.controller.account.selected.as_deref(), if mobile { None } else { Some("three") }, "Only desktop opens the most recent chat");
        assert_eq!(h.app.root.legacy.show_chats, mobile, "Only desktop opens the remembered chat");
        h.app.apply(Action::Select("older".into())).unwrap();
        h.frame();
        h.app.apply(Action::SelectProject("general".into())).unwrap();
        h.frame();
        assert_eq!(h.app.controller.account.selected.as_deref(), if mobile { None } else { Some("demo") });
        h.app.apply(Action::SelectProject("p24".into())).unwrap();
        h.frame();
        assert_eq!(h.app.controller.account.selected.as_deref(), if mobile { None } else { Some("older") }, "Desktop remembers the last-open chat; mobile waits for a choice");
        let saved: crate::store::Account = h.app.controller.store.get(&h.app.controller.identity, "account").unwrap();
        assert_eq!(saved.last_chat_by_project.get("general").map(String::as_str), Some("demo"));
        assert_eq!(saved.last_chat_by_project.get("p24").map(String::as_str), Some("older"));
        let reopened = Store::open(h.app.controller.store.root.clone()).unwrap();
        let account: crate::store::Account = reopened.get(&h.app.controller.identity, "account").unwrap();
        assert_eq!(account.selected.as_deref(), if mobile { None } else { Some("older") });
        assert_eq!(account.last_chat_by_project, saved.last_chat_by_project);

        // A deleted or moved chat cannot be resurrected by stale local selection.
        let remaining = h.app.controller.account.sessions.iter().filter(|s| s.id != "older").cloned().collect();
        h.app.controller.message(ServerMessage::Sessions { sessions: remaining }).unwrap();
        assert!(!h.app.controller.account.last_chat_by_project.contains_key("p24"));
        h.app.apply(Action::SelectProject("general".into())).unwrap();
        h.app.apply(Action::SelectProject("p24".into())).unwrap();
        h.frame();
        assert_eq!(h.app.controller.account.selected.as_deref(), if mobile { None } else { Some("three") });
        let mut moved = h.app.controller.account.sessions.clone();
        moved.iter_mut().find(|s| s.id == "three").unwrap().project_id = "p1".into();
        h.app.controller.message(ServerMessage::Sessions { sessions: moved }).unwrap();
        h.app.apply(Action::SelectProject("p24".into())).unwrap();
        h.frame();
        assert!(h.app.controller.account.selected.is_none(), "truly empty topic has no ghost chat");
        assert!(h.app.root.legacy.show_chats);
    }
}

#[test]
fn topic_tabs_follow_contained_chat_activity_on_desktop_and_mobile() {
    for (size, mobile) in [((1000, 800), false), ((360, 720), true)] {
        let mut h = Harness::new(size, mobile);
        h.app.controller.message(ServerMessage::Projects { projects: vec![
            Project::general(), Project { id:"p1".into(), name:"First".into(), prompt:String::new(), revision:0 },
            Project { id:"p2".into(), name:"Second".into(), prompt:String::new(), revision:0 },
        ] }).unwrap();
        let mut sessions = h.app.controller.account.sessions.clone();
        for chat in &mut sessions {
            match chat.id.as_str() {
                "demo" => chat.updated_at_ms = 100,
                "two" => { chat.project_id = "p1".into(); chat.updated_at_ms = 10; }
                "three" => { chat.project_id = "p2".into(); chat.updated_at_ms = 20; }
                _ => unreachable!(),
            }
        }
        h.app.controller.message(ServerMessage::Sessions { sessions }).unwrap();
        h.frame();
        fn tabs(h: &Harness) -> Vec<String> {
            h.app.test_projects().iter().map(|(_, id)| id.clone()).collect()
        }
        assert_eq!(tabs(&h), ["general", "p2", "p1"]);
        h.app.controller.select("two").unwrap();
        h.app.tick(0.);
        if mobile { h.app.back(); }
        h.frame();
        assert_eq!(tabs(&h), ["general", "p2", "p1"], "selection does not move the topic tab");
        h.app.controller.draft("bump first topic".into()).unwrap();
        h.frame();
        assert_eq!(tabs(&h), ["general", "p1", "p2"], "tab hit regions and paint use the new order");
    }
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
        h.app.apply(Action::SelectProject("p24".into())).unwrap();
        h.frame();
        assert!(h.app.root.sidebar.projects.scroll.value > 0.);
        assert!(h.app.test_projects().iter().any(|(_, id)| id == "p24"));
        h.app.apply(Action::Select("three".into())).unwrap();
        h.app.tick(0.);
        if mobile { h.app.apply(Action::Back).unwrap(); }
        h.frame();
        h.app.controller.draft("bump this topic".into()).unwrap();
        h.frame();
        assert_eq!(h.app.controller.account.projects[1].id, "p24");
        assert!(h.app.test_projects().iter().any(|(_, id)| id == "p24"), "the selected tab remains visible after moving left (mobile={mobile}, scroll={}, tabs={:?})", h.app.root.sidebar.projects.scroll.value, h.app.test_projects().iter().map(|(_, id)| id.as_str()).collect::<Vec<_>>());
    }
}

#[test]
fn chat_activity_updates_the_visible_list_on_desktop_and_mobile() {
    for (size, mobile) in [((1000, 800), false), ((360, 720), true)] {
        let mut h = Harness::new(size, mobile);
        h.frame();
        assert_eq!(h.app.test_chats()[0].1, "demo");
        h.click(|a| matches!(a, Action::Select(id) if id == "two"));
        h.app.input("typing in the older chat");
        if mobile { h.app.apply(Action::Back).unwrap(); }
        h.frame();
        assert_eq!(h.app.test_chats()[0].1, "two");
        h.click(|a| matches!(a, Action::Select(id) if id == "demo"));
        h.app.key("ArrowRight", false, false);
        if mobile { h.app.apply(Action::Back).unwrap(); }
        h.frame();
        assert_eq!(h.app.test_chats()[0].1, "two", "selection and caret movement do not bump");
        h.click(|a| matches!(a, Action::New));
        let created = h.app.controller.account.selected.clone().unwrap();
        if mobile { h.app.apply(Action::Back).unwrap(); }
        h.frame();
        assert_eq!(h.app.test_chats()[0].1, created, "new chat appears at the top immediately, even offline");
    }
}
