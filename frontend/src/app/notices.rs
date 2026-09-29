//! Popup lifetime is separate from persistent inline settings/recovery errors.
use std::time::{Duration, Instant};
use crate::notice::Notice;

pub(super) const LIFETIME: Duration = Duration::from_secs(4);
#[derive(Default)]
pub(super) struct NoticePopup {
    message: Option<Notice>,
    until: Option<Instant>,
}
impl NoticePopup {
    pub fn observe(&mut self, message: Option<&Notice>, now: Instant) -> bool {
        if self.message.as_ref() != message {
            self.message = message.cloned();
            self.until = message.map(|_| now + LIFETIME);
            return true;
        }
        if self.until.is_some_and(|until| now >= until) {
            self.until = None;
            return true;
        }
        false
    }
    pub fn remaining(&self, now: Instant) -> Option<Duration> {
        self.until.map(|until| until.saturating_duration_since(now))
    }
    pub fn visible(&self) -> bool { self.until.is_some() }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn duplicate_updates_do_not_extend_popups_but_new_alerts_get_their_own_deadline() {
        let now = Instant::now(); let mut popup = NoticePopup::default();
        assert!(popup.observe(Some(&Notice::from("Saved")), now));
        assert!(!popup.observe(Some(&Notice::from("Saved")), now + Duration::from_secs(3)));
        assert_eq!(popup.remaining(now + Duration::from_secs(3)), Some(Duration::from_secs(1)));
        assert!(popup.observe(Some(&Notice::from("Saved")), now + LIFETIME)); assert!(!popup.visible());
        assert!(!popup.observe(Some(&Notice::from("Saved")), now + LIFETIME * 2));
        assert!(popup.observe(Some(&Notice::from("File saved")), now + LIFETIME * 2)); assert!(popup.visible());
        assert!(popup.observe(None, now + LIFETIME * 2)); assert!(!popup.visible());
        assert!(popup.observe(Some(&Notice::from("File saved")), now + LIFETIME * 2)); assert!(popup.visible());
    }
    #[test]
    fn equal_notice_text_with_different_destinations_gets_a_new_deadline() {
        use crate::notice::DownloadTarget;
        let now = Instant::now();
        let mut popup = NoticePopup::default();
        let target = DownloadTarget { identity: "account".into(), lineage: "source".into(), session: "chat".into(), entry: "first".into() };
        let first = Notice::download("Saved to Downloads/Tau".into(), target.clone());
        let second = Notice::download(first.to_string(), DownloadTarget { entry: "second".into(), ..target });
        assert!(popup.observe(Some(&first), now));
        assert!(!popup.observe(Some(&first), now + Duration::from_secs(3)));
        assert!(popup.observe(Some(&second), now + Duration::from_secs(3)));
        assert_eq!(popup.remaining(now + Duration::from_secs(3)), Some(LIFETIME));
        let ordinary = Notice::from(second.to_string());
        assert!(popup.observe(Some(&ordinary), now + Duration::from_secs(4)));
        assert!(popup.message.as_ref().unwrap().download.is_none());
    }

}

#[cfg(all(test, not(target_os = "android")))]
mod render_tests {
    use super::*;
    use crate::app::*;
    use chad::{Config, HeadlessCtx};
    use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};

    #[test]
    fn alerts_fit_text_center_the_vector_close_and_expire_without_input() {
        for (size, scale, name) in [((1000,700),1.,"desktop"), ((360,720),1.,"phone"), ((1080,2160),2.5,"scaled-phone")] {
            let root = tempfile::tempdir().unwrap();
            let ctx = HeadlessCtx::new(&Config { size, device_limits: crate::desktop::limits(), ..Default::default() }).unwrap();
            let wakes = Arc::new(AtomicUsize::new(0)); let wake = wakes.clone();
            let mut app = App::new(&ctx, Store::open(root.path().into()).unwrap(), Arc::new(move || {wake.fetch_add(1,Ordering::SeqCst);}), name != "desktop").unwrap();
            app.back(); crate::demo::populate(&mut app.controller).unwrap();
            app.resize(size,scale,Vec2::new(0.,0.)); app.tick(0.); app.root.workspace.show_chats = false;
            let bounds = Rect::new(0.,0.,size.0 as f32,size.1 as f32);
            for (variant, text) in [("short","Settings saved"), ("wrapped","Your changes could not be saved. The saved draft is still here; check the settings and try again.")] {
                app.controller.notice = Some(text.into()); app.tick(0.); app.frame(&ctx,ctx.view());
                if let Some(root) = std::env::var_os("TAU_NOTICE_PREVIEW_DIR") {
                    let root = std::path::PathBuf::from(root); std::fs::create_dir_all(&root).unwrap();
                    image::save_buffer(root.join(format!("notice-{variant}-{name}.png")), &ctx.read_rgba8().unwrap(), size.0,size.1,image::ColorType::Rgba8).unwrap();
                }
                 let mut layer = Layer::default();
                app.with_ui(|root,cx| root.notice.visit_perframe(&mut ui::Frame {layer:&mut layer,bounds,clip:bounds},cx));
                let card = app.root.notice.body.rect.unwrap(); let close = app.root.notice.close.rect.unwrap();
                assert!((card.y + card.height/2. - close.y - close.height/2.).abs() < 0.01);
                assert!(card.x >= 0. && card.x + card.width <= bounds.width && card.y + card.height <= bounds.height);
                assert_eq!(layer.draws.len(),1,"Only the message is text, never the close icon");
                let draw = layer.draws[0]; assert_eq!(draw.size,16.*scale);
                let layout = app.services.renderer.text.measure(draw.block);
                assert!(draw.at.y + layout.height_em()*draw.size <= card.y + card.height - 11.*scale);
                if variant == "short" { assert!(card.height >= 48.*scale && card.height <= 56.*scale, "Compact card follows the actual font metrics: {card:?}"); }
                assert_eq!(layer.images.len(),1); assert!(layer.images[0].0.to_string_lossy().contains("tau-icon/close/"));
                let icon = layer.images[0].1;
                assert!((icon.x + icon.width/2. - close.x - close.width/2.).abs()<0.01);
                assert!((icon.y + icon.height/2. - card.y - card.height/2.).abs()<0.01);
                let point = Vec2::new(close.x+close.width/2.,close.y+close.height/2.);
                app.press(42,point,name!="desktop"); app.release(42,point);
                assert!(app.controller.notice.is_none());
                assert!(app.controller.selected().unwrap().local.pending.is_empty(),"Dismissal cannot activate controls underneath");
                app.tick(0.);
            }
            app.controller.notice=Some("Keep this error available inline".into());
            app.root.notice.popup.observe(app.controller.notice.as_ref(),Instant::now());
            // Exercise the real idle wake/expiry path without sleeping four seconds per scale.
            app.root.notice.popup.until=Some(Instant::now()+Duration::from_millis(60));
            app.tick(0.); let before=wakes.load(Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(120));
            assert!(wakes.load(Ordering::SeqCst)>before,"An idle app wakes to remove the popup");
            app.tick(0.); app.frame(&ctx,ctx.view());
            assert!(app.root.notice.close.rect.is_none());
            assert!(app.controller.notice.is_some(),"Persistent inline settings/recovery messages are not lost");
        }
    }
}
