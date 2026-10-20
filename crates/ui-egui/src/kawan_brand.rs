//! KawanEditor branding (logo, About line) for the WordCraft UI. WordCraft's own UI calls these
//! at a few hook points; everything here reads the host policy (`wordcraft_engine::kawan`) and
//! falls back to KawanEditor's default logo.

use egui::{Sense, vec2};
use wordcraft_engine::kawan as policy;

/// The default logo: KawanEditor's mark, 256 x 256.
const DEFAULT_LOGO: &[u8] = include_bytes!("../assets/kawan-logo.png");
/// Larger host logos are refused (a small PNG can inflate to a huge bitmap).
const MAX_SIDE: u32 = 2048;

/// Decode a PNG into an egui image; `None` for anything that is not a PNG of a sane size.
pub fn decode_png(bytes: &[u8]) -> Option<egui::ColorImage> {
    if !policy::is_png(bytes) {
        return None;
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(64 * 1024 * 1024);
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), image::ImageFormat::Png);
    reader.limits(limits);
    let rgba = reader.decode().ok()?.to_rgba8();
    let (w, h) = rgba.dimensions();
    if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
        return None;
    }
    let size = [usize::try_from(w).ok()?, usize::try_from(h).ok()?];
    let raw = rgba.as_raw();
    (raw.len() == size[0].checked_mul(size[1])?.checked_mul(4)?).then(|| egui::ColorImage::from_rgba_unmultiplied(size, raw))
}

/// The logo texture: the host's PNG when it sent one that decodes, otherwise KawanEditor's.
/// Cached per policy generation, so a new policy brings its new logo.
pub fn logo_texture(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    let id = egui::Id::new("kawan-logo");
    let generation = policy::generation();
    if let Some((built, texture)) = ctx.data(|d| d.get_temp::<(u64, egui::TextureHandle)>(id))
        && built == generation
    {
        return Some(texture);
    }
    let image = policy::logo_png().and_then(|png| decode_png(&png)).or_else(|| decode_png(DEFAULT_LOGO))?;
    let texture = ctx.load_texture("kawan-logo", image, egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, (generation, texture.clone())));
    Some(texture)
}

/// The logo, `size` points square, labelled "KawanEditor". With a host `href` it is a link that
/// opens in a new tab.
pub fn logo(ui: &mut egui::Ui, size: f32) -> egui::Response {
    let href = policy::logo_href();
    let sense = if href.is_some() { Sense::click() } else { Sense::hover() };
    let response = match logo_texture(ui.ctx()) {
        Some(texture) => ui.add(egui::Image::from_texture(&texture).fit_to_exact_size(vec2(size, size)).alt_text(policy::PRODUCT).sense(sense)),
        None => {
            let response = ui.allocate_response(vec2(size, size), sense);
            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Image, true, policy::PRODUCT));
            response
        }
    };
    match href {
        Some(href) => {
            let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
            if response.clicked() {
                ui.ctx().open_url(egui::OpenUrl::new_tab(href));
            }
            response
        }
        None => response,
    }
}

/// The small logo in the title bar's right-to-left row, where the community pill was: the space
/// after it is its left margin.
pub fn header_logo(ui: &mut egui::Ui) -> egui::Response {
    let response = logo(ui, 18.0);
    ui.add_space(8.0);
    response
}

/// The About dialog's version and credit line.
pub fn about() -> String {
    policy::about("WordCraft")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bad_host_png_falls_back_to_default() {
        assert!(decode_png(b"nope").is_none());
        let mut truncated = DEFAULT_LOGO.to_vec();
        truncated.truncate(40);
        assert!(decode_png(&truncated).is_none(), "a PNG signature alone is not enough");
        assert_eq!(decode_png(DEFAULT_LOGO).map(|i| i.size), Some([256, 256]));
    }

    #[test]
    fn oversized_png_is_refused() {
        let img = image::RgbaImage::new(MAX_SIDE + 1, 1);
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        assert!(decode_png(&png).is_none());
        let img = image::GrayImage::new(3, 2);
        let mut png = Vec::new();
        image::DynamicImage::ImageLuma8(img).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        assert_eq!(decode_png(&png).map(|i| i.size), Some([3, 2]), "grey PNGs decode too");
    }

    #[test]
    fn logo_texture_follows_the_policy() {
        let img = image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 0, 0, 255]));
        let mut red = Vec::new();
        image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut red), image::ImageFormat::Png).unwrap();
        let ctx = egui::Context::default();
        policy::set(policy::Policy { logo_png: Some(red.clone().into()), ..policy::Policy::default() });
        assert_eq!(logo_texture(&ctx).map(|t| t.size()), Some([4, 4]));
        let first = logo_texture(&ctx).map(|t| t.id());
        assert_eq!(logo_texture(&ctx).map(|t| t.id()), first, "cached while the policy stays");
        let textures = ctx.tex_manager().read().num_allocated();
        policy::set(policy::Policy { logo_png: Some(b"nope".to_vec().into()), ..policy::Policy::default() });
        assert_eq!(logo_texture(&ctx).map(|t| t.size()), Some([256, 256]), "a bad host PNG falls back");
        assert_ne!(logo_texture(&ctx).map(|t| t.id()), first, "a new policy, a new texture");
        for _ in 0..3 {
            policy::set(policy::Policy { logo_png: Some(red.clone().into()), ..policy::Policy::default() });
            assert_eq!(logo_texture(&ctx).map(|t| t.size()), Some([4, 4]));
        }
        assert_eq!(ctx.tex_manager().read().num_allocated(), textures, "the old logo is freed, not kept per generation");
        policy::set(policy::Policy::default());
    }

    #[test]
    fn about_names_word_craft() {
        assert_eq!(about(), policy::about("WordCraft"));
        assert!(about().starts_with("KawanEditor "));
    }
}
