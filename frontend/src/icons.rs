//! Native 24×24 UI glyphs, rasterized at the current physical pixel size.
use tiny_skia::{FillRule, LineCap, Paint, PathBuilder, Pixmap, Stroke, Transform};
#[derive(Clone, Copy, Debug)]
pub enum Icon {
    Attach,
    Attachments,
    Send,
    Stop,
    Play,
    ChevronDown,
    ChevronRight,
    Gear,
    Autoscroll,
    Context(Option<f32>),
    CacheTtl(Option<f32>),
}
impl Icon {
    pub fn name(self) -> &'static str {
        match self {
            Self::Attach => "attach",
            Self::Attachments => "attachments",
            Self::Send => "send",
            Self::Stop => "stop",
            Self::Play => "play",
            Self::ChevronDown => "chevron-down",
            Self::ChevronRight => "chevron-right",
            Self::Gear => "gear",
            Self::Autoscroll => "autoscroll",
            Self::Context(_) => "context",
            Self::CacheTtl(_) => "cache-ttl",
        }
    }
    pub fn stamp(self, color: u32) -> u64 {
        u64::from(color) << 32
            | match self {
                Self::Context(Some(r)) | Self::CacheTtl(Some(r)) => {
                    (r.clamp(0., 1.) * 1000.).round() as u64 + 1
                }
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
            Self::Attachments => {
                p.move_to(5., 2.);
                p.line_to(14., 2.);
                p.line_to(20., 8.);
                p.line_to(20., 22.);
                p.line_to(5., 22.);
                p.close();
                p.move_to(14., 2.);
                p.line_to(14., 8.);
                p.line_to(20., 8.);
                pixmap.stroke_path(
                    &p.finish().unwrap(), &paint,
                    &Stroke { width: 1.8, ..Default::default() }, transform, None,
                );
                return straight_alpha(pixmap);
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
            Self::Play => {
                p.move_to(7., 4.5);
                p.line_to(19., 12.);
                p.line_to(7., 19.5);
                p.close();
            }
            Self::ChevronDown | Self::ChevronRight => {
                if matches!(self, Self::ChevronRight) {
                    p.move_to(9., 5.5);
                    p.line_to(15.5, 12.);
                    p.line_to(9., 18.5);
                } else {
                    p.move_to(5.5, 9.);
                    p.line_to(12., 15.5);
                    p.line_to(18.5, 9.);
                }
                pixmap.stroke_path(
                    &p.finish().unwrap(),
                    &paint,
                    &Stroke {
                        width: 2.5,
                        line_cap: LineCap::Round,
                        ..Default::default()
                    },
                    transform,
                    None,
                );
                return straight_alpha(pixmap);
            }
            Self::Autoscroll => {
                // A centered up/down scroll marker, not a font-dependent arrow.
                p.move_to(12., 3.5);
                p.line_to(6.7, 9.5);
                p.line_to(17.3, 9.5);
                p.close();
                p.move_to(6.7, 14.5);
                p.line_to(12., 20.5);
                p.line_to(17.3, 14.5);
                p.close();
                p.push_circle(12., 12., 1.3);
            }
            Self::Gear => {
                // Eight squared-off teeth and a cut-out center, sharing the
                // same 24-unit canvas as the other header controls.
                let mut first = true;
                for tooth in 0..8 {
                    for (angle, radius) in [
                        (-22.5, 8.),
                        (-14., 8.),
                        (-14., 10.),
                        (14., 10.),
                        (14., 8.),
                        (22.5, 8.),
                    ] {
                        let angle =
                            (-90. + tooth as f32 * 45. + angle) * std::f32::consts::PI / 180.;
                        let (x, y) = (12. + radius * angle.cos(), 12. + radius * angle.sin());
                        if first {
                            p.move_to(x, y);
                            first = false;
                        } else {
                            p.line_to(x, y);
                        }
                    }
                }
                p.close();
                p.push_circle(12., 12., 3.2);
                pixmap.fill_path(
                    &p.finish().unwrap(),
                    &paint,
                    FillRule::EvenOdd,
                    transform,
                    None,
                );
                return straight_alpha(pixmap);
            }
            Self::Context(ratio) | Self::CacheTtl(ratio) => {
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

#[cfg(test)]
mod tests {
    use super::Icon;

    #[test]
    fn autoscroll_arrows_and_center_dot_are_centered_at_multiple_scales() {
        for size in [24, 48] {
            let pixels = Icon::Autoscroll.pixels(size, 0x67d4ff);
            let mut weight = 0f64;
            let (mut x, mut y) = (0f64, 0f64);
            for row in 0..size as usize {
                for column in 0..size as usize {
                    let alpha = pixels[(row * size as usize + column) * 4 + 3] as f64;
                    weight += alpha;
                    x += (column as f64 + 0.5) * alpha;
                    y += (row as f64 + 0.5) * alpha;
                }
            }
            assert!(weight > 0.);
            assert!((x / weight - size as f64 / 2.).abs() < 0.2);
            assert!((y / weight - size as f64 / 2.).abs() < 0.2);
            let alpha = |column: u32, row: u32| pixels[((row * size + column) * 4 + 3) as usize];
            assert!(alpha(size / 2, size / 2) > 0, "center dot is visible");
            assert!(alpha(size / 2, size / 4) > 0, "up arrow is visible");
            assert!(alpha(size / 2, size * 3 / 4) > 0, "down arrow is visible");
        }
    }
}
