use super::*;
use crate::app::{App, PlatformAction, SavedAction, Store};
use chad::{Config,HeadlessCtx};
use sanscale::Vec2;
use std::sync::Arc;

struct Harness {app:App,ctx:HeadlessCtx,_dir:tempfile::TempDir}
impl Harness {
    fn new()->Self {
        let dir=tempfile::tempdir().unwrap();let ctx=HeadlessCtx::new(&Config {size:(360,600),device_limits:crate::desktop::limits(),..Default::default()}).unwrap();
        let mut app=App::new(&ctx,Store::open(dir.path().into()).unwrap(),Arc::new(||{}),true).unwrap();
        app.back();crate::demo::populate(&mut app.controller).unwrap();app.tick(0.);app.root.legacy.show_chats=false;
        let template=app.controller.chats["demo"].feed.events.values().next().unwrap().clone();
        let path=dir.path().join("saved.txt");std::fs::write(&path,b"already saved").unwrap();
        let mut events=vec![];
        for order in 0..20 {
            let id=format!("entry-{order}");let mut event=template.clone();event.id=id.clone();event.entry_id=id.clone();event.order=order;
            event.text.clear();event.attachment=Some(ChatAttachment {source_path:None,kind:AttachmentKind::File,file_name:"saved.txt".into(),caption:None,size:Some(13)});
            app.controller.record_download(&app.controller.identity.clone(),"","demo",&id,crate::store::SavedDownload {reference:path.to_string_lossy().into(),location:"Downloads/Tau".into(),mime_type:"text/plain".into()}).unwrap();events.push(event);
        }
        app.controller.message(ServerMessage::TranscriptSnapshot {session_id:"demo".into(),snapshot:TranscriptSnapshot {generation:"nested".into(),sequence:1,events,queue:QueueState::default(),before:None,delivered:vec![]}}).unwrap();
        app.root.attachments.show=true;let mut h=Self {app,ctx,_dir:dir};h.frame();h
    }
    fn frame(&mut self){self.app.tick(0.);self.app.frame(&self.ctx,self.ctx.view());}
    fn open(&self)->(super::super::Target,Rect){
        self.app.root.attachments.cards.cards.values().flat_map(|card|card.controls.items.iter()).find_map(|(_,b,a)|matches!(a,CardChoice::UseSaved(_,entry,SavedAction::Open) if entry=="entry-19").then_some((b.control.target,b.control.rect.unwrap()))).unwrap()
    }
}
#[test]
fn nested_attachment_button_promotes_touch_to_its_scroll_parent_without_activation(){
    let mut h=Harness::new();let (target,rect)=h.open();let p=Vec2::new(rect.x+rect.width/2.,rect.y+rect.height/2.);
    h.app.press(10,p,true);assert_eq!(h.app.ui.capture.unwrap().target,target);
    h.app.motion(10,Vec2::new(p.x,p.y-45.));
    assert_eq!(h.app.ui.capture.unwrap().target,h.app.root.attachments.scroll.target);
    assert!(h.app.root.attachments.scroll.value>0.);assert_eq!(h.app.root.legacy.scroll,0.);
    h.app.release(10,p);assert!(h.app.actions().is_empty(),"Scroll takeover cancels the child activation even on a returning release");
    h.app.root.attachments.scroll.stop();h.app.root.attachments.scroll.value=0.;h.frame();
    let (_,rect)=h.open();let p=Vec2::new(rect.x+rect.width/2.,rect.y+rect.height/2.);
    h.app.press(11,p,true);h.app.release(11,p);
    assert!(matches!(&h.app.actions()[..],[PlatformAction::UseDownload(_,SavedAction::Open,target)] if target.entry=="entry-19"));
}
#[test]
fn nested_capture_is_clipped_and_cannot_activate_a_replaced_card_or_another_pointer(){
    let mut h=Harness::new();let (old,rect)=h.open();let p=Vec2::new(rect.x+rect.width/2.,rect.y+rect.height/2.);
    h.app.press(21,p,true);h.app.release(22,p);assert!(h.app.actions().is_empty());assert_eq!(h.app.ui.capture.unwrap().pointer,21);
    // Reconcile the captured node away between press and release.
    h.app.controller.chats.get_mut("demo").unwrap().feed.events.remove(&19);h.frame();
    assert!(h.app.ui.capture.is_none_or(|c|c.target!=old));h.app.release(21,p);assert!(h.app.actions().is_empty());
    h.app.root.attachments.scroll.value=60.;h.frame();
    let top=h.app.root.attachments.scroll.rect.y;
    let clipped=h.app.root.attachments.cards.cards.values().flat_map(|card|card.controls.items.iter()).find(|(_,b,_)|b.control.rect.is_some_and(|r|r.y<top));
    if let Some((_,b,_))=clipped {let r=b.control.rect.unwrap();let p=Vec2::new(r.x+r.width/2.,top-1.);assert!(!b.control.contains(p));}
    h.app.with_ui(|root,cx| root.attachments.handle_event(&Event::Cancel,cx));
    assert!(h.app.root.attachments.scroll.velocity==0.);
}
