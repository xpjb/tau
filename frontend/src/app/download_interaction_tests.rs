//! Exercise the real hit dispatch and save lifecycle without invoking OS apps or a daemon.
use super::*;
use tau_protocol::blocks::{BlockHeader, BlockKind};
use super::download_render_tests::{Case, cases, install, controls, save, hints};
use chad::{Config, HeadlessCtx};
use std::{sync::Arc, time::Duration};

pub(super) fn fixture(size: (u32, u32), mobile: bool) -> (App, HeadlessCtx, tempfile::TempDir) {
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
pub(super) fn paint(app: &mut App, ctx: &HeadlessCtx, case: &Case, file: &ChatAttachment) {
    if !app.controller.chats["demo"].feed.events.values().any(|e| e.entry_id == case.id) {
        let item = event(app, case, file.clone(), 0);
        let chat = app.controller.chats.get_mut("demo").unwrap();
        chat.feed.events.clear();
        chat.feed.events.insert(0, item);
    }
    app.root.workspace.show_chats = false;
    app.root.workspace.attachments.show = true;
    app.tick(0.);
    app.frame(ctx, ctx.view());
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
    assert!(matches!(&app.services.platform[..], [PlatformAction::SaveDownload { key:k, source, .. }] if k==&key && source==&path));
    paint(&mut app, &ctx, &case, &file);
    assert!(matches!(controls(&app)[0].action, ui::CardChoice::Noop));
    tap(&mut app,0,false); assert_eq!(app.services.platform.len(),1,"saving cannot queue a duplicate export");
    app.services.platform.clear(); app.complete_save(&key, Err("Disk full".into()));
    paint(&mut app, &ctx, &case, &file);
    assert!(hints(&app).iter().any(|(_,info)| matches!(info, Info::Attachment(_,title,_) if title=="Retry")));
    tap(&mut app,0,false);
    assert_eq!(app.services.platform.len(),1);
    let saved = crate::downloads::save_into(&root.path().join("user-downloads"), &path, &file.file_name).unwrap();
    app.services.platform.clear(); app.complete_save(&key, Ok(saved.clone())); app.controller.notice=None;
    paint(&mut app, &ctx, &case, &file); tap(&mut app,0,false); tap(&mut app,1,false);
    assert!(matches!(&app.services.platform[..], [PlatformAction::UseDownload(_,SavedAction::Open,_), PlatformAction::UseDownload(_,SavedAction::Show,_)]));
    assert!(!app.services.transfers.export_errors.contains_key(&key));
    drop(app);
    let mut app = App::new(&ctx,Store::open(root.path().into()).unwrap(),Arc::new(||{}),false).unwrap();
    app.back(); crate::demo::populate(&mut app.controller).unwrap(); app.resize(ctx.size(),1.,Vec2::new(0.,0.));
    paint(&mut app,&ctx,&case,&file);
    assert!(matches!(controls(&app)[0].action,ui::CardChoice::UseSaved(_,_,SavedAction::Open)),"saved copy survives restart");
    std::fs::remove_file(&saved.reference).unwrap();
    paint(&mut app,&ctx,&case,&file);
    assert!(matches!(controls(&app)[0].action,ui::CardChoice::SaveAttachment(..)),"missing copy can be saved again from cache");
    assert!(app.controller.saved_download("demo",&case.id).is_none());
    std::fs::remove_file(path).unwrap(); paint(&mut app,&ctx,&case,&file);
    assert!(matches!(controls(&app)[0].action,ui::CardChoice::Attachment(_,_,_,false)),"evicted cache returns to Download");
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
    app.ui.capture.as_mut().unwrap().started=Instant::now()-Duration::from_millis(500);
    app.tick(0.); app.release(1,point);
    assert!(app.root.tooltips.info.pinned);
    assert!(matches!(&app.root.tooltips.target,Info::Attachment(_,title,_) if title=="View image"));
    assert!(app.root.viewer.is_none() && app.services.platform.is_empty(),"a held control shows its details, never acts");
    app.root.tooltips.info=Tooltip::default(); paint(&mut app,&ctx,&case,&file);
    tap(&mut app,0,true); assert!(app.root.viewer.is_some()); assert!(app.services.platform.is_empty());
    app.back(); paint(&mut app,&ctx,&case,&file);
    tap(&mut app,1,true);
    assert!(matches!(&app.services.platform[..],[PlatformAction::SaveDownload {..}]));
    let key=Controller::download_key("demo",&case.id);
    app.services.platform.clear(); app.complete_save(&key,Err("Permission denied".into()));
    paint(&mut app,&ctx,&case,&file);
    assert!(matches!(controls(&app)[0].action,ui::CardChoice::SaveAttachment(..)),"retry resumes saving, not just viewing");
    assert!(matches!(controls(&app)[1].action,ui::CardChoice::Attachment(_,_,_,true)),"save failure keeps View");
    tap(&mut app,0,true); assert!(matches!(&app.services.platform[..],[PlatformAction::SaveDownload {..}]));
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
        app.root.workspace.show_chats=false; app.tick(0.); app.frame(&ctx,ctx.view());
        save(&ctx,&format!("context-{name}-chat"));
        app.root.workspace.attachments.show=true; app.frame(&ctx,ctx.view());
        save(&ctx,&format!("context-{name}-attachments"));
        let (rect,info)=hints(&app).iter().find(|(_,info)| matches!(info,Info::Attachment(key,title,_)
            if key.starts_with("attachments:") && title=="Save to Downloads")).unwrap().clone();
        app.root.tooltips.target=info; app.root.tooltips.info.region=rect; app.root.tooltips.info.pinned=true; app.root.tooltips.info.progress=1.;
        app.frame(&ctx,ctx.view());
        assert_eq!(app.root.tooltips.info.region,rect,"duplicate file in chat must not steal sidebar tooltip anchor");
        save(&ctx,&format!("context-{name}-tooltip"));
        assert!(hints(&app).iter().all(|(_,info)| !matches!(info,Info::Attachment(key,_,_)
            if key.ends_with(":details") || key.ends_with(":caption"))), "Card text must not repeat itself in a tooltip");
        app.root.workspace.attachments.show=false; app.frame(&ctx,ctx.view());
        assert!(!app.root.tooltips.info.pinned,"hidden attachment pane must dismiss its tooltip");
    }
}

#[test]
fn saved_zip_text_actions_dispatch_open_show_extract_to_the_correct_file() {
    let (mut app,ctx,_root)=fixture((320,420),false);
    let mut case=cases().into_iter().find(|c|c.id=="18-saved-zip").unwrap();
    case.name=Some("source-code.ZIP".into());
    let file=install(&mut app,&case); paint(&mut app,&ctx,&case,&file);
    for index in 0..3 { tap(&mut app,index,false); }
    assert!(matches!(&app.services.platform[..],[PlatformAction::UseDownload(_,SavedAction::Open,_),
        PlatformAction::UseDownload(_,SavedAction::Show,_),PlatformAction::UseDownload(_,SavedAction::Extract,_)]));
    for action in &app.services.platform {
        let PlatformAction::UseDownload(saved,_,target)=action else {panic!("unexpected action")};
        assert_eq!(target.session,"demo"); assert_eq!(target.entry,case.id);
        assert!(saved.reference.ends_with(&format!("saved-{}",case.id)));
    }
}

#[test]
fn extract_button_has_no_hover_or_context_tooltip() {
    let (mut app, ctx, _root) = fixture((552, 420), false);
    let case = cases().into_iter().find(|c| c.id == "18-saved-zip").unwrap();
    let file = install(&mut app, &case);
    paint(&mut app, &ctx, &case, &file);
    let r = controls(&app)[2].rect;
    let point = Vec2::new(r.x + r.width / 2., r.y + r.height / 2.);
    assert!(!hints(&app).iter().any(|(r, _)| contains(*r, point)));
    app.hover(Some(point)); app.tick(1.);
    app.context_at(point);
    assert!(app.ui.hint.is_none());
    assert!(!app.root.tooltips.info.pinned && app.root.tooltips.info.progress == 0.);
    assert!(app.actions().is_empty());
}

#[test]
fn extract_clicks_queue_only_one_operation_even_before_repaint() {
    let (mut app, ctx, _root) = fixture((552, 420), false);
    let case = cases().into_iter().find(|c| c.id == "18-saved-zip").unwrap();
    let file = install(&mut app, &case);
    paint(&mut app, &ctx, &case, &file);
    for _ in 0..4 { tap(&mut app, 2, false); }
    assert_eq!(app.actions().len(), 1, "Rapid clicks cannot start multiple extraction/open workers");
    // Draining the platform queue does not end the asynchronous operation.
    tap(&mut app, 2, false);
    assert!(app.actions().is_empty());
}

#[test]
fn extract_busy_state_clears_on_failure_and_success_and_allows_retry() {
    let (mut app, ctx, _root) = fixture((320, 420), false);
    let case = cases().into_iter().find(|c| c.id == "18-saved-zip").unwrap();
    let file = install(&mut app, &case);
    let target = app.export_target("demo", &case.id);
    for result in [Err("Disk full".into()), Ok(())] {
        paint(&mut app, &ctx, &case, &file);
        assert!(matches!(controls(&app)[2].action, ui::CardChoice::UseSaved(_, _, SavedAction::Extract)));
        tap(&mut app, 2, false);
        assert!(matches!(&app.actions()[..], [PlatformAction::UseDownload(_, SavedAction::Extract, t)] if t == &target));
        paint(&mut app, &ctx, &case, &file);
        assert!(matches!(controls(&app)[2].action, ui::CardChoice::Noop));
        for _ in 0..3 { tap(&mut app, 2, false); }
        assert!(app.actions().is_empty());
        save(&ctx, "extract-busy-sidebar");
        app.complete_extraction(&target, result.clone());
        assert!(!app.services.transfers.extracting_downloads.contains(&target));
        assert_eq!(app.controller.notice.as_deref(), result.as_ref().err().map(String::as_str));
        app.controller.notice = None;
    }
    paint(&mut app, &ctx, &case, &file);
    assert!(matches!(controls(&app)[2].action, ui::CardChoice::UseSaved(_, _, SavedAction::Extract)));
}

#[test]
fn extraction_busy_state_is_shared_by_chat_and_sidebar_and_survives_navigation() {
    let (mut app, ctx, _root) = fixture((1280, 900), false);
    let case = cases().into_iter().find(|c| c.id == "18-saved-zip").unwrap();
    let file = install(&mut app, &case);
    let item = event(&app, &case, file, 0);
    app.controller.message(ServerMessage::TranscriptSnapshot { session_id: "demo".into(), snapshot: TranscriptSnapshot {
        generation: "demo".into(), sequence: 1, events: vec![item], queue: QueueState::default(), before: None, delivered: vec![],
    }}).unwrap();
    app.root.workspace.attachments.show = true;
    app.tick(0.); app.frame(&ctx, ctx.view());
    let extract_controls = |app: &App| {
        app.root.workspace.chat.transcript.rows.iter().filter_map(|r| r.attachment.as_ref())
            .chain(app.root.workspace.attachments.cards.cards.values())
            .flat_map(|c| c.controls.items.iter())
            .filter(|(_, button, _)| matches!(button.label.as_str(), "Extract" | "Extracting…"))
            .filter_map(|(_, b, _)| b.control.rect.map(|r| (r, b.control.enabled, b.control.info.is_some())))
            .collect::<Vec<_>>()
    };
    let buttons = extract_controls(&app);
    assert_eq!(buttons.len(), 2, "The ZIP is visible in both surfaces");
    for (r, enabled, tooltip) in buttons {
        assert!(enabled && !tooltip);
        let point = Vec2::new(r.x + r.width / 2., r.y + r.height / 2.);
        app.press(1, point, false); app.release(1, point);
    }
    assert_eq!(app.actions().len(), 1, "Even stale controls on different surfaces share the same claim");
    app.frame(&ctx, ctx.view());
    assert!(extract_controls(&app).iter().all(|(_, enabled, tooltip)| !enabled && !tooltip));
    save(&ctx, "extract-busy-chat-and-sidebar");
    // Destroy/rebuild the attachment pane while the worker is still running.
    app.root.workspace.attachments.show = false; app.frame(&ctx, ctx.view());
    app.root.workspace.attachments.show = true; app.frame(&ctx, ctx.view());
    assert_eq!(extract_controls(&app).len(), 2);
    assert!(extract_controls(&app).iter().all(|(_, enabled, _)| !enabled));
    app.complete_extraction(&app.export_target("demo", &case.id), Ok(()));
    app.frame(&ctx, ctx.view());
    assert!(extract_controls(&app).iter().all(|(_, enabled, tooltip)| *enabled && !tooltip));
}

#[test]
fn extraction_completion_cannot_clear_another_accounts_or_sources_busy_state() {
    let (mut app, ctx, _root) = fixture((552, 420), false);
    let case = cases().into_iter().find(|c| c.id == "18-saved-zip").unwrap();
    let file = install(&mut app, &case);
    let saved = app.controller.saved_download("demo", &case.id).unwrap();
    let mut targets = vec![];
    for (identity, lineage) in [(app.controller.identity.clone(), ""), ("other-account".into(), ""), ("other-account".into(), "other-source")] {
        app.controller.identity = identity.clone();
        app.controller.account.source_lineage = Some(lineage.into());
        app.controller.record_download(&identity, lineage, "demo", &case.id, saved.clone()).unwrap();
        paint(&mut app, &ctx, &case, &file);
        tap(&mut app, 2, false);
        assert_eq!(app.actions().len(), 1, "A distinct account/source is not blocked by an old job");
        targets.push(app.export_target("demo", &case.id));
    }
    assert_eq!(app.services.transfers.extracting_downloads.len(), 3);
    app.controller.notice = None;
    for target in &targets[..2] {
        app.complete_extraction(target, Err("Old source failure".into()));
        assert!(app.controller.notice.is_none(), "Old failures must not surface in another account/source");
    }
    assert!(app.services.transfers.extracting_downloads.contains(&targets[2]));
    app.complete_extraction(&targets[2], Err("Current source failure".into()));
    assert!(app.services.transfers.extracting_downloads.is_empty());
    assert_eq!(app.controller.notice.as_deref(), Some("Current source failure"));
}
