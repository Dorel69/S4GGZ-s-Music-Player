//! A clipped title marquee never participates in the footer's width calculation.
use eframe::egui::{self, RichText, Vec2};
use std::time::Duration;

const HEIGHT: f32 = 20.0;
const SPEED: f64 = 28.0;
const END_PAUSE: f64 = 1.0;

#[derive(Clone)]
struct Marquee {
    text: String,
    width: f32,
    overflow: f32,
    started_at: f64,
}

struct Motion {
    offset: f32,
    repaint_after: Duration,
}

fn motion(elapsed: f64, overflow: f32) -> Motion {
    let travel = f64::from(overflow) / SPEED;
    let phase = elapsed.max(0.0) % (2.0 * (END_PAUSE + travel));
    let (offset, delay) = if phase < END_PAUSE {
        (0.0, END_PAUSE - phase)
    } else if phase < END_PAUSE + travel {
        ((phase - END_PAUSE) * SPEED, 0.016)
    } else if phase < 2.0 * END_PAUSE + travel {
        (f64::from(overflow), 2.0 * END_PAUSE + travel - phase)
    } else {
        (
            f64::from(overflow) - (phase - 2.0 * END_PAUSE - travel) * SPEED,
            0.016,
        )
    };
    Motion {
        offset: (offset as f32).clamp(0.0, overflow),
        repaint_after: Duration::from_secs_f64(delay.max(0.001)),
    }
}

pub(super) fn show(ui: &mut egui::Ui, id: egui::Id, text: &str, width: f32) -> egui::Response {
    let width = if width.is_finite() {
        width.max(0.0)
    } else {
        0.0
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, HEIGHT), egui::Sense::hover());
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, ui.is_enabled(), text));
    if ui.is_rect_visible(rect) && width > 0.0 {
        let color = ui.visuals().strong_text_color();
        let galley = egui::WidgetText::from(RichText::new(text).strong().size(14.0).color(color))
            .into_galley(
                ui,
                Some(egui::TextWrapMode::Extend),
                f32::INFINITY,
                egui::FontSelection::Default,
            );
        let overflow = (galley.size().x - width).max(0.0);
        let state_id = id.with("footer-title-marquee");
        let offset = if overflow > 0.5 {
            let now = ui.input(|input| input.time);
            let started_at = ui.ctx().data_mut(|data| {
                let mut state = data.get_temp::<Marquee>(state_id);
                if state.as_ref().is_none_or(|state| {
                    state.text != text
                        || (state.width - width).abs() > 0.5
                        || (state.overflow - overflow).abs() > 0.5
                        || now < state.started_at
                }) {
                    state = Some(Marquee {
                        text: text.to_owned(),
                        width,
                        overflow,
                        started_at: now,
                    });
                }
                let state = state.unwrap();
                let started_at = state.started_at;
                data.insert_temp(state_id, state);
                started_at
            });
            let movement = motion(now - started_at, overflow);
            ui.ctx().request_repaint_after(movement.repaint_after);
            movement.offset
        } else {
            ui.ctx().data_mut(|data| data.remove::<Marquee>(state_id));
            0.0
        };
        // Painting the complete galley retains a clean single line; only this
        // title's clip moves through it, while the artist and transport stay put.
        let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
        painter.galley(
            egui::pos2(
                rect.left() - offset,
                rect.center().y - galley.size().y * 0.5,
            ),
            galley,
            color,
        );
    }
    response.on_hover_text(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::Rect;

    #[test]
    fn marquee_reads_both_ends_and_pauses_for_one_second_before_restarting() {
        // A 56-point overflow travels for two seconds at the reading speed.
        for (elapsed, expected) in [
            (0.0, 0.0),
            (0.75, 0.0),
            (1.0, 0.0),
            (2.0, 28.0),
            (3.0, 56.0),
            (3.75, 56.0),
            (4.0, 56.0),
            (5.0, 28.0),
            (6.0, 0.0),
            (6.75, 0.0),
            (7.0, 0.0),
            (8.0, 28.0),
        ] {
            assert!(
                (motion(elapsed, 56.0).offset - expected).abs() < 0.001,
                "at {elapsed}s"
            );
        }
        assert_eq!(motion(0.0, 56.0).repaint_after, Duration::from_secs(1));
        assert_eq!(motion(3.0, 56.0).repaint_after, Duration::from_secs(1));
        assert!(motion(2.0, 56.0).repaint_after <= Duration::from_millis(17));
    }

    fn render(
        ctx: &egui::Context,
        time: f64,
        text: &str,
        width: f32,
    ) -> (egui::FullOutput, Rect, Rect) {
        let mut title = Rect::NOTHING;
        let mut next = Rect::NOTHING;
        let output = crate::test_support::run_ui(
            ctx,
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(500.0, 100.0),
                )),
                time: Some(time),
                ..Default::default()
            },
            |ui| {
                ui.horizontal(|ui| {
                    title = show(ui, egui::Id::new("test-title"), text, width).rect;
                    next = ui.label("Transport").rect;
                });
            },
        );
        (output, title, next)
    }

    fn painted_title(output: &egui::FullOutput, title: &str) -> (egui::Pos2, Rect) {
        output
            .shapes
            .iter()
            .find_map(|entry| {
                if let egui::Shape::Text(text) = &entry.shape {
                    if text.galley.text() == title {
                        return Some((text.pos, entry.clip_rect));
                    }
                }
                None
            })
            .expect("The full title must be painted")
    }

    #[test]
    fn long_titles_are_clipped_without_moving_neighboring_footer_controls_and_reset_cleanly() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let long = "A very long song title that cannot fit into this small footer slot";
        let (first, slot, next) = render(&ctx, 1.0, long, 100.0);
        let (start, clip) = painted_title(&first, long);
        assert_eq!(slot.width(), 100.0);
        assert_eq!(slot.height(), HEIGHT);
        assert_eq!(start.x, slot.left());
        assert_eq!(clip.x_range(), slot.x_range());
        let (moving, moving_slot, moving_next) = render(&ctx, 2.5, long, 100.0);
        assert_eq!(moving_slot, slot);
        assert_eq!(moving_next, next);
        assert!(painted_title(&moving, long).0.x < slot.left());
        let renamed = "A different long title must restart from the beginning";
        let (changed, changed_slot, _) = render(&ctx, 3.0, renamed, 100.0);
        assert_eq!(painted_title(&changed, renamed).0.x, changed_slot.left());
        let (resized, resized_slot, _) = render(&ctx, 5.0, renamed, 120.0);
        assert_eq!(painted_title(&resized, renamed).0.x, resized_slot.left());
        let (short, short_slot, short_next) = render(&ctx, 10.0, "Song", 100.0);
        assert_eq!(short_slot, slot);
        assert_eq!(short_next, next);
        assert_eq!(painted_title(&short, "Song").0.x, short_slot.left());
        assert!(short
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .any(|(_, node)| node.value() == Some("Song")));
    }
}
