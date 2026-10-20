//! KawanEditor branding (logo, About line) and the host's accent colour for the WordCraft UI.
//! WordCraft's own UI calls these at a few hook points; everything here reads the host policy
//! (`wordcraft_engine::kawan`) and falls back to KawanEditor's default logo and WordCraft's own
//! colours.

use egui::{Color32, Sense, vec2};
use wordcraft_engine::kawan as policy;

use crate::theme::Tokens;

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

/// The File tab's side panel: WordCraft's fixed app colour (in both themes), or the host accent.
pub fn app_color() -> Color32 {
    policy::accent().map_or(crate::theme::APP_COLOR, rgb)
}

/// True once per new host policy: the frame that sees it re-applies the theme (the accent).
pub fn policy_changed(ctx: &egui::Context) -> bool {
    let generation = policy::generation();
    ctx.data_mut(|d| std::mem::replace(d.get_temp_mut_or(egui::Id::new("kawan-generation"), generation), generation) != generation)
}

fn rgb(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

/// WordCraft's tokens with the host accent: the accent, its text colour on panels, the text on
/// top of it, the checked wash (blended from the panel colour) and the selection. Unchanged
/// without an accent.
pub fn tint(t: Tokens) -> Tokens {
    match policy::accent() {
        Some(a) => tint_with(t, a),
        None => t,
    }
}

fn tint_with(mut t: Tokens, a: [u8; 3]) -> Tokens {
    let s = [t.panel.r(), t.panel.g(), t.panel.b()];
    let (text, checked, alpha) = if t.dark {
        (policy::mix(a, [255; 3], 0.30), policy::mix(s, a, 0.30), 0x55)
    } else {
        (policy::mix(a, [0; 3], 0.20), policy::mix(s, a, 0.15), 0x48)
    };
    t.accent = rgb(a);
    t.accent_text = rgb(text);
    t.on_accent = rgb(policy::on_color(a));
    t.checked = rgb(checked);
    t.selection = Color32::from_rgba_unmultiplied(a[0], a[1], a[2], alpha);
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_accent(accent: Option<[u8; 3]>) {
        policy::set(policy::Policy { accent, ..policy::Policy::default() });
    }

    #[test]
    fn tint_is_identity_without_accent() {
        with_accent(None);
        for t in [Tokens::light(), Tokens::dark()] {
            assert_eq!(format!("{:?}", tint(t)), format!("{t:?}"));
        }
        assert_eq!(app_color(), crate::theme::APP_COLOR);
    }

    #[test]
    fn tint_replaces_the_accent_family() {
        let a = [0xE1, 0x1D, 0x48];
        with_accent(Some(a));
        let light = tint(Tokens::light());
        assert_eq!(light.accent, rgb(a));
        assert_eq!(light.accent_text, rgb([180, 23, 58]));
        assert_eq!(light.on_accent, rgb([255, 255, 255]));
        assert_eq!(light.checked, rgb([246, 217, 223]));
        assert_eq!(light.selection, Color32::from_rgba_unmultiplied(0xE1, 0x1D, 0x48, 0x48));
        let dark = tint(Tokens::dark());
        assert_eq!(dark.accent, rgb(a));
        assert_eq!(dark.accent_text, rgb([234, 97, 127]));
        assert_eq!(dark.on_accent, rgb([255, 255, 255]));
        assert_eq!(dark.checked, rgb([94, 35, 48]));
        assert_eq!(dark.selection, Color32::from_rgba_unmultiplied(0xE1, 0x1D, 0x48, 0x55));
        // Everything else stays WordCraft's.
        assert_eq!(dark.panel, Tokens::dark().panel);
        assert_eq!(light.hover, Tokens::light().hover);
        assert_eq!(app_color(), rgb(a));
        with_accent(None);
    }

    #[test]
    fn on_accent_is_readable() {
        with_accent(Some([0xFF, 0xFF, 0x00]));
        for t in [tint(Tokens::light()), tint(Tokens::dark())] {
            assert_eq!(t.on_accent, rgb([17, 17, 17]));
        }
        with_accent(None);
    }

    #[test]
    fn policy_changed_fires_once_per_policy() {
        let ctx = egui::Context::default();
        with_accent(None);
        assert!(!policy_changed(&ctx), "the first look only records the generation");
        assert!(!policy_changed(&ctx));
        with_accent(Some([1, 2, 3]));
        assert!(policy_changed(&ctx));
        assert!(!policy_changed(&ctx));
        with_accent(None);
    }

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
