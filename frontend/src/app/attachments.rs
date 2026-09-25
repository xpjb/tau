use super::*;

impl App {
    pub(super) fn attachment_card(
        &mut self,
        ctx: &impl RenderContext,
        layer: &mut Layer,
        session: &str,
        entry: &str,
        attachment: &ChatAttachment,
        rect: Rect,
        viewport: Rect,
    ) {
        let s = self.scale;
        let image = attachment.kind == AttachmentKind::Image;
        let path = self.controller.attachment_path(session, entry);
        let saved = path.is_file();
        let key = Controller::download_key(session, entry);
        let x = rect.x + 14. * s;
        let width = rect.width - 28. * s;
        let bottom = rect.y + rect.height;
        if image && saved {
            let preview = Rect::new(x, bottom - 372. * s, width, 230. * s);
            let clip = crate::render::intersect(preview, viewport);
            if clip.height > 0. && let Ok((w, h)) = self.renderer.image_size(ctx, &path) {
                let fit = (width / w as f32).min(230. * s / h as f32);
                let image_rect = Rect::new(x, preview.y, w as f32 * fit, h as f32 * fit);
                layer.images.push((path.clone(), image_rect, clip));
                self.hits.push(Hit {
                    rect: crate::render::intersect(image_rect, clip),
                    action: Action::Attachment(session.into(), entry.into(), attachment.file_name.clone(), true),
                });
            }
        } else if image && self.controller.content_authorized()
            && !self.controller.downloads.contains_key(&key)
            && let Err(error) = self.controller.download(session, entry, 10_000_000)
        {
            self.controller.notice = Some(error.to_string());
        }
        self.renderer.clipped_label(layer, &attachment.file_name,
            Rect::new(x, bottom - 132. * s, width, 22. * s),
            13. * s, color(0xe5eaf0), true, viewport);
        if let Some(caption) = &attachment.caption {
            self.renderer.clipped_label(layer, caption,
                Rect::new(x, bottom - 106. * s, width, 52. * s),
                13. * s, color(0xb7c2ce), false, viewport);
        }
        let status = if saved {
            "Saved on this device".into()
        } else if let Some(download) = self.controller.downloads.get(&key) {
            download.status.failure.clone().unwrap_or_else(||
                format!("{} / {} bytes", download.status.transferred, download.status.total))
        } else {
            format!("{} · {}", if image { "Image" } else { "File" },
                attachment.size.map_or_else(|| "size unknown".into(), |n| format!("{n} bytes")))
        };
        self.renderer.clipped_label(layer, &status,
            Rect::new(x, bottom - 50. * s, width, 18. * s),
            11. * s, color(0x82909f), false, viewport);
        let mut actions = vec![(
            if saved { if image { "View image" } else { "Save file" } } else { "Download" },
            Action::Attachment(session.into(), entry.into(), attachment.file_name.clone(), image),
        )];
        if self.controller.downloads.get(&key).is_some_and(|download| !download.status.done) {
            actions.push(("Cancel", Action::CancelDownload(key)));
        }
        let mut ax = rect.x + 10. * s;
        for (label, action) in actions {
            let width = (label.chars().count() as f32 * 7. + 18.) * s;
            if ax + width > rect.x + rect.width { break; }
            let r = crate::render::intersect(Rect::new(ax, bottom - 28. * s, width, 24. * s), viewport);
            if r.height > 0. {
                button(&mut self.renderer, layer, &mut self.hits, r, label, action, s, false);
            }
            ax += width + 5. * s;
        }
    }
}
