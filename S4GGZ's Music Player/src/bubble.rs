use crate::settings::EffectDensity;
#[cfg(test)]
use eframe::egui::Vec2;
use eframe::egui::{self, Color32, TextureHandle};
use std::path::PathBuf;

const DURATION: f32 = 3.0;
const GROWTH_END: f32 = 1.2;
const MIN_BUBBLES: usize = 20;
const MAX_BUBBLES: usize = 60;

#[derive(Clone)]
struct Bubble {
    origin: egui::Pos2,
    started_at: f64,
    max_diameter: f32,
    initial_diameter: f32,
    peak_at: f32,
    rise_distance: f32,
    wave_amplitude: f32,
    wave_frequency: f32,
    phase: f32,
    wave_origin: f32,
}

#[derive(Clone, Default)]
pub struct BubbleSystem {
    disabled: bool,
    density: EffectDensity,
    frame_hovered: Option<(egui::Id, egui::Rect)>,
    frame_clicked: Option<(egui::Id, egui::Rect)>,
    last_hovered: Option<egui::Id>,
    bubbles: Vec<Bubble>,
    random: u64,
}

impl BubbleSystem {
    fn id() -> egui::Id {
        egui::Id::new("aura-bubble-system")
    }

    pub fn load_texture(
        ctx: &egui::Context,
        directories: &[PathBuf],
        file: &str,
    ) -> Option<TextureHandle> {
        directories.iter().find_map(|directory| {
            let image = image::open(directory.join(file)).ok()?;
            let image = image.to_rgba8();
            Some(ctx.load_texture(
                "button-bubble",
                egui::ColorImage::from_rgba_unmultiplied(
                    [image.width() as usize, image.height() as usize],
                    image.as_raw(),
                ),
                egui::TextureOptions::LINEAR,
            ))
        })
    }

    pub fn begin_frame(ctx: &egui::Context) {
        ctx.data_mut(|data| {
            let system = data.get_temp_mut_or_default::<Self>(Self::id());
            system.frame_hovered = None;
            system.frame_clicked = None;
        });
    }

    pub fn reset(ctx: &egui::Context) {
        ctx.data_mut(|data| {
            let system = data.get_temp_mut_or_default::<Self>(Self::id());
            system.frame_hovered = None;
            system.frame_clicked = None;
            system.last_hovered = None;
            system.bubbles.clear();
        });
    }

    pub fn set_enabled(ctx: &egui::Context, enabled: bool) {
        ctx.data_mut(|data| {
            let system = data.get_temp_mut_or_default::<Self>(Self::id());
            system.disabled = !enabled;
            if !enabled {
                system.frame_hovered = None;
                system.frame_clicked = None;
                system.last_hovered = None;
                system.bubbles.clear();
            }
        });
    }

    pub fn set_density(ctx: &egui::Context, density: EffectDensity) {
        ctx.data_mut(|data| {
            let system = data.get_temp_mut_or_default::<Self>(Self::id());
            if system.density != density {
                system.density = density;
                system.frame_hovered = None;
                system.frame_clicked = None;
                system.last_hovered = None;
                system.bubbles.clear();
            }
        });
    }

    pub fn register_button(ctx: &egui::Context, response: &egui::Response) {
        if !response.hovered() {
            return;
        }
        // Response input queries must happen before taking the context data lock.
        let clicked = response.clicked_by(egui::PointerButton::Primary);
        ctx.data_mut(|data| {
            let system = data.get_temp_mut_or_default::<Self>(Self::id());
            if !system.disabled {
                system.frame_hovered = Some((response.id, response.rect));
                if clicked {
                    system.frame_clicked = Some((response.id, response.rect));
                }
            }
        });
    }

    fn random_unit(&mut self) -> f32 {
        if self.random == 0 {
            self.random = 0x7A31_95D2_4B6C_8E0F;
        }
        self.random ^= self.random << 13;
        self.random ^= self.random >> 7;
        self.random ^= self.random << 17;
        (self.random >> 40) as f32 / 16_777_215.0
    }

    fn random_between(&mut self, minimum: f32, maximum: f32) -> f32 {
        minimum + self.random_unit() * (maximum - minimum)
    }

    fn bubble_count(button_width: f32) -> usize {
        let progress = ((button_width - 48.0) / (480.0 - 48.0)).clamp(0.0, 1.0);
        MIN_BUBBLES + ((MAX_BUBBLES - MIN_BUBBLES) as f32 * progress).round() as usize
    }

    fn density_count(&self, button_width: f32) -> usize {
        match self.density {
            EffectDensity::Many => Self::bubble_count(button_width),
            EffectDensity::Medium => Self::bubble_count(button_width).div_ceil(2),
            EffectDensity::Few => {
                let progress = ((button_width - 48.0) / (480.0 - 48.0)).clamp(0.0, 1.0);
                5 + (5.0 * progress).round() as usize
            }
        }
    }

    fn spawn(&mut self, rect: egui::Rect, now: f64) {
        let count = self.density_count(rect.width());
        let corners_only = self.density == EffectDensity::Few;
        let max_diameter = rect.width() * 0.05;
        let wave_amplitude = (rect.width() * 0.16).min(42.0);
        for index in 0..count {
            let origin = if corners_only {
                if index % 2 == 0 {
                    rect.left_top()
                } else {
                    rect.right_top()
                }
            } else {
                egui::pos2(self.random_between(rect.left(), rect.right()), rect.top())
            };
            let maximum_size = max_diameter * self.random_between(0.55, 1.0);
            let initial_size = max_diameter * self.random_between(0.06, 0.18);
            let peak_at = self.random_between(0.78, GROWTH_END);
            let rise_distance = self.random_between(
                (rect.height() * 1.5).max(38.0),
                (rect.height() * 3.2).max(72.0),
            );
            let wave_amplitude = if corners_only {
                self.random_between(1.0, 4.0)
            } else {
                self.random_between(4.0, wave_amplitude.max(4.0))
            };
            let wave_frequency = self.random_between(2.0, 5.5);
            let phase = self.random_between(0.0, std::f32::consts::TAU);
            self.bubbles.push(Bubble {
                origin,
                started_at: now,
                max_diameter: maximum_size,
                initial_diameter: initial_size,
                peak_at,
                rise_distance,
                wave_amplitude,
                wave_frequency,
                phase,
                wave_origin: if corners_only { phase.sin() } else { 0.0 },
            });
        }
    }

    fn smoothstep(value: f32) -> f32 {
        let value = value.clamp(0.0, 1.0);
        value * value * (3.0 - 2.0 * value)
    }

    fn paint_bubble(painter: &egui::Painter, texture: &TextureHandle, bubble: &Bubble, now: f64) {
        let age = (now - bubble.started_at).clamp(0.0, DURATION as f64) as f32;
        let grow_progress = Self::smoothstep(age / bubble.peak_at);
        let (diameter, opacity) = if age <= bubble.peak_at {
            (
                egui::lerp(bubble.initial_diameter..=bubble.max_diameter, grow_progress),
                Self::smoothstep(age / 0.12),
            )
        } else {
            let shrink_progress =
                Self::smoothstep((age - bubble.peak_at) / (DURATION - bubble.peak_at));
            (
                bubble.max_diameter * (1.0 - shrink_progress),
                1.0 - shrink_progress,
            )
        };
        let pulse = 0.84 + 0.16 * ((age * bubble.wave_frequency + bubble.phase).sin() + 1.0) * 0.5;
        let diameter = (diameter * pulse).clamp(0.0, bubble.max_diameter);
        let progress = age / DURATION;
        let center = egui::pos2(
            bubble.origin.x
                + ((age * bubble.wave_frequency + bubble.phase).sin() - bubble.wave_origin)
                    * bubble.wave_amplitude,
            bubble.origin.y - bubble.rise_distance * progress,
        );
        let alpha = (opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
        painter.image(
            texture.id(),
            egui::Rect::from_center_size(
                center,
                texture.size_vec2() / texture.size_vec2().max_elem() * diameter,
            ),
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            Color32::from_white_alpha(alpha),
        );
    }

    pub fn paint(ctx: &egui::Context, texture: &TextureHandle) {
        let now = ctx.input(|input| input.time);
        let bubbles = ctx.data_mut(|data| {
            let system = data.get_temp_mut_or_default::<Self>(Self::id());
            if system.disabled {
                return Vec::new();
            }
            if system.density == EffectDensity::Few {
                if let Some((_, rect)) = system.frame_clicked.take() {
                    system.spawn(rect, now);
                }
                system.last_hovered = None;
            } else if let Some((hovered_id, rect)) = system.frame_hovered {
                if system.last_hovered != Some(hovered_id) {
                    system.spawn(rect, now);
                }
                system.last_hovered = Some(hovered_id);
            } else {
                system.last_hovered = None;
            }
            system
                .bubbles
                .retain(|bubble| now - bubble.started_at < DURATION as f64);
            system.bubbles.clone()
        });

        if bubbles.is_empty() {
            return;
        }

        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("aura-bubble-layer"),
        ));
        for bubble in &bubbles {
            Self::paint_bubble(&painter, texture, bubble, now);
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_mouse_click_registration_finishes_without_reentering_context_lock() {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let ctx = egui::Context::default();
            BubbleSystem::set_density(&ctx, EffectDensity::Few);
            let mut rect = egui::Rect::NOTHING;
            let input = |time, events| egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(320.0, 160.0),
                )),
                time: Some(time),
                events,
                ..Default::default()
            };
            let _ = crate::test_support::run_ui(&ctx, input(0.0, vec![]), |ui| {
                let response = ui.button("Click test");
                rect = response.rect;
                BubbleSystem::register_button(&ctx, &response);
            });
            let pos = rect.center();
            let mut clicked = false;
            for (time, pressed) in [(0.1, true), (0.2, false)] {
                BubbleSystem::begin_frame(&ctx);
                let _ = crate::test_support::run_ui(
                    &ctx,
                    input(
                        time,
                        vec![
                            egui::Event::PointerMoved(pos),
                            egui::Event::PointerButton {
                                pos,
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: egui::Modifiers::NONE,
                            },
                        ],
                    ),
                    |ui| {
                        let response = ui.button("Click test");
                        clicked |= response.clicked_by(egui::PointerButton::Primary);
                        BubbleSystem::register_button(&ctx, &response);
                    },
                );
            }
            let registered = ctx.data_mut(|data| {
                data.get_temp_mut_or_default::<BubbleSystem>(BubbleSystem::id())
                    .frame_clicked
                    .is_some()
            });
            let _ = tx.send((clicked, registered));
        });
        let (clicked, registered) = rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("Mouse click processing blocked on the egui context lock");
        assert!(clicked);
        assert!(registered);
    }

    #[test]
    fn density_preserves_full_effect_halves_medium_and_limits_few_to_five_through_ten() {
        let mut system = BubbleSystem::default();
        for width in [40.0, 100.0, 220.0, 320.0, 600.0] {
            system.density = EffectDensity::Many;
            let full = system.density_count(width);
            assert_eq!(full, BubbleSystem::bubble_count(width));
            system.density = EffectDensity::Medium;
            assert_eq!(system.density_count(width), full.div_ceil(2));
            system.density = EffectDensity::Few;
            assert!((5..=10).contains(&system.density_count(width)));
        }
    }

    #[test]
    fn few_bubbles_spawn_only_at_both_top_corners() {
        let rect = egui::Rect::from_min_size(egui::pos2(25.0, 40.0), Vec2::new(220.0, 40.0));
        let mut system = BubbleSystem {
            density: EffectDensity::Few,
            ..Default::default()
        };
        system.spawn(rect, 0.0);
        assert!((5..=10).contains(&system.bubbles.len()));
        assert!(system
            .bubbles
            .iter()
            .all(|bubble| bubble.origin == rect.left_top() || bubble.origin == rect.right_top()));
        assert!(system
            .bubbles
            .iter()
            .any(|bubble| bubble.origin == rect.left_top()));
        assert!(system
            .bubbles
            .iter()
            .any(|bubble| bubble.origin == rect.right_top()));
        assert!(system
            .bubbles
            .iter()
            .all(|bubble| bubble.wave_origin == bubble.phase.sin()));
    }

    #[test]
    fn few_bubbles_ignore_hover_and_respond_to_each_separate_click() {
        let ctx = egui::Context::default();
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(220.0, 40.0));
        let id = egui::Id::new("click-button");
        let texture = ctx.load_texture(
            "click-bubble-test",
            egui::ColorImage::new([1, 1], vec![Color32::WHITE]),
            egui::TextureOptions::LINEAR,
        );
        BubbleSystem::set_density(&ctx, EffectDensity::Few);
        ctx.data_mut(|data| {
            data.get_temp_mut_or_default::<BubbleSystem>(BubbleSystem::id())
                .frame_hovered = Some((id, rect))
        });
        BubbleSystem::paint(&ctx, &texture);
        let count = || {
            ctx.data_mut(|data| {
                data.get_temp_mut_or_default::<BubbleSystem>(BubbleSystem::id())
                    .bubbles
                    .len()
            })
        };
        assert_eq!(count(), 0);
        ctx.data_mut(|data| {
            data.get_temp_mut_or_default::<BubbleSystem>(BubbleSystem::id())
                .frame_clicked = Some((id, rect))
        });
        BubbleSystem::paint(&ctx, &texture);
        let first = count();
        assert!((5..=10).contains(&first));
        BubbleSystem::paint(&ctx, &texture);
        assert_eq!(count(), first);
        BubbleSystem::begin_frame(&ctx);
        ctx.data_mut(|data| {
            data.get_temp_mut_or_default::<BubbleSystem>(BubbleSystem::id())
                .frame_clicked = Some((id, rect))
        });
        BubbleSystem::paint(&ctx, &texture);
        assert_eq!(count(), first * 2);
        BubbleSystem::set_density(&ctx, EffectDensity::Medium);
        assert_eq!(count(), 0);
    }

    #[test]
    fn disabling_bubbles_clears_particles_and_blocks_spawning_until_reenabled() {
        let ctx = egui::Context::default();
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(180.0, 35.0));
        let texture = ctx.load_texture(
            "bubble-test",
            egui::ColorImage::new([1, 1], vec![Color32::WHITE]),
            egui::TextureOptions::LINEAR,
        );
        ctx.data_mut(|data| {
            let system = data.get_temp_mut_or_default::<BubbleSystem>(BubbleSystem::id());
            system.spawn(rect, 0.0);
            system.last_hovered = Some(egui::Id::new("button"));
        });
        BubbleSystem::set_enabled(&ctx, false);
        ctx.data_mut(|data| {
            let system = data.get_temp_mut_or_default::<BubbleSystem>(BubbleSystem::id());
            assert!(system.bubbles.is_empty());
            assert!(system.last_hovered.is_none());
            system.frame_hovered = Some((egui::Id::new("button"), rect));
        });
        BubbleSystem::paint(&ctx, &texture);
        assert!(ctx.data_mut(|data| data
            .get_temp_mut_or_default::<BubbleSystem>(BubbleSystem::id())
            .bubbles
            .is_empty()));
        BubbleSystem::set_enabled(&ctx, true);
        BubbleSystem::paint(&ctx, &texture);
        assert!(!ctx.data_mut(|data| data
            .get_temp_mut_or_default::<BubbleSystem>(BubbleSystem::id())
            .bubbles
            .is_empty()));
    }

    #[test]
    fn bubble_count_scales_with_button_width() {
        assert_eq!(BubbleSystem::bubble_count(40.0), MIN_BUBBLES);
        assert!(BubbleSystem::bubble_count(220.0) > MIN_BUBBLES);
        assert_eq!(BubbleSystem::bubble_count(600.0), MAX_BUBBLES);
    }

    #[test]
    fn bubble_sizes_stay_within_five_percent_of_button_diagonal() {
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(220.0, 40.0));
        let mut system = BubbleSystem::default();
        system.spawn(rect, 0.0);
        assert!(system.bubbles.iter().all(|bubble| {
            bubble.max_diameter <= rect.width() * 0.05
                && bubble.initial_diameter < bubble.max_diameter
                && bubble.peak_at <= GROWTH_END
        }));
    }
}
