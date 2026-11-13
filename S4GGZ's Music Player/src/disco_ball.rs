//! A lit mirror globe: the material rotates, while the stage lights stay put.
use eframe::egui::{self, Color32, Pos2, Shape, Vec2};
use std::f32::consts::{PI, TAU};

const LONGITUDES: usize = 32;
const LATITUDES: usize = 16;
const HALO_STEPS: usize = 64;

/// The caller paints this group below its glass panels. Lighting has no wall
/// clock input, so a static globe keeps exactly the same pose and highlights.
pub fn shapes(anchor: Pos2, center: Pos2, radius: f32, rotation: f32, mirror: &[f32]) -> Shape {
    if !anchor.x.is_finite()
        || !anchor.y.is_finite()
        || !center.x.is_finite()
        || !center.y.is_finite()
        || !radius.is_finite()
        || radius <= 0.0
        || !rotation.is_finite()
    {
        return Shape::Noop;
    }
    let mut shapes = Vec::with_capacity(LONGITUDES * LATITUDES / 2 + 24);
    let down = (center - anchor).normalized();
    let attachment = center - down * (radius + 2.0);
    shapes.push(Shape::line_segment(
        [anchor + Vec2::X, attachment + Vec2::X],
        egui::Stroke::new(3.0, Color32::from_black_alpha(160)),
    ));
    shapes.push(Shape::line_segment(
        [anchor, attachment],
        egui::Stroke::new(1.2, Color32::from_rgb(160, 163, 182)),
    ));
    shapes.push(Shape::circle_filled(
        attachment,
        3.0,
        Color32::from_rgb(235, 233, 200),
    ));

    // One vertex-colored mesh supplies the broad feathered aura. The body is
    // drawn over its inner rings, leaving a clean luminous outline rather than
    // a transparent wash over the mirror facets.
    shapes.push(halo(center, radius));
    shapes.push(Shape::circle_filled(
        center + Vec2::new(1.0, 3.0),
        radius + 1.0,
        Color32::from_black_alpha(70),
    ));
    // Keep the silver base visible in the seams between individual mirrors.
    shapes.push(Shape::circle_filled(
        center,
        radius,
        Color32::from_rgb(34, 35, 49),
    ));

    let project = |longitude: f32, latitude: f32| {
        let longitude = longitude + rotation;
        let x = latitude.sin() * longitude.sin();
        let y = -latitude.cos();
        let z = latitude.sin() * longitude.cos();
        (center + Vec2::new(x, y) * radius, x, y, z)
    };
    for row in 0..LATITUDES {
        let latitude = PI * (row as f32 + 0.5) / LATITUDES as f32;
        for column in 0..LONGITUDES {
            let longitude = TAU * (column as f32 + 0.5) / LONGITUDES as f32;
            let (_, x, y, z) = project(longitude, latitude);
            if z <= 0.015 {
                continue;
            }
            let material = mirror
                .get(row * LONGITUDES + column)
                .copied()
                .filter(|value| value.is_finite())
                .unwrap_or(0.6)
                .clamp(0.0, 1.0);
            // Stable object-space hues travel with each mirror. Broad colored
            // lights and white specular reflections describe the sphere's
            // volume, rather than making it look like a flat colored grid.
            let variation = ((row * 17 + column * 29) % 31) as f32 / 31.0;
            let hue =
                (column as f32 / LONGITUDES as f32 + row as f32 * 0.075 + variation * 0.22).fract();
            let pigment: Color32 =
                egui::ecolor::Hsva::new(hue, 0.58 + variation * 0.20, 1.0, 1.0).into();
            let key = (-x * 0.42 - y * 0.50 + z * 0.76).max(0.0);
            let cyan = (x * 0.57 + y * 0.16 + z * 0.80).max(0.0).powi(3);
            let pink = (-x * 0.64 + y * 0.36 + z * 0.68).max(0.0).powi(4);
            let specular = key.powi(34) * 0.90 + cyan.powi(8) * 0.16;
            let rim = (1.0 - z).powi(4) * 0.24;
            let brightness = 0.43 + material * 0.24 + key * 0.28;
            let channels = [pigment.r(), pigment.g(), pigment.b()];
            let tint = [
                0.07 * pink + 0.02 * cyan,
                0.03 * pink + 0.10 * cyan,
                0.08 * pink + 0.12 * cyan,
            ];
            let rgb = std::array::from_fn::<u8, 3, _>(|channel| {
                let base = f32::from(channels[channel]) / 255.0;
                ((base * brightness + specular + rim + tint[channel]).clamp(0.0, 1.0) * 255.0) as u8
            });
            let color = Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
            let gap = 0.012;
            let top = PI * row as f32 / LATITUDES as f32 + gap;
            let bottom = PI * (row + 1) as f32 / LATITUDES as f32 - gap;
            let left = TAU * column as f32 / LONGITUDES as f32 + gap;
            let right = TAU * (column + 1) as f32 / LONGITUDES as f32 - gap;
            shapes.push(Shape::convex_polygon(
                vec![
                    project(left, top).0,
                    project(right, top).0,
                    project(right, bottom).0,
                    project(left, bottom).0,
                ],
                color,
                egui::Stroke::NONE,
            ));
        }
    }
    shapes.push(luminous_rim(center, radius));
    let pulse = 0.78 + (rotation * 2.0).cos().powi(8) * 0.22;
    sparkle(
        &mut shapes,
        center + Vec2::new(-0.42, -0.57) * radius,
        radius * 0.13,
        pulse,
    );
    sparkle(
        &mut shapes,
        center + Vec2::new(0.42, 0.54) * radius,
        radius * 0.075,
        0.50,
    );
    Shape::Vec(shapes)
}

fn halo(center: Pos2, radius: f32) -> Shape {
    let mut mesh = egui::epaint::Mesh::default();
    for (scale, alpha) in [(0.92, 105), (1.025, 95), (1.28, 25), (1.82, 0)] {
        for step in 0..HALO_STEPS {
            let angle = step as f32 * TAU / HALO_STEPS as f32;
            let rainbow: Color32 =
                egui::ecolor::Hsva::new((angle / TAU + 0.1).fract(), 0.60, 1.0, 1.0).into();
            let warm = [255.0, 239.0, 148.0];
            let rgb = [rainbow.r(), rainbow.g(), rainbow.b()];
            let color = std::array::from_fn::<u8, 3, _>(|channel| {
                (f32::from(rgb[channel]) * 0.43 + warm[channel] * 0.57) as u8
            });
            mesh.colored_vertex(
                center + Vec2::angled(angle) * (radius * scale),
                Color32::from_rgba_unmultiplied(color[0], color[1], color[2], alpha),
            );
        }
    }
    for ring in 0..3 {
        for step in 0..HALO_STEPS {
            let a = (ring * HALO_STEPS + step) as u32;
            let b = (ring * HALO_STEPS + (step + 1) % HALO_STEPS) as u32;
            let c = a + HALO_STEPS as u32;
            let d = b + HALO_STEPS as u32;
            mesh.add_triangle(a, c, b);
            mesh.add_triangle(b, c, d);
        }
    }
    Shape::mesh(mesh)
}

fn luminous_rim(center: Pos2, radius: f32) -> Shape {
    let mut mesh = egui::epaint::Mesh::default();
    for step in 0..HALO_STEPS {
        let angle = step as f32 * TAU / HALO_STEPS as f32;
        let direction = Vec2::angled(angle);
        let highlight = direction.dot(Vec2::new(-0.58, -0.81)).max(0.0).powi(2);
        let alpha = (68.0 + highlight * 150.0) as u8;
        let warm = Color32::from_rgba_unmultiplied(255, 250, 193, alpha);
        mesh.colored_vertex(center + direction * (radius * 0.992), warm);
        mesh.colored_vertex(
            center + direction * (radius * 1.035),
            Color32::from_rgba_unmultiplied(255, 246, 164, alpha / 4),
        );
    }
    for step in 0..HALO_STEPS {
        let a = (step * 2) as u32;
        let b = (((step + 1) % HALO_STEPS) * 2) as u32;
        mesh.add_triangle(a, a + 1, b);
        mesh.add_triangle(b, a + 1, b + 1);
    }
    Shape::mesh(mesh)
}

fn sparkle(shapes: &mut Vec<Shape>, center: Pos2, size: f32, strength: f32) {
    let mut mesh = egui::epaint::Mesh::default();
    let core = Color32::from_white_alpha((230.0 * strength) as u8);
    mesh.colored_vertex(center, core);
    for index in 0..8 {
        let angle = index as f32 * TAU / 8.0;
        let length = if index % 2 == 0 { size } else { size * 0.20 };
        mesh.colored_vertex(center + Vec2::angled(angle) * length, Color32::TRANSPARENT);
    }
    for index in 1..=8 {
        mesh.add_triangle(0, index, if index == 8 { 1 } else { index + 1 });
    }
    shapes.push(Shape::mesh(mesh));
    shapes.push(Shape::circle_filled(center, 0.8, core));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_lighting_is_repeatable_and_a_turn_moves_the_colored_facets() {
        let anchor = Pos2::new(250.0, 0.0);
        let center = Pos2::new(250.0, 130.0);
        let material = vec![0.6; LONGITUDES * LATITUDES];
        let first = shapes(anchor, center, 38.0, 0.4, &material);
        assert_eq!(first, shapes(anchor, center, 38.0, 0.4, &material));
        assert_ne!(first, shapes(anchor, center, 38.0, 0.85, &material));
        let Shape::Vec(parts) = first else {
            panic!("missing ball geometry")
        };
        let facets = parts
            .iter()
            .filter_map(|shape| match shape {
                Shape::Path(path) if path.points.len() == 4 => Some(path.fill),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(facets.len() > 200);
        assert!(facets
            .iter()
            .any(|color| color.r() > color.g().saturating_add(35)));
        assert!(facets
            .iter()
            .any(|color| color.g() > color.r().saturating_add(35)));
        assert!(facets
            .iter()
            .any(|color| color.b() > color.r().saturating_add(35)));
        assert!(facets
            .iter()
            .any(|color| color.r() > 225 && color.g() > 225));
    }

    #[test]
    fn feathered_halo_and_geometry_remain_finite_and_local() {
        let anchor = Pos2::new(400.0, 0.0);
        let center = Pos2::new(400.0, 130.0);
        let radius = 43.0;
        // Missing or invalid image material still yields a valid lit globe.
        let Shape::Vec(parts) = shapes(anchor, center, radius, 1.0, &[f32::NAN]) else {
            panic!("missing ball geometry")
        };
        let outer = radius * 1.83;
        for shape in &parts {
            if let Shape::Mesh(mesh) = shape {
                assert!(mesh
                    .indices
                    .iter()
                    .all(|index| (*index as usize) < mesh.vertices.len()));
                for vertex in &mesh.vertices {
                    assert!(vertex.pos.x.is_finite() && vertex.pos.y.is_finite());
                    assert!(vertex.pos.distance(center) <= outer);
                }
            }
        }
        let Shape::Mesh(aura) = &parts[3] else {
            panic!("missing feathered halo")
        };
        assert!(aura.vertices.iter().any(|vertex| vertex.color.a() > 80));
        assert!(aura
            .vertices
            .iter()
            .skip(HALO_STEPS * 3)
            .all(|vertex| vertex.color.a() == 0));
        assert_eq!(shapes(anchor, center, 0.0, 0.0, &[]), Shape::Noop);
    }
}
