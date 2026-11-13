//! Local text contrast, resolved after egui has filled its deferred frame shapes.
//! Glyphs are painted once. Uniform light surfaces use dark ink; mixed photos
//! receive a softly feathered surface, only where the existing contrast fails.
use eframe::egui::{self, Color32, LayerId, Pos2, Rect, Shape, Vec2};
use std::sync::Arc;

#[derive(Clone, Copy, Debug)]
struct Colors {
    low: [u8; 3],
    high: [u8; 3],
}

impl Colors {
    fn solid(color: Color32) -> Self {
        let [r, g, b, _] = color.to_srgba_unmultiplied();
        Self {
            low: [r, g, b],
            high: [r, g, b],
        }
    }
    fn include(&mut self, other: Self) {
        for c in 0..3 {
            self.low[c] = self.low[c].min(other.low[c]);
            self.high[c] = self.high[c].max(other.high[c]);
        }
    }
    fn over(self, foreground: Color32) -> Self {
        let [r, g, b, a] = foreground.to_srgba_unmultiplied();
        let rgb = [r, g, b];
        let blend = |background: [u8; 3]| {
            std::array::from_fn(|c| {
                ((u32::from(rgb[c]) * u32::from(a)
                    + u32::from(background[c]) * u32::from(255 - a)
                    + 127)
                    / 255) as u8
            })
        };
        Self {
            low: blend(self.low),
            high: blend(self.high),
        }
    }
    fn contrast(self, ink: Color32) -> f32 {
        let [r, g, b, _] = ink.to_srgba_unmultiplied();
        let text = luminance([r, g, b]);
        let low = luminance(self.low);
        let high = luminance(self.high);
        if (low..=high).contains(&text) {
            1.0
        } else {
            ratio(text, low).min(ratio(text, high))
        }
    }
}

fn luminance(rgb: [u8; 3]) -> f32 {
    let linear = |c: u8| {
        let value = f32::from(c) / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(rgb[0]) + 0.7152 * linear(rgb[1]) + 0.0722 * linear(rgb[2])
}
fn ratio(a: f32, b: f32) -> f32 {
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

struct Photo {
    columns: usize,
    rows: usize,
    cells: Vec<Colors>,
}
impl Photo {
    fn new(image: &image::RgbaImage) -> Self {
        let columns = (image.width() as usize).clamp(1, 128);
        let rows = (image.height() as usize).clamp(1, 96);
        let mut photo = Self {
            columns,
            rows,
            cells: vec![
                Colors {
                    low: [255; 3],
                    high: [0; 3]
                };
                columns * rows
            ],
        };
        // Cache conservative RGB ranges once per image. No decoding, image
        // sampling or GPU readback happens in the frame loop.
        for (x, y, pixel) in image.enumerate_pixels() {
            let cell = (y as usize * rows / image.height() as usize) * columns
                + x as usize * columns / image.width() as usize;
            let color = Color32::from_rgba_unmultiplied(pixel[0], pixel[1], pixel[2], pixel[3]);
            photo.cells[cell].include(Colors::solid(crate::ABYSS).over(color));
        }
        if image.width() == 0 || image.height() == 0 {
            photo.cells.fill(Colors::solid(crate::ABYSS));
        }
        photo
    }
    fn colors(&self, uv: Rect) -> Colors {
        let x = |value: f32| {
            ((value.clamp(0.0, 1.0) * self.columns as f32) as usize).min(self.columns - 1)
        };
        let y =
            |value: f32| ((value.clamp(0.0, 1.0) * self.rows as f32) as usize).min(self.rows - 1);
        let mut colors = self.cells[y(uv.top()) * self.columns + x(uv.left())];
        for row in y(uv.top())..=y(uv.bottom()) {
            for column in x(uv.left())..=x(uv.right()) {
                colors.include(self.cells[row * self.columns + column]);
            }
        }
        colors
    }
}

#[derive(Clone)]
struct Scene {
    root: LayerId,
    start: usize,
    rect: Rect,
    uv: Option<Rect>,
    photo: Option<Arc<Photo>>,
}
impl Scene {
    fn colors(&self, rect: Rect) -> Colors {
        let (Some(uv), Some(photo)) = (self.uv, &self.photo) else {
            return Colors::solid(crate::ABYSS);
        };
        let rect = rect.intersect(self.rect);
        if !rect.is_positive() {
            return Colors::solid(crate::ABYSS);
        }
        let map = |p: Pos2| uv.min + ((p - self.rect.min) / self.rect.size()) * uv.size();
        photo.colors(Rect::from_min_max(map(rect.min), map(rect.max)))
    }
}

pub fn set_photo(ctx: &egui::Context, image: &image::RgbaImage) {
    let photo = Arc::new(Photo::new(image));
    ctx.data_mut(|data| data.insert_temp(egui::Id::new("readability-photo"), photo));
}

/// Call after the photo/ambient pass, before panels, widgets and text.
pub fn begin_frame(ui: &egui::Ui, uv: Option<Rect>) {
    let start = ui.painter().add(Shape::Noop).0 + 1;
    let photo = ui
        .ctx()
        .data(|data| data.get_temp::<Arc<Photo>>(egui::Id::new("readability-photo")));
    let scene = Scene {
        root: ui.layer_id(),
        start,
        rect: ui.max_rect(),
        uv,
        photo,
    };
    let scene_id = egui::Id::new("readability-scene").with(ui.ctx().viewport_id());
    ui.ctx().data_mut(|data| data.insert_temp(scene_id, scene));
}

struct Surface {
    rect: Rect,
    clip: Rect,
    fill: Color32,
    menu: bool,
}

fn backing(colors: Colors, inks: &[Color32], target: f32, tint: Color32) -> Option<Color32> {
    let fits = |fill: Color32| {
        inks.iter()
            .all(|ink| colors.over(fill).contrast(*ink) >= target)
    };
    let dark = Color32::from_rgb(tint.r(), tint.g(), tint.b());
    // Pick one surface for the complete text run, preserving emphasis colors.
    for rgb in [dark, Color32::from_rgb(246, 247, 249)] {
        if !fits(Color32::from_rgba_unmultiplied(
            rgb.r(),
            rgb.g(),
            rgb.b(),
            248,
        )) {
            continue;
        }
        let (mut low, mut high) = (0u8, 248u8);
        while low < high {
            let mid = low + (high - low) / 2;
            if fits(Color32::from_rgba_unmultiplied(
                rgb.r(),
                rgb.g(),
                rgb.b(),
                mid,
            )) {
                high = mid;
            } else {
                low = mid + 1;
            }
        }
        return Some(Color32::from_rgba_unmultiplied(
            rgb.r(),
            rgb.g(),
            rgb.b(),
            high,
        ));
    }
    None
}

fn adjust(
    shape: &Shape,
    clip: Rect,
    transform: egui::emath::TSTransform,
    scene: &Scene,
    surfaces: &mut Vec<Surface>,
    tint: Color32,
    menu_fills: &[Color32],
) -> Option<Shape> {
    match shape {
        // A textured rectangle's fill is its modulation color (often opaque
        // white), not the actual pixels. Frost grain must not hide its panel.
        Shape::Rect(rect)
            if rect.fill.a() > 0 && rect.blur_width == 0.0 && rect.brush.is_none() =>
        {
            surfaces.push(Surface {
                rect: rect.rect,
                clip,
                fill: rect.fill,
                menu: menu_fills.contains(&rect.fill),
            });
            None
        }
        Shape::Vec(shapes) => {
            let changes: Vec<_> = shapes
                .iter()
                .map(|shape| adjust(shape, clip, transform, scene, surfaces, tint, menu_fills))
                .collect();
            changes.iter().any(Option::is_some).then(|| {
                Shape::Vec(
                    shapes
                        .iter()
                        .zip(changes)
                        .map(|(original, changed)| changed.unwrap_or_else(|| original.clone()))
                        .collect(),
                )
            })
        }
        Shape::Text(text)
            if text.opacity_factor >= 0.95
                && text.angle == 0.0
                && text.galley.text().chars().any(char::is_alphanumeric) =>
        {
            let rect = text.visual_bounding_rect().intersect(clip);
            if !rect.is_positive() {
                return None;
            }
            // Menus already provide their own dark surface. Preserve their
            // white text and intentional secondary/accent colors regardless
            // of the photograph behind the menu.
            if surfaces.iter().any(|surface| {
                surface.menu && surface.rect.contains_rect(rect) && surface.clip.contains_rect(rect)
            }) {
                return None;
            }
            let mut colors = scene.colors(transform * rect);
            for surface in surfaces.iter() {
                if surface.rect.contains_rect(rect) && surface.clip.contains_rect(rect) {
                    colors = colors.over(surface.fill);
                } else if surface.rect.intersect(surface.clip).intersects(rect) {
                    let covered = colors.over(surface.fill);
                    colors.include(covered);
                }
            }
            let mut inks = Vec::new();
            let mut font_size = f32::INFINITY;
            for section in &text.galley.job.sections {
                let color = text.override_text_color.unwrap_or(
                    if section.format.color == Color32::PLACEHOLDER {
                        text.fallback_color
                    } else {
                        section.format.color
                    },
                );
                if color.a() < 240 {
                    return None;
                }
                if !inks.contains(&color) {
                    inks.push(color);
                }
                font_size = font_size.min(section.format.font_id.size);
            }
            if inks.is_empty() {
                return None;
            }
            let target = if font_size >= 24.0 { 3.2 } else { 4.8 };
            if inks.iter().all(|ink| colors.contrast(*ink) >= target) {
                return None;
            }
            if inks.len() == 1 {
                let dark = Color32::from_rgb(24, 28, 34);
                // On a consistently bright region, a clean dark glyph needs no
                // surface at all. Mixed light/dark regions use a soft backing.
                if colors.contrast(dark) >= target {
                    return Some(Shape::Text(text.clone().with_override_text_color(dark)));
                }
            }
            let fill = backing(colors, &inks, target, tint)?;
            let scrim = egui::epaint::RectShape::filled(
                text.visual_bounding_rect().expand2(Vec2::new(12.0, 7.0)),
                8.0,
                fill,
            )
            .with_blur_width(12.0);
            Some(Shape::Vec(vec![
                Shape::Rect(scrim),
                Shape::Text(text.clone()),
            ]))
        }
        _ => None,
    }
}

/// Resolve the current layout, including panels whose fills were deferred by
/// egui::Frame. Paint changes never alter hit targets, text layout or labels.
pub fn finish_frame(ctx: &egui::Context) {
    let scene_id = egui::Id::new("readability-scene").with(ctx.viewport_id());
    let Some(scene) = ctx.data(|data| data.get_temp::<Scene>(scene_id)) else {
        return;
    };
    let palette = crate::theme::current(ctx);
    let menu_fills = [palette.card, palette.sidebar];
    let tint = palette.sidebar.to_srgba_unmultiplied();
    let tint = Color32::from_rgb(tint[0], tint[1], tint[2]);
    let mut layers = ctx.memory(|memory| memory.layer_ids().collect::<Vec<_>>());
    if !layers.contains(&scene.root) {
        layers.push(scene.root);
    }
    let layers: Vec<_> = layers
        .into_iter()
        .map(|layer| {
            (
                layer,
                ctx.layer_transform_to_global(layer).unwrap_or_default(),
            )
        })
        .collect();
    ctx.graphics_mut(|graphics| {
        for (layer, transform) in layers {
            let Some(list) = graphics.get_mut(layer) else {
                continue;
            };
            let mut surfaces = Vec::new();
            let start = if layer == scene.root { scene.start } else { 0 };
            let changes: Vec<_> = list
                .all_entries()
                .enumerate()
                .skip(start)
                .filter_map(|(index, entry)| {
                    adjust(
                        &entry.shape,
                        entry.clip_rect,
                        transform,
                        &scene,
                        &mut surfaces,
                        tint,
                        &menu_fills,
                    )
                    .map(|shape| (egui::layers::ShapeIdx(index), shape))
                })
                .collect();
            for (index, shape) in changes {
                list.mutate_shape(index, |entry| entry.shape = shape);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    fn texts<'a>(shape: &'a Shape, output: &mut Vec<&'a egui::epaint::TextShape>) {
        match shape {
            Shape::Text(text) => output.push(text),
            Shape::Vec(shapes) => {
                for shape in shapes {
                    texts(shape, output)
                }
            }
            _ => {}
        }
    }
    fn blurred(shape: &Shape) -> usize {
        match shape {
            Shape::Rect(rect) => usize::from(rect.blur_width > 0.0),
            Shape::Vec(shapes) => shapes.iter().map(blurred).sum(),
            _ => 0,
        }
    }
    fn render(image: &image::RgbaImage, panel: bool) -> egui::FullOutput {
        let ctx = egui::Context::default();
        set_photo(&ctx, image);
        crate::test_support::run_ui(
            &ctx,
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 200.0))),
                ..Default::default()
            },
            |ui| {
                begin_frame(
                    ui,
                    Some(Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0))),
                );
                let label = |ui: &mut egui::Ui| {
                    ui.label(
                        egui::RichText::new("Albumele mele")
                            .size(29.0)
                            .strong()
                            .color(crate::WHITE),
                    );
                };
                if panel {
                    egui::Frame::new()
                        .fill(Color32::from_rgba_unmultiplied(20, 22, 30, 240))
                        .show(ui, label);
                } else {
                    label(ui);
                }
                finish_frame(&ctx);
            },
        )
    }
    #[test]
    fn white_photo_has_one_clean_dark_text_run() {
        let output = render(
            &image::RgbaImage::from_pixel(400, 200, image::Rgba([255; 4])),
            false,
        );
        let mut runs = Vec::new();
        for entry in &output.shapes {
            texts(&entry.shape, &mut runs);
        }
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].galley.text(), "Albumele mele");
        assert_eq!(
            runs[0].override_text_color,
            Some(Color32::from_rgb(24, 28, 34))
        );
        assert_eq!(
            output
                .shapes
                .iter()
                .map(|entry| blurred(&entry.shape))
                .sum::<usize>(),
            0
        );
    }
    #[test]
    fn mixed_photo_has_one_text_run_over_a_soft_surface() {
        let image = image::RgbaImage::from_fn(400, 200, |x, _| {
            if x % 20 < 10 {
                image::Rgba([255; 4])
            } else {
                image::Rgba([10, 10, 10, 255])
            }
        });
        let output = render(&image, false);
        let mut runs = Vec::new();
        for entry in &output.shapes {
            texts(&entry.shape, &mut runs);
        }
        assert_eq!(runs.len(), 1);
        assert!(runs[0].override_text_color.is_none());
        assert_eq!(
            output
                .shapes
                .iter()
                .map(|entry| blurred(&entry.shape))
                .sum::<usize>(),
            1
        );
        let range = Colors {
            low: [0; 3],
            high: [255; 3],
        };
        let fill = backing(range, &[crate::MUTED], 4.8, Color32::from_rgb(24, 28, 34)).unwrap();
        assert!(range.over(fill).contrast(crate::MUTED) >= 4.8);
    }
    #[test]
    fn dark_deferred_panel_needs_no_ink_change_or_backing() {
        let output = render(
            &image::RgbaImage::from_pixel(400, 200, image::Rgba([255; 4])),
            true,
        );
        let mut runs = Vec::new();
        for entry in &output.shapes {
            texts(&entry.shape, &mut runs);
        }
        assert_eq!(runs.len(), 1);
        assert!(runs[0].override_text_color.is_none());
        assert_eq!(
            output
                .shapes
                .iter()
                .map(|entry| blurred(&entry.shape))
                .sum::<usize>(),
            0
        );
    }
    #[test]
    fn beer_panels_keep_light_text_over_a_white_photo() {
        let ctx = egui::Context::default();
        crate::theme::install(
            &ctx,
            crate::GlassPalette::for_appearance(crate::Appearance::Beer),
        );
        set_photo(
            &ctx,
            &image::RgbaImage::from_pixel(800, 600, image::Rgba([255; 4])),
        );
        let frost = crate::load_frost(&ctx);
        let output = crate::test_support::run_ui(
            &ctx,
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))),
                ..Default::default()
            },
            |ui| {
                begin_frame(
                    ui,
                    Some(Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0))),
                );
                egui::Panel::bottom("footer")
                    .exact_size(100.0)
                    .frame(egui::Frame::new())
                    .show(ui, |ui| {
                        crate::glass_frame(ui, crate::SIDEBAR, 18, 12).show(ui, |ui| {
                            crate::paint_frost(ui, &frost);
                            ui.label(egui::RichText::new("Footer").color(crate::WHITE));
                        });
                    });
                egui::Panel::left("sidebar")
                    .exact_size(200.0)
                    .frame(crate::glass_frame(ui, crate::SIDEBAR, 18, 15))
                    .show(ui, |ui| {
                        crate::paint_frost(ui, &frost);
                        ui.label(egui::RichText::new("Sidebar").color(crate::WHITE));
                        ui.label(egui::RichText::new("Secondary menu text").color(crate::MUTED));
                    });
                egui::CentralPanel::default()
                    .frame(egui::Frame::new())
                    .show(ui, |ui| {
                        crate::glass_frame(ui, crate::CARD, 14, 10).show(ui, |ui| {
                            crate::paint_frost(ui, &frost);
                            egui::ScrollArea::vertical().show(ui, |ui| {
                                ui.label(egui::RichText::new("Table").color(crate::WHITE));
                            });
                        });
                    });
                finish_frame(&ctx);
            },
        );
        let mut runs = Vec::new();
        for entry in &output.shapes {
            texts(&entry.shape, &mut runs);
        }
        assert_eq!(runs.len(), 4);
        for run in runs {
            assert!(
                run.override_text_color.is_none(),
                "{} should retain light ink",
                run.galley.text()
            );
        }
    }
    #[test]
    fn small_bright_patch_and_crop_are_detected_locally() {
        let image = image::RgbaImage::from_fn(400, 200, |x, y| {
            if x < 240 && y < 60 {
                image::Rgba([255; 4])
            } else {
                image::Rgba([10, 10, 10, 255])
            }
        });
        assert!(!crate::theme::background_is_bright(&image));
        let output = render(&image, false);
        let mut runs = Vec::new();
        for entry in &output.shapes {
            texts(&entry.shape, &mut runs);
        }
        assert!(runs[0].override_text_color.is_some());
        let photo = Arc::new(Photo::new(&image));
        let scene = Scene {
            root: LayerId::background(),
            start: 0,
            rect: Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 200.0)),
            uv: Some(Rect::from_min_max(Pos2::new(0.7, 0.0), Pos2::new(1.0, 1.0))),
            photo: Some(photo),
        };
        assert!(
            scene
                .colors(Rect::from_min_size(
                    Pos2::new(10.0, 10.0),
                    Vec2::new(200.0, 30.0)
                ))
                .contrast(crate::WHITE)
                > 10.0
        );
    }
}
