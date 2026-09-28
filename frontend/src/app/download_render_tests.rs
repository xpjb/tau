//! Offline screenshots of the actual shared attachment renderer (no network or OS actions).
use super::*;
use chad::{Config, HeadlessCtx};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Deserialize)]
pub(super) struct Case {
    pub(super) id: String,
    pub(super) label: String,
    pub(super) state: String,
    pub(super) size: Option<u64>,
    #[serde(default)] pub(super) transferred: u64,
    pub(super) rate: Option<u64>,
    pub(super) error: Option<String>,
    pub(super) name: Option<String>,
    pub(super) caption: Option<String>,
    #[serde(default)] pub(super) image: bool,
}
pub(super) fn cases() -> Vec<Case> {
    serde_json::from_str(include_str!("../../qa/downloads/cases.json")).unwrap()
}
pub(super) fn install(app: &mut App, case: &Case) -> ChatAttachment {
    let entry = case.id.as_str();
    let path = app.controller.attachment_path("demo", entry);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let cached = matches!(case.state.as_str(), "cached" | "saving" | "save-failed" | "decode-failed")
        || case.image && case.state == "saved";
    if cached {
        if case.image && case.state != "decode-failed" {
            let image = image::RgbaImage::from_fn(360, 216, |x,y| {
                if (x / 36 + y / 36) % 2 == 0 { image::Rgba([47, 128, 156, 255]) }
                else { image::Rgba([22, 78, 99, 255]) }
            });
            image.save_with_format(&path, image::ImageFormat::Png).unwrap();
        } else { std::fs::write(&path, b"fixture cached bytes").unwrap(); }
    }
    let key = Controller::download_key("demo", entry);
    if matches!(case.state.as_str(), "active" | "failed" | "unavailable") {
        let mut download = crate::controller::Download::new(tau_transfer::TransferStatus {
            transferred: case.transferred, total: case.size.unwrap_or(0), network_bytes: case.transferred,
            done: case.state != "active", failure: case.error.clone(),
        }, path.clone());
        download.bytes_per_second = case.rate;
        app.controller.downloads.insert(key.clone(), download);
    }
    if case.state == "saving" { app.services.transfers.saving_downloads.insert(key.clone()); }
    if case.state == "save-failed" { app.services.transfers.export_errors.insert(key, case.error.clone().unwrap()); }
    if matches!(case.state.as_str(), "saved" | "saved-no-cache" | "missing" | "extracting") {
        let saved_path = app.controller.store.root.join(format!("saved-{entry}"));
        if case.state != "missing" { std::fs::write(&saved_path, b"saved fixture").unwrap(); }
        let identity = app.controller.identity.clone();
        app.controller.record_download(&identity, "", "demo", entry, crate::store::SavedDownload {
            reference: saved_path.to_string_lossy().into(), location: "Downloads/Tau".into(), mime_type: "application/octet-stream".into(),
        }).unwrap();
    }
    if case.state == "extracting" {
        app.services.transfers.extracting_downloads.insert(app.export_target("demo", entry));
    }
    ChatAttachment { source_path: None, kind: if case.image { AttachmentKind::Image } else { AttachmentKind::File },
        file_name: case.name.clone().unwrap_or_else(|| if case.image { "preview.png" } else { "release-notes.pdf" }.into()),
        size: case.size, caption: case.caption.clone() }
}
pub(super) fn save(ctx: &HeadlessCtx, name: &str) {
    if let Some(root) = std::env::var_os("TAU_DOWNLOAD_PREVIEW_DIR") {
        let root = PathBuf::from(root);
        std::fs::create_dir_all(&root).unwrap();
        image::save_buffer(root.join(format!("{name}.png")), &ctx.read_rgba8().unwrap(),
            ctx.size().0, ctx.size().1, image::ColorType::Rgba8).unwrap();
    }
}

pub(super) struct CardControl { pub rect: Rect, pub action: ui::CardChoice }
pub(super) fn controls(app: &App) -> Vec<CardControl> {
    app.root.test_cards.cards.values().flat_map(|card| card.controls.items.iter()).filter_map(|(key,button,choice)| {
        if !key.starts_with("action:") { return None; }
        let control = &button.control;
        let rect = crate::render::intersect(control.rect?,control.clip);
        (rect.width > 0. && rect.height > 0.).then(|| CardControl { rect, action: choice.clone() })
    }).collect()
}
pub(super) fn hints(app: &App) -> Vec<(Rect,Info)> { app.root.test_cards.hints().chain(app.root.workspace.attachments.cards.hints()).map(|(r,i)|(r,i.clone())).collect() }
pub(super) fn panel(app: &mut App, ctx: &HeadlessCtx, case: &Case, file: &ChatAttachment, interaction: Interaction, viewport: Rect) -> Layer {
    let s = app.ui.scale;
    app.root.test_cards.begin();
    let mut layer = Layer::new(interaction);
    layer.rect(Rect::new(0., 0., ctx.size().0 as f32, ctx.size().1 as f32), color(0x0e141b));
    app.services.renderer.label(&mut layer, &case.label,
        Rect::new(12. * s, 10. * s, ctx.size().0 as f32 - 24. * s, 28. * s), 12. * s, color(0xb7c2ce), false);
    let rect = Rect::new(12. * s, 44. * s, ctx.size().0 as f32 - 24. * s, attachments::card_height(file) * s);
    layer.clipped_rounded_rect(rect, 12. * s, color(0x18212b), viewport);
    app.attachment_card(ctx, &mut layer, "demo", &case.id, file, "gallery", rect, viewport);
    app.with_ui(|root,cx| root.test_cards.finish(cx));
    layer
}

#[test]
fn render_download_state_matrix() {
    for (name, width, scale, mobile) in [("desktop", 552, 1., false), ("sidebar", 320, 1., false),
        ("phone", 360, 1., true), ("phone-2x5", 900, 2.5, true)] {
        let size = (width, (420. * scale) as u32);
        let ctx = HeadlessCtx::new(&Config { size, device_limits: crate::desktop::limits(), ..Default::default() }).unwrap();
        let root = tempfile::tempdir().unwrap();
        let mut app = App::new(&ctx, Store::open(root.path().into()).unwrap(), Arc::new(|| {}), mobile).unwrap();
        app.back(); crate::demo::populate(&mut app.controller).unwrap();
        app.resize(size, scale, Vec2::new(0., 0.));
        for case in cases() {
            let file = install(&mut app, &case);
            let bounds = Rect::new(0., 0., width as f32, size.1 as f32);
            let layer = panel(&mut app, &ctx, &case, &file, Interaction::default(), bounds);
            let buttons = controls(&app).iter().map(|h| h.rect).collect::<Vec<_>>();
            let expected = match case.state.as_str() {
                "cached" | "save-failed" if case.image => 2,
                "saved" | "extracting" => 1 + usize::from(case.image) + if mobile { 0 } else { 1 + usize::from(file.file_name.ends_with(".zip")) },
                "saved-no-cache" => if mobile { 1 } else { 2 },
                _ => 1,
            };
            assert_eq!(buttons.len(), expected, "{name}/{} must expose every applicable action", case.id);
            for (i, button) in buttons.iter().enumerate() {
                let target = if mobile { 44. } else { 40. } * scale;
                assert_eq!(button.height, target);
                assert!(button.width >= target, "labels keep the minimum hit target");
                assert_eq!(layer.draws.iter().filter(|d| contains(*button, d.at)).count(), 1,
                    "every action must have a visible text label, not a custom glyph");
                assert!(layer.images.iter().all(|(_, rect, _)| {
                    let overlap = crate::render::intersect(*button, *rect);
                    overlap.width == 0. || overlap.height == 0.
                }), "action buttons must be text-only");
                assert!(button.x >= bounds.x && button.x + button.width <= bounds.width);
                assert!(button.y >= bounds.y && button.y + button.height <= bounds.height);
                for other in &buttons[i+1..] {
                    assert_eq!(crate::render::intersect(*button, *other).width, 0., "no overlapping controls");
                }
            }
            // All labels inside the actual control must be single-line, measured and non-overlapping.
            for draw in layer.draws.iter().filter(|d| d.at.y >= 44. * scale) {
                let layout = app.services.renderer.text.measure(draw.block);
                assert_eq!(layout.line_count(), 1, "{}: labels must ellipsize, never wrap under buttons", case.id);
                assert!(layout.width_em() * draw.size <= draw.clip.unwrap().width + 0.1);
                if let Some(button) = buttons.iter().find(|b| contains(**b, draw.at)) {
                    assert!(draw.at.x + layout.width_em() * draw.size <= button.x + button.width + 0.1);
                    assert!(draw.at.y + layout.height_em() * draw.size <= button.y + button.height + 0.1);
                } else {
                    assert!(buttons.iter().all(|b| draw.at.y + layout.height_em() * draw.size <= b.y
                        || draw.at.y >= b.y + b.height || draw.at.x + layout.width_em() * draw.size <= b.x),
                        "{}: filename/status must not run into a button", case.id);
                }
            }
            app.services.renderer.draw(&ctx, ctx.view(), &[layer]);
            save(&ctx, &format!("{name}-{}", case.id));
            for (i, button) in buttons.iter().enumerate() {
                let point = Vec2::new(button.x + button.width / 2., button.y + button.height / 2.);
                for (state, pressed) in [("hover", false), ("pressed", true)] {
                    let layer = panel(&mut app, &ctx, &case, &file, Interaction {
                        hover: Some(point), pressed: pressed.then_some(point), held: pressed,
                    }, bounds);
                    app.services.renderer.draw(&ctx, ctx.view(), &[layer]);
                    save(&ctx, &format!("{name}-{}-{state}-{i}", case.id));
                }
                let mut layer = panel(&mut app, &ctx, &case, &file, Interaction::default(), bounds);
                let hint = hints(&app).into_iter().find(|(r, _)| contains(*r, point));
                if !mobile && file.file_name.ends_with(".zip") && i == buttons.len() - 1 && matches!(case.state.as_str(), "saved" | "extracting") {
                    assert!(hint.is_none(), "Extract must not show a redundant tooltip");
                    continue;
                }
                let (rect, info) = hint.expect("Other download actions keep their descriptions");
                app.root.tooltips.target = info;
                app.root.tooltips.info.region = rect; app.root.tooltips.info.progress = 1.;
                app.with_ui(|root, cx| root.tooltips.info_frame(cx, &mut layer, bounds));
                assert!(app.root.tooltips.info.content.text.contains(&file.file_name), "tooltip names the target file");
                assert!(app.root.tooltips.info.card.y + app.root.tooltips.info.card.height <= bounds.height);
                app.services.renderer.draw(&ctx, ctx.view(), &[layer]);
                save(&ctx, &format!("{name}-{}-tooltip-{i}", case.id));
            }
            // Scrolling clips geometry rather than moving the label into a partial button.
            let last = *buttons.last().unwrap();
            let viewport = Rect::new(0., last.y + last.height / 2., bounds.width, bounds.height - last.y - last.height / 2.);
            let layer = panel(&mut app, &ctx, &case, &file, Interaction::default(), viewport);
            assert!(app.placed_controls().iter().all(|h| h.rect.y >= viewport.y));
            assert!(controls(&app).iter().all(|h| h.rect.height <= last.height / 2.));
            app.services.renderer.draw(&ctx, ctx.view(), &[layer]);
            save(&ctx, &format!("{name}-{}-clipped", case.id));
        }
    }
}
