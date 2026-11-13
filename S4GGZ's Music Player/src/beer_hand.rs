use crate::settings::BeerFrequency;
use eframe::egui::{self, Color32, Pos2, Rect, TextureHandle, Vec2};
use std::{path::PathBuf, time::Duration};

const ENTER: f32 = 0.7;
const SHAKE: f32 = 0.9;
const EXIT: f32 = 0.4;
const TOTAL: f32 = ENTER + SHAKE + EXIT;
const MAX_ANGLE: f32 = 18.0 * std::f32::consts::PI / 180.0;
// The arm meets the outer PNG edge near its lower end, at elbow height.
const ELBOW_Y: f32 = 0.86;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    Left,
    Right,
}

#[derive(Clone, Copy)]
struct Visit {
    side: Side,
    started: f64,
    height: f32,
}

#[derive(Clone, Copy, Debug)]
struct Pose {
    pivot: Pos2,
    anchor: Vec2,
    size: Vec2,
    angle: f32,
}

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Reserve vertical clearance around the elbow. The cropped forearm edge stays
/// beyond the window boundary for the whole swing, without stretching the PNG.
fn pose(bounds: Rect, aspect: f32, visit: Visit, now: f64) -> Option<Pose> {
    if bounds.width() < 80.0 || bounds.height() < 80.0 || !aspect.is_finite() || aspect <= 0.0 {
        return None;
    }
    let age = (now - visit.started) as f32;
    if !(0.0..TOTAL).contains(&age) {
        return None;
    }
    let swing = MAX_ANGLE.sin();
    let margin = 6.0;
    let scale = (bounds.width() * 0.35 / aspect)
        .min(bounds.height() * 0.36)
        .min(240.0)
        .min((bounds.height() - margin * 2.0) / (1.0 + 2.0 * aspect * swing));
    let size = Vec2::new(aspect, 1.0) * scale;
    let above = size.y * ELBOW_Y + size.x * swing;
    let below = size.y * (1.0 - ELBOW_Y) + size.x * swing;
    let y = egui::lerp(
        bounds.top() + above + margin..=bounds.bottom() - below - margin,
        visit.height,
    );
    let outward = size.y * ELBOW_Y * swing + margin;
    let hidden = size.x + outward;
    let (outside, inside, anchor_x) = match visit.side {
        Side::Left => (bounds.left() - hidden, bounds.left() - outward, 0.0),
        Side::Right => (bounds.right() + hidden, bounds.right() + outward, 1.0),
    };
    let x = if age < ENTER {
        egui::lerp(outside..=inside, smooth(age / ENTER))
    } else if age < ENTER + SHAKE {
        inside
    } else {
        egui::lerp(inside..=outside, smooth((age - ENTER - SHAKE) / EXIT))
    };
    let shake = ((age - ENTER) / SHAKE).clamp(0.0, 1.0);
    // Smoothly begin/end the shake: no angle jump at either phase boundary.
    let envelope = (std::f32::consts::PI * shake).sin().powi(2);
    let direction = if visit.side == Side::Left { -1.0 } else { 1.0 };
    let wave = (shake * std::f32::consts::TAU * 2.0).sin() * envelope;
    Some(Pose {
        pivot: egui::pos2(x, y),
        anchor: Vec2::new(anchor_x, ELBOW_Y),
        size,
        angle: direction * wave * MAX_ANGLE,
    })
}

fn vertices(pose: Pose) -> [Pos2; 4] {
    let rotation = egui::emath::Rot2::from_angle(pose.angle);
    [
        Vec2::new(0.0, 0.0),
        Vec2::new(1.0, 0.0),
        Vec2::new(1.0, 1.0),
        Vec2::new(0.0, 1.0),
    ]
    .map(|corner| pose.pivot + rotation * ((corner - pose.anchor) * pose.size))
}

pub struct BeerHandAnimation {
    texture: Option<TextureHandle>,
    visits: Vec<Visit>,
    next_visit: Option<f64>,
    frequency: BeerFrequency,
    random: u64,
}

impl BeerHandAnimation {
    pub fn new(ctx: &egui::Context, directories: &[PathBuf]) -> Self {
        let texture = directories.iter().find_map(|directory| {
            let image = image::open(directory.join("beer-hand.png"))
                .ok()?
                .to_rgba8();
            Some(ctx.load_texture(
                "beer-hand",
                egui::ColorImage::from_rgba_unmultiplied(
                    [image.width() as usize, image.height() as usize],
                    image.as_raw(),
                ),
                egui::TextureOptions::LINEAR,
            ))
        });
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1, |time| time.as_nanos() as u64)
            .max(1);
        Self {
            texture,
            visits: Vec::new(),
            next_visit: None,
            frequency: BeerFrequency::default(),
            random: seed,
        }
    }

    pub fn pause(&mut self) {
        self.visits.clear();
        self.next_visit = None;
    }

    pub fn set_frequency(&mut self, frequency: BeerFrequency) {
        if self.frequency != frequency {
            self.frequency = frequency;
            self.pause();
        }
    }

    fn random_unit(&mut self) -> f64 {
        self.random ^= self.random << 13;
        self.random ^= self.random >> 7;
        self.random ^= self.random << 17;
        (self.random >> 11) as f64 / ((1_u64 << 53) - 1) as f64
    }

    fn interval(&mut self) -> f64 {
        let (min, max) = self.frequency.interval();
        min + self.random_unit() * (max - min)
    }

    fn tick(&mut self, now: f64) {
        self.visits
            .retain(|visit| now - visit.started < f64::from(TOTAL));
        let Some(next) = self.next_visit else {
            self.next_visit = Some(now + self.interval());
            return;
        };
        if now < next {
            return;
        }
        let side = if self.random_unit() < 0.5 {
            Side::Left
        } else {
            Side::Right
        };
        let height = self.random_unit() as f32;
        self.visits.push(Visit {
            side,
            started: now,
            height,
        });
        if self.frequency == BeerFrequency::Frequent && self.random_unit() < 0.4 {
            let height = self.random_unit() as f32;
            self.visits.push(Visit {
                side: if side == Side::Left {
                    Side::Right
                } else {
                    Side::Left
                },
                started: now,
                height,
            });
        }
        // All intervals exceed the two-second visit, so separate visits never
        // overlap. Only the deliberate frequent-mode pair shares a start time.
        self.next_visit = Some(now + self.interval());
    }

    pub fn paint(&mut self, ui: &egui::Ui) {
        if self.texture.is_none() {
            return;
        }
        let now = ui.input(|input| input.time);
        self.tick(now);
        let texture = self.texture.as_ref().unwrap();
        let bounds = ui.max_rect();
        // Called just after the background, before any glass panels or controls.
        let painter = ui.painter().with_clip_rect(bounds);
        for visit in &self.visits {
            let Some(pose) = pose(bounds, texture.aspect_ratio(), *visit, now) else {
                continue;
            };
            // The supplied arm enters from the right; mirror it for the left edge.
            let (left, right) = if visit.side == Side::Left {
                (1.0, 0.0)
            } else {
                (0.0, 1.0)
            };
            let uvs = [
                egui::pos2(left, 0.0),
                egui::pos2(right, 0.0),
                egui::pos2(right, 1.0),
                egui::pos2(left, 1.0),
            ];
            let mut mesh = egui::Mesh::with_texture(texture.id());
            for (pos, uv) in vertices(pose).into_iter().zip(uvs) {
                mesh.vertices.push(egui::epaint::Vertex {
                    pos,
                    uv,
                    color: Color32::WHITE,
                });
            }
            mesh.add_triangle(0, 1, 2);
            mesh.add_triangle(0, 2, 3);
            painter.add(egui::Shape::mesh(mesh));
        }
        if self.visits.is_empty() {
            let wait = (self.next_visit.unwrap_or(now) - now).max(0.001);
            ui.ctx()
                .request_repaint_after(Duration::from_secs_f64(wait));
        } else {
            ui.ctx().request_repaint_after(Duration::from_millis(16));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elbow_stays_fixed_on_outer_png_edge_while_mug_moves_up_and_down() {
        let bounds = Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 800.0));
        for side in [Side::Left, Side::Right] {
            let visit = Visit {
                side,
                started: 0.0,
                height: 0.5,
            };
            let rest = pose(bounds, 642.0 / 350.0, visit, f64::from(ENTER)).unwrap();
            let outer = if side == Side::Left { (0, 3) } else { (1, 2) };
            let mug = if side == Side::Left { 1 } else { 0 };
            let baseline = vertices(rest)[mug].y;
            let mut raised = false;
            let mut lowered = false;
            for frame in 0..=90 {
                let pose = pose(
                    bounds,
                    642.0 / 350.0,
                    visit,
                    f64::from(ENTER) + frame as f64 / 100.0,
                )
                .unwrap();
                assert!(pose.pivot.distance(rest.pivot) < 0.001);
                let corners = vertices(pose);
                let elbow = corners[outer.0].lerp(corners[outer.1], ELBOW_Y);
                assert!(elbow.distance(rest.pivot) < 0.001);
                raised |= corners[mug].y < baseline - 20.0;
                lowered |= corners[mug].y > baseline + 20.0;
            }
            assert!(raised && lowered);
        }
    }

    #[test]
    fn hand_is_painted_before_glass_panel_and_menu_text() {
        let ctx = egui::Context::default();
        let texture = ctx.load_texture(
            "test-hand-layer",
            egui::ColorImage::new([1, 1], vec![Color32::WHITE]),
            egui::TextureOptions::LINEAR,
        );
        let texture_id = texture.id();
        let mut animation = BeerHandAnimation::new(&ctx, &[]);
        animation.texture = Some(texture);
        animation.visits.push(Visit {
            side: Side::Right,
            started: 0.0,
            height: 0.5,
        });
        animation.next_visit = Some(20.0);
        let glass = Color32::from_rgba_unmultiplied(40, 30, 10, 180);
        let output = crate::test_support::run_ui(
            &ctx,
            egui::RawInput {
                time: Some(1.0),
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 800.0))),
                ..Default::default()
            },
            |ui| {
                animation.paint(ui);
                egui::Frame::new().fill(glass).show(ui, |ui| {
                    ui.label("Menu above hand");
                });
            },
        );
        let hand = output
            .shapes
            .iter()
            .position(|shape| {
                matches!(&shape.shape,
            egui::Shape::Mesh(mesh) if mesh.texture_id == texture_id)
            })
            .unwrap();
        if let egui::Shape::Mesh(mesh) = &output.shapes[hand].shape {
            // A single unwarped texture quad: no repeated edge pixels or
            // additional geometry to invent missing parts of the forearm.
            assert_eq!(mesh.vertices.len(), 4);
            assert_eq!(mesh.indices.len(), 6);
        }
        let panel = output
            .shapes
            .iter()
            .position(|shape| {
                matches!(&shape.shape,
            egui::Shape::Rect(rect) if rect.fill == glass)
            })
            .unwrap();
        let text = output
            .shapes
            .iter()
            .position(|shape| {
                matches!(&shape.shape,
            egui::Shape::Text(text) if text.galley.text() == "Menu above hand")
            })
            .unwrap();
        assert!(hand < panel && panel < text);
    }

    #[test]
    fn shake_hides_cropped_edge_and_keeps_vertical_clearance_without_distortion() {
        for size in [
            Vec2::new(1280.0, 800.0),
            Vec2::new(800.0, 600.0),
            Vec2::new(300.0, 900.0),
            Vec2::new(900.0, 180.0),
        ] {
            let bounds = Rect::from_min_size(egui::pos2(17.0, 31.0), size);
            for side in [Side::Left, Side::Right] {
                for height in [0.0, 0.5, 1.0] {
                    let visit = Visit {
                        side,
                        height,
                        started: 0.0,
                    };
                    for frame in 0..=90 {
                        let pose = pose(
                            bounds,
                            642.0 / 350.0,
                            visit,
                            f64::from(ENTER) + frame as f64 / 100.0,
                        )
                        .unwrap();
                        let corners = vertices(pose);
                        for corner in corners {
                            assert!(
                                corner.y >= bounds.top() && corner.y <= bounds.bottom(),
                                "{bounds:?}: {corner:?}"
                            );
                        }
                        let outer = if side == Side::Left { [0, 3] } else { [1, 2] };
                        for index in outer {
                            if side == Side::Left {
                                assert!(corners[index].x < bounds.left());
                            } else {
                                assert!(corners[index].x > bounds.right());
                            }
                        }
                        let horizontal = corners[1] - corners[0];
                        let vertical = corners[3] - corners[0];
                        assert!(
                            (horizontal.length() / vertical.length() - 642.0 / 350.0).abs()
                                < 0.0001
                        );
                        assert!(
                            (horizontal.normalized().dot(vertical.normalized())).abs() < 0.0001
                        );
                        // The mug occupies the central part of the supplied PNG;
                        // its top and bottom remain visible throughout the shake.
                        let rotation = egui::emath::Rot2::from_angle(pose.angle);
                        for x in [0.38, 0.6] {
                            for y in [0.05, 0.95] {
                                let x = if side == Side::Left { 1.0 - x } else { x };
                                let point = pose.pivot
                                    + rotation * ((Vec2::new(x, y) - pose.anchor) * pose.size);
                                assert!(
                                    bounds.contains(point),
                                    "Mug clipped: {bounds:?}: {point:?}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn slide_and_retraction_use_same_edge_and_shake_starts_and_stops_at_rest() {
        let bounds = Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 800.0));
        for side in [Side::Left, Side::Right] {
            let visit = Visit {
                side,
                height: 0.3,
                started: 0.0,
            };
            let start = pose(bounds, 1.8, visit, 0.0).unwrap();
            let hold = pose(bounds, 1.8, visit, f64::from(ENTER)).unwrap();
            let finish = pose(bounds, 1.8, visit, f64::from(ENTER + SHAKE)).unwrap();
            let end = pose(bounds, 1.8, visit, f64::from(TOTAL) - 0.00001).unwrap();
            assert!(start.angle.abs() < 0.0001 && finish.angle.abs() < 0.0001);
            assert!((hold.pivot.x - finish.pivot.x).abs() < 0.001);
            assert!((start.pivot.x - end.pivot.x).abs() < 0.01);
            assert!(pose(bounds, 1.8, visit, f64::from(TOTAL)).is_none());
        }
    }

    #[test]
    fn frequency_scheduler_sleeps_resets_and_only_frequent_can_spawn_both_sides() {
        let ctx = egui::Context::default();
        for frequency in [
            BeerFrequency::Rare,
            BeerFrequency::Medium,
            BeerFrequency::Frequent,
        ] {
            let mut animation = BeerHandAnimation::new(&ctx, &[]);
            animation.random = 1;
            animation.set_frequency(frequency);
            animation.tick(0.0);
            let (min, max) = frequency.interval();
            assert!((min..=max).contains(&animation.next_visit.unwrap()));
            let mut double_seen = false;
            for _ in 0..100 {
                let next = animation.next_visit.unwrap();
                animation.tick(next);
                assert!((1..=2).contains(&animation.visits.len()));
                if animation.visits.len() == 2 {
                    assert_eq!(frequency, BeerFrequency::Frequent);
                    assert_ne!(animation.visits[0].side, animation.visits[1].side);
                    double_seen = true;
                }
                let interval = animation.next_visit.unwrap() - next;
                assert!(interval >= min - 0.00001 && interval <= max + 0.00001);
            }
            assert_eq!(double_seen, frequency == BeerFrequency::Frequent);
            animation.pause();
            assert!(animation.visits.is_empty() && animation.next_visit.is_none());
        }
    }
}
