//! Exercise the real hit dispatch and save lifecycle without invoking OS apps or a daemon.
use super::*;
use tau_protocol::blocks::{BlockHeader, BlockKind};
use super::download_render_tests::{Case, cases, install, controls, panel, save};
use chad::{Config, HeadlessCtx};
use std::{sync::Arc, time::Duration};

fn fixture(size: (u32, u32), mobile: bool) -> (App, HeadlessCtx, tempfile::TempDir) {
    let root = tempfile::tempdir().unwrap();
    let ctx = HeadlessCtx::new(&Config { size, device_limits: crate::desktop::limits(), ..Default::default() }).unwrap();
    let mut app = App::new(&ctx, Store::open(root.path().into()).unwrap(), Arc::new(|| {}), mobile).unwrap();
    app.back(); crate::demo::populate(&mut app.controller).unwrap();
    app.resize(size, 1., Vec2::new(0.,0.)); app.tick(0.);
    (app, ctx, root)
}
fn event(app: &App, case: &Case, file: ChatAttachment, order: u64) -> Event {
    let mut event = app.controller.chats["demo"].feed.events.values().next().unwrap().clone();
    event.id = case.id.clone(); event.entry_id = case.id.clone(); event.order = order;
    event.role = EventRole::Assistant; event.kind = EventKind::Text; event.phase = EventPhase::Saved;
    event.text.clear(); event.attachment = Some(file); event.timestamp_ms = Some(1_790_510_400_000);
    event
}
fn verified(app: &mut App, case: &Case) {
    use sha2::{Digest, Sha256};
    let path = app.controller.attachment_path("demo", &case.id);
    let bytes = std::fs::read(&path).unwrap();
    let cache = app.controller.store.root.join("blocks").join(format!("{:x}.sqlite3", Sha256::digest(app.controller.identity.as_bytes())));
    let mut db = rusqlite::Connection::open(cache).unwrap();
    let lineage = tau_blocks::cursor(&db).unwrap().lineage;
    let tx = db.transaction().unwrap();
    tau_blocks::cache_header(&tx, "demo", &BlockHeader { id: format!("file:{}", case.id), parent: None, order: 0,
        kind: if case.image { BlockKind::Image } else { BlockKind::File },
        meta: serde_json::json!({"sha256": format!("{:x}", Sha256::digest(&bytes))}), version: 1, revision: 1,
        length: bytes.len() as u64, sealed: true }).unwrap();
    tx.commit().unwrap();
    app.controller.store.bind_source(&app.controller.identity, &lineage).unwrap();
    app.controller.account.source_lineage = Some(lineage);
}
fn paint(app: &mut App, ctx: &HeadlessCtx, case: &Case, file: &ChatAttachment) {
    let layer = panel(app, ctx, case, file, Interaction::default(), Rect::new(0.,0.,ctx.size().0 as f32,ctx.size().1 as f32));
    app.renderer.draw(ctx, ctx.view(), &[layer]);
}
fn tap(app: &mut App, index: usize, touch: bool) {
    let r = controls(app)[index].rect;
    let point = Vec2::new(r.x+r.width/2.,r.y+r.height/2.);
    app.press(7, point, touch); app.release(7, point);
}

#[test]
fn cached_large_file_saves_once_retries_failure_and_survives_restart_then_missing_copy() {
    let (mut app, ctx, root) = fixture((552,420), false);
    let case = cases().into_iter().find(|c| c.id=="14-cached").unwrap();
    let file = install(&mut app, &case);
    let path = app.controller.attachment_path("demo", &case.id);
    // Larger than the image limit: this exact cached-file Save used to fail at 10 MB.
    std::fs::File::create(&path).unwrap().set_len(12*1024*1024).unwrap();
    verified(&mut app, &case);
    let item = event(&app, &case, file.clone(), 5);
    app.controller.chats.get_mut("demo").unwrap().feed.events.insert(5, item);
    let key = Controller::download_key("demo", &case.id);
    paint(&mut app, &ctx, &case, &file); tap(&mut app, 0, false);
    assert!(matches!(&app.platform[..], [PlatformAction::SaveDownload { key:k, source, .. }] if k==&key && source==&path));
    paint(&mut app, &ctx, &case, &file);
    assert!(matches!(controls(&app)[0].action, Action::Noop));
    tap(&mut app,0,false); assert_eq!(app.platform.len(),1,"saving cannot queue a duplicate export");
    app.platform.clear(); app.complete_save(&key, Err("Disk full".into()));
    paint(&mut app, &ctx, &case, &file);
    assert!(app.info_areas.iter().any(|(_,info)| matches!(info, Info::Attachment(_,title,_) if title=="Retry")));
    tap(&mut app,0,false);
    assert_eq!(app.platform.len(),1);
    let saved = crate::downloads::save_into(&root.path().join("user-downloads"), &path, &file.file_name).unwrap();
    app.platform.clear(); app.complete_save(&key, Ok(saved.clone())); app.controller.notice=None;
    paint(&mut app, &ctx, &case, &file); tap(&mut app,0,false); tap(&mut app,1,false);
    assert!(matches!(&app.platform[..], [PlatformAction::UseDownload(_,SavedAction::Open,_), PlatformAction::UseDownload(_,SavedAction::Show,_)]));
    assert!(!app.export_errors.contains_key(&key));
    drop(app);
    let mut app = App::new(&ctx,Store::open(root.path().into()).unwrap(),Arc::new(||{}),false).unwrap();
    app.back(); crate::demo::populate(&mut app.controller).unwrap(); app.resize(ctx.size(),1.,Vec2::new(0.,0.));
    paint(&mut app,&ctx,&case,&file);
    assert!(matches!(controls(&app)[0].action,Action::UseSaved(_,_,SavedAction::Open)),"saved copy survives restart");
    std::fs::remove_file(&saved.reference).unwrap();
    paint(&mut app,&ctx,&case,&file);
    assert!(matches!(controls(&app)[0].action,Action::SaveAttachment(..)),"missing copy can be saved again from cache");
    assert!(app.controller.saved_download("demo",&case.id).is_none());
    std::fs::remove_file(path).unwrap(); paint(&mut app,&ctx,&case,&file);
    assert!(matches!(controls(&app)[0].action,Action::Attachment(_,_,_,false)),"evicted cache returns to Download");
}

#[test]
fn image_view_save_and_touch_labels_do_not_confuse_actions_or_trigger_on_long_press() {
    let (mut app,ctx,_root) = fixture((360,720),true);
    let case=cases().into_iter().find(|c|c.id=="25-image-cached").unwrap();
    let file=install(&mut app,&case); verified(&mut app,&case);
    let item=event(&app,&case,file.clone(),5);
    app.controller.chats.get_mut("demo").unwrap().feed.events.insert(5,item);
    paint(&mut app,&ctx,&case,&file);
    let view=controls(&app)[0].rect;
    let point=Vec2::new(view.x+22.,view.y+22.);
    app.press(1,point,true);
    app.pointer.as_mut().unwrap().started=Instant::now()-Duration::from_millis(500);
    app.tick(0.); app.release(1,point);
    assert!(app.info_tip.pinned);
    assert!(matches!(&app.info_target,Info::Attachment(_,title,_) if title=="View image"));
    assert!(app.viewer.is_none() && app.platform.is_empty(),"a held icon shows its label, never acts");
    app.info_tip=Tooltip::default(); paint(&mut app,&ctx,&case,&file);
    tap(&mut app,0,true); assert!(app.viewer.is_some()); assert!(app.platform.is_empty());
    app.back(); paint(&mut app,&ctx,&case,&file);
    tap(&mut app,1,true);
    assert!(matches!(&app.platform[..],[PlatformAction::SaveDownload {..}]));
    let key=Controller::download_key("demo",&case.id);
    app.platform.clear(); app.complete_save(&key,Err("Permission denied".into()));
    paint(&mut app,&ctx,&case,&file);
    assert!(matches!(controls(&app)[0].action,Action::SaveAttachment(..)),"retry resumes saving, not just viewing");
    assert!(matches!(controls(&app)[1].action,Action::Attachment(_,_,_,true)),"save failure keeps View");
    tap(&mut app,0,true); assert!(matches!(&app.platform[..],[PlatformAction::SaveDownload {..}]));
}

#[test]
fn actual_chat_sidebar_and_phone_use_shared_geometry_and_independent_tooltip_anchors() {
    for (size,mobile,name) in [((1280,900),false,"desktop"),((360,900),true,"phone")] {
        let (mut app,ctx,_root)=fixture(size,mobile);
        let mut events=vec![];
        for case in cases().into_iter().filter(|c|["07-progress","18-saved-zip","25-image-cached"].contains(&c.id.as_str())) {
            let file=install(&mut app,&case);
            events.push(event(&app,&case,file,events.len() as u64));
        }
        app.controller.message(ServerMessage::TranscriptSnapshot {session_id:"demo".into(),snapshot:TranscriptSnapshot {
            generation:"demo".into(),sequence:1,events,queue:QueueState::default(),before:None,delivered:vec![]}}).unwrap();
        app.controller.account.sessions[0].title="Inline file downloads".into();
        app.show_chats=false; app.tick(0.); app.frame(&ctx,ctx.view());
        save(&ctx,&format!("context-{name}-chat"));
        app.show_attachments=true; app.frame(&ctx,ctx.view());
        save(&ctx,&format!("context-{name}-attachments"));
        let (rect,info)=app.info_areas.iter().find(|(_,info)| matches!(info,Info::Attachment(key,title,_)
            if key.starts_with("attachments:") && title=="Save to Downloads")).unwrap().clone();
        app.info_target=info; app.info_tip.region=rect; app.info_tip.pinned=true; app.info_tip.progress=1.;
        app.frame(&ctx,ctx.view());
        assert_eq!(app.info_tip.region,rect,"duplicate file in chat must not steal sidebar tooltip anchor");
        save(&ctx,&format!("context-{name}-tooltip"));
        // A changing byte count keeps the same pinned details card alive and updates its content.
        app.info_tip=Tooltip::default();
        let (rect,info)=app.info_areas.iter().find(|(_,info)| matches!(info,Info::Attachment(key,_,_)
            if key.starts_with("attachments:") && key.contains("07-progress") && key.ends_with(":details"))).unwrap().clone();
        app.info_target=info; app.info_tip.region=rect; app.info_tip.pinned=true; app.info_tip.progress=1.;
        let key=Controller::download_key("demo","07-progress");
        app.controller.downloads.get_mut(&key).unwrap().status.transferred=6*1024*1024;
        app.frame(&ctx,ctx.view());
        assert!(app.info_tip.pinned && app.info_tip.content.text.contains("50%"));
        app.show_attachments=false; app.frame(&ctx,ctx.view());
        assert!(!app.info_tip.pinned,"hidden attachment pane must dismiss its tooltip");
    }
}

#[test]
fn saved_zip_actions_dispatch_open_show_extract_to_the_correct_file() {
    let (mut app,ctx,_root)=fixture((320,420),false);
    let mut case=cases().into_iter().find(|c|c.id=="18-saved-zip").unwrap();
    case.name=Some("source-code.ZIP".into());
    let file=install(&mut app,&case); paint(&mut app,&ctx,&case,&file);
    for index in 0..3 { tap(&mut app,index,false); }
    assert!(matches!(&app.platform[..],[PlatformAction::UseDownload(_,SavedAction::Open,_),
        PlatformAction::UseDownload(_,SavedAction::Show,_),PlatformAction::UseDownload(_,SavedAction::Extract,_)]));
    for action in &app.platform {
        let PlatformAction::UseDownload(saved,_,target)=action else {panic!("unexpected action")};
        assert_eq!(target.session,"demo"); assert_eq!(target.entry,case.id);
        assert!(saved.reference.ends_with(&format!("saved-{}",case.id)));
    }
}
