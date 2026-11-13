use eframe::egui::{self, Color32};

const DISCO_ACCENT: Color32 = Color32::from_rgb(239, 113, 255);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GlassPalette {
    pub card: Color32,
    pub sidebar: Color32,
    pub accent: Color32,
    pub secondary: Color32,
}

impl Default for GlassPalette {
    fn default() -> Self {
        Self {
            card: crate::CARD,
            sidebar: crate::SIDEBAR,
            accent: crate::ACCENT,
            secondary: crate::BLUE,
        }
    }
}

fn linear(channel: f64) -> f64 {
    if channel <= 0.04045 {
        channel / 12.92
    } else {
        ((channel + 0.055) / 1.055).powf(2.4)
    }
}

fn srgb(channel: f64) -> u8 {
    let value = if channel <= 0.0031308 {
        channel * 12.92
    } else {
        1.055 * channel.powf(1.0 / 2.4) - 0.055
    };
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

pub fn vibrant_average(image: &image::RgbaImage) -> Option<[u8; 3]> {
    let mut vibrant = [0.0_f64; 3];
    let mut fallback = [0.0_f64; 3];
    let mut vibrant_weight = 0.0;
    let mut fallback_weight = 0.0;
    let step_x = image.width().div_ceil(96).max(1) as usize;
    let step_y = image.height().div_ceil(96).max(1) as usize;
    for y in (0..image.height()).step_by(step_y) {
        for x in (0..image.width()).step_by(step_x) {
            let pixel = image.get_pixel(x, y).0;
            let alpha = f64::from(pixel[3]) / 255.0;
            if alpha < 0.05 {
                continue;
            }
            let rgb = [
                f64::from(pixel[0]) / 255.0,
                f64::from(pixel[1]) / 255.0,
                f64::from(pixel[2]) / 255.0,
            ];
            let max = rgb.into_iter().fold(0.0_f64, f64::max);
            let min = rgb.into_iter().fold(1.0_f64, f64::min);
            let saturation = if max > 0.0 { (max - min) / max } else { 0.0 };
            let weight = if saturation >= 0.2 && max >= 0.15 {
                alpha * saturation.powi(2) * max
            } else {
                0.0
            };
            for channel in 0..3 {
                let value = linear(rgb[channel]);
                vibrant[channel] += value * weight;
                fallback[channel] += value * alpha;
            }
            vibrant_weight += weight;
            fallback_weight += alpha;
        }
    }
    let (sum, weight) = if vibrant_weight > 0.0 {
        (vibrant, vibrant_weight)
    } else {
        (fallback, fallback_weight)
    };
    (weight > 0.0).then(|| sum.map(|value| srgb(value / weight)))
}

#[cfg(test)]
pub fn background_is_bright(image: &image::RgbaImage) -> bool {
    let step_x = image.width().div_ceil(64).max(1);
    let step_y = image.height().div_ceil(64).max(1);
    let mut brightness = 0.0;
    let mut weight = 0.0;
    let mut bright_samples = 0;
    let mut samples = 0;
    for y in (0..image.height()).step_by(step_y as usize) {
        for x in (0..image.width()).step_by(step_x as usize) {
            let pixel = image.get_pixel(x, y).0;
            let alpha = f64::from(pixel[3]) / 255.0;
            if alpha < 0.05 {
                continue;
            }
            let luminance = (0.2126 * f64::from(pixel[0])
                + 0.7152 * f64::from(pixel[1])
                + 0.0722 * f64::from(pixel[2]))
                / 255.0;
            let visible_luminance = luminance * alpha + 0.03 * (1.0 - alpha);
            brightness += visible_luminance * alpha;
            weight += alpha;
            bright_samples += usize::from(visible_luminance >= 0.78);
            samples += 1;
        }
    }
    weight > 0.0 && (brightness / weight >= 0.68 || bright_samples * 4 >= samples)
}

impl GlassPalette {
    pub fn for_appearance(appearance: crate::settings::Appearance) -> Self {
        match appearance {
            crate::settings::Appearance::Aqua => Self::default(),
            crate::settings::Appearance::Beer => Self {
                card: Color32::from_rgba_unmultiplied(55, 36, 12, 195),
                sidebar: Color32::from_rgba_unmultiplied(35, 23, 9, 205),
                accent: Color32::from_rgb(255, 199, 64),
                secondary: Color32::from_rgb(221, 142, 34),
            },
            crate::settings::Appearance::Disco => Self {
                card: Color32::from_rgba_unmultiplied(31, 19, 49, 180),
                sidebar: Color32::from_rgba_unmultiplied(20, 13, 34, 195),
                accent: DISCO_ACCENT,
                secondary: Color32::from_rgb(89, 216, 255),
            },
        }
    }

    pub fn with_appearance(mut self, appearance: crate::settings::Appearance) -> Self {
        let base = Self::for_appearance(appearance);
        self.accent = base.accent;
        self.secondary = base.secondary;
        self
    }

    pub fn from_image(image: &image::RgbaImage) -> Self {
        let Some(rgb) = vibrant_average(image) else {
            return Self::default();
        };
        let card = rgb.map(|channel| (14.0 + f32::from(channel) * 0.18).round() as u8);
        let sidebar = rgb.map(|channel| (8.0 + f32::from(channel) * 0.11).round() as u8);
        Self {
            card: Color32::from_rgba_unmultiplied(card[0], card[1], card[2], 210),
            sidebar: Color32::from_rgba_unmultiplied(sidebar[0], sidebar[1], sidebar[2], 220),
            ..Self::default()
        }
    }
}

pub fn accent(ctx: &egui::Context) -> Color32 {
    current(ctx).accent
}
pub fn secondary(ctx: &egui::Context) -> Color32 {
    current(ctx).secondary
}
pub fn glow(ctx: &egui::Context, alpha: u8) -> Color32 {
    let color = accent(ctx);
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

pub fn highlight(ctx: &egui::Context) -> Color32 {
    if accent(ctx) == crate::ACCENT {
        Color32::from_rgb(145, 204, 221)
    } else if accent(ctx) == DISCO_ACCENT {
        Color32::from_rgb(250, 205, 255)
    } else {
        Color32::from_rgb(255, 226, 158)
    }
}

pub fn ambient(ctx: &egui::Context, secondary: bool) -> Color32 {
    if accent(ctx) == crate::ACCENT {
        if secondary {
            Color32::from_rgba_unmultiplied(56, 120, 255, 35)
        } else {
            Color32::from_rgba_unmultiplied(0, 160, 180, 42)
        }
    } else if accent(ctx) == DISCO_ACCENT {
        if secondary {
            Color32::from_rgba_unmultiplied(45, 126, 245, 39)
        } else {
            Color32::from_rgba_unmultiplied(165, 30, 220, 48)
        }
    } else {
        glow(ctx, if secondary { 35 } else { 42 })
    }
}

pub fn cover_colors(ctx: &egui::Context) -> (Color32, Color32) {
    if accent(ctx) == crate::ACCENT {
        (
            Color32::from_rgb(22, 102, 133),
            Color32::from_rgb(8, 27, 53),
        )
    } else if accent(ctx) == DISCO_ACCENT {
        (
            Color32::from_rgb(106, 43, 139),
            Color32::from_rgb(24, 16, 52),
        )
    } else {
        (Color32::from_rgb(133, 88, 22), Color32::from_rgb(53, 34, 8))
    }
}

pub fn current(ctx: &egui::Context) -> GlassPalette {
    ctx.data_mut(|data| {
        data.get_temp::<GlassPalette>(egui::Id::new("glass-palette"))
            .unwrap_or_default()
    })
}

pub fn install(ctx: &egui::Context, palette: GlassPalette) {
    ctx.data_mut(|data| data.insert_temp(egui::Id::new("glass-palette"), palette));
    // Keep context operations outside the temporary-data lock.
    ctx.set_visuals(crate::dark_visuals_for(palette));
}

pub fn tint(ctx: &egui::Context, original: Color32) -> Color32 {
    let palette = current(ctx);
    if original == crate::CARD {
        palette.card
    } else if original == crate::SIDEBAR {
        palette.sidebar
    } else if original == crate::CARD.gamma_multiply(0.35) {
        palette.card.gamma_multiply(0.35)
    } else {
        original
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disco_photo_preserves_pink_and_cyan_accents_and_cool_highlights() {
        let image = image::RgbaImage::from_pixel(8, 8, image::Rgba([255, 180, 10, 255]));
        let palette =
            GlassPalette::from_image(&image).with_appearance(crate::settings::Appearance::Disco);
        assert!(palette.card.r() > palette.card.b());
        assert_eq!(palette.accent, DISCO_ACCENT);
        assert!(palette.secondary.b() > palette.secondary.r());
        let ctx = egui::Context::default();
        install(&ctx, palette);
        assert!(highlight(&ctx).b() > highlight(&ctx).g());
        let (top, bottom) = cover_colors(&ctx);
        assert!(top.b() > top.r() && bottom.b() > bottom.r());
        assert!(ambient(&ctx, true).b() > ambient(&ctx, true).r());
        assert_eq!(
            ctx.style_of(egui::Theme::Dark).visuals.hyperlink_color,
            DISCO_ACCENT
        );
    }

    #[test]
    fn beer_accents_remain_warm_when_custom_photo_tints_the_glass() {
        let image = image::RgbaImage::from_pixel(10, 10, image::Rgba([10, 20, 240, 255]));
        let palette =
            GlassPalette::from_image(&image).with_appearance(crate::settings::Appearance::Beer);
        assert!(palette.card.b() > palette.card.r());
        assert!(palette.accent.r() > palette.accent.g() && palette.accent.g() > palette.accent.b());
        let ctx = egui::Context::default();
        install(&ctx, palette);
        assert_eq!(
            ctx.style_of(egui::Theme::Dark).visuals.hyperlink_color,
            palette.accent
        );
        install(
            &ctx,
            GlassPalette::for_appearance(crate::settings::Appearance::Aqua),
        );
        assert_eq!(accent(&ctx), crate::ACCENT);
    }

    #[test]
    fn average_combines_vibrant_colors_with_alpha_weighting() {
        let mut image = image::RgbaImage::new(2, 1);
        image.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
        image.put_pixel(1, 0, image::Rgba([0, 0, 255, 128]));
        let color = vibrant_average(&image).unwrap();
        assert!(color[0] > color[2] && color[2] > 100);
        assert_eq!(color[1], 0);
    }

    #[test]
    fn vibrant_pixels_drive_tint_instead_of_neutral_or_transparent_pixels() {
        let mut image = image::RgbaImage::from_pixel(20, 20, image::Rgba([170, 170, 170, 255]));
        for x in 0..4 {
            image.put_pixel(x, 0, image::Rgba([245, 20, 30, 255]));
        }
        image.put_pixel(19, 19, image::Rgba([0, 255, 0, 0]));
        let color = vibrant_average(&image).unwrap();
        assert!(color[0] > 200 && color[1] < 50 && color[2] < 50);
        let palette = GlassPalette::from_image(&image);
        assert!(palette.card.r() > palette.card.g());
        assert!(palette.card.r() < 70 && palette.card.a() >= 200);
        assert_eq!(
            GlassPalette::from_image(&image::RgbaImage::new(10, 10)),
            GlassPalette::default()
        );
    }

    #[test]
    fn grayscale_fallback_is_neutral_and_reset_restores_original_glass() {
        let image = image::RgbaImage::from_pixel(12, 12, image::Rgba([120, 120, 120, 255]));
        assert_eq!(vibrant_average(&image), Some([120, 120, 120]));
        let ctx = egui::Context::default();
        install(&ctx, GlassPalette::from_image(&image));
        assert_ne!(tint(&ctx, crate::CARD), crate::CARD);
        install(&ctx, GlassPalette::default());
        assert_eq!(tint(&ctx, crate::CARD), crate::CARD);
        assert_eq!(tint(&ctx, crate::SIDEBAR), crate::SIDEBAR);
    }

    #[test]
    fn bright_background_detection_catches_white_images_and_ignores_dark_ones() {
        let white = image::RgbaImage::from_pixel(32, 24, image::Rgba([255, 255, 255, 255]));
        let dark = image::RgbaImage::from_pixel(32, 24, image::Rgba([20, 24, 30, 255]));
        assert!(background_is_bright(&white));
        assert!(!background_is_bright(&dark));
    }
}
