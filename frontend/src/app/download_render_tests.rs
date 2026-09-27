//! Offline screenshots of the actual shared attachment renderer (no network or OS actions).
use super::*;
use chad::{Config, HeadlessCtx};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Deserialize)]
struct Case {
    id: String,
    label: String,
    state: String,
    size: Option<u64>,
    #[serde(default)] transferred: u64,
    rate: Option<u64>,
    error: Option<String>,
    name: Option<String>,
    caption: Option<String>,
    #[serde(default)] image: bool,
}
fn cases() -> Vec<Case> {
    serde_json::from_str(include_str!("../../qa/downloads/cases.json")).unwrap()
}
fn install(app: &mut App, case: &Case) -> ChatAttachment {
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
    if matches!(case.state.as_str(), "saved" | "missing") {
        let saved_path = app.controller.store.root.join(format!("saved-{entry}"));
        if case.state == "saved" { std::fs::write(&saved_path, b"saved fixture").unwrap(); }
        let identity = app.controller.identity.clone();
        app.controller.record_download(&identity, "", "demo", entry, crate::store::SavedDownload {
            reference: saved_path.to_string_lossy().into(), location: "Downloads/Tau".into(), mime_type: "application/octet-stream".into(),
        }).unwrap();
    }
    ChatAttachment { source_path: None, kind: if case.image { AttachmentKind::Image } else { AttachmentKind::File },
        file_name: case.name.clone().unwrap_or_else(|| if case.image { "preview.png" } else { "release-notes.pdf" }.into()),
        size: case.size, caption: case.caption.clone() }
}
fn save(ctx: &HeadlessCtx, name: &str) {
    if let Some(root) = std::env::var_os("TAU_DOWNLOAD_PREVIEW_DIR") {
        let root = PathBuf::from(root);
        std::fs::create_dir_all(&root).unwrap();
        image::save_buffer(root.join(format!("{name}.png")), &ctx.read_rgba8().unwrap(),
            ctx.size().0, ctx.size().1, image::ColorType::Rgba8).unwrap();
    }
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
            let rect = Rect::new(12. * scale, 44. * scale, width as f32 - 24. * scale,
                attachments::card_height(&file) * scale);
            app.hits.clear(); app.info_areas.clear();
            let mut layer = Layer::default();
            layer.rect(bounds, color(0x0e141b));
            app.renderer.label(&mut layer, &case.label, Rect::new(12. * scale, 10. * scale,
                bounds.width - 24. * scale, 28. * scale), 12. * scale, color(0xb7c2ce), false);
            layer.rounded_rect(rect, 12. * scale, color(0x18212b));
            app.attachment_card(&ctx, &mut layer, "demo", &case.id, &file, rect, bounds);
            app.renderer.draw(&ctx, ctx.view(), &[layer]);
            save(&ctx, &format!("{name}-{}", case.id));
            assert!(!ctx.read_rgba8().unwrap().is_empty());
        }
    }
}
