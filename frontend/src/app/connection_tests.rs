//! Real GPU status card with injected heartbeat events, no live socket.
use super::*;
use chad::{Config, HeadlessCtx};
use std::{sync::Arc, time::Duration};

fn elapsed(text: &str, label: &str) -> u128 {
    text.lines()
        .find_map(|line| line.strip_prefix(label))
        .unwrap_or_else(|| panic!("missing {label} in {text}"))
        .strip_suffix("ms")
        .unwrap()
        .parse()
        .unwrap()
}

#[test]
fn connection_card_shows_live_ack_and_waiting_counters_but_leaves_unread_dot_alone() {
    let root = tempfile::tempdir().unwrap();
    let ctx = HeadlessCtx::new(&Config {
        size: (1000, 700),
        device_limits: crate::desktop::limits(),
        ..Default::default()
    })
    .unwrap();
    let mut app = App::new(
        &ctx,
        Store::open(root.path().into()).unwrap(),
        Arc::new(|| {}),
        false,
    )
    .unwrap();
    app.back();
    crate::demo::populate(&mut app.controller).unwrap();
    app.resize(ctx.size(), 1., Vec2::new(0., 0.));
    app.tick(0.);

    app.preview_connection(ConnectionPreview::Received);
    app.frame(&ctx, ctx.view());
    let received = ctx.read_rgba8().unwrap();
    assert!(
        app.info_tip
            .text
            .starts_with("min: 123ms\nmax: 420ms\nlatest: 123ms\nreceived: "),
        "{}",
        app.info_tip.text
    );
    assert!(elapsed(&app.info_tip.text, "received: ") >= 1234);
    assert!(!app.info_tip.text.contains("tau.example.invalid"));
    assert_eq!(app.controller.health.color(Instant::now()), 0x4ade80);

    app.controller
        .health
        .sent(Instant::now() - Duration::from_millis(1350));
    assert!(app.tick(0.), "waiting must repaint");
    app.frame(&ctx, ctx.view());
    let waiting = ctx.read_rgba8().unwrap();
    assert_ne!(received, waiting, "waiting must change the GPU frame");
    assert!(elapsed(&app.info_tip.text, "waiting: ") >= 1350);
    assert!(app.info_tip.text.contains("latest: 123ms\nwaiting: "), "pending probes are not acknowledged RTTs");
    assert_eq!(app.controller.health.color(Instant::now()), 0xfb923c);
    assert_eq!(app.info_tip.text.lines().count(), 4);

    // Pong: the live timer switches to age since receipt, not the previous send.
    app.controller
        .health
        .reply(Duration::from_millis(1350), Instant::now());
    assert!(app.tick(0.));
    app.frame(&ctx, ctx.view());
    assert!(
        app.info_tip
            .text
            .starts_with("min: 123ms\nmax: 1350ms\nlatest: 1350ms\nreceived: ")
    );
    let before = elapsed(&app.info_tip.text, "received: ");
    std::thread::sleep(Duration::from_millis(60));
    assert!(app.tick(0.), "received timer must continue while visible");
    app.frame(&ctx, ctx.view());
    assert!(elapsed(&app.info_tip.text, "received: ") > before);
    assert_eq!(app.controller.health.color(Instant::now()), 0xfb923c);
    app.controller
        .health
        .sent(Instant::now() - Duration::from_millis(21));
    app.controller
        .health
        .reply(Duration::from_millis(21), Instant::now());
    app.tick(0.);
    app.frame(&ctx, ctx.view());
    assert!(
        app.info_tip
            .text
            .starts_with("min: 21ms\nmax: 1350ms\nlatest: 21ms\nreceived: ")
    );
    assert_eq!(app.controller.health.color(Instant::now()), 0x4ade80);

    app.set_connection_visible(false); // Suspended Android / occluded desktop.
    assert!(!app.tick(0.), "hidden surface must not keep rendering");
    app.set_connection_visible(true);
    assert!(app.tick(0.));
    app.frame(&ctx, ctx.view());
    app.info_tip = Tooltip::default(); // Closed card: no 50ms redraw loop.
    assert!(app.tick(0.)); // One final frame to close the card.
    assert_eq!(app.counter_bucket, None);
    assert!(!app.tick(0.));

    // Socket loss does not rewrite the chat's last known Working label or unread marker.
    app.preview_connection(ConnectionPreview::Disconnected);
    app.frame(&ctx, ctx.view());
    assert!(
        app.info_tip
            .text
            .starts_with("Reconnecting…\nmin: 123ms\nmax: 420ms\nlatest: 123ms\nreceived: ")
    );
    assert!(app.info_tip.text.ends_with("\nPing timed out"));
    assert_eq!(app.controller.health.color(Instant::now()), 0xff5a5f);
    let disconnected = ctx.read_rgba8().unwrap();
    let pixel = |image: &[u8], x, y| {
        let offset = ((y * ctx.size().0 + x) * 4) as usize;
        image[offset..offset + 3].to_vec()
    };
    // Tau's dot stays solid in both states; only its color changes.
    assert_ne!(pixel(&received, 86, 39), pixel(&received, 95, 39));
    assert_ne!(pixel(&disconnected, 86, 39), pixel(&disconnected, 95, 39));
    assert_ne!(pixel(&received, 86, 39), pixel(&disconnected, 86, 39));

    app.preview_connection(ConnectionPreview::Unconfigured);
    app.frame(&ctx, ctx.view());
    assert_eq!(app.info_tip.text, "Offline\nmin: —\nmax: —\nlatest: —\nreceived: —");
    assert_eq!(app.controller.health.color(Instant::now()), 0xff5a5f);
}

#[test]
fn finished_reply_stays_unread_in_background_until_its_chat_is_visible_and_focused() {
    let root = tempfile::tempdir().unwrap();
    let ctx = HeadlessCtx::new(&Config {
        size: (1000, 700),
        device_limits: crate::desktop::limits(),
        ..Default::default()
    }).unwrap();
    let mut app = App::new(&ctx, Store::open(root.path().into()).unwrap(), Arc::new(|| {}), false).unwrap();
    app.back();
    crate::demo::populate(&mut app.controller).unwrap();
    app.resize(ctx.size(), 1., Vec2::new(0., 0.));
    app.tick(0.);
    app.controller.message(ServerMessage::Sessions { sessions: app.controller.account.sessions.clone() }).unwrap();
    assert!(!app.controller.unread(&app.controller.account.sessions[0]));

    app.window_focused = false;
    app.tick(0.);
    let mut sessions = app.controller.account.sessions.clone();
    sessions[0].updated_at_ms += 1;
    sessions[0].status = SessionStatus::Running;
    app.controller.message(ServerMessage::Sessions { sessions }).unwrap();
    let selected = &app.controller.account.sessions[0];
    assert!(app.controller.unread(selected), "background streaming must not mark the chat read");
    app.controller.message(ServerMessage::SessionState {
        session_id: selected.id.clone(), revision:1, restore_review:None, status: SessionStatus::Idle, detail: None, context_usage: None,
    }).unwrap();
    app.tick(0.);
    assert!(app.controller.unread(&app.controller.account.sessions[0]), "completion must stay unread while unfocused");

    app.window_focused = true;
    app.tick(0.);
    assert!(!app.controller.unread(&app.controller.account.sessions[0]));
    assert!(app.controller.unread(&app.controller.account.sessions[1]), "other chats stay unread after focusing");
}

#[test]
fn hidden_card_wakes_only_when_the_dot_crosses_a_color_boundary() {
    let root = tempfile::tempdir().unwrap();
    let ctx = HeadlessCtx::new(&Config {
        size: (1000, 700),
        device_limits: crate::desktop::limits(),
        ..Default::default()
    })
    .unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = App::new(
        &ctx,
        Store::open(root.path().into()).unwrap(),
        Arc::new(move || {
            let _ = tx.send(());
        }),
        false,
    )
    .unwrap();
    app.back();
    crate::demo::populate(&mut app.controller).unwrap();
    app.resize(ctx.size(), 1., Vec2::new(0., 0.));
    app.tick(0.);
    app.preview_connection(ConnectionPreview::Received);
    app.frame(&ctx, ctx.view());
    app.info_tip = Tooltip::default();
    app.controller
        .health
        .sent(Instant::now() - Duration::from_millis(920));
    app.tick(0.);
    assert_eq!(app.dot_color, 0xfbbf24);
    assert_eq!(app.counter_bucket, None);
    let deadline = Instant::now() + Duration::from_secs(2);
    while app.dot_color != 0xfb923c {
        assert!(
            Instant::now() < deadline,
            "hidden color threshold did not wake the UI"
        );
        rx.recv_timeout(Duration::from_millis(300)).unwrap();
        app.tick(0.);
    }
    app.frame(&ctx, ctx.view());
    assert_eq!(
        app.counter_bucket, None,
        "hidden card must not start a 50ms loop"
    );
}

#[test]
fn saved_actions_and_restore_warning_fit_mobile_and_preserve_intents() {
    use crate::store::PendingControl;
    for size in [(360,740),(1000,700)] {
        let root=tempfile::tempdir().unwrap();let ctx=HeadlessCtx::new(&Config {size,device_limits:crate::desktop::limits(),..Default::default()}).unwrap();
        let mut app=App::new(&ctx,Store::open(root.path().into()).unwrap(),Arc::new(||{}),size.0<500).unwrap();app.back();app.resize(ctx.size(),1.,Vec2::new(0.,0.));
        for n in 0..12 {let id=format!("saved-{n:02}");app.controller.account.pending_controls.insert(id.clone(),PendingControl {request:ClientRequest {id,command:ClientCommand::RenameSession {session_id:"chat".into(),title:"Owned title".into()}},deleted_chats:vec![],blocked:true,accepted:false});}
        app.controller.store.put(&app.controller.identity,"account",&app.controller.account).unwrap();
        for action in [Action::Settings,Action::Outbox(0),Action::Outbox(1),Action::InspectControl("saved-00".into()),Action::ReviewRestore("chat".into())] {
            app.apply(action).unwrap();app.tick(0.);app.frame(&ctx,ctx.view());
            for hit in &app.hits {assert!(hit.rect.y>=0. && hit.rect.y+hit.rect.height<=size.1 as f32,"Unreachable modal action at {:?}",hit.rect);}
        }
        let modal=app.modal.as_ref().unwrap();let width=(size.0 as f32-24.).min(620.)-40.;assert!(app.renderer.label_height(&modal.title,width,17.,true)>60.,"Fixture must exercise the complete multi-line warning");
        if size.0<500 {image::save_buffer("/tmp/tau2-restore-mobile.png",&ctx.read_rgba8().unwrap(),size.0,size.1,image::ColorType::Rgba8).unwrap();}
        assert_eq!(app.controller.account.pending_controls.len(),12);app.apply(Action::ForgetControl("saved-00".into())).unwrap();assert_eq!(app.controller.account.pending_controls.len(),12);app.apply(Action::Confirm).unwrap();assert_eq!(app.controller.account.pending_controls.len(),11);
    }
}
