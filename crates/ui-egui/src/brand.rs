//! The PhotoCraft brand mark at the left of the title bar: the app icon (the kitsune,
//! `assets/app-icon/`, see its README), where Photoshop shows its "Ps" tile. The PNG carries the
//! icon's rounded corners; it is decoded once per context into a mipmapped texture, so it stays
//! crisp at the title bar's size on any display scale.

use egui::{Color32, Context, Id, Rect, TextureHandle, TextureOptions, Ui, pos2};
use photocraft_codecs::{ChannelLayout, SampleType};

/// 128 px: sharp at 20 pt on a 3× display, and the mipmaps keep it clean at 1×.
const ICON_PNG: &[u8] = include_bytes!("../../../assets/app-icon/hicolor/128x128/apps/ai.storyteller.photocraft.png");

/// The icon's pixels (one transparent pixel if it couldn't be decoded, which a test rules out).
fn decode() -> egui::ColorImage {
    let rgba = photocraft_codecs::decode(ICON_PNG).ok().map(|img| img.convert(ChannelLayout::Rgba, SampleType::U8));
    match rgba {
        Some(img) if img.data().len() == img.width() as usize * img.height() as usize * 4 => {
            egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.data())
        }
        _ => egui::ColorImage::filled([1, 1], Color32::TRANSPARENT),
    }
}

/// The icon texture, uploaded on first use and kept in the context.
fn texture(ctx: &Context) -> TextureHandle {
    let id = Id::new("photocraft-brand-mark");
    if let Some(tex) = ctx.data(|d| d.get_temp::<TextureHandle>(id)) {
        return tex;
    }
    let options = TextureOptions { mipmap_mode: Some(egui::TextureFilter::Linear), ..TextureOptions::LINEAR };
    let tex = ctx.load_texture("photocraft-brand-mark", decode(), options);
    ctx.data_mut(|d| d.insert_temp(id, tex.clone()));
    tex
}

/// The engraving's line work alone, as white alpha: the blue field and the paper figure fall away,
/// leaving a kitsune silhouette the empty states can tint with the theme's ink or paper. Kept to
/// the dark strokes so the watermark reads as a drawing rather than a filled shape.
fn watermark_image() -> egui::ColorImage {
    let icon = decode();
    let brand = crate::theme::BRAND_BLUE;
    let pixels = icon
        .pixels
        .iter()
        .map(|p| {
            let blue = (f32::from(p.r()) - f32::from(brand.r())).powi(2)
                + (f32::from(p.g()) - f32::from(brand.g())).powi(2)
                + (f32::from(p.b()) - f32::from(brand.b())).powi(2);
            let away = ((blue.sqrt() - 30.0) / 60.0).clamp(0.0, 1.0);
            let luminance = 0.2126 * f32::from(p.r()) + 0.7152 * f32::from(p.g()) + 0.0722 * f32::from(p.b());
            let ink = ((100.0 - luminance) / 60.0).clamp(0.0, 1.0);
            let alpha = (f32::from(p.a()) / 255.0 * ink * away * 255.0).round().clamp(0.0, 255.0) as u8;
            Color32::from_white_alpha(alpha)
        })
        .collect();
    egui::ColorImage::new(icon.size, pixels)
}

/// The watermark texture, uploaded on first use and cached like the mark.
fn watermark_texture(ctx: &Context) -> TextureHandle {
    let id = Id::new("photocraft-brand-watermark-texture");
    if let Some(tex) = ctx.data(|d| d.get_temp::<TextureHandle>(id)) {
        return tex;
    }
    let options = TextureOptions { mipmap_mode: Some(egui::TextureFilter::Linear), ..TextureOptions::LINEAR };
    let tex = ctx.load_texture("photocraft-brand-watermark", watermark_image(), options);
    ctx.data_mut(|d| d.insert_temp(id, tex.clone()));
    tex
}

/// Where the mark was painted this frame (tests and screenshots).
fn rect_id() -> Id {
    Id::new("photocraft-brand-mark-rect")
}

/// Where the empty-state watermark was last painted (tests).
#[cfg(test)]
fn watermark_id() -> Id {
    Id::new("photocraft-brand-watermark-rect")
}

fn paint(ui: &Ui, r: Rect, tint: Color32) {
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    ui.painter().image(texture(ui.ctx()).id(), r, uv, tint);
}

/// Paint the mark into `r` (square).
pub fn paint_mark(ui: &Ui, r: Rect) {
    paint(ui, r, Color32::WHITE);
    ui.ctx().data_mut(|d| d.insert_temp(rect_id(), r));
}

/// Paint the mark's engraved line work as a watermark, tinted to a quiet background mark.
/// Empty-state cards can report a zero-height layout rect before their content is measured, so the
/// painter's clip is widened to cover `r` rather than dropping the mark.
pub fn paint_watermark(ui: &Ui, r: Rect, tint: Color32) {
    let painter = ui.painter().with_clip_rect(ui.clip_rect().union(r));
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    painter.image(watermark_texture(ui.ctx()).id(), r, uv, tint);
    #[cfg(test)]
    ui.ctx().data_mut(|d| d.insert_temp(watermark_id(), r));
}

/// The rect the mark was last painted into.
#[cfg(test)]
pub fn mark_rect(ctx: &Context) -> Option<Rect> {
    ctx.data(|d| d.get_temp(rect_id()))
}

/// The rect the empty-state watermark was last painted into.
#[cfg(test)]
pub fn watermark_rect(ctx: &Context) -> Option<Rect> {
    ctx.data(|d| d.get_temp(watermark_id()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_icon_decodes_to_a_rounded_128_px_tile() {
        let img = decode();
        assert_eq!(img.size, [128, 128]);
        // Rounded corners: the corner pixel is transparent, the centre opaque.
        assert_eq!(img.pixels[0].a(), 0);
        assert_eq!(img.pixels[64 * 128 + 64].a(), 255);
    }

    #[test]
    fn the_watermark_keeps_the_engraving_lines_and_drops_the_blue_field() {
        let img = watermark_image();
        assert_eq!(img.size, [128, 128]);
        let visible = img.pixels.iter().filter(|p| p.a() > 0).count();
        assert!(visible > 2_000 && visible < 8_000, "the line work covers part of the tile, not all of it ({visible})");
        // The field inside the rounded corner is blue, so it leaves no watermark.
        assert_eq!(img.pixels[8 * 128 + 64].a(), 0, "the solid blue field is transparent");
        assert!(img.pixels.iter().any(|p| p.a() > 128), "the engraved ink stays opaque enough to tint");
    }

    #[test]
    fn the_title_bar_paints_the_icon_from_one_cached_texture() {
        let ctx = Context::default();
        crate::PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        // The system title bar hides the in-app mark (the OS draws its own icon), so paint the
        // app-drawn bar here.
        app.custom_titlebar = true;
        let mut frame = || {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| crate::panels::title_bar(&mut app, ui));
            let uploads = out.textures_delta.set.values().flat_map(|d| d.iter()).filter(|d| d.image.size() == [128, 128]).count();
            out.textures_delta.clear();
            uploads
        };
        assert_eq!(frame(), 1, "uploaded on first use");
        assert_eq!(frame(), 0, "then reused");
        assert!(mark_rect(&ctx).is_some());
    }
}
