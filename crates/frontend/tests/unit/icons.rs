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
