//! Small, bounded 3D ray tracer for Disco's glass faces and reflective rims.
//! Positions use egui points in X/Y; positive Z is in front of the interface.
use eframe::egui::{Pos2, Rect};
#[cfg(test)]
use std::f32::consts::TAU;
use std::ops::{Add, Mul, Sub};

pub const MAX_BOUNCES: usize = 3;
const EDGE_DEPTH: f32 = 12.0;
const EPSILON: f32 = 0.035;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }
    pub fn xy(self) -> Pos2 {
        Pos2::new(self.x, self.y)
    }
    pub fn dot(self, other: Self) -> f32 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }
    pub fn length(self) -> f32 {
        self.dot(self).sqrt()
    }
    pub fn normalized(self) -> Self {
        let length = self.length();
        if length > 1.0e-8 && length.is_finite() {
            self * (1.0 / length)
        } else {
            Self::default()
        }
    }
    pub fn reflected(self, normal: Self) -> Self {
        self - normal * (2.0 * self.dot(normal))
    }
    fn finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }
}
impl Add for Vec3 {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self::new(self.x + rhs.x, self.y + rhs.y, self.z + rhs.z)
    }
}
impl Sub for Vec3 {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self::new(self.x - rhs.x, self.y - rhs.y, self.z - rhs.z)
    }
}
impl Mul<f32> for Vec3 {
    type Output = Self;
    fn mul(self, rhs: f32) -> Self {
        Self::new(self.x * rhs, self.y * rhs, self.z * rhs)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Panel {
    pub rect: Rect,
    pub clip: Rect,
    pub radius: f32,
}

impl Panel {
    fn radius(self) -> f32 {
        self.radius
            .max(0.0)
            .min(self.rect.width() * 0.5)
            .min(self.rect.height() * 0.5)
    }
    pub fn contains(self, point: Pos2) -> bool {
        if !self.rect.is_positive() || !self.clip.contains(point) || !self.rect.contains(point) {
            return false;
        }
        let radius = self.radius();
        let inner = self.rect.shrink(radius);
        let closest = Pos2::new(
            point.x.clamp(inner.left(), inner.right()),
            point.y.clamp(inner.top(), inner.bottom()),
        );
        point.distance_sq(closest) <= radius * radius + 0.001
    }
}

/// Random azimuths cover the sphere while a latitude clamp leaves a clear cap
/// around each pole, including during the party-mode swing.
pub fn emission_direction(azimuth: f32, vertical: f32, rotation: f32, swing: f32) -> Vec3 {
    let vertical = vertical.clamp(-0.72, 0.72);
    let horizontal = (1.0 - vertical * vertical).sqrt();
    let azimuth = azimuth + rotation;
    let (s, c) = azimuth.sin_cos();
    let direction = Vec3::new(horizontal * c, vertical, horizontal * s);
    let (s, c) = swing.sin_cos();
    let swung = Vec3::new(
        direction.x * c - direction.y * s,
        direction.x * s + direction.y * c,
        direction.z,
    );
    let vertical = swung.y.clamp(-0.78, 0.78);
    let horizontal = (swung.x * swung.x + swung.z * swung.z).sqrt().max(1.0e-8);
    let horizontal_scale = (1.0 - vertical * vertical).sqrt() / horizontal;
    Vec3::new(
        swung.x * horizontal_scale,
        vertical,
        swung.z * horizontal_scale,
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitKind {
    Face,
    Edge,
    WindowEdge,
}

#[derive(Clone, Copy, Debug)]
pub struct Segment {
    pub start: Vec3,
    pub end: Vec3,
    pub strength: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Impact {
    pub point: Vec3,
    pub normal: Vec3,
    pub incident: Vec3,
    pub kind: HitKind,
    pub panel_index: Option<usize>,
    pub strength: f32,
}

#[derive(Clone, Debug, Default)]
pub struct Trace {
    pub segments: Vec<Segment>,
    pub impacts: Vec<Impact>,
}

/// A persistent per-ray seed makes the decision stable while the ball turns.
/// The probability can change without creating frame-to-frame flicker.
#[derive(Clone, Copy, Debug)]
pub struct WindowReflections {
    pub chance: f32,
    pub seed: u64,
}

fn window_reflects(policy: WindowReflections) -> bool {
    let chance = if policy.chance.is_finite() {
        policy.chance.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let mut hash = policy.seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    hash = (hash ^ (hash >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    hash = (hash ^ (hash >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    hash ^= hash >> 31;
    let sample = (hash >> 40) as f32 / 16_777_216.0;
    sample < chance
}

#[derive(Clone, Copy, Debug)]
pub struct Footprint {
    pub major: f32,
    pub minor: f32,
    pub angle: f32,
}

/// Circular at normal incidence and stretched along the ray's tangent at an
/// oblique incidence (sqrt(2) times longer at 45 degrees). Grazing incidence is
/// capped to keep the glow local and the menu readable.
pub fn footprint(incident: Vec3, normal: Vec3, base_radius: f32) -> Footprint {
    let incident = incident.normalized();
    let normal = normal.normalized();
    let cosine = incident.dot(normal).abs().clamp(0.16, 1.0);
    let tangent = incident - normal * incident.dot(normal);
    Footprint {
        major: base_radius / cosine,
        minor: base_radius,
        angle: tangent.y.atan2(tangent.x),
    }
}

#[derive(Clone, Copy, Debug)]
struct Hit {
    distance: f32,
    normal: Vec3,
    kind: HitKind,
    panel_index: Option<usize>,
}

fn nearer(best: &mut Option<Hit>, candidate: Hit) {
    if candidate.distance > EPSILON
        && candidate.distance.is_finite()
        && best.is_none_or(|old| {
            candidate.distance < old.distance - 0.0001
                || ((candidate.distance - old.distance).abs() <= 0.0001
                    && candidate.kind == HitKind::Edge)
        })
    {
        *best = Some(candidate);
    }
}

fn panel_hit(origin: Vec3, direction: Vec3, panel: Panel, index: usize) -> Option<Hit> {
    if !panel.rect.intersect(panel.clip).is_positive() {
        return None;
    }
    let mut best = None;
    if direction.z.abs() > 1.0e-6 {
        let distance = -origin.z / direction.z;
        if distance > EPSILON && panel.contains((origin + direction * distance).xy()) {
            nearer(
                &mut best,
                Hit {
                    distance,
                    normal: Vec3::new(0.0, 0.0, if direction.z < 0.0 { 1.0 } else { -1.0 }),
                    kind: HitKind::Face,
                    panel_index: Some(index),
                },
            );
        }
    }
    let rect = panel.rect;
    let radius = panel.radius();
    let mut edge = |distance: f32, normal: Vec3| {
        let point = origin + direction * distance;
        let incidence = direction.dot(normal);
        if distance > EPSILON
            && (-0.001..=EDGE_DEPTH).contains(&point.z)
            && panel.clip.expand(0.001).contains(point.xy())
            && incidence.abs() > 1.0e-6
        {
            // A thin mirror rim is visible from both sides. Point the normal
            // back toward the incident side so the reflected ray and epsilon
            // offset stay inside the panel after an inner-rim contact.
            let normal = if incidence > 0.0 {
                normal * -1.0
            } else {
                normal
            };
            nearer(
                &mut best,
                Hit {
                    distance,
                    normal,
                    kind: HitKind::Edge,
                    panel_index: Some(index),
                },
            );
        }
    };
    if direction.x.abs() > 1.0e-6 {
        for (x, normal) in [
            (rect.left(), Vec3::new(-1.0, 0.0, 0.0)),
            (rect.right(), Vec3::new(1.0, 0.0, 0.0)),
        ] {
            let distance = (x - origin.x) / direction.x;
            let y = origin.y + direction.y * distance;
            if y >= rect.top() + radius && y <= rect.bottom() - radius {
                edge(distance, normal);
            }
        }
    }
    if direction.y.abs() > 1.0e-6 {
        for (y, normal) in [
            (rect.top(), Vec3::new(0.0, -1.0, 0.0)),
            (rect.bottom(), Vec3::new(0.0, 1.0, 0.0)),
        ] {
            let distance = (y - origin.y) / direction.y;
            let x = origin.x + direction.x * distance;
            if x >= rect.left() + radius && x <= rect.right() - radius {
                edge(distance, normal);
            }
        }
    }
    let horizontal = direction.x * direction.x + direction.y * direction.y;
    if radius > 0.0 && horizontal > 1.0e-8 {
        for (center, sx, sy) in [
            (
                rect.left_top() + eframe::egui::Vec2::splat(radius),
                -1.0,
                -1.0,
            ),
            (
                rect.right_top() + eframe::egui::Vec2::new(-radius, radius),
                1.0,
                -1.0,
            ),
            (
                rect.right_bottom() - eframe::egui::Vec2::splat(radius),
                1.0,
                1.0,
            ),
            (
                rect.left_bottom() + eframe::egui::Vec2::new(radius, -radius),
                -1.0,
                1.0,
            ),
        ] {
            let x = origin.x - center.x;
            let y = origin.y - center.y;
            let b = x * direction.x + y * direction.y;
            let discriminant = b * b - horizontal * (x * x + y * y - radius * radius);
            if discriminant >= 0.0 {
                for distance in [
                    (-b - discriminant.sqrt()) / horizontal,
                    (-b + discriminant.sqrt()) / horizontal,
                ] {
                    let point = origin + direction * distance;
                    let x = point.x - center.x;
                    let y = point.y - center.y;
                    if x * sx >= -0.001 && y * sy >= -0.001 {
                        edge(
                            distance,
                            Vec3::new(x / radius, y / radius, 0.0).normalized(),
                        );
                    }
                }
            }
        }
    }
    // Scrolling cuts the rendered panel at its clip boundary. The visible rim
    // remains a reflector; neither face spots nor reflected segments can leak
    // into a portion of the panel that was scrolled outside the viewport.
    for (coordinate, normal, x_axis) in [
        (panel.clip.left(), Vec3::new(-1.0, 0.0, 0.0), true),
        (panel.clip.right(), Vec3::new(1.0, 0.0, 0.0), true),
        (panel.clip.top(), Vec3::new(0.0, -1.0, 0.0), false),
        (panel.clip.bottom(), Vec3::new(0.0, 1.0, 0.0), false),
    ] {
        let component = if x_axis { direction.x } else { direction.y };
        if component.abs() < 1.0e-6 {
            continue;
        }
        let distance = (coordinate - if x_axis { origin.x } else { origin.y }) / component;
        let before = origin + direction * (distance - EPSILON);
        let after = origin + direction * (distance + EPSILON);
        if panel.contains(before.xy()) || panel.contains(after.xy()) {
            edge(distance, normal);
        }
    }
    best
}

fn window_hit(origin: Vec3, direction: Vec3, bounds: Rect, room_depth: f32) -> Option<Hit> {
    let mut best: Option<Hit> = None;
    for (coordinate, normal, x_axis) in [
        (bounds.left(), Vec3::new(1.0, 0.0, 0.0), true),
        (bounds.right(), Vec3::new(-1.0, 0.0, 0.0), true),
        (bounds.top(), Vec3::new(0.0, 1.0, 0.0), false),
        (bounds.bottom(), Vec3::new(0.0, -1.0, 0.0), false),
    ] {
        let component = if x_axis { direction.x } else { direction.y };
        if component.abs() < 1.0e-6 || direction.dot(normal) >= 0.0 {
            continue;
        }
        let distance = (coordinate - if x_axis { origin.x } else { origin.y }) / component;
        let point = origin + direction * distance;
        if distance > EPSILON
            && bounds.expand(0.01).contains(point.xy())
            && (0.0..=room_depth).contains(&point.z)
        {
            if let Some(old) = best
                .as_mut()
                .filter(|old| (old.distance - distance).abs() < 0.001)
            {
                old.normal = (old.normal + normal).normalized();
            } else {
                nearer(
                    &mut best,
                    Hit {
                        distance,
                        normal,
                        kind: HitKind::WindowEdge,
                        panel_index: None,
                    },
                );
            }
        }
    }
    best
}

/// Legacy full-reflection tracer, retained for isolated geometric tests.
/// The scene uses `trace_with_reflections` for sparse, seeded window bounces.
#[cfg(test)]
pub fn trace(origin: Vec3, direction: Vec3, bounds: Rect, panels: &[Panel]) -> Trace {
    trace_impl(origin, direction, bounds, panels, None)
}

pub fn trace_with_reflections(
    origin: Vec3,
    direction: Vec3,
    bounds: Rect,
    panels: &[Panel],
    window: WindowReflections,
) -> Trace {
    trace_impl(origin, direction, bounds, panels, Some(window))
}

fn trace_impl(
    mut origin: Vec3,
    mut direction: Vec3,
    bounds: Rect,
    panels: &[Panel],
    window: Option<WindowReflections>,
) -> Trace {
    let mut output = Trace::default();
    direction = direction.normalized();
    if !origin.finite() || !direction.finite() || direction.length() < 0.5 || !bounds.is_positive()
    {
        return output;
    }
    let maximum_distance = bounds.size().length().max(100.0) * 2.5;
    let room_depth = origin.z.max(0.0) + bounds.size().length() * 0.8;
    let mut remaining = maximum_distance;
    let mut strength = 1.0;
    let mut window_reflections = 0;
    for bounce in 0..=MAX_BOUNCES {
        let mut hit = window_hit(origin, direction, bounds, room_depth);
        // Last painted panel wins a coincident front-face hit.
        for (index, panel) in panels.iter().enumerate().rev() {
            if let Some(candidate) = panel_hit(origin, direction, *panel, index) {
                nearer(&mut hit, candidate);
            }
        }
        let collision = hit.filter(|hit| hit.distance <= remaining);
        let distance = collision.map_or(remaining, |hit| hit.distance);
        let end = origin + direction * distance;
        output.segments.push(Segment {
            start: origin,
            end,
            strength,
        });
        let Some(hit) = collision else {
            break;
        };
        if hit.kind == HitKind::WindowEdge {
            if let Some(policy) = window {
                // An unreflected ray leaves the window quietly. In particular,
                // it does not leave a false impact point along the frame.
                if window_reflections >= 1 || !window_reflects(policy) {
                    break;
                }
            }
            window_reflections += 1;
        }
        output.impacts.push(Impact {
            point: end,
            normal: hit.normal,
            incident: direction,
            kind: hit.kind,
            panel_index: hit.panel_index,
            strength,
        });
        if hit.kind == HitKind::Face || bounce == MAX_BOUNCES {
            break;
        }
        remaining -= distance;
        if remaining <= EPSILON {
            break;
        }
        direction = direction.reflected(hit.normal).normalized();
        origin = end + hit.normal * EPSILON + direction * EPSILON;
        strength *= if hit.kind == HitKind::WindowEdge && window.is_some() {
            0.34
        } else {
            0.64
        };
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    fn panel(min: (f32, f32), max: (f32, f32), radius: f32) -> Panel {
        let rect = Rect::from_min_max(Pos2::new(min.0, min.1), Pos2::new(max.0, max.1));
        Panel {
            rect,
            clip: rect,
            radius,
        }
    }
    fn bounds() -> Rect {
        Rect::from_min_max(Pos2::new(-500.0, -500.0), Pos2::new(1500.0, 1000.0))
    }

    #[test]
    fn randomized_emissions_cover_both_depth_directions_and_repeat_after_a_turn() {
        let mut directions = Vec::new();
        for azimuth in [0.0, TAU * 0.25, TAU * 0.5, TAU * 0.75] {
            for vertical in [-0.65, 0.0, 0.65] {
                let direction = emission_direction(azimuth, vertical, 0.3, 0.2);
                assert!((direction.length() - 1.0).abs() < 0.0001);
                assert!(direction.y.abs() <= 0.78);
                assert!(
                    (direction - emission_direction(azimuth, vertical, 0.3 + TAU, 0.2)).length()
                        < 0.0001
                );
                directions.push(direction);
            }
        }
        assert!(directions.iter().any(|direction| direction.x > 0.8));
        assert!(directions.iter().any(|direction| direction.x < -0.8));
        assert!(directions.iter().any(|direction| direction.z > 0.8));
        assert!(directions.iter().any(|direction| direction.z < -0.8));
    }

    #[test]
    fn randomized_rays_turn_visibly_and_stay_out_of_the_polar_caps() {
        for index in 0..256 {
            let azimuth = index as f32 * 2.399_963_1;
            let vertical = ((index as f32 * 0.618_034).fract() * 1.44) - 0.72;
            let first = emission_direction(azimuth, vertical, 0.0, 0.0);
            let quarter_turn = emission_direction(azimuth, vertical, TAU * 0.25, 0.0);
            assert!((first - quarter_turn).length() > 0.9);
            for swing in [-0.82, 0.0, 0.82] {
                let direction = emission_direction(azimuth, vertical, 0.0, swing);
                assert!(direction.y.abs() <= 0.78);
                assert!((direction.length() - 1.0).abs() < 0.0001);
            }
        }
    }

    #[test]
    fn perpendicular_face_is_only_a_dot_and_oblique_face_has_a_longer_footprint() {
        let glass = panel((100.0, 100.0), (900.0, 700.0), 16.0);
        let perpendicular = trace(
            Vec3::new(300.0, 300.0, 150.0),
            Vec3::new(0.0, 0.0, -1.0),
            bounds(),
            &[glass],
        );
        assert_eq!(perpendicular.impacts.len(), 1);
        assert_eq!(perpendicular.impacts[0].kind, HitKind::Face);
        assert_eq!(
            perpendicular.segments[0].start.xy(),
            perpendicular.segments[0].end.xy()
        );
        let round = footprint(
            perpendicular.impacts[0].incident,
            perpendicular.impacts[0].normal,
            2.0,
        );
        assert!((round.major - round.minor).abs() < 0.0001);
        let oblique = trace(
            Vec3::new(300.0, 300.0, 150.0),
            Vec3::new(1.0, 0.0, -1.0),
            bounds(),
            &[glass],
        );
        assert_eq!(oblique.impacts.len(), 1);
        assert_eq!(oblique.impacts[0].kind, HitKind::Face);
        assert!((oblique.impacts[0].point.x - 450.0).abs() < 0.001);
        let oval = footprint(oblique.impacts[0].incident, oblique.impacts[0].normal, 2.0);
        assert!((oval.major / oval.minor - 2.0_f32.sqrt()).abs() < 0.001);
        assert_eq!(oblique.segments.len(), 1, "the face must absorb the beam");
    }

    #[test]
    fn reflective_menu_rim_can_redirect_a_beam_onto_another_glass_face() {
        let small = panel((100.0, 100.0), (200.0, 200.0), 8.0);
        let larger = panel((-100.0, 80.0), (250.0, 250.0), 10.0);
        let light = trace(
            Vec3::new(50.0, 150.0, 8.0),
            Vec3::new(1.0, 0.0, -0.05),
            bounds(),
            &[larger, small],
        );
        assert_eq!(light.impacts.len(), 2, "{light:?}");
        assert_eq!(light.impacts[0].kind, HitKind::Edge);
        assert_eq!(light.impacts[0].panel_index, Some(1));
        assert_eq!(light.impacts[1].kind, HitKind::Face);
        assert_eq!(light.impacts[1].panel_index, Some(0));
        assert!(light.segments[1].end.x < light.segments[1].start.x);
        assert!(light.segments[1].strength < light.segments[0].strength);
    }

    #[test]
    fn inner_menu_and_scrolled_rims_reflect_back_toward_the_incident_side() {
        let glass = panel((100.0, 100.0), (200.0, 200.0), 8.0);
        for glass in [
            glass,
            Panel {
                clip: Rect::from_min_max(Pos2::new(130.0, 100.0), Pos2::new(190.0, 200.0)),
                ..glass
            },
        ] {
            let light = trace(
                Vec3::new(150.0, 150.0, 8.0),
                Vec3::new(1.0, 0.0, -0.1),
                bounds(),
                &[glass],
            );
            assert_eq!(light.impacts.len(), 2, "{light:?}");
            assert_eq!(light.impacts[0].kind, HitKind::Edge);
            assert!(
                (light.impacts[0].point.x - glass.rect.intersect(glass.clip).right()).abs() < 0.001
            );
            assert!(light.impacts[0].normal.x < -0.99);
            assert!(light.impacts[0].incident.dot(light.impacts[0].normal) < 0.0);
            assert!(light.segments[1].start.x < light.impacts[0].point.x);
            assert!(light.segments[1].end.x < light.segments[1].start.x);
            assert_eq!(light.impacts[1].kind, HitKind::Face);
            assert!(glass.contains(light.impacts[1].point.xy()));
        }
    }

    #[test]
    fn window_reflections_are_bounded_and_rays_can_escape_without_a_hit() {
        let viewport = Rect::from_min_max(Pos2::ZERO, Pos2::new(800.0, 600.0));
        let bounced = trace(
            Vec3::new(400.0, 300.0, 100.0),
            Vec3::new(1.0, 0.3, 0.0),
            viewport,
            &[],
        );
        assert!(bounced.impacts.len() >= 2);
        assert!(bounced
            .impacts
            .iter()
            .all(|impact| impact.kind == HitKind::WindowEdge));
        assert!(bounced.segments.len() <= MAX_BOUNCES + 1);
        assert!(bounced
            .segments
            .iter()
            .all(|segment| segment.start.finite() && segment.end.finite()));
        let escaped = trace(
            Vec3::new(400.0, 300.0, 100.0),
            Vec3::new(0.0, 0.0, 1.0),
            viewport,
            &[],
        );
        assert!(escaped.impacts.is_empty());
        assert_eq!(escaped.segments.len(), 1);
    }

    #[test]
    fn production_window_policy_exits_without_a_dot_or_reflects_only_once() {
        let viewport = Rect::from_min_max(Pos2::ZERO, Pos2::new(800.0, 600.0));
        let origin = Vec3::new(400.0, 300.0, 100.0);
        let direction = Vec3::new(1.0, 0.3, 0.0);
        let exited = trace_with_reflections(
            origin,
            direction,
            viewport,
            &[],
            WindowReflections {
                chance: 0.0,
                seed: 42,
            },
        );
        assert!(
            exited.impacts.is_empty(),
            "an exiting ray leaves no border dot"
        );
        assert_eq!(exited.segments.len(), 1);
        assert!((exited.segments[0].end.x - viewport.right()).abs() < 0.001);
        let reflected = trace_with_reflections(
            origin,
            direction,
            viewport,
            &[],
            WindowReflections {
                chance: 1.0,
                seed: 42,
            },
        );
        assert_eq!(reflected.impacts.len(), 1);
        assert_eq!(reflected.impacts[0].kind, HitKind::WindowEdge);
        assert_eq!(
            reflected.segments.len(),
            2,
            "the next window contact must exit instead of making a web"
        );
        assert!((reflected.segments[1].strength - 0.34).abs() < 0.001);
        assert!(reflected.segments[1].end.x < reflected.segments[1].start.x);
    }

    #[test]
    fn window_probability_selects_a_sparse_stable_subset_of_rays() {
        let viewport = Rect::from_min_max(Pos2::ZERO, Pos2::new(800.0, 600.0));
        let origin = Vec3::new(400.0, 300.0, 100.0);
        let mut selected = 0;
        for seed in 0..256 {
            let policy = WindowReflections { chance: 0.18, seed };
            let initial =
                trace_with_reflections(origin, Vec3::new(1.0, 0.3, 0.0), viewport, &[], policy);
            let later =
                trace_with_reflections(origin, Vec3::new(0.8, 0.2, 0.0), viewport, &[], policy);
            assert_eq!(
                initial.impacts.len(),
                later.impacts.len(),
                "motion must not reroll the window reflection each frame"
            );
            assert!(initial.impacts.len() <= 1);
            selected += initial.impacts.len();
        }
        assert!((25..80).contains(&selected), "selected {selected}/256 rays");
    }

    #[test]
    fn rounded_and_scrolled_glass_uses_only_its_visible_face_and_rim() {
        let glass = panel((100.0, 100.0), (200.0, 200.0), 20.0);
        assert!(!glass.contains(Pos2::new(101.0, 101.0)));
        let missed = trace(
            Vec3::new(101.0, 101.0, 100.0),
            Vec3::new(0.0, 0.0, -1.0),
            bounds(),
            &[glass],
        );
        assert!(missed.impacts.is_empty());
        let clipped = Panel {
            clip: Rect::from_min_max(Pos2::new(130.0, 100.0), Pos2::new(190.0, 200.0)),
            ..glass
        };
        let reflected = trace(
            Vec3::new(110.0, 150.0, 8.0),
            Vec3::new(1.0, 0.0, -0.01),
            bounds(),
            &[clipped],
        );
        assert_eq!(reflected.impacts[0].kind, HitKind::Edge);
        assert!((reflected.impacts[0].point.x - 130.0).abs() < 0.001);
        let hidden = trace(
            Vec3::new(120.0, 150.0, 100.0),
            Vec3::new(0.0, 0.0, -1.0),
            bounds(),
            &[clipped],
        );
        assert!(hidden.impacts.is_empty());
    }
}
