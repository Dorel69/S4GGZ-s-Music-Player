//! Native, audio-reactive disco scene. All geometry shares egui's existing pass.
use crate::{
    audio_analysis::AudioReactive,
    disco_floor::DiscoFloor,
    disco_optics::{self, HitKind, Panel as GlassPanel, Vec3},
    settings::DiscoMode,
};
use eframe::egui::{self, Color32, Pos2, Rect, Shape, Vec2};
use std::{
    f32::consts::{PI, TAU},
    path::PathBuf,
    time::Duration,
};

const LONGITUDES: usize = 32;
const LATITUDES: usize = 16;
// The longer suspension keeps the ball lower, with clearance at full swing.
const MAX_SWING: f32 = 0.82;

#[derive(Clone, Default)]
struct PanelRegistry(Vec<GlassPanel>);

/// Optional hook for custom glass surfaces; normal frames are discovered from
/// their actual, current-frame paint shapes, including nested shadow groups.
#[allow(dead_code)]
pub fn register_panel(ctx: &egui::Context, rect: Rect, radius: f32) {
    if rect.is_positive() {
        ctx.data_mut(|data| {
            data.get_temp_mut_or_default::<PanelRegistry>(egui::Id::new("disco-glass-panels"))
                .0
                .push(GlassPanel {
                    rect,
                    clip: rect,
                    radius,
                })
        });
    }
}

fn collect_shape_panels(
    shape: &Shape,
    clip: Rect,
    transform: egui::emath::TSTransform,
    fills: &[Color32],
    panels: &mut Vec<GlassPanel>,
) {
    match shape {
        Shape::Rect(shape)
            if fills.contains(&shape.fill)
                && shape.rect.width() >= 80.0
                && shape.rect.height() >= 60.0 =>
        {
            let rect = transform * shape.rect;
            let clip = transform * clip;
            if rect.intersect(clip).is_positive() {
                panels.push(GlassPanel {
                    rect,
                    clip,
                    radius: f32::from(shape.corner_radius.nw) * transform.scaling.abs(),
                });
            }
        }
        Shape::Vec(shapes) => {
            for shape in shapes {
                collect_shape_panels(shape, clip, transform, fills, panels);
            }
        }
        _ => {}
    }
}

fn current_panels(ctx: &egui::Context, root: egui::LayerId) -> Vec<GlassPanel> {
    let palette = crate::theme::current(ctx);
    let fills = [
        palette.card,
        palette.sidebar,
        palette.card.gamma_multiply(0.35),
        palette.card.gamma_multiply(0.65),
        ctx.style_of(ctx.theme()).visuals.window_fill,
    ];
    let mut layers = ctx.memory(|memory| memory.layer_ids().collect::<Vec<_>>());
    if !layers.contains(&root) {
        layers.insert(0, root);
    }
    // Avoid context re-entry while egui's graphics lock is held.
    let layers = layers
        .into_iter()
        .map(|layer| {
            (
                layer,
                ctx.layer_transform_to_global(layer).unwrap_or_default(),
            )
        })
        .collect::<Vec<_>>();
    let mut panels = ctx
        .data(|data| data.get_temp::<PanelRegistry>(egui::Id::new("disco-glass-panels")))
        .unwrap_or_default()
        .0;
    ctx.graphics(|graphics| {
        for (layer, transform) in layers {
            if let Some(list) = graphics.get(layer) {
                for shape in list.all_entries() {
                    collect_shape_panels(
                        &shape.shape,
                        shape.clip_rect,
                        transform,
                        &fills,
                        &mut panels,
                    );
                }
            }
        }
    });
    panels
}

#[derive(Clone, Copy, Debug, Default)]
struct Pendulum {
    angle: f32,
    velocity: f32,
}

impl Pendulum {
    fn step(&mut self, dt: f32) {
        // Fixed small substeps keep a fast pointer or a resumed window stable.
        let count = (dt / 0.004).ceil().max(1.0) as usize;
        let step = dt / count as f32;
        for _ in 0..count {
            self.velocity += (-17.0 * self.angle.sin() - 1.55 * self.velocity) * step;
            self.angle += self.velocity * step;
            if self.angle.abs() > MAX_SWING {
                self.angle = self.angle.clamp(-MAX_SWING, MAX_SWING);
                self.velocity *= -0.34;
            }
        }
        if self.angle.abs() < 0.0001 && self.velocity.abs() < 0.0001 {
            *self = Self::default();
        }
    }

    fn strike(&mut self, pointer_velocity: Vec2, length: f32, contact: Vec2) {
        let tangent = Vec2::new(self.angle.cos(), -self.angle.sin());
        let tangential = pointer_velocity.dot(tangent);
        let push = if tangential.abs() > pointer_velocity.length() * 0.08 {
            tangential
        } else {
            pointer_velocity.length() * if contact.x >= 0.0 { -0.28 } else { 0.28 }
        };
        self.velocity = (self.velocity + push / length.max(1.0) * 0.31).clamp(-10.0, 10.0);
    }
}

#[derive(Clone, Copy)]
struct Ray {
    index: usize,
    azimuth: f32,
    vertical: f32,
    hue: f32,
    next_color: f64,
    fade_started: f64,
    reflection_seed: u64,
}

struct FramePaint {
    painter: egui::Painter,
    rays: egui::layers::ShapeIdx,
    ball: egui::layers::ShapeIdx,
    bounds: Rect,
    now: f64,
    dt: f32,
    mode: DiscoMode,
    ball_enabled: bool,
    pointer: Option<Pos2>,
}

pub struct DiscoScene {
    random: u64,
    // The supplied PNG contributes the mirror material; only points inside its
    // sphere are sampled, so the baked checkerboard never enters the scene.
    mirror: Vec<f32>,
    floor: DiscoFloor,
    stage: Option<Rect>,
    ray_count: usize,
    rays: Vec<Ray>,
    pendulum: Pendulum,
    rotation: f32,
    mode: DiscoMode,
    last_time: Option<f64>,
    last_pointer: Option<Pos2>,
    last_strike: f64,
    pending: Option<FramePaint>,
    window_reflection_percent: u8,
}

impl DiscoScene {
    pub fn new(_ctx: &egui::Context, directories: &[PathBuf]) -> Self {
        let image = directories
            .iter()
            .find_map(|directory| image::open(directory.join("discoball.png")).ok())
            .map(|image| image.to_rgba8());
        let mut mirror = Vec::with_capacity(LONGITUDES * LATITUDES);
        for row in 0..LATITUDES {
            let latitude = PI * (row as f32 + 0.5) / LATITUDES as f32;
            for column in 0..LONGITUDES {
                let longitude = TAU * (column as f32 + 0.5) / LONGITUDES as f32;
                let brightness = image.as_ref().map_or(0.58, |image| {
                    let x = ((0.5 + longitude.sin() * latitude.sin() * 0.43) * image.width() as f32)
                        as u32;
                    let y = ((0.5 - latitude.cos() * 0.43) * image.height() as f32) as u32;
                    let pixel =
                        image.get_pixel(x.min(image.width() - 1), y.min(image.height() - 1));
                    (f32::from(pixel[0]) + f32::from(pixel[1]) + f32::from(pixel[2])) / 765.0
                });
                mirror.push(brightness);
            }
        }
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1, |time| time.as_nanos() as u64)
            .max(1);
        Self {
            random: seed,
            mirror,
            floor: DiscoFloor::new(),
            stage: None,
            ray_count: 144,
            rays: Vec::new(),
            pendulum: Pendulum::default(),
            rotation: 0.0,
            mode: DiscoMode::Party,
            last_time: None,
            last_pointer: None,
            last_strike: -1.0,
            pending: None,
            window_reflection_percent: 18,
        }
    }

    pub fn pause(&mut self) {
        self.last_time = None;
        self.last_pointer = None;
        self.pendulum = Pendulum::default();
        self.pending = None;
        self.floor.pause();
    }

    pub fn set_window_reflection_percent(&mut self, percent: u8) {
        self.window_reflection_percent = percent.min(100);
    }

    fn random_unit(&mut self) -> f32 {
        self.random ^= self.random << 13;
        self.random ^= self.random >> 7;
        self.random ^= self.random << 17;
        (self.random >> 40) as f32 / 16_777_216.0
    }

    pub fn set_ray_count(&mut self, count: u16) {
        self.ray_count = usize::from(count.min(256));
    }

    fn update_rays(&mut self, now: f64) {
        if self.rays.len() != self.ray_count {
            self.rays.clear();
            for index in 0..self.ray_count {
                let azimuth = self.random_unit() * TAU;
                let vertical = self.random_unit() * 1.44 - 0.72;
                let hue = self.random_unit();
                let next_color = now + 0.5 + f64::from(self.random_unit());
                let fade_started = now - f64::from(self.random_unit());
                let reflection_seed = self.random;
                self.rays.push(Ray {
                    index,
                    azimuth,
                    vertical,
                    hue,
                    next_color,
                    fade_started,
                    reflection_seed,
                });
            }
        }
        for index in 0..self.rays.len() {
            if now >= self.rays[index].next_color {
                let hue = self.random_unit();
                let next_color = now + 0.5 + f64::from(self.random_unit());
                self.rays[index].hue = hue;
                self.rays[index].fade_started = now;
                self.rays[index].next_color = next_color;
            }
        }
    }

    /// Called after the photo/ambient background, before the first UI panel.
    /// Deferred slots retain that draw order while using this frame's exact
    /// panel rectangles to stop the rays, even during resize and scrolling.
    pub fn begin_frame(
        &mut self,
        ui: &egui::Ui,
        mode: DiscoMode,
        ball_enabled: bool,
        floor_enabled: bool,
        audio: AudioReactive,
    ) {
        let now = ui.input(|input| input.time);
        let dt = self
            .last_time
            .map_or(1.0 / 60.0, |last| (now - last).clamp(0.0, 0.05) as f32);
        self.last_time = Some(now);
        if self.mode != mode {
            self.last_pointer = None;
        }
        self.mode = mode;
        if mode != DiscoMode::Party || !ball_enabled {
            self.pendulum = Pendulum::default();
        }
        self.pendulum.step(dt);
        if mode != DiscoMode::Static && ball_enabled {
            self.rotation = (self.rotation + dt * 0.67).rem_euclid(TAU);
        }
        ui.ctx().data_mut(|data| {
            data.insert_temp(
                egui::Id::new("disco-glass-panels"),
                PanelRegistry::default(),
            )
        });
        let bounds = ui.max_rect();
        let painter = ui.painter().with_clip_rect(bounds);
        if floor_enabled {
            self.floor.paint(&painter, bounds, now, dt, audio);
        } else {
            self.floor.pause();
        }
        if ball_enabled {
            self.update_rays(now);
        }
        self.pending = Some(FramePaint {
            rays: painter.add(Shape::Noop),
            ball: painter.add(Shape::Noop),
            painter,
            bounds,
            now,
            dt,
            mode,
            ball_enabled,
            pointer: ui.input(|input| input.pointer.hover_pos()),
        });
        if ball_enabled {
            if mode == DiscoMode::Static {
                if !self.rays.is_empty() {
                    ui.ctx().request_repaint_after(Duration::from_millis(16));
                }
            } else {
                ui.ctx().request_repaint_after(Duration::from_millis(16));
            }
        }
        if floor_enabled && audio.energy > 0.008 {
            ui.ctx().request_repaint_after(Duration::from_millis(33));
        }
    }

    fn geometry(bounds: Rect, pendulum: Pendulum) -> (Pos2, Pos2, f32, f32) {
        let radius = (bounds.height() * 0.047).clamp(29.0, 43.0);
        let length = radius + 92.0;
        let anchor = Pos2::new(bounds.left() + bounds.width() * 0.73, bounds.top());
        let center = anchor + Vec2::new(pendulum.angle.sin(), pendulum.angle.cos()) * length;
        (anchor, center, radius, length)
    }

    pub fn stage_bottom(bounds: Rect) -> f32 {
        let (_, center, radius, _) = Self::geometry(bounds, Pendulum::default());
        center.y + radius + 14.0
    }

    pub fn set_stage_bounds(&mut self, bounds: Rect) {
        self.stage = bounds.is_positive().then_some(bounds);
    }

    fn frame_geometry(&self, bounds: Rect) -> (Pos2, Pos2, f32, f32) {
        let (mut anchor, mut center, radius, length) = Self::geometry(bounds, self.pendulum);
        if let Some(stage) = self.stage {
            let clearance = length * MAX_SWING.sin() + radius + 2.0;
            let min = stage.left() + clearance;
            let max = stage.right() - clearance;
            let preferred = stage.left() + stage.width() * 0.65;
            let x = if min <= max {
                preferred.clamp(min, max)
            } else {
                stage.center().x
            };
            center.x += x - anchor.x;
            anchor.x = x;
        }
        (anchor, center, radius, length)
    }

    fn pointer_strike(
        &mut self,
        frame: &FramePaint,
        center: Pos2,
        radius: f32,
        length: f32,
        panels: &[GlassPanel],
    ) {
        if frame.mode == DiscoMode::Party && frame.now - self.last_strike > 0.075 {
            if let (Some(previous), Some(pointer)) = (self.last_pointer, frame.pointer) {
                let travel = pointer - previous;
                let velocity = travel / frame.dt.max(1.0 / 240.0);
                if velocity.length_sq() > 45.0_f32.powi(2) && travel.length_sq() > 0.01 {
                    let along =
                        ((center - previous).dot(travel) / travel.length_sq()).clamp(0.0, 1.0);
                    let contact = previous + travel * along;
                    if contact.distance_sq(center) <= radius * radius
                        && !panels.iter().any(|panel| panel.contains(contact))
                    {
                        self.pendulum.strike(velocity, length, contact - center);
                        self.last_strike = frame.now;
                    }
                }
            }
        }
        self.last_pointer = frame.pointer;
    }

    /// Called after the UI. Only laser impact points occupy the foreground;
    /// rope, sphere, floor and all beam segments keep their background slots.
    pub fn finish_frame(&mut self, ctx: &egui::Context) {
        let Some(frame) = self.pending.take() else {
            return;
        };
        if !frame.ball_enabled {
            self.last_pointer = frame.pointer;
            return;
        }
        let panels = current_panels(ctx, frame.painter.layer_id());
        let (anchor, center, radius, length) = self.frame_geometry(frame.bounds);
        self.pointer_strike(&frame, center, radius, length, &panels);
        frame
            .painter
            .set(frame.ball, self.ball_shapes(anchor, center, radius));
        let mut shapes = Vec::new();
        let spots = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("disco-laser-spots"),
        ));
        let count = self.rays.len().max(1);
        let emitter = Vec3::new(center.x, center.y, 150.0);
        if !self.rays.is_empty() && frame.mode != DiscoMode::Static {
            for (slot, panel) in panels.iter().enumerate() {
                crate::disco_reflections::paint_wash(&spots, *panel, center, self.rotation, slot);
            }
        }
        for (slot, ray) in self.rays.iter().enumerate() {
            let fade = if frame.mode == DiscoMode::Static {
                let duration = (ray.next_color - ray.fade_started).max(0.001);
                let progress = ((frame.now - ray.fade_started) / duration).clamp(0.0, 1.0);
                (progress * std::f64::consts::PI).sin().powi(2) as f32
            } else {
                1.0
            };
            let panel_target = panels.get(slot).and_then(|panel| {
                crate::disco_reflections::target(*panel, center, self.rotation, slot)
            });
            let direction = panel_target.map_or_else(
                || {
                    disco_optics::emission_direction(
                        ray.azimuth,
                        ray.vertical,
                        self.rotation,
                        self.pendulum.angle,
                    )
                },
                |target| {
                    Vec3::new(target.x - center.x, target.y - center.y, -emitter.z).normalized()
                },
            );
            let trace = disco_optics::trace_with_reflections(
                emitter + direction * (radius * 0.96),
                direction,
                frame.bounds,
                &panels,
                disco_optics::WindowReflections {
                    chance: f32::from(self.window_reflection_percent) / 100.0,
                    seed: ray.reflection_seed,
                },
            );
            let color_hue = if frame.mode == DiscoMode::Static {
                ray.hue
            } else {
                (ray.index as f32 / count as f32 * 0.36
                    + self.rotation.sin() * 0.08
                    + ray.hue * 0.025)
                    .rem_euclid(1.0)
            };
            let color = hue(color_hue, 0.72, 1.0);
            let air_strength = fade
                * if panel_target.is_some() {
                    0.18
                } else if ray.index % 3 == 0 {
                    1.0
                } else {
                    0.36
                };
            for segment in &trace.segments {
                let start = segment.start.xy();
                let end = segment.end.xy();
                if start.distance_sq(end) > 0.25 {
                    shapes.push(beam(
                        start,
                        end,
                        5.0,
                        color,
                        (9.0 * segment.strength * air_strength) as u8,
                        (3.0 * segment.strength * air_strength) as u8,
                    ));
                    shapes.push(beam(
                        start,
                        end,
                        1.15,
                        color,
                        (62.0 * segment.strength * air_strength) as u8,
                        (28.0 * segment.strength * air_strength) as u8,
                    ));
                }
            }
            for impact in &trace.impacts {
                let strength = impact.strength * fade;
                let point = impact.point.xy();
                let footprint = disco_optics::footprint(impact.incident, impact.normal, 1.7);
                let (trail_alpha, glow_alpha, spot_alpha, core_alpha): (u8, u8, u8, u8) =
                    if impact.panel_index.is_some() {
                        (35, 12, 72, 90)
                    } else {
                        (105, 40, 220, 230)
                    };
                let clip = impact.panel_index.map_or(frame.bounds, |index| {
                    panels[index].clip.intersect(panels[index].rect)
                });
                let painter = spots.with_clip_rect(clip);
                if impact.kind == HitKind::Face && footprint.major > footprint.minor * 1.05 {
                    let direction = Vec2::new(impact.incident.x, impact.incident.y).normalized();
                    let trail = ((footprint.major / footprint.minor - 1.0) * 12.0).min(18.0);
                    painter.add(beam(
                        point - direction * trail,
                        point,
                        1.2,
                        color,
                        0,
                        (f32::from(trail_alpha) * strength) as u8,
                    ));
                }
                ellipse_light(
                    &painter,
                    point,
                    footprint.major * 4.0,
                    footprint.minor * 4.0,
                    footprint.angle,
                    with_alpha(color, (f32::from(glow_alpha) * strength) as u8),
                    true,
                );
                ellipse_light(
                    &painter,
                    point,
                    footprint.major,
                    footprint.minor,
                    footprint.angle,
                    with_alpha(color, (f32::from(spot_alpha) * strength) as u8),
                    false,
                );
                ellipse_light(
                    &painter,
                    point,
                    footprint.major * 0.34,
                    footprint.minor * 0.34,
                    footprint.angle,
                    Color32::from_white_alpha((f32::from(core_alpha) * strength) as u8),
                    false,
                );
            }
        }
        frame.painter.set(frame.rays, Shape::Vec(shapes));
    }

    fn ball_shapes(&self, anchor: Pos2, center: Pos2, radius: f32) -> Shape {
        crate::disco_ball::shapes(anchor, center, radius, self.rotation, &self.mirror)
    }
}

fn hue(hue: f32, saturation: f32, value: f32) -> Color32 {
    egui::ecolor::Hsva::new(hue, saturation, value, 1.0).into()
}

fn with_alpha(color: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

fn beam(
    start: Pos2,
    end: Pos2,
    width: f32,
    color: Color32,
    start_alpha: u8,
    end_alpha: u8,
) -> Shape {
    let perpendicular = (end - start).normalized().rot90() * (width * 0.5);
    let mut mesh = egui::epaint::Mesh::default();
    mesh.colored_vertex(start + perpendicular, with_alpha(color, start_alpha));
    mesh.colored_vertex(start - perpendicular, with_alpha(color, start_alpha));
    mesh.colored_vertex(end - perpendicular, with_alpha(color, end_alpha));
    mesh.colored_vertex(end + perpendicular, with_alpha(color, end_alpha));
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    Shape::mesh(mesh)
}

fn ellipse_light(
    painter: &egui::Painter,
    center: Pos2,
    major: f32,
    minor: f32,
    angle: f32,
    color: Color32,
    feathered: bool,
) {
    let mut mesh = egui::epaint::Mesh::default();
    mesh.colored_vertex(center, color);
    let rotation = egui::emath::Rot2::from_angle(angle);
    for index in 0..24 {
        mesh.colored_vertex(
            center + rotation * (Vec2::angled(index as f32 * TAU / 24.0) * Vec2::new(major, minor)),
            if feathered {
                Color32::TRANSPARENT
            } else {
                color
            },
        );
    }
    for index in 1..=24 {
        mesh.add_triangle(0, index, if index == 24 { 1 } else { index + 1 });
    }
    painter.add(Shape::mesh(mesh));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panel(rect: Rect, radius: f32) -> GlassPanel {
        GlassPanel {
            rect,
            clip: rect,
            radius,
        }
    }

    #[test]
    fn strongest_swing_keeps_the_entire_ball_below_the_window_edge() {
        for size in [
            Vec2::new(950.0, 610.0),
            Vec2::new(950.0, 790.0),
            Vec2::new(950.0, 1080.0),
            Vec2::new(1280.0, 790.0),
            Vec2::new(3840.0, 2160.0),
        ] {
            let bounds = Rect::from_min_size(Pos2::ZERO, size);
            for angle in [-MAX_SWING, 0.0, MAX_SWING] {
                let (_, center, radius, _) = DiscoScene::geometry(
                    bounds,
                    Pendulum {
                        angle,
                        velocity: 0.0,
                    },
                );
                assert!(center.y - radius >= bounds.top());
                assert!(center.x - radius >= bounds.left());
                assert!(center.x + radius <= bounds.right());
            }
        }
    }

    #[test]
    fn harder_cursor_hits_swing_farther_and_both_swings_return_to_rest() {
        let mut gentle = Pendulum::default();
        let mut fast = Pendulum::default();
        gentle.strike(Vec2::new(150.0, 0.0), 62.0, Vec2::new(-30.0, 0.0));
        fast.strike(Vec2::new(1200.0, 0.0), 62.0, Vec2::new(-30.0, 0.0));
        let mut gentle_max = 0.0_f32;
        let mut fast_max = 0.0_f32;
        for _ in 0..240 {
            gentle.step(1.0 / 120.0);
            fast.step(1.0 / 120.0);
            gentle_max = gentle_max.max(gentle.angle.abs());
            fast_max = fast_max.max(fast.angle.abs());
        }
        assert!(fast_max > gentle_max * 2.5);
        for _ in 0..2400 {
            gentle.step(1.0 / 120.0);
            fast.step(1.0 / 120.0);
        }
        assert!(gentle.angle.abs() < 0.001 && fast.angle.abs() < 0.001);
        assert!(gentle.velocity.abs() < 0.001 && fast.velocity.abs() < 0.001);
    }

    #[test]
    fn static_lasers_keep_geometry_and_change_colors_on_independent_deadlines() {
        let ctx = egui::Context::default();
        let mut scene = DiscoScene::new(&ctx, &[]);
        scene.random = 0x1234_5678;
        scene.rotation = 1.25;
        scene.set_ray_count(36);
        scene.update_rays(10.0);
        assert_eq!(scene.rays.len(), 36);
        let angles = scene
            .rays
            .iter()
            .map(|ray| (ray.azimuth, ray.vertical))
            .collect::<Vec<_>>();
        let deadlines = scene
            .rays
            .iter()
            .map(|ray| ray.next_color)
            .collect::<Vec<_>>();
        assert!(deadlines
            .iter()
            .all(|deadline| (10.5..11.5).contains(deadline)));
        assert!(deadlines
            .windows(2)
            .any(|pair| (pair[0] - pair[1]).abs() > 0.1));
        let first = deadlines.iter().copied().fold(f64::INFINITY, f64::min);
        let before = scene.rays.iter().map(|ray| ray.hue).collect::<Vec<_>>();
        scene.update_rays(first + 0.00001);
        let changed = scene
            .rays
            .iter()
            .zip(before)
            .filter(|(ray, hue)| ray.hue != *hue)
            .count();
        assert!(changed > 0 && changed < 36);
        assert_eq!(
            angles,
            scene
                .rays
                .iter()
                .map(|ray| (ray.azimuth, ray.vertical))
                .collect::<Vec<_>>()
        );
        assert_eq!(scene.rotation, 1.25);
    }

    fn includes(shape: &Shape, predicate: &impl Fn(&Shape) -> bool) -> bool {
        predicate(shape)
            || matches!(shape, Shape::Vec(shapes) if shapes.iter().any(|shape| includes(shape, predicate)))
    }

    #[test]
    fn actual_shadowed_panels_stop_deferred_beams_in_the_current_frame_and_preserve_layers() {
        let ctx = egui::Context::default();
        let palette =
            crate::theme::GlassPalette::for_appearance(crate::settings::Appearance::Disco);
        crate::theme::install(&ctx, palette);
        let photo = ctx.load_texture(
            "disco-layer-photo",
            egui::ColorImage::new([1, 1], vec![Color32::WHITE]),
            egui::TextureOptions::LINEAR,
        );
        let mut scene = DiscoScene::new(&ctx, &[]);
        for (frame, x) in [(0, 1005.0), (1, 1085.0)] {
            let mut menu_rect = Rect::NOTHING;
            let output = crate::test_support::run_ui(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 790.0))),
                    time: Some(10.0 + frame as f64 * 0.2),
                    ..Default::default()
                },
                |ui| {
                    ui.painter().image(
                        photo.id(),
                        ui.max_rect(),
                        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                        Color32::WHITE,
                    );
                    scene.begin_frame(ui, DiscoMode::Moving, true, true, AudioReactive::default());
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(
                        Rect::from_min_size(Pos2::new(x, 100.0), Vec2::new(170.0, 500.0)),
                    ));
                    let frame = egui::Frame::new()
                        .fill(palette.card)
                        .corner_radius(14)
                        .inner_margin(10)
                        .shadow(egui::epaint::Shadow {
                            offset: [0, 6],
                            blur: 20,
                            spread: 0,
                            color: Color32::from_black_alpha(65),
                        })
                        .show(&mut child, |ui| {
                            ui.set_min_size(Vec2::new(150.0, 450.0));
                            ui.label("Glass menu above the ball and floor");
                            let button = ui.add_sized(
                                [120.0, 40.0],
                                egui::Button::new("Control").fill(palette.card),
                            );
                            assert_eq!(button.rect.size(), Vec2::new(120.0, 40.0));
                            let button_rect = button.rect;
                            let panels = current_panels(&ctx, ui.layer_id());
                            assert!(!panels.iter().any(|panel| panel.rect == button_rect));
                        });
                    menu_rect = frame.response.rect;
                    let panels = current_panels(&ctx, ui.layer_id());
                    assert!(panels.iter().any(|panel| panel.rect == menu_rect));
                    assert!(
                        !panels
                            .iter()
                            .any(|panel| panel.rect.left()
                                == if x > 1050.0 { 1005.0 } else { 1085.0 })
                    );
                    let target = menu_rect.center();
                    let trace = disco_optics::trace(
                        Vec3::new(target.x, target.y, 150.0),
                        Vec3::new(0.0, 0.0, -1.0),
                        ui.max_rect(),
                        &panels,
                    );
                    assert_eq!(trace.impacts.len(), 1);
                    assert_eq!(trace.impacts[0].kind, HitKind::Face);
                    assert_eq!(trace.impacts[0].point.xy(), target);
                    scene.finish_frame(&ctx);
                },
            );
            let index_of = |predicate: &dyn Fn(&Shape) -> bool| {
                output
                    .shapes
                    .iter()
                    .position(|shape| includes(&shape.shape, &|shape| predicate(shape)))
                    .unwrap()
            };
            let photo_index = index_of(
                &|shape| matches!(shape, Shape::Mesh(mesh) if mesh.texture_id == photo.id()),
            );
            let floor_index = index_of(
                &|shape| matches!(shape, Shape::Path(path) if path.points.len() == 4 && path.fill.a() == 210),
            );
            let ball_index = index_of(
                &|shape| matches!(shape, Shape::Circle(circle) if circle.fill == Color32::from_rgb(34, 35, 49)),
            );
            let menu_index =
                index_of(&|shape| matches!(shape, Shape::Rect(rect) if rect.fill == palette.card));
            let spot_index = index_of(
                &|shape| matches!(shape, Shape::Mesh(mesh) if mesh.vertices.len() == 25 && mesh.vertices[0].color.a() == 72),
            );
            let beam_index = index_of(
                &|shape| matches!(shape, Shape::Mesh(mesh) if mesh.vertices.len() == 4 && mesh.texture_id == egui::TextureId::default() && mesh.vertices[0].color.a() == 9),
            );
            assert!(
                photo_index < floor_index
                    && floor_index < beam_index
                    && beam_index < ball_index
                    && ball_index < menu_index
                    && menu_index < spot_index
            );
        }
    }

    #[test]
    fn every_visible_menu_gets_reflected_light_within_the_existing_ray_budget() {
        let ctx = egui::Context::default();
        let palette =
            crate::theme::GlassPalette::for_appearance(crate::settings::Appearance::Disco);
        crate::theme::install(&ctx, palette);
        let mut scene = DiscoScene::new(&ctx, &[]);
        scene.set_ray_count(72);
        scene.set_window_reflection_percent(0);
        let rectangles = [
            Rect::from_min_max(egui::pos2(12.0, 12.0), egui::pos2(235.0, 660.0)),
            Rect::from_min_max(egui::pos2(16.0, 676.0), egui::pos2(1264.0, 778.0)),
            Rect::from_min_max(egui::pos2(264.0, 210.0), egui::pos2(1256.0, 390.0)),
            Rect::from_min_max(egui::pos2(264.0, 410.0), egui::pos2(418.0, 490.0)),
            Rect::from_min_max(egui::pos2(670.0, 510.0), egui::pos2(860.0, 660.0)),
        ];
        let output = crate::test_support::run_ui(
            &ctx,
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 790.0))),
                time: Some(1.0),
                ..Default::default()
            },
            |ui| {
                scene.begin_frame(ui, DiscoMode::Moving, true, false, AudioReactive::default());
                for rect in rectangles {
                    ui.painter().rect_filled(rect, 14, palette.card);
                }
                scene.finish_frame(&ctx);
            },
        );
        assert_eq!(scene.rays.len(), 72);
        let points = output
            .shapes
            .iter()
            .filter_map(|shape| {
                if let Shape::Mesh(mesh) = &shape.shape {
                    (mesh.vertices.len() == 25 && mesh.vertices[0].color.a() == 72)
                        .then_some(mesh.vertices[0].pos)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        for rect in rectangles {
            assert!(
                points.iter().any(|point| rect.contains(*point)),
                "No surface reflection on {rect:?}; points: {points:?}"
            );
        }
    }

    #[test]
    fn configured_ray_budget_applies_in_every_mode_and_lower_ball_has_an_accessible_stage() {
        let ctx = egui::Context::default();
        let mut scene = DiscoScene::new(&ctx, &[]);
        for count in [144, 108, 72] {
            scene.set_ray_count(count);
            scene.update_rays(1.0);
            assert_eq!(scene.rays.len(), usize::from(count));
            assert!(scene.rays.iter().all(|ray| ray.vertical.abs() <= 0.72));
            assert!(scene
                .rays
                .windows(2)
                .any(|pair| pair[0].azimuth != pair[1].azimuth));
        }
        for size in [
            Vec2::new(950.0, 610.0),
            Vec2::new(950.0, 790.0),
            Vec2::new(950.0, 1080.0),
            Vec2::new(1280.0, 790.0),
        ] {
            let bounds = Rect::from_min_size(Pos2::ZERO, size);
            for drawer in [false, true] {
                let stage = Rect::from_x_y_ranges(
                    263.0..=bounds.right() - if drawer { 344.0 } else { 24.0 },
                    bounds.y_range(),
                );
                scene.set_stage_bounds(stage);
                for angle in [-MAX_SWING, 0.0, MAX_SWING] {
                    scene.pendulum.angle = angle;
                    let (_, center, radius, _) = scene.frame_geometry(bounds);
                    assert!(center.x - radius >= stage.left());
                    assert!(center.x + radius <= stage.right());
                    assert!(center.y + radius < DiscoScene::stage_bottom(bounds));
                    assert!(center.y - radius > bounds.top());
                }
            }
        }
    }

    #[test]
    fn static_begin_frame_freezes_pose_and_moving_mode_ignores_cursor_hits() {
        let ctx = egui::Context::default();
        let mut scene = DiscoScene::new(&ctx, &[]);
        scene.rotation = 1.3;
        scene.pendulum = Pendulum {
            angle: 0.6,
            velocity: 2.0,
        };
        for time in [1.0, 2.0, 4.0] {
            crate::test_support::run_ui(
                &ctx,
                egui::RawInput {
                    time: Some(time),
                    ..Default::default()
                },
                |ui| {
                    scene.begin_frame(ui, DiscoMode::Static, true, false, AudioReactive::default());
                    assert_eq!(scene.rotation, 1.3);
                    assert_eq!(scene.pendulum.angle, 0.0);
                    assert_eq!(scene.pendulum.velocity, 0.0);
                    scene.finish_frame(&ctx);
                },
            );
        }
        crate::test_support::run_ui(
            &ctx,
            egui::RawInput {
                time: Some(4.1),
                ..Default::default()
            },
            |ui| {
                scene.begin_frame(ui, DiscoMode::Moving, true, false, AudioReactive::default());
                let mut frame = scene.pending.take().unwrap();
                let (_, center, radius, length) =
                    DiscoScene::geometry(frame.bounds, scene.pendulum);
                scene.last_pointer = Some(center - Vec2::X * 100.0);
                frame.pointer = Some(center + Vec2::X * 100.0);
                scene.pointer_strike(&frame, center, radius, length, &[]);
                assert_eq!(scene.pendulum.velocity, 0.0);
                assert_eq!(scene.pendulum.angle, 0.0);
                assert!(scene.rotation > 1.3);
                // Identical sweep in Party mode is blocked by covering glass.
                frame.mode = DiscoMode::Party;
                scene.last_pointer = Some(center - Vec2::X * 100.0);
                let covering = panel(
                    Rect::from_center_size(center, Vec2::splat(radius * 3.0)),
                    10.0,
                );
                scene.pointer_strike(&frame, center, radius, length, &[covering]);
                assert_eq!(scene.pendulum.velocity, 0.0);
                scene.last_pointer = Some(center - Vec2::X * 100.0);
                scene.pointer_strike(&frame, center, radius, length, &[]);
                assert!(scene.pendulum.velocity > 1.0);
            },
        );
    }
}

#[cfg(test)]
include!("disco_regression_tests.rs");
