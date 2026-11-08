use super::*;
use crate::{app::{App, ui::{Dialog, DialogSpec}}, store::Store};
use chad::{Config, HeadlessCtx};
use sanscale::Vec2;
use std::sync::Arc;
fn center(r: Rect) -> Vec2 { Vec2::new(r.x + r.width/2., r.y + r.height/2.) }
fn setup(mobile: bool) -> (tempfile::TempDir, HeadlessCtx, App) {
    let dir=tempfile::tempdir().unwrap();let size=if mobile {(360,740)} else {(1000,800)};
    let ctx=HeadlessCtx::new(&Config {size,device_limits:crate::desktop::limits(),..Default::default()}).unwrap();
    let mut app=App::new(&ctx,Store::open(dir.path().into()).unwrap(),Arc::new(||{}),mobile).unwrap();
    app.back();crate::demo::populate(&mut app.controller).unwrap();app.resize(size,1.,Vec2::new(0.,0.));app.tick(0.);app.root.workspace.show_chats=false;
    (dir,ctx,app)
}
fn render(app: &mut App, ctx: &HeadlessCtx) { app.tick(0.);app.frame(ctx,ctx.view()); }
fn dialog(app: &mut App) -> &mut CodexDialog {
    let Some(Dialog::CodexLogin(d))=app.root.dialog.as_mut() else {panic!("Sign-in dialog not open")};d
}
fn click(app: &mut App, label: &str) {
    let point=center(dialog(app).buttons().into_iter().find(|(s,_)|*s==label).unwrap().1);
    app.press(7,point,app.ui.mobile);app.release(7,point);
}
#[test]
fn codex_signin_prompt_renders_full_link_and_code_opens_browser_and_fences_source_changes() {
    for mobile in [false,true] {
        let (_dir,ctx,mut app)=setup(mobile);
        app.controller.message(tau_net::ServerMessage::CodexLoginRequired {session_id:"other-chat".into()}).unwrap();
        assert!(app.controller.codex_login_required.is_none());
        app.controller.message(tau_net::ServerMessage::CodexLoginRequired {session_id:"demo".into()}).unwrap();
        render(&mut app,&ctx);assert!(matches!(app.root.dialog,Some(Dialog::CodexLogin(_))));
        let d=dialog(&mut app);d.request=None;d.login_id=Some("device-fixture".into());
        d.state=Some(CodexLogin::Pending {login_id:"device-fixture".into(),user_code:"ABCD-EFGH".into(),verification_uri:"https://untrusted.invalid/not-used".into(),expires_at_ms:u64::MAX});
        render(&mut app,&ctx);
        let text=dialog(&mut app).text(false);let first=dialog(&mut app).buttons().into_iter().map(|(_,r)|r.y).reduce(f32::min).unwrap();
        let height=app.services.renderer.label_height(&text,if mobile {328.} else {620.},16.,false);
        assert!(height <= first-48.,"Sign-in instructions must fit without clipping: {height} / {}",first-48.);
        if let Some(path)=std::env::var_os("TAU_SIGNIN_PREVIEW_DIR") {
            std::fs::create_dir_all(&path).unwrap();let (w,h)=ctx.size();
            image::save_buffer(std::path::PathBuf::from(path).join(if mobile {"signin-mobile.png"} else {"signin-desktop.png"}),&ctx.read_rgba8().unwrap(),w,h,image::ColorType::Rgba8).unwrap();
        }
        app.actions();click(&mut app,"Copy code");assert!(app.actions().iter().any(|a|matches!(a,PlatformAction::Copy(s) if s=="ABCD-EFGH")));
        click(&mut app,"Open Codex sign-in");assert!(app.actions().iter().any(|a|matches!(a,PlatformAction::OpenUrl(s) if s==CODEX_LOGIN_URL)));
        dialog(&mut app).state=Some(CodexLogin::Complete);render(&mut app,&ctx);
        assert!(dialog(&mut app).buttons().iter().any(|(label,_)|*label=="Resume chat"));
        assert!(app.controller.account.pending_controls.is_empty(),"Approval must not replay an action");
        app.controller.identity="different-source".into();app.update_widgets(0.);assert!(app.root.dialog.is_none());
    }
}
#[test]
fn codex_signin_errors_wrap_and_right_click_copies_error_metadata_and_partial_output() {
    for partial in ["", "Partial assistant response"] {
        let (_dir,ctx,mut app)=setup(false);
        let mut event=app.controller.chats["demo"].feed.events[&3].clone();
        event.phase=tau_net::EventPhase::Saved;event.text=partial.into();
        let error="Shared Codex access needs its primary owner's refresh. Retry after that refresh, or sign into this beta independently with taud --login-codex";
        event.error_message=Some(error.into());event.stop_reason=Some("error".into());
        let expected=crate::feed::message_text(&event);
        app.controller.preview("demo",vec![event],Default::default(),None).unwrap();
        render(&mut app,&ctx);
        let row=app.root.workspace.chat.transcript.rows.iter().find(|r|r.key=="demo/event-3").unwrap();
        let key=row.key.clone();let point=center(row.control.rect.unwrap());
        assert!(app.services.renderer.messages[&key].source.contains("primary owner"));
        assert!(app.services.renderer.messages[&key].view.height>40.,"Error is wrapped content, not a clipped header");
        app.context_at(point);assert!(app.root.menu.as_ref().unwrap().options.iter().any(|(label,_)|label=="Sign in to Codex"));
        app.key("Enter",false,false);
        assert!(app.actions().iter().any(|a|matches!(a,PlatformAction::Copy(s) if s==&expected)));
        app.open_ui(DialogSpec::Operation(super::super::Operation::Link("tau:codex-login".into()))).unwrap();
        assert!(matches!(app.root.dialog,Some(Dialog::CodexLogin(_))));
    }
}
