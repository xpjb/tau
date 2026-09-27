//! Actual rich-text shaping/GPU previews, plus automatic quota reads against a local socket.
use super::*;
use chad::{Config, HeadlessCtx};
use std::{sync::Arc, time::Duration};

fn report() -> CodexUsage {
    CodexUsage { provider: "openai-codex".into(), plan: Some("pro".into()), fetched_at_ms: 1_800_000_000_000,
        age_ms: 0, limit_reached: false, windows: vec![
            CodexUsageWindow { id: "primary".into(), label: "5-hour".into(), duration_seconds: Some(18000),
                remaining_percent: Some(74.), resets_at_ms: Some(1_800_007_200_000) },
            CodexUsageWindow { id: "secondary".into(), label: "Weekly".into(), duration_seconds: Some(604800),
                remaining_percent: Some(18.5), resets_at_ms: Some(1_800_180_000_000) },
        ] }
}
fn assert_card(app: &mut App, bounds: Rect, usage: bool) -> usize {
    let mut layer = Layer::default();
    if usage { app.usage_frame(&mut layer, bounds); } else { app.info_frame(&mut layer, bounds); }
    let card = if usage { app.usage.card } else { app.info_tip.card };
    assert!(card.x >= bounds.x && card.y >= bounds.y && card.x + card.width <= bounds.x + bounds.width
        && card.y + card.height <= bounds.y + bounds.height, "Card must fit the viewport: {card:?}");
    assert_eq!(layer.draws.len(), 1, "One shaped rich block, no refresh button");
    let draw = layer.draws[0];
    assert!(draw.paint.is_some(), "Inline colours must reach the actual text draw");
    let layout = app.renderer.text.measure(draw.block);
    assert!(layout.line_count() > 0, "Rich font spans must shape successfully");
    let clip = draw.clip.unwrap();
    assert!(draw.at.y + layout.height_em() * draw.size <= clip.y + clip.height + 0.1,
        "Measured rich text must not be cut off at the bottom");
    assert!(layout.width_em() * draw.size <= clip.width + 0.1, "Wrapped text must fit horizontally");
    layout.line_count()
}
fn preview(ctx: &HeadlessCtx, name: &str) {
    if let Some(root) = std::env::var_os("TAU_TOOLTIP_PREVIEW_DIR") {
        let root = std::path::PathBuf::from(root);
        std::fs::create_dir_all(&root).unwrap();
        image::save_buffer(root.join(format!("{name}.png")), &ctx.read_rgba8().unwrap(),
            ctx.size().0, ctx.size().1, image::ColorType::Rgba8).unwrap();
    }
}

#[test]
fn rich_tooltips_fit_desktop_phone_and_scaled_phone_without_clipping() {
    for (size, scale, name) in [((1000,700), 1., "desktop"), ((360,720), 1., "phone"), ((1080,2160), 2.5, "scaled-phone")] {
        let root = tempfile::tempdir().unwrap();
        let ctx = HeadlessCtx::new(&Config { size, device_limits: crate::desktop::limits(), ..Default::default() }).unwrap();
        let mut app = App::new(&ctx, Store::open(root.path().into()).unwrap(), Arc::new(|| {}), name != "desktop").unwrap();
        app.back(); crate::demo::populate(&mut app.controller).unwrap();
        app.resize(size, scale, Vec2::new(0.,0.)); app.tick(0.); app.show_chats = false;
        let bounds = Rect::new(0.,0.,size.0 as f32,size.1 as f32);
        app.controller.account.sessions.iter_mut().find(|s|s.id=="demo").unwrap().model.as_mut().unwrap().provider = "openai-codex".into();
        app.controller.epoch = Some(1); // Rendering only: no tick/network in this fixture.
        app.controller.codex_usage.report = Some(report());
        app.controller.codex_usage.received = Some(Instant::now());
        app.usage.pinned = true; app.usage.progress = 1.;
        app.frame(&ctx,ctx.view());
        assert_card(&mut app, bounds, true);
        assert!(app.usage.content.spans.iter().any(|s|s.bold && s.tint==crate::tooltip::GOOD));
        assert!(app.usage.content.spans.iter().any(|s|s.bold && s.tint==crate::tooltip::WARNING));
        preview(&ctx, &format!("quota-{name}"));

        app.controller.codex_usage.report.as_mut().unwrap().windows[0].label = "Longer quota window with literal **stars** and 日本語".into();
        app.controller.codex_usage.error = Some("Codex quota unavailable: sign in to Codex or renew its credentials.".into());
        app.frame(&ctx,ctx.view());
        let lines = assert_card(&mut app, bounds, true);
        assert!(lines > app.usage.content.text.lines().count(), "Exercise real wrapping, not just newline counting");
        assert!(app.usage.content.text.contains("**stars**"), "Provider data stays literal");
        preview(&ctx, &format!("quota-wrapped-{name}"));

        app.usage = Tooltip::default(); app.show_chats = true;
        let session = app.controller.account.sessions.iter_mut().find(|s|s.id=="two").unwrap();
        session.updated_at_ms = clock::now_ms().unwrap() - 52 * 60_000;
        app.info_target = Info::CacheTtl("two".into());
        app.info_tip.pinned = true; app.info_tip.progress = 1.;
        app.frame(&ctx,ctx.view());
        assert_eq!(assert_card(&mut app,bounds,false), 1);
        assert_eq!(app.info_tip.content.text, "TTL ~8m remaining");
        preview(&ctx, &format!("ttl-{name}"));
        app.controller.account.sessions.iter_mut().find(|s|s.id=="two").unwrap().status = SessionStatus::Running;
        app.frame(&ctx,ctx.view());
        assert_eq!(assert_card(&mut app,bounds,false), 1);
        assert_eq!(app.info_tip.content.text, "Working...");
        preview(&ctx, &format!("working-{name}"));

        app.preview_connection(ConnectionPreview::Received);
        app.frame(&ctx,ctx.view());
        assert_card(&mut app,bounds,false);
        preview(&ctx, &format!("connection-{name}"));
        app.preview_connection(ConnectionPreview::Disconnected);
        app.controller.connection = "A longer connection failure reason which should wrap naturally without clipping or changing the live attempt counters.".into();
        app.frame(&ctx,ctx.view());
        assert_card(&mut app,bounds,false);
        preview(&ctx, &format!("connection-wrapped-{name}"));
    }
}

#[tokio::test]
async fn quota_refreshes_without_opening_a_tooltip_and_stops_when_hidden_or_non_codex() {
    use axum::{Router, extract::ws::{Message, WebSocketUpgrade}, routing::get};
    use futures_util::StreamExt;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = Router::new().route("/v1/ws", get(move |ws: WebSocketUpgrade| {
        let count = count.clone();
        async move { ws.on_upgrade(move |mut socket| async move {
            let hello = ServerMessage::Hello { protocol_version: PROTOCOL_VERSION, daemon_version: "fixture".into(), lineage: Some("quota-fixture".into()) };
            socket.send(Message::Text(serde_json::to_string(&hello).unwrap().into())).await.unwrap();
            while let Some(Ok(message)) = socket.next().await {
                match message {
                    Message::Ping(payload) => { if socket.send(Message::Pong(payload)).await.is_err() { break; } }
                    Message::Text(value) => {
                        let request: ClientRequest = serde_json::from_str(&value).unwrap();
                        if let ClientCommand::GetCodexUsage { force } = request.command {
                            assert!(!force, "Periodic reads respect the daemon's shared cache");
                            count.fetch_add(1,Ordering::SeqCst);
                            let response = ServerMessage::CodexUsage { request_id: request.id, report: Some(report()), error: None };
                            if socket.send(Message::Text(serde_json::to_string(&response).unwrap().into())).await.is_err() { break; }
                        }
                    }
                    _ => {}
                }
            }
        }) }
    }));
    let server = tokio::spawn(async move { axum::serve(listener,router).await.unwrap(); });
    let root = tempfile::tempdir().unwrap();
    let store = Store::open(root.path().into()).unwrap();
    store.put("", "settings", &Settings { server_url: format!("http://{addr}"), token: "fixture".into() }).unwrap();
    let ctx = HeadlessCtx::new(&Config { size: (1000,700), device_limits: crate::desktop::limits(), ..Default::default() }).unwrap();
    let mut app = App::new(&ctx,store,Arc::new(|| {}),false).unwrap();
    app.back(); app.resize(ctx.size(),1.,Vec2::new(0.,0.));
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.controller.epoch.is_none() {
        assert!(Instant::now()<deadline); app.tick(0.); tokio::time::sleep(Duration::from_millis(10)).await;
    }
    crate::demo::populate(&mut app.controller).unwrap();
    app.controller.account.sessions.iter_mut().find(|s|s.id=="demo").unwrap().model.as_mut().unwrap().provider="openai-codex".into();
    for expected in 1..=2 {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            assert!(Instant::now()<deadline);
            app.tick(0.); tokio::time::sleep(Duration::from_millis(10)).await;
            if calls.load(Ordering::SeqCst)==expected && app.controller.codex_usage.in_flight.is_none() { break; }
        }
        assert_eq!(app.usage.progress,0.,"No hover/click is needed");
        assert!(app.controller.codex_usage.report.is_some());
        for _ in 0..5 { app.tick(0.); }
        assert_eq!(calls.load(Ordering::SeqCst),expected,"No duplicate reads while fresh");
        app.controller.codex_usage.attempted=Some(Instant::now()-Duration::from_secs(301));
    }
    let attempted=app.controller.codex_usage.attempted;
    app.window_focused=false; app.tick(0.);
    assert_eq!(app.controller.codex_usage.attempted,attempted);
    app.window_focused=true;
    app.set_connection_visible(false); app.tick(0.);
    assert_eq!(app.controller.codex_usage.attempted,attempted);
    app.set_connection_visible(true);
    app.controller.account.sessions.iter_mut().find(|s|s.id=="demo").unwrap().model.as_mut().unwrap().provider="anthropic".into();
    app.tick(0.);
    assert_eq!(app.controller.codex_usage.attempted,attempted);
    assert_eq!(calls.load(Ordering::SeqCst),2);
    assert!(app.controller.selected().unwrap().local.pending.is_empty(),"Quota reads never send prompts");
    drop(app); server.abort();
}
