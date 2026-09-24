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
        app.controller.account.projects.extend((1..=24).map(|i| Project { id:format!("p{i}"), name:format!("Project {i}"), prompt:format!("Exact prompt {i}\n"),revision:i }));
        app.controller.account.projects[1].name = "Tau development".into();
        app.controller.account.projects[2].name = "Research".into();
        app.controller.account.sessions[2].project_id = "p24".into();
        app.resize(size,1.,Vec2::new(0.,0.)); app.tick(0.); app.show_chats = mobile;
        Self { app,ctx,_root:root }
    }
    fn frame(&mut self) { self.app.tick(0.); self.app.frame(&self.ctx,self.ctx.view()); }
    fn click(&mut self, pred: impl Fn(&Action)->bool) {
        let r = self.app.hits.iter().find(|h| pred(&h.action)).expect("visible action").rect;
        let p = Vec2::new(r.x+r.width/2.,r.y+r.height/2.);
        self.app.press(1,p,self.app.mobile); self.app.release(1,p); self.frame();
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
        assert!(h.app.controller.project_unread("p24"));
        assert_eq!(h.app.chat_areas.iter().filter(|(r,_)| contains(h.app.list_rect, Vec2::new(r.x+1.,r.y+1.))).count(),2,"Only General's chats appear");
        assert!(!h.app.hits.iter().any(|hit| matches!(hit.action,Action::NewProject)),"The plus belongs at the scrolling end");
        let point = Vec2::new(100.,160.);
        h.app.wheel(300.,false,point);
        assert_eq!(h.app.wheel.as_ref().unwrap().lane,Lane::Projects);
        h.app.wheel.as_mut().unwrap().last = Instant::now()-std::time::Duration::from_secs(1);
        h.frame(); assert!(h.app.project_scroll>0.); assert_eq!(h.app.list_scroll,0.);
        h.app.wheel(100.,true,point); assert_eq!(h.app.wheel.as_ref().unwrap().lane,Lane::Projects);
        h.app.press(4,point,true); h.app.motion(4,Vec2::new(35.,161.));
        assert!(h.app.pointer.as_ref().unwrap().dragged); h.app.release(4,Vec2::new(35.,161.));
        assert!(h.app.context_menu.is_none(),"Swiping is not a long press or a tab selection");
        h.app.project_velocity=0.; h.app.apply(Action::SelectProject("p24".into())).unwrap(); h.frame();
        assert_eq!(h.app.chat_areas.iter().map(|(_,id)| id.as_str()).collect::<Vec<_>>(),vec!["three"]);
        assert!(h.app.project_areas.iter().any(|(_,id)| id=="p24"),"Selected tab auto-reveals");
        assert!(h.app.controller.project_unread("p24"),"Switching tabs does not read the chat");
        h.click(|a| matches!(a,Action::Select(id) if id=="three"));
        assert!(!h.app.controller.project_unread("p24"));
        h.app.apply(Action::SelectProject("general".into())).unwrap(); h.frame();
        h.app.controller.select("demo").unwrap(); h.app.tick(0.); h.app.show_chats=mobile; h.frame();
        let target = h.app.chat_areas.iter().find(|(_,id)| id=="two").unwrap().0;
        let point=Vec2::new(target.x+60.,target.y+20.);
        if mobile {
            h.app.press(2,point,true); h.app.pointer.as_mut().unwrap().started=Instant::now()-std::time::Duration::from_millis(500);
            h.frame(); assert!(h.app.pointer.is_none(),"Hold opens before lift");
            h.app.release(2,point);
        } else { h.app.context_at(point); h.frame(); }
        assert_eq!(h.app.controller.account.selected.as_deref(),Some("demo"));
        assert_eq!(h.app.context_menu.as_ref().unwrap().chat.as_deref(),Some("two"));
        h.click(|a| matches!(a,Action::MoveMenu(id) if id=="two"));
        assert!(h.app.context_menu.as_ref().unwrap().parent.is_some());
        h.dump(if mobile {"projects-phone-menu.png"} else {"projects-desktop-menu.png"});
        for _ in 0..24 { h.app.key("ArrowDown",false,false); }
        h.frame();
        assert!(h.app.context_menu.as_ref().unwrap().scroll>0.);
        assert!(h.app.hits.iter().any(|hit| matches!(&hit.action,Action::MoveChat(chat,project) if chat=="two" && project=="p24")),"Every project is reachable in the clipped submenu");
        assert!(h.app.hits.iter().all(|hit| hit.rect.y>=0. && hit.rect.y+hit.rect.height<=size.1 as f32));
        h.app.key("ArrowLeft",false,false); h.frame(); assert!(h.app.context_menu.as_ref().unwrap().parent.is_none());
        h.app.key("Escape",false,false); h.frame();
        let general_tab=h.app.project_areas.iter().find(|(_,id)| id=="general").unwrap().0;
        let point=Vec2::new(general_tab.x+20.,general_tab.y+20.);
        h.app.context_at(point); h.frame();
        assert_eq!(h.app.context_menu.as_ref().unwrap().options.len(),1,"General is permanent but its prompt is editable");
        h.app.key("Escape",false,false);
        h.app.project_scroll=h.app.max_project_scroll; h.frame();
        h.click(|a| matches!(a,Action::NewProject));
        assert!(matches!(h.app.modal.as_ref().unwrap().kind,ModalKind::NewProject(_)));
        h.app.input("New work");
        h.app.focus=Some(Some(1)); h.app.input("  Exact\ncontext\n"); h.frame();
        assert_eq!(h.app.modal.as_ref().unwrap().fields[1].1.value,"  Exact\ncontext\n");
        h.dump(if mobile {"projects-phone-new.png"} else {"projects-desktop-new.png"});
        h.click(|a| matches!(a,Action::CancelModal));
        h.app.apply(Action::DeleteProject("p24".into())).unwrap(); h.frame();
        h.click(|a| matches!(a,Action::Confirm));
        assert!(matches!(h.app.modal.as_ref().unwrap().kind,ModalKind::DeleteProjectChoice(ref p) if p.id=="p24"));
        assert_eq!(h.app.modal.as_ref().unwrap().options.len(),3);
        h.dump(if mobile {"projects-phone-delete.png"} else {"projects-desktop-delete.png"});
        h.click(|a| matches!(a,Action::CancelModal));
        assert_eq!(h.app.controller.account.projects.len(),25);
        assert_eq!(h.app.controller.account.sessions.len(),3,"Cancellation changes nothing");
        h.app.project_scroll=0.; h.app.apply(Action::SelectProject("general".into())).unwrap(); h.frame();
        h.dump(if mobile {"projects-phone-tabs.png"} else {"projects-desktop-tabs.png"});
    }
}
