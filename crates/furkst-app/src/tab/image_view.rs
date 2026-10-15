//! Before/after preview of a changed image file, shown in the diff pane.

use egui::{Color32, ColorImage, Rect, RichText, Sense, TextureHandle, TextureOptions, Ui, Vec2, pos2};

/// Textures are limited in size; larger images are shrunk before upload.
const MAX_TEXTURE_SIDE: u32 = 4096;
const CHECKER_SQUARE: f32 = 8.0;

/// The decoded old and new versions of an image; a side is `None` when the file doesn't exist there.
pub struct ImagePair {
    old: Option<ImageView>,
    new: Option<ImageView>,
}

struct ImageView {
    /// Size of the encoded file.
    bytes: usize,
    /// Pixel dimensions of the original image.
    dimensions: [u32; 2],
    /// Decoded pixels until they are uploaded as a texture.
    pixels: Option<ColorImage>,
    texture: Option<TextureHandle>,
}

impl ImageView {
    fn decode(data: &[u8]) -> Option<Self> {
        let (pixels, dimensions) = decode(data, MAX_TEXTURE_SIDE)?;
        Some(Self { bytes: data.len(), dimensions, pixels: Some(pixels), texture: None })
    }

    fn caption(&self) -> String {
        let [w, h] = self.dimensions;
        format!("{w} × {h} px, {}", crate::widgets::format_size(self.bytes))
    }

    fn show(&mut self, ui: &mut Ui, name: &str, available_height: f32) {
        let texture = match (&self.texture, self.pixels.take()) {
            (Some(texture), _) => texture,
            (None, Some(pixels)) => self.texture.insert(ui.ctx().load_texture(name, pixels, TextureOptions::LINEAR)),
            (None, None) => return,
        };
        // Fit into the free space, but never blow small images up.
        let natural = Vec2::new(self.dimensions[0] as f32, self.dimensions[1] as f32);
        let room = Vec2::new(ui.available_width(), available_height.max(40.0));
        let scale = (room.x / natural.x).min(room.y / natural.y).min(1.0);
        let (rect, _) = ui.allocate_exact_size(natural * scale, Sense::hover());
        paint_checkerboard(ui, rect);
        ui.painter().image(texture.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    }
}

impl ImagePair {
    /// Decodes both versions; `None` if either one can't be decoded.
    pub fn decode(old: Option<&[u8]>, new: Option<&[u8]>) -> Option<Self> {
        let side = |data: Option<&[u8]>| match data {
            Some(data) => ImageView::decode(data).map(Some),
            None => Some(None),
        };
        Some(Self { old: side(old)?, new: side(new)? })
    }

    pub fn show(&mut self, ui: &mut Ui) {
        let sides: Vec<(&str, &mut ImageView)> =
            [("Before", self.old.as_mut()), ("After", self.new.as_mut())].into_iter().filter_map(|(label, view)| Some((label, view?))).collect();
        let height = ui.available_height() - ui.text_style_height(&egui::TextStyle::Body) * 2.0 - 16.0;
        ui.columns(sides.len().max(1), |columns| {
            for (ui, (label, view)) in columns.iter_mut().zip(sides) {
                ui.label(RichText::new(label).strong());
                ui.label(RichText::new(view.caption()).weak());
                view.show(ui, label, height);
            }
        });
    }
}

fn paint_checkerboard(ui: &Ui, rect: Rect) {
    let dark = ui.visuals().dark_mode;
    let (base, alt) = if dark {
        (Color32::from_gray(48), Color32::from_gray(72))
    } else {
        (Color32::from_gray(232), Color32::from_gray(205))
    };
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(rect, 0.0, base);
    let (cols, rows) = ((rect.width() / CHECKER_SQUARE).ceil() as usize, (rect.height() / CHECKER_SQUARE).ceil() as usize);
    for row in 0..rows {
        for col in (row % 2..cols).step_by(2) {
            let min = rect.min + Vec2::new(col as f32, row as f32) * CHECKER_SQUARE;
            painter.rect_filled(Rect::from_min_size(min, Vec2::splat(CHECKER_SQUARE)), 0.0, alt);
        }
    }
}

/// Decodes PNG, JPEG, GIF, BMP, WebP or ICO data into pixels at most `max_side` wide and high, keeping the original
/// dimensions alongside.
pub fn decode(data: &[u8], max_side: u32) -> Option<(ColorImage, [u32; 2])> {
    let mut image = image::load_from_memory(data).ok()?;
    let dimensions = [image.width(), image.height()];
    if dimensions.iter().any(|&side| side > max_side) {
        image = image.thumbnail(max_side, max_side);
    }
    let size = [image.width() as usize, image.height() as usize];
    Some((ColorImage::from_rgba_unmultiplied(size, image.to_rgba8().as_raw()), dimensions))
}
