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
    if case.state == "saving" { app.saving_downloads.insert(key.clone()); }
    if case.state == "save-failed" { app.export_errors.insert(key, case.error.clone().unwrap()); }
    if matches!(case.state.as_str(), "saved" | "saved-no-cache" | "missing") {
        let saved_path = app.controller.store.root.join(format!("saved-{entry}"));
        if case.state != "missing" { std::fs::write(&saved_path, b"saved fixture").unwrap(); }
        let identity = app.controller.identity.clone();
        app.controller.record_download(&identity, "", "demo", entry, crate::store::SavedDownload {
            reference: saved_path.to_string_lossy().into(), location: "Downloads/Tau".into(), mime_type: "application/octet-stream".into(),
        }).unwrap();
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

pub(super) fn controls(app: &App) -> Vec<&Hit> {
    app.hits.iter().filter(|h| matches!(h.action, Action::Attachment(..) | Action::SaveAttachment(..)
        | Action::UseSaved(..) | Action::CancelDownload(..) | Action::Noop)
        && h.rect.width <= 44. * app.scale).collect()
}
pub(super) fn panel(app: &mut App, ctx: &HeadlessCtx, case: &Case, file: &ChatAttachment, interaction: Interaction, viewport: Rect) -> Layer {
    let s = app.scale;
    app.hits.clear(); app.info_areas.clear();
    let mut layer = Layer::new(interaction);
    layer.rect(Rect::new(0., 0., ctx.size().0 as f32, ctx.size().1 as f32), color(0x0e141b));
    app.renderer.label(&mut layer, &case.label,
        Rect::new(12. * s, 10. * s, ctx.size().0 as f32 - 24. * s, 28. * s), 12. * s, color(0xb7c2ce), false);
    let rect = Rect::new(12. * s, 44. * s, ctx.size().0 as f32 - 24. * s, attachments::card_height(file) * s);
    layer.clipped_rounded_rect(rect, 12. * s, color(0x18212b), viewport);
    app.attachment_card(ctx, &mut layer, "demo", &case.id, file, "gallery", rect, viewport);
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
                "saved" => 1 + usize::from(case.image) + if mobile { 0 } else { 1 + usize::from(file.file_name.ends_with(".zip")) },
                "saved-no-cache" => if mobile { 1 } else { 2 },
                _ => 1,
            };
            assert_eq!(buttons.len(), expected, "{name}/{} must expose every applicable icon", case.id);
            for (i, button) in buttons.iter().enumerate() {
                let target = if mobile { 44. } else { 40. } * scale;
                assert_eq!((button.width, button.height), (target, target));
                assert!(button.x >= bounds.x && button.x + button.width <= bounds.width);
                assert!(button.y >= bounds.y && button.y + button.height <= bounds.height);
                for other in &buttons[i+1..] {
                    assert_eq!(crate::render::intersect(*button, *other).width, 0., "no overlapping controls");
                }
            }
            // All labels inside the actual control must be single-line, measured and non-overlapping.
            for draw in layer.draws.iter().filter(|d| d.at.y >= 44. * scale) {
                let layout = app.renderer.text.measure(draw.block);
                assert_eq!(layout.line_count(), 1, "{}: labels must ellipsize, never wrap under buttons", case.id);
                assert!(layout.width_em() * draw.size <= draw.clip.unwrap().width + 0.1);
                assert!(buttons.iter().all(|b| draw.at.y + layout.height_em() * draw.size <= b.y
                    || draw.at.y >= b.y + b.height || draw.at.x + layout.width_em() * draw.size <= b.x),
                    "{}: text must not run into an icon", case.id);
            }
            app.renderer.draw(&ctx, ctx.view(), &[layer]);
            save(&ctx, &format!("{name}-{}", case.id));
            for (i, button) in buttons.iter().enumerate() {
                let point = Vec2::new(button.x + button.width / 2., button.y + button.height / 2.);
                for (state, pressed) in [("hover", false), ("pressed", true)] {
                    let layer = panel(&mut app, &ctx, &case, &file, Interaction {
                        hover: Some(point), pressed: pressed.then_some(point), held: pressed,
                    }, bounds);
                    app.renderer.draw(&ctx, ctx.view(), &[layer]);
                    save(&ctx, &format!("{name}-{}-{state}-{i}", case.id));
                }
                let mut layer = panel(&mut app, &ctx, &case, &file, Interaction::default(), bounds);
                let (rect, info) = app.info_areas.iter().find(|(r, _)| contains(*r, point)).unwrap().clone();
                app.info_target = info;
                app.info_tip.region = rect; app.info_tip.progress = 1.;
                app.info_frame(&mut layer, bounds);
                assert!(app.info_tip.content.text.contains(&file.file_name), "tooltip names the target file");
                assert!(app.info_tip.card.y + app.info_tip.card.height <= bounds.height);
                app.renderer.draw(&ctx, ctx.view(), &[layer]);
                save(&ctx, &format!("{name}-{}-tooltip-{i}", case.id));
            }
            // Scrolling clips geometry rather than moving the icon/label into a partial button.
            let last = *buttons.last().unwrap();
            let viewport = Rect::new(0., last.y + last.height / 2., bounds.width, bounds.height - last.y - last.height / 2.);
            let layer = panel(&mut app, &ctx, &case, &file, Interaction::default(), viewport);
            assert!(app.hits.iter().all(|h| h.rect.y >= viewport.y));
            assert!(controls(&app).iter().all(|h| h.rect.height <= last.height / 2.));
            app.renderer.draw(&ctx, ctx.view(), &[layer]);
            save(&ctx, &format!("{name}-{}-clipped", case.id));
        }
    }
}
