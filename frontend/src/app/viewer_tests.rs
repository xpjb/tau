use super::*;
use chad::{Config, HeadlessCtx};
use std::sync::Arc;

#[test]
fn image_viewer_closes_on_background_tap_but_not_image_controls_or_pan() {
    for size in [(1000, 800), (420, 780)] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("sample.png");
        image::save_buffer(
            &path,
            &vec![220; 32 * 32 * 4],
            32,
            32,
            image::ColorType::Rgba8,
        )
        .unwrap();
        let ctx = HeadlessCtx::new(&Config {
            size,
            device_limits: crate::desktop::limits(),
            ..Default::default()
        })
        .unwrap();
        let mut app = App::new(
            &ctx,
            Store::open(root.path().join("client")).unwrap(),
            Arc::new(|| {}),
            false,
        )
        .unwrap();
        app.back();
        crate::demo::populate(&mut app.controller).unwrap();
        app.resize(size, 1., Vec2::new(0., 0.));
        let open = |app: &mut App| {
            app.viewer = Some(Viewer {
                path: path.clone(),
                name: "sample.png".into(),
                session: "demo".into(),
                entry: "entry-1".into(),
                zoom: 1.,
                pan: Vec2::new(0., 0.),
            });
            app.tick(0.);
            app.frame(&ctx, ctx.view());
        };
        let background = |image: Rect| {
            if image.x > 20. {
                Vec2::new(10., image.y + image.height / 2.)
            } else {
                Vec2::new(image.x + image.width / 2., image.y - 20.)
            }
        };
        let tap = |app: &mut App, id: u64, point: Vec2, touch: bool| {
            app.press(id, point, touch);
            app.release(id, point);
        };

        open(&mut app);
        let image = app.viewer_image.unwrap();
        let center = Vec2::new(image.x + image.width / 2., image.y + image.height / 2.);
        tap(&mut app, 1, center, false);
        assert!(
            app.viewer.is_some(),
            "clicking the image itself must not close it"
        );
        let off_image = background(image);
        assert!(!contains(image, off_image));
        tap(&mut app, 2, off_image, false);
        assert!(
            app.viewer.is_none(),
            "desktop click on dim background closes viewer"
        );
        assert!(app.context_menu.is_none());

        open(&mut app);
        let off_image = background(app.viewer_image.unwrap());
        app.press(3, off_image, true);
        app.motion(3, Vec2::new(off_image.x + 35., off_image.y));
        app.release(3, Vec2::new(off_image.x + 35., off_image.y));
        assert!(
            app.viewer.as_ref().is_some_and(|v| v.pan.x > 0.),
            "background drag still pans the viewer"
        );

        open(&mut app);
        let off_image = background(app.viewer_image.unwrap());
        tap(&mut app, 4, off_image, true);
        assert!(
            app.viewer.is_none(),
            "touching the background closes the viewer"
        );

        open(&mut app);
        app.viewer.as_mut().unwrap().zoom = 2.;
        app.frame(&ctx, ctx.view());
        let fit = app
            .hits
            .iter()
            .find(|h| matches!(h.action, Action::Fit))
            .unwrap()
            .rect;
        tap(
            &mut app,
            5,
            Vec2::new(fit.x + fit.width / 2., fit.y + fit.height / 2.),
            false,
        );
        assert_eq!(
            app.viewer.as_ref().unwrap().zoom,
            1.,
            "viewer controls still work"
        );
        app.frame(&ctx, ctx.view());
        let back = app
            .hits
            .iter()
            .find(|h| matches!(h.action, Action::Back))
            .unwrap()
            .rect;
        tap(
            &mut app,
            6,
            Vec2::new(back.x + back.width / 2., back.y + back.height / 2.),
            false,
        );
        assert!(app.viewer.is_none(), "Back still closes the viewer");
    }
}
