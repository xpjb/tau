//! The same 24×24 paths as Tau 1, rasterized at the current physical pixel size.
use tiny_skia::{FillRule, LineCap, Paint, PathBuilder, Pixmap, Stroke, Transform};
#[derive(Clone, Copy, Debug)]
pub enum Icon {
    Attach,
    Send,
    Stop,
    Context(Option<f32>),
}
impl Icon {
    pub fn name(self) -> &'static str {
        match self {
            Self::Attach => "attach",
            Self::Send => "send",
            Self::Stop => "stop",
            Self::Context(_) => "context",
        }
    }
    pub fn stamp(self, color: u32) -> u64 {
        u64::from(color) << 32
            | match self {
                Self::Context(Some(r)) => (r.clamp(0., 1.) * 1000.).round() as u64 + 1,
                _ => 0,
            }
    }
    pub fn pixels(self, size: u32, color: u32) -> Vec<u8> {
        let mut pixmap = Pixmap::new(size, size).expect("nonzero icon size");
        let mut paint = Paint::default();
        paint.set_color_rgba8((color >> 16) as u8, (color >> 8) as u8, color as u8, 255);
        paint.anti_alias = true;
        let transform = Transform::from_scale(size as f32 / 24., size as f32 / 24.);
        let mut p = PathBuilder::new();
        match self {
            Self::Attach => {
                p.move_to(16.5, 6.);
                p.line_to(16.5, 17.5);
                p.cubic_to(16.5, 19.71, 14.71, 21.5, 12.5, 21.5);
                p.cubic_to(10.29, 21.5, 8.5, 19.71, 8.5, 17.5);
                p.line_to(8.5, 5.);
                p.cubic_to(8.5, 3.62, 9.62, 2.5, 11., 2.5);
                p.cubic_to(12.38, 2.5, 13.5, 3.62, 13.5, 5.);
                p.line_to(13.5, 15.5);
                p.cubic_to(13.5, 16.05, 13.05, 16.5, 12.5, 16.5);
                p.cubic_to(11.95, 16.5, 11.5, 16.05, 11.5, 15.5);
                p.line_to(11.5, 6.);
                p.line_to(10., 6.);
                p.line_to(10., 15.5);
                p.cubic_to(10., 16.88, 11.12, 18., 12.5, 18.);
                p.cubic_to(13.88, 18., 15., 16.88, 15., 15.5);
                p.line_to(15., 5.);
                p.cubic_to(15., 2.79, 13.21, 1., 11., 1.);
                p.cubic_to(8.79, 1., 7., 2.79, 7., 5.);
                p.line_to(7., 17.5);
                p.cubic_to(7., 20.54, 9.46, 23., 12.5, 23.);
                p.cubic_to(15.54, 23., 18., 20.54, 18., 17.5);
                p.line_to(18., 6.);
                p.close();
            }
            Self::Send => {
                p.move_to(2.01, 21.);
                p.line_to(23., 12.);
                p.line_to(2.01, 3.);
                p.line_to(2., 10.);
                p.line_to(17., 12.);
                p.line_to(2., 14.);
                p.close();
            }
            Self::Stop => {
                p.move_to(6., 6.);
                p.line_to(18., 6.);
                p.line_to(18., 18.);
                p.line_to(6., 18.);
                p.close();
            }
            Self::Context(ratio) => {
                // 20dp circle, 2dp stroke, like Tau 1. Coordinates below use 24 units.
                let stroke = Stroke {
                    width: 2.4,
                    line_cap: LineCap::Round,
                    ..Default::default()
                };
                let mut ring = PathBuilder::new();
                ring.push_circle(12., 12., 10.8);
                let mut track = Paint::default();
                track.set_color_rgba8(67, 77, 91, 255);
                track.anti_alias = true;
                pixmap.stroke_path(&ring.finish().unwrap(), &track, &stroke, transform, None);
                if let Some(ratio) = ratio {
                    let count = (ratio.clamp(0., 1.) * 96.).ceil() as usize;
                    if count > 0 {
                        for i in 0..=count {
                            let a = -std::f32::consts::FRAC_PI_2
                                + std::f32::consts::TAU * ratio.clamp(0., 1.) * i as f32
                                    / count as f32;
                            let (x, y) = (12. + 10.8 * a.cos(), 12. + 10.8 * a.sin());
                            if i == 0 {
                                p.move_to(x, y);
                            } else {
                                p.line_to(x, y);
                            }
                        }
                    }
                } else {
                    p.move_to(9.6, 12.);
                    p.line_to(14.4, 12.);
                }
                if let Some(path) = p.finish() {
                    pixmap.stroke_path(&path, &paint, &stroke, transform, None);
                }
                return straight_alpha(pixmap);
            }
        }
        pixmap.fill_path(
            &p.finish().unwrap(),
            &paint,
            FillRule::Winding,
            transform,
            None,
        );
        straight_alpha(pixmap)
    }
}
fn straight_alpha(pixmap: Pixmap) -> Vec<u8> {
    let mut bytes = pixmap.take();
    for rgba in bytes.as_chunks_mut::<4>().0 {
        let a = rgba[3] as u32;
        for c in &mut rgba[..3] {
            *c = (*c as u32 * 255 + a / 2)
                .checked_div(a)
                .unwrap_or(0)
                .min(255) as u8;
        }
    }
    bytes
}
