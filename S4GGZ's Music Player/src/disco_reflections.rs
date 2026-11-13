//! Quiet, coherent reflections on every visible frosted-glass surface.
use crate::disco_optics::Panel;
use eframe::egui::{self, Color32, Pos2, Rect, Vec2};
use std::f32::consts::{PI, TAU};

fn visible(panel: Panel) -> Option<Rect> {
    let rect = panel.rect.intersect(panel.clip);
    (rect.is_positive() && rect.is_finite()).then_some(rect)
}

/// A rounded rectangle is convex and grows inward toward its center. Its
/// closest visible point to that center is therefore a safe starting point
/// even when a scroll clip reveals only a small portion of a rounded corner.
fn safe_point(panel: Panel, rect: Rect) -> Option<Pos2> {
    let center = panel.rect.center();
    let point = Pos2::new(
        center.x.clamp(rect.left(), rect.right()),
        center.y.clamp(rect.top(), rect.bottom()),
    );
    panel.contains(point).then_some(point)
}

fn inside_toward(panel: Panel, start: Pos2, desired: Pos2) -> Pos2 {
    if panel.contains(desired) {
        return desired;
    }
    let mut inside = 0.0;
    let mut outside = 1.0;
    for _ in 0..18 {
        let middle = (inside + outside) * 0.5;
        if panel.contains(start.lerp(desired, middle)) {
            inside = middle;
        } else {
            outside = middle;
        }
    }
    start.lerp(desired, inside * 0.995)
}

/// The glint moves smoothly along an off-center ellipse, never jumping between
/// random positions. Favoring the farther lateral rim keeps broad/tall panels
/// from introducing nearly stationary rays directly below the ball.
pub fn target(panel: Panel, emitter: Pos2, rotation: f32, slot: usize) -> Option<Pos2> {
    if !rotation.is_finite() || !emitter.x.is_finite() || !emitter.y.is_finite() {
        return None;
    }
    let rect = visible(panel)?;
    let safe = safe_point(panel, rect)?;
    // The visible rim protrudes 12 points toward the light. Leave enough
    // independent X/Y clearance for the incoming segment at that depth, so a
    // glint aimed at the face does not first strike the panel's reflective side.
    let base_padding = panel.radius.max(0.0) + 6.0;
    let padding = Vec2::new(
        base_padding
            .max((rect.center().x - emitter.x).abs() * 0.09 + 4.0)
            .min((rect.width() * 0.5 - 2.0).max(0.0)),
        base_padding
            .max((rect.center().y - emitter.y).abs() * 0.09 + 4.0)
            .min((rect.height() * 0.5 - 2.0).max(0.0)),
    );
    let interior = Rect::from_min_max(rect.min + padding, rect.max - padding);
    let side = if emitter.x >= rect.center().x {
        -1.0
    } else {
        1.0
    };
    let phase = rotation + slot as f32 * 1.12;
    let desired = interior.center()
        + Vec2::new(
            interior.width() * (side * 0.31 + phase.cos() * 0.12),
            interior.height() * phase.sin() * 0.36,
        );
    Some(inside_toward(panel, safe, desired))
}

fn wash_mesh(panel: Panel, center: Pos2, size: Vec2, color: Color32) -> egui::epaint::Mesh {
    let mut mesh = egui::epaint::Mesh::default();
    mesh.colored_vertex(center, color);
    for index in 0..40 {
        let direction = Vec2::angled(index as f32 * TAU / 40.0);
        let edge = inside_toward(panel, center, center + direction * size);
        mesh.colored_vertex(edge, Color32::TRANSPARENT);
    }
    for index in 1..=40 {
        mesh.add_triangle(0, index, if index == 40 { 1 } else { index + 1 });
    }
    mesh
}

fn wash_color(hue: f32, alpha: u8) -> Color32 {
    let rgb: Color32 = egui::ecolor::Hsva::new(hue.rem_euclid(1.0), 0.72, 1.0, 1.0).into();
    Color32::from_rgba_unmultiplied(rgb.r(), rgb.g(), rgb.b(), alpha)
}

/// Two feathered lobes tint the glass without obscuring text. All geometry is
/// contained in the rounded panel as well as its current scroll clip. Inputs
/// deliberately omit wall time so Static mode freezes the whole reflection.
pub fn paint_wash(
    painter: &egui::Painter,
    panel: Panel,
    emitter: Pos2,
    rotation: f32,
    slot: usize,
) {
    let Some(rect) = visible(panel) else {
        return;
    };
    let Some(primary) = target(panel, emitter, rotation, slot) else {
        return;
    };
    let reflected_emitter = Pos2::new(rect.center().x * 2.0 - emitter.x, emitter.y);
    let secondary = target(panel, reflected_emitter, rotation + PI, slot).unwrap_or(primary);
    let clip = rect.intersect(painter.clip_rect());
    if !clip.is_positive() {
        return;
    }
    let painter = painter.with_clip_rect(clip);
    let size = Vec2::new(
        (rect.width() * 0.40).clamp(8.0, 190.0),
        (rect.height() * 0.39).clamp(8.0, 160.0),
    );
    let hue = rotation / TAU + slot as f32 * 0.137;
    painter.add(egui::Shape::mesh(wash_mesh(
        panel,
        primary,
        size,
        wash_color(hue, 5),
    )));
    painter.add(egui::Shape::mesh(wash_mesh(
        panel,
        secondary,
        size * 0.88,
        wash_color(hue + 0.34, 3),
    )));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panel(rect: Rect, clip: Rect, radius: f32) -> Panel {
        Panel { rect, clip, radius }
    }

    #[test]
    fn targets_stay_on_rounded_glass_even_for_narrow_corner_clips() {
        let rect = Rect::from_min_max(Pos2::new(100.0, 100.0), Pos2::new(500.0, 350.0));
        let panels = [
            panel(rect, rect, 24.0),
            panel(
                rect,
                Rect::from_min_max(Pos2::new(120.0, 110.0), Pos2::new(370.0, 330.0)),
                24.0,
            ),
            panel(
                rect,
                Rect::from_min_max(Pos2::new(100.0, 100.0), Pos2::new(110.0, 118.0)),
                30.0,
            ),
            panel(
                rect,
                Rect::from_min_max(Pos2::new(100.0, 150.0), Pos2::new(105.0, 320.0)),
                24.0,
            ),
        ];
        for glass in panels {
            for step in 0..80 {
                for slot in [0, 5, 12] {
                    let point = target(
                        glass,
                        Pos2::new(850.0, 80.0),
                        step as f32 * TAU / 80.0,
                        slot,
                    )
                    .unwrap();
                    assert!(glass.contains(point), "{glass:?}: {point:?}");
                    assert!(glass.clip.contains(point));
                }
            }
        }
        let invisible = panel(rect, Rect::from_min_size(rect.min, Vec2::splat(2.0)), 30.0);
        assert!(target(invisible, Pos2::ZERO, 0.0, 0).is_none());
    }

    #[test]
    fn reflection_is_static_for_fixed_pose_and_sweeps_coherently_with_rotation() {
        let rect = Rect::from_min_max(Pos2::new(20.0, 500.0), Pos2::new(1260.0, 770.0));
        let glass = panel(rect, rect, 18.0);
        let emitter = Pos2::new(910.0, 130.0);
        let first = target(glass, emitter, 0.4, 2).unwrap();
        assert_eq!(target(glass, emitter, 0.4, 2), Some(first));
        let after_turn = target(glass, emitter, 0.4 + TAU, 2).unwrap();
        assert!(first.distance(after_turn) < 0.001);
        let moved = target(glass, emitter, 0.8, 2).unwrap();
        assert!(first.distance(moved) > 5.0);
        assert!(first.x < rect.center().x);
        assert!((first.x - emitter.x).abs() > 400.0);
        let next = target(glass, emitter, 0.401, 2).unwrap();
        assert!(first.distance(next) < 1.0);
    }

    #[test]
    fn targeted_glints_reach_common_menu_faces_without_a_prior_rim_hit() {
        use crate::disco_optics::{self, HitKind, Vec3, WindowReflections};
        let bounds = Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 790.0));
        let emitter = Vec3::new(910.0, 129.0, 150.0);
        let menus = [
            Rect::from_min_max(Pos2::new(12.0, 12.0), Pos2::new(235.0, 660.0)),
            Rect::from_min_max(Pos2::new(16.0, 676.0), Pos2::new(1264.0, 778.0)),
            Rect::from_min_max(Pos2::new(264.0, 210.0), Pos2::new(1256.0, 390.0)),
            Rect::from_min_max(Pos2::new(670.0, 510.0), Pos2::new(860.0, 660.0)),
        ];
        for (slot, rect) in menus.into_iter().enumerate() {
            let glass = panel(rect, rect, 18.0);
            for rotation in [0.0, 0.5, 1.0, 2.0, 3.0, 4.0, 5.0] {
                let point = target(glass, emitter.xy(), rotation, slot).unwrap();
                let direction = Vec3::new(point.x - emitter.x, point.y - emitter.y, -emitter.z);
                let trace = disco_optics::trace_with_reflections(
                    emitter,
                    direction,
                    bounds,
                    &[glass],
                    WindowReflections {
                        chance: 0.0,
                        seed: 7,
                    },
                );
                assert_eq!(
                    trace.impacts.len(),
                    1,
                    "slot {slot}, rotation {rotation}: {trace:?}"
                );
                assert_eq!(
                    trace.impacts[0].kind,
                    HitKind::Face,
                    "slot {slot}, rotation {rotation}: {trace:?}"
                );
                assert!(trace.impacts[0].point.xy().distance(point) < 0.01);
            }
        }
    }

    #[test]
    fn low_alpha_wash_vertices_are_clipped_to_the_actual_rounded_surface() {
        let ctx = egui::Context::default();
        let rect = Rect::from_min_max(Pos2::new(100.0, 100.0), Pos2::new(500.0, 350.0));
        let glass = panel(
            rect,
            Rect::from_min_max(Pos2::new(104.0, 108.0), Pos2::new(355.0, 340.0)),
            30.0,
        );
        let output = crate::test_support::run_ui(&ctx, egui::RawInput::default(), |ui| {
            paint_wash(ui.painter(), glass, Pos2::new(850.0, 80.0), 0.7, 4);
        });
        let meshes = output
            .shapes
            .iter()
            .filter_map(|shape| {
                if let egui::Shape::Mesh(mesh) = &shape.shape {
                    Some((shape.clip_rect, mesh))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(meshes.len(), 2);
        for (clip, mesh) in meshes {
            assert!(glass.rect.contains_rect(clip));
            assert!(glass.clip.contains_rect(clip));
            assert!(mesh
                .vertices
                .iter()
                .all(|vertex| glass.contains(vertex.pos)));
            assert!(mesh.vertices.iter().all(|vertex| vertex.color.a() <= 10));
            assert!(mesh.vertices.iter().any(|vertex| vertex.color.a() > 0));
            assert!(mesh.vertices.iter().any(|vertex| vertex.color.a() == 0));
        }
    }
}
