//! Offline screenshots of the actual shared attachment renderer (no network or OS actions).
use super::*;
use chad::HeadlessCtx;
use serde::Deserialize;

#[derive(Deserialize)]
pub(super) struct Case {
    pub(super) id: String,
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
    app.root.workspace.attachments.cards.values().flat_map(|card| card.controls.items.iter()).filter_map(|(key,button,choice)| {
        if key.is_none() { return None; }
        let control = &button.control;
        let rect = crate::render::intersect(control.rect?,control.clip);
        (rect.width > 0. && rect.height > 0.).then(|| CardControl { rect, action: choice.clone() })
    }).collect()
}
pub(super) fn hints(app: &App) -> Vec<(Rect,Info)> {
    app.root.workspace.attachments.cards.values().flat_map(|card| card.controls.hints()).map(|(r,i)|(r,i.clone())).collect()
}

#[test]
fn download_actions_and_labels_fit_a_narrow_card() {
    let (mut app, ctx, _dir) = super::download_interaction_tests::fixture((320, 420), false);
    let case = cases().into_iter().find(|case| case.id == "18-saved-zip").unwrap();
    let file = install(&mut app, &case);
    let bounds = Rect::new(0., 0., ctx.size().0 as f32, ctx.size().1 as f32);
    super::download_interaction_tests::paint(&mut app, &ctx, &case, &file);
    let mut layer = Layer::default();
    app.with_ui(|root, cx| root.workspace.attachments.visit_perframe(
        &mut ui::Frame { layer: &mut layer, bounds, clip: bounds }, cx));
    let buttons = controls(&app);
    for (i, button) in buttons.iter().enumerate() {
        assert!(button.rect.x >= 0. && button.rect.x + button.rect.width <= bounds.width);
        assert!(button.rect.y >= 0. && button.rect.y + button.rect.height <= bounds.height);
        for other in &buttons[i + 1..] {
            assert_eq!(crate::render::intersect(button.rect, other.rect).width, 0., "Overlapping actions");
        }
        for draw in layer.draws.iter().filter(|d| contains(button.rect, d.at)) {
            let layout = app.services.renderer.text.measure(draw.block);
            assert!(draw.at.x + layout.width_em() * draw.size <= button.rect.x + button.rect.width + 0.1);
            assert!(draw.at.y + layout.height_em() * draw.size <= button.rect.y + button.rect.height + 0.1);
        }
    }
    app.services.renderer.draw(&ctx, ctx.view(), &[layer]);
    save(&ctx, "narrow-card");
}
