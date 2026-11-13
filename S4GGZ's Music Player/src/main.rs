#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! S4GGZ Native — an actual desktop UI, not a browser or embedded WebView.
#[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
compile_error!("This edition of S4GGZ supports Windows x64 only.");

mod album_art;
#[cfg(test)]
mod album_art_ui_tests;
mod audio_analysis;
mod beer_hand;
mod bubble;
mod cover;
mod database;
mod deletion;
mod disco;
mod disco_ball;
mod disco_floor;
mod disco_optics;
mod disco_reflections;
mod display_scale;
mod downloader;
mod equalizer;
mod equalizer_ui;
mod fish;
mod footer_title;
mod music_search;
mod playback_modes;
#[cfg(test)]
mod playback_ui_tests;
mod player;
mod process_runner;
mod readability;
#[cfg(test)]
mod regression_tests;
mod settings;
mod sorting;
#[cfg(test)]
mod test_support;
mod theme;
mod track_table;
mod yt_dlp;

use beer_hand::BeerHandAnimation;
use bubble::BubbleSystem;
use database::{Album, Database, Page, Playlist, Track};
use disco::DiscoScene;
use downloader::Downloader;
use eframe::egui::{self, Color32, RichText, TextureHandle, Vec2};
use egui_extras::{Column, TableBuilder};
use fish::FishAnimation;
use player::{AudioPlayer, PlayerStatus};
use settings::{AppSettings, Appearance, BeerFrequency, DiscoMode, EffectDensity};
use sorting::{SortColumn, TrackSort};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
    time::{Duration, Instant},
};
use theme::GlassPalette;

const ACCENT: Color32 = Color32::from_rgb(0, 242, 254);
const BLUE: Color32 = Color32::from_rgb(79, 172, 254);
const MUTED: Color32 = Color32::from_rgb(159, 162, 193);
const ABYSS: Color32 = Color32::from_rgb(5, 8, 20);
// These RGB channels are already premultiplied by alpha, avoiding additive halos.
const SIDEBAR: Color32 = Color32::from_rgba_premultiplied(7, 16, 29, 170);
const CARD: Color32 = Color32::from_rgba_premultiplied(12, 28, 43, 170);
const WHITE: Color32 = Color32::from_rgb(244, 242, 252);
const PAGE_SIZE: i64 = 20;
const MUSIC_SCAN_INTERVAL: Duration = Duration::from_secs(3);
const ALBUM_CARD_SIZE: f32 = 190.0;
const ALBUM_CARD_GAP: f32 = 12.0;
const APP_DESCRIPTION: &str =
    "S4GGZ's Music Player, descarca-ti maneaua favorita si asculta online-offline fara nici un bai";

fn load_background(ctx: &egui::Context, appearance: Appearance) -> Option<TextureHandle> {
    for directory in theme_directories(appearance) {
        let Ok(image) = image::open(directory.join(appearance.background_file())) else {
            continue;
        };
        let image = image.to_rgba8();
        readability::set_photo(ctx, &image);
        return Some(ctx.load_texture(
            "application-background",
            egui::ColorImage::from_rgba_unmultiplied(
                [image.width() as usize, image.height() as usize],
                image.as_raw(),
            ),
            egui::TextureOptions::LINEAR,
        ));
    }
    None
}

fn load_frost(ctx: &egui::Context) -> TextureHandle {
    // A tiny, seamless noise texture is generated/uploaded once, never each frame.
    let mut seed = 0x6271_54ab_u32;
    let pixels = (0..64 * 64)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            Color32::from_white_alpha((seed % 9) as u8)
        })
        .collect();
    ctx.load_texture(
        "frost-grain",
        egui::ColorImage::new([64, 64], pixels),
        egui::TextureOptions {
            wrap_mode: egui::TextureWrapMode::Repeat,
            ..egui::TextureOptions::LINEAR
        },
    )
}

fn paint_frost_grain(ui: &egui::Ui, texture: &TextureHandle) {
    paint_frost_grain_in_rect(ui, texture, ui.max_rect());
}

fn paint_frost_grain_in_rect(ui: &egui::Ui, texture: &TextureHandle, rect: egui::Rect) {
    if !rect.is_positive() {
        return;
    }
    egui::Image::new((texture.id(), rect.size()))
        .uv(egui::Rect::from_min_max(
            egui::Pos2::ZERO,
            (rect.size() / 64.0).to_pos2(),
        ))
        .corner_radius(12.0)
        .paint_at(ui, rect);
}

fn paint_frost(ui: &egui::Ui, texture: &TextureHandle) {
    let rect = ui.max_rect();
    paint_frost_grain(ui, texture);
    specular_edge(ui.painter(), rect.expand(12.0), 16.0, 0.8);
}

fn glass_edge() -> egui::Stroke {
    egui::Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 35))
}

fn glass_frame(ui: &egui::Ui, fill: Color32, radius: u8, margin: i8) -> egui::Frame {
    egui::Frame::new()
        .fill(theme::tint(ui.ctx(), fill))
        .corner_radius(radius)
        .inner_margin(margin)
        .stroke(glass_edge())
        .shadow(egui::epaint::Shadow {
            offset: [0, 6],
            blur: 22,
            spread: 0,
            color: Color32::from_black_alpha(65),
        })
}

fn specular_edge(painter: &egui::Painter, rect: egui::Rect, radius: f32, strength: f32) {
    let inset = radius.min(rect.width() * 0.4);
    painter.line_segment(
        [
            egui::pos2(rect.left() + inset, rect.top() + 1.0),
            egui::pos2(rect.right() - inset, rect.top() + 1.0),
        ],
        egui::Stroke::new(1.0, Color32::from_white_alpha((45.0 * strength) as u8)),
    );
}

/// One small triangle fan gives rounded geometry and a vertex-color gradient.
/// This uses the existing egui render pass: no render targets, image uploads or blur shaders.
fn gradient_tile(
    painter: &egui::Painter,
    rect: egui::Rect,
    radius: f32,
    top: Color32,
    bottom: Color32,
) {
    if !rect.is_positive() || !painter.is_visible() {
        return;
    }
    let radius = radius.min(rect.width() * 0.5).min(rect.height() * 0.5);
    let mut mesh = egui::epaint::Mesh::default();
    mesh.vertices.reserve(37);
    mesh.indices.reserve(108);
    let color_at = |p: egui::Pos2| {
        let t = ((p.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
        let a = top.to_array();
        let b = bottom.to_array();
        Color32::from_rgba_premultiplied(
            egui::lerp(a[0] as f32..=b[0] as f32, t) as u8,
            egui::lerp(a[1] as f32..=b[1] as f32, t) as u8,
            egui::lerp(a[2] as f32..=b[2] as f32, t) as u8,
            egui::lerp(a[3] as f32..=b[3] as f32, t) as u8,
        )
    };
    mesh.colored_vertex(rect.center(), color_at(rect.center()));
    for (center, start) in [
        (rect.right_top() + Vec2::new(-radius, radius), -90.0_f32),
        (rect.right_bottom() - Vec2::splat(radius), 0.0),
        (rect.left_bottom() + Vec2::new(radius, -radius), 90.0),
        (rect.left_top() + Vec2::splat(radius), 180.0),
    ] {
        for step in 0..=8 {
            let angle = (start + step as f32 * 90.0 / 8.0).to_radians();
            let point = center + Vec2::angled(angle) * radius;
            mesh.colored_vertex(point, color_at(point));
        }
    }
    for i in 1..=36 {
        mesh.add_triangle(0, i, if i == 36 { 1 } else { i + 1 });
    }
    painter.add(egui::Shape::mesh(mesh));
}

/// A feathered, elliptical light pool underneath transparent panels suggests
/// frosted refraction without pretending to sample or blur the desktop behind us.
fn ambient_light(painter: &egui::Painter, center: egui::Pos2, size: Vec2, color: Color32) {
    let mut mesh = egui::epaint::Mesh::default();
    mesh.vertices.reserve(49);
    mesh.indices.reserve(144);
    mesh.colored_vertex(center, color);
    for i in 0..48 {
        let direction = Vec2::angled(i as f32 * std::f32::consts::TAU / 48.0);
        mesh.colored_vertex(center + direction * size, Color32::TRANSPARENT);
    }
    for i in 1..=48 {
        mesh.add_triangle(0, i, if i == 48 { 1 } else { i + 1 });
    }
    painter.add(egui::Shape::mesh(mesh));
}

fn paint_ambient(ui: &egui::Ui, background: Option<&TextureHandle>) {
    let rect = ui.max_rect();
    let painter = ui.painter();
    painter.rect_filled(rect, 0, ABYSS);
    let mut photo_uv = None;
    if let Some(texture) = background {
        let image_size = texture.size_vec2();
        let target_aspect = rect.width() / rect.height();
        let image_aspect = image_size.x / image_size.y;
        let uv = if target_aspect > image_aspect {
            let visible_height = image_aspect / target_aspect;
            egui::Rect::from_min_max(
                egui::pos2(0.0, (1.0 - visible_height) * 0.5),
                egui::pos2(1.0, (1.0 + visible_height) * 0.5),
            )
        } else {
            let visible_width = target_aspect / image_aspect;
            egui::Rect::from_min_max(
                egui::pos2((1.0 - visible_width) * 0.5, 0.0),
                egui::pos2((1.0 + visible_width) * 0.5, 1.0),
            )
        };
        painter.image(texture.id(), rect, uv, Color32::WHITE);
        photo_uv = Some(uv);
    }
    ambient_light(
        painter,
        rect.left_top() + rect.size() * Vec2::new(0.19, 0.15),
        rect.size() * Vec2::new(0.6, 0.75),
        theme::ambient(ui.ctx(), false),
    );
    ambient_light(
        painter,
        rect.left_top() + rect.size() * Vec2::new(0.85, 0.7),
        rect.size() * Vec2::new(0.55, 0.65),
        theme::ambient(ui.ctx(), true),
    );
    readability::begin_frame(ui, photo_uv);
}

/// Pointer, keyboard and focus semantics are retained even though the rail and
/// thumb are custom painted. Seek commits remain owned by the existing footer.
fn liquid_slider(
    ui: &mut egui::Ui,
    value: &mut f64,
    max: f64,
    width: f32,
    name: &str,
) -> egui::Response {
    let (rect, mut response) = ui.allocate_exact_size(
        Vec2::new(width.max(20.0), 28.0),
        egui::Sense::click_and_drag(),
    );
    let rail = egui::Rect::from_center_size(
        rect.center(),
        Vec2::new((rect.width() - 16.0).max(1.0), 5.0),
    );
    let old = *value;
    if let Some(pointer) = response
        .interact_pointer_pos()
        .filter(|_| response.clicked() || response.dragged())
    {
        response.request_focus();
        *value = (((pointer.x - rail.left()) / rail.width()).clamp(0.0, 1.0) as f64) * max;
    }
    if response.has_focus() {
        ui.input(|input| {
            let step = if max <= 1.0 { 0.02 } else { 5.0 };
            if input.key_pressed(egui::Key::ArrowLeft) {
                *value -= step;
            }
            if input.key_pressed(egui::Key::ArrowRight) {
                *value += step;
            }
            if input.key_pressed(egui::Key::Home) {
                *value = 0.0;
            }
            if input.key_pressed(egui::Key::End) {
                *value = max;
            }
        });
    }
    *value = value.clamp(0.0, max);
    if *value != old {
        response.mark_changed();
    }
    response.widget_info(|| egui::WidgetInfo::slider(ui.is_enabled(), *value, name));
    let active = ui.ctx().animate_bool_with_time(
        response.id.with("aura"),
        response.hovered() || response.dragged() || response.has_focus(),
        0.16,
    );
    let painter = ui.painter();
    painter.rect_filled(rail.expand(2.0), 5, theme::glow(ui.ctx(), 12));
    painter.rect_filled(rail, 3, Color32::from_black_alpha(95));
    let x = egui::lerp(rail.left()..=rail.right(), (*value / max.max(0.01)) as f32);
    let filled = egui::Rect::from_min_max(rail.min, egui::pos2(x, rail.bottom()));
    gradient_tile(
        painter,
        filled,
        3.0,
        theme::accent(ui.ctx()),
        theme::secondary(ui.ctx()),
    );
    let center = egui::pos2(x, rail.center().y);
    painter.circle_filled(
        center,
        10.0 + active * 4.0,
        theme::accent(ui.ctx()).gamma_multiply(0.08 + active * 0.06),
    );
    painter.circle_filled(
        center,
        7.0 + active,
        theme::accent(ui.ctx()).gamma_multiply(0.12),
    );
    gradient_tile(
        painter,
        egui::Rect::from_center_size(center, Vec2::splat(10.0 + active * 2.0)),
        8.0,
        WHITE,
        theme::highlight(ui.ctx()),
    );
    painter.circle_filled(center + Vec2::new(-1.3, -1.6), 1.5, Color32::WHITE);
    response.on_hover_text(name)
}

fn play_button(ui: &mut egui::Ui, paused: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(48.0), egui::Sense::click());
    BubbleSystem::register_button(ui.ctx(), &response);
    let hover = ui.ctx().animate_bool_with_time(
        response.id.with("light"),
        response.hovered() || response.has_focus(),
        0.18,
    );
    let center = rect.center();
    let painter = ui.painter();
    painter.circle_filled(
        center + Vec2::new(0.0, 3.0),
        25.0,
        Color32::from_black_alpha(70),
    );
    painter.circle_filled(
        center,
        25.0 + hover * 2.0,
        theme::accent(ui.ctx()).gamma_multiply(0.10 + hover * 0.08),
    );
    gradient_tile(
        painter,
        rect.shrink(2.0 - hover),
        24.0,
        theme::accent(ui.ctx()),
        theme::secondary(ui.ctx()),
    );
    painter.circle_stroke(center, 22.0 + hover, glass_edge());
    painter.line_segment(
        [
            center + Vec2::new(-9.0, -16.0),
            center + Vec2::new(9.0, -16.0),
        ],
        egui::Stroke::new(1.0, Color32::from_white_alpha(140)),
    );
    // Geometric transport glyphs remain crisp even when the system font lacks symbols.
    if paused {
        painter.add(egui::Shape::convex_polygon(
            vec![
                center + Vec2::new(-5.0, -8.0),
                center + Vec2::new(8.0, 0.0),
                center + Vec2::new(-5.0, 8.0),
            ],
            WHITE,
            egui::Stroke::NONE,
        ));
    } else {
        for x in [-5.0, 5.0] {
            painter.rect_filled(
                egui::Rect::from_center_size(center + Vec2::new(x, 0.0), Vec2::new(3.0, 15.0)),
                1,
                WHITE,
            );
        }
    }
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            if paused { "Play" } else { "Pause" },
        )
    });
    response.on_hover_text(if paused { "Redă" } else { "Pauză" })
}

/// Asset-backed buttons share one animated glass treatment, including keyboard focus.
struct GlassButton {
    text: String,
    texture: Option<egui::TextureId>,
    icon: &'static str,
    fill: Color32,
    radius: f32,
}

impl GlassButton {
    fn fill(mut self, fill: Color32) -> Self {
        self.fill = fill;
        self
    }
    fn corner_radius(mut self, radius: f32) -> Self {
        self.radius = radius;
        self
    }
}

impl egui::Widget for GlassButton {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let galley =
            ui.painter()
                .layout_no_wrap(self.text.clone(), egui::FontId::proportional(13.0), WHITE);
        let icon_width = if self.text.is_empty() { 18.0 } else { 26.0 };
        let desired = Vec2::new(galley.size().x + icon_width + 20.0, 32.0);
        let (rect, response) = ui.allocate_at_least(desired, egui::Sense::click());
        BubbleSystem::register_button(ui.ctx(), &response);
        let hover = ui.ctx().animate_bool_with_time(
            response.id.with("glass"),
            response.hovered() || response.has_focus(),
            0.18,
        );
        let pressed = ui.ctx().animate_bool_with_time(
            response.id.with("press"),
            response.is_pointer_button_down_on(),
            0.1,
        );
        let painter = ui.painter();
        painter.rect_filled(rect, self.radius, theme::tint(ui.ctx(), self.fill));
        gradient_tile(
            painter,
            rect,
            self.radius,
            theme::glow(ui.ctx(), (10.0 + hover * 25.0 + pressed * 20.0) as u8),
            theme::accent(ui.ctx()).gamma_multiply(hover * 0.06),
        );
        painter.rect_stroke(
            rect.shrink(0.5),
            self.radius,
            glass_edge(),
            egui::StrokeKind::Inside,
        );
        specular_edge(painter, rect, self.radius, 0.7 + hover);
        let start = rect.center().x - (galley.size().x + icon_width) * 0.5;
        let icon_rect = egui::Rect::from_center_size(
            egui::pos2(start + 9.0, rect.center().y),
            Vec2::splat(18.0 + hover),
        );
        if let Some(texture) = self.texture {
            painter.image(
                texture,
                icon_rect,
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        } else {
            paint_section_icon(painter, icon_rect, self.icon);
        }
        painter.galley(
            egui::pos2(start + icon_width, rect.center().y - galley.size().y * 0.5),
            galley,
            WHITE,
        );
        response.widget_info(|| {
            let accessible_name = if self.text.is_empty() {
                match self.icon {
                    "queue" => "Coada de redare",
                    "close" => "Închide meniul",
                    "arrow-up" => "Mută piesa mai sus",
                    "arrow-down" => "Mută piesa mai jos",
                    _ => self.icon,
                }
            } else {
                &self.text
            };
            egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), accessible_name)
        });
        response
    }
}

/// Small native vector fallbacks avoid font-dependent missing-glyph boxes.
/// User-supplied assets still take priority over these neutral outline symbols.
fn paint_section_icon(painter: &egui::Painter, rect: egui::Rect, name: &str) {
    let c = rect.center();
    let s = egui::Stroke::new(1.3, WHITE);
    let p = |x: f32, y: f32| c + Vec2::new(x, y) * (rect.width() / 18.0);
    match name {
        "close" => {
            painter.line_segment([p(-5., -5.), p(5., 5.)], s);
            painter.line_segment([p(-5., 5.), p(5., -5.)], s);
        }
        "arrow-up" | "arrow-down" | "arrow-first" | "arrow-last" => {
            let direction = if matches!(name, "arrow-up" | "arrow-first") {
                -1.
            } else {
                1.
            };
            painter.line_segment([p(0., -6. * direction), p(0., 6. * direction)], s);
            painter.line_segment([p(-5., direction), p(0., 6. * direction)], s);
            painter.line_segment([p(5., direction), p(0., 6. * direction)], s);
            if matches!(name, "arrow-first" | "arrow-last") {
                painter.line_segment([p(-5., 8. * direction), p(5., 8. * direction)], s);
            }
        }
        "trash" => {
            painter.line_segment([p(-6., -4.), p(6., -4.)], s);
            painter.line_segment([p(-2., -7.), p(2., -7.)], s);
            painter.add(egui::Shape::line(
                vec![p(-5., -4.), p(-4., 7.), p(4., 7.), p(5., -4.)],
                s,
            ));
            for x in [-1.5, 1.5] {
                painter.line_segment([p(x, -1.), p(x, 4.)], s);
            }
        }
        "equalizer" => {
            for (x, y) in [(-5., -3.), (0., 3.), (5., -1.)] {
                painter.line_segment([p(x, -7.), p(x, 7.)], s);
                painter.line_segment([p(x - 2., y), p(x + 2., y)], egui::Stroke::new(2.5, WHITE));
            }
        }
        "home" => {
            painter.add(egui::Shape::line(
                vec![p(-7.0, -1.0), p(0.0, -7.0), p(7.0, -1.0)],
                s,
            ));
            painter.add(egui::Shape::line(
                vec![p(-5.0, -2.0), p(-5.0, 6.0), p(5.0, 6.0), p(5.0, -2.0)],
                s,
            ));
        }
        "albums" => {
            for x in [-4.0, 4.0] {
                for y in [-4.0, 4.0] {
                    painter.rect_stroke(
                        egui::Rect::from_center_size(p(x, y), Vec2::splat(5.0)),
                        1,
                        s,
                        egui::StrokeKind::Inside,
                    );
                }
            }
        }
        "songs" => {
            painter.circle_filled(p(-4.0, 5.0), 2.5, WHITE);
            painter.line_segment([p(-1.5, 5.0), p(-1.5, -6.0)], s);
            painter.line_segment([p(-1.5, -6.0), p(5.0, -4.0)], s);
        }
        "favorites" => {
            painter.add(egui::Shape::closed_line(
                vec![
                    p(0.0, 6.0),
                    p(-6.0, 0.0),
                    p(-6.0, -4.0),
                    p(-3.0, -6.0),
                    p(0.0, -3.0),
                    p(3.0, -6.0),
                    p(6.0, -4.0),
                    p(6.0, 0.0),
                ],
                s,
            ));
        }
        "add-music" => {
            painter.rect_stroke(
                egui::Rect::from_min_max(p(-7.0, -5.0), p(7.0, 6.0)),
                2,
                s,
                egui::StrokeKind::Inside,
            );
            painter.line_segment([p(-3.0, 0.0), p(3.0, 0.0)], s);
            painter.line_segment([p(0.0, -3.0), p(0.0, 3.0)], s);
        }
        "settings" => {
            painter.circle_stroke(c, 4.0, s);
            for i in 0..8 {
                let d = Vec2::angled(i as f32 * std::f32::consts::TAU / 8.0);
                painter.line_segment([c + d * 5.0, c + d * 7.0], s);
            }
        }
        "rescan" => {
            painter.circle_stroke(c, 5.0, s);
            painter.add(egui::Shape::line(
                vec![p(2.0, -7.0), p(6.0, -5.0), p(3.0, -2.0)],
                s,
            ));
        }
        "fish" | "fish-off" => {
            let body = (0..24)
                .map(|index| {
                    let angle = index as f32 * std::f32::consts::TAU / 24.0;
                    p(1.5 + angle.cos() * 5.5, angle.sin() * 3.8)
                })
                .collect();
            painter.add(egui::Shape::closed_line(body, s));
            painter.add(egui::Shape::closed_line(
                vec![p(-4.0, 0.0), p(-8.0, -4.5), p(-8.0, 4.5)],
                s,
            ));
            painter.circle_filled(p(4.0, -1.0), rect.width() / 22.0, WHITE);
            painter.line_segment([p(0.0, 0.0), p(2.0, 2.0)], s);
        }
        "bubble" | "bubble-off" => {
            painter.circle_stroke(c, rect.width() * 0.38, s);
            painter.circle_stroke(p(-1.7, -2.5), rect.width() * 0.12, s);
        }
        "disco-ball" | "disco-ball-off" => {
            painter.line_segment([p(0.0, -9.0), p(0.0, -6.0)], s);
            painter.circle_stroke(p(0.0, 1.0), rect.width() * 0.36, s);
            for x in [-2.5, 2.5] {
                painter.line_segment([p(x, -4.0), p(x, 6.0)], s);
            }
            for y in [-1.5, 2.5] {
                painter.line_segment([p(-5.5, y), p(5.5, y)], s);
            }
        }
        "disco-floor" | "disco-floor-off" => {
            let point = |u: f32, v: f32| p((u - 0.5) * (9.0 + 7.0 * v), -5.0 + v * 12.0);
            for value in [0.0, 0.33, 0.67, 1.0] {
                painter.line_segment([point(value, 0.0), point(value, 1.0)], s);
                painter.line_segment([point(0.0, value), point(1.0, value)], s);
            }
        }
        "beer" | "beer-off" | "beer-hand" | "beer-hand-off" => {
            let hand = matches!(name, "beer-hand" | "beer-hand-off");
            let x = if hand { -2.0 } else { -1.0 };
            let bottom = if hand { 2.0 } else { 6.0 };
            painter.add(egui::Shape::closed_line(
                vec![
                    p(x - 4.0, -4.0),
                    p(x - 4.0, bottom),
                    p(x + 3.0, bottom),
                    p(x + 3.0, -4.0),
                ],
                s,
            ));
            painter.add(egui::Shape::line(
                vec![
                    p(x + 3.0, -2.5),
                    p(x + 6.0, -2.5),
                    p(x + 6.0, 1.0),
                    p(x + 3.0, 1.0),
                ],
                s,
            ));
            // Scalloped foam and glass ribs, all drawn in the same neutral color.
            painter.add(egui::Shape::line(
                vec![
                    p(x - 4.0, -4.0),
                    p(x - 4.0, -6.0),
                    p(x - 2.5, -7.0),
                    p(x - 1.0, -6.0),
                    p(x + 0.5, -7.0),
                    p(x + 2.0, -6.0),
                    p(x + 3.0, -6.0),
                    p(x + 3.0, -4.0),
                ],
                s,
            ));
            for offset in [-1.5, 1.0] {
                painter.line_segment([p(x + offset, -2.5), p(x + offset, bottom - 1.5)], s);
            }
            if hand {
                // Open palm below the mug, thumb and wrist entering from the right.
                painter.add(egui::Shape::line(
                    vec![
                        p(8.0, 3.0),
                        p(4.0, 3.0),
                        p(1.5, 1.0),
                        p(0.0, 1.5),
                        p(1.5, 4.0),
                        p(-4.5, 4.0),
                        p(-6.0, 3.0),
                        p(-7.0, 4.0),
                        p(-3.0, 7.0),
                        p(3.0, 7.0),
                        p(5.0, 6.0),
                        p(8.0, 6.0),
                    ],
                    s,
                ));
            }
        }
        "background" => {
            painter.rect_stroke(
                egui::Rect::from_min_max(p(-7.0, -6.0), p(7.0, 6.0)),
                2,
                s,
                egui::StrokeKind::Inside,
            );
            painter.circle_stroke(p(3.0, -2.5), rect.width() * 0.08, s);
            painter.add(egui::Shape::line(
                vec![
                    p(-6.0, 4.0),
                    p(-2.0, -1.0),
                    p(1.0, 2.0),
                    p(3.0, 0.0),
                    p(6.0, 4.0),
                ],
                s,
            ));
        }
        _ => {
            for y in [-5.0, 0.0, 5.0] {
                painter.circle_filled(p(-6.0, y), 1.0, WHITE);
                painter.line_segment([p(-2.0, y), p(6.0, y)], s);
            }
        }
    }
    if matches!(
        name,
        "fish-off"
            | "bubble-off"
            | "beer-off"
            | "beer-hand-off"
            | "disco-ball-off"
            | "disco-floor-off"
    ) {
        painter.line_segment([p(-7.5, 7.5), p(7.5, -7.5)], egui::Stroke::new(3.5, ABYSS));
        painter.line_segment([p(-7.5, 7.5), p(7.5, -7.5)], egui::Stroke::new(1.7, WHITE));
    }
}

/// Compact vector controls retain a 24-point target and accessible names.
fn compact_icon_button(ui: &mut egui::Ui, icon: &str, name: &str, enabled: bool) -> egui::Response {
    ui.add_enabled_ui(enabled, |ui| {
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(24.), egui::Sense::click());
        let fill = if response.hovered() || response.has_focus() {
            theme::accent(ui.ctx()).gamma_multiply(0.18)
        } else {
            Color32::from_white_alpha(5)
        };
        ui.painter().rect_filled(rect, 5, fill);
        paint_section_icon(
            ui.painter(),
            egui::Rect::from_center_size(rect.center(), Vec2::splat(14.)),
            icon,
        );
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, name));
        if enabled {
            BubbleSystem::register_button(ui.ctx(), &response);
        }
        response.on_hover_text(name)
    })
    .inner
}

fn skip_button(ui: &mut egui::Ui, next: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(32.0, 34.0), egui::Sense::click());
    let hover = ui.ctx().animate_bool_with_time(
        response.id.with("skip"),
        response.hovered() || response.has_focus(),
        0.18,
    );
    gradient_tile(
        ui.painter(),
        rect,
        10.0,
        theme::accent(ui.ctx()).gamma_multiply(0.08 + hover * 0.16),
        theme::tint(ui.ctx(), CARD),
    );
    ui.painter()
        .rect_stroke(rect, 10, glass_edge(), egui::StrokeKind::Inside);
    specular_edge(ui.painter(), rect, 10.0, 1.0);
    let c = rect.center();
    let direction = if next { 1.0 } else { -1.0 };
    ui.painter().add(egui::Shape::convex_polygon(
        vec![
            c + Vec2::new(-5.0 * direction, -5.0),
            c + Vec2::new(3.0 * direction, 0.0),
            c + Vec2::new(-5.0 * direction, 5.0),
        ],
        WHITE,
        egui::Stroke::NONE,
    ));
    ui.painter().line_segment(
        [
            c + Vec2::new(5.0 * direction, -5.0),
            c + Vec2::new(5.0 * direction, 5.0),
        ],
        egui::Stroke::new(2.0, WHITE),
    );
    let label = if next {
        "Următoarea piesă"
    } else {
        "Piesa anterioară"
    };
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
    });
    response.on_hover_text(label)
}

fn album_columns(width: f32) -> usize {
    ((width + ALBUM_CARD_GAP) / (ALBUM_CARD_SIZE + ALBUM_CARD_GAP))
        .floor()
        .max(1.0) as usize
}

fn download_queue_width(window_width: f32) -> f32 {
    (window_width * 0.235).clamp(236.0, 300.0)
}

fn centered_label(ui: &mut egui::Ui, text: impl Into<String>, size: f32, color: Color32) {
    let text = text.into();
    let (rect, response) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        &text,
        egui::FontId::proportional(size),
        color,
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &text));
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Home,
    Songs,
    Albums,
    Playlists,
    Favorites,
    AddSongs,
    Settings,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Drawer {
    Queue,
    // Local lyrics remain supported internally; the toolbar now opens EQ.
    #[allow(dead_code)]
    Lyrics,
}

/// Adapts secondary columns to the available content width. The title always
/// takes the leftover horizontal space and the favorite control stays last.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TrackTableLayout {
    show_artist: bool,
    show_album: bool,
}

impl TrackTableLayout {
    fn for_width(width: f32) -> Self {
        Self {
            show_artist: width >= 510.0,
            show_album: width >= 690.0,
        }
    }
}
#[derive(Clone)]
struct Lyric {
    time_ms: u64,
    text: String,
}

enum UiEvent {
    Scanned(Result<database::ScanStats, String>),
    DownloadedMusicScanned(Result<database::ScanStats, String>),
    LibraryLoaded {
        version: u64,
        result: Result<LibraryData, String>,
    },
    FavoriteToggled(Result<bool, String>),
    SongData {
        version: u64,
        id: Option<i64>,
        queue: Vec<i64>,
        lyrics: Option<String>,
        tracks: Vec<Track>,
    },
    Cover {
        id: i64,
        image: Option<(Vec<u8>, usize, usize)>,
    },
    AlbumCover {
        key: String,
        image: Option<(Vec<u8>, usize, usize)>,
    },
    AlbumPictureChosen {
        id: i64,
        result: Result<Option<String>, String>,
    },
}

struct LibraryRequest {
    version: u64,
    page_number: i64,
    query: String,
    favorites: bool,
    album_id: Option<i64>,
    playlist_id: Option<i64>,
    sort: TrackSort,
}
struct LibraryData {
    page_number: i64,
    page: Page,
    albums: Vec<Album>,
    playlists: Vec<Playlist>,
    roots: Vec<String>,
}

impl LibraryRequest {
    fn load(&self, db: &Database) -> Result<LibraryData, String> {
        let load_page = |offset| {
            if let Some(playlist_id) = self.playlist_id {
                db.playlist_page_sorted(playlist_id, offset, PAGE_SIZE, &self.query, self.sort)
            } else {
                db.page_sorted(
                    offset,
                    PAGE_SIZE,
                    &self.query,
                    self.favorites,
                    self.album_id,
                    self.sort,
                )
            }
        };
        let mut page = load_page(self.page_number.saturating_mul(PAGE_SIZE))?;
        let page_number = self
            .page_number
            .clamp(0, ((page.total - 1) / PAGE_SIZE).max(0));
        if page_number != self.page_number {
            page = load_page(page_number * PAGE_SIZE)?;
        }
        Ok(LibraryData {
            page_number,
            page,
            albums: db.albums(&self.query)?,
            playlists: db.playlists()?,
            roots: db.roots()?,
        })
    }
}

struct AuraApp {
    context: egui::Context,
    display_scale: display_scale::DisplayScale,
    refresh_version: u64,
    refresh_running: bool,
    refresh_pending: Option<LibraryRequest>,
    refresh_after: Instant,
    song_data_version: u64,
    db: Database,
    player: AudioPlayer,
    primary_playback_mode: player::PlaybackMode,
    downloader: Downloader,
    screen: Screen,
    drawer: Option<Drawer>,
    preferences: AppSettings,
    settings_path: Option<PathBuf>,
    equalizer_save_after: Option<Instant>,
    equalizer_open: bool,
    equalizer_band: usize,
    deletions: deletion::DeleteState,
    search: String,
    page_number: i64,
    sort: TrackSort,
    table_scroll_to_top: bool,
    page: Page,
    albums: Vec<Album>,
    playlists: Vec<Playlist>,
    album_filter: Option<i64>,
    album_filter_title: String,
    playlist_filter: Option<i64>,
    track_rename: Option<(i64, String)>,
    playlist_dialog: Option<PlaylistDialog>,
    roots: Vec<String>,
    scan_running: bool,
    music_scan_running: bool,
    last_music_scan: Instant,
    music_directory: Result<PathBuf, String>,
    tx: Sender<UiEvent>,
    rx: Receiver<UiEvent>,
    covers: HashMap<i64, Option<TextureHandle>>,
    album_art: Result<album_art::Store, String>,
    album_covers: HashMap<String, Option<TextureHandle>>,
    pending_album_covers: HashSet<String>,
    album_art_busy: HashSet<i64>,
    album_rename: Option<(i64, String)>,
    playlist_rename: Option<(i64, String)>,
    collection_delete_pending: Option<CollectionDelete>,
    pending_covers: HashSet<i64>,
    current_id: Option<i64>,
    lyrics: Vec<Lyric>,
    unsynced_lyrics: Option<String>,
    queue_ids: Vec<i64>,
    queue_tracks: Vec<Track>,
    seek_preview: Option<f64>,
    notification: Option<(String, Instant)>,
    reported_player_error: Option<String>,
    icons: HashMap<String, TextureHandle>,
    frost: TextureHandle,
    background_texture: Option<TextureHandle>,
    bubble_texture: Option<TextureHandle>,
    fish: FishAnimation,
    equalizer_fish: FishAnimation,
    beer_hand: BeerHandAnimation,
    disco: DiscoScene,
}

struct PlaylistDialog {
    track_id: i64,
    selected_playlist: Option<i64>,
    creating: bool,
    new_title: String,
}

#[derive(Clone)]
enum CollectionDelete {
    Playlist {
        id: i64,
        title: String,
        delete_tracks: bool,
    },
    Album {
        id: i64,
        title: String,
        delete_tracks: bool,
    },
}

fn fmt_time(ms: i64) -> String {
    let secs = ms.max(0) / 1000;
    if secs >= 3600 {
        format!("{}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60)
    } else {
        format!("{}:{:02}", secs / 60, secs % 60)
    }
}
fn label(ui: &mut egui::Ui, text: impl Into<String>, size: f32, color: Color32) {
    let text = RichText::new(text.into()).size(size).color(color);
    ui.label(text);
}
fn title(ui: &mut egui::Ui, text: impl Into<String>) {
    let text = RichText::new(text.into())
        .size(29.0)
        .strong()
        .color(ui.visuals().strong_text_color());
    ui.label(text);
}

fn density_selector(
    ui: &mut egui::Ui,
    id: &'static str,
    density: &mut EffectDensity,
    fish: bool,
) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt(id)
        .width(108.0)
        .selected_text(density.label(fish))
        .show_ui(ui, |ui| {
            for value in [
                EffectDensity::Many,
                EffectDensity::Medium,
                EffectDensity::Few,
            ] {
                changed |= ui
                    .selectable_value(density, value, value.label(fish))
                    .changed();
            }
        });
    changed
}
fn sort_header(ui: &mut egui::Ui, title: &str, column: SortColumn, sort: TrackSort) -> bool {
    let response = ui.add_sized(
        ui.available_size(),
        egui::Button::new(
            RichText::new(sort.label(column, title))
                .size(11.0)
                .color(MUTED),
        )
        .frame(false),
    );
    BubbleSystem::register_button(ui.ctx(), &response);
    response.clicked()
}

fn asset_directories() -> Vec<PathBuf> {
    // Runtime files: replacing artwork does not require rebuilding the player.
    let mut directories = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            directories.push(parent.join("assets"));
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        directories.push(cwd.join("assets"));
    }
    directories.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets"));
    directories
}

fn theme_directories(appearance: Appearance) -> Vec<PathBuf> {
    asset_directories()
        .into_iter()
        .map(|root| root.join(appearance.label()))
        .collect()
}

fn load_icons(ctx: &egui::Context) -> HashMap<String, TextureHandle> {
    let directories = asset_directories();
    let mut icons = HashMap::new();
    for name in [
        "home",
        "songs",
        "albums",
        "favorites",
        "add-music",
        "rescan",
        "settings",
        "queue",
        "lyrics",
        "equalizer",
    ] {
        'search: for directory in &directories {
            for extension in ["png", "webp", "jpg"] {
                let path = directory.join(format!("{name}.{extension}"));
                let Ok(image) = image::open(&path) else {
                    continue;
                };
                let image = image.thumbnail(64, 64).to_rgba8();
                let size = [image.width() as usize, image.height() as usize];
                icons.insert(
                    name.to_owned(),
                    ctx.load_texture(
                        format!("section-icon-{name}"),
                        egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw()),
                        egui::TextureOptions::LINEAR,
                    ),
                );
                break 'search;
            }
        }
    }
    icons
}
fn parse_lyrics(text: &str) -> (Vec<Lyric>, Option<String>) {
    let mut synced = Vec::<Lyric>::new();
    let mut plain = Vec::<String>::new();
    for line in text.lines() {
        let mut rest = line.trim();
        let mut timestamps = Vec::new();
        while let Some(after_open) = rest.strip_prefix('[') {
            let Some(close) = after_open.find(']') else {
                break;
            };
            let stamp = &after_open[..close];
            let Some((mm, ss)) = stamp.split_once(':') else {
                break;
            };
            let (Ok(mm), Ok(ss)) = (mm.parse::<u64>(), ss.parse::<f64>()) else {
                break;
            };
            if !(0.0..60.0).contains(&ss) {
                break;
            }
            let Some(time_ms) = mm
                .checked_mul(60_000)
                .and_then(|minutes| minutes.checked_add((ss * 1000.0) as u64))
                .filter(|time| *time <= 7 * 24 * 60 * 60 * 1000)
            else {
                break;
            };
            timestamps.push(time_ms);
            rest = after_open[close + 1..].trim();
        }
        if timestamps.is_empty() {
            if !rest.is_empty() && !rest.starts_with('[') {
                plain.push(rest.into());
            }
        } else {
            for time_ms in timestamps {
                synced.push(Lyric {
                    time_ms,
                    text: rest.to_owned(),
                });
            }
        }
    }
    synced.sort_by_key(|l| l.time_ms);
    if !synced.is_empty() {
        (synced, None)
    } else {
        (vec![], (!plain.is_empty()).then(|| plain.join("\n")))
    }
}

impl AuraApp {
    fn new(db: Database, ctx: &egui::Context) -> Self {
        Self::with_settings_path(
            db,
            ctx,
            downloader::music_directory(),
            AppSettings::user_path(),
        )
    }
    #[cfg(test)]
    fn with_music_directory(
        db: Database,
        ctx: &egui::Context,
        music_directory: Result<PathBuf, String>,
    ) -> Self {
        Self::with_settings_path(db, ctx, music_directory, None)
    }
    fn with_settings_path(
        db: Database,
        ctx: &egui::Context,
        music_directory: Result<PathBuf, String>,
        settings_path: Option<PathBuf>,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        ctx.set_theme(egui::ThemePreference::Dark);
        ctx.set_visuals(dark_visuals());
        ctx.style_mut_of(egui::Theme::Dark, |style| {
            style.animation_time = 0.18;
            style.spacing.button_padding = Vec2::new(12.0, 7.0);
            style.spacing.item_spacing = Vec2::new(8.0, 6.0);
            style.spacing.scroll = egui::style::ScrollStyle::solid();
        });
        let page = Page {
            total: 0,
            tracks: vec![],
        };
        let albums = vec![];
        let roots = vec![];
        let player = AudioPlayer::new(db.clone());
        let loaded = settings_path
            .as_deref()
            .map(AppSettings::load)
            .unwrap_or_else(|| Ok(AppSettings::default()));
        let (mut preferences, mut settings_error) = match loaded {
            Ok(preferences) => (preferences, None),
            Err(error) => (AppSettings::default(), Some(error)),
        };
        if let Err(error) = preferences.equalizer.validate() {
            preferences.equalizer = equalizer::Equalizer::default();
            settings_error = Some(format!("Egalizator resetat: {error}"));
        }
        if let Err(error) = player.set_equalizer(preferences.equalizer) {
            settings_error = Some(error);
        }
        let appearance = preferences.appearance;
        let (background_texture, palette) = match preferences
            .active_background()
            .map(settings::background_image)
        {
            Some(Ok(image)) => {
                let palette = GlassPalette::from_image(&image).with_appearance(appearance);
                (Some(Self::background_texture(ctx, image)), palette)
            }
            Some(Err(error)) => {
                settings_error = Some(error);
                (
                    load_background(ctx, appearance),
                    GlassPalette::for_appearance(appearance),
                )
            }
            None => (
                load_background(ctx, appearance),
                GlassPalette::for_appearance(appearance),
            ),
        };
        theme::install(ctx, palette);
        let mut equalizer_fish = FishAnimation::new(ctx, &theme_directories(Appearance::Aqua));
        equalizer_fish.set_density(EffectDensity::Few);
        let art_directory = if cfg!(test) {
            music_directory
                .as_ref()
                .ok()
                .and_then(|path| path.parent())
                .map(|path| path.join("AlbumArt"))
                .unwrap_or_else(album_art::directory)
        } else {
            album_art::directory()
        };
        let album_art = album_art::Store::open(art_directory);
        if let Err(error) = &album_art {
            settings_error = Some(error.clone());
        }
        let mut downloader = Downloader::new();
        downloader.set_cookie_browser(preferences.youtube_cookie_browser.clone());
        let mut app = Self {
            context: ctx.clone(),
            display_scale: Default::default(),
            refresh_version: 0,
            refresh_running: false,
            refresh_pending: None,
            refresh_after: Instant::now(),
            song_data_version: 0,
            db,
            player,
            primary_playback_mode: player::PlaybackMode::Off,
            downloader,
            screen: Screen::Home,
            drawer: None,
            preferences,
            settings_path,
            equalizer_save_after: None,
            equalizer_open: false,
            equalizer_band: 0,
            deletions: deletion::DeleteState::default(),
            search: String::new(),
            page_number: 0,
            sort: TrackSort::default(),
            table_scroll_to_top: true,
            page,
            albums,
            playlists: Vec::new(),
            album_filter: None,
            album_filter_title: String::new(),
            playlist_filter: None,
            track_rename: None,
            playlist_dialog: None,
            roots,
            scan_running: false,
            music_scan_running: false,
            last_music_scan: Instant::now(),
            music_directory,
            tx,
            rx,
            covers: HashMap::new(),
            album_art,
            album_covers: HashMap::new(),
            pending_album_covers: HashSet::new(),
            album_art_busy: HashSet::new(),
            album_rename: None,
            playlist_rename: None,
            collection_delete_pending: None,
            pending_covers: HashSet::new(),
            current_id: None,
            lyrics: vec![],
            unsynced_lyrics: None,
            queue_ids: vec![],
            queue_tracks: vec![],
            seek_preview: None,
            notification: None,
            reported_player_error: None,
            icons: load_icons(ctx),
            frost: load_frost(ctx),
            background_texture,
            bubble_texture: BubbleSystem::load_texture(
                ctx,
                &theme_directories(appearance),
                appearance.particle_file(),
            ),
            fish: FishAnimation::new(ctx, &theme_directories(Appearance::Aqua)),
            equalizer_fish,
            beer_hand: BeerHandAnimation::new(ctx, &theme_directories(Appearance::Beer)),
            disco: DiscoScene::new(
                ctx,
                &theme_directories(Appearance::Disco)
                    .into_iter()
                    .chain(asset_directories())
                    .collect::<Vec<_>>(),
            ),
        };
        BubbleSystem::set_enabled(
            ctx,
            app.preferences.bubbles_enabled && appearance != Appearance::Disco,
        );
        BubbleSystem::set_density(ctx, app.preferences.bubble_density);
        app.fish.set_density(app.preferences.fish_density);
        app.beer_hand.set_frequency(app.preferences.beer_frequency);
        if let Some(error) = settings_error {
            app.toast(error);
        }
        app.refresh();
        app.scan_music_folder(ctx);
        app
    }

    fn background_texture(ctx: &egui::Context, image: image::RgbaImage) -> TextureHandle {
        readability::set_photo(ctx, &image);
        ctx.load_texture(
            "custom-background",
            egui::ColorImage::from_rgba_unmultiplied(
                [image.width() as usize, image.height() as usize],
                image.as_raw(),
            ),
            egui::TextureOptions::LINEAR,
        )
    }

    fn save_preferences(&mut self) {
        let result = self
            .settings_path
            .as_deref()
            .ok_or_else(|| "Folderul local pentru setări nu este disponibil".to_owned())
            .and_then(|path| self.preferences.save(path));
        if let Err(error) = result {
            self.toast(error);
        }
    }

    fn reload_appearance(&mut self, ctx: &egui::Context) {
        let appearance = self.preferences.appearance;
        let mut palette = GlassPalette::for_appearance(appearance);
        self.background_texture = match self
            .preferences
            .active_background()
            .map(settings::background_image)
        {
            Some(Ok(image)) => {
                palette = GlassPalette::from_image(&image).with_appearance(appearance);
                Some(Self::background_texture(ctx, image))
            }
            Some(Err(error)) => {
                self.toast(error);
                load_background(ctx, appearance)
            }
            None => load_background(ctx, appearance),
        };
        theme::install(ctx, palette);
        self.bubble_texture = BubbleSystem::load_texture(
            ctx,
            &theme_directories(appearance),
            appearance.particle_file(),
        );
        BubbleSystem::reset(ctx);
        self.fish.pause();
        self.equalizer_fish.pause();
        self.beer_hand.pause();
        self.disco.pause();
        ctx.request_repaint();
    }

    fn choose_background(&mut self, ctx: &egui::Context) {
        let Some(source) = rfd::FileDialog::new()
            .set_title("Alege imaginea de fundal")
            .add_filter(
                "Imagini",
                &[
                    "png", "jpg", "jpeg", "webp", "bmp", "gif", "tif", "tiff", "qoi", "tga", "pnm",
                    "ico",
                ],
            )
            .pick_file()
        else {
            return;
        };
        let directory = self.settings_path.as_deref().and_then(|path| path.parent());
        let result = directory
            .ok_or_else(|| "Folderul local pentru fundal nu este disponibil".to_owned())
            .and_then(|directory| {
                let directory = match self.preferences.appearance {
                    Appearance::Aqua => directory.to_path_buf(),
                    appearance => directory.join(appearance.label()),
                };
                settings::import_background(&source, &directory)
            });
        match result {
            Ok((path, image)) => {
                theme::install(
                    ctx,
                    GlassPalette::from_image(&image).with_appearance(self.preferences.appearance),
                );
                self.background_texture = Some(Self::background_texture(ctx, image));
                self.preferences.set_background(Some(path));
                self.save_preferences();
                ctx.request_repaint();
            }
            Err(error) => self.toast(error),
        }
    }
    fn scan_music_folder(&mut self, ctx: &egui::Context) {
        if self.music_scan_running || self.scan_running {
            return;
        }
        self.music_scan_running = true;
        self.last_music_scan = Instant::now();
        let database = self.db.clone();
        let sender = self.tx.clone();
        let repaint = ctx.clone();
        let directory = self.music_directory.clone();
        std::thread::spawn(move || {
            let result = directory.and_then(|directory| database.scan_downloaded_music(&directory));
            let _ = sender.send(UiEvent::DownloadedMusicScanned(result));
            repaint.request_repaint();
        });
    }
    fn toast(&mut self, message: impl Into<String>) {
        self.notification = Some((message.into(), Instant::now()));
    }
    fn album_rename_window(&mut self, ctx: &egui::Context) {
        let Some((album_id, mut name)) = self.album_rename.take() else {
            return;
        };
        let mut open = true;
        let mut save = false;
        let mut cancel = false;
        egui::Window::new("Redenumește albumul")
            .id(egui::Id::new("rename-album-window"))
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label("Numele nou al albumului");
                let input = ui.add(
                    egui::TextEdit::singleline(&mut name)
                        .char_limit(120)
                        .desired_width(280.0),
                );
                if input.lost_focus() && ui.input(|state| state.key_pressed(egui::Key::Enter)) {
                    save = true;
                }
                ui.horizontal(|ui| {
                    if ui.button("Salvează").clicked() {
                        save = true;
                    }
                    if ui.button("Anulează").clicked() {
                        cancel = true;
                    }
                });
            });
        if cancel {
            open = false;
        }
        if save {
            match self.db.rename_album(album_id, &name) {
                Ok(name) => {
                    if self.album_filter == Some(album_id) {
                        self.album_filter_title = name.clone();
                    }
                    self.toast(format!("Albumul a fost redenumit în „{name}”."));
                    self.refresh();
                }
                Err(error) => {
                    self.toast(error);
                    if open {
                        self.album_rename = Some((album_id, name));
                    }
                }
            }
        } else if open {
            self.album_rename = Some((album_id, name));
        }
    }
    fn playlist_rename_window(&mut self, ctx: &egui::Context) {
        let Some((playlist_id, mut name)) = self.playlist_rename.take() else {
            return;
        };
        let mut open = true;
        let mut save = false;
        let mut cancel = false;
        egui::Window::new("Redenumește playlistul")
            .id(egui::Id::new("rename-playlist-window"))
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label("Numele nou al playlistului");
                let input = ui.add(
                    egui::TextEdit::singleline(&mut name)
                        .char_limit(120)
                        .desired_width(280.0),
                );
                if input.lost_focus() && ui.input(|state| state.key_pressed(egui::Key::Enter)) {
                    save = true;
                }
                ui.horizontal(|ui| {
                    if ui.button("Salvează").clicked() {
                        save = true;
                    }
                    if ui.button("Anulează").clicked() {
                        cancel = true;
                    }
                });
            });
        if cancel {
            open = false;
        }
        if save {
            match self.db.rename_playlist(playlist_id, &name) {
                Ok(name) => {
                    self.playlists = self.db.playlists().unwrap_or_default();
                    self.toast(format!("Playlistul a fost redenumit în „{name}”."));
                    self.refresh();
                }
                Err(error) => {
                    self.toast(error);
                    if open {
                        self.playlist_rename = Some((playlist_id, name));
                    }
                }
            }
        } else if open {
            self.playlist_rename = Some((playlist_id, name));
        }
    }
    fn playlist_delete_window(&mut self, ctx: &egui::Context) {
        let Some(action) = self.collection_delete_pending.as_ref() else {
            return;
        };
        let action = action.clone();
        let (title, deletes_audio, heading, button) = match &action {
            CollectionDelete::Playlist {
                title,
                delete_tracks,
                ..
            } => (
                title.clone(),
                *delete_tracks,
                "Ștergi playlistul?",
                if *delete_tracks {
                    "Șterge tot"
                } else {
                    "Șterge playlistul"
                },
            ),
            CollectionDelete::Album {
                title,
                delete_tracks,
                ..
            } => (
                title.clone(),
                *delete_tracks,
                if *delete_tracks {
                    "Ștergi albumul și melodiile?"
                } else {
                    "Elimini albumul din Albume?"
                },
                if *delete_tracks {
                    "Șterge tot"
                } else {
                    "Șterge albumul"
                },
            ),
        };
        let mut confirm = false;
        let mut cancel = false;
        let modal =
            egui::Modal::new(egui::Id::new("collection-delete-confirmation")).show(ctx, |ui| {
                ui.set_width(390.0);
                ui.heading(heading);
                let description = if deletes_audio {
                    "Fișierele audio ale pieselor selectate vor fi șterse definitiv de pe disc."
                } else if matches!(&action, CollectionDelete::Playlist { .. }) {
                    "Playlistul va fi eliminat; melodiile și fișierele audio se păstrează."
                } else {
                    "Albumul dispare din Albume, iar piesele rămân în Toate piesele."
                };
                ui.label(format!("„{title}”. {description}"));
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    cancel = ui.button("Anulează").clicked();
                    confirm = ui.button(button).clicked();
                });
            });
        if confirm {
            self.collection_delete_pending = None;
            let result = match &action {
                CollectionDelete::Playlist {
                    id, delete_tracks, ..
                } => {
                    let ids = if *delete_tracks {
                        self.db.playlist_order_sorted(*id, "", self.sort)
                    } else {
                        Ok(Vec::new())
                    };
                    ids.and_then(|ids| self.delete_track_ids(&ids))
                        .and_then(|_| self.db.delete_playlist(*id))
                }
                CollectionDelete::Album {
                    id, delete_tracks, ..
                } => {
                    if *delete_tracks {
                        self.db
                            .order_sorted(Some(*id), false, "", self.sort)
                            .and_then(|ids| self.delete_track_ids(&ids))
                    } else {
                        self.db.hide_album(*id)
                    }
                }
            };
            match result {
                Ok(()) => match action {
                    CollectionDelete::Playlist {
                        id, delete_tracks, ..
                    } => {
                        if self.playlist_filter == Some(id) {
                            self.navigate(Screen::Playlists);
                        } else {
                            self.refresh();
                        }
                        self.toast(if delete_tracks {
                            "Playlistul și piesele sale au fost șterse"
                        } else {
                            "Playlistul a fost șters; muzica a fost păstrată"
                        });
                    }
                    CollectionDelete::Album { delete_tracks, .. } => {
                        self.refresh();
                        self.toast(if delete_tracks {
                            "Albumul și fișierele sale audio au fost șterse"
                        } else {
                            "Albumul a fost eliminat; piesele au fost păstrate"
                        });
                    }
                },
                Err(error) => self.toast(error),
            }
        } else if cancel || modal.should_close() {
            self.collection_delete_pending = None;
        }
    }
    fn delete_track_ids(&mut self, ids: &[i64]) -> Result<(), String> {
        let result = (|| {
            for track in self.db.get_many(ids)? {
                let receiver = self.player.delete_track(track.id, track.file_path)?;
                receiver
                    .recv()
                    .map_err(|_| "Ștergerea audio nu a putut fi finalizată".to_owned())??;
                self.covers.remove(&track.id);
                self.pending_covers.remove(&track.id);
                self.queue_tracks.retain(|queued| queued.id != track.id);
            }
            Ok(())
        })();
        self.refresh();
        self.sync_song();
        result
    }
    fn track_rename_window(&mut self, ctx: &egui::Context) {
        let Some((track_id, mut name)) = self.track_rename.take() else {
            return;
        };
        let mut open = true;
        let mut save = false;
        let mut cancel = false;
        egui::Window::new("Redenumește melodia")
            .id(egui::Id::new("rename-track-window"))
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label("Titlul piesei în bibliotecă");
                let input = ui.add(
                    egui::TextEdit::singleline(&mut name)
                        .char_limit(200)
                        .desired_width(300.0),
                );
                if input.lost_focus() && ui.input(|state| state.key_pressed(egui::Key::Enter)) {
                    save = true;
                }
                ui.horizontal(|ui| {
                    if ui.button("Salvează").clicked() {
                        save = true;
                    }
                    if ui.button("Anulează").clicked() {
                        cancel = true;
                    }
                });
            });
        if cancel {
            open = false;
        }
        if save {
            match self.db.rename_track(track_id, &name) {
                Ok(name) => {
                    self.toast(format!("Melodia a fost redenumită în „{name}”."));
                    self.refresh();
                }
                Err(error) => {
                    self.toast(error);
                    if open {
                        self.track_rename = Some((track_id, name));
                    }
                }
            }
        } else if open {
            self.track_rename = Some((track_id, name));
        }
    }
    fn playlist_dialog_window(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.playlist_dialog.take() else {
            return;
        };
        let mut open = true;
        let mut add = false;
        let mut create_and_add = false;
        egui::Window::new("Adaugă melodia la playlist")
            .id(egui::Id::new("add-track-playlist-window"))
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                if dialog.creating {
                    ui.label("Numele playlistului nou");
                    ui.add(
                        egui::TextEdit::singleline(&mut dialog.new_title)
                            .char_limit(120)
                            .desired_width(280.0),
                    );
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(
                                !dialog.new_title.trim().is_empty(),
                                egui::Button::new("Creează și adaugă"),
                            )
                            .clicked()
                        {
                            create_and_add = true;
                        }
                        if !self.playlists.is_empty() && ui.button("Alege unul existent").clicked()
                        {
                            dialog.creating = false;
                        }
                    });
                } else {
                    egui::ComboBox::from_id_salt("track-playlist-selection")
                        .selected_text(
                            self.playlists
                                .iter()
                                .find(|playlist| Some(playlist.id) == dialog.selected_playlist)
                                .map(|playlist| playlist.title.as_str())
                                .unwrap_or("Selectează un playlist"),
                        )
                        .show_ui(ui, |ui| {
                            for playlist in &self.playlists {
                                ui.selectable_value(
                                    &mut dialog.selected_playlist,
                                    Some(playlist.id),
                                    format!("{} ({} piese)", playlist.title, playlist.track_count),
                                );
                            }
                        });
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(
                                dialog.selected_playlist.is_some(),
                                egui::Button::new("Adaugă"),
                            )
                            .clicked()
                        {
                            add = true;
                        }
                        if ui.button("Creează playlist nou").clicked() {
                            dialog.creating = true;
                        }
                    });
                }
            });
        if !open {
            return;
        }
        let result = if create_and_add {
            self.db
                .create_playlist(&dialog.new_title)
                .and_then(|playlist_id| {
                    self.db
                        .add_track_to_playlist(playlist_id, dialog.track_id)?;
                    Ok((playlist_id, dialog.new_title.trim().to_owned(), true))
                })
        } else if add {
            dialog
                .selected_playlist
                .ok_or_else(|| "Selectează un playlist.".to_owned())
                .and_then(|playlist_id| {
                    let title = self
                        .playlists
                        .iter()
                        .find(|playlist| playlist.id == playlist_id)
                        .map(|playlist| playlist.title.clone())
                        .unwrap_or_else(|| "playlist".to_owned());
                    self.db
                        .add_track_to_playlist(playlist_id, dialog.track_id)?;
                    Ok((playlist_id, title, false))
                })
        } else {
            self.playlist_dialog = Some(dialog);
            return;
        };
        match result {
            Ok((playlist_id, title, created)) => {
                self.toast(if created {
                    format!("Playlistul „{title}” a fost creat și melodia adăugată.")
                } else {
                    format!("Melodia a fost adăugată în „{title}”.")
                });
                self.playlists = self.db.playlists().unwrap_or_default();
                if created {
                    self.open_playlist(playlist_id);
                } else {
                    self.refresh();
                }
            }
            Err(error) => {
                self.toast(error);
                self.playlist_dialog = Some(dialog);
            }
        }
    }
    fn icon_button(&self, name: &'static str, _fallback: &str, text: &str) -> GlassButton {
        let texture = self.icons.get(name).map(TextureHandle::id);
        GlassButton {
            text: text.to_owned(),
            icon: name,
            texture,
            fill: CARD,
            radius: 9.0,
        }
    }
    fn refresh(&mut self) {
        self.request_refresh(Duration::ZERO);
    }
    fn request_refresh(&mut self, delay: Duration) {
        let favorites = self.screen == Screen::Favorites;
        let album_id = if self.screen == Screen::Songs {
            self.album_filter
        } else {
            None
        };
        let playlist_id = if self.screen == Screen::Playlists {
            self.playlist_filter
        } else {
            None
        };
        self.refresh_version = self.refresh_version.wrapping_add(1);
        self.refresh_after = Instant::now() + delay;
        self.refresh_pending = Some(LibraryRequest {
            version: self.refresh_version,
            page_number: self.page_number,
            query: self.search.clone(),
            favorites,
            album_id,
            playlist_id,
            sort: self.sort,
        });
        self.start_refresh();
    }
    fn start_refresh(&mut self) {
        if self.refresh_running || Instant::now() < self.refresh_after {
            return;
        }
        let Some(request) = self.refresh_pending.take() else {
            return;
        };
        self.refresh_running = true;
        let db = self.db.clone();
        let sender = self.tx.clone();
        let ctx = self.context.clone();
        std::thread::spawn(move || {
            let result = request.load(&db);
            let _ = sender.send(UiEvent::LibraryLoaded {
                version: request.version,
                result,
            });
            ctx.request_repaint();
        });
    }
    fn navigate(&mut self, screen: Screen) {
        self.screen = screen;
        self.search.clear();
        self.page_number = 0;
        self.table_scroll_to_top = true;
        self.album_filter = None;
        self.playlist_filter = None;
        self.refresh();
    }
    fn open_playlist(&mut self, playlist_id: i64) {
        self.screen = Screen::Playlists;
        self.playlist_filter = Some(playlist_id);
        self.album_filter = None;
        self.search.clear();
        self.page_number = 0;
        self.table_scroll_to_top = true;
        self.refresh();
    }
    fn choose_folder(&mut self, ctx: &egui::Context) {
        // Open native Windows folder dialogs on the GUI thread.
        if let Some(folder) = rfd::FileDialog::new()
            .set_title("Selectează directorul cu muzică")
            .pick_folder()
        {
            self.scan(folder.to_string_lossy().into_owned(), ctx);
        }
    }
    fn scan(&mut self, folder: String, ctx: &egui::Context) {
        if self.scan_running {
            self.toast("Indexarea este deja în desfășurare");
            return;
        }
        self.scan_running = true;
        let tx = self.tx.clone();
        let db = self.db.clone();
        let repaint = ctx.clone();
        std::thread::spawn(move || {
            let result = db.scan(&folder);
            let _ = tx.send(UiEvent::Scanned(result));
            repaint.request_repaint();
        });
    }
    fn scan_all(&mut self, ctx: &egui::Context) {
        if self.roots.is_empty() {
            self.choose_folder(ctx);
            return;
        }
        if self.scan_running {
            return;
        }
        self.scan_running = true;
        let tx = self.tx.clone();
        let db = self.db.clone();
        let roots = self.roots.clone();
        let repaint = ctx.clone();
        std::thread::spawn(move || {
            let mut total = database::ScanStats {
                discovered: 0,
                updated: 0,
                missing: 0,
                errors: 0,
                issues: vec![],
            };
            for root in roots {
                match db.scan(&root) {
                    Ok(s) => {
                        total.discovered += s.discovered;
                        total.updated += s.updated;
                        total.missing += s.missing;
                        total.errors += s.errors;
                        total.issues.extend(
                            s.issues
                                .into_iter()
                                .take(20usize.saturating_sub(total.issues.len())),
                        );
                    }
                    Err(e) => {
                        let _ = tx.send(UiEvent::Scanned(Err(e)));
                        repaint.request_repaint();
                        return;
                    }
                }
            }
            let _ = tx.send(UiEvent::Scanned(Ok(total)));
            repaint.request_repaint();
        });
    }
    fn cover(&mut self, id: i64, path: &str, ctx: &egui::Context) -> Option<TextureHandle> {
        if let Some(texture) = self.covers.get(&id) {
            return texture.clone();
        }
        if self.pending_covers.len() < 16 && !self.pending_covers.contains(&id) {
            let pool = cover::worker_pool()?;
            self.pending_covers.insert(id);
            let tx = self.tx.clone();
            let path = path.to_string();
            let repaint = ctx.clone();
            pool.spawn(move || {
                let image = cover::rgba_cover(path);
                let _ = tx.send(UiEvent::Cover { id, image });
                repaint.request_repaint();
            });
        }
        None
    }
    fn album_cover(&mut self, id: i64, ctx: &egui::Context) -> Option<TextureHandle> {
        let path = self.album_art.as_ref().ok()?.path(id)?;
        let key = path.to_string_lossy().into_owned();
        if let Some(texture) = self.album_covers.get(&key) {
            return texture.clone();
        }
        if self.pending_album_covers.len() < 8 && !self.pending_album_covers.contains(&key) {
            let pool = cover::worker_pool()?;
            self.pending_album_covers.insert(key.clone());
            let tx = self.tx.clone();
            let repaint = ctx.clone();
            pool.spawn(move || {
                let image = album_art::rgba(&path);
                let _ = tx.send(UiEvent::AlbumCover { key, image });
                repaint.request_repaint();
            });
        }
        None
    }
    fn choose_album_picture(&mut self, id: i64, ctx: &egui::Context) {
        if self.album_art_busy.contains(&id) {
            return;
        }
        let directory = match &self.album_art {
            Ok(store) => store.directory().to_path_buf(),
            Err(error) => {
                self.toast(error.clone());
                return;
            }
        };
        self.album_art_busy.insert(id);
        let tx = self.tx.clone();
        let repaint = ctx.clone();
        std::thread::spawn(move || {
            let result = rfd::FileDialog::new()
                .set_title("Alege coperta albumului")
                .add_filter(
                    "Imagini",
                    &["png", "jpg", "jpeg", "webp", "bmp", "gif", "tif", "tiff"],
                )
                .pick_file()
                .map(|source| album_art::import(&directory, id, &source))
                .transpose();
            let _ = tx.send(UiEvent::AlbumPictureChosen { id, result });
            repaint.request_repaint();
        });
    }
    fn poll_events(&mut self, ctx: &egui::Context) {
        if let Some(result) = self.downloader.poll() {
            match result {
                Ok(message) => {
                    self.refresh();
                    self.toast(message);
                }
                Err(error) => self.toast(error),
            }
        }
        while let Ok(event) = self.rx.try_recv() {
            match event {
                UiEvent::LibraryLoaded { version, result } => {
                    self.refresh_running = false;
                    if version == self.refresh_version {
                        match result {
                            Ok(data) => {
                                if data.page_number != self.page_number {
                                    self.table_scroll_to_top = true;
                                }
                                self.page_number = data.page_number;
                                self.page = data.page;
                                self.albums = data.albums;
                                self.playlists = data.playlists;
                                self.roots = data.roots;
                            }
                            Err(error) => self.toast(format!("Eroare în bibliotecă: {error}")),
                        }
                    }
                }
                UiEvent::FavoriteToggled(result) => {
                    if let Err(error) = result {
                        self.toast(error);
                    }
                    self.refresh();
                }
                UiEvent::SongData {
                    version,
                    id,
                    queue,
                    lyrics,
                    tracks,
                } => {
                    if version == self.song_data_version
                        && id == self.current_id
                        && queue == self.queue_ids
                    {
                        self.queue_tracks = tracks;
                        if let Some(text) = lyrics {
                            (self.lyrics, self.unsynced_lyrics) = parse_lyrics(&text);
                        }
                    }
                }
                UiEvent::Scanned(result) => {
                    self.scan_running = false;
                    match result {
                        Ok(s) => self.toast(format!(
                            "Bibliotecă actualizată: {} fișiere, {} modificate, {} lipsă, {} erori. {}",
                            s.discovered, s.updated, s.missing,s.errors,s.issues.join("; ")
                        )),
                        Err(e) => self.toast(format!("Scanare eșuată: {e}")),
                    }
                    self.refresh();
                    // A regular rescan may read embedded tags for Music files.
                    // Reapply the folder names and the shared downloads album.
                    self.scan_music_folder(ctx);
                }
                UiEvent::DownloadedMusicScanned(result) => {
                    self.music_scan_running = false;
                    self.last_music_scan = Instant::now();
                    match result {
                        Ok(stats) if stats.updated > 0 || stats.missing > 0 || stats.errors > 0 => {
                            self.refresh();
                            self.toast(format!(
                                "Music: {} piese noi sau actualizate, {} lipsă, {} erori. {}",
                                stats.updated,
                                stats.missing,
                                stats.errors,
                                stats.issues.join("; ")
                            ));
                        }
                        Err(error) => {
                            self.toast(format!("Scanarea folderului Music a eșuat: {error}"))
                        }
                        _ => {}
                    }
                }
                UiEvent::Cover { id, image } => {
                    self.pending_covers.remove(&id);
                    let texture = image.map(|(pixels, w, h)| {
                        let color = egui::ColorImage::from_rgba_unmultiplied([w, h], &pixels);
                        ctx.load_texture(
                            format!("aura-cover-{id}"),
                            color,
                            egui::TextureOptions::LINEAR,
                        )
                    });
                    // Keep thumbnails bounded regardless of library size.
                    if self.covers.len() >= 100 {
                        if let Some(key) = self.covers.keys().next().copied() {
                            self.covers.remove(&key);
                        }
                    }
                    self.covers.insert(id, texture);
                }
                UiEvent::AlbumCover { key, image } => {
                    self.pending_album_covers.remove(&key);
                    let texture = image.map(|(pixels, w, h)| {
                        ctx.load_texture(
                            format!("album-art-{key}"),
                            egui::ColorImage::from_rgba_unmultiplied([w, h], &pixels),
                            egui::TextureOptions::LINEAR,
                        )
                    });
                    if self.album_covers.len() >= 100 {
                        if let Some(old) = self.album_covers.keys().next().cloned() {
                            self.album_covers.remove(&old);
                        }
                    }
                    self.album_covers.insert(key, texture);
                }
                UiEvent::AlbumPictureChosen { id, result } => {
                    self.album_art_busy.remove(&id);
                    match result {
                        Ok(Some(name)) => {
                            let saved = self
                                .album_art
                                .as_mut()
                                .map_err(|error| error.clone())
                                .and_then(|store| store.register(id, name));
                            match saved {
                                Ok(()) => {
                                    if let Some(path) = self
                                        .album_art
                                        .as_ref()
                                        .ok()
                                        .and_then(|store| store.path(id))
                                    {
                                        self.album_covers
                                            .remove(&path.to_string_lossy().into_owned());
                                    }
                                    self.toast("Coperta albumului a fost salvată în AlbumArt");
                                }
                                Err(error) => self.toast(error),
                            }
                        }
                        Ok(None) => {}
                        Err(error) => self.toast(error),
                    }
                }
            }
        }
        self.start_refresh();
        if self.refresh_pending.is_some() {
            ctx.request_repaint_after(self.refresh_after.saturating_duration_since(Instant::now()));
        }
        if self.last_music_scan.elapsed() >= MUSIC_SCAN_INTERVAL {
            self.scan_music_folder(ctx);
        }
    }
    fn play(&mut self, id: i64) {
        if self.screen == Screen::Playlists {
            if let Some(playlist_id) = self.playlist_filter {
                if let Err(error) =
                    self.player
                        .play_playlist_sorted(id, playlist_id, &self.search, self.sort)
                {
                    self.toast(error);
                }
                self.sync_song();
                return;
            }
        }
        let album_id = if self.screen == Screen::Songs {
            self.album_filter
        } else {
            None
        };
        let favorites = self.screen == Screen::Favorites;
        if let Err(e) =
            self.player
                .play_sorted(&self.db, id, album_id, favorites, &self.search, self.sort)
        {
            self.toast(e);
        }
        self.sync_song();
    }
    fn open_album_tracks(&mut self, album_id: i64, title: String) {
        self.screen = Screen::Songs;
        self.album_filter = Some(album_id);
        self.album_filter_title = title;
        self.search.clear();
        self.page_number = 0;
        self.table_scroll_to_top = true;
        self.refresh();
    }
    fn play_album(&mut self, album_id: i64) {
        match self
            .db
            .order_sorted(Some(album_id), false, "", TrackSort::default())
        {
            Ok(tracks) => match tracks.first() {
                Some(&first_track) => {
                    if let Err(error) = self.player.play_sorted(
                        &self.db,
                        first_track,
                        Some(album_id),
                        false,
                        "",
                        TrackSort::default(),
                    ) {
                        self.toast(error);
                    }
                }
                None => self.toast("Albumul nu conține piese disponibile."),
            },
            Err(error) => self.toast(error),
        }
        self.sync_song();
    }
    fn sync_song(&mut self) {
        let Ok(status) = self.player.status() else {
            return;
        };
        let id = status.current.as_ref().map(|t| t.id);
        let song_changed = id != self.current_id;
        if song_changed {
            self.current_id = id;
            self.lyrics.clear();
            self.unsynced_lyrics = None;
        }
        if song_changed || status.queue != self.queue_ids {
            self.queue_ids = status.queue.clone();
            self.queue_tracks.clear();
            self.song_data_version = self.song_data_version.wrapping_add(1);
            let version = self.song_data_version;
            let queue = self.queue_ids.clone();
            let db = self.db.clone();
            let sender = self.tx.clone();
            let ctx = self.context.clone();
            std::thread::spawn(move || {
                let lyrics = id.and_then(|id| db.lyrics(id).ok());
                let tracks = db.get_many(&queue).unwrap_or_default();
                let _ = sender.send(UiEvent::SongData {
                    version,
                    id,
                    queue,
                    lyrics,
                    tracks,
                });
                ctx.request_repaint();
            });
        }
    }
    fn action(&mut self, result: Result<(), String>) {
        if let Err(err) = result {
            self.toast(err);
        }
        self.sync_song();
    }
    fn sidebar(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
    ) -> egui::scroll_area::ScrollAreaOutput<()> {
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            let (logo, _) = ui.allocate_exact_size(Vec2::splat(34.0), egui::Sense::hover());
            gradient_tile(
                ui.painter(),
                logo,
                10.0,
                theme::accent(ui.ctx()).gamma_multiply(0.4),
                theme::secondary(ui.ctx()).gamma_multiply(0.15),
            );
            paint_section_icon(ui.painter(), logo.shrink(6.0), "songs");
            ui.vertical(|ui| {
                ui.label(
                    RichText::new("S4GGZ")
                        .color(ui.visuals().strong_text_color())
                        .strong()
                        .size(21.0),
                );
                label(ui, "MUSIC PLAYER", 10.0, MUTED);
            });
        });
        ui.add_space(30.0);
        // Keep the navigation and library actions reachable when the footer
        // leaves only a short viewport on a compact window.
        let navigation = egui::ScrollArea::vertical()
            .id_salt("sidebar-navigation")
            .auto_shrink([false, true])
            .max_height((ui.available_height() - 74.0).max(80.0))
            .show(ui, |ui| {
                label(ui, "BIBLIOTECĂ", 11.0, MUTED);
                ui.add_space(10.0);
                for (screen, icon, fallback, text) in [
                    (Screen::Home, "home", "⌂", "Acasă"),
                    (Screen::Songs, "songs", "♫", "Toate piesele"),
                    (Screen::Albums, "albums", "▦", "Albume"),
                    (Screen::Playlists, "queue", "≡", "Playlisturi"),
                    (Screen::Favorites, "favorites", "♥", "Favorite"),
                ] {
                    let selected = self.screen == screen;
                    let button = self
                        .icon_button(icon, fallback, text)
                        .fill(if selected {
                            theme::accent(ui.ctx()).gamma_multiply(0.24)
                        } else {
                            CARD.gamma_multiply(0.35)
                        })
                        .corner_radius(11.0);
                    let response = ui.add_sized([ui.available_width().min(190.0), 41.0], button);
                    let hover = ctx.animate_bool_with_time(
                        response.id.with("dock-glow"),
                        response.hovered() || response.has_focus(),
                        0.18,
                    );
                    ui.painter().rect_stroke(
                        response.rect.shrink(0.5),
                        11,
                        egui::Stroke::new(
                            1.0,
                            theme::accent(ui.ctx()).gamma_multiply(0.08 + hover * 0.3),
                        ),
                        egui::StrokeKind::Inside,
                    );
                    specular_edge(ui.painter(), response.rect, 11.0, 0.6 + hover);
                    if selected {
                        let capsule = egui::Rect::from_center_size(
                            egui::pos2(response.rect.left() + 5.0, response.rect.center().y),
                            Vec2::new(2.0, 20.0),
                        );
                        ui.painter().rect_filled(
                            capsule.expand(2.0),
                            4,
                            theme::accent(ui.ctx()).gamma_multiply(0.12),
                        );
                        ui.painter()
                            .rect_filled(capsule, 2, theme::accent(ui.ctx()));
                    }
                    if response.clicked() {
                        self.navigate(screen);
                    }
                    ui.add_space(3.0);
                }
                ui.add_space(26.0);
                label(ui, "COLECȚIA TA", 11.0, MUTED);
                ui.add_space(12.0);
                if ui
                    .add_sized(
                        [ui.available_width().min(190.0), 35.0],
                        self.icon_button("add-music", "＋", "Adaugă muzică")
                            .fill(CARD),
                    )
                    .clicked()
                {
                    self.choose_folder(ctx);
                }
                if ui
                    .add_sized(
                        [ui.available_width().min(190.0), 35.0],
                        self.icon_button("add-music", "↓", "Descarcă melodii").fill(
                            if self.screen == Screen::AddSongs {
                                theme::accent(ui.ctx()).gamma_multiply(0.20)
                            } else {
                                CARD
                            },
                        ),
                    )
                    .clicked()
                {
                    self.navigate(Screen::AddSongs);
                }
                if ui
                    .add_sized(
                        [ui.available_width().min(190.0), 35.0],
                        self.icon_button("rescan", "⟳", "Rescanează")
                            .fill(Color32::TRANSPARENT),
                    )
                    .clicked()
                {
                    self.scan_all(ctx);
                }
                ui.add_space(20.0);
                if self.scan_running {
                    ui.spinner();
                    label(ui, "Indexare în fundal...", 12.0, MUTED);
                }
            });
        ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
            if ui
                .add_sized(
                    [ui.available_width().min(190.0), 38.0],
                    self.icon_button("settings", "⚙", "Setări").fill(
                        if self.screen == Screen::Settings {
                            CARD
                        } else {
                            Color32::TRANSPARENT
                        },
                    ),
                )
                .clicked()
            {
                self.navigate(Screen::Settings);
            }
            ui.add_space(10.0);
            ui.separator();
            label(ui, "NATIVE EDITION  ·  v0.2", 10.0, MUTED);
            ui.add_space(15.0);
        });
        navigation
    }
    fn toolbar_search(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let import_width = ui
            .painter()
            .layout_no_wrap(
                "Importă folder".into(),
                egui::FontId::proportional(13.),
                WHITE,
            )
            .size()
            .x
            + 46.;
        let search_width =
            (ui.available_width() - import_width - ui.spacing().item_spacing.x * 3. - 9.)
                .clamp(60., 370.);
        let search = ui.add_sized(
            [search_width, 38.],
            egui::TextEdit::singleline(&mut self.search)
                .char_limit(512)
                .hint_text("Caută titluri, artiști, albume...")
                .margin(egui::Margin::symmetric(14, 8)),
        );
        if search.changed() {
            self.page_number = 0;
            self.table_scroll_to_top = true;
            self.request_refresh(Duration::from_millis(150));
        }
        ui.add_space(9.);
        if ui
            .add(self.icon_button("add-music", "", "Importă folder"))
            .clicked()
        {
            self.choose_folder(ctx);
        }
    }
    fn toolbar_actions(&mut self, ui: &mut egui::Ui) {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add(self.icon_button("queue", "", "Coada")).clicked() {
                self.drawer = if self.drawer == Some(Drawer::Queue) {
                    None
                } else {
                    Some(Drawer::Queue)
                };
            }
            if ui
                .add(self.icon_button("equalizer", "", "Egalizator"))
                .clicked()
            {
                self.equalizer_open = !self.equalizer_open;
            }
        });
    }
    fn toolbar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let width = ui.available_width();
        if width >= 560. {
            ui.horizontal(|ui| {
                ui.allocate_ui_with_layout(
                    Vec2::new(width - 220., 38.),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        self.toolbar_search(ui, ctx);
                    },
                );
                self.toolbar_actions(ui);
            });
        } else {
            ui.horizontal(|ui| self.toolbar_search(ui, ctx));
            self.toolbar_actions(ui);
        }
        ui.add_space(20.);
    }
    fn main_content(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if self.screen == Screen::Home {
            self.home(ui, ctx);
            return;
        }
        if self.screen == Screen::AddSongs {
            self.reserve_disco_stage(ui, ctx, 0.0);
            self.downloader.show(ui, ctx, &self.frost, &self.db);
            let browser = self.downloader.cookie_browser().map(str::to_owned);
            if self.preferences.youtube_cookie_browser != browser {
                self.preferences.youtube_cookie_browser = browser;
                self.save_preferences();
            }
        } else {
            self.toolbar(ui, ctx);
            self.reserve_disco_stage(ui, ctx, 0.0);
            match self.screen {
                Screen::Home => unreachable!(),
                Screen::Songs | Screen::Favorites => self.songs(ui),
                Screen::Playlists if self.playlist_filter.is_some() => self.songs(ui),
                Screen::Playlists => self.playlists_ui(ui),
                Screen::Albums => self.albums_ui(ui, ctx),
                Screen::Settings => self.settings(ui, ctx),
                Screen::AddSongs => unreachable!(),
            }
        }
        self.show_notification(ui);
    }

    fn right_panels(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, status: &PlayerStatus) {
        if self.screen == Screen::AddSongs {
            let width = download_queue_width(ctx.content_rect().width());
            egui::Panel::right("download-queue-panel")
                .exact_size(width)
                .resizable(false)
                .show_separator_line(false)
                .frame(egui::Frame::NONE.inner_margin(egui::Margin {
                    left: 4,
                    right: 12,
                    top: 12,
                    bottom: 4,
                }))
                .show(ui, |ui| {
                    self.downloader.show_queue(ui, ctx, &self.frost, &self.db)
                });
            // Playback's queue remains available without squeezing the search
            // form between two sidebars on compact windows.
            if self.drawer.is_some() {
                let mut open = true;
                egui::Window::new("Lista de redare")
                    .id(egui::Id::new("download-playback-queue"))
                    .default_width(320.0)
                    .max_height((ctx.content_rect().height() - 160.0).max(200.0))
                    .frame(glass_frame(ui, SIDEBAR, 16, 18))
                    .open(&mut open)
                    .show(ctx, |ui| {
                        paint_frost(ui, &self.frost);
                        self.drawer_ui(ui, status);
                    });
                if !open {
                    self.drawer = None;
                }
            }
        } else if self.drawer.is_some() {
            egui::Panel::right("aura-drawer")
                .exact_size(320.0)
                .resizable(false)
                .show_separator_line(false)
                .frame(glass_frame(ui, SIDEBAR, 18, 18).outer_margin(egui::Margin {
                    left: 4,
                    right: 12,
                    top: 12,
                    bottom: 4,
                }))
                .show(ui, |ui| {
                    paint_frost(ui, &self.frost);
                    self.drawer_ui(ui, status);
                });
        }
    }
    fn show_notification(&self, ui: &mut egui::Ui) {
        if let Some((message, instant)) = &self.notification {
            if instant.elapsed() < Duration::from_secs(6) {
                ui.add_space(12.0);
                label(ui, message.clone(), 12.0, theme::accent(ui.ctx()));
            }
        }
    }
    fn reserve_disco_stage(&self, ui: &mut egui::Ui, ctx: &egui::Context, scroll_offset: f32) {
        if self.preferences.appearance == Appearance::Disco && self.preferences.disco_ball_enabled {
            // The initial clearance scrolls with Home's panels. Recomputing it
            // at a fixed screen position would cancel the user's scrolling.
            let target = DiscoScene::stage_bottom(ctx.content_rect()) - scroll_offset;
            ui.add_space((target - ui.cursor().top()).max(0.0));
        }
    }
    fn home(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
    ) -> egui::scroll_area::ScrollAreaOutput<()> {
        egui::ScrollArea::vertical()
            .id_salt("home-page")
            .auto_shrink([false, false])
            .show_viewport(ui, |ui, viewport| {
                self.toolbar(ui, ctx);
                self.reserve_disco_stage(ui, ctx, viewport.min.y);
                self.home_panels(ui, ctx);
                self.show_notification(ui);
            })
    }
    fn home_panels(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let hero = glass_frame(ui, CARD, 18, 20).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            label(ui, "BIBLIOTECA TA AUDIO", 12.0, theme::accent(ui.ctx()));
            ui.add_space(6.0);
            ui.label(
                RichText::new("Redescoperă muzica ta.")
                    .size(30.0)
                    .strong()
                    .color(WHITE),
            );
            ui.add_space(7.0);
            label(
                ui,
                "Toată colecția locală, într-un singur loc. Fără browser. Fără cloud.",
                14.0,
                MUTED,
            );
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui
                    .add(
                        self.icon_button("songs", "♫", "Explorează biblioteca")
                            .fill(theme::accent(ui.ctx()))
                            .corner_radius(9.0),
                    )
                    .clicked()
                {
                    self.navigate(Screen::Songs);
                }
                if ui
                    .add(self.icon_button("add-music", "＋", "Adaugă muzică..."))
                    .clicked()
                {
                    self.choose_folder(ctx);
                }
            });
        });
        specular_edge(ui.painter(), hero.response.rect, 18.0, 1.0);
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            stats_card(ui, "MELODII", self.page.total.to_string());
            stats_card(ui, "ALBUME", self.albums.len().to_string());
            stats_card(ui, "FOLDERE", self.roots.len().to_string());
        });
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Din colecția ta").strong().size(22.0));
            if ui
                .add(self.icon_button("albums", "▦", "Vezi toate albumele"))
                .clicked()
            {
                self.navigate(Screen::Albums);
            }
        });
        ui.add_space(12.0);
        if self.albums.is_empty() {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                label(ui, "♪", 60.0, theme::accent(ui.ctx()));
                ui.heading("Biblioteca este goală");
                label(
                    ui,
                    "Alege un folder cu fișiere MP3, FLAC, WAV, M4A sau OGG.",
                    14.0,
                    MUTED,
                );
                ui.add_space(14.0);
                if ui
                    .add(self.icon_button("add-music", "＋", "Alege un folder"))
                    .clicked()
                {
                    self.choose_folder(ctx);
                }
            });
            return;
        }
        let albums = self.albums.clone();
        self.album_cards(ui, ctx, &albums, Some(ui.clip_rect()));
    }
    fn songs(&mut self, ui: &mut egui::Ui) {
        let heading = match self.screen {
            Screen::Favorites => "Favoritele mele".to_string(),
            Screen::Playlists if self.playlist_filter.is_some() => self
                .playlists
                .iter()
                .find(|playlist| Some(playlist.id) == self.playlist_filter)
                .map(|playlist| playlist.title.clone())
                .unwrap_or_else(|| "Playlist".to_owned()),
            _ if self.album_filter.is_some() => self.album_filter_title.clone(),
            _ => "Toate melodiile".to_string(),
        };
        ui.horizontal(|ui| {
            title(ui, heading);
            ui.add_space(10.0);
            label(ui, format!("{} piese", self.page.total), 13.0, MUTED);
            if !self.deletions.confirmation_required(Instant::now()) {
                ui.label(RichText::new("Ștergere rapidă activă").small().color(MUTED))
                    .on_hover_text("Ai șters peste 3 piese în ultimele 60 de secunde. Următoarele ștergeri nu cer confirmare până când scade numărul lor sub acest prag.");
            }
        });
        if self.album_filter.is_some()
            && ui
                .add(self.icon_button("albums", "▦", "Toate albumele"))
                .clicked()
        {
            self.navigate(Screen::Albums);
        }
        if self.screen == Screen::Playlists && ui.button("← Toate playlisturile").clicked() {
            self.navigate(Screen::Playlists);
        }
        ui.add_space(15.0);
        let scroll_to_top = std::mem::take(&mut self.table_scroll_to_top);
        let tracks = &self.page.tracks;
        let sort = self.sort;
        let mut sort_requested = None;
        let mut to_play = None;
        let mut toggle_favorite = None;
        let mut to_delete = None;
        let mut to_rename = None;
        let mut to_playlist = None;
        let allow_delete = !self.deletions.busy();
        // Keep the last (favorite) column flush with the right edge, even
        // while the native window or the Queue/Lyrics drawer is resized.
        glass_frame(ui, CARD, 14, 10)
            .stroke(egui::Stroke::NONE)
            .show(ui, |ui| {
                paint_frost(ui, &self.frost);
                ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
                let table_layout = TrackTableLayout::for_width(
                    ui.available_width() - ui.spacing().scroll.allocated_width(),
                );
                // max_scroll_height covers only the body, so reserve the header
                // and the complete pagination panel instead of overflowing it.
                let pagination_height = ui.spacing().interact_size.y.max(
                    ui.text_style_height(&egui::TextStyle::Button)
                        + 2.0 * ui.spacing().button_padding.y,
                ) + 20.0;
                let table_height = (ui.available_height()
                    - 34.0
                    - ui.spacing().item_spacing.y
                    - 10.0
                    - pagination_height)
                    .max(44.0);
                let mut table = TableBuilder::new(ui)
                    .id_salt("aura-tracks")
                    .striped(true)
                    .sense(egui::Sense::click())
                    .resizable(true)
                    .auto_shrink([false, false])
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                    .max_scroll_height(table_height)
                    .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                    .column(Column::exact(36.0))
                    // Recompute the flexible column on resize instead of restoring its
                    // previous width and leaving the favorites column short of the edge.
                    .column(
                        Column::remainder()
                            .at_least(135.0)
                            .clip(true)
                            .resizable(false),
                    );
                if scroll_to_top {
                    table = table.vertical_scroll_offset(0.0);
                }
                if table_layout.show_artist {
                    table = table.column(Column::initial(145.0).at_least(105.0).clip(true));
                }
                if table_layout.show_album {
                    table = table.column(Column::initial(145.0).at_least(105.0).clip(true));
                }
                table
                    .column(Column::exact(75.0))
                    .column(Column::exact(46.0))
                    .column(Column::exact(28.0))
                    .header(34.0, |mut row| {
                        row.col(|ui| {
                            centered_label(ui, "#", 11.0, MUTED);
                        });
                        row.col(|ui| {
                            if sort_header(ui, "TITLU", SortColumn::Title, sort) {
                                sort_requested = Some(SortColumn::Title);
                            }
                        });
                        if table_layout.show_artist {
                            row.col(|ui| {
                                if sort_header(ui, "ARTIST", SortColumn::Artist, sort) {
                                    sort_requested = Some(SortColumn::Artist);
                                }
                            });
                        }
                        if table_layout.show_album {
                            row.col(|ui| {
                                centered_label(ui, "ALBUM", 11.0, MUTED);
                            });
                        }
                        row.col(|ui| {
                            if sort_header(ui, "DURATĂ", SortColumn::Duration, sort) {
                                sort_requested = Some(SortColumn::Duration);
                            }
                        });
                        row.col(|ui| {
                            centered_label(ui, "♡", 11.0, MUTED);
                        });
                        row.col(|ui| {
                            centered_label(ui, "", 11.0, MUTED);
                        });
                    })
                    .body(|body| {
                        body.rows(44.0, tracks.len(), |mut row| {
                            let i = row.index();
                            let track = &tracks[i];
                            let action = track_table::draw_track_row(
                                &mut row,
                                track,
                                i as i64 + 1 + self.page_number * PAGE_SIZE,
                                self.current_id == Some(track.id),
                                table_layout,
                                allow_delete,
                            );
                            if action.play_requested() {
                                to_play = Some(track.id);
                            }
                            if action.favorite {
                                toggle_favorite = Some(track.id);
                            }
                            if action.delete {
                                to_delete = Some(track.clone());
                            }
                            if action.rename {
                                to_rename = Some((track.id, track.title.clone()));
                            }
                            if action.add_to_playlist {
                                to_playlist = Some(track.id);
                            }
                        });
                    });
            });
        if let Some(track) = to_delete {
            self.request_track_deletion(track);
        }
        if let Some((track_id, title)) = to_rename {
            self.track_rename = Some((track_id, title));
        }
        if let Some(track_id) = to_playlist {
            self.playlist_dialog = Some(PlaylistDialog {
                track_id,
                selected_playlist: self.playlists.first().map(|playlist| playlist.id),
                creating: self.playlists.is_empty(),
                new_title: String::new(),
            });
        }
        if let Some(column) = sort_requested {
            self.sort.toggle(column);
            self.page_number = 0;
            self.table_scroll_to_top = true;
            self.refresh();
        }
        if let Some(id) = to_play {
            self.play(id);
        }
        if let Some(id) = toggle_favorite {
            let db = self.db.clone();
            let sender = self.tx.clone();
            let ctx = self.context.clone();
            std::thread::spawn(move || {
                let _ = sender.send(UiEvent::FavoriteToggled(db.favorites(id)));
                ctx.request_repaint();
            });
        }
        ui.add_space(10.0);
        let pagination = glass_frame(ui, CARD, 12, 10).show(ui, |ui| {
            ui.horizontal(|ui| {
                label(
                    ui,
                    format!(
                        "Pagina {} / {}",
                        self.page_number + 1,
                        ((self.page.total + PAGE_SIZE - 1) / PAGE_SIZE).max(1)
                    ),
                    12.0,
                    MUTED,
                );
                let previous_page =
                    ui.add_enabled(self.page_number > 0, egui::Button::new("← Anterior"));
                BubbleSystem::register_button(ui.ctx(), &previous_page);
                if previous_page.clicked() {
                    self.page_number -= 1;
                    self.table_scroll_to_top = true;
                    self.refresh();
                }
                let next_page = ui.add_enabled(
                    (self.page_number + 1) * PAGE_SIZE < self.page.total,
                    egui::Button::new("Următor →"),
                );
                BubbleSystem::register_button(ui.ctx(), &next_page);
                if next_page.clicked() {
                    self.page_number += 1;
                    self.table_scroll_to_top = true;
                    self.refresh();
                }
            });
        });
        // The available max_rect here may be wider and shorter than the
        // actual panel. Paint only its measured content bounds after layout.
        paint_frost_grain_in_rect(ui, &self.frost, pagination.response.rect.shrink(10.0));
    }
    fn album_cards(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        albums: &[Album],
        viewport: Option<egui::Rect>,
    ) {
        ui.scope(|ui| {
            ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
            ui.spacing_mut().item_spacing = Vec2::splat(ALBUM_CARD_GAP);
            let scrollbar_width = if viewport.is_some() {
                0.0
            } else {
                ui.spacing().scroll.allocated_width()
            };
            let columns = album_columns(ui.available_width() - scrollbar_width);
            let rows = albums.len().div_ceil(columns);
            let mut selected = None;
            let mut open_album = None;
            let mut change_picture = None;
            let mut rename_album = None;
            let mut delete_album = None;
            let draw = |ui: &mut egui::Ui,
                        range: std::ops::Range<usize>,
                        self_: &mut Self,
                        selected: &mut Option<(i64, String)>,
                        open_album: &mut Option<(i64, String)>,
                        change_picture: &mut Option<i64>,
                        rename_album: &mut Option<(i64, String)>,
                        delete_album: &mut Option<CollectionDelete>| {
                for row in range {
                    ui.horizontal(|ui| {
                        for column in 0..columns {
                            let index = row * columns + column;
                            let Some(album) = albums.get(index) else {
                                continue;
                            };
                            // Reserve a stable grid slot; only the painted child lifts.
                            // Reflow and scroll virtualization therefore never jitter on hover.
                            let (slot, response) = ui.allocate_exact_size(
                                Vec2::splat(ALBUM_CARD_SIZE),
                                egui::Sense::click(),
                            );
                            response.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::WidgetType::Button,
                                    true,
                                    format!("Redă albumul {}", album.title),
                                )
                            });
                            if response.clicked() || response.secondary_clicked() {
                                response.request_focus();
                            }
                            if response.clicked() {
                                *selected = Some((album.id, album.title.clone()));
                            }
                            let art = self_.album_cover(album.id, ctx).or_else(|| {
                                self_.cover(
                                    album.representative_track,
                                    &album.representative_path,
                                    ctx,
                                )
                            });
                            let has_picture = art.is_some()
                                || self_
                                    .album_art
                                    .as_ref()
                                    .ok()
                                    .and_then(|store| store.path(album.id))
                                    .is_some();
                            let hover = ctx.animate_bool_with_time(
                                ui.id().with(("album-lift", album.id)),
                                response.hovered(),
                                0.2,
                            );
                            let lifted = slot.translate(Vec2::new(0.0, -2.0 * hover));
                            ui.scope_builder(
                                egui::UiBuilder::new()
                                    .max_rect(lifted)
                                    .layout(egui::Layout::top_down(egui::Align::LEFT)),
                                |ui| {
                                    let card = glass_frame(ui, CARD, 14, 11)
                                        .stroke(egui::Stroke::new(
                                            1.0,
                                            Color32::from_rgba_unmultiplied(
                                                170,
                                                241,
                                                255,
                                                (35.0 + hover * 85.0) as u8,
                                            ),
                                        ))
                                        .show(ui, |ui| {
                                            ui.set_width(ALBUM_CARD_SIZE - 24.0);
                                            ui.set_min_height(ALBUM_CARD_SIZE - 24.0);
                                            ui.spacing_mut().item_spacing = Vec2::new(8.0, 3.0);
                                            cover_tile(ui, art, 91.0);
                                            ui.add_space(6.0);
                                            ui.add(
                                                egui::Label::new(
                                                    RichText::new(&album.title).strong().size(13.0),
                                                )
                                                .truncate(),
                                            )
                                            .on_hover_text(&album.title);
                                            ui.add(
                                                egui::Label::new(
                                                    RichText::new(&album.artist)
                                                        .size(11.0)
                                                        .color(MUTED),
                                                )
                                                .truncate(),
                                            )
                                            .on_hover_text(&album.artist);
                                            ui.label(
                                                RichText::new(format!(
                                                    "▶  {} piese",
                                                    album.track_count
                                                ))
                                                .size(11.0)
                                                .color(theme::accent(ui.ctx())),
                                            );
                                        });
                                    specular_edge(
                                        ui.painter(),
                                        card.response.rect,
                                        14.0,
                                        1.0 + hover,
                                    );
                                },
                            );
                            response.context_menu(|ui| {
                                if ui.button("Deschide album").clicked() {
                                    *open_album = Some((album.id, album.title.clone()));
                                    ui.close();
                                }
                                if ui
                                    .add_enabled(
                                        !self_.album_art_busy.contains(&album.id),
                                        egui::Button::new(if has_picture {
                                            "Change picture"
                                        } else {
                                            "Add picture"
                                        }),
                                    )
                                    .clicked()
                                {
                                    *change_picture = Some(album.id);
                                    ui.close();
                                }
                                if ui.button("Redenumește albumul").clicked() {
                                    *rename_album = Some((album.id, album.title.clone()));
                                    ui.close();
                                }
                                if ui.button("Șterge albumul").clicked() {
                                    *delete_album = Some(CollectionDelete::Album {
                                        id: album.id,
                                        title: album.title.clone(),
                                        delete_tracks: false,
                                    });
                                    ui.close();
                                }
                                if ui.button("Șterge tot (album și melodii)").clicked() {
                                    *delete_album = Some(CollectionDelete::Album {
                                        id: album.id,
                                        title: album.title.clone(),
                                        delete_tracks: true,
                                    });
                                    ui.close();
                                }
                            });
                        }
                    });
                }
            };
            if let Some(viewport) = viewport {
                // Share Home's scroll position instead of nesting an album
                // scroller. Reserve the full grid but paint only visible rows.
                let stride = ALBUM_CARD_SIZE + ALBUM_CARD_GAP;
                let height = (rows as f32 * stride - ALBUM_CARD_GAP).max(0.0);
                let (grid, _) = ui.allocate_exact_size(
                    Vec2::new(ui.available_width(), height),
                    egui::Sense::hover(),
                );
                let first =
                    (((viewport.top() - grid.top()) / stride).floor().max(0.0) as usize).min(rows);
                let end = ((((viewport.bottom() - grid.top()) / stride).ceil().max(0.0) as usize)
                    .saturating_add(1))
                .min(rows);
                if first < end {
                    let visible = egui::Rect::from_x_y_ranges(
                        grid.x_range(),
                        grid.top() + first as f32 * stride..=grid.top() + end as f32 * stride,
                    );
                    ui.scope_builder(egui::UiBuilder::new().max_rect(visible), |ui| {
                        ui.skip_ahead_auto_ids(first);
                        draw(
                            ui,
                            first..end,
                            self,
                            &mut selected,
                            &mut open_album,
                            &mut change_picture,
                            &mut rename_album,
                            &mut delete_album,
                        );
                    });
                }
            } else {
                egui::ScrollArea::vertical()
                    .id_salt("album-cards")
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                    .auto_shrink([false, false])
                    .show_rows(ui, ALBUM_CARD_SIZE, rows, |ui, visible_rows| {
                        draw(
                            ui,
                            visible_rows,
                            self,
                            &mut selected,
                            &mut open_album,
                            &mut change_picture,
                            &mut rename_album,
                            &mut delete_album,
                        );
                    });
            }
            if let Some((id, name)) = selected {
                self.open_album_tracks(id, name);
                self.play_album(id);
            }
            if let Some((id, name)) = open_album {
                self.open_album_tracks(id, name);
            }
            if let Some(id) = change_picture {
                self.choose_album_picture(id, ctx);
            }
            if rename_album.is_some() {
                self.album_rename = rename_album;
            }
            if delete_album.is_some() {
                self.collection_delete_pending = delete_album;
            }
        });
    }
    fn playlists_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            title(ui, "Playlisturile mele");
            ui.add_space(10.0);
            label(
                ui,
                format!("{} playlisturi", self.playlists.len()),
                13.0,
                MUTED,
            );
        });
        ui.add_space(14.0);
        if self.playlists.is_empty() {
            glass_frame(ui, CARD, 14, 18).show(ui, |ui| {
                ui.label("Nu ai încă playlisturi.");
                ui.label(
                    "Click dreapta pe o melodie și alege «Adaugă la playlist» pentru a crea unul.",
                );
            });
            return;
        }
        let playlists = self.playlists.clone();
        let mut open = None;
        let mut play = None;
        let mut rename = None;
        let mut delete = None;
        let mut delete_all = None;
        egui::ScrollArea::vertical()
            .id_salt("user-playlists")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for playlist in playlists {
                    let mut play_rect = None;
                    let row = glass_frame(ui, CARD, 12, 14).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.label(RichText::new(&playlist.title).strong());
                                label(ui, format!("{} piese", playlist.track_count), 12.0, MUTED);
                            });
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let button = ui.button("▶  Play");
                                    play_rect = Some(button.rect);
                                    if button.clicked() {
                                        play = Some(playlist.id);
                                    }
                                },
                            );
                        });
                    });
                    let response = row.response.interact(egui::Sense::click());
                    if response.clicked() {
                        // The card response spans the child button too. Route the
                        // click by its position in case egui reports it on the card.
                        let on_play = play_rect.is_some_and(|rect| {
                            ui.ctx().input(|input| {
                                input
                                    .pointer
                                    .interact_pos()
                                    .is_some_and(|position| rect.contains(position))
                            })
                        });
                        if on_play {
                            play = Some(playlist.id);
                        } else {
                            open = Some(playlist.id);
                        }
                    }
                    response.context_menu(|ui| {
                        if ui.button("Redenumește playlistul").clicked() {
                            rename = Some((playlist.id, playlist.title.clone()));
                            ui.close();
                        }
                        if ui.button("Șterge playlistul").clicked() {
                            delete = Some((playlist.id, playlist.title.clone()));
                            ui.close();
                        }
                        if ui.button("Șterge tot (playlist și melodii)").clicked() {
                            delete_all = Some(CollectionDelete::Playlist {
                                id: playlist.id,
                                title: playlist.title.clone(),
                                delete_tracks: true,
                            });
                            ui.close();
                        }
                    });
                    ui.add_space(6.0);
                }
            });
        if let Some(playlist_id) = open {
            self.open_playlist(playlist_id);
        }
        if let Some(playlist_id) = play {
            match self
                .db
                .playlist_page_sorted(playlist_id, 0, 1, "", self.sort)
            {
                Ok(page) => match page.tracks.first() {
                    Some(track) => {
                        if let Err(error) =
                            self.player
                                .play_playlist_sorted(track.id, playlist_id, "", self.sort)
                        {
                            self.toast(error);
                        }
                        self.sync_song();
                    }
                    None => self.toast("Playlistul nu conține piese disponibile."),
                },
                Err(error) => self.toast(error),
            }
        }
        if rename.is_some() {
            self.playlist_rename = rename;
        }
        if let Some((id, title)) = delete {
            self.collection_delete_pending = Some(CollectionDelete::Playlist {
                id,
                title,
                delete_tracks: false,
            });
        }
        if delete_all.is_some() {
            self.collection_delete_pending = delete_all;
        }
    }
    fn albums_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal(|ui| {
            title(ui, "Albumele mele");
            ui.add_space(10.0);
            label(
                ui,
                format!("{} albume afișate", self.albums.len()),
                13.0,
                MUTED,
            );
        });
        ui.add_space(16.0);
        let albums = self.albums.clone();
        self.album_cards(ui, ctx, &albums, None);
    }
    fn disco_settings(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal_wrapped(|ui| {
            let enabled = self.preferences.disco_ball_enabled;
            if ui
                .add_sized(
                    [ui.available_width().min(320.0), 42.0],
                    self.icon_button(
                        if enabled {
                            "disco-ball"
                        } else {
                            "disco-ball-off"
                        },
                        "",
                        if enabled {
                            "Bila disco: activată"
                        } else {
                            "Bila disco: dezactivată"
                        },
                    ),
                )
                .clicked()
            {
                self.preferences.disco_ball_enabled = !enabled;
                self.disco.pause();
                self.save_preferences();
                ctx.request_repaint();
            }
            if self.preferences.disco_ball_enabled {
                let before = self.preferences.disco_mode;
                egui::ComboBox::from_id_salt("disco-mode")
                    .width(150.0)
                    .selected_text(before.label())
                    .show_ui(ui, |ui| {
                        for mode in [DiscoMode::Party, DiscoMode::Moving, DiscoMode::Static] {
                            ui.selectable_value(
                                &mut self.preferences.disco_mode,
                                mode,
                                mode.label(),
                            );
                        }
                    });
                if before != self.preferences.disco_mode {
                    self.save_preferences();
                    ctx.request_repaint();
                }
            }
        });
        if self.preferences.disco_ball_enabled {
            label(ui, match self.preferences.disco_mode {
                DiscoMode::Party => "Rotație 360° și balans: lovește bila cu mouse-ul pentru a o pune în mișcare.",
                DiscoMode::Moving => "Bila se rotește 360°; poziția ei rămâne fixă.",
                DiscoMode::Static => "Bila și direcțiile rămân fixe. Razele apar și dispar treptat prin fade, independent.",
            }, 12.0, MUTED);
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.label("Număr total de raze");
                let mode = self.preferences.disco_mode;
                let mut count = self.preferences.disco_ray_count(mode);
                if ui.add(egui::Slider::new(&mut count, 0..=256)).changed() {
                    self.preferences.set_disco_ray_count(mode, count);
                    self.save_preferences();
                    ctx.request_repaint();
                }
            });
            label(
                ui,
                "Valoare separată pentru fiecare mod. La 0, bila rămâne luminată, fără raze.",
                12.0,
                MUTED,
            );
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.label("Reflexii la marginea ferestrei");
                if ui
                    .add(
                        egui::Slider::new(
                            &mut self.preferences.disco_window_reflection_percent,
                            0..=100,
                        )
                        .suffix("%"),
                    )
                    .changed()
                {
                    self.save_preferences();
                    ctx.request_repaint();
                }
            });
            label(ui, "Probabilitate aleatoare pentru fiecare rază; maximum o reflexie pe marginea ferestrei.", 12.0, MUTED);
        }
        ui.add_space(8.0);
        let enabled = self.preferences.disco_floor_enabled;
        if ui
            .add_sized(
                [ui.available_width().min(320.0), 42.0],
                self.icon_button(
                    if enabled {
                        "disco-floor"
                    } else {
                        "disco-floor-off"
                    },
                    "",
                    if enabled {
                        "Podeaua disco: activată"
                    } else {
                        "Podeaua disco: dezactivată"
                    },
                ),
            )
            .clicked()
        {
            self.preferences.disco_floor_enabled = !enabled;
            self.save_preferences();
            ctx.request_repaint();
        }
        if self.preferences.disco_floor_enabled {
            label(
                ui,
                "Culorile urmăresc ritmul și energia melodiei. Fără muzică, podeaua rămâne gri.",
                12.0,
                MUTED,
            );
        }
    }

    fn settings(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
        egui::ScrollArea::vertical().id_salt("settings-page")
            .auto_shrink([false, false]).show(ui, |ui| {
        let panel = glass_frame(ui, CARD, 14, 18).show(ui, |ui| {
            ui.set_width(ui.available_width());
            title(ui, "Setări");
            ui.add_space(16.0);
            ui.heading("Aspect");
            label(ui, "Interfață în modul întunecat", 13.0, MUTED);
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.label("Temă");
                let before = self.preferences.appearance;
                egui::ComboBox::from_id_salt("appearance-selector")
                    .width(160.0).selected_text(before.label()).show_ui(ui, |ui| {
                        for appearance in [Appearance::Aqua, Appearance::Beer, Appearance::Disco] {
                            ui.selectable_value(&mut self.preferences.appearance, appearance, appearance.label());
                        }
                    });
                if before != self.preferences.appearance {
                    self.reload_appearance(ctx);
                    self.save_preferences();
                }
            });
            ui.add_space(12.0);
            if self.preferences.appearance == Appearance::Disco {
                self.disco_settings(ui, ctx);
            } else {
            let beer = self.preferences.appearance == Appearance::Beer;
            ui.horizontal_wrapped(|ui| {
                let fish_icon = match (beer, self.preferences.fish_enabled) {
                    (true, true) => "beer-hand", (true, false) => "beer-hand-off",
                    (false, true) => "fish", (false, false) => "fish-off",
                };
                let fish_label = match (beer, self.preferences.fish_enabled) {
                    (true, true) => "Mâna cu bere: activată", (true, false) => "Mâna cu bere: dezactivată",
                    (false, true) => "Pești: activați", (false, false) => "Pești: dezactivați",
                };
                if ui.add_sized([ui.available_width().min(320.0), 42.0], self.icon_button(fish_icon, "", fish_label)).clicked() {
                    self.preferences.fish_enabled = !self.preferences.fish_enabled;
                    self.fish.pause();
                    self.beer_hand.pause();
                    self.save_preferences();
                    ctx.request_repaint();
                }
                if self.preferences.fish_enabled {
                    if beer {
                        let before = self.preferences.beer_frequency;
                        egui::ComboBox::from_id_salt("beer-frequency").width(150.0)
                            .selected_text(before.label()).show_ui(ui, |ui| {
                                for frequency in [BeerFrequency::Rare, BeerFrequency::Medium, BeerFrequency::Frequent] {
                                    ui.selectable_value(&mut self.preferences.beer_frequency, frequency, frequency.label());
                                }
                            });
                        if before != self.preferences.beer_frequency {
                            self.beer_hand.set_frequency(self.preferences.beer_frequency);
                            self.save_preferences();
                        }
                    } else if density_selector(ui, "fish-density", &mut self.preferences.fish_density, true) {
                        self.fish.set_density(self.preferences.fish_density);
                        self.save_preferences();
                        ctx.request_repaint();
                    }
                }
            });
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                let bubble_icon = match (beer, self.preferences.bubbles_enabled) {
                    (true, true) => "beer", (true, false) => "beer-off",
                    (false, true) => "bubble", (false, false) => "bubble-off",
                };
                let bubble_label = if beer {
                    if !self.preferences.bubbles_enabled { "Halbe: dezactivate" }
                    else if self.preferences.bubble_density == EffectDensity::Few { "Halbe la click: activate" }
                    else { "Halbe la hover: activate" }
                } else if !self.preferences.bubbles_enabled { "Bule: dezactivate" }
                    else if self.preferences.bubble_density == EffectDensity::Few { "Bule la click: activate" }
                    else { "Bule la hover: activate" };
                if ui.add_sized([ui.available_width().min(320.0), 42.0], self.icon_button(bubble_icon, "", bubble_label)).clicked() {
                    self.preferences.bubbles_enabled = !self.preferences.bubbles_enabled;
                    BubbleSystem::set_enabled(ctx, self.preferences.bubbles_enabled);
                    self.save_preferences();
                    ctx.request_repaint();
                }
                if self.preferences.bubbles_enabled && density_selector(ui, "bubble-density", &mut self.preferences.bubble_density, false) {
                    BubbleSystem::set_density(ctx, self.preferences.bubble_density);
                    self.save_preferences();
                    ctx.request_repaint();
                }
            });
            if self.preferences.bubbles_enabled && self.preferences.bubble_density == EffectDensity::Few {
                label(ui, if beer { "5–10 halbe, numai la click, din colțurile de sus ale butonului." }
                    else { "5–10 bule, numai la click, din colțurile de sus ale butonului." }, 12.0, MUTED);
            }
            }
            ui.add_space(20.0);
            ui.heading("Imagine de fundal");
            label(ui, if self.preferences.active_background().is_some() { "Fundal personalizat" } else { "Fundal implicit" }, 13.0, MUTED);
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                if ui.add(self.icon_button("background", "", "Alege o imagine din PC")).clicked() {
                    self.choose_background(ctx);
                }
                if self.preferences.active_background().is_some() && ui.button("Restabilește fundalul implicit").clicked() {
                    self.preferences.set_background(None);
                    self.reload_appearance(ctx);
                    self.save_preferences();
                    ctx.request_repaint();
                }
            });
            if self.preferences.active_background().is_some() {
                if let Some(texture) = &self.background_texture {
                    ui.add_space(8.0);
                    let width = ui.available_width().min(240.0);
                    let size = texture.size_vec2();
                    let scale = (width / size.x).min(130.0 / size.y).min(1.0);
                    ui.add(egui::Image::new((texture.id(), size * scale)).corner_radius(8));
                }
            }
            ui.add_space(20.0);
            ui.heading("Directoare muzicale");
            if self.roots.is_empty() {
                label(ui, "Nu ai configurat niciun folder.", 14.0, MUTED);
            }
            for root in &self.roots {
                ui.label(root);
            }
            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add(self.icon_button("add-music", "＋", "Adaugă director"))
                    .clicked()
                {
                    self.choose_folder(ctx);
                }
                if ui
                    .add(self.icon_button("rescan", "⟳", "Actualizează biblioteca"))
                    .clicked()
                {
                    self.scan_all(ctx);
                }
            });
            ui.add_space(25.0);
            ui.heading("Despre S4GGZ");
            label(
                ui,
                APP_DESCRIPTION,
                14.0,
                MUTED,
            );
        });
        paint_frost_grain_in_rect(ui, &self.frost, panel.response.rect.shrink(18.0));
        });
    }
    fn drawer_ui(&mut self, ui: &mut egui::Ui, status: &PlayerStatus) {
        ui.horizontal(|ui| {
            ui.heading(match self.drawer {
                Some(Drawer::Queue) => "Coada de redare",
                _ => "Versuri",
            });
            let close_button = compact_icon_button(ui, "close", "Închide meniul", true);
            if close_button.clicked() {
                self.drawer = None;
            }
        });
        ui.add_space(8.);
        match self.drawer {
            Some(Drawer::Queue) => {
                label(
                    ui,
                    format!("{} piese următoare", status.queue_len),
                    12.0,
                    MUTED,
                );
                ui.add_space(10.0);
                let mut movement = None;
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for (i, track) in self.queue_tracks.iter().enumerate() {
                        let row_width = ui.available_width();
                        ui.horizontal(|ui| {
                            ui.set_width(row_width);
                            let controls_width = 108.0;
                            ui.allocate_ui_with_layout(
                                Vec2::new(
                                    (row_width - controls_width - ui.spacing().item_spacing.x)
                                        .max(100.0),
                                    41.0,
                                ),
                                egui::Layout::top_down(egui::Align::LEFT),
                                |ui| {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(&track.title).size(13.0).strong(),
                                        )
                                        .truncate(),
                                    );
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(&track.artist).size(11.0).color(MUTED),
                                        )
                                        .truncate(),
                                    );
                                },
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.spacing_mut().item_spacing.x = 4.;
                                    for (icon, name, kind, enabled) in [
                                        (
                                            "arrow-last",
                                            "Mută piesa ultima în coadă",
                                            player::QueueMove::Last,
                                            i + 1 < status.queue_len,
                                        ),
                                        (
                                            "arrow-down",
                                            "Mută piesa mai jos",
                                            player::QueueMove::Down,
                                            i + 1 < status.queue_len,
                                        ),
                                        (
                                            "arrow-up",
                                            "Mută piesa mai sus",
                                            player::QueueMove::Up,
                                            i > 0,
                                        ),
                                        (
                                            "arrow-first",
                                            "Mută piesa prima în coadă",
                                            player::QueueMove::First,
                                            i > 0,
                                        ),
                                    ] {
                                        if compact_icon_button(ui, icon, name, enabled).clicked() {
                                            movement = Some((track.id, kind));
                                        }
                                    }
                                },
                            );
                        });
                        ui.separator();
                    }
                });
                if let Some((id, movement)) = movement {
                    let result = self.player.move_queued(id, movement);
                    self.action(result);
                }
            }
            Some(Drawer::Lyrics) => {
                if self.lyrics.is_empty() && self.unsynced_lyrics.is_none() {
                    ui.add_space(30.0);
                    label(ui, "Nu sunt disponibile versuri locale.", 14.0, MUTED);
                    label(
                        ui,
                        "Pune un fișier .lrc lângă fișierul audio, cu același nume.",
                        12.0,
                        MUTED,
                    );
                } else {
                    let active = self
                        .lyrics
                        .iter()
                        .rposition(|l| l.time_ms <= status.position_ms);
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        if let Some(plain) = &self.unsynced_lyrics {
                            ui.label(plain);
                        }
                        for (i, line) in self.lyrics.iter().enumerate() {
                            let selected = Some(i) == active;
                            ui.add_space(7.0);
                            ui.label(
                                RichText::new(if line.text.is_empty() {
                                    "♪"
                                } else {
                                    &line.text
                                })
                                .size(if selected { 19.0 } else { 15.0 })
                                .color(if selected {
                                    theme::accent(ui.ctx())
                                } else {
                                    MUTED
                                })
                                .strong(),
                            );
                        }
                    });
                }
            }
            None => {}
        }
    }
    fn footer(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, status: &PlayerStatus) {
        let currently = status.current.clone();
        let footer_width = ui.available_width();
        let info_width = 240.0;
        let spacing = ui.spacing().item_spacing.x;
        ui.horizontal(|ui| {
            ui.set_min_height(78.0);
            ui.set_width(footer_width);

            // Song identification is bounded so long titles never eat the seek bar.
            ui.allocate_ui_with_layout(
                Vec2::new(info_width, 64.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_min_width(info_width);
                    ui.set_max_width(info_width);
                    let texture = currently.as_ref().and_then(|track| {
                        self.album_cover(track.album_id, ctx)
                            .or_else(|| self.cover(track.id, &track.file_path, ctx))
                    });
                    cover_tile(ui, texture, 56.0);
                    ui.add_space(6.0);
                    ui.vertical(|ui| {
                        let text_width = info_width - 82.0;
                        ui.set_min_width(text_width);
                        ui.set_max_width(text_width);
                        footer_title::show(
                            ui,
                            egui::Id::new("footer-title")
                                .with(currently.as_ref().map(|track| track.id)),
                            currently
                                .as_ref()
                                .map_or("Nicio piesă selectată", |track| &track.title),
                            text_width,
                        );
                        ui.add_sized(
                            [text_width, 18.0],
                            egui::Label::new(
                                RichText::new(currently.as_ref().map_or("S4GGZ", |t| &t.artist))
                                    .size(12.0)
                                    .color(MUTED),
                            )
                            .truncate(),
                        );
                    });
                },
            );

            // Volume precedes the transport cluster. From NEXT until QUEUE the
            // entire remaining space is now reserved for the timeline.
            let mut volume = status.volume as f64;
            label(ui, "♫", 15.0, MUTED);
            if liquid_slider(ui, &mut volume, 1.0, 78.0, "Volume").changed() {
                let result = self.player.volume(volume as f32);
                self.action(result);
            }
            ui.add_space(7.0);

            if skip_button(ui, false).clicked() {
                let result = self.player.previous(&self.db);
                self.action(result);
            }
            if play_button(ui, status.paused).clicked() {
                let result = self.player.toggle();
                self.action(result);
            }
            if let Some(mode) =
                playback_modes::show(ui, status.playback_mode, &mut self.primary_playback_mode)
            {
                let result = self.player.set_playback_mode(mode);
                self.action(result);
            }
            if skip_button(ui, true).clicked() {
                let result = self.player.next(&self.db);
                self.action(result);
            }

            // Queue owns the final 38 points on the right. The timeline consumes
            // ALL the flexible width between the Next and Queue buttons.
            let queue_width = 38.0;
            let timeline_width = (ui.available_width() - queue_width - spacing).max(0.0);
            ui.allocate_ui_with_layout(
                Vec2::new(timeline_width, 35.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    // Allocated child UIs otherwise shrink to their contents.
                    ui.set_min_width(timeline_width);
                    let duration = (status.duration_ms.max(0) as f64 / 1000.0).max(0.01);
                    let mut seconds = self
                        .seek_preview
                        .unwrap_or(status.position_ms as f64 / 1000.0)
                        .clamp(0.0, duration);
                    let timestamp_width = 53.0;
                    ui.add_sized(
                        [timestamp_width, 20.0],
                        egui::Label::new(
                            RichText::new(fmt_time(status.position_ms as i64))
                                .size(11.0)
                                .color(MUTED),
                        ),
                    );
                    // No artificial 360px cap; adapt to the native window width.
                    let bar_width = (ui.available_width() - timestamp_width - spacing).max(0.0);
                    let response =
                        liquid_slider(ui, &mut seconds, duration, bar_width, "Poziție redare");
                    if response.dragged() {
                        self.seek_preview = Some(seconds);
                    } else if response.changed() || self.seek_preview.take().is_some() {
                        let result = self.player.seek((seconds * 1000.0) as u64);
                        self.action(result);
                    }
                    ui.add_sized(
                        [timestamp_width, 20.0],
                        egui::Label::new(
                            RichText::new(fmt_time(status.duration_ms))
                                .size(11.0)
                                .color(MUTED),
                        ),
                    );
                },
            );
            if ui
                .add_sized([queue_width, 34.0], self.icon_button("queue", "☷", ""))
                .on_hover_text("Coada de redare")
                .clicked()
            {
                self.drawer = if self.drawer == Some(Drawer::Queue) {
                    None
                } else {
                    Some(Drawer::Queue)
                };
            }
        });
    }
}

fn cover_tile(ui: &mut egui::Ui, texture: Option<TextureHandle>, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let radius = (size * 0.15).min(14.0);
    if let Some(texture) = texture {
        let size = texture.size_vec2();
        let uv = if size.x > size.y {
            let visible = size.y / size.x;
            egui::Rect::from_min_max(
                egui::pos2((1.0 - visible) * 0.5, 0.0),
                egui::pos2((1.0 + visible) * 0.5, 1.0),
            )
        } else {
            let visible = size.x / size.y;
            egui::Rect::from_min_max(
                egui::pos2(0.0, (1.0 - visible) * 0.5),
                egui::pos2(1.0, (1.0 + visible) * 0.5),
            )
        };
        egui::Image::new((texture.id(), rect.size()))
            .uv(uv)
            .corner_radius(radius)
            .paint_at(ui, rect);
    } else {
        gradient_tile(
            ui.painter(),
            rect,
            radius,
            theme::cover_colors(ui.ctx()).0,
            theme::cover_colors(ui.ctx()).1,
        );
        ui.painter().circle_filled(
            rect.center(),
            size * 0.29,
            theme::accent(ui.ctx()).gamma_multiply(0.07),
        );
        ui.painter().circle_stroke(
            rect.center(),
            size * 0.28,
            egui::Stroke::new(1.0, theme::accent(ui.ctx()).gamma_multiply(0.18)),
        );
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "♫",
            egui::FontId::proportional(size * 0.42),
            WHITE,
        );
    }
    // A diagonal reflection entirely inside the rounded silhouette. Vertex alpha
    // feathers the highlight instead of applying a flat white overlay to artwork.
    let mut sheen = egui::epaint::Mesh::default();
    sheen.colored_vertex(
        rect.left_top() + Vec2::new(size * 0.35, 2.0),
        Color32::TRANSPARENT,
    );
    sheen.colored_vertex(
        rect.right_top() + Vec2::new(-radius, 2.0),
        Color32::from_white_alpha(65),
    );
    sheen.colored_vertex(
        rect.right_top() + Vec2::new(-2.0, radius),
        Color32::from_white_alpha(45),
    );
    sheen.colored_vertex(
        rect.right_top() + Vec2::new(-2.0, size * 0.65),
        Color32::TRANSPARENT,
    );
    sheen.add_triangle(0, 1, 2);
    sheen.add_triangle(0, 2, 3);
    ui.painter().add(egui::Shape::mesh(sheen));
    ui.painter()
        .rect_stroke(rect, radius, glass_edge(), egui::StrokeKind::Inside);
    specular_edge(ui.painter(), rect, radius, 1.0);
}
fn stats_card(ui: &mut egui::Ui, name: &str, count: String) {
    let card = glass_frame(ui, CARD, 14, 16).show(ui, |ui| {
        ui.vertical(|ui| {
            ui.set_min_width(120.0);
            label(ui, name, 11.0, MUTED);
            ui.label(RichText::new(count).size(26.0).strong().color(WHITE));
        });
    });
    specular_edge(ui.painter(), card.response.rect, 14.0, 1.0);
}
fn dark_visuals() -> egui::Visuals {
    dark_visuals_for(GlassPalette::default())
}
fn dark_visuals_for(palette: GlassPalette) -> egui::Visuals {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = Color32::TRANSPARENT;
    visuals.window_fill = palette.sidebar;
    visuals.window_corner_radius = 16.into();
    visuals.menu_corner_radius = 12.into();
    visuals.window_stroke = glass_edge();
    visuals.override_text_color = Some(WHITE);
    visuals.weak_text_color = Some(MUTED);
    visuals.faint_bg_color = Color32::from_white_alpha(5);
    visuals.extreme_bg_color = Color32::from_black_alpha(55);
    visuals.text_edit_bg_color = Some(if palette == GlassPalette::default() {
        Color32::from_rgba_premultiplied(5, 15, 25, 145)
    } else {
        palette.sidebar
    });
    visuals.hyperlink_color = palette.accent;
    for (widget, fill) in [
        (&mut visuals.widgets.noninteractive, palette.card),
        (&mut visuals.widgets.inactive, palette.card),
        (
            &mut visuals.widgets.hovered,
            palette.accent.gamma_multiply(0.18),
        ),
        (
            &mut visuals.widgets.active,
            palette.accent.gamma_multiply(0.30),
        ),
        (
            &mut visuals.widgets.open,
            palette.accent.gamma_multiply(0.24),
        ),
    ] {
        widget.bg_fill = fill;
        widget.weak_bg_fill = fill;
        widget.bg_stroke = glass_edge();
        widget.fg_stroke = egui::Stroke::new(1.0, WHITE);
        widget.corner_radius = 9.into();
        widget.expansion = 0.0;
    }
    visuals.selection.bg_fill = palette.accent.gamma_multiply(0.12);
    visuals.selection.stroke = egui::Stroke::new(1.0, palette.accent.gamma_multiply(0.5));
    visuals
}
impl eframe::App for AuraApp {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.display_scale.prepare(ctx, raw);
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.display_scale.apply(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.defer_close_for_deletion(ui.ctx());
        self.poll_track_deletion();
        if self
            .equalizer_save_after
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.equalizer_save_after = None;
            self.save_preferences();
        }
        let ctx = ui.ctx().clone();
        let disco = self.preferences.appearance == Appearance::Disco;
        BubbleSystem::set_enabled(&ctx, self.preferences.bubbles_enabled && !disco);
        BubbleSystem::set_density(&ctx, self.preferences.bubble_density);
        BubbleSystem::begin_frame(&ctx);
        paint_ambient(ui, self.background_texture.as_ref());
        self.fish.set_density(self.preferences.fish_density);
        self.beer_hand
            .set_frequency(self.preferences.beer_frequency);
        if self.preferences.fish_enabled {
            match self.preferences.appearance {
                Appearance::Aqua => self.fish.paint(ui),
                Appearance::Beer => self.beer_hand.paint(ui),
                Appearance::Disco => {}
            }
        }
        self.poll_events(&ctx);
        self.sync_song();
        if self.deletions.pending.is_none()
            && !ctx.egui_wants_keyboard_input()
            && ctx.input(|i| i.key_pressed(egui::Key::Space))
        {
            let result = self.player.toggle();
            self.action(result);
        }
        if self.deletions.pending.is_none()
            && ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::O))
        {
            self.choose_folder(&ctx);
        }
        let status = self.player.status().unwrap_or(PlayerStatus {
            current: None,
            position_ms: 0,
            duration_ms: 0,
            paused: true,
            playback_mode: player::PlaybackMode::Off,
            #[cfg(test)]
            shuffle: false,
            #[cfg(test)]
            repeat: player::RepeatMode::Off,
            volume: 0.74,
            queue: vec![],
            queue_len: 0,
            error: None,
            audio_reactive: Default::default(),
        });
        if disco {
            let bounds = ui.max_rect();
            self.disco.set_ray_count(
                self.preferences
                    .disco_ray_count(self.preferences.disco_mode),
            );
            self.disco
                .set_window_reflection_percent(self.preferences.disco_window_reflection_percent);
            self.disco.set_stage_bounds(egui::Rect::from_x_y_ranges(
                bounds.left() + 263.0
                    ..=bounds.right()
                        - if self.screen == Screen::AddSongs {
                            download_queue_width(bounds.width()) + 24.0
                        } else if self.drawer.is_some() {
                            344.0
                        } else {
                            24.0
                        },
                bounds.y_range(),
            ));
            self.disco.begin_frame(
                ui,
                self.preferences.disco_mode,
                self.preferences.disco_ball_enabled,
                self.preferences.disco_floor_enabled,
                status.audio_reactive,
            );
        }
        match status.error.as_ref() {
            Some(error) if self.reported_player_error.as_ref() != Some(error) => {
                self.reported_player_error = Some(error.clone());
                self.toast(error.clone());
            }
            Some(_) => {}
            None => self.reported_player_error = None,
        }
        egui::Panel::bottom("aura-footer")
            .exact_size(126.0)
            .show_separator_line(false)
            .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(16, 10)))
            .show(ui, |ui| {
                let island = glass_frame(ui, SIDEBAR, 18, 12).show(ui, |ui| {
                    paint_frost(ui, &self.frost);
                    self.footer(ui, &ctx, &status);
                });
                specular_edge(ui.painter(), island.response.rect, 18.0, 1.4);
            });
        egui::Panel::left("aura-sidebar")
            .exact_size(239.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(glass_frame(ui, SIDEBAR, 18, 15).outer_margin(egui::Margin {
                left: 12,
                right: 4,
                top: 12,
                bottom: 4,
            }))
            .show(ui, |ui| {
                paint_frost(ui, &self.frost);
                self.sidebar(ui, &ctx);
            });
        self.right_panels(ui, &ctx, &status);
        egui::CentralPanel::default()
            .frame(egui::Frame::new().inner_margin(24))
            .show(ui, |ui| self.main_content(ui, &ctx));
        if disco {
            self.disco.finish_frame(&ctx);
        }
        if self.preferences.bubbles_enabled && !disco {
            if let Some(texture) = &self.bubble_texture {
                BubbleSystem::paint(&ctx, texture);
            }
        }
        readability::finish_frame(&ctx);
        self.equalizer_window(&ctx);
        self.deletion_popup(&ctx);
        self.playlist_delete_window(&ctx);
        self.album_rename_window(&ctx);
        self.playlist_rename_window(&ctx);
        self.track_rename_window(&ctx);
        self.playlist_dialog_window(&ctx);
        // Animations request their own frames while settling; playback ticks at
        // display cadence. Idle/paused windows remain inexpensive.
        ctx.request_repaint_after(Duration::from_millis(if status.paused { 300 } else { 16 }));
    }
    fn on_exit(&mut self) {
        self.equalizer_open = false;
        self.context
            .send_viewport_cmd_to(equalizer_ui::viewport_id(), egui::ViewportCommand::Close);
        if self.equalizer_save_after.take().is_some() {
            self.save_preferences();
        }
    }
}

fn main() -> eframe::Result {
    let local = dirs::data_local_dir()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or(PathBuf::from(".")));
    let db = match Database::new(local.join("AuraPlayer").join("aura_library.sqlite3")) {
        Ok(db) => db,
        Err(e) => {
            eprintln!("Baza de date nu poate fi inițializată: {e}");
            rfd::MessageDialog::new().set_title("S4GGZ — Biblioteca nu poate fi deschisă")
                .set_description(format!("Baza de date: {}\n\n{e}\n\nVerifică accesul și spațiul disponibil. Datele nu sunt șterse automat.",local.join("AuraPlayer/aura_library.sqlite3").display()))
                .set_level(rfd::MessageLevel::Error).show();
            std::process::exit(1);
        }
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("S4GGZ — Native Edition")
            .with_inner_size([1280.0, 790.0])
            .with_min_inner_size([950.0, 610.0]),
        ..Default::default()
    };
    eframe::run_native(
        "S4GGZ",
        options,
        Box::new(move |cc| Ok(Box::new(AuraApp::new(db, &cc.egui_ctx)))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{wait_for_scans, TestLibrary};

    #[test]
    fn disco_settings_restore_independent_effects_mode_and_photo() {
        let library = TestLibrary::new("disco-startup");
        let path = library.directory.join("settings.json");
        let photo = library.directory.join("disco-photo.png");
        image::RgbaImage::from_pixel(32, 18, image::Rgba([60, 30, 180, 255]))
            .save(&photo)
            .unwrap();
        let preferences = AppSettings {
            appearance: Appearance::Disco,
            disco_mode: DiscoMode::Static,
            disco_background: Some(photo),
            fish_enabled: false,
            bubbles_enabled: false,
            ..Default::default()
        };
        preferences.save(&path).unwrap();
        let ctx = egui::Context::default();
        let mut app = AuraApp::with_settings_path(
            library.db.clone(),
            &ctx,
            Ok(library.music.clone()),
            Some(path),
        );
        wait_for_scans(&mut app, &ctx);
        assert_eq!(app.preferences, preferences);
        assert_eq!(app.background_texture.as_ref().unwrap().size(), [32, 18]);
        assert!(app.bubble_texture.is_none());
        let labels = |output: &egui::FullOutput| {
            output
                .shapes
                .iter()
                .filter_map(|shape| {
                    if let egui::Shape::Text(text) = &shape.shape {
                        Some(text.galley.text().to_owned())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        };
        let output = crate::test_support::run_ui(&ctx, egui::RawInput::default(), |ui| {
            app.settings(ui, &ctx)
        });
        let texts = labels(&output);
        assert!(texts.iter().any(|text| text == "Disco"));
        assert!(texts.iter().any(|text| text == "Statică"));
        assert!(texts.iter().any(|text| text == "Bila disco: activată"));
        assert!(texts.iter().any(|text| text == "Podeaua disco: activată"));
        assert!(!texts
            .iter()
            .any(|text| text.starts_with("Pești:") || text.starts_with("Bule")));
        app.preferences.disco_ball_enabled = false;
        let output = crate::test_support::run_ui(&ctx, egui::RawInput::default(), |ui| {
            app.settings(ui, &ctx)
        });
        let texts = labels(&output);
        assert!(texts.iter().any(|text| text == "Bila disco: dezactivată"));
        assert!(!texts.iter().any(|text| text == "Statică"));
        assert!(texts.iter().any(|text| text == "Podeaua disco: activată"));
        app.preferences.appearance = Appearance::Beer;
        app.reload_appearance(&ctx);
        app.preferences.appearance = Appearance::Disco;
        app.reload_appearance(&ctx);
        assert_eq!(app.background_texture.as_ref().unwrap().size(), [32, 18]);
        assert!(!app.preferences.disco_ball_enabled);
        assert_eq!(app.preferences.disco_mode, DiscoMode::Static);
    }

    #[test]
    fn density_dropdowns_are_visible_only_for_enabled_effects() {
        let library = TestLibrary::new("effect-dropdowns");
        let ctx = egui::Context::default();
        let mut app =
            AuraApp::with_music_directory(library.db.clone(), &ctx, Ok(library.music.clone()));
        wait_for_scans(&mut app, &ctx);
        let labels = |output: &egui::FullOutput| {
            output
                .shapes
                .iter()
                .filter_map(|shape| {
                    if let egui::Shape::Text(text) = &shape.shape {
                        Some(text.galley.text().to_owned())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        };
        let enabled = crate::test_support::run_ui(&ctx, egui::RawInput::default(), |ui| {
            app.settings(ui, &ctx)
        });
        let enabled = labels(&enabled);
        assert!(enabled.iter().any(|text| text == "Mulți"));
        assert!(enabled.iter().any(|text| text == "Multe"));
        app.preferences.fish_enabled = false;
        app.preferences.bubbles_enabled = false;
        let disabled = crate::test_support::run_ui(&ctx, egui::RawInput::default(), |ui| {
            app.settings(ui, &ctx)
        });
        let disabled = labels(&disabled);
        assert!(!disabled
            .iter()
            .any(|text| text == "Mulți" || text == "Multe"));
    }

    #[test]
    fn startup_restores_effect_preferences_and_custom_background_in_dark_mode() {
        let library = TestLibrary::new("appearance-startup");
        let path = library.directory.join("settings.json");
        let background = library.directory.join("background.png");
        image::RgbaImage::from_pixel(30, 20, image::Rgba([30, 60, 90, 255]))
            .save(&background)
            .unwrap();
        let preferences = AppSettings {
            fish_enabled: false,
            bubbles_enabled: false,
            fish_density: EffectDensity::Medium,
            bubble_density: EffectDensity::Few,
            background: Some(background),
            ..Default::default()
        };
        preferences.save(&path).unwrap();
        let ctx = egui::Context::default();
        ctx.set_theme(egui::ThemePreference::Light);
        let mut app = AuraApp::with_settings_path(
            library.db.clone(),
            &ctx,
            Ok(library.music.clone()),
            Some(path),
        );
        wait_for_scans(&mut app, &ctx);
        assert_eq!(app.preferences, preferences);
        assert_eq!(app.background_texture.as_ref().unwrap().size(), [30, 20]);
        assert_ne!(theme::current(&ctx), GlassPalette::default());
        assert!(theme::current(&ctx).card.b() > theme::current(&ctx).card.r());
        assert_eq!(ctx.theme(), egui::Theme::Dark);
        assert!(ctx.style_of(egui::Theme::Dark).visuals.dark_mode);
    }

    #[test]
    fn theme_switch_loads_real_assets_and_restores_custom_aqua_background() {
        let library = TestLibrary::new("theme-switch");
        let ctx = egui::Context::default();
        let mut app =
            AuraApp::with_music_directory(library.db.clone(), &ctx, Ok(library.music.clone()));
        wait_for_scans(&mut app, &ctx);
        assert!(app.background_texture.is_some() && app.bubble_texture.is_some());
        let photo = library.directory.join("aqua-photo.png");
        image::RgbaImage::from_pixel(24, 16, image::Rgba([200, 10, 80, 255]))
            .save(&photo)
            .unwrap();
        app.preferences.set_background(Some(photo));
        app.reload_appearance(&ctx);
        assert_eq!(app.background_texture.as_ref().unwrap().size(), [24, 16]);
        app.preferences.appearance = Appearance::Beer;
        app.reload_appearance(&ctx);
        assert_eq!(app.background_texture.as_ref().unwrap().size(), [1200, 675]);
        assert_eq!(app.bubble_texture.as_ref().unwrap().size(), [256, 256]);
        assert_eq!(
            theme::current(&ctx),
            GlassPalette::for_appearance(Appearance::Beer)
        );
        let output = crate::test_support::run_ui(&ctx, egui::RawInput::default(), |ui| {
            app.settings(ui, &ctx)
        });
        let texts: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text.galley.text()),
                _ => None,
            })
            .collect();
        assert!(texts.contains(&"Beer"));
        assert!(texts.contains(&"Doar o bere"));
        assert!(texts.contains(&"Mâna cu bere: activată"));
        assert!(!texts.contains(&"Pești: activați"));
        app.preferences.appearance = Appearance::Aqua;
        app.reload_appearance(&ctx);
        assert_eq!(app.background_texture.as_ref().unwrap().size(), [24, 16]);
        assert_eq!(theme::accent(&ctx), ACCENT);
        assert!(theme::current(&ctx).card.r() > theme::current(&ctx).card.b());
    }

    #[test]
    fn library_and_favorites_paginate_in_batches_of_twenty_without_losing_tracks() {
        let library = TestLibrary::new("twenty-track-pages");
        for index in 0..45 {
            library.mp3("Pagination Artist", &format!("Track {index:02}"));
        }
        let ctx = egui::Context::default();
        let mut app =
            AuraApp::with_music_directory(library.db.clone(), &ctx, Ok(library.music.clone()));
        wait_for_scans(&mut app, &ctx);
        app.navigate(Screen::Songs);
        wait_for_scans(&mut app, &ctx);
        let mut ids = Vec::new();
        for (page_number, expected_length) in [(0, 20), (1, 20), (2, 5)] {
            app.page_number = page_number;
            app.refresh();
            wait_for_scans(&mut app, &ctx);
            assert_eq!(app.page.total, 45);
            assert_eq!(app.page.tracks.len(), expected_length);
            ids.extend(app.page.tracks.iter().map(|track| track.id));
        }
        assert_eq!(ids, library.db.order(None, false).unwrap());
        assert_eq!(ids.iter().collect::<HashSet<_>>().len(), 45);

        for &id in ids.iter().take(23) {
            library.db.favorites(id).unwrap();
        }
        app.navigate(Screen::Favorites);
        wait_for_scans(&mut app, &ctx);
        assert_eq!(app.page_number, 0);
        assert_eq!(app.page.total, 23);
        assert_eq!(app.page.tracks.len(), 20);
        app.page_number = 1;
        app.refresh();
        wait_for_scans(&mut app, &ctx);
        assert_eq!(app.page.tracks.len(), 3);
        assert!(app.page.tracks.iter().all(|track| track.is_favorite));
    }

    #[test]
    fn startup_and_periodic_scans_update_songs_and_the_downloads_album() {
        let library = TestLibrary::new("automatic-scans");
        library.mp3("First Author", "First Song");
        let ctx = egui::Context::default();
        let mut app =
            AuraApp::with_music_directory(library.db.clone(), &ctx, Ok(library.music.clone()));
        wait_for_scans(&mut app, &ctx);
        assert_eq!(app.page.total, 1);
        assert_eq!(app.albums.len(), 1);
        assert_eq!(app.albums[0].title, "Descarcate");
        let album_id = app.albums[0].id;

        app.navigate(Screen::Albums);
        wait_for_scans(&mut app, &ctx);
        library.mp3("Second Author", "Second Song");
        app.last_music_scan = Instant::now() - MUSIC_SCAN_INTERVAL;
        app.poll_events(&ctx);
        assert!(app.music_scan_running);
        wait_for_scans(&mut app, &ctx);
        assert_eq!(app.albums.len(), 1);
        assert_eq!(app.albums[0].track_count, 2);

        app.navigate(Screen::Songs);
        wait_for_scans(&mut app, &ctx);
        app.album_filter = Some(album_id);
        app.refresh();
        wait_for_scans(&mut app, &ctx);
        assert_eq!(app.page.total, 2);
        let titles: Vec<_> = app
            .page
            .tracks
            .iter()
            .map(|track| track.title.as_str())
            .collect();
        assert!(titles.contains(&"First Song"));
        assert!(titles.contains(&"Second Song"));
        assert_eq!(
            library
                .db
                .scan_downloaded_music(&library.music)
                .unwrap()
                .updated,
            0
        );
    }

    #[test]
    fn manual_rescan_keeps_music_files_in_the_downloads_album() {
        let library = TestLibrary::new("manual-music-scan");
        let path = library.mp3("Folder Author", "File Title");
        let ctx = egui::Context::default();
        let mut app =
            AuraApp::with_music_directory(library.db.clone(), &ctx, Ok(library.music.clone()));
        wait_for_scans(&mut app, &ctx);
        let album_id = app.albums[0].id;
        let audio = std::fs::read(&path).unwrap();
        std::fs::write(&path, audio.repeat(2)).unwrap();
        app.scan(library.music.to_string_lossy().into_owned(), &ctx);
        wait_for_scans(&mut app, &ctx);
        assert_eq!(app.albums.len(), 1);
        assert_eq!(app.albums[0].id, album_id);
        assert_eq!(app.page.tracks[0].title, "File Title");
        assert_eq!(app.page.tracks[0].artist, "Folder Author");
        assert_eq!(app.page.tracks[0].album, "Descarcate");
    }

    #[test]
    fn lrc_timestamps_and_order() {
        let (lines, plain) = parse_lyrics("[00:12.40]Hello\n[00:01.20]Start\n");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "Start");
        assert_eq!(lines[1].time_ms, 12_400);
        assert!(plain.is_none());
        let (two, _) = parse_lyrics("[00:01.00][00:02.00]Repeat");
        assert_eq!(two.len(), 2);
        assert_eq!(two[0].text, "Repeat");
    }
    #[test]
    fn responsive_track_table_columns() {
        assert_eq!(
            TrackTableLayout::for_width(350.0),
            TrackTableLayout {
                show_artist: false,
                show_album: false
            }
        );
        assert_eq!(
            TrackTableLayout::for_width(600.0),
            TrackTableLayout {
                show_artist: true,
                show_album: false
            }
        );
        assert_eq!(
            TrackTableLayout::for_width(900.0),
            TrackTableLayout {
                show_artist: true,
                show_album: true
            }
        );
    }
    #[test]
    fn time_formatting() {
        assert_eq!(fmt_time(65_500), "1:05");
        assert_eq!(fmt_time(3_605_000), "1:00:05");
    }
    #[test]
    fn album_cards_wrap_only_when_a_whole_card_fits() {
        assert_eq!(album_columns(190.0), 1);
        assert_eq!(album_columns(391.0), 1);
        assert_eq!(album_columns(392.0), 2);
        assert_eq!(album_columns(593.0), 2);
        assert_eq!(album_columns(594.0), 3);
        assert_eq!(album_columns(1200.0), 6);
    }
    #[test]
    fn clicking_an_album_card_opens_it_and_requests_playback() {
        let library = TestLibrary::new("album-card-play");
        library.mp3("Artist", "First Track");
        let ctx = egui::Context::default();
        let mut app =
            AuraApp::with_music_directory(library.db.clone(), &ctx, Ok(library.music.clone()));
        wait_for_scans(&mut app, &ctx);
        let album = app.albums[0].clone();
        let albums = app.albums.clone();
        let screen_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 700.0));
        let mut output = crate::test_support::run_ui(
            &ctx,
            egui::RawInput {
                screen_rect: Some(screen_rect),
                ..Default::default()
            },
            |ui| app.album_cards(ui, &ctx, &albums, None),
        );
        output.textures_delta.clear();
        let position = egui::pos2(95.0, 95.0);
        for pressed in [true, false] {
            let mut output = crate::test_support::run_ui(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(screen_rect),
                    events: vec![
                        egui::Event::PointerMoved(position),
                        egui::Event::PointerButton {
                            pos: position,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                    ..Default::default()
                },
                |ui| app.album_cards(ui, &ctx, &albums, None),
            );
            output.textures_delta.clear();
        }
        assert!(app.screen == Screen::Songs);
        assert_eq!(app.album_filter, Some(album.id));
        drop(app);
        std::thread::sleep(Duration::from_millis(100));
    }
    #[test]
    fn right_clicking_an_album_card_keeps_its_context_menu_available() {
        let library = TestLibrary::new("album-card-context-menu");
        library.mp3("Artist", "First Track");
        let ctx = egui::Context::default();
        let mut app =
            AuraApp::with_music_directory(library.db.clone(), &ctx, Ok(library.music.clone()));
        wait_for_scans(&mut app, &ctx);
        app.screen = Screen::Albums;
        let albums = app.albums.clone();
        let screen_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 700.0));
        let position = egui::pos2(95.0, 95.0);
        let mut output = crate::test_support::run_ui(
            &ctx,
            egui::RawInput {
                screen_rect: Some(screen_rect),
                ..Default::default()
            },
            |ui| app.album_cards(ui, &ctx, &albums, None),
        );
        output.textures_delta.clear();
        let mut output = crate::test_support::run_ui(
            &ctx,
            egui::RawInput {
                screen_rect: Some(screen_rect),
                events: vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Secondary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                ..Default::default()
            },
            |ui| app.album_cards(ui, &ctx, &albums, None),
        );
        output.textures_delta.clear();
        let mut output = crate::test_support::run_ui(
            &ctx,
            egui::RawInput {
                screen_rect: Some(screen_rect),
                events: vec![egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Secondary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                ..Default::default()
            },
            |ui| app.album_cards(ui, &ctx, &albums, None),
        );
        output.textures_delta.clear();
        let mut output = crate::test_support::run_ui(
            &ctx,
            egui::RawInput {
                screen_rect: Some(screen_rect),
                ..Default::default()
            },
            |ui| app.album_cards(ui, &ctx, &albums, None),
        );
        let menu_text = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                _ => None,
            })
            .collect::<Vec<_>>();
        output.textures_delta.clear();
        assert!(
            menu_text.iter().any(|text| text == "Redenumește albumul"),
            "Context menu text: {menu_text:?}"
        );
        assert!(menu_text.iter().any(|text| text == "Șterge albumul"));
        assert!(menu_text
            .iter()
            .any(|text| text == "Șterge tot (album și melodii)"));
        assert!(app.screen == Screen::Albums);
        drop(app);
        std::thread::sleep(Duration::from_millis(100));
    }
    #[test]
    fn right_clicking_a_playlist_shows_the_rename_action() {
        let library = TestLibrary::new("playlist-context-menu");
        library.db.create_playlist("Road Mix").unwrap();
        let ctx = egui::Context::default();
        let mut app =
            AuraApp::with_music_directory(library.db.clone(), &ctx, Ok(library.music.clone()));
        wait_for_scans(&mut app, &ctx);
        app.refresh();
        wait_for_scans(&mut app, &ctx);
        app.screen = Screen::Playlists;
        let screen_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 700.0));
        let position = egui::pos2(95.0, 95.0);
        let mut output = crate::test_support::run_ui(
            &ctx,
            egui::RawInput {
                screen_rect: Some(screen_rect),
                ..Default::default()
            },
            |ui| app.playlists_ui(ui),
        );
        output.textures_delta.clear();
        for pressed in [true, false] {
            let mut output = crate::test_support::run_ui(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(screen_rect),
                    events: vec![
                        egui::Event::PointerMoved(position),
                        egui::Event::PointerButton {
                            pos: position,
                            button: egui::PointerButton::Secondary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                    ..Default::default()
                },
                |ui| app.playlists_ui(ui),
            );
            output.textures_delta.clear();
        }
        let output = crate::test_support::run_ui(
            &ctx,
            egui::RawInput {
                screen_rect: Some(screen_rect),
                ..Default::default()
            },
            |ui| app.playlists_ui(ui),
        );
        let menu_text = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(menu_text
            .iter()
            .any(|text| text == "Redenumește playlistul"));
        assert!(menu_text.iter().any(|text| text == "Șterge playlistul"));
        assert!(menu_text
            .iter()
            .any(|text| text == "Șterge tot (playlist și melodii)"));
        assert!(app.screen == Screen::Playlists);
        drop(app);
        std::thread::sleep(Duration::from_millis(100));
    }
    #[test]
    fn clicking_playlist_play_button_runs_the_play_action() {
        let library = TestLibrary::new("playlist-play-button");
        library.mp3("Artist", "Song");
        library.db.scan_downloaded_music(&library.music).unwrap();
        let track = library
            .db
            .page(0, 10, "", false, None)
            .unwrap()
            .tracks
            .remove(0);
        let playlist_id = library.db.create_playlist("Play this track").unwrap();
        library
            .db
            .add_track_to_playlist(playlist_id, track.id)
            .unwrap();
        let ctx = egui::Context::default();
        let mut app =
            AuraApp::with_music_directory(library.db.clone(), &ctx, Ok(library.music.clone()));
        wait_for_scans(&mut app, &ctx);
        app.screen = Screen::Playlists;
        let screen_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 700.0));
        let mut output = crate::test_support::run_ui(
            &ctx,
            egui::RawInput {
                screen_rect: Some(screen_rect),
                ..Default::default()
            },
            |ui| app.playlists_ui(ui),
        );
        let position = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == "▶  Play" => {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                _ => None,
            })
            .expect("Play button text should be visible");
        output.textures_delta.clear();
        for pressed in [true, false] {
            let mut output = crate::test_support::run_ui(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(screen_rect),
                    events: vec![
                        egui::Event::PointerMoved(position),
                        egui::Event::PointerButton {
                            pos: position,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                    ..Default::default()
                },
                |ui| app.playlists_ui(ui),
            );
            output.textures_delta.clear();
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        let status = loop {
            let status = app.player.status().unwrap();
            if status
                .current
                .as_ref()
                .is_some_and(|current| current.id == track.id)
                || status.error.is_some()
                || Instant::now() >= deadline
            {
                break status;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        assert!(
            status
                .current
                .as_ref()
                .is_some_and(|current| current.id == track.id)
                || status.error.is_some(),
            "Play click must send the selected playlist track to the audio worker"
        );
        drop(app);
        std::thread::sleep(Duration::from_millis(100));
    }
    #[test]
    fn liquid_slider_keyboard_clamps_and_reports_changes() {
        let ctx = egui::Context::default();
        let mut value = 0.99;
        let mut id = None;
        let mut output = crate::test_support::run_ui(&ctx, egui::RawInput::default(), |ui| {
            let response = liquid_slider(ui, &mut value, 1.0, 180.0, "Volume");
            id = Some(response.id);
            response.request_focus();
        });
        output.textures_delta.clear();
        let mut changed = false;
        let mut output = crate::test_support::run_ui(
            &ctx,
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: egui::Key::ArrowRight,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                ..Default::default()
            },
            |ui| {
                let response = liquid_slider(ui, &mut value, 1.0, 180.0, "Volume");
                assert_eq!(Some(response.id), id);
                changed |= response.changed();
            },
        );
        output.textures_delta.clear();
        assert_eq!(value, 1.0);
        assert!(changed);
    }
    #[test]
    fn liquid_slider_click_seeks_to_rail_position() {
        let ctx = egui::Context::default();
        let mut value = 0.0;
        let mut rect = egui::Rect::NOTHING;
        let mut output = crate::test_support::run_ui(&ctx, egui::RawInput::default(), |ui| {
            rect = liquid_slider(ui, &mut value, 200.0, 200.0, "Seek").rect;
        });
        output.textures_delta.clear();
        let pos = rect.center();
        let mut changed = false;
        for pressed in [true, false] {
            let mut output = crate::test_support::run_ui(
                &ctx,
                egui::RawInput {
                    events: vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                    ..Default::default()
                },
                |ui| {
                    changed |= liquid_slider(ui, &mut value, 200.0, 200.0, "Seek").changed();
                },
            );
            output.textures_delta.clear();
        }
        assert!((value - 100.0).abs() < 0.1);
        assert!(changed);
    }
}
