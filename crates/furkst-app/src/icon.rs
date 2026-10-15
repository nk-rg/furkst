//! The application icon. `assets/icon.png` and `assets/icon.ico` are generated from code by the ignored
//! `write_icon_assets` test, so the logo always matches the branch glyph drawn in the toolbar:
//!
//! ```sh
//! cargo test -p furkst write_icon_assets -- --ignored
//! ```

/// The window and taskbar icon, decoded from the embedded `.ico` (its largest size).
pub fn window_icon() -> egui::IconData {
    let image = image::load_from_memory(include_bytes!("../assets/icon.ico")).expect("the embedded icon decodes").into_rgba8();
    egui::IconData { width: image.width(), height: image.height(), rgba: image.into_raw() }
}

#[cfg(test)]
mod generate {
    use std::path::Path;

    use egui::{Pos2, Rect, pos2, vec2};
    use image::codecs::ico::{IcoEncoder, IcoFrame};
    use image::{ExtendedColorType, Rgba, RgbaImage};

    use crate::widgets::BranchGlyph;

    const ICO_SIZES: [u32; 7] = [16, 24, 32, 48, 64, 128, 256];
    const TOP_LEFT: [f32; 3] = [47.0, 129.0, 247.0];
    const BOTTOM_RIGHT: [f32; 3] = [138.0, 99.0, 210.0];

    /// Regenerates the icon files from the drawing code below.
    #[test]
    #[ignore = "writes assets; run explicitly after changing the logo"]
    fn write_icon_assets() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
        std::fs::create_dir_all(&dir).unwrap();
        render(1024).save(dir.join("icon.png")).unwrap();

        // Each size is rendered natively rather than scaled down, which keeps small icons crisp.
        let frames: Vec<IcoFrame<'_>> = ICO_SIZES
            .iter()
            .map(|&size| IcoFrame::as_png(render(size).as_raw(), size, size, ExtendedColorType::Rgba8).unwrap())
            .collect();
        IcoEncoder::new(std::fs::File::create(dir.join("icon.ico")).unwrap()).encode_images(&frames).unwrap();
    }

    #[test]
    fn embedded_icon_decodes() {
        let icon = super::window_icon();
        assert_eq!((icon.width, icon.height), (256, 256));
    }

    /// The logo: the branch glyph in white on a rounded square with a blue-to-violet gradient.
    fn render(size: u32) -> RgbaImage {
        let pixel = 1.0 / size as f32;
        let glyph = BranchGlyph::fit(Rect::from_center_size(pos2(0.5, 0.5), vec2(0.82, 0.82)));
        // Anti-aliased coverage of a shape from the signed distance of a pixel center to its edge.
        let coverage = |distance: f32| (0.5 - distance / pixel).clamp(0.0, 1.0);
        RgbaImage::from_fn(size, size, |x, y| {
            let p = pos2((x as f32 + 0.5) * pixel, (y as f32 + 0.5) * pixel);
            let background = coverage(rounded_square(p, 0.03, 0.2));
            let glyph_distance = [
                segment(p, glyph.trunk[0], glyph.trunk[1]) - glyph.stroke / 2.0,
                bezier(p, glyph.fork) - glyph.stroke / 2.0,
                glyph.nodes.iter().map(|n| n.distance(p) - glyph.radius).fold(f32::MAX, f32::min),
            ]
            .into_iter()
            .fold(f32::MAX, f32::min);
            let white = coverage(glyph_distance);
            let t = (p.x + p.y) / 2.0;
            let channel = |i: usize| {
                let base = TOP_LEFT[i] + (BOTTOM_RIGHT[i] - TOP_LEFT[i]) * t;
                (base + (255.0 - base) * white).round() as u8
            };
            Rgba([channel(0), channel(1), channel(2), (background * 255.0).round() as u8])
        })
    }

    /// Signed distance to a square filling the unit square minus `margin`, with corner `radius`.
    fn rounded_square(p: Pos2, margin: f32, radius: f32) -> f32 {
        let half = 0.5 - margin - radius;
        let q = vec2((p.x - 0.5).abs() - half, (p.y - 0.5).abs() - half);
        q.max(vec2(0.0, 0.0)).length() + q.x.max(q.y).min(0.0) - radius
    }

    fn segment(p: Pos2, a: Pos2, b: Pos2) -> f32 {
        let (ab, ap) = (b - a, p - a);
        let t = (ap.dot(ab) / ab.length_sq()).clamp(0.0, 1.0);
        (ap - ab * t).length()
    }

    /// Distance to a cubic Bézier curve, approximated by a fine polyline.
    fn bezier(p: Pos2, [a, b, c, d]: [Pos2; 4]) -> f32 {
        const STEPS: usize = 64;
        let at = |t: f32| {
            let u = 1.0 - t;
            (a.to_vec2() * u * u * u + b.to_vec2() * 3.0 * u * u * t + c.to_vec2() * 3.0 * u * t * t + d.to_vec2() * t * t * t)
                .to_pos2()
        };
        (0..STEPS).map(|i| segment(p, at(i as f32 / STEPS as f32), at((i + 1) as f32 / STEPS as f32))).fold(f32::MAX, f32::min)
    }
}
