//! The large control is the selected mode; the small ellipse selects the other.
use crate::{player::PlaybackMode, theme, BubbleSystem, CARD, WHITE};
use eframe::egui::{self, Color32, Pos2, Rect, Vec2};

fn glyph(painter: &egui::Painter, rect: Rect, repeat: bool, color: Color32) {
    let scale = (rect.width() / 36.0).min(rect.height() / 34.0);
    let center = rect.center() - Vec2::new(0.0, scale);
    let point = |x, y| center + Vec2::new(x, y) * scale;
    let stroke = egui::Stroke::new(1.7 * scale.max(0.8), color);
    if repeat {
        for direction in [-1.0, 1.0] {
            painter.add(egui::Shape::line(
                vec![
                    point(-7.0 * direction, 2.0 * direction),
                    point(-7.0 * direction, -3.0 * direction),
                    point(-5.0 * direction, -5.0 * direction),
                    point(7.0 * direction, -5.0 * direction),
                ],
                stroke,
            ));
            painter.add(egui::Shape::line(
                vec![
                    point(4.0 * direction, -8.0 * direction),
                    point(7.0 * direction, -5.0 * direction),
                    point(4.0 * direction, -2.0 * direction),
                ],
                stroke,
            ));
        }
    } else {
        for direction in [-1.0, 1.0] {
            painter.add(egui::Shape::line(
                vec![
                    point(-8.0, -5.0 * direction),
                    point(-4.0, -5.0 * direction),
                    point(4.0, 5.0 * direction),
                    point(8.0, 5.0 * direction),
                ],
                stroke,
            ));
            painter.add(egui::Shape::line(
                vec![
                    point(5.0, 2.0 * direction),
                    point(8.0, 5.0 * direction),
                    point(5.0, 8.0 * direction),
                ],
                stroke,
            ));
        }
    }
}

fn button(ui: &mut egui::Ui, rect: Rect, repeat: bool, selected: bool) -> egui::Response {
    let name = if repeat { "Loop" } else { "Shuffle" };
    let response = ui.interact(rect, ui.id().with(name), egui::Sense::click());
    if response.clicked() {
        response.request_focus();
    }
    BubbleSystem::register_button(ui.ctx(), &response);
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), selected, name)
    });
    let hint = if repeat {
        "Loop — repetă piesa curentă"
    } else {
        "Shuffle — ordine aleatorie"
    };
    response
        .on_hover_text(hint)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn paint(ui: &egui::Ui, response: &egui::Response, repeat: bool, selected: bool, small: bool) {
    let rect = response.rect;
    let accent = theme::accent(ui.ctx());
    let hover = ui.ctx().animate_bool_with_time(
        response.id.with("light"),
        response.hovered() || response.has_focus(),
        0.18,
    );
    let painter = ui.painter();
    if small {
        // The opaque rim separates the small ellipse from the glass underneath.
        painter.add(egui::Shape::ellipse_filled(
            rect.center(),
            rect.size() * 0.5 + Vec2::splat(1.5),
            crate::ABYSS,
        ));
        painter.add(egui::Shape::ellipse_filled(
            rect.center(),
            rect.size() * 0.5,
            theme::tint(ui.ctx(), CARD),
        ));
        painter.add(egui::Shape::ellipse_filled(
            rect.center(),
            rect.size() * 0.5,
            accent.gamma_multiply(if selected { 0.24 } else { 0.08 } + hover * 0.12),
        ));
        painter.add(egui::Shape::ellipse_stroke(
            rect.center(),
            rect.size() * 0.5,
            egui::Stroke::new(
                1.0,
                if selected || response.has_focus() {
                    accent.gamma_multiply(0.8)
                } else {
                    Color32::from_white_alpha(75)
                },
            ),
        ));
    } else {
        crate::gradient_tile(
            painter,
            rect,
            10.0,
            accent.gamma_multiply(if selected { 0.28 } else { 0.08 } + hover * 0.12),
            theme::tint(ui.ctx(), CARD),
        );
        painter.rect_stroke(
            rect,
            10,
            if selected || response.has_focus() {
                egui::Stroke::new(1.0, accent.gamma_multiply(0.65))
            } else {
                crate::glass_edge()
            },
            egui::StrokeKind::Inside,
        );
        crate::specular_edge(painter, rect, 10.0, 1.0);
    }
    glyph(painter, rect, repeat, if selected { accent } else { WHITE });
    if selected && !small {
        painter.circle_filled(rect.center_bottom() - Vec2::new(0.0, 3.5), 1.5, accent);
    }
}

pub(super) fn show(
    ui: &mut egui::Ui,
    mode: PlaybackMode,
    primary: &mut PlaybackMode,
) -> Option<PlaybackMode> {
    if mode != PlaybackMode::Off {
        *primary = mode;
    }
    if *primary == PlaybackMode::Off {
        *primary = PlaybackMode::Shuffle;
    }
    let (bounds, _) = ui.allocate_exact_size(Vec2::new(52.0, 62.0), egui::Sense::hover());
    let main_rect = Rect::from_min_size(
        Pos2::new(bounds.left(), bounds.top() + 14.0),
        Vec2::new(36.0, 34.0),
    );
    let small_rect = Rect::from_min_size(
        Pos2::new(bounds.left() + 25.0, bounds.top()),
        Vec2::new(26.0, 20.0),
    );
    let primary_repeat = *primary == PlaybackMode::Loop;
    // Register the upper control last so only it receives overlapping clicks.
    let selected = mode != PlaybackMode::Off;
    let main = button(ui, main_rect, primary_repeat, selected);
    let small = button(ui, small_rect, !primary_repeat, false);
    paint(ui, &main, primary_repeat, selected, false);
    paint(ui, &small, !primary_repeat, false, true);
    if small.clicked() {
        Some(if primary_repeat {
            PlaybackMode::Shuffle
        } else {
            PlaybackMode::Loop
        })
    } else if main.clicked() {
        Some(if selected {
            PlaybackMode::Off
        } else {
            *primary
        })
    } else {
        None
    }
}
